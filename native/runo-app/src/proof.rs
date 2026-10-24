//! Proof of done: the project's checks run after each turn that changed files, a red
//! result goes back to the agent, and an agent from another vendor reviews the change.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How long one check may run before it is stopped and counted as failed.
pub const CHECK_LIMIT: Duration = Duration::from_secs(15 * 60);
/// How long the second-opinion agent may take.
pub const REVIEW_LIMIT: Duration = Duration::from_secs(10 * 60);
/// Lines of a check's output kept for the card and the fix prompt.
const TAIL_LINES: usize = 40;
const TAIL_BYTES: usize = 6000;

/// The verdict on a session's last turn, kept with the session.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Proof {
    /// Index of the user message whose turn was checked.
    pub turn: usize,
    #[serde(default)]
    pub results: Vec<CheckRun>,
    /// Automatic fix rounds sent in a row; back to 0 once the checks pass.
    #[serde(default)]
    pub rounds: u32,
    /// What the last failure looked like, so the same failure twice stops fixing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_failure: String,
    /// What the second opinion said, such as "Codex left 2 notes".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub review: String,
    /// Why the checks didn't run, such as a session on a connection.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub skipped: String,
}

impl Proof {
    pub fn passed(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.ok)
    }

    pub fn failed(&self) -> Vec<&CheckRun> {
        self.results.iter().filter(|r| !r.ok).collect()
    }
}

/// One check's result.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct CheckRun {
    pub command: String,
    pub ok: bool,
    #[serde(default)]
    pub secs: u64,
    /// The end of its output, colours removed.
    #[serde(default)]
    pub tail: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

/// The project's checks: one command per line, blank lines and `#` comments skipped.
pub fn commands(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Checks worth suggesting for the project in `dir`, from its manifests.
pub fn detect(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let has = |name: &str| dir.join(name).exists();
    if has("package.json") {
        let pm = if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else if has("bun.lockb") || has("bun.lock") {
            "bun"
        } else {
            "npm"
        };
        let scripts = std::fs::read_to_string(dir.join("package.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("scripts").cloned())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        for name in ["typecheck", "lint", "test", "build"] {
            let Some(script) = scripts.get(name).and_then(|s| s.as_str()) else {
                continue;
            };
            // npm init's placeholder test only fails.
            if script.contains("no test specified") {
                continue;
            }
            out.push(if pm == "npm" && name == "test" {
                "npm test".to_string()
            } else {
                format!("{pm} run {name}")
            });
        }
    }
    if has("Cargo.toml") {
        out.push("cargo test".into());
    }
    if has("go.mod") {
        out.push("go vet ./...".into());
        out.push("go test ./...".into());
    }
    if has("pyproject.toml") || has("setup.py") || has("requirements.txt") {
        let pyproject = std::fs::read_to_string(dir.join("pyproject.toml")).unwrap_or_default();
        if pyproject.contains("ruff") || has("ruff.toml") || has(".ruff.toml") {
            out.push("ruff check .".into());
        }
        if pyproject.contains("pytest") || has("pytest.ini") || has("tests") {
            out.push("pytest -q".into());
        }
    }
    if out.is_empty() && has("Makefile") {
        let make = std::fs::read_to_string(dir.join("Makefile")).unwrap_or_default();
        for target in ["check", "lint", "test"] {
            if make.lines().any(|l| l.starts_with(&format!("{target}:"))) {
                out.push(format!("make {target}"));
            }
        }
    }
    out
}

/// Run each check in turn in `dir`. Every check runs, so one card shows all that failed.
pub fn run_all(dir: &str, commands: &[String], limit: Duration) -> Vec<CheckRun> {
    commands.iter().map(|c| run_one(dir, c, limit)).collect()
}

fn run_one(dir: &str, command: &str, limit: Duration) -> CheckRun {
    let started = Instant::now();
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        // One pipe for both streams keeps errors next to what led to them.
        let mut c = Command::new("sh");
        c.args(["-c", &format!("exec 2>&1\n{command}")]);
        c
    };
    cmd.current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("CI", "1")
        .env("NO_COLOR", "1");
    // A worktree's build goes to the main checkout's target, not a copy of its own.
    crate::cargotarget::apply(&mut cmd, dir);
    if crate::cargotarget::is_rust(dir) && std::env::var_os("CARGO_BUILD_JOBS").is_none() {
        if let Some(jobs) = crate::cargotarget::check_jobs() {
            cmd.env("CARGO_BUILD_JOBS", jobs.to_string());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so a timeout also stops what it started.
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            return CheckRun {
                command: command.into(),
                ok: false,
                secs: 0,
                tail: format!("Could not start it: {error}"),
                timed_out: false,
            }
        }
    };
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = started + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            _ => {
                #[cfg(unix)]
                unsafe {
                    // SAFETY: kill on our own child's process group id.
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let secs = started.elapsed().as_secs();
    let text = match status {
        Some(_) => {
            let mut text = out.join().unwrap_or_default();
            text.push_str(&err.join().unwrap_or_default());
            text
        }
        // A grandchild may still hold the pipes open after a timeout.
        None => String::new(),
    };
    let timed_out = status.is_none();
    let mut tail = tail(&crate::agent::strip_ansi(&text));
    if timed_out {
        tail = format!("Stopped after {} minutes.", limit.as_secs() / 60);
    } else if let Some(code) = status.and_then(|s| s.code()).filter(|c| *c != 0) {
        if tail.is_empty() {
            tail = format!("Exited with code {code} and printed nothing.");
        }
    }
    CheckRun {
        command: command.into(),
        ok: status.is_some_and(|s| s.success()),
        secs,
        tail,
        timed_out,
    }
}

/// The last lines of `text`, at most `TAIL_LINES` lines and about `TAIL_BYTES` bytes.
pub fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    let mut out = lines[start..].join("\n");
    if out.len() > TAIL_BYTES {
        let mut cut = out.len() - TAIL_BYTES;
        while !out.is_char_boundary(cut) {
            cut += 1;
        }
        out = out[cut..].to_string();
    }
    out.trim_start_matches('\n').to_string()
}

/// What a set of failures looks like, without numbers (timings and counts change between
/// runs), so a fix round that changed nothing is noticed.
pub fn failure_key(results: &[CheckRun]) -> String {
    results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| {
            let end: Vec<&str> = r
                .tail
                .lines()
                .rev()
                .filter(|l| !l.trim().is_empty())
                .take(3)
                .collect();
            let end: String = end
                .join("|")
                .chars()
                .filter(|c| !c.is_ascii_digit())
                .collect();
            format!("{}#{}", r.command, end)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The follow-up that asks the agent to make the failing checks pass.
pub fn fix_prompt(results: &[CheckRun]) -> String {
    let failed: Vec<&CheckRun> = results.iter().filter(|r| !r.ok).collect();
    let mut out = format!(
        "Runo ran the project's checks after your last turn and {} failed. Fix the cause in \
         the code. Don't skip, disable or loosen tests or lint rules to make them pass. Run \
         the failing command{} yourself to confirm before you stop.\n",
        if failed.len() == 1 {
            "one".to_string()
        } else {
            format!("{} of them", failed.len())
        },
        if failed.len() == 1 { "" } else { "s" }
    );
    for r in failed {
        let why = if r.timed_out { " (timed out)" } else { "" };
        out.push_str(&format!("\n`{}`{why}:\n```\n{}\n```\n", r.command, r.tail));
    }
    out
}

/// What Runo's checks showed after the session's last turn, for a reviewer's guide.
pub fn tested_note(proof: &Proof) -> String {
    if !proof.skipped.is_empty() {
        return format!("Runo's checks didn't run: {}", proof.skipped);
    }
    let mut lines: Vec<String> = proof
        .results
        .iter()
        .map(|r| {
            let verdict = if r.timed_out {
                "timed out"
            } else if r.ok {
                "passed"
            } else {
                "failed"
            };
            format!("`{}` {verdict} ({}s)", r.command, r.secs)
        })
        .collect();
    if !proof.review.is_empty() {
        lines.push(format!("Second opinion: {}", proof.review));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "Runo ran the project's checks after the agent's last turn on this branch:\n- {}",
        lines.join("\n- ")
    )
}

/// Ask another agent to review the turn's change, read-only, one note per line.
pub fn review_prompt(task: &str, patch: &str) -> String {
    let task: String = task.chars().take(2000).collect();
    format!(
        "You are giving a second opinion on another coding agent's change in this folder. \
         Do not edit any files. Read the diff below, and open files here when you need more \
         context. Report only real problems: bugs, missed cases, broken behaviour, security \
         issues, or code that does not do what was asked. Skip style and naming.\n\n\
         Reply with one finding per line, exactly in this form:\n\
         NOTE path/to/file:LINE: what is wrong and how to fix it\n\
         LINE is the line number in the new version of the file. If nothing needs changing, \
         reply with exactly: NO NOTES\n\n\
         The agent was asked:\n<task>\n{task}\n</task>\n\n<diff>\n{patch}\n</diff>"
    )
}

/// One finding from a second opinion.
#[derive(Clone, PartialEq, Debug)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// The `NOTE path:line: text` lines of a second opinion's reply.
pub fn parse_review(reply: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for raw in reply.lines() {
        let line = raw.trim().trim_start_matches(['-', '*', '`', ' ', '>']);
        let Some(rest) = line.strip_prefix("NOTE") else {
            continue;
        };
        let rest = rest.trim_start_matches([':', ' ']).trim_end_matches('`');
        // The first `:digits:` splits the path from the line and the text.
        let found = rest.match_indices(':').find_map(|(i, _)| {
            let after = &rest[i + 1..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                return None;
            }
            let more = &after[digits.len()..];
            let text = more.strip_prefix(':')?;
            Some((i, digits.parse::<usize>().ok()?, text.trim()))
        });
        let Some((i, line_no, text)) = found else {
            continue;
        };
        let path = rest[..i].trim().trim_matches('`').trim_start_matches("./");
        if path.is_empty() || text.is_empty() {
            continue;
        }
        out.push(Finding {
            path: path.to_string(),
            line: line_no,
            text: text.to_string(),
        });
    }
    out
}

/// Agents preferred for a second opinion, best first.
const REVIEWERS: [&str; 10] = [
    "codex", "claude", "gemini", "cursor", "opencode", "amp", "copilot", "droid", "qwen", "grok",
];

/// Another vendor's agent to review a change made by `own`, among the ones that can run.
pub fn pick_reviewer(own: &str, available: impl Fn(&str) -> bool) -> Option<&'static str> {
    REVIEWERS
        .iter()
        .copied()
        .find(|p| *p != own && available(p))
}

/// What a second opinion found, for the card.
pub fn review_summary(name: &str, findings: usize, said_none: bool) -> String {
    match findings {
        0 if said_none => format!("{name} found nothing to change."),
        0 => format!("{name} replied without notes Runo could place."),
        1 => format!("{name} left 1 note on the diff."),
        n => format!("{name} left {n} notes on the diff."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a real `cargo build` of a tiny crate, so it needs cargo and must run without
    /// `CARGO_TARGET_DIR` (which `cargo test` passes on).
    #[test]
    #[ignore = "env -u CARGO_TARGET_DIR <test binary> rust_checks_in_a_worktree --ignored"]
    fn rust_checks_in_a_worktree_build_into_the_main_checkout() {
        let base = std::env::temp_dir().join(format!("runo-proof-cargo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let main = base.join("app");
        let tree = crate::git::worktree_path(main.to_str().unwrap(), "runo/x");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(tree.join("src")).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: elsewhere\n").unwrap();
        std::fs::write(
            tree.join("Cargo.toml"),
            "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(tree.join("src/main.rs"), "fn main() {}\n").unwrap();
        let runs = run_all(
            tree.to_str().unwrap(),
            &["cargo build --offline".into()],
            Duration::from_secs(120),
        );
        assert!(runs[0].ok, "{}", runs[0].tail);
        assert!(!tree.join("target").exists());
        assert!(main.join("target/debug").is_dir());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_guide_hears_how_the_checks_went() {
        let mut proof = Proof {
            results: vec![
                CheckRun {
                    command: "cargo test".into(),
                    ok: true,
                    secs: 47,
                    ..Default::default()
                },
                CheckRun {
                    command: "cargo clippy".into(),
                    ok: false,
                    secs: 12,
                    ..Default::default()
                },
            ],
            review: "Codex left 2 notes".into(),
            ..Default::default()
        };
        let note = tested_note(&proof);
        assert!(note.contains("`cargo test` passed (47s)"));
        assert!(note.contains("`cargo clippy` failed (12s)"));
        assert!(note.contains("Second opinion: Codex left 2 notes"));
        proof.results.clear();
        proof.review.clear();
        assert_eq!(tested_note(&proof), "");
        proof.skipped = "the session runs on a connection".into();
        assert!(tested_note(&proof).contains("didn't run"));
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-proof-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn commands_skip_blanks_and_comments() {
        assert_eq!(
            commands("npm test\n\n  # later\n cargo test \n"),
            vec!["npm test", "cargo test"]
        );
    }

    #[test]
    fn checks_are_found_from_manifests() {
        let dir = temp("detect");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"build":"vite build","test":"vitest","lint":"eslint .","dev":"vite"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(
            detect(&dir),
            vec![
                "pnpm run lint",
                "pnpm run test",
                "pnpm run build",
                "cargo test"
            ]
        );
        std::fs::remove_file(dir.join("pnpm-lock.yaml")).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        std::fs::remove_file(dir.join("Cargo.toml")).unwrap();
        assert!(detect(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn makefile_targets_are_used_when_nothing_else_is_found() {
        let dir = temp("make");
        std::fs::write(
            dir.join("Makefile"),
            "build:\n\tcc x.c\ntest: build\n\t./t\n",
        )
        .unwrap();
        assert_eq!(detect(&dir), vec!["make test"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn checks_run_and_report_failures_with_their_output() {
        let dir = temp("run");
        let d = dir.to_string_lossy().to_string();
        let runs = run_all(
            &d,
            &["echo fine".into(), "echo broke >&2; exit 3".into()],
            Duration::from_secs(20),
        );
        assert!(runs[0].ok);
        assert_eq!(runs[0].tail, "fine");
        assert!(!runs[1].ok);
        assert_eq!(runs[1].tail, "broke");
        let timed = run_one(&d, "sleep 5", Duration::from_millis(300));
        assert!(!timed.ok && timed.timed_out);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tails_keep_the_end() {
        let text: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let t = tail(&text);
        assert!(t.starts_with("line 61") && t.ends_with("line 100"));
    }

    #[test]
    fn the_same_failure_is_recognised_across_runs() {
        let a = vec![CheckRun {
            command: "cargo test".into(),
            ok: false,
            tail: "test x ... FAILED\nfinished in 1.23s".into(),
            ..Default::default()
        }];
        let mut b = a.clone();
        b[0].tail = "test x ... FAILED\nfinished in 4.56s".into();
        assert_eq!(failure_key(&a), failure_key(&b));
        b[0].tail = "test y ... FAILED".into();
        assert_ne!(failure_key(&a), failure_key(&b));
    }

    #[test]
    fn fix_prompts_name_each_failing_check() {
        let runs = vec![
            CheckRun {
                command: "npm test".into(),
                ok: false,
                tail: "1 failed".into(),
                ..Default::default()
            },
            CheckRun {
                command: "npm run lint".into(),
                ok: true,
                ..Default::default()
            },
        ];
        let p = fix_prompt(&runs);
        assert!(p.contains("`npm test`") && p.contains("1 failed"));
        assert!(!p.contains("npm run lint"));
    }

    #[test]
    fn review_notes_are_parsed() {
        let reply = "Here is what I found:\n\
            NOTE src/app.ts:42: user can be null here; check it first\n\
            - `NOTE ./lib/a b.rs:7: off by one in the loop`\n\
            NOTE no line here\n\
            NOTE C:/x/y.rs:3: windows path\n";
        let found = parse_review(reply);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].path, "src/app.ts");
        assert_eq!(found[0].line, 42);
        assert_eq!(found[0].text, "user can be null here; check it first");
        assert_eq!(found[1].path, "lib/a b.rs");
        assert_eq!(found[1].text, "off by one in the loop");
        assert_eq!(found[2].path, "C:/x/y.rs");
        assert!(parse_review("NO NOTES").is_empty());
    }

    #[test]
    fn reviewers_come_from_another_vendor() {
        assert_eq!(pick_reviewer("claude", |_| true), Some("codex"));
        assert_eq!(pick_reviewer("codex", |_| true), Some("claude"));
        assert_eq!(pick_reviewer("codex", |p| p == "gemini"), Some("gemini"));
        assert_eq!(pick_reviewer("claude", |p| p == "claude"), None);
    }
}
