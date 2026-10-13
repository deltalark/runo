//! Persistent projects and sessions, stored as JSON under `~/.runo/state.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Agent,
    System,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Status {
    #[default]
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
    /// User turns: a snapshot commit of the worktree taken just before the agent ran,
    /// so the turn can be undone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checkpoint: String,
    /// Agent turns: "kept" or "undone" once the person decided on the turn's changes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resolved: String,
    /// Agent turns that were undone: a snapshot from just before the undo, for Redo.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub redo: String,
    /// User turns: images attached to the prompt (absolute paths).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// Agent turns: token use reported by the CLI.
    #[serde(default, skip_serializing_if = "Usage::is_empty")]
    pub usage: Usage,
    /// Agent turns: tools the agent asked for but was not allowed to use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied: Vec<String>,
    /// Agent turns: a plan the agent proposed (plan mode), shown as a card.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan: String,
    /// Agent turns: questions the agent asked, each with its options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<Question>,
    /// User turns: the playbook this message ran. `text` holds the full prompt sent;
    /// the conversation shows the playbook's name instead.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub playbook: String,
    /// User turns run from a playbook: what the person typed with it, if anything.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Token use of one agent turn.
#[derive(Clone, Copy, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    /// Tokens of the prompt read from or written to the cache; they fill the context too.
    pub cached: u64,
    /// Total cost in US cents, when the CLI reports it.
    pub cost_cents: f64,
    /// The model's context window, when the CLI reports it.
    #[serde(default)]
    pub window: u64,
}

impl Usage {
    pub fn is_empty(&self) -> bool {
        self.input == 0 && self.output == 0 && self.cached == 0
    }

    /// Tokens in the context after this turn.
    pub fn context(&self) -> u64 {
        self.input + self.cached + self.output
    }
}

/// A multiple-choice question the agent asked (Claude's AskUserQuestion tool).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub options: Vec<String>,
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
    /// A turn stopped after it ran tools but before it wrote anything still counts: the CLI
    /// saved that conversation.
    pub fn has_conversation(&self) -> bool {
        self.messages.iter().skip(self.conversation_start).any(|m| {
            (m.role == Role::Agent && (!m.text.is_empty() || !m.steps.is_empty()))
                || (m.role == Role::System && m.meta.starts_with("cli:"))
        })
    }

    /// Switch to another agent CLI. Its options and conversation start fresh.
    pub fn switch_provider(&mut self, provider: &str) {
        if self.provider == provider {
            return;
        }
        self.switched_from = None;
        self.provider = provider.to_string();
        self.opts.clear();
        self.model.clear();
        self.mode.clear();
        self.agent_session.clear();
        self.conversation_start = self.messages.len();
    }

    /// Switch to another agent, remembering the one it replaces so `switch_back` can
    /// restore it while nothing has been sent yet.
    pub fn switch_provider_keeping(&mut self, provider: &str) {
        if self.provider == provider {
            return;
        }
        let from = SwitchedFrom {
            provider: self.provider.clone(),
            agent_session: self.agent_session.clone(),
            model: self.model.clone(),
            mode: self.mode.clone(),
            opts: self.opts.clone(),
            conversation_start: self.conversation_start,
            carried: self.carried.clone(),
        };
        self.switch_provider(provider);
        self.switched_from = Some(from);
    }

    /// The agent a switch can still be undone to: nothing was sent since the switch.
    pub fn undoable_switch(&self) -> Option<&SwitchedFrom> {
        let from = self.switched_from.as_ref()?;
        let sent = self.messages[self.conversation_start.min(self.messages.len())..]
            .iter()
            .any(|m| m.role != Role::System);
        (!sent).then_some(from)
    }

    /// Undo the last switch: the earlier agent resumes its own conversation, its settings
    /// come back, and the pending handoff and switch divider are dropped.
    pub fn switch_back(&mut self) -> bool {
        let Some(from) = self.undoable_switch().cloned() else {
            return false;
        };
        let start = self.conversation_start.min(self.messages.len());
        if let Some(i) = self.messages[start.saturating_sub(1)..]
            .iter()
            .rposition(|m| m.role == Role::System && m.meta.starts_with("switch:"))
        {
            self.messages.remove(start.saturating_sub(1) + i);
        }
        self.provider = from.provider;
        self.agent_session = from.agent_session;
        self.model = from.model;
        self.mode = from.mode;
        self.opts = from.opts;
        self.conversation_start = from.conversation_start.min(self.messages.len());
        self.handoff.clear();
        self.carried = from.carried;
        self.switched_from = None;
        true
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
            checkpoint: String::new(),
            resolved: String::new(),
            redo: String::new(),
            images: Vec::new(),
            usage: Usage::default(),
            denied: Vec::new(),
            plan: String::new(),
            questions: Vec::new(),
            playbook: String::new(),
            note: String::new(),
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

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    pub path: String,
    /// Accent color key from the New project popup ("" for none).
    #[serde(default)]
    pub color: String,
    /// Icon key from the New project popup ("" for the plain folder).
    #[serde(default)]
    pub icon: String,
    /// Where new sessions in this project run by default: a connection id, or 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub connection: u64,
    /// Command run in each new worktree of this project before its agent starts; empty
    /// uses the one in Settings.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub setup: String,
}

fn default_mode() -> String {
    "edit".into()
}

#[derive(Clone, Default, Serialize, Deserialize)]
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
    /// Review notes left on diff lines, waiting to be sent to the agent.
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Sessions started together on one task share the id of the first one; 0 if none.
    #[serde(default)]
    pub race: u64,
    /// Prompts typed while the agent was working; each is sent when the turn ends.
    #[serde(default)]
    pub queue: Vec<Queued>,
    /// A handoff brief from the previous agent, sent with the next prompt.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub handoff: String,
    /// The last handoff, kept for agents that cannot resume their own conversation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub carried: String,
    /// Shown in the CLI view: the agent's own CLI on the same conversation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cli: bool,
    /// The agent this session just switched away from, kept until the next message so
    /// switching straight back resumes its own conversation instead of starting over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switched_from: Option<SwitchedFrom>,
    /// Runs inside its own Cove VM (`runo-<id>`) instead of directly on this machine.
    /// Replaced by `connection`; kept so older saves load.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cove: bool,
    /// Where the session runs: a connection's id, or 0 for this machine.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub connection: u64,
    /// The running turn is on a connection, where it keeps going while Runo is closed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote_turn: bool,
    /// The branch this session's branch started from and lands in, such as `main`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base: String,
    /// The pull request opened for this session's branch.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pr_url: String,
    /// The newest snapshot of the folder Runo took (end of a turn, an undo or a redo),
    /// so Undo can leave alone what changed after it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snapshot: String,
    /// Told to the agent with the next prompt, such as that its last turn was undone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub undo_note: String,
    /// Connections it ran on before, whose copies are deleted along with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_on: Vec<u64>,
    /// The playbook that started it, or 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub playbook: u64,
    /// What that playbook declared it may do (`read`, `edit`, `shell`, `network`); every
    /// turn of the session keeps to it. Empty means no limits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<String>,
}

/// A prompt waiting for the agent, with the images attached to it.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
#[serde(from = "SavedQueued")]
pub struct Queued {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl Queued {
    pub fn new(text: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            text: text.into(),
            images,
        }
    }
}

/// Older saves keep queued prompts as plain strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum SavedQueued {
    Text(String),
    Full {
        text: String,
        #[serde(default)]
        images: Vec<String>,
    },
}

impl From<SavedQueued> for Queued {
    fn from(saved: SavedQueued) -> Self {
        match saved {
            SavedQueued::Text(text) => Queued::new(text, Vec::new()),
            SavedQueued::Full { text, images } => Queued::new(text, images),
        }
    }
}

/// What a switch replaced, enough to undo it before anything is sent.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct SwitchedFrom {
    pub provider: String,
    pub agent_session: String,
    pub model: String,
    pub mode: String,
    pub opts: HashMap<String, String>,
    pub conversation_start: usize,
    pub carried: String,
}

impl Session {
    /// How to run things on the session's connection, unless it runs on this machine.
    pub fn sandbox(&self) -> Option<crate::connections::Sandbox> {
        let conn = crate::connections::get(self.connection)?;
        let folder = std::path::Path::new(&self.dir)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(crate::connections::Sandbox::for_session(
            &conn, self.id, &folder, &self.dir,
        ))
    }

    /// Runs somewhere other than this machine.
    pub fn elsewhere(&self) -> bool {
        self.connection != crate::connections::LOCAL
    }
}

/// A review comment on one line of the session's diff.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Note {
    pub path: String,
    pub line: usize,
    /// The diff line as shown, with its +/− marker, so the agent sees what was meant.
    pub code: String,
    pub text: String,
}

/// One follow-up prompt that asks the agent to address every note.
pub fn notes_prompt(notes: &[Note]) -> String {
    let mut out = String::from("Please address these review notes on your changes:\n");
    for (i, note) in notes.iter().enumerate() {
        // Removed lines only exist in the old file, so their number is the old one.
        let place = if note.code.starts_with('−') {
            format!(
                "{}, line {} of the old version (removed)",
                note.path, note.line
            )
        } else {
            format!("{}:{}", note.path, note.line)
        };
        out.push_str(&format!(
            "\n{}. `{place}`: {}\n   On the line `{}`\n",
            i + 1,
            note.text.trim(),
            note.code.trim()
        ));
    }
    out
}

#[derive(Default, Serialize, Deserialize)]
pub struct State {
    pub next_id: u64,
    pub projects: Vec<Project>,
    pub sessions: Vec<Session>,
    /// Free-form notes per project id.
    #[serde(default)]
    pub notes: HashMap<u64, String>,
    /// Pinning, sharing with agents and last edit of each project's notes.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub notes_meta: HashMap<u64, NoteMeta>,
    /// Recent events across sessions, newest last.
    #[serde(default)]
    pub activity: Vec<Activity>,
    #[serde(default)]
    pub automations: Vec<Automation>,
    /// Open tabs and panels, restored on the next launch.
    #[serde(default)]
    pub layout: Layout,
    /// Cost reported by the agents per day ("%Y-%m-%d"), in US cents; the last 31 days.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub spend: std::collections::BTreeMap<String, f64>,
}

/// How a project's notes are kept and used.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct NoteMeta {
    /// Listed first on the Project notes page.
    #[serde(default)]
    pub pinned: bool,
    /// Sent to the agent at the start of each new conversation in the project.
    #[serde(default)]
    pub shared: bool,
    /// Unix seconds of the last edit.
    #[serde(default)]
    pub updated: u64,
}

/// Something that happened in a session, for the Activity feed.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Activity {
    /// Local time, "%Y-%m-%d %H:%M".
    pub time: String,
    pub session: u64,
    pub text: String,
}

/// A playbook: a saved task (instructions and agent) that runs in any project on demand,
/// and optionally on a schedule in one project.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, Default)]
pub struct Automation {
    pub id: u64,
    pub name: String,
    /// The project it always runs in, or 0 to run in whichever project it is started from.
    pub project_id: u64,
    pub provider: String,
    pub prompt: String,
    /// One line on what it does, for menus and agents; empty uses the prompt's first sentence.
    #[serde(default)]
    pub description: String,
    /// Minutes between scheduled runs; 0 runs it only when started by hand.
    pub every_minutes: u64,
    /// Unix seconds of the last run; 0 if it never ran.
    #[serde(default)]
    pub last_run: u64,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Unix seconds it last actually ran. `last_run` also moves when a
    /// schedule starts counting, so it can't say whether it ever ran.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ran: u64,
    /// Who wrote it and which version, for playbooks installed from a file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Where it was installed from (a link, repository or file) and the file inside it,
    /// so it can be updated; empty for playbooks made in Runo.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// What it may do (`read`, `edit`, `shell`, `network`); empty means not declared,
    /// which runs like any session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
    /// Labels and defaults for its `{{key}}` placeholders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<crate::pbfile::Input>,
    /// Its latest runs, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<PlaybookRun>,
    /// A fingerprint of the playbook as it was installed, to tell edits made here from
    /// changes at the source.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub installed: String,
}

/// One run of a playbook, for its history.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug, Default)]
pub struct PlaybookRun {
    /// Unix seconds it started, or was due.
    pub at: u64,
    /// The session it ran in, or 0 when it didn't start.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub session: u64,
    /// "running", "done", "failed", "skipped" or "not started".
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Runs kept in a playbook's history.
const RUNS_KEPT: usize = 20;

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn yes() -> bool {
    true
}

impl Automation {
    /// The values it asks for when it runs.
    pub fn run_inputs(&self) -> Vec<crate::pbfile::Input> {
        crate::pbfile::inputs_of(&self.inputs, &self.prompt)
    }

    /// Add a run to the history, dropping the oldest past the limit.
    pub fn record(&mut self, run: PlaybookRun) {
        self.runs.push(run);
        if self.runs.len() > RUNS_KEPT {
            let extra = self.runs.len() - RUNS_KEPT;
            self.runs.drain(..extra);
        }
    }

    /// What it is, for telling whether it changed: everything a playbook file holds.
    pub fn fingerprint(&self) -> String {
        let file = crate::pbfile::PlaybookFile {
            author: String::new(),
            version: String::new(),
            ..self.to_file()
        };
        crate::pbfile::render(&file)
    }

    /// Edited here since it was installed.
    pub fn edited_here(&self) -> bool {
        !self.installed.is_empty() && self.installed != self.fingerprint()
    }

    /// Whether it declared that it may do `what`; undeclared playbooks may do anything.
    pub fn may(&self, what: &str) -> bool {
        self.permissions.is_empty() || self.permissions.iter().any(|p| p == what)
    }

    /// As a shareable file.
    pub fn to_file(&self) -> crate::pbfile::PlaybookFile {
        crate::pbfile::PlaybookFile {
            name: self.name.clone(),
            description: self.description.clone(),
            author: self.author.clone(),
            version: self.version.clone(),
            agent: self.provider.clone(),
            permissions: if self.permissions.is_empty() {
                crate::pbfile::PERMISSIONS
                    .iter()
                    .map(|p| p.0.to_string())
                    .collect()
            } else {
                self.permissions.clone()
            },
            inputs: self.run_inputs(),
            prompt: self.prompt.clone(),
        }
    }

    /// When it last ran, 0 if never. Older playbooks only have `last_run`,
    /// which for one run by hand is the same thing.
    pub fn ran_at(&self) -> u64 {
        if self.ran > 0 {
            self.ran
        } else if self.scheduled() {
            0
        } else {
            self.last_run
        }
    }

    /// The description, or the prompt's first sentence when there is none.
    pub fn summary(&self) -> String {
        let text = if self.description.trim().is_empty() {
            let p = self.prompt.trim();
            match p.find(". ") {
                Some(end) => &p[..=end],
                None => p.lines().next().unwrap_or(""),
            }
        } else {
            self.description.trim()
        };
        let mut out: String = text.chars().take(100).collect();
        if text.chars().count() > 100 {
            out.push('…');
        }
        out
    }

    /// A name usable as a slash command or MCP prompt: `Find bugs` → `find-bugs`.
    pub fn slug(&self) -> String {
        let mut out = String::new();
        for c in self.name.trim().chars() {
            if c.is_alphanumeric() {
                out.extend(c.to_lowercase());
            } else if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
        }
        while out.ends_with('-') {
            out.pop();
        }
        if out.is_empty() {
            format!("playbook-{}", self.id)
        } else {
            out
        }
    }

    /// Whether the automation should run now.
    pub fn due(&self, now: u64) -> bool {
        self.scheduled() && self.enabled && now >= self.last_run + self.every_minutes * 60
    }

    /// Runs on a timer as well as by hand. A schedule needs a project to run in.
    pub fn scheduled(&self) -> bool {
        self.every_minutes > 0 && self.project_id != 0
    }
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Layout {
    pub tabs: Vec<u64>,
    pub selected: Option<u64>,
    #[serde(default)]
    pub hide_sidebar: bool,
    #[serde(default)]
    pub hide_panel: bool,
    /// The mode on the rail, by its label ("Home", "Sessions", ...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    /// The session open beside the selected one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<u64>,
    /// The right panel's tab: "changes", "browser" or "files".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub right_tab: String,
    /// Projects open in the sidebar; `None` (files from before) opens them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<Vec<u64>>,
    /// The project in the top bar's switcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<u64>,
    /// Sidebar width in pixels; 0 is the default.
    #[serde(default)]
    pub sidebar_width: f32,
    /// Home lists every project instead of the one in the top bar.
    #[serde(default)]
    pub home_all: bool,
    /// Where the window was and how big.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowPlace>,
}

/// The window's last position and size, in logical pixels.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct WindowPlace {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

/// The saved window place, read before the rest of the state so the window opens there.
pub fn saved_window() -> Option<WindowPlace> {
    #[derive(Deserialize, Default)]
    struct Only {
        #[serde(default)]
        layout: Layout,
    }
    let text = std::fs::read_to_string(state_path()).ok()?;
    let only: Only = serde_json::from_str(&text).ok()?;
    only.layout
        .window
        .filter(|w| w.width >= 400. && w.height >= 300.)
}

/// Today's date as the spend log keys it.
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl State {
    /// Add a turn's cost to `day`, keeping a month of days.
    pub fn add_spend(&mut self, day: &str, cents: f64) {
        if cents <= 0.0 {
            return;
        }
        *self.spend.entry(day.to_string()).or_default() += cents;
        while self.spend.len() > 31 {
            let first = self.spend.keys().next().cloned().unwrap_or_default();
            self.spend.remove(&first);
        }
    }

    /// Record an event for the Activity feed, keeping the latest 500.
    pub fn log(&mut self, session: u64, text: impl Into<String>) {
        self.activity.push(Activity {
            time: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            session,
            text: text.into(),
        });
        if self.activity.len() > 500 {
            self.activity.drain(..100);
        }
    }
}

/// Where agents and the `runo` command leave requests for the running app.
pub fn inbox_dir() -> PathBuf {
    data_dir().join("inbox")
}

/// A request to start a playbook, left in the inbox by Runo's MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybookRequest {
    pub playbook: u64,
    pub project: u64,
    /// Who asked, such as "claude-code".
    pub from: String,
    /// The folder of the session that asked.
    #[serde(default)]
    pub dir: String,
}

/// Leave a request in `inbox`. Written under a temporary name and renamed, so the app
/// never reads half a file.
pub fn send_request(inbox: &Path, request: &PlaybookRequest) -> std::io::Result<()> {
    std::fs::create_dir_all(inbox)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{stamp}-{}", std::process::id());
    let tmp = inbox.join(format!("{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(request)?)?;
    std::fs::rename(tmp, inbox.join(format!("{name}.json")))
}

/// Take every request waiting in `inbox`, oldest first, removing them.
pub fn take_requests(inbox: &Path) -> Vec<PlaybookRequest> {
    let Ok(entries) = std::fs::read_dir(inbox) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read(&path).ok();
            let _ = std::fs::remove_file(&path);
            serde_json::from_slice(&text?).ok()
        })
        .collect()
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
        // A turn on a connection keeps going there; Runo picks it up again on start.
        for session in &mut state.sessions {
            if session.status == Status::Working && !session.remote_turn {
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
            ..Default::default()
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
                notes: Vec::new(),
                race: 0,
                ..Default::default()
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
                project(1, "runo-app"),
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
            ..Default::default()
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
    fn turns_on_connections_stay_running_across_restarts() {
        let dir = temp("remote-turn");
        let path = dir.join("state.json");
        let mut state = State::demo();
        let session = state.session_mut(11).unwrap();
        session.connection = 3;
        session.remote_turn = true;
        let before = session.messages.len();
        write_json(&path, &state);
        let loaded = State::load_from(&path);
        let session = loaded.session(11).unwrap();
        assert!(session.status == Status::Working);
        assert_eq!(session.messages.len(), before);
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
        // A first turn stopped after it ran tools keeps its conversation.
        session.switch_provider("codex");
        let mut stopped = Message::new(Role::Agent, "");
        stopped.steps.push("Read src/main.rs".into());
        session.messages.push(stopped);
        assert!(session.has_conversation());
    }

    #[test]
    fn queued_prompts_load_from_older_saves() {
        let old: Vec<Queued> =
            serde_json::from_str(r#"["Fix it",{"text":"Look","images":["a.png"]}]"#).unwrap();
        assert_eq!(old[0], Queued::new("Fix it", Vec::new()));
        assert_eq!(old[1].images, ["a.png"]);
        let saved = serde_json::to_string(&old[0]).unwrap();
        assert_eq!(saved, r#"{"text":"Fix it"}"#);
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

    #[test]
    fn notes_become_one_prompt() {
        let notes = vec![
            Note {
                path: "src/a.rs".into(),
                line: 12,
                code: "+    let x = 1;  ".into(),
                text: "Name this better".into(),
            },
            Note {
                path: "b.css".into(),
                line: 3,
                code: "−color: red;".into(),
                text: " Keep red here ".into(),
            },
        ];
        assert_eq!(
            notes_prompt(&notes),
            "Please address these review notes on your changes:\n\n1. `src/a.rs:12`: Name this better\n   On the line `+    let x = 1;`\n\n2. `b.css, line 3 of the old version (removed)`: Keep red here\n   On the line `−color: red;`\n"
        );
    }

    #[test]
    fn old_layouts_still_load() {
        let layout: Layout = serde_json::from_str(r#"{"tabs":[3],"selected":3}"#).unwrap();
        assert_eq!(layout.tabs, vec![3]);
        assert_eq!(layout.expanded, None);
        assert!(layout.mode.is_empty() && layout.window.is_none());
        let full = Layout {
            mode: "Home".into(),
            expanded: Some(vec![1, 2]),
            window: Some(WindowPlace {
                x: 10.,
                y: 20.,
                width: 1200.,
                height: 800.,
                maximized: false,
            }),
            ..Layout::default()
        };
        let back: Layout = serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(back, full);
    }

    #[test]
    fn spend_adds_up_per_day_and_keeps_a_month() {
        let mut state = State::default();
        state.add_spend("2026-10-13", 12.5);
        state.add_spend("2026-10-13", 7.5);
        state.add_spend("2026-10-13", 0.0);
        assert_eq!(state.spend.get("2026-10-13"), Some(&20.0));
        for day in 1..=40 {
            state.add_spend(&format!("2026-11-{day:02}"), 1.0);
        }
        assert_eq!(state.spend.len(), 31);
        assert!(!state.spend.contains_key("2026-10-13"));
    }
}
