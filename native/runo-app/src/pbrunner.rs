//! Scheduled playbooks while Runo is closed. On macOS a launchd agent runs
//! `runo --run-due-playbooks` every few minutes; it runs each due playbook in a new
//! worktree with the agent's one-shot command and leaves the result in a file. Runo
//! turns those files into sessions when it opens. While Runo is open it runs the
//! schedules itself, so the runner does nothing.

use super::*;
use crate::store::{Automation, PlaybookRun, Project, State};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The launchd job's label.
#[cfg(target_os = "macos")]
const LABEL: &str = "dev.runo.playbooks";

/// How often launchd starts the runner.
const INTERVAL_SECS: u64 = 300;

/// Runo counts as open while its heartbeat is this recent.
const ALIVE_SECS: u64 = 90;

/// The longest a run may take before it is stopped.
const RUN_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);

/// A run the runner made while Runo was closed, for Runo to pick up.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct HeadlessRun {
    pub playbook: u64,
    pub project: u64,
    pub provider: String,
    pub branch: String,
    pub dir: String,
    pub base: String,
    pub prompt: String,
    pub started: u64,
    pub finished: u64,
    /// "done", "failed" or "skipped".
    pub status: String,
    pub output: String,
    pub note: String,
    /// The agent's own conversation id, when it was given one.
    #[serde(default)]
    pub agent_session: String,
}

fn runs_dir() -> PathBuf {
    store::data_dir().join("playbook-runs")
}

fn running_file(playbook: u64) -> PathBuf {
    runs_dir().join(format!("running-{playbook}"))
}

fn age(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.elapsed().unwrap_or_default().as_secs())
}

/// Mark Runo as open, so the runner leaves the schedules to it.
pub fn beat() {
    let dir = runs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("alive"), workspace::unix_now().to_string());
}

/// Whether the runner is working on this playbook now.
pub fn running(playbook: u64) -> bool {
    age(&running_file(playbook)).is_some_and(|a| a < RUN_LIMIT.as_secs() + 600)
}

/// Runs the runner finished, oldest first. Their files are removed as they are read.
fn take_results() -> Vec<HeadlessRun> {
    let Ok(entries) = std::fs::read_dir(runs_dir()) else {
        return Vec::new();
    };
    let mut runs: Vec<HeadlessRun> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("done-"))
        .filter_map(|e| {
            let run = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok();
            let _ = std::fs::remove_file(e.path());
            run
        })
        .collect();
    runs.sort_by_key(|r| r.started);
    runs
}

/// When the runner last started this playbook, from results Runo hasn't picked up.
fn last_result(playbook: u64) -> u64 {
    let Ok(entries) = std::fs::read_dir(runs_dir()) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("done-{playbook}-"))
        })
        .filter_map(|e| serde_json::from_slice::<HeadlessRun>(&std::fs::read(e.path()).ok()?).ok())
        .map(|r| r.started)
        .max()
        .unwrap_or(0)
}

/// The command for one unattended run. Playbooks that may edit get the agent's
/// edit mode where Runo knows it; the rest run read-only.
pub fn command(
    auto: &Automation,
    bin: &str,
    prompt: &str,
    conversation: &str,
) -> std::process::Command {
    let edit = auto.may("edit");
    let program = if bin.trim().is_empty() {
        agent::provider(&auto.provider).bin
    } else {
        bin.trim()
    };
    match auto.provider.as_str() {
        "claude" if edit => {
            let mut c = std::process::Command::new(program);
            c.args(["-p", "--permission-mode", "acceptEdits"]);
            if !conversation.is_empty() {
                c.args(["--session-id", conversation]);
            }
            c.args(["--", prompt]);
            c
        }
        "codex" if edit => {
            let mut c = std::process::Command::new(program);
            c.args([
                "exec",
                "--sandbox",
                "workspace-write",
                "--skip-git-repo-check",
                prompt,
            ]);
            c
        }
        _ => agent::ask_command(&auto.provider, bin, prompt),
    }
}

/// `runo --run-due-playbooks`: run the scheduled playbooks that are due, unless Runo is
/// open. Returns the exit code.
pub fn run_due() -> i32 {
    let dir = runs_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return 1;
    }
    if age(&dir.join("alive")).is_some_and(|a| a < ALIVE_SECS) {
        return 0;
    }
    // One runner at a time; launchd may start the next while a long run goes on.
    let Ok(lock) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("runner.lock"))
    else {
        return 1;
    };
    if lock.try_lock().is_err() {
        return 0;
    }
    let settings = AppSettings::load();
    let state = State::load();
    for auto in &state.automations {
        let mut auto = auto.clone();
        auto.last_run = auto.last_run.max(last_result(auto.id));
        let now = workspace::unix_now();
        if !auto.due(now) || running(auto.id) {
            continue;
        }
        // Runo may have opened while an earlier playbook ran.
        if age(&dir.join("alive")).is_some_and(|a| a < ALIVE_SECS) {
            break;
        }
        let Some(project) = state.projects.iter().find(|p| p.id == auto.project_id) else {
            continue;
        };
        let at = auto.next_run().unwrap_or(now);
        let late = now.saturating_sub(at) > INTERVAL_SECS + 60;
        let mut run = if late && !settings.playbook_catch_up {
            HeadlessRun {
                status: "skipped".into(),
                note:
                    "Missed while the computer was off or asleep; catching up is off in Settings."
                        .into(),
                ..Default::default()
            }
        } else if project.connection != connections::LOCAL {
            HeadlessRun {
                status: "skipped".into(),
                note: "Its project runs on a connection, which needs Runo open.".into(),
                ..Default::default()
            }
        } else {
            let _ = std::fs::write(running_file(auto.id), now.to_string());
            let mut run = run_one(&auto, project, &settings);
            let _ = std::fs::remove_file(running_file(auto.id));
            if let Some(note) = workspace::missed_note(auto.runs_due(now), late.then_some(at)) {
                run.note = note;
            }
            run
        };
        run.playbook = auto.id;
        run.project = project.id;
        run.provider = auto.provider.clone();
        run.started = run.started.max(now);
        run.finished = workspace::unix_now();
        let file = dir.join(format!("done-{}-{}.json", auto.id, run.started));
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&run).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
        eprintln!("Runo: ran playbook {} ({})", auto.name, run.status);
    }
    0
}

/// Run one playbook in a new worktree and wait for it.
fn run_one(auto: &Automation, project: &Project, settings: &AppSettings) -> HeadlessRun {
    let started = workspace::unix_now();
    let prompt = crate::pbfile::fill(&auto.prompt, &auto.run_inputs(), &[]);
    let mut run = HeadlessRun {
        prompt: prompt.clone(),
        started,
        ..Default::default()
    };
    if git::is_repo(&project.path) {
        let base = git::default_branch(&project.path);
        let branch = format!("runo/playbook-{}-{started}", auto.id);
        match ship::prepare_worktree(&project.path, &branch, &base, None) {
            Ok((dir, branch, _)) => {
                run.dir = dir;
                run.branch = branch;
                run.base = base;
            }
            Err(error) => {
                run.status = "failed".into();
                run.output = format!("Could not create the worktree: {error}");
                return run;
            }
        }
    } else if auto.may("edit") {
        // Without git there is no worktree to keep its edits apart.
        run.status = "skipped".into();
        run.note =
            "Its project is not a git repository, so it only runs while Runo is open.".into();
        return run;
    } else {
        run.dir = project.path.clone();
        run.branch = "local folder".into();
    }
    let opts = settings.agent(&auto.provider);
    // Claude Code takes the conversation's id, so the session can carry on with it.
    if auto.provider == "claude" && auto.may("edit") {
        run.agent_session = agent::new_uuid();
    }
    let mut command = command(auto, &opts.binary, &prompt, &run.agent_session);
    for line in opts.env.lines() {
        if let Some((key, value)) = line.split_once('=') {
            command.env(key.trim(), value.trim());
        }
    }
    command
        .current_dir(&run.dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match wait_with_limit(command, RUN_LIMIT) {
        Ok((true, out, _)) => {
            run.status = "done".into();
            run.output = out.trim().to_string();
        }
        Ok((false, out, err)) => {
            run.status = "failed".into();
            run.output = if err.trim().is_empty() { out } else { err }
                .trim()
                .to_string();
        }
        Err(error) => {
            run.status = "failed".into();
            run.output = error;
        }
    }
    run
}

/// Run a command, stopping it after `limit`. Returns success, stdout and stderr.
fn wait_with_limit(
    mut command: std::process::Command,
    limit: Duration,
) -> Result<(bool, String, String), String> {
    use std::io::Read;
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start the agent: {e}"))?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut text);
            }
            text
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = std::time::Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_secs(1)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok((
        ok,
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    ))
}

impl RunoApp {
    /// Turn runs made while Runo was closed into sessions, and mark Runo as open.
    pub(crate) fn import_headless_runs(&mut self) {
        beat();
        for run in take_results() {
            let Some(auto) = self
                .state
                .automations
                .iter()
                .find(|a| a.id == run.playbook)
                .cloned()
            else {
                continue;
            };
            let mut record = PlaybookRun {
                at: run.started,
                status: run.status.clone(),
                note: run.note.clone(),
                ..Default::default()
            };
            if run.status != "skipped" && !run.dir.is_empty() {
                let id = self.state.next_id();
                let mut messages = vec![Message::new(Role::User, run.prompt.clone())];
                if !run.note.is_empty() {
                    let mut note = Message::new(Role::System, run.note.clone());
                    note.meta = "note".into();
                    messages.insert(0, note);
                }
                messages.push(Message::new(
                    if run.status == "done" {
                        Role::Agent
                    } else {
                        Role::System
                    },
                    if run.output.is_empty() {
                        "The agent finished without a reply.".to_string()
                    } else {
                        run.output.clone()
                    },
                ));
                let when = chrono::DateTime::from_timestamp(run.started as i64, 0)
                    .map(|d| {
                        d.with_timezone(&chrono::Local)
                            .format("%b %-d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                self.state.sessions.push(Session {
                    id,
                    project_id: run.project,
                    title: format!("{} · {when}", auto.name),
                    provider: run.provider.clone(),
                    branch: run.branch.clone(),
                    dir: run.dir.clone(),
                    isolated: run.branch != "local folder",
                    base: run.base.clone(),
                    status: if run.status == "done" {
                        Status::Done
                    } else {
                        Status::Failed
                    },
                    messages,
                    agent_session: run.agent_session.clone(),
                    unread: true,
                    playbook: auto.id,
                    limits: auto.permissions.clone(),
                    ..Default::default()
                });
                self.state.log(
                    id,
                    format!("Playbook: {} (ran while Runo was closed)", auto.name),
                );
                record.session = id;
                if record.note.is_empty() {
                    record.note = "Ran while Runo was closed.".into();
                }
            } else if run.status == "failed" && record.note.is_empty() {
                record.note = run.output.clone();
            }
            if let Some(a) = self
                .state
                .automations
                .iter_mut()
                .find(|a| a.id == run.playbook)
            {
                a.last_run = a.last_run.max(run.started);
                if run.status != "skipped" {
                    a.ran = a.ran.max(run.started);
                }
                a.record(record);
            }
            self.prune_playbook_runs(run.playbook);
            self.save();
        }
    }
}

/// Keep the launchd agent in line with the setting: installed and pointing at this
/// copy of Runo when on, gone when off. Runs in the background.
pub fn sync_launch_agent(enabled: bool) {
    #[cfg(target_os = "macos")]
    std::thread::spawn(move || {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let plist = PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let job = format!("{domain}/{LABEL}");
        let launchctl = |args: &[&str]| {
            let _ = std::process::Command::new("launchctl")
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        };
        let current = std::fs::read_to_string(&plist).ok();
        if !enabled {
            if current.is_some() {
                launchctl(&["bootout", job.as_str()]);
                let _ = std::fs::remove_file(&plist);
            }
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let wanted = launch_plist(
            &exe.to_string_lossy(),
            &runs_dir().join("runner.log").to_string_lossy(),
        );
        if current.as_deref() == Some(wanted.as_str()) {
            return;
        }
        if let Some(parent) = plist.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::create_dir_all(runs_dir());
        if std::fs::write(&plist, wanted).is_ok() {
            let path = plist.to_string_lossy().into_owned();
            launchctl(&["bootout", job.as_str()]);
            launchctl(&["bootstrap", domain.as_str(), path.as_str()]);
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = enabled;
}

/// The launchd job: the runner every few minutes, in the background.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launch_plist(exe: &str, log: &str) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.runo.playbooks</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>--run-due-playbooks</string>
  </array>
  <key>StartInterval</key><integer>{INTERVAL_SECS}</integer>
  <key>RunAtLoad</key><false/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        esc(exe),
        esc(log),
        esc(log)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn unattended_runs_edit_only_when_allowed() {
        let mut auto = Automation {
            provider: "claude".into(),
            ..Default::default()
        };
        let args = |c: &std::process::Command| {
            c.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            args(&command(&auto, "", "Fix it", "abc")),
            "-p --permission-mode acceptEdits --session-id abc -- Fix it"
        );
        auto.permissions = vec!["read".into()];
        assert_eq!(
            args(&command(&auto, "", "Look", "")),
            "-p --permission-mode plan -- Look"
        );
        auto.provider = "codex".into();
        auto.permissions = vec!["read".into(), "edit".into()];
        let codex = command(&auto, "/opt/codex", "Fix it", "");
        assert_eq!(codex.get_program(), "/opt/codex");
        assert_eq!(
            args(&codex),
            "exec --sandbox workspace-write --skip-git-repo-check Fix it"
        );
    }

    #[test]
    fn the_launch_agent_runs_the_runner() {
        let plist = launch_plist(
            "/Applications/R&D/Runo.app/Contents/MacOS/Runo",
            "/tmp/l.log",
        );
        assert!(
            plist.contains("<string>/Applications/R&amp;D/Runo.app/Contents/MacOS/Runo</string>")
        );
        assert!(plist.contains("<string>--run-due-playbooks</string>"));
        assert!(plist.contains("<integer>300</integer>"));
    }

    #[test]
    fn results_round_trip() {
        let run = HeadlessRun {
            playbook: 3,
            status: "done".into(),
            output: "All good".into(),
            ..Default::default()
        };
        let back: HeadlessRun =
            serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(back, run);
        // Files from an older runner without a conversation id still read.
        let old: HeadlessRun = serde_json::from_str(
            r#"{"playbook":1,"project":2,"provider":"codex","branch":"b","dir":"d","base":"main","prompt":"p","started":5,"finished":6,"status":"failed","output":"o","note":""}"#,
        )
        .unwrap();
        assert_eq!(old.agent_session, "");
    }
}
