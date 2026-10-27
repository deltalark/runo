//! Turns that have gone quiet. A running agent prints something every few seconds while
//! it thinks, writes or calls tools; one that prints nothing for many minutes is most
//! often stuck on a command that never ends (`gh run watch`, a sleep loop), a CLI that
//! stalled, or an approval nobody saw. After `stuck_after_mins` of silence (longer while a
//! known long command such as a build or a test run is running) the Working line, the
//! thread's card and Crew say so, with Stop and Retry. Nothing is stopped unless the
//! person set `stuck_stop_after_mins`.

use std::time::Duration;

/// How many times longer a known long command may stay quiet.
const LONG_FACTOR: u32 = 3;
/// The least a known long command gets, in minutes.
const LONG_MIN_MINS: u32 = 30;

/// Commands that can run for many minutes without printing, judged from their text.
pub fn long_command(command: &str) -> bool {
    let c = command.to_lowercase();
    const LONG: &[&str] = &[
        "cargo build",
        "cargo test",
        "cargo nextest",
        "cargo check",
        "cargo clippy",
        "cargo run",
        "cargo bench",
        "cargo install",
        "npm run build",
        "npm test",
        "npm run test",
        "npm ci",
        "npm install",
        "pnpm build",
        "pnpm test",
        "pnpm install",
        "yarn build",
        "yarn test",
        "yarn install",
        "make",
        "cmake --build",
        "ninja",
        "xcodebuild",
        "swift build",
        "swift test",
        "gradle",
        "gradlew",
        "mvn ",
        "pytest",
        "tox",
        "go build",
        "go test",
        "docker build",
        "docker compose build",
        "bazel ",
        "dotnet build",
        "dotnet test",
        "pod install",
    ];
    // Waiting on something else isn't a long command, however long it takes.
    !blocking_wait(command)
        && LONG.iter().any(|l| {
            c.match_indices(l).any(|(at, _)| {
                // At the start of a word, so "remake" isn't make.
                at == 0 || !c.as_bytes()[at - 1].is_ascii_alphanumeric()
            })
        })
}

/// Commands that wait on something else with no end of their own: `gh run watch`, `gh pr
/// checks --watch` without a timeout, and sleep loops. Runo's watchers do this waiting.
pub fn blocking_wait(command: &str) -> bool {
    let c = command.to_lowercase();
    let timeout = c.contains("timeout ") || c.contains("--timeout");
    let sleep_loop =
        c.contains("sleep") && (c.contains("while ") || c.contains("until ") || c.contains("for "));
    ((c.contains("gh run watch") || (c.contains("gh pr checks") && c.contains("--watch")))
        && !timeout)
        || sleep_loop
}

/// How long a turn may stay quiet before it counts as stalled, with `base_mins` from the
/// settings and the command it is running, if any. None when the check is off.
pub fn threshold(base_mins: u32, command: Option<&str>) -> Option<Duration> {
    if base_mins == 0 {
        return None;
    }
    let mins = if command.is_some_and(long_command) {
        (base_mins * LONG_FACTOR).max(LONG_MIN_MINS)
    } else {
        base_mins
    };
    Some(Duration::from_secs(u64::from(mins) * 60))
}

/// Whether a turn quiet for `quiet` has stalled.
pub fn stalled(quiet: Duration, base_mins: u32, command: Option<&str>) -> bool {
    threshold(base_mins, command).is_some_and(|t| quiet >= t)
}

/// Whether a stalled turn is stopped on its own, with `stop_mins` from the settings (0,
/// the default, never).
pub fn auto_stop(quiet: Duration, stop_mins: u32) -> bool {
    stop_mins > 0 && quiet >= Duration::from_secs(u64::from(stop_mins) * 60)
}

/// "10m", "1h 6m".
pub fn mins(d: Duration) -> String {
    let m = d.as_secs() / 60;
    if m >= 60 {
        format!("{}h {}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

/// What the turn is on, for the lines below: a running command's text, else its step.
fn on(step: &str) -> &str {
    step.strip_prefix("Ran ").unwrap_or(step)
}

/// The Working line's note: "No output for 10m · gh run watch".
pub fn working_text(quiet: Duration, step: Option<&str>) -> String {
    match step {
        Some(s) => format!("No output for {} · {}", mins(quiet), on(s)),
        None => format!("No output for {}", mins(quiet)),
    }
}

/// The line for Crew: "#76 stalled 10m on gh run watch".
pub fn crew_text(thread: u64, quiet: Duration, step: Option<&str>) -> String {
    let mut text = format!("#{thread} stalled {}", mins(quiet));
    if let Some(s) = step {
        text.push_str(&format!(" on {}", on(s)));
        if blocking_wait(s) {
            text.push_str(
                " (a wait with no end; Runo's watchers cover it, so stopping it is safe)",
            );
        }
    }
    text.push_str(". Tell the person, or stop or retry it from its card.");
    text
}

/// The prompt Retry sends once the stalled turn has stopped.
pub fn retry_prompt(quiet: Duration, step: Option<&str>) -> String {
    let what = match step {
        Some(s) => format!("`{}`", on(s)),
        None => "your last step".into(),
    };
    format!(
        "Your turn printed nothing for {} on {what}, so Runo stopped it. Pick up where you left off. Don't run waits with no end (`gh run watch`, `gh pr checks --watch`, sleep loops): give a command a timeout, or end your turn and let Runo watch CI.",
        mins(quiet)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_quiet_turn_stalls_after_the_setting() {
        assert!(!stalled(MIN * 9, 10, None));
        assert!(stalled(MIN * 10, 10, None));
        assert!(stalled(MIN * 10, 10, Some("Ran gh run watch 123")));
        // Off.
        assert!(!stalled(MIN * 600, 0, None));
        assert_eq!(threshold(0, None), None);
    }

    #[test]
    fn a_long_command_gets_longer() {
        assert_eq!(
            threshold(10, Some("Ran cargo test --workspace")),
            Some(MIN * 30)
        );
        assert_eq!(
            threshold(20, Some("cd app && npm run build")),
            Some(MIN * 60)
        );
        assert!(!stalled(MIN * 25, 10, Some("Ran xcodebuild -scheme App")));
        assert!(stalled(MIN * 30, 10, Some("Ran xcodebuild -scheme App")));
        // Words that only contain a long command's name don't count.
        assert_eq!(threshold(10, Some("Ran remake.sh")), Some(MIN * 10));
        assert_eq!(threshold(10, Some("Ran git status")), Some(MIN * 10));
    }

    #[test]
    fn waits_with_no_end_are_known() {
        assert!(blocking_wait("gh run watch 123"));
        assert!(blocking_wait("gh pr checks 7 --watch"));
        assert!(blocking_wait("while true; do gh run list; sleep 30; done"));
        assert!(!blocking_wait("timeout 600 gh run watch 123"));
        assert!(!blocking_wait("gh pr checks 7"));
        assert!(!blocking_wait("sleep 2"));
        // A build loop around a wait is still a wait.
        assert!(!long_command("until cargo test; do sleep 5; done"));
    }

    #[test]
    fn auto_stop_is_off_unless_set() {
        assert!(!auto_stop(MIN * 600, 0));
        assert!(!auto_stop(MIN * 44, 45));
        assert!(auto_stop(MIN * 45, 45));
    }

    #[test]
    fn the_lines_say_how_long_and_on_what() {
        assert_eq!(
            working_text(MIN * 10, Some("Ran gh run watch 123")),
            "No output for 10m · gh run watch 123"
        );
        assert_eq!(
            working_text(MIN * 66 + Duration::from_secs(16), None),
            "No output for 1h 6m"
        );
        let crew = crew_text(76, MIN * 10, Some("Ran gh run watch"));
        assert!(crew.starts_with("#76 stalled 10m on gh run watch (a wait with no end"));
        assert!(retry_prompt(MIN * 12, Some("Ran gh run watch")).contains("`gh run watch`"));
    }
}
