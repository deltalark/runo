//! App-wide popups: permission requests, questions and plans from any session, and
//! agent CLI updates. They stack in the corner so the person can answer a session
//! without opening it.

use super::*;

/// How often installed agents are checked for a newer release.
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// A newer release of an installed agent CLI.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentUpdate {
    pub provider: &'static str,
    pub current: String,
    pub latest: String,
}

/// An agent update or install while it runs, briefly after it succeeds, and after it
/// fails until dismissed.
pub(crate) struct UpdateRun {
    /// An install of a missing agent, rather than an update.
    pub install: bool,
    pub started: std::time::Instant,
    /// The newest line the updater printed.
    pub line: String,
    /// A percentage the updater printed itself, when it prints one.
    pub percent: Option<f32>,
    /// How far the updater's own messages say it got (downloading, installing, done).
    pub phase: f32,
    /// The installer's current step, as (step, of, label), when its output shows it.
    pub stage: Option<(u8, u8, &'static str)>,
    /// Packages npm has downloaded so far.
    pub fetched: u32,
    pub finished: Option<std::time::Instant>,
    /// Why it failed or is waiting on the person, and what they can do.
    pub failed: Option<updater::Failure>,
    /// The command that runs, as the person could type it.
    pub command: String,
    /// Where the updated CLI lands, when the updater says (npm with a prefix).
    pub installs_to: Option<std::path::PathBuf>,
}

impl UpdateRun {
    pub(crate) fn new(install: bool, first_line: String) -> Self {
        UpdateRun {
            install,
            started: std::time::Instant::now(),
            line: first_line,
            percent: None,
            phase: 0.,
            stage: None,
            fetched: 0,
            finished: None,
            failed: None,
            command: String::new(),
            installs_to: None,
        }
    }

    /// A card that waits on the person instead of running anything.
    pub(crate) fn blocked(install: bool, failure: updater::Failure) -> Self {
        let mut run = UpdateRun::new(install, String::new());
        run.failed = Some(failure);
        run
    }

    pub(crate) fn running(&self) -> bool {
        self.finished.is_none() && self.failed.is_none()
    }
}

/// The log file npm points to when it fails ("A complete log of this run can be found in: …").
pub(crate) fn log_path(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let (_, path) = l.split_once("can be found in:")?;
        Some(path.trim().to_string()).filter(|p| !p.is_empty())
    })
}

/// Why an update or install failed, in words a person can act on.
/// The general case; `updater::explain` handles the errors it recognises first.
pub(crate) fn failure_summary(output: &str) -> String {
    let noise = |l: &str| {
        let l = l.trim();
        l.is_empty()
            || l.contains("complete log of this run")
            || l.contains(" http fetch ")
            || l.trim_start_matches("npm")
                .trim_start_matches(" error")
                .trim_start_matches(" ERR!")
                .trim()
                .is_empty()
    };
    let detail = |l: &str| {
        l.trim()
            .trim_start_matches("npm error")
            .trim_start_matches("npm ERR!")
            .trim()
            .to_string()
    };
    let errors: Vec<&str> = output
        .lines()
        .filter(|l| (l.contains("npm error") || l.contains("npm ERR!")) && !noise(l))
        .collect();
    // npm's first error lines are "code X" and the message; prefer the message.
    errors
        .iter()
        .find(|l| !detail(l).starts_with("code ") && !detail(l).starts_with("errno "))
        .or(errors.first())
        .map(|l| detail(l))
        .or_else(|| {
            output
                .lines()
                .rev()
                .find(|l| !noise(l))
                .map(|l| l.trim().to_string())
        })
        .unwrap_or_else(|| "It stopped without saying why.".into())
}

/// How long the success badge stays before the card hides itself.
pub(crate) const UPDATE_DONE_SHOWN: Duration = Duration::from_millis(2600);

impl UpdateRun {
    /// Fraction done for the bar: the updater's own percentage when it prints one, else
    /// the furthest phase it reported, eased forward by elapsed time so it never stalls.
    pub(crate) fn progress(&self, now: std::time::Instant) -> f32 {
        if self.finished.is_some() {
            return 1.;
        }
        if let Some(p) = self.percent {
            return (p / 100.).clamp(0.02, 0.98);
        }
        let t = now.duration_since(self.started).as_secs_f32();
        let timed = 0.85 * (1. - (-t / 25.).exp());
        timed.max(self.phase).clamp(0.03, 0.97)
    }

    fn take_line(&mut self, line: &str) {
        if let Some(p) = percent_of(line) {
            self.percent = Some(p);
        }
        if let Some(phase) = phase_of(line) {
            self.phase = self.phase.max(phase);
        }
        if line.contains("http fetch") {
            self.fetched += 1;
        }
        if let Some((step, of, label)) = stage_of(&self.command, line) {
            if self.stage.is_none_or(|(at, _, _)| step >= at) {
                self.stage = Some((step, of, label));
                // The bar reaches the middle of the step the installer is on.
                self.phase = self.phase.max((step as f32 - 0.5) / of as f32);
            }
        }
        self.line = line.chars().take(160).collect();
    }
}

/// A percentage printed in a progress line, like "Downloading 42%" or "[####] 42.5 %".
fn percent_of(line: &str) -> Option<f32> {
    let at = line.rfind('%')?;
    let digits: String = line[..at]
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits
        .parse::<f32>()
        .ok()
        .filter(|p| (0. ..=100.).contains(p))
}

/// Which step an installer is on, from its own messages: Homebrew's `==>` headings,
/// npm's fetch and install lines, or the words a vendor installer prints.
fn stage_of(command: &str, line: &str) -> Option<(u8, u8, &'static str)> {
    let l = line.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    if command.starts_with("brew") {
        if !l.starts_with("==>") && !l.contains('🍺') {
            return None;
        }
        return if has(&["summary", "caveats", "🍺"]) {
            Some((5, 5, "Finishing"))
        } else if has(&["pouring", "linking"]) {
            Some((4, 5, "Linking"))
        } else if has(&["upgrading", "installing"]) {
            Some((3, 5, "Installing"))
        } else if has(&["fetching", "downloading"]) {
            Some((2, 5, "Downloading"))
        } else if has(&["updating homebrew", "auto-updat"]) {
            Some((1, 5, "Updating Homebrew"))
        } else {
            None
        };
    }
    if command.contains("npm") {
        return if has(&["added ", "changed ", "up to date"]) {
            Some((3, 3, "Finishing"))
        } else if has(&["reify", "postinstall", "build", "linking"]) {
            Some((2, 3, "Installing"))
        } else if has(&["http fetch"]) {
            Some((1, 3, "Downloading"))
        } else {
            None
        };
    }
    if has(&["successfully", "complete", "up to date", "installed"]) {
        Some((3, 3, "Finishing"))
    } else if has(&["install", "extract", "unpack", "verif"]) {
        Some((2, 3, "Installing"))
    } else if has(&["download", "fetch"]) {
        Some((1, 3, "Downloading"))
    } else {
        None
    }
}

/// How far along an updater is, from the words in its output.
fn phase_of(line: &str) -> Option<f32> {
    let l = line.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    if has(&[
        "added ",
        "changed ",
        "updated",
        "successfully",
        "up to date",
        "complete",
    ]) {
        Some(0.95)
    } else if has(&["verif", "linking", "postinstall", "reify"]) {
        Some(0.8)
    } else if has(&["extract", "unpack", "installing"]) {
        Some(0.6)
    } else if has(&["download", "fetch", "get 200", "get 304"]) {
        Some(0.35)
    } else {
        None
    }
}

#[cfg(test)]
mod stage_tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn installers_report_their_step() {
        let brew = "brew upgrade gemini-cli";
        assert_eq!(
            stage_of(brew, "==> Fetching downloads for: gemini-cli"),
            Some((2, 5, "Downloading"))
        );
        assert_eq!(
            stage_of(
                brew,
                "==> Pouring gemini-cli--0.47.0.arm64_sequoia.bottle.tar.gz"
            ),
            Some((4, 5, "Linking"))
        );
        assert_eq!(
            stage_of(
                brew,
                "🍺  /opt/homebrew/Cellar/gemini-cli/0.47.0: 9,001 files"
            ),
            Some((5, 5, "Finishing"))
        );
        assert_eq!(stage_of(brew, "Removing: /opt/homebrew/Cellar/x"), None);
        let npm = "npm install -g @google/gemini-cli@latest";
        assert_eq!(
            stage_of(
                npm,
                "npm http fetch GET 200 https://registry.npmjs.org/x 12ms"
            ),
            Some((1, 3, "Downloading"))
        );
        assert_eq!(
            stage_of(npm, "added 412 packages in 9s"),
            Some((3, 3, "Finishing"))
        );
        let mut run = UpdateRun::new(false, String::new());
        run.command = npm.into();
        run.take_line("npm http fetch GET 200 https://registry.npmjs.org/a 3ms");
        run.take_line("npm http fetch GET 200 https://registry.npmjs.org/b 3ms");
        assert_eq!((run.fetched, run.stage), (2, Some((1, 3, "Downloading"))));
    }
}

/// A line of updater output without colour codes or spinner characters.
fn clean_line(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip an escape sequence: ESC [ ... final letter, or ESC and one character.
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if !c.is_control() && !('\u{2800}'..='\u{28ff}').contains(&c) {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Run an updater, sending each line it prints as `<key>-line` and the result as `<key>`.
pub(crate) fn run_streaming(key: String, mut command: std::process::Command, tx: Sender<Event>) {
    use std::io::Read;
    use std::process::Stdio;
    command
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    std::thread::spawn(move || {
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = tx.send(Event::Output(
                    key,
                    Err(format!(
                        "could not start {:?}: {error}",
                        command.get_program()
                    )),
                ));
                return;
            }
        };
        let tail = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let mut readers = Vec::new();
        let streams: Vec<Box<dyn Read + Send>> = [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .collect();
        for mut stream in streams {
            let (tx, tail, line_key) = (tx.clone(), tail.clone(), format!("{key}-line"));
            readers.push(std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut pending = Vec::new();
                let emit = |bytes: &[u8]| {
                    let line = clean_line(&String::from_utf8_lossy(bytes));
                    if line.is_empty() {
                        return;
                    }
                    if let Ok(mut tail) = tail.lock() {
                        tail.push(line.clone());
                        let extra = tail.len().saturating_sub(30);
                        tail.drain(..extra);
                    }
                    let _ = tx.send(Event::Output(line_key.clone(), Ok(line)));
                };
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    for &b in &buf[..n] {
                        // Progress bars redraw with \r, so treat it as a line end too.
                        if b == b'\n' || b == b'\r' {
                            emit(&pending);
                            pending.clear();
                        } else {
                            pending.push(b);
                        }
                    }
                }
                emit(&pending);
            }));
        }
        for reader in readers {
            let _ = reader.join();
        }
        let ok = child.wait().is_ok_and(|status| status.success());
        let text = tail.lock().map(|t| t.join("\n")).unwrap_or_default();
        let _ = tx.send(Event::Output(key, if ok { Ok(text) } else { Err(text) }));
    });
}

/// A finished card: hold the success badge, then fade out just before it is removed.
fn fade_after_badge(card: AnyElement, id: String) -> AnyElement {
    let total = UPDATE_DONE_SHOWN.as_secs_f32();
    div().child(card).animated(
        SharedString::from(id),
        Animation::new(UPDATE_DONE_SHOWN),
        move |el, d| {
            let fade_from = 1. - 0.5 / total;
            el.opacity(if d < fade_from {
                1.
            } else {
                1. - (d - fade_from) / (1. - fade_from)
            })
        },
    )
}

/// The green pill shown when an update or install finishes.
fn update_done_badge(n: usize, label: &'static str) -> AnyElement {
    let pill = div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .h(px(24.))
        .rounded_full()
        .bg(rgb(settings::mix(c_green(), c_raised(), 0.18)))
        .border_1()
        .border_color(rgb(settings::mix(c_green(), c_border(), 0.5)))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(c_green()))
        .child(icon(IconName::Check, 12., c_green()))
        .child(label);
    pill.animated(
        ("up-done", n),
        Animation::new(Duration::from_millis(420)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d).ml(px(8. * (1. - d))),
    )
}

/// Something a session is waiting on the person for.
pub(crate) enum Waiting {
    Denied(Vec<String>),
    Question(store::Question),
    Plan,
}

/// The npm package each agent CLI ships as, for looking up its latest release.
pub(crate) fn npm_package(provider: &str) -> Option<&'static str> {
    Some(match provider {
        "claude" => "@anthropic-ai/claude-code",
        "codex" => "@openai/codex",
        "opencode" => "opencode-ai",
        "gemini" => "@google/gemini-cli",
        "copilot" => "@github/copilot",
        "amp" => "@sourcegraph/amp",
        "qwen" => "@qwen-code/qwen-code",
        _ => return None,
    })
}

/// The numeric parts of a version such as "v2.1.296 (Claude Code)" or "codex-cli 0.50.1".
fn version_parts(v: &str) -> Vec<u64> {
    let start = v.find(|c: char| c.is_ascii_digit()).unwrap_or(v.len());
    v[start..]
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()
        .unwrap_or("")
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// Whether `latest` is a newer release than `current`.
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    let (l, c) = (version_parts(latest), version_parts(current));
    !l.is_empty() && !c.is_empty() && l > c
}

/// What the latest message of a session asks of the person, if anything.
pub(crate) fn waiting_on(session: &Session) -> Option<(usize, Waiting)> {
    let index = session.messages.len().checked_sub(1)?;
    let m = &session.messages[index];
    if m.role != Role::Agent {
        return None;
    }
    if !m.denied.is_empty() {
        Some((index, Waiting::Denied(m.denied.clone())))
    } else if let Some(q) = m.questions.first().filter(|q| !q.options.is_empty()) {
        Some((index, Waiting::Question(q.clone())))
    } else if !m.plan.trim().is_empty() {
        Some((index, Waiting::Plan))
    } else {
        None
    }
}

/// Join names as "A", "A and B", "A, B and C".
pub(crate) fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl RunoApp {
    /// Look up the latest release of every installed agent, in the background, once
    /// their installed versions are known.
    pub(crate) fn check_updates(&mut self) {
        self.update_checked = Some(std::time::Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // Discovery runs at start; give it a moment to read installed versions.
            let start = std::time::Instant::now();
            let ready = || {
                agent::PROVIDERS
                    .iter()
                    .all(|p| !agent::installed(p.bin) || !caps::get(p.id).version.is_empty())
            };
            while !ready() && start.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(500));
            }
            let mut found = Vec::new();
            for p in agent::PROVIDERS.iter() {
                let current = caps::get(p.id).version;
                let Some(package) = npm_package(p.id) else {
                    continue;
                };
                if current.is_empty() {
                    continue;
                }
                let out = std::process::Command::new("curl")
                    .args([
                        "-sL",
                        "-m",
                        "8",
                        &format!("https://registry.npmjs.org/{package}/latest"),
                    ])
                    .output();
                let latest = out
                    .ok()
                    .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
                    .and_then(|v| v["version"].as_str().map(str::to_string))
                    .unwrap_or_default();
                if is_newer(&latest, &current) {
                    found.push(format!("{}\t{current}\t{latest}", p.id));
                }
            }
            let _ = tx.send(Event::Output("updates".into(), Ok(found.join("\n"))));
        });
    }

    /// Re-check for updates now and then while the app stays open.
    pub(crate) fn maybe_check_updates(&mut self) {
        if self
            .update_checked
            .is_none_or(|at| at.elapsed() > UPDATE_EVERY)
        {
            self.check_updates();
        }
    }

    /// Update the exact copy of an agent Runo runs, the way it was installed. When that
    /// isn't safe (another app owns it, it's busy), the card explains instead.
    pub(crate) fn run_update(
        &mut self,
        provider: &'static str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let p = agent::provider(provider);
        let bin = caps::binary(p);
        let blocked = |failure: updater::Failure| UpdateRun::blocked(false, failure);
        let Some(runs) = updater::resolve(&bin) else {
            self.update_runs.insert(
                provider,
                blocked(updater::Failure {
                    why: format!("Runo can't find `{bin}` any more. Re-detect agents in Settings, or set its path there."),
                    needs_you: true,
                    ..Default::default()
                }),
            );
            cx.notify();
            return;
        };
        let busy = self
            .started
            .keys()
            .filter(|id| {
                self.state
                    .session(**id)
                    .is_some_and(|s| s.provider == provider)
            })
            .count();
        if busy > 0 && !force {
            self.update_runs.insert(
                provider,
                blocked(updater::Failure {
                    why: format!(
                        "{} is working in {busy} session{} right now. Updating replaces its files, which can break those runs. Update when they finish, or update anyway.",
                        p.name,
                        if busy == 1 { "" } else { "s" }
                    ),
                    needs_you: true,
                    force: true,
                    ..Default::default()
                }),
            );
            cx.notify();
            return;
        }
        let real = std::fs::canonicalize(&runs).unwrap_or_else(|_| runs.clone());
        match updater::plan(provider, p.name, &runs, &real) {
            updater::Plan::Manual { why, command } => {
                // Offer a newer copy that's already on this computer, if there is one.
                // Another copy newer than this one (it gets updated after switching).
                let current = self
                    .updates
                    .iter()
                    .find(|u| u.provider == provider)
                    .map(|u| u.current.clone())
                    .unwrap_or_default();
                let mut copies: Vec<(String, std::path::PathBuf)> =
                    updater::other_copies(p.bin, &runs)
                        .into_iter()
                        .take(4)
                        .map(|path| (updater::version_of(&path), path))
                        .filter(|(v, _)| !v.is_empty() && is_newer(v, &current))
                        .collect();
                copies.sort_by_key(|(v, _)| std::cmp::Reverse(version_parts(v)));
                let use_copy = copies
                    .into_iter()
                    .map(|(v, path)| (path.to_string_lossy().into_owned(), v))
                    .next();
                let why = match &use_copy {
                    Some((path, version)) => format!(
                        "{why} You also have {} {} at {path}. Runo can switch to it and keep that one updated.",
                        p.name,
                        version_parts(version)
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                            .join(".")
                    ),
                    None => why,
                };
                self.update_runs.insert(
                    provider,
                    blocked(updater::Failure {
                        why,
                        command,
                        use_copy,
                        needs_you: true,
                        ..Default::default()
                    }),
                );
            }
            updater::Plan::Run {
                program,
                installs_to,
            } => {
                let mut cmd = std::process::Command::new(&program[0]);
                cmd.args(&program[1..]);
                let shown = program.join(" ");
                let mut run = UpdateRun::new(false, format!("Running {shown}…"));
                run.command = shown;
                run.installs_to = installs_to;
                self.update_runs.insert(provider, run);
                run_streaming(format!("update:{provider}"), cmd, self.tx.clone());
            }
        }
        cx.notify();
    }

    /// Hide update cards whose success badge has been shown long enough.
    pub(crate) fn prune_update_runs(&mut self) -> bool {
        let done: Vec<&'static str> = self
            .update_runs
            .iter()
            .filter(|(_, run)| {
                run.finished
                    .is_some_and(|at| at.elapsed() > UPDATE_DONE_SHOWN)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &done {
            if self.update_runs.remove(id).is_some_and(|run| !run.install) {
                self.updates.retain(|u| u.provider != *id);
            }
        }
        !done.is_empty()
    }

    /// Results of update checks and update runs.
    pub(crate) fn handle_alert_job(&mut self, key: &str, result: Result<String, String>) -> bool {
        if let Some(provider) = key
            .strip_prefix("update:")
            .or_else(|| key.strip_prefix("install:"))
            .and_then(|k| k.strip_suffix("-line"))
        {
            if let (Some(run), Ok(line)) = (self.update_runs.get_mut(provider), result) {
                run.take_line(&line);
            }
            return true;
        }
        if let Some(provider) = key.strip_prefix("install:") {
            self.finish_install(provider, result);
            return true;
        }
        if key == "updates" {
            let text = result.unwrap_or_default();
            self.updates = text
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split('\t');
                    let id = parts.next()?;
                    let provider = agent::PROVIDERS.iter().find(|p| p.id == id)?.id;
                    Some(AgentUpdate {
                        provider,
                        current: parts.next()?.to_string(),
                        latest: parts.next()?.to_string(),
                    })
                })
                .collect();
            return true;
        }
        let (provider, verified) = if let Some(p) = key.strip_prefix("update-check:") {
            (p, Some(result.clone().unwrap_or_default()))
        } else if let Some(p) = key.strip_prefix("update:") {
            (p, None)
        } else {
            return false;
        };
        let Some(provider) = agent::PROVIDERS
            .iter()
            .find(|p| p.id == provider)
            .map(|p| p.id)
        else {
            return true;
        };
        let name = agent::provider(provider).name;
        let latest = self
            .updates
            .iter()
            .find(|u| u.provider == provider)
            .map(|u| u.latest.clone())
            .unwrap_or_default();
        if let Some(found) = verified {
            // The updater said it worked: check the copy Runo actually runs.
            let Some(run) = self.update_runs.get_mut(provider) else {
                return true;
            };
            if !found.is_empty() && is_newer(&latest, &found) {
                let runs = updater::resolve(&caps::binary(agent::provider(provider)))
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let fresh = run
                    .installs_to
                    .as_ref()
                    .filter(|p| p.is_file())
                    .map(|p| (p.display().to_string(), updater::version_of(p)))
                    .filter(|(_, v)| !v.is_empty() && !is_newer(&latest, v));
                let now = version_parts(&found)
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(".");
                let package = npm_package(provider);
                let why = if run.command.starts_with("brew") {
                    // Homebrew updated the copy Runo runs; its formula just lags the release.
                    format!(
                        "Homebrew finished, but its newest {name} is still {now} and {latest} is out. Homebrew usually catches up within a few days. To get {latest} now, install it from npm with the command below, then pick that copy under Settings › {name}."
                    )
                } else if fresh.is_some() {
                    format!(
                        "The update finished, but Runo still runs {runs}, which is {now}. Another copy comes first on your PATH. Switch Runo to the new copy below."
                    )
                } else {
                    format!(
                        "The updater finished, but {runs} still reports {now}, not {latest}. The release may not have reached this installer yet."
                    )
                };
                let command = (run.command.starts_with("brew") || fresh.is_none())
                    .then(|| package.map(|p| format!("npm install -g {p}@latest")))
                    .flatten();
                run.failed = Some(updater::Failure {
                    why,
                    command,
                    use_copy: fresh,
                    needs_you: true,
                    ..Default::default()
                });
            } else {
                // The card turns into a success badge, then hides itself.
                run.finished = Some(std::time::Instant::now());
                self.state.log(0, format!("Updated {name} to {latest}"));
                caps::refresh();
            }
            return true;
        }
        match result {
            Ok(_) => {
                if let Some(run) = self.update_runs.get_mut(provider) {
                    run.line = "Checking the version Runo runs now…".into();
                    run.phase = 0.97;
                }
                let bin = caps::binary(agent::provider(provider));
                let mut check = std::process::Command::new(&bin);
                check.arg("--version");
                agent::oneshot(
                    format!("update-check:{provider}"),
                    check,
                    &std::env::temp_dir().to_string_lossy(),
                    self.tx.clone(),
                );
            }
            Err(error) => {
                // The card stays and says what went wrong, and what to do about it.
                let command = self
                    .update_runs
                    .get(provider)
                    .map(|r| r.command.clone())
                    .unwrap_or_default();
                let failure = updater::explain(&error, &command);
                self.state
                    .log(0, format!("Updating {name} failed: {}", failure.why));
                if let Some(run) = self.update_runs.get_mut(provider) {
                    run.failed = Some(failure);
                }
            }
        }
        true
    }

    /// Send a reply to any session, not just the one on screen.
    pub(crate) fn answer_session(
        &mut self,
        id: u64,
        prompt: String,
        allow: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.handles.contains_key(&Target::Session(id)) || self.demo_guard(cx) {
            return;
        }
        let Some(provider) = self.state.session(id).map(|s| s.provider.clone()) else {
            return;
        };
        let mut opts = cx.global::<AppSettings>().agent(&provider);
        if !allow.is_empty() {
            let mut tools: Vec<String> = opts
                .allowed_tools
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            tools.extend(allow);
            opts.allowed_tools = tools.join(",");
        }
        self.start_turn(id, prompt, Vec::new(), &opts);
    }

    /// Sessions waiting on the person, except the one already on screen.
    fn waiting_sessions(&self) -> Vec<(u64, usize, Waiting)> {
        let on_screen = (self.view == View::Session)
            .then_some(self.selected)
            .flatten();
        self.state
            .sessions
            .iter()
            .filter(|s| !s.archived && Some(s.id) != on_screen)
            .filter(|s| !self.handles.contains_key(&Target::Session(s.id)))
            .filter_map(|s| waiting_on(s).map(|(i, w)| (s.id, i, w)))
            .filter(|(id, i, _)| !self.alerts_hidden.contains(&format!("s:{id}:{i}")))
            .collect()
    }

    /// The popups in the corner of the window.
    pub(crate) fn alerts_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.demo || self.view == View::Settings || self.tour.is_some() {
            return None;
        }
        let mut cards: Vec<AnyElement> = Vec::new();
        for (n, (id, index, waiting)) in self.waiting_sessions().into_iter().enumerate() {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let key = format!("s:{id}:{index}");
            let (accent, glyph, title, body) = match &waiting {
                Waiting::Denied(tools) => (
                    c_amber(),
                    IconName::ShieldAlert,
                    "Permission needed",
                    format!(
                        "{} wants to use {} and is paused until you allow it.",
                        provider.name,
                        and_list(tools)
                    ),
                ),
                Waiting::Question(q) => (
                    c_blue(),
                    IconName::CircleQuestionMark,
                    "Question",
                    q.question.clone(),
                ),
                Waiting::Plan => (
                    c_blue(),
                    IconName::ListChecks,
                    "Plan ready for review",
                    format!(
                        "{} proposed a plan and waits for your go-ahead.",
                        provider.name
                    ),
                ),
            };
            let mut actions = div().flex().flex_wrap().gap_2();
            match waiting {
                Waiting::Denied(tools) => {
                    let (once, always) = (tools.clone(), tools);
                    actions = actions
                        .child(
                            self.button(("al-once", n), "Allow once", false)
                                .h(px(26.))
                                .child(icon(IconName::Check, 12., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer_session(
                                        id,
                                        format!(
                                            "You now have permission to use {}. Continue.",
                                            once.join(", ")
                                        ),
                                        once.clone(),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button(("al-always", n), "Always allow", true)
                                .h(px(26.))
                                .child(icon(IconName::ShieldCheck, 12., c_canvas()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(provider) =
                                        this.state.session(id).map(|s| s.provider.clone())
                                    {
                                        this.allow_tools_for(&provider, &always, cx);
                                    }
                                    this.answer_session(
                                        id,
                                        format!(
                                            "You now have permission to use {}. Continue.",
                                            always.join(", ")
                                        ),
                                        Vec::new(),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        );
                }
                Waiting::Question(q) => {
                    for (oi, option) in q.options.iter().take(4).enumerate() {
                        let answer = format!("My answer: {} {option}", q.question);
                        actions = actions.child(
                            self.chip(("al-opt", n * 10 + oi), option.clone(), false)
                                .h(px(26.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer_session(id, answer.clone(), Vec::new(), cx);
                                    cx.notify();
                                })),
                        );
                    }
                }
                Waiting::Plan => {
                    actions = actions.child(
                        self.button(("al-plan", n), "Approve and build", true)
                            .h(px(26.))
                            .child(icon(IconName::Play, 12., c_canvas()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(s) = this.state.session_mut(id) {
                                    s.opts.insert("mode".into(), "edit".into());
                                }
                                this.answer_session(
                                    id,
                                    "The plan is approved. Go ahead and implement it.".into(),
                                    Vec::new(),
                                    cx,
                                );
                                cx.notify();
                            })),
                    );
                }
            }
            cards.push(self.alert_card(
                SharedString::from(format!("alert-{key}")),
                accent,
                glyph,
                title,
                Some(session.title.clone()),
                Some(self.provider_badge(provider, 18.)),
                body,
                actions,
                key,
                Some(id),
                cx,
            ));
        }
        // Updates and installs in progress, finished or failed come first, so the one the
        // person just started is never hidden behind "more waiting".
        let mut run_cards = Vec::new();
        for (n, update) in self.updates.iter().enumerate() {
            let key = format!("u:{}:{}", update.provider, update.latest);
            let run = self.update_runs.get(update.provider).filter(|r| !r.install);
            let done = run.is_some_and(|r| r.finished.is_some());
            let failed = run.and_then(|r| r.failed.clone());
            if self.alerts_hidden.contains(&key) && !done {
                continue;
            }
            let provider = agent::provider(update.provider);
            let id = update.provider;
            let latest = update.latest.trim_start_matches('v').to_string();
            let current = version_parts(&update.current)
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(".");
            let (body, actions) = match run {
                Some(_) if done => (
                    format!("{} is now on version {latest}.", provider.name),
                    div().flex().child(update_done_badge(n, "Updated")),
                ),
                Some(_) if failed.is_some() => {
                    let failure = failed.clone().unwrap_or_default();
                    (
                        failure.why.clone(),
                        self.failure_actions(("up", n), id, failure, false, cx),
                    )
                }
                Some(run) => (
                    format!("Updating from {current} to {latest}."),
                    self.update_progress_view(n, run),
                ),
                None => (
                    format!("Version {latest} is out. You have {current}."),
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            self.button(("up-now", n), "Update now", true)
                                .h(px(26.))
                                .child(icon(IconName::Download, 12., c_canvas()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_update(id, false, cx)
                                })),
                        )
                        .child({
                            let later = key.clone();
                            self.button(("up-later", n), "Later", false)
                                .h(px(26.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.alerts_hidden.insert(later.clone());
                                    cx.notify();
                                }))
                        }),
                ),
            };
            let card = self.alert_card(
                SharedString::from(format!("alert-{key}")),
                match &failed {
                    Some(f) if f.needs_you => c_amber(),
                    Some(_) => c_red(),
                    None => c_green(),
                },
                if done {
                    IconName::CircleCheck
                } else if failed.is_some() {
                    IconName::TriangleAlert
                } else {
                    IconName::Download
                },
                if done {
                    "Update complete"
                } else if failed.as_ref().is_some_and(|f| f.needs_you) {
                    "Update needs you"
                } else if failed.is_some() {
                    "Update failed"
                } else if run.is_some() {
                    "Updating"
                } else {
                    "Update available"
                },
                Some(provider.name.to_string()),
                Some(self.provider_badge(provider, 18.)),
                body,
                actions,
                key.clone(),
                None,
                cx,
            );
            let target = if run.is_some() {
                &mut run_cards
            } else {
                &mut cards
            };
            target.push(if done {
                fade_after_badge(card, format!("up-out-{key}"))
            } else {
                card
            });
        }
        let mut installs: Vec<(&&'static str, &UpdateRun)> =
            self.update_runs.iter().filter(|(_, r)| r.install).collect();
        installs.sort_by_key(|(_, r)| r.started);
        for (n, (&id, run)) in installs.into_iter().enumerate() {
            let provider = agent::provider(id);
            let done = run.finished.is_some();
            let key = format!("i:{id}:{:?}", run.started);
            if self.alerts_hidden.contains(&key) && !done {
                continue;
            }
            let (body, actions) = if done {
                (
                    format!(
                        "Sign in once by running `{}` in a terminal, then pick it in New session.",
                        provider.bin
                    ),
                    div().flex().child(update_done_badge(n, "Installed")),
                )
            } else if let Some(failure) = run.failed.clone() {
                (
                    failure.why.clone(),
                    self.failure_actions(("in", n), id, failure, true, cx),
                )
            } else {
                (
                    "Downloading and setting up the CLI.".to_string(),
                    self.update_progress_view(n + 100, run),
                )
            };
            let card = self.alert_card(
                SharedString::from(format!("alert-{key}")),
                if run.failed.is_some() {
                    c_red()
                } else {
                    c_green()
                },
                if done {
                    IconName::CircleCheck
                } else if run.failed.is_some() {
                    IconName::TriangleAlert
                } else {
                    IconName::Download
                },
                if done {
                    "Installed"
                } else if run.failed.is_some() {
                    "Install failed"
                } else {
                    "Installing"
                },
                Some(provider.name.to_string()),
                Some(self.provider_badge(provider, 18.)),
                body,
                actions,
                key.clone(),
                None,
                cx,
            );
            run_cards.push(if done {
                fade_after_badge(card, format!("in-out-{key}"))
            } else {
                card
            });
        }
        run_cards.extend(cards);
        let cards = run_cards;
        if cards.is_empty() {
            return None;
        }
        let total = cards.len();
        let shown = if self.alerts_expanded { total } else { 3 };
        let mut stack = div()
            .absolute()
            .right_4()
            .bottom(px(36.))
            .w(px(380.))
            .max_w(relative(0.9))
            .v_flex()
            .gap_2()
            .occlude();
        if total > shown || self.alerts_expanded && total > 3 {
            let more = total.saturating_sub(shown);
            stack = stack.child(
                div()
                    .id("alerts-more")
                    .self_end()
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_raised()))
                    .shadow_md()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts_expanded = !this.alerts_expanded;
                        cx.notify();
                    }))
                    .child(if self.alerts_expanded {
                        "Show fewer".to_string()
                    } else {
                        format!("{more} more waiting")
                    }),
            );
        }
        let mut list = div().v_flex().gap_2();
        for card in cards.into_iter().take(shown) {
            list = list.child(card);
        }
        stack = stack.child(
            div()
                .id("alerts-scroll")
                .max_h(px(640.))
                .overflow_y_scroll()
                .child(list),
        );
        Some(stack.into_any_element())
    }

    /// What the person can do about a failed or blocked run: switch to a newer copy, copy
    /// a command, update anyway, retry, read the log, or dismiss.
    fn failure_actions(
        &self,
        id: (&'static str, usize),
        provider: &'static str,
        failure: updater::Failure,
        install: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let (tag, n) = id;
        let bid = |name: &str| (SharedString::from(format!("{tag}-{name}")), n);
        let mut primary_used = false;
        let mut row = div().flex().flex_wrap().gap_2();
        if let Some((path, version)) = failure.use_copy.clone() {
            primary_used = true;
            let label = format!(
                "Switch to {}",
                version_parts(&version)
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            );
            row = row.child(
                self.button(bid("use"), label, true)
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let path = path.clone();
                        settings::update(cx, |s| {
                            let mut options = s.agent(provider);
                            options.binary = path;
                            s.agents.insert(provider.into(), options);
                        });
                        caps::refresh();
                        this.update_runs.remove(provider);
                        let behind = this
                            .updates
                            .iter()
                            .any(|u| u.provider == provider && is_newer(&u.latest, &version));
                        if behind && !install {
                            // That copy is older than the latest too: update it now.
                            for u in this.updates.iter_mut().filter(|u| u.provider == provider) {
                                u.current = version.clone();
                            }
                            this.run_update(provider, false, cx);
                        } else {
                            for u in this.updates.iter_mut().filter(|u| u.provider == provider) {
                                u.latest = version.clone();
                            }
                            let mut run = UpdateRun::new(install, String::new());
                            run.finished = Some(std::time::Instant::now());
                            this.update_runs.insert(provider, run);
                        }
                        cx.notify();
                    })),
            );
        }
        if failure.force {
            primary_used = true;
            row = row.child(
                self.button(bid("force"), "Update anyway", true)
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_runs.remove(provider);
                        this.run_update(provider, true, cx);
                    })),
            );
        }
        if let Some(command) = failure.command.clone() {
            let first = !primary_used;
            primary_used = true;
            row = row.child(
                self.button(bid("copy"), "Copy command", first)
                    .h(px(26.))
                    .child(icon(
                        IconName::Copy,
                        12.,
                        if first { c_canvas() } else { c_secondary() },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                        this.info = Some((
                            "Copied. Paste it into a terminal.".into(),
                            std::time::Instant::now(),
                        ));
                        cx.notify();
                    })),
            );
        }
        if !failure.needs_you {
            row = row.child(
                self.button(bid("retry"), "Try again", !primary_used)
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_runs.remove(provider);
                        if install {
                            this.run_install(provider, cx);
                        } else {
                            this.run_update(provider, false, cx);
                        }
                    })),
            );
        }
        if let Some(log) = failure.log {
            row = row.child(
                self.button(bid("log"), "View log", false)
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_path(std::path::Path::new(&log));
                        cx.notify();
                    })),
            );
        }
        let row = row.child(
            self.button(bid("dismiss"), "Dismiss", false)
                .h(px(26.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.update_runs.remove(provider);
                    if !install {
                        this.updates.retain(|u| u.provider != provider);
                    }
                    cx.notify();
                })),
        );
        div()
            .v_flex()
            .gap_2()
            .w_full()
            .when_some(failure.command, |el, command| {
                el.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(tint(c_subtle()))
                        .border_1()
                        .border_color(rgb(c_border()))
                        .font_family(self.mono_font.clone())
                        .text_size(px(11.))
                        .child(command),
                )
            })
            .child(row)
    }

    /// The live progress of an update: a bar, what the updater is doing, and the time.
    fn update_progress_view(&self, n: usize, run: &UpdateRun) -> Div {
        let now = std::time::Instant::now();
        let fraction = run.progress(now);
        let secs = now.duration_since(run.started).as_secs();
        let elapsed = if secs < 60 {
            format!("{secs}s")
        } else {
            format!("{}m {:02}s", secs / 60, secs % 60)
        };
        // The installer's own percentage when it prints one; otherwise the step it is on,
        // since Homebrew and npm print no percentage when they aren't in a terminal.
        let figure = match (run.percent, run.stage) {
            (Some(p), _) => format!("{p:.0}% · {elapsed}"),
            (None, Some((step, of, label))) => {
                let packages = if run.fetched > 0 && label == "Downloading" {
                    format!(
                        " {} package{}",
                        run.fetched,
                        if run.fetched == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                };
                format!("Step {step} of {of} · {label}{packages} · {elapsed}")
            }
            (None, None) => format!("Starting · {elapsed}"),
        };
        let bar = div()
            .relative()
            .h(px(5.))
            .w_full()
            .rounded_full()
            .overflow_hidden()
            .bg(rgb(settings::mix(c_green(), c_border(), 0.2)))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(relative(fraction))
                    .rounded_full()
                    .bg(rgb(c_green())),
            )
            .child(
                // A light sweep across the filled part, so the bar always reads as alive.
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(relative(0.25))
                    .bg(hsla(0., 0., 1., 0.35))
                    .animated(
                        ("up-sweep", n),
                        Animation::new(Duration::from_millis(1400)).repeat(),
                        move |el, d| el.left(relative(d * (fraction + 0.25) - 0.25)),
                    ),
            );
        div().v_flex().gap(px(6.)).w_full().child(bar).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(icon(IconName::LoaderCircle, 11., c_green()).animated(
                    ("up-spin", n),
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |i, d| i.rotate(percentage(d)),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(self.mono_font.clone())
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(run.line.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(rgb(c_secondary()))
                        .child(figure),
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn alert_card(
        &self,
        id: SharedString,
        accent: u32,
        glyph: IconName,
        title: &'static str,
        subject: Option<String>,
        badge: Option<Div>,
        body: String,
        actions: Div,
        key: String,
        open: Option<u64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        slide_in(
            div()
                .id(id.clone())
                .relative()
                .v_flex()
                .gap(px(6.))
                .p_3()
                .rounded_xl()
                .border_1()
                .border_color(rgb(settings::mix(accent, c_border(), 0.45)))
                .bg(float(c_raised()))
                .shadow_xl()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(glyph, 14., accent))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .child(title),
                        )
                        .child(div().flex_1())
                        .child(
                            self.ghost_button_sized(
                                SharedString::from(format!("{id}-x")),
                                IconName::X,
                                "Hide",
                                22.,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.alerts_hidden.insert(key.clone());
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .when_some(subject, |el, subject| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .children(badge)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(subject),
                            )
                            .when_some(open, |el, sid| {
                                el.child(
                                    div()
                                        .id(SharedString::from(format!("{id}-open")))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_1()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(c_secondary()))
                                        .hover(|el| {
                                            el.bg(tint(c_subtle())).text_color(rgb(c_text()))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select(sid);
                                            this.view = View::Session;
                                            cx.notify();
                                        }))
                                        .child("Open")
                                        .child(icon(IconName::ArrowRight, 11., c_secondary())),
                                )
                            }),
                    )
                })
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child(body),
                )
                .child(actions),
            id,
            260,
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn versions_compare_and_updates_have_commands() {
        assert!(is_newer("2.1.300", "2.1.296 (Claude Code)"));
        assert!(is_newer("0.51.0", "codex-cli 0.50.1"));
        assert!(!is_newer("2.1.296", "v2.1.296"));
        assert!(!is_newer("1.0.0", "1.2.0"));
        assert!(!is_newer("", "1.0.0"));
        assert_eq!(percent_of("Downloading 2.1.296 66%"), Some(66.));
        assert_eq!(percent_of("[####    ] 42.5 %"), Some(42.5));
        assert_eq!(percent_of("no number here %"), None);
        assert_eq!(
            clean_line("\x1b[32mDownloading\x1b[0m 12% \u{280b}"),
            "Downloading 12%"
        );
        assert_eq!(
            phase_of("npm http fetch GET 200 https://x 41ms"),
            Some(0.35)
        );
        assert_eq!(phase_of("changed 1 package in 7s"), Some(0.95));
        let start = std::time::Instant::now();
        let mut run = UpdateRun::new(false, String::new());
        run.started = start;
        let early = run.progress(start);
        let later = run.progress(start + Duration::from_secs(30));
        assert!(early < later && later < 0.97, "time eases the bar forward");
        run.take_line("Verifying checksum...");
        assert!(run.progress(start) >= 0.8, "a later phase moves the bar");
        run.take_line("Downloading 50%");
        assert_eq!(run.progress(start), 0.5, "a printed percentage wins");
        run.finished = Some(start);
        assert_eq!(run.progress(start), 1.);
        let other = "npm error code E500\nnpm error 500 Internal Server Error - GET https://registry.npmjs.org/x\nnpm error A complete log of this run can be found in: /tmp/l.log";
        assert_eq!(
            failure_summary(other),
            "500 Internal Server Error - GET https://registry.npmjs.org/x"
        );

        assert_eq!(
            and_list(&["A".into(), "B".into(), "C".into()]),
            "A, B and C"
        );
    }

    #[test]
    fn sessions_waiting_on_the_person() {
        let mut s = Session {
            messages: vec![Message::new(Role::Agent, "done")],
            ..Session::default()
        };
        assert!(waiting_on(&s).is_none());
        s.messages[0].denied = vec!["Bash".into()];
        assert!(matches!(waiting_on(&s), Some((0, Waiting::Denied(_)))));
        s.messages.push(Message::new(Role::User, "ok"));
        assert!(waiting_on(&s).is_none());
    }
}
