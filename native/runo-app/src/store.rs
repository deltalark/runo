//! Persistent projects and sessions, stored as JSON under `~/.runo` (see `persist`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// Saved enums take a value they don't know, as a newer Runo may have written it, as
// their plainest variant (`serde(other)`), so going back to an older build keeps the
// session instead of failing to read it.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Agent,
    #[serde(other)]
    System,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Status {
    #[default]
    #[serde(other)]
    Idle,
    Working,
    Done,
    Failed,
    /// The person stopped the turn on purpose: not an error.
    Stopped,
}

impl Status {
    /// The word shown for a session's status, everywhere it is shown.
    pub fn label(self) -> &'static str {
        match self {
            Status::Idle => "Idle",
            Status::Working => "Working",
            Status::Done => "Completed",
            Status::Failed => "Failed",
            Status::Stopped => "Stopped",
        }
    }
}

pub use crate::harness::{StepState, ToolStep};

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
    /// Agent turns: the tool calls shown above the reply, such as "Read src/main.rs",
    /// each with its kind and how it stands. Sessions saved when steps were plain labels
    /// load those as finished steps.
    #[serde(default, deserialize_with = "steps_or_labels")]
    pub steps: Vec<ToolStep>,
    #[serde(default)]
    pub time: String,
    /// Run summary shown under the reply, such as "12s · 3 turns · $0.04".
    #[serde(default)]
    pub meta: String,
    /// User turns: a snapshot commit of the worktree taken just before the agent ran,
    /// so the turn can be undone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checkpoint: String,
    /// Agent turns: the files the turn changed, found at its end by comparing the folder
    /// with the checkpoint taken before it ran. Its files changed card lists only these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
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
    /// In a Crew chat: a card drawn with the message, such as the threads it started.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "or_none"
    )]
    pub crew: Option<crate::crew::Card>,
    /// User turns in a Crew chat: the thread whose message woke Crew, drawn as a chip
    /// naming the thread instead of as something the person typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_thread: Option<u64>,
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
    /// Current selections: model, effort, mode, tier and the rest.
    pub fn selections(&self) -> HashMap<String, String> {
        self.opts.clone()
    }

    /// For a session that works in a folder it doesn't have to itself, the files its
    /// agent changed, turn by turn. Anything else uncommitted there came from somewhere
    /// else: the person, another session, or Runo's Design and Architecture modes. An
    /// isolated session owns its whole worktree, so it gets `None`.
    pub fn touched(&self) -> Option<std::collections::HashSet<String>> {
        (!self.isolated).then(|| {
            self.messages
                .iter()
                .flat_map(|m| m.changed.iter().cloned())
                .collect()
        })
    }
}

/// Changes cut down to the files in `touched`, when there is such a set.
pub fn only_touched(
    changes: Vec<crate::git::Change>,
    touched: Option<&std::collections::HashSet<String>>,
) -> Vec<crate::git::Change> {
    match touched {
        Some(t) => changes
            .into_iter()
            .filter(|c| t.contains(&c.path))
            .collect(),
        None => changes,
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

    /// Pick a model ("" for the agent's default). Effort and speed depend on the model, so
    /// they reset with it.
    pub fn set_model(&mut self, model: String) {
        self.opts.remove("effort");
        self.opts.remove("tier");
        self.opts.insert("model".into(), model);
    }

    /// Switch to another agent CLI. Its options and conversation start fresh.
    pub fn switch_provider(&mut self, provider: &str) {
        if self.provider == provider {
            return;
        }
        self.switched_from = None;
        self.provider = provider.to_string();
        self.opts.clear();
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
        self.opts = from.opts;
        self.conversation_start = from.conversation_start.min(self.messages.len());
        self.handoff.clear();
        self.carried = from.carried;
        self.switched_from = None;
        true
    }
}

/// An optional part of a session, or none when it can't be read (a newer Runo saved it
/// in a shape this one doesn't know), so the rest of the session still loads.
fn or_none<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    d: D,
) -> Result<Option<T>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

/// Steps as saved now, or as the plain labels sessions were saved with before.
fn steps_or_labels<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ToolStep>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Label(String),
        Step(ToolStep),
    }
    Ok(Vec::<Saved>::deserialize(d)?
        .into_iter()
        .map(|s| match s {
            Saved::Label(label) => ToolStep::done(label),
            Saved::Step(step) => step,
        })
        .collect())
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
            changed: Vec::new(),
            resolved: String::new(),
            redo: String::new(),
            images: Vec::new(),
            usage: Usage::default(),
            denied: Vec::new(),
            plan: String::new(),
            questions: Vec::new(),
            playbook: String::new(),
            note: String::new(),
            crew: None,
            from_thread: None,
        }
    }

    /// The thread that sent this message to Crew, if a thread did. Before `from_thread`
    /// such messages were saved as a playbook named "From <title>" with the wake's text;
    /// those count too, unless `is_playbook` knows a playbook by that name.
    pub fn thread_origin(&self, is_playbook: impl Fn(&str) -> bool) -> Option<u64> {
        if self.role != Role::User {
            return None;
        }
        if self.from_thread.is_some() {
            return self.from_thread;
        }
        if !self.playbook.starts_with("From ") || is_playbook(&self.playbook) {
            return None;
        }
        crate::crew::parse_wake(&self.text).map(|(thread, _)| thread)
    }

    /// Where step `i` stands, for its icon: still running (only while the turn is live),
    /// failed, or done.
    pub fn step_state(&self, i: usize, live: bool) -> StepState {
        match self.steps.get(i).map(|s| s.state) {
            Some(StepState::Running) if !live => StepState::Done,
            Some(state) => state,
            None => StepState::Done,
        }
    }

    /// A running step's result arrived: it is no longer running, and is marked failed
    /// when its tool reported an error. Returns false when no running step has that id.
    pub fn finish_step(&mut self, id: &str, ok: bool) -> bool {
        let Some(step) = self
            .steps
            .iter_mut()
            .rev()
            .find(|s| s.state == StepState::Running && s.id == id)
        else {
            return false;
        };
        step.finish(ok);
        true
    }

    /// The turn is over: steps whose result never came are taken as done.
    pub fn settle_steps(&mut self) {
        for step in &mut self.steps {
            if step.state == StepState::Running {
                step.finish(true);
            }
        }
    }

    /// The steps' labels, oldest first.
    pub fn step_labels(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().map(|s| s.label.as_str())
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
    /// How pull requests are merged on GitHub: "merge", "squash" or "rebase"; empty
    /// picks one the repository allows.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub merge_method: String,
    /// Commands run after each turn that changed files, one per line (tests, lint,
    /// typecheck, build). Empty runs none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checks: String,
    /// Which checks run: `proof::LEVEL_*`, where empty follows Settings. `None` only for
    /// a project saved before levels, until `State::load` migrates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_level: Option<String>,
    /// How many times in a row failing checks go back to the agent on their own; 0 asks.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub fix_rounds: u32,
    /// What the person last let the checks run (`proof::fingerprint`), for sessions whose
    /// mode asks before shell commands: they run on their own while it stays the same.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checks_ok: String,
    /// How the person works here: `crew` through Crew's chat, empty directly in
    /// sessions. See `crew::is_crew`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    /// Let agent CLIs use settings committed in the repository that start programs
    /// (Cursor and Gemini MCP servers, hooks, tool commands): Runo trusts the folder
    /// for them and approves the servers up front. Off, Runo trusts a folder only when
    /// it holds no such settings.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trust_agent_config: bool,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
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
    /// The canvas the session was asked from, `design` or `architecture`: it opens there
    /// and stays out of the session lists, with its chat a click away on that canvas.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub canvas: String,
    /// The canvas context last sent with an ask, so the next ask in the same conversation
    /// sends only what changed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub canvas_context: String,
    /// The newest snapshot of the folder Runo took (end of a turn, an undo or a redo),
    /// so Undo can leave alone what changed after it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snapshot: String,
    /// The branch is a pull request from someone else's fork: its scripts are theirs, so
    /// Runo never runs its checks on its own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fork: bool,
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
    /// Proof of done for the last turn: its checks and the second opinion on it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "or_none"
    )]
    pub proof: Option<crate::proof::Proof>,
    /// What Crew keeps for this session when it is one of Crew's threads.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "or_none"
    )]
    pub crew: Option<crate::crew::ThreadInfo>,
}

/// A prompt waiting for the agent, with the images attached to it.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Queued {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// A thread's message to Crew waiting for Crew's turn: the thread that sent it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_thread: Option<u64>,
}

impl Queued {
    pub fn new(text: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            text: text.into(),
            images,
            from_thread: None,
        }
    }
}

/// What a switch replaced, enough to undo it before anything is sent.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct SwitchedFrom {
    pub provider: String,
    pub agent_session: String,
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

#[derive(Clone, Default, Serialize, Deserialize)]
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
    /// The same cost split by agent ("claude", "opencode"), for the Usage popover; the
    /// last 31 days. Days from before it was kept have only the total in `spend`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub agent_spend: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    /// Turns started and tokens reported per day ("%Y-%m-%d"), for the turn and token
    /// limits; the last 31 days, like `spend`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub day_use: std::collections::BTreeMap<String, DayUse>,
}

/// What agents did on one day, across every session.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct DayUse {
    /// Turns started.
    #[serde(default)]
    pub turns: u32,
    /// Input and output tokens the agents reported, cached ones not counted.
    #[serde(default)]
    pub tokens: u64,
}

/// Drop the oldest days past a month.
fn keep_month<V>(days: &mut std::collections::BTreeMap<String, V>) {
    while days.len() > 31 {
        days.pop_first();
    }
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
    /// A cron expression it runs on instead, in local time; a time of day on chosen
    /// weekdays is stored this way too. Empty uses `every_minutes`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cron: String,
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
        self.enabled && self.next_run().is_some_and(|at| now >= at)
    }

    /// When the schedule next runs (Unix seconds), counting from the last run.
    pub fn next_run(&self) -> Option<u64> {
        if !self.scheduled() {
            None
        } else if self.cron.is_empty() {
            Some(self.last_run + self.every_minutes * 60)
        } else {
            crate::schedule::next_local(&self.cron, self.last_run)
        }
    }

    /// How many scheduled runs fell between the last run and `now`, up to 1000.
    pub fn runs_due(&self, now: u64) -> u64 {
        if !self.scheduled() || now <= self.last_run {
            0
        } else if self.cron.is_empty() {
            (now - self.last_run) / (self.every_minutes * 60).max(1)
        } else {
            crate::schedule::runs_between(&self.cron, self.last_run, now, 1000)
        }
    }

    /// "Hourly", "Weekdays at 09:00", "Cron 0 */2 * * *".
    pub fn schedule_label(&self) -> String {
        if self.cron.is_empty() {
            crate::playbooks::every_label(self.every_minutes)
        } else {
            crate::schedule::describe(&self.cron)
        }
    }

    /// Runs on a timer as well as by hand. A schedule needs a project to run in.
    pub fn scheduled(&self) -> bool {
        (self.every_minutes > 0 || !self.cron.is_empty()) && self.project_id != 0
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
    /// Right panel width in pixels; 0 is the default.
    #[serde(default)]
    pub panel_width: f32,
    /// Terminal panel height in pixels; 0 is the default.
    #[serde(default)]
    pub term_height: f32,
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
    /// Add a turn's cost to `day`, in total and for `provider`, keeping a month of days.
    pub fn add_spend(&mut self, day: &str, provider: &str, cents: f64) {
        if cents <= 0.0 {
            return;
        }
        *self.spend.entry(day.to_string()).or_default() += cents;
        keep_month(&mut self.spend);
        *self
            .agent_spend
            .entry(day.to_string())
            .or_default()
            .entry(provider.to_string())
            .or_default() += cents;
        keep_month(&mut self.agent_spend);
    }

    /// What `provider` reported spending on `day`, in cents.
    pub fn agent_spend_on(&self, day: &str, provider: &str) -> f64 {
        self.agent_spend
            .get(day)
            .and_then(|by| by.get(provider))
            .copied()
            .unwrap_or_default()
    }

    /// Count a turn started on `day`, keeping a month of days.
    pub fn add_turn(&mut self, day: &str) {
        self.day_use.entry(day.to_string()).or_default().turns += 1;
        keep_month(&mut self.day_use);
    }

    /// Add a turn's reported tokens to `day`, keeping a month of days.
    pub fn add_tokens(&mut self, day: &str, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let used = self.day_use.entry(day.to_string()).or_default();
        used.tokens = used.tokens.saturating_add(tokens);
        keep_month(&mut self.day_use);
    }

    /// Turns and tokens on `day`; a day with nothing yet, such as a new one, is zero.
    pub fn used_on(&self, day: &str) -> DayUse {
        self.day_use.get(day).copied().unwrap_or_default()
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
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{stamp}-{}", std::process::id());
    write_private(
        &inbox.join(format!("{name}.json")),
        &serde_json::to_vec(request)?,
    )
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
/// parsed is moved aside first, so the next save cannot silently overwrite the user's data,
/// and whatever of it can still be read is kept (see `salvage`).
pub fn read_json_or_backup<T: Default + Serialize + serde::de::DeserializeOwned>(path: &Path) -> T {
    let Ok(text) = std::fs::read_to_string(path) else {
        return T::default();
    };
    match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            move_aside(path, &error.to_string());
            serde_json::from_str(&text)
                .ok()
                .and_then(salvage)
                .unwrap_or_default()
        }
    }
}

/// The parts of a JSON object that can still be read as a `T`, the rest left at their
/// defaults: one field saved in a shape this Runo doesn't know (by a newer one, or a
/// hand edit) then costs that field, not the whole file. `None` when `value` isn't an
/// object.
pub fn salvage<T: Default + Serialize + serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> Option<T> {
    let serde_json::Value::Object(fields) = value else {
        return None;
    };
    let serde_json::Value::Object(mut kept) = serde_json::to_value(T::default()).ok()? else {
        return None;
    };
    for (key, field) in fields {
        let before = kept.insert(key.clone(), field);
        if serde_json::from_value::<T>(serde_json::Value::Object(kept.clone())).is_err() {
            match before {
                Some(old) => kept.insert(key, old),
                None => kept.remove(&key),
            };
        }
    }
    serde_json::from_value(serde_json::Value::Object(kept)).ok()
}

/// Move a file Runo can't read out of the way, keeping it next to where it was.
pub fn move_aside(path: &Path, error: &str) {
    set_aside(path, error, true);
}

/// Copy a file Runo could read only in part next to where it was, before it is saved over.
pub fn keep_aside(path: &Path, error: &str) {
    set_aside(path, error, false);
}

fn set_aside(path: &Path, error: &str, moved: bool) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_extension(format!("json.corrupt-{stamp}"));
    let done = if moved {
        std::fs::rename(path, &backup).is_ok()
    } else {
        std::fs::copy(path, &backup).is_ok()
    };
    crate::applog::error(
        "load",
        format!(
            "could not read all of {} ({error}); {} it to {}",
            path.display(),
            match (moved, done) {
                (true, true) => "moved",
                (false, true) => "copied",
                _ => "could not keep",
            },
            backup.display()
        ),
    );
}

/// Write JSON atomically and privately (see `write_private`).
pub fn write_json<T: Serialize>(path: &Path, value: &T) {
    let written = serde_json::to_string_pretty(value)
        .map_err(|e| e.to_string())
        .and_then(|text| write_private(path, text.as_bytes()).map_err(|e| e.to_string()));
    if let Err(e) = written {
        crate::applog::error_once("save", format!("could not save {}: {e}", path.display()));
    }
}

/// Write `data` to `path` so only the person can read it, without anyone seeing it half
/// written: into a new temporary file created 0600 (`create_new`, so a link planted there
/// fails rather than being followed), then renamed over the target, which replaces a link
/// there instead of writing through it. A missing folder is made private (0700).
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_private_dir(parent)?;
    }
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| file.write_all(data));
    match written.and_then(|_| std::fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Make `dir` and any missing parents; the ones made here are private (0700).
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Make Runo's data folder private: 0700, and its files and request folders readable
/// only by the person. Installs from before this kept it 0755 with 0644 files, which
/// left settings (API keys, env lines), transcripts and approvals open to every
/// account on the computer. On Windows the folder sits in the person's profile, which
/// is theirs alone already.
pub fn tighten_data_dir() {
    let dir = data_dir();
    let _ = create_private_dir(&dir);
    #[cfg(unix)]
    tighten(&dir, 1);
}

/// Folders 0700 and files 0600, `depth` levels down. Links are left alone: they may
/// point anywhere.
#[cfg(unix)]
fn tighten(dir: &Path, depth: u32) {
    use std::os::unix::fs::PermissionsExt;
    let set = |path: &Path, mode: u32| {
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if !meta.file_type().is_symlink() && meta.permissions().mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
            }
        }
    };
    set(dir, 0o700);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() {
            set(&entry.path(), 0o600);
        } else if kind.is_dir() && depth > 0 {
            tighten(&entry.path(), depth - 1);
        }
    }
}

/// When this Runo is not the one that last ran on `root` (an update, or going back to an
/// older build), keep a copy of the files it is about to rewrite: `state.json.prev` and
/// `settings.json.prev`. When the last one was newer, every session file is copied too, to
/// `backups/<version>-<time>/`, since an older build drops what it doesn't know as it saves.
/// The version that ran is kept in `version`.
pub fn keep_before_version_change(root: &Path, current: &str) {
    let marker = root.join("version");
    let last = std::fs::read_to_string(&marker).unwrap_or_default();
    let last = last.trim();
    if last == current {
        return;
    }
    for name in ["state.json", "settings.json"] {
        let from = root.join(name);
        if from.is_file() {
            let _ = std::fs::copy(&from, root.join(format!("{name}.prev")));
        }
    }
    if crate::alerts::is_newer(last, current) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = root.join("backups").join(format!("{last}-{stamp}"));
        let copied = copy_saved_files(root, &backup);
        crate::applog::warn(
            "load",
            format!(
                "Runo {last} last used this data and this is {current}; copied {copied} files \
                 to {} before saving over them",
                backup.display()
            ),
        );
    }
    if std::fs::create_dir_all(root).is_ok() {
        let _ = std::fs::write(&marker, current);
    }
}

/// Copy `state.json`, `settings.json` and every `sessions/<id>/session.json` under `root`
/// to the same places under `to`. Returns how many files were copied.
fn copy_saved_files(root: &Path, to: &Path) -> usize {
    let mut files = vec![PathBuf::from("state.json"), PathBuf::from("settings.json")];
    if let Ok(entries) = std::fs::read_dir(root.join("sessions")) {
        files.extend(entries.flatten().map(|e| {
            Path::new("sessions")
                .join(e.file_name())
                .join("session.json")
        }));
    }
    files
        .into_iter()
        .filter(|file| root.join(file).is_file())
        .filter(|file| {
            let target = to.join(file);
            target
                .parent()
                .is_some_and(|dir| create_private_dir(dir).is_ok())
                && std::fs::copy(root.join(file), target).is_ok()
        })
        .count()
}

/// The person's home folder: `$HOME`, else `%USERPROFILE%` (Windows sets only that).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()))
        .map(PathBuf::from)
}

/// Runo's data folder: `$RUNO_HOME`, else `~/.runo`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("RUNO_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".runo")))
        .unwrap_or_else(|| PathBuf::from(".runo"))
}

/// Give a project saved before check levels its level: checks Runo suggested follow the
/// Settings level from now on, and commands the person typed stay as they are.
fn migrate_check_level(project: &mut Project) {
    if project.check_level.is_some() {
        return;
    }
    let suggested = if crate::proof::commands(&project.checks).is_empty() {
        Vec::new()
    } else {
        crate::proof::detect(Path::new(&project.path))
    };
    let (level, keep) = crate::proof::migrate(&project.checks, &suggested);
    project.check_level = Some(level.into());
    if !keep {
        project.checks.clear();
    }
}

fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

impl State {
    pub fn load() -> Self {
        Self::load_from(&state_path())
    }

    /// Load the state whose index is at `path`, with the session files next to it.
    pub fn load_from(path: &Path) -> Self {
        let root = path.parent().unwrap_or(Path::new("."));
        let mut state = match crate::persist::read(path) {
            Ok((state, mut written)) => {
                if let Some(error) = written.index_error.take() {
                    keep_aside(path, &error);
                }
                crate::persist::seed(root, written);
                state
            }
            Err(error) => {
                move_aside(path, &error);
                State::default()
            }
        };
        // A turn on a connection keeps going there, and a local turn runs detached from
        // Runo; both are picked up again on start. Anything else could not survive.
        for session in &mut state.sessions {
            if session.status == Status::Working
                && !session.remote_turn
                && crate::detach::left_behind(root, session.id).is_none()
            {
                session.status = Status::Failed;
                session.messages.push(Message::new(
                    Role::System,
                    "Interrupted: Runo closed while this run was active.",
                ));
            }
        }
        for project in &mut state.projects {
            migrate_check_level(project);
        }
        state
    }

    /// Read the saved state without changing anything, for other processes such as
    /// Runo's MCP server.
    pub fn peek() -> Self {
        crate::persist::read(&state_path())
            .map(|(state, ..)| state)
            .unwrap_or_default()
    }

    /// Save in the background: the files are written a moment later, and only those
    /// that changed.
    pub fn save(&self) {
        crate::persist::queue(&data_dir(), self.clone());
    }

    /// Write what is still waiting to be saved; for quitting.
    pub fn flush() {
        crate::persist::flush(std::time::Duration::from_secs(5));
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

#[cfg(test)]
impl State {
    /// Five projects and seven sessions for tests.
    pub fn sample() -> Self {
        // Each sample project gets its own color so the sidebar shows the project rails.
        let project = |id: u64, name: &str| Project {
            id,
            name: name.into(),
            path: format!("/sample/{name}"),
            color: ["blue", "violet", "teal", "orange", "pink"][(id as usize - 1) % 5].into(),
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
        // One unread and one pinned session.
        state.sessions[1].unread = true;
        state.sessions[2].pinned = true;
        state
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn thread_messages_saved_as_playbooks_are_recognised() {
        let mut old = Message::new(Role::User, "Thread #41 (Refine Arcade) says: Ready.");
        old.playbook = "From Refine Arcade".into();
        assert_eq!(old.thread_origin(|_| false), Some(41));
        // A playbook really named that is still a playbook run.
        assert_eq!(old.thread_origin(|n| n == "From Refine Arcade"), None);
        // Saved before the field existed, it loads and is still recognised.
        let json = r#"{"role":"User","text":"Thread #41 (Refine Arcade) says: Ready.","playbook":"From Refine Arcade"}"#;
        let loaded: Message = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.from_thread, None);
        assert_eq!(loaded.thread_origin(|_| false), Some(41));
    }

    #[test]
    fn playbook_runs_are_not_thread_messages() {
        let mut run = Message::new(Role::User, "Write the release notes for…");
        run.playbook = "Release notes".into();
        assert_eq!(run.thread_origin(|_| true), None);
        assert_eq!(run.thread_origin(|_| false), None);
        // "From …" alone isn't enough: the text must be a thread's message.
        run.playbook = "From scratch".into();
        assert_eq!(run.thread_origin(|_| false), None);
        assert!(!serde_json::to_string(&run).unwrap().contains("from_thread"));
        let typed = Message::new(Role::User, "Thread #3 (x) says: hi");
        assert_eq!(typed.thread_origin(|_| false), None);
    }

    #[test]
    fn a_shared_folder_counts_only_what_the_agent_changed() {
        let change = |path: &str| crate::git::Change {
            path: path.into(),
            status: "A".into(),
            added: 1,
            removed: 0,
            staged: false,
            unstaged: true,
            binary: false,
        };
        let mut reply = Message::new(Role::Agent, "Done.");
        reply.changed = vec!["src/lib.rs".into()];
        let mut session = Session {
            messages: vec![
                Message::new(Role::User, "Fix it"),
                reply,
                Message::new(Role::User, "Did you change any files?"),
                Message::new(Role::Agent, "No."),
            ],
            ..Default::default()
        };
        let folder = vec![change("src/lib.rs"), change("architecture/model.toml")];
        let touched = session.touched();
        let mine = only_touched(folder.clone(), touched.as_ref());
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].path, "src/lib.rs");
        // An isolated session owns its worktree, so everything in it counts.
        session.isolated = true;
        assert_eq!(session.touched(), None);
        assert_eq!(only_touched(folder, None).len(), 2);
    }

    #[test]
    fn schedules_say_when_they_run_next() {
        let mut a = Automation {
            project_id: 1,
            every_minutes: 60,
            last_run: 1_000_000,
            enabled: true,
            ..Default::default()
        };
        assert_eq!(a.next_run(), Some(1_003_600));
        assert!(!a.due(1_003_599) && a.due(1_003_600));
        assert_eq!(a.runs_due(1_000_000 + 3 * 3600 + 5), 3);
        // A cron schedule wins over the minutes; this one runs every minute.
        a.cron = "* * * * *".into();
        let start = 1_000_020;
        a.last_run = start;
        assert_eq!(a.next_run(), Some(1_000_080));
        assert_eq!(a.runs_due(start + 600), 10);
        assert_eq!(a.schedule_label(), "Cron * * * * *");
        a.cron = "30 9 * * 1,2,3,4,5".into();
        assert_eq!(a.schedule_label(), "Weekdays at 09:30");
        a.project_id = 0;
        assert!(!a.scheduled() && a.next_run().is_none());
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn an_old_open_data_folder_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let open = |p: &Path, m| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m));
        let dir = temp("tighten");
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.join("approvals")).unwrap();
        std::fs::write(dir.join("approvals").join("1-1.json"), "{}").unwrap();
        open(&dir, 0o755).unwrap();
        open(&dir.join("settings.json"), 0o644).unwrap();
        open(&dir.join("approvals"), 0o755).unwrap();
        tighten(&dir, 1);
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("settings.json")), 0o600);
        assert_eq!(mode(&dir.join("approvals")), 0o700);
        assert_eq!(mode(&dir.join("approvals").join("1-1.json")), 0o600);
        // New saves are private from the start.
        write_json(&dir.join("state.json"), &serde_json::json!({ "a": 1 }));
        assert_eq!(mode(&dir.join("state.json")), 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn picking_a_model_resets_its_effort_and_speed() {
        let mut session = State::sample().sessions[0].clone();
        session.opts.insert("effort".into(), "high".into());
        session.opts.insert("tier".into(), "priority".into());
        session.opts.insert("mode".into(), "read-only".into());
        session.set_model("gpt-5.5".into());
        let sel = session.selections();
        assert_eq!(sel.get("model").map(String::as_str), Some("gpt-5.5"));
        assert!(!sel.contains_key("effort") && !sel.contains_key("tier"));
        assert_eq!(sel.get("mode").map(String::as_str), Some("read-only"));
        // The agent's default is an explicit empty pick.
        session.set_model(String::new());
        assert_eq!(
            session.selections().get("model").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn saves_and_loads_round_trip() {
        let dir = temp("roundtrip");
        let path = dir.join("state.json");
        let mut state = State::sample();
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
        write_json(&path, &State::sample());
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
        let mut state = State::sample();
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
    fn switching_agents_starts_a_new_conversation() {
        let mut state = State::sample();
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
    fn a_stopped_session_reads_stopped_not_failed() {
        assert_eq!(Status::Stopped.label(), "Stopped");
        assert_eq!(Status::Failed.label(), "Failed");
        let saved: Status =
            serde_json::from_str(&serde_json::to_string(&Status::Stopped).unwrap()).unwrap();
        assert!(saved == Status::Stopped);
    }

    #[test]
    fn steps_run_until_finished_and_keep_failures() {
        let mut m = Message::new(Role::Agent, "");
        m.steps = vec![
            ToolStep::running("a", "Bash", "Ran sleep 60"),
            ToolStep::running("b", "Bash", "Ran false"),
        ];
        assert_eq!(m.step_state(0, true), StepState::Running);
        assert_eq!(
            m.step_state(0, false),
            StepState::Done,
            "nothing runs once the turn is over"
        );
        assert!(m.finish_step("b", false));
        assert!(!m.finish_step("b", false), "already finished");
        assert_eq!(m.step_state(1, true), StepState::Failed);
        assert!(m.finish_step("a", true));
        assert_eq!(m.step_state(0, true), StepState::Done);
        let saved: Message = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(saved.step_state(1, false), StepState::Failed);
        assert_eq!(saved.steps[0].kind, crate::harness::ToolKind::Shell);
    }

    #[test]
    fn unfinished_steps_settle_as_done_when_the_turn_ends() {
        let mut m = Message::new(Role::Agent, "");
        m.steps = vec![ToolStep::running("a", "Bash", "Ran sleep 60")];
        m.settle_steps();
        assert_eq!(m.steps[0].state, StepState::Done);
        assert!(!m.finish_step("a", false), "nothing is left running");
    }

    #[test]
    fn steps_saved_as_plain_labels_still_load() {
        let saved = r#"{"role":"Agent","text":"ok","steps":["Read a.rs",
            {"label":"Ran false","kind":"shell","state":"failed"}],"failed_steps":[1]}"#;
        let m: Message = serde_json::from_str(saved).unwrap();
        assert_eq!(
            m.step_labels().collect::<Vec<_>>(),
            ["Read a.rs", "Ran false"]
        );
        assert_eq!(m.step_state(0, false), StepState::Done);
        assert_eq!(m.step_state(1, false), StepState::Failed);
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

    /// Sample state written to `dir` the way Runo writes it, and the first session's file.
    fn written_sample(dir: &Path) -> (State, PathBuf) {
        let mut state = State::sample();
        crate::persist::write_all(dir, &mut state, &mut crate::persist::Written::default());
        let file = crate::persist::session_dir(dir, state.sessions[0].id).join("session.json");
        (state, file)
    }

    fn edit_json(path: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        edit(&mut value);
        std::fs::write(path, value.to_string()).unwrap();
    }

    fn aside_copies(dir: &Path, prefix: &str) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .count()
    }

    #[test]
    fn a_session_from_a_newer_runo_still_loads() {
        let dir = temp("newer-session");
        let (state, file) = written_sample(&dir);
        edit_json(&file, |v| {
            v["status"] = "Paused".into();
            v["something_new"] = serde_json::json!({ "a": 1 });
            v["proof"] = serde_json::json!({ "shape": "from the future" });
            let message = &mut v["messages"][1];
            message["role"] = "Tool".into();
            message["crew"] = serde_json::json!({ "Hologram": { "id": 1 } });
            message["steps"] = serde_json::json!([
                { "label": "Beamed up", "kind": "teleport", "state": "queued" }
            ]);
        });
        let loaded = State::load_from(&dir.join("state.json"));
        let session = loaded.session(state.sessions[0].id).expect("still listed");
        assert!(session.status == Status::Idle);
        assert!(session.proof.is_none());
        let message = &session.messages[1];
        assert_eq!(message.role, Role::System);
        assert!(message.crew.is_none());
        assert_eq!(message.steps[0].label, "Beamed up");
        assert_eq!(message.steps[0].kind, crate::harness::ToolKind::Other);
        assert_eq!(message.steps[0].state, StepState::Done);
        assert!(file.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_session_with_a_field_it_cannot_read_keeps_the_rest() {
        let dir = temp("salvage-session");
        let (state, file) = written_sample(&dir);
        let id = state.sessions[0].id;
        edit_json(&file, |v| v["pinned"] = "very".into());
        let loaded = State::load_from(&dir.join("state.json"));
        let session = loaded.session(id).expect("kept in the sidebar");
        assert_eq!(session.title, state.sessions[0].title);
        assert_eq!(session.messages.len(), state.sessions[0].messages.len());
        assert!(!session.pinned);
        assert_eq!(
            aside_copies(file.parent().unwrap(), "session.json.corrupt-"),
            1
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_index_read_in_part_keeps_its_projects_and_a_copy() {
        let dir = temp("salvage-index");
        let (state, _) = written_sample(&dir);
        let path = dir.join("state.json");
        edit_json(&path, |v| v["notes"] = "not a map".into());
        let loaded = State::load_from(&path);
        assert_eq!(loaded.projects.len(), state.projects.len());
        assert_eq!(loaded.sessions.len(), state.sessions.len());
        assert!(path.exists());
        assert_eq!(aside_copies(&dir, "state.json.corrupt-"), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn salvage_keeps_what_reads() {
        #[derive(Default, Serialize, Deserialize, PartialEq, Debug)]
        struct Saved {
            name: String,
            count: u32,
            #[serde(default)]
            on: bool,
        }
        let value = serde_json::json!({ "name": "kept", "count": "many", "on": true, "new": 1 });
        let saved: Saved = salvage(value).unwrap();
        assert_eq!(
            saved,
            Saved {
                name: "kept".into(),
                count: 0,
                on: true
            }
        );
        assert!(salvage::<Saved>(serde_json::json!([1])).is_none());
    }

    #[test]
    fn data_from_a_newer_runo_is_copied_before_it_is_saved_over() {
        let dir = temp("version");
        let (state, file) = written_sample(&dir);
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        std::fs::write(dir.join("version"), "9.9.9").unwrap();
        keep_before_version_change(&dir, "0.5.9");
        assert!(dir.join("state.json.prev").is_file());
        assert!(dir.join("settings.json.prev").is_file());
        let backups: Vec<PathBuf> = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("9.9.9-"));
        let id = state.sessions[0].id.to_string();
        let copy = backups[0].join("sessions").join(id).join("session.json");
        assert_eq!(
            std::fs::read_to_string(copy).unwrap(),
            std::fs::read_to_string(&file).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("version")).unwrap(),
            "0.5.9"
        );
        // The same version again changes nothing; an update keeps only the .prev copies.
        std::fs::remove_file(dir.join("state.json.prev")).unwrap();
        keep_before_version_change(&dir, "0.5.9");
        assert!(!dir.join("state.json.prev").exists());
        keep_before_version_change(&dir, "0.6.0");
        assert!(dir.join("state.json.prev").is_file());
        assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
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
        state.add_spend("2026-10-13", "claude", 12.5);
        state.add_spend("2026-10-13", "opencode", 7.5);
        state.add_spend("2026-10-13", "claude", 0.0);
        assert_eq!(state.spend.get("2026-10-13"), Some(&20.0));
        assert_eq!(state.agent_spend_on("2026-10-13", "claude"), 12.5);
        assert_eq!(state.agent_spend_on("2026-10-13", "opencode"), 7.5);
        assert_eq!(state.agent_spend_on("2026-10-13", "codex"), 0.0);
        for day in 1..=40 {
            state.add_spend(&format!("2026-11-{day:02}"), "claude", 1.0);
        }
        assert_eq!(state.spend.len(), 31);
        assert_eq!(state.agent_spend.len(), 31);
        assert!(!state.spend.contains_key("2026-10-13"));
    }

    #[test]
    fn turns_and_tokens_add_up_per_day_and_keep_a_month() {
        let mut state = State::default();
        state.add_turn("2026-10-13");
        state.add_turn("2026-10-13");
        state.add_tokens("2026-10-13", 1_500);
        state.add_tokens("2026-10-13", 0);
        state.add_tokens("2026-10-13", 500);
        assert_eq!(
            state.used_on("2026-10-13"),
            DayUse {
                turns: 2,
                tokens: 2_000
            }
        );
        // A new day starts from nothing, and yesterday's count stays as it was.
        assert_eq!(state.used_on("2026-10-14"), DayUse::default());
        state.add_turn("2026-10-14");
        assert_eq!(state.used_on("2026-10-14").turns, 1);
        assert_eq!(state.used_on("2026-10-13").turns, 2);
        // Tokens alone start a day too, with no turns.
        state.add_tokens("2026-10-15", 7);
        assert_eq!(
            state.used_on("2026-10-15"),
            DayUse {
                turns: 0,
                tokens: 7
            }
        );
        for day in 1..=40 {
            state.add_turn(&format!("2026-11-{day:02}"));
        }
        assert_eq!(state.day_use.len(), 31);
        assert!(!state.day_use.contains_key("2026-10-13"));
        assert_eq!(state.used_on("2026-10-13"), DayUse::default());
    }

    #[test]
    fn turns_and_tokens_per_day_survive_a_restart() {
        let mut state = State::default();
        state.add_turn("2026-10-13");
        state.add_tokens("2026-10-13", 42);
        let text = serde_json::to_string(&state).unwrap();
        let back: State = serde_json::from_str(&text).unwrap();
        assert_eq!(
            back.used_on("2026-10-13"),
            DayUse {
                turns: 1,
                tokens: 42
            }
        );
    }

    /// Held by any test that changes process-wide environment variables,
    /// since cargo runs tests on parallel threads.
    pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Puts the named variables back as they were when dropped.
    struct RestoreEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            for (key, old) in &self.0 {
                match old {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn data_dir_uses_runo_home_else_falls_back_to_home_dot_runo() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = RestoreEnv(
            ["RUNO_HOME", "HOME"]
                .into_iter()
                .map(|k| (k, std::env::var_os(k)))
                .collect(),
        );
        let base = std::env::temp_dir().join(format!("runo-data-dir-{}", std::process::id()));

        let runo_home = base.join("runo-home");
        std::env::set_var("RUNO_HOME", &runo_home);
        assert_eq!(data_dir(), runo_home);

        let home = base.join("home");
        std::env::remove_var("RUNO_HOME");
        std::env::set_var("HOME", &home);
        assert_eq!(data_dir(), home.join(".runo"));
    }
}
