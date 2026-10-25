//! Proof of done: the project's checks run after each turn that changed files, a red
//! result goes back to the agent, and an agent from another vendor reviews the change.

use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
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

/// What the card says when a turn changed only docs and the checks didn't run.
pub const DOCS_ONLY: &str = "Checks skipped: only docs changed.";

/// Whether a changed file can't affect any project's checks: prose, images and media,
/// and git or editor settings. Anything else, such as code, manifests, lockfiles and
/// build or tool configuration, counts as code.
pub fn is_docs(path: &str) -> bool {
    let path = path.replace('\\', "/");
    let name = path.rsplit('/').next().unwrap_or(&path);
    let lower = name.to_ascii_lowercase();
    // A top-level folder of documentation.
    let first = path.split('/').next().unwrap_or("").to_ascii_lowercase();
    if path.contains('/') && matches!(first.as_str(), "docs" | "doc") {
        return true;
    }
    if path.starts_with(".github/") && !path.starts_with(".github/actions/") {
        return true;
    }
    let stem = lower.split('.').next().unwrap_or("");
    if matches!(
        stem,
        "license" | "licence" | "copying" | "notice" | "authors" | "contributors" | "codeowners"
    ) || matches!(
        lower.as_str(),
        ".gitignore" | ".gitattributes" | ".mailmap" | ".editorconfig"
    ) {
        return true;
    }
    let Some((_, ext)) = lower.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) else {
        return false;
    };
    matches!(
        ext,
        "md" | "markdown"
            | "mdown"
            | "txt"
            | "rst"
            | "adoc"
            | "asciidoc"
            | "org"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "avif"
            | "bmp"
            | "ico"
            | "icns"
            | "tif"
            | "tiff"
            | "heic"
            | "psd"
            | "pdf"
            | "mp4"
            | "mov"
            | "webm"
            | "mp3"
            | "wav"
    )
}

/// Whether a turn's changed files are all docs, so its checks can be skipped. A turn
/// with no changes recorded doesn't count: its checks run.
pub fn only_docs(changed: &[String]) -> bool {
    !changed.is_empty() && changed.iter().all(|p| is_docs(p))
}

/// Where Cargo should build a linked worktree's checks: the main checkout's own target
/// folder, so each new worktree reuses its compiled dependencies instead of building
/// every one again into a target of its own. None when the person set a target already,
/// outside a worktree, or when the main checkout has no Rust project there.
pub fn shared_target(dir: &str, commands: &[String]) -> Option<std::path::PathBuf> {
    if !commands.iter().any(|c| c.contains("cargo"))
        || std::env::var_os("CARGO_TARGET_DIR").is_some()
    {
        return None;
    }
    let main = crate::git::main_checkout_dir(dir)?;
    main.join("Cargo.toml")
        .is_file()
        .then(|| main.join("target"))
}

/// Post-turn check sets across every session and project, at most a set number at once
/// (see `AppSettings::checks_limit`). The rest wait in order; the time limit only starts
/// once a set leaves the queue.
#[derive(Default, Debug)]
pub struct CheckQueue {
    running: Vec<u64>,
    /// Sessions waiting, first in line first, each with whether its turn just finished.
    waiting: VecDeque<(u64, bool)>,
}

impl CheckQueue {
    /// Line up a session's checks, unless they are already waiting or running. `fill`
    /// says when they start.
    pub fn join(&mut self, id: u64, auto: bool) {
        if !self.running.contains(&id) && !self.is_waiting(id) {
            self.waiting.push_back((id, auto));
        }
    }

    /// Move sessions from the front of the line to running while fewer than `limit` run
    /// (0 for no limit): the ones to start now, in order.
    pub fn fill(&mut self, limit: usize) -> Vec<(u64, bool)> {
        let mut out = Vec::new();
        while limit == 0 || self.running.len() < limit {
            let Some((id, auto)) = self.waiting.pop_front() else {
                break;
            };
            self.running.push(id);
            out.push((id, auto));
        }
        out
    }

    /// The session's running checks finished, freeing their place. False when they
    /// weren't running.
    pub fn done(&mut self, id: u64) -> bool {
        let before = self.running.len();
        self.running.retain(|r| *r != id);
        self.running.len() != before
    }

    /// Take a waiting session out of the line. True when it was waiting.
    pub fn drop_waiting(&mut self, id: u64) -> bool {
        let before = self.waiting.len();
        self.waiting.retain(|(w, _)| *w != id);
        self.waiting.len() != before
    }

    pub fn is_waiting(&self, id: u64) -> bool {
        self.waiting.iter().any(|(w, _)| *w == id)
    }
}

/// Windows' priority class for checks; processes they start inherit it.
#[cfg_attr(not(windows), allow(dead_code))]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// The programs that run a check at low CPU and disk priority on this OS, found by `find`
/// (a usable path, or `None` when the tool is missing). Children inherit the priority.
fn low_prefix(mut find: impl FnMut(&str, &[&str]) -> Option<String>) -> Vec<String> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        // Background QoS: low CPU, disk and network priority.
        if let Some(tp) = find("taskpolicy", &["-b"]) {
            out.extend([tp, "-b".into()]);
        }
    } else if cfg!(unix) {
        if let Some(ionice) = find("ionice", &["-c", "3"]) {
            out.extend([ionice, "-c".into(), "3".into()]);
        }
        if let Some(nice) = find("nice", &["-n", "10"]) {
            out.extend([nice, "-n".into(), "10".into()]);
        }
    }
    out
}

/// The program and arguments that run `command`, after `prefix`.
fn check_argv(command: &str, prefix: &[String]) -> Vec<String> {
    let mut out = prefix.to_vec();
    if cfg!(windows) {
        out.extend(["cmd".into(), "/C".into(), command.into()]);
    } else {
        // One pipe for both streams keeps errors next to what led to them.
        out.extend(["sh".into(), "-c".into(), format!("exec 2>&1\n{command}")]);
    }
    out
}

/// A priority tool that is on this computer and works here (a container may refuse
/// `ionice`), tried once with `true`.
fn working_tool(name: &str, args: &[&str]) -> Option<String> {
    let dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let fixed = ["/usr/sbin", "/usr/bin", "/sbin", "/bin"].map(std::path::PathBuf::from);
    let path = dirs
        .into_iter()
        .chain(fixed)
        .map(|d| d.join(name))
        .find(|p| p.is_file())?;
    let ok = Command::new(&path)
        .args(args)
        .arg("true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ok.then(|| path.to_string_lossy().into_owned())
}

/// The low-priority prefix for this computer, looked up once. A missing tool is noted
/// once in the log and its checks run at normal priority.
fn low_prefix_here() -> &'static [String] {
    static PREFIX: OnceLock<Vec<String>> = OnceLock::new();
    PREFIX.get_or_init(|| {
        let mut missing = Vec::new();
        let prefix = low_prefix(|name, args| {
            let found = working_tool(name, args);
            if found.is_none() {
                missing.push(name.to_string());
            }
            found
        });
        if !missing.is_empty() {
            crate::applog::info(
                "proof",
                format!(
                    "{} isn't available, so checks run without it at normal priority",
                    missing.join(" and ")
                ),
            );
        }
        prefix
    })
}

/// Run each check in turn in `dir`. Every check runs, so one card shows all that failed.
/// `low` runs them at low CPU and disk priority, with everything they start.
/// `env` is added to each check's environment.
pub fn run_all(
    dir: &str,
    commands: &[String],
    limit: Duration,
    low: bool,
    env: &[(String, String)],
) -> Vec<CheckRun> {
    commands
        .iter()
        .map(|c| run_one(dir, c, limit, low, env))
        .collect()
}

fn run_one(
    dir: &str,
    command: &str,
    limit: Duration,
    low: bool,
    env: &[(String, String)],
) -> CheckRun {
    let started = Instant::now();
    let prefix = if low { low_prefix_here() } else { &[] };
    let argv = check_argv(command, prefix);
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    #[cfg(windows)]
    if low {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(BELOW_NORMAL_PRIORITY_CLASS);
    }
    cmd.current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("CI", "1")
        .env("NO_COLOR", "1")
        .envs(env.iter().map(|(k, v)| (k, v)));
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
            false,
            &[],
        );
        assert!(runs[0].ok);
        assert_eq!(runs[0].tail, "fine");
        assert!(!runs[1].ok);
        assert_eq!(runs[1].tail, "broke");
        let timed = run_one(&d, "sleep 5", Duration::from_millis(300), true, &[]);
        assert!(!timed.ok && timed.timed_out);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_wait_their_turn_in_order() {
        let mut q = CheckQueue::default();
        q.join(1, true);
        assert_eq!(q.fill(1), [(1, true)]);
        q.join(2, true);
        q.join(3, false);
        assert!(q.fill(1).is_empty());
        // Asking again while waiting or running keeps its place.
        q.join(2, false);
        q.join(1, true);
        assert!(q.is_waiting(2) && q.is_waiting(3) && !q.is_waiting(1));
        // Only a running set's end moves the line on.
        assert!(!q.done(3));
        assert!(q.fill(1).is_empty());
        assert!(q.done(1));
        assert_eq!(q.fill(1), [(2, true)]);
        assert!(!q.is_waiting(2));
        assert!(q.done(2));
        assert_eq!(q.fill(1), [(3, false)]);
        assert!(q.done(3));
        q.join(4, true);
        assert_eq!(q.fill(1), [(4, true)]);
    }

    #[test]
    fn several_check_sets_run_at_once_and_the_rest_keep_their_order() {
        let mut q = CheckQueue::default();
        for id in 1..=5 {
            q.join(id, id % 2 == 0);
        }
        assert_eq!(q.fill(2), [(1, false), (2, true)]);
        assert!(q.fill(2).is_empty());
        assert!(q.is_waiting(3) && q.is_waiting(4) && q.is_waiting(5));
        // Whichever finishes first, the next in line takes its place.
        assert!(q.done(2));
        assert_eq!(q.fill(2), [(3, false)]);
        assert!(q.done(1));
        assert!(q.done(3));
        assert_eq!(q.fill(2), [(4, true), (5, false)]);
        // A higher limit starts more at once; none starts everything waiting.
        for id in 6..=9 {
            q.join(id, true);
        }
        assert_eq!(q.fill(3), [(6, true)]);
        assert_eq!(q.fill(0).len(), 3);
        assert!(!q.is_waiting(9));
        // A lower limit lets running sets finish but starts none until it's met again.
        q.join(10, true);
        assert!(q.fill(1).is_empty());
        for id in [4, 5, 6, 7, 8] {
            q.done(id);
        }
        assert!(q.fill(1).is_empty());
        q.done(9);
        assert_eq!(q.fill(1), [(10, true)]);
    }

    #[test]
    fn a_cancelled_session_leaves_the_line() {
        let mut q = CheckQueue::default();
        for id in 1..=4 {
            q.join(id, true);
        }
        assert_eq!(q.fill(2).len(), 2);
        assert!(q.drop_waiting(3));
        assert!(!q.drop_waiting(3));
        // Running sets aren't dropped; they finish and hand on.
        assert!(!q.drop_waiting(1));
        assert!(q.done(1));
        assert_eq!(q.fill(2), [(4, true)]);
        assert!(q.done(2) && q.done(4));
        assert!(q.fill(2).is_empty());
    }

    /// The limit is each check's own, counted from when it starts: two checks that each
    /// take most of it both pass, though together they take longer than it.
    #[cfg(unix)]
    #[test]
    fn the_time_limit_starts_when_the_check_starts() {
        let dir = temp("limit");
        let d = dir.to_string_lossy().to_string();
        let limit = Duration::from_millis(1500);
        // Time spent waiting in the queue before the run doesn't count either.
        let mut q = CheckQueue::default();
        for id in 1..=3 {
            q.join(id, true);
        }
        assert_eq!(q.fill(2).len(), 2);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(q.done(1));
        assert_eq!(q.fill(2), [(3, true)]);
        // Two sets running side by side each keep their own limit.
        let sets: Vec<_> = (0..2)
            .map(|_| {
                let d = d.clone();
                std::thread::spawn(move || {
                    run_all(&d, &["sleep 1".into(), "sleep 1".into()], limit, true, &[])
                })
            })
            .collect();
        for set in sets {
            let runs = set.join().unwrap();
            assert!(runs.iter().all(|r| r.ok && !r.timed_out), "{runs:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_run_normally_without_a_priority_tool() {
        let argv = check_argv("make test", &low_prefix(|_, _| None));
        if cfg!(windows) {
            assert_eq!(argv, ["cmd", "/C", "make test"]);
        } else {
            assert_eq!(argv, ["sh", "-c", "exec 2>&1\nmake test"]);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_checks_run_with_background_priority() {
        let prefix = low_prefix(|name, _| Some(format!("/usr/sbin/{name}")));
        assert_eq!(
            check_argv("npm test", &prefix),
            [
                "/usr/sbin/taskpolicy",
                "-b",
                "sh",
                "-c",
                "exec 2>&1\nnpm test"
            ]
        );
        // What the check starts runs in the background band too (priority 4).
        let dir = temp("mac-low");
        let run = run_one(
            &dir.to_string_lossy(),
            "sh -c 'ps -o pri= -p $$'",
            Duration::from_secs(20),
            true,
            &[],
        );
        assert_eq!(run.tail.trim(), "4", "{run:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_checks_run_niced_and_idle_for_disk() {
        let prefix = low_prefix(|name, _| Some(format!("/usr/bin/{name}")));
        assert_eq!(
            check_argv("go test ./...", &prefix),
            [
                "/usr/bin/ionice",
                "-c",
                "3",
                "/usr/bin/nice",
                "-n",
                "10",
                "sh",
                "-c",
                "exec 2>&1\ngo test ./..."
            ]
        );
        // Without ionice, nice alone.
        let prefix = low_prefix(|name, _| (name == "nice").then(|| "nice".to_string()));
        assert_eq!(prefix, ["nice", "-n", "10"]);
        // What the check starts inherits the niceness.
        let dir = temp("linux-low");
        let run = run_one(
            &dir.to_string_lossy(),
            "sh -c nice",
            Duration::from_secs(20),
            true,
            &[],
        );
        assert_eq!(run.tail.trim(), "10", "{run:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_checks_start_below_normal() {
        assert!(low_prefix(|_, _| Some("x".into())).is_empty());
        assert_eq!(BELOW_NORMAL_PRIORITY_CLASS, 0x4000);
    }

    #[test]
    fn turns_that_change_only_docs_skip_their_checks() {
        let files = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(only_docs(&files(&["README.md"])));
        assert!(only_docs(&files(&[
            "docs/perf-audit.md",
            "docs/guide/setup.html",
            "CHANGELOG.md",
            "notes.txt",
            "assets/logo.png",
            "public/Shot.JPG",
            "LICENSE",
            "NOTICE",
            ".gitignore",
            ".github/workflows/ci.yml",
            "native\\app\\docs.rst",
        ])));
        // Any code, manifest, lockfile or tool configuration runs them, in any language.
        for code in [
            "src/main.rs",
            "app/views.py",
            "web/index.ts",
            "cmd/main.go",
            "Cargo.toml",
            "Cargo.lock",
            "package.json",
            "pnpm-lock.yaml",
            "tsconfig.json",
            "Makefile",
            ".eslintrc",
            "src/docs/mod.rs",
            "README",
            ".github/actions/setup/action.yml",
            "assets/icon.svg",
        ] {
            assert!(!only_docs(&files(&["README.md", code])), "{code}");
        }
        // Nothing recorded as changed: the checks run, as before.
        assert!(!only_docs(&[]));
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
