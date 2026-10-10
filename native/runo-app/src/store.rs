//! Persistent projects and sessions, stored as JSON under `~/.runo/state.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Agent,
    System,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Idle,
    Working,
    Done,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
    /// Completed tool steps shown above the reply (e.g. "Explored 8 files").
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub time: String,
}

impl Session {
    /// Current selections, including values saved by older versions.
    pub fn selections(&self) -> HashMap<String, String> {
        let mut sel = self.opts.clone();
        if !sel.contains_key("model") && !self.model.is_empty() {
            sel.insert("model".into(), self.model.clone());
        }
        if !sel.contains_key("mode") && !self.mode.is_empty() {
            sel.insert("mode".into(), self.mode.clone());
        }
        sel
    }
}

impl Message {
    pub fn new(role: Role, text: impl Into<String>) -> Self {
        Self { role, text: text.into(), steps: Vec::new(), time: String::new() }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub path: String,
}

fn default_mode() -> String {
    "edit".into()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: u64,
    pub project_id: u64,
    pub title: String,
    pub provider: String,
    pub branch: String,
    /// Directory the agent and terminal run in (a worktree, or the project itself).
    pub dir: String,
    pub isolated: bool,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub model: String,
    /// Per-session choices for the agent's discovered options (model, effort, mode, tier…).
    #[serde(default)]
    pub opts: HashMap<String, String>,
    pub status: Status,
    pub messages: Vec<Message>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub next_id: u64,
    pub projects: Vec<Project>,
    pub sessions: Vec<Session>,
}

fn state_path() -> PathBuf {
    let home = std::env::var_os("RUNO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".runo"))
        })
        .unwrap_or_else(|| PathBuf::from(".runo"));
    home.join("state.json")
}

impl State {
    pub fn load() -> Self {
        let mut state: State = std::fs::read_to_string(state_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        // A process cannot survive an app restart; do not show it as running.
        for session in &mut state.sessions {
            if session.status == Status::Working {
                session.status = Status::Failed;
                session.messages.push(Message::new(Role::System, "Interrupted: Runo closed while this run was active."));
            }
        }
        state
    }

    pub fn save(&self) {
        let path = state_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(tmp, path);
            }
        }
    }

    pub fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn session(&self, id: u64) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn session_mut(&mut self, id: u64) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }
}

impl State {
    /// Sample workspace shown until the first real project is added.
    pub fn demo() -> Self {
        let project = |id: u64, name: &str| Project { id, name: name.into(), path: format!("/demo/{name}") };
        let session = |id, project_id, title: &str, provider: &str, branch: &str, status, messages| Session {
            id,
            project_id,
            title: title.into(),
            provider: provider.into(),
            branch: branch.into(),
            dir: String::new(),
            isolated: true,
            mode: default_mode(),
            model: String::new(),
            opts: HashMap::new(),
            status,
            messages,
        };
        let nav = vec![
            Message { time: "10:24 AM".into(), ..Message::new(Role::User, "Refine the workspace navigation. Keep it compact, intuitive, and consistent across desktop platforms.") },
            Message {
                time: "10:24 AM".into(),
                steps: vec!["Explored 8 files".into(), "Updated 4 files".into(), "Build passed · 0.97s".into()],
                ..Message::new(Role::Agent, "I've simplified the sidebar and given your workspace more breathing room.\n\nWhat changed\n•  Projects and sessions in one sidebar\n•  Clearer running and review states\n•  Consistent keyboard navigation\n\nReady for your review.")
            },
        ];
        State {
            next_id: 100,
            projects: vec![
                project(1, "runo-native"),
                project(2, "Worldigo"),
                project(3, "trademe"),
                project(4, "scalpy"),
                project(5, "botoxy"),
            ],
            sessions: vec![
                session(11, 1, "Refine navigation", "claude", "feature/navigation", Status::Working, nav.clone()),
                session(12, 1, "Search indexing", "codex", "feature/search", Status::Idle, nav.clone()),
                session(13, 1, "Fix build warnings", "opencode", "chore/build", Status::Done, nav.clone()),
                session(21, 2, "main", "claude", "main", Status::Done, nav.clone()),
                session(31, 3, "Scalping backtest", "claude", "indiana-scalping-backtest", Status::Done, nav.clone()),
                session(41, 4, "Update dashboard", "codex", "feat/dashboard", Status::Done, nav.clone()),
                session(51, 5, "Fix webhook retry", "opencode", "fix/webhooks", Status::Done, nav),
            ],
        }
    }
}
