//! Persistent projects and sessions, stored as JSON under `~/.runo/state.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    /// Run summary shown under the reply, such as "12s · 3 turns · $0.04".
    #[serde(default)]
    pub meta: String,
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

impl Session {
    /// Whether the current agent already replied, so the next prompt continues its conversation.
    pub fn has_conversation(&self) -> bool {
        self.messages
            .iter()
            .skip(self.conversation_start)
            .any(|m| m.role == Role::Agent && !m.text.is_empty())
    }

    /// Switch to another agent CLI. Its options and conversation start fresh.
    pub fn switch_provider(&mut self, provider: &str) {
        if self.provider == provider {
            return;
        }
        self.provider = provider.to_string();
        self.opts.clear();
        self.model.clear();
        self.mode.clear();
        self.agent_session.clear();
        self.conversation_start = self.messages.len();
    }
}

impl Message {
    pub fn new(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            steps: Vec::new(),
            time: String::new(),
            meta: String::new(),
        }
    }

    /// A message stamped with the current local time, for the conversation header.
    pub fn now(role: Role, text: impl Into<String>) -> Self {
        Self {
            time: chrono::Local::now().format("%-I:%M %p").to_string(),
            ..Self::new(role, text)
        }
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
    /// The agent CLI's own conversation id (Claude Code `--session-id`), so follow-ups
    /// resume this session's conversation and not whichever ran last in the folder.
    #[serde(default)]
    pub agent_session: String,
    /// Index of the first message of the current agent conversation. Switching agents
    /// starts a new conversation, so earlier replies must not trigger a resume.
    #[serde(default)]
    pub conversation_start: usize,
    /// Pinned sessions sort to the top of their project.
    #[serde(default)]
    pub pinned: bool,
    /// Archived sessions leave the sidebar but stay under All sessions.
    #[serde(default)]
    pub archived: bool,
    /// The agent finished while this session was not on screen.
    #[serde(default)]
    pub unread: bool,
}

#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub next_id: u64,
    pub projects: Vec<Project>,
    pub sessions: Vec<Session>,
}

/// Read a JSON file, or the default when it is missing. A file that exists but cannot be
/// parsed is moved aside first, so the next save cannot silently overwrite the user's data.
pub fn read_json_or_backup<T: Default + serde::de::DeserializeOwned>(path: &Path) -> T {
    let Ok(text) = std::fs::read_to_string(path) else {
        return T::default();
    };
    match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup = path.with_extension(format!("json.corrupt-{stamp}"));
            let _ = std::fs::rename(path, &backup);
            eprintln!(
                "runo: could not read {} ({error}); moved it to {}",
                path.display(),
                backup.display()
            );
            T::default()
        }
    }
}

/// Write JSON atomically: to a temporary file first, then rename over the target.
pub fn write_json<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

/// Runo's data folder: `$RUNO_HOME`, else `~/.runo`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("RUNO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".runo"))
        })
        .unwrap_or_else(|| PathBuf::from(".runo"))
}

fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

impl State {
    pub fn load() -> Self {
        Self::load_from(&state_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let mut state: State = read_json_or_backup(path);
        // A process cannot survive an app restart; do not show it as running.
        for session in &mut state.sessions {
            if session.status == Status::Working {
                session.status = Status::Failed;
                session.messages.push(Message::new(
                    Role::System,
                    "Interrupted: Runo closed while this run was active.",
                ));
            }
        }
        state
    }

    pub fn save(&self) {
        write_json(&state_path(), self);
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
        let project = |id: u64, name: &str| Project {
            id,
            name: name.into(),
            path: format!("/demo/{name}"),
        };
        let session =
            |id, project_id, title: &str, provider: &str, branch: &str, status, messages| Session {
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
                agent_session: String::new(),
                conversation_start: 0,
                pinned: false,
                archived: false,
                unread: false,
            };
        let nav = vec![
            Message { time: "10:24 AM".into(), ..Message::new(Role::User, "Refine the workspace navigation. Keep it compact, intuitive, and consistent across desktop platforms.") },
            Message {
                time: "10:24 AM".into(),
                steps: vec!["Explored 8 files".into(), "Updated 4 files".into(), "Build passed · 0.97s".into()],
                ..Message::new(Role::Agent, "I've simplified the sidebar and given your workspace more breathing room.\n\n**What changed**\n\n- Projects and sessions in one sidebar\n- Clearer running and review states\n- Consistent keyboard navigation\n\nReady for your review.")
            },
        ];
        let mut state = State {
            next_id: 100,
            projects: vec![
                project(1, "runo-native"),
                project(2, "Worldigo"),
                project(3, "trademe"),
                project(4, "scalpy"),
                project(5, "botoxy"),
            ],
            sessions: vec![
                session(
                    11,
                    1,
                    "Refine navigation",
                    "claude",
                    "feature/navigation",
                    Status::Working,
                    nav.clone(),
                ),
                session(
                    12,
                    1,
                    "Search indexing",
                    "codex",
                    "feature/search",
                    Status::Idle,
                    nav.clone(),
                ),
                session(
                    13,
                    1,
                    "Fix build warnings",
                    "opencode",
                    "chore/build",
                    Status::Done,
                    nav.clone(),
                ),
                session(21, 2, "main", "claude", "main", Status::Done, nav.clone()),
                session(
                    31,
                    3,
                    "Scalping backtest",
                    "claude",
                    "indiana-scalping-backtest",
                    Status::Done,
                    nav.clone(),
                ),
                session(
                    41,
                    4,
                    "Update dashboard",
                    "codex",
                    "feat/dashboard",
                    Status::Done,
                    nav.clone(),
                ),
                session(
                    51,
                    5,
                    "Fix webhook retry",
                    "opencode",
                    "fix/webhooks",
                    Status::Done,
                    nav,
                ),
            ],
        };
        // Show the inbox and pinning in the demo.
        state.sessions[1].unread = true;
        state.sessions[2].pinned = true;
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_and_loads_round_trip() {
        let dir = temp("roundtrip");
        let path = dir.join("state.json");
        let mut state = State::demo();
        state.sessions[1].agent_session = "abc".into();
        write_json(&path, &state);
        let loaded = State::load_from(&path);
        assert_eq!(loaded.projects.len(), state.projects.len());
        assert_eq!(loaded.sessions.len(), state.sessions.len());
        assert_eq!(loaded.next_id, state.next_id);
        assert_eq!(loaded.session(12).unwrap().agent_session, "abc");
        assert!(!dir.join("state.json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn running_sessions_load_as_interrupted() {
        let dir = temp("interrupted");
        let path = dir.join("state.json");
        write_json(&path, &State::demo());
        let loaded = State::load_from(&path);
        let session = loaded.session(11).unwrap();
        assert!(session.status == Status::Failed);
        assert!(session
            .messages
            .last()
            .unwrap()
            .text
            .starts_with("Interrupted"));
        assert!(loaded.sessions.iter().all(|s| s.status != Status::Working));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_state_is_moved_aside_not_overwritten() {
        let dir = temp("corrupt");
        let path = dir.join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        let loaded = State::load_from(&path);
        assert!(loaded.projects.is_empty());
        assert!(!path.exists());
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("state.json.corrupt-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(backups[0].path()).unwrap(),
            "{ not json"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_state_is_empty() {
        let dir = temp("missing");
        let loaded = State::load_from(&dir.join("nope.json"));
        assert!(loaded.projects.is_empty() && loaded.sessions.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn older_state_files_still_load() {
        let json = r#"{"next_id":3,"projects":[{"id":1,"name":"p","path":"/p"}],
            "sessions":[{"id":2,"project_id":1,"title":"t","provider":"claude","branch":"b",
            "dir":"/p","isolated":false,"model":"opus","status":"Done",
            "messages":[{"role":"User","text":"hi"}]}]}"#;
        let state: State = serde_json::from_str(json).unwrap();
        let session = state.session(2).unwrap();
        assert_eq!(session.mode, "edit");
        assert_eq!(
            session.selections().get("model").map(String::as_str),
            Some("opus")
        );
        assert_eq!(
            session.selections().get("mode").map(String::as_str),
            Some("edit")
        );
        assert!(session.agent_session.is_empty());
    }

    #[test]
    fn switching_agents_starts_a_new_conversation() {
        let mut state = State::demo();
        let session = state.session_mut(12).unwrap();
        session.opts.insert("model".into(), "gpt".into());
        session.agent_session = "id".into();
        assert!(session.has_conversation());
        session.switch_provider("codex");
        assert!(
            session.has_conversation(),
            "same provider keeps the conversation"
        );
        session.switch_provider("claude");
        assert!(!session.has_conversation());
        assert!(session.opts.is_empty() && session.agent_session.is_empty());
        session.messages.push(Message::new(Role::Agent, ""));
        assert!(
            !session.has_conversation(),
            "an empty reply is not a conversation"
        );
        session.messages.last_mut().unwrap().text = "done".into();
        assert!(session.has_conversation());
    }

    #[test]
    fn ids_increase() {
        let mut state = State::default();
        assert_eq!(state.next_id(), 1);
        assert_eq!(state.next_id(), 2);
    }

    #[test]
    fn stamped_messages_have_a_time() {
        assert!(Message::now(Role::User, "x").time.contains(':'));
        assert!(Message::new(Role::User, "x").time.is_empty());
    }
}
