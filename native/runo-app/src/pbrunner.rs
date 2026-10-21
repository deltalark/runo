//! Scheduled playbooks while Runo is closed. A background job (launchd on macOS, a
//! systemd user timer on Linux, Task Scheduler on Windows; see `job`) runs
//! `runo --run-due-playbooks` every few minutes; it runs each due playbook in a new
//! worktree, after the project's setup command, with the same agent settings and limits
//! as a run in Runo, and leaves the result in a file. Runo
//! turns those files into sessions when it opens. While Runo is open it runs the
//! schedules itself, so the runner does nothing.

use super::*;
use crate::store::{Automation, PlaybookRun, Project, State};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod job;
pub use job::{note as job_note, sync as sync_job};

/// How often the background job starts the runner.
const INTERVAL_SECS: u64 = 300;

/// Runo counts as open while its heartbeat is this recent.
const ALIVE_SECS: u64 = 90;

/// The longest a run may take before it is stopped.
const RUN_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);

/// The longest the worktree setup command may take.
const SETUP_LIMIT: Duration = Duration::from_secs(30 * 60);

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

/// What the runner has made that Runo has no session for yet: the folders and branches
/// of finished runs not picked up, and branch prefixes of runs still going.
pub fn in_flight() -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut dirs, mut branches, mut prefixes) = (Vec::new(), Vec::new(), Vec::new());
    let Ok(entries) = std::fs::read_dir(runs_dir()) else {
        return (dirs, branches, prefixes);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_prefix("running-") {
            if id.parse::<u64>().is_ok_and(running) {
                prefixes.push(format!("runo/playbook-{id}-"));
            }
        } else if name.starts_with("done-") {
            let run = std::fs::read(entry.path())
                .ok()
                .and_then(|b| serde_json::from_slice::<HeadlessRun>(&b).ok());
            if let Some(run) = run {
                dirs.push(run.dir);
                branches.push(run.branch);
            }
        }
    }
    (dirs, branches, prefixes)
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

/// The level an unattended run works at: read-only without `edit`, else the agent's
/// edit mode. Claude Code's edit mode refuses commands in a run nobody watches, so it
/// runs everything when the playbook may run commands; its other limits still hold.
fn unattended_mode(auto: &Automation) -> &'static str {
    if !auto.may("edit") {
        "plan"
    } else if auto.provider == "claude" && auto.may("shell") {
        "full"
    } else {
        "edit"
    }
}

/// The command for one unattended run: the same arguments as a turn in Runo (the
/// agent's model, effort, extra arguments, profile and tools), held to the playbook's
/// permissions. `conversation` names a new conversation for agents that take one.
pub fn command(
    auto: &Automation,
    opts: &AgentOptions,
    prompt: &str,
    conversation: &str,
    in_git: bool,
) -> Result<std::process::Command, String> {
    let mode = unattended_mode(auto);
    if let Some(error) = agent::mode_error(&auto.provider, mode) {
        return Err(error);
    }
    let mut opts = opts.clone();
    if mode == "plan" {
        // An OpenCode agent picked by name would replace its plan agent.
        opts.sub_agent.clear();
    }
    workspace::playbook_limits(&auto.provider, &auto.permissions, &mut opts);
    let sel = std::collections::HashMap::from([("mode".to_string(), mode.to_string())]);
    let spec = agent::RunSpec {
        provider: &auto.provider,
        prompt,
        resume: false,
        sel: &sel,
        opts: &opts,
        in_git,
        session_id: conversation,
        images: &[],
        memory: None,
        local: true,
        thread: 0,
    };
    Ok(agent::turn_command(&spec))
}

/// The reply in a run's output, read as Runo reads a turn: its text, the error it
/// reported and the conversation it named. Plain output is the reply as it is.
fn read_output(provider: &str, dir: &str, out: &str) -> (String, Option<String>, String) {
    let Some(mut parser) = crate::stream::Parser::for_provider(provider, dir) else {
        return (out.trim().to_string(), None, String::new());
    };
    let (mut text, mut error, mut conversation) = (String::new(), None, String::new());
    for line in out.lines() {
        for update in parser.feed(line) {
            match update {
                crate::stream::Update::Text(t) => text.push_str(&t),
                crate::stream::Update::Error(e) => error = Some(e),
                crate::stream::Update::Conversation(c) => conversation = c,
                _ => {}
            }
        }
    }
    (text.trim().to_string(), error, conversation)
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
    // One runner at a time; the background job may start the next while a long run goes on.
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
    crate::caps::set_binaries(
        settings
            .agents
            .iter()
            .map(|(id, a)| (id.clone(), a.binary.clone()))
            .collect(),
    );
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
    // The project's setup command runs first in the new worktree, as in Runo.
    let setup = settings.setup_for(&project.setup);
    if run.branch != "local folder" && !setup.is_empty() {
        let mut shell = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", &setup]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", &setup]);
            c
        };
        shell
            .current_dir(&run.dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if !matches!(wait_with_limit(shell, SETUP_LIMIT), Ok((true, ..))) {
            run.note = format!("The setup command `{setup}` failed; the agent ran anyway.");
        }
    }
    // What the installed CLI supports decides some of its flags.
    crate::caps::detect(agent::provider(&auto.provider));
    // Agents that take a new conversation's id get one, so the session can carry on.
    let conversation = if agent::names_conversation(&auto.provider) {
        agent::new_uuid()
    } else {
        String::new()
    };
    let opts = settings.agent(&auto.provider).for_project(project.id);
    let mut command = match command(auto, &opts, &prompt, &conversation, git::is_repo(&run.dir)) {
        Ok(command) => command,
        Err(error) => {
            run.status = "failed".into();
            run.output = error;
            return run;
        }
    };
    agent::prepare_folder(
        &agent::RunSpec {
            provider: &auto.provider,
            prompt: &prompt,
            resume: false,
            sel: &Default::default(),
            opts: &opts,
            in_git: true,
            session_id: "",
            images: &[],
            memory: None,
            local: true,
            thread: 0,
        },
        &run.dir,
        false,
    );
    command
        .current_dir(&run.dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match wait_with_limit(command, RUN_LIMIT) {
        Ok((ok, out, err)) => {
            let (reply, error, named) = read_output(&auto.provider, &run.dir, &out);
            run.agent_session = if named.is_empty() {
                conversation
            } else {
                named
            };
            if ok && error.is_none() {
                run.status = "done".into();
                run.output = reply;
            } else {
                run.status = "failed".into();
                let tip = crate::modelcheck::tip(
                    &auto.provider,
                    &format!("{}\n{err}", error.clone().unwrap_or_default()),
                );
                run.output = error
                    .or_else(|| agent::sign_in_hint(&auto.provider, &err))
                    .unwrap_or_else(|| if err.trim().is_empty() { reply } else { err });
                run.output = run.output.trim().to_string();
                // A model or version problem says what to do; the run's session offers the fix.
                if let Some(tip) = tip {
                    run.output = format!("{}\n{tip}", run.output);
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    fn args(c: &std::process::Command) -> String {
        c.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn playbook(provider: &str, permissions: &[&str]) -> Automation {
        Automation {
            provider: provider.into(),
            permissions: permissions.iter().map(|p| p.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn unattended_runs_use_the_agent_settings_and_the_playbook_limits() {
        let opts = AgentOptions {
            model_override: "opus".into(),
            effort: "high".into(),
            extra_args: "--verbose-x".into(),
            allowed_tools: "Read".into(),
            ..Default::default()
        };
        // Allowed to run commands, Claude Code runs everything else it isn't denied.
        let all = command(&playbook("claude", &[]), &opts, "Fix it", "abc", true).unwrap();
        let line = args(&all);
        assert!(
            line.contains("--permission-mode bypassPermissions"),
            "{line}"
        );
        assert!(line.contains("--model opus") && line.contains("--effort high"));
        assert!(line.contains("--allowed-tools Read") && line.contains("--verbose-x"));
        assert!(line.ends_with("--session-id abc -- Fix it"), "{line}");
        // Edits without commands keep its edit mode, with the shell turned off.
        let edit = args(
            &command(
                &playbook("claude", &["read", "edit"]),
                &opts,
                "Fix",
                "",
                true,
            )
            .unwrap(),
        );
        assert!(edit.contains("--permission-mode acceptEdits"), "{edit}");
        assert!(
            edit.contains("--disallowed-tools Bash,WebFetch,WebSearch"),
            "{edit}"
        );
        let read = args(&command(&playbook("claude", &["read"]), &opts, "Look", "", true).unwrap());
        assert!(read.contains("--permission-mode plan"), "{read}");

        let codex_opts = AgentOptions {
            binary: "/opt/codex".into(),
            profile: "work".into(),
            ..Default::default()
        };
        let codex = command(
            &playbook("codex", &["read", "edit"]),
            &codex_opts,
            "Fix it",
            "",
            true,
        )
        .unwrap();
        assert_eq!(codex.get_program(), "/opt/codex");
        let line = args(&codex);
        assert!(
            line.starts_with("exec --json --sandbox workspace-write"),
            "{line}"
        );
        assert!(
            line.contains("-p work") && line.ends_with("Fix it"),
            "{line}"
        );
        assert!(line.contains("sandbox_workspace_write.network_access=false"));
    }

    #[test]
    fn every_agent_gets_its_edit_mode() {
        let opts = AgentOptions::default();
        let edit = |provider: &str| {
            args(
                &command(
                    &playbook(provider, &["read", "edit"]),
                    &opts,
                    "Go",
                    "",
                    true,
                )
                .unwrap(),
            )
        };
        assert!(edit("droid").contains("--auto medium"));
        assert!(edit("gemini").contains("--approval-mode auto_edit"));
        assert!(edit("copilot").contains("--allow-all-tools"));
        assert!(!edit("opencode").contains("--agent plan"));
        let read = args(&command(&playbook("opencode", &["read"]), &opts, "Go", "", true).unwrap());
        assert!(read.contains("--agent plan"));
        // Amp has no read-only mode, so a read-only playbook doesn't run on it.
        assert!(command(&playbook("amp", &["read"]), &opts, "Go", "", true).is_err());
    }

    #[test]
    fn the_reply_is_read_from_structured_output() {
        let out = r#"{"type":"thread.started","thread_id":"t-1"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"All good."}}
{"type":"turn.completed","usage":{"input_tokens":5,"cached_input_tokens":0,"output_tokens":2}}"#;
        assert_eq!(
            read_output("codex", "/w", out),
            ("All good.".to_string(), None, "t-1".to_string())
        );
        assert_eq!(
            read_output("grok", "/w", " Plain reply \n"),
            ("Plain reply".to_string(), None, String::new())
        );
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
