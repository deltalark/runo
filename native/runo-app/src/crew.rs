//! Crew: one chat per project that answers the person and runs threads for them. A thread
//! is an ordinary session with a `ThreadInfo` beside it: the brief Crew gave it, the
//! checklist and reports its agent keeps, and whether it needs the person.
//!
//! Threads talk back to the app through Runo's MCP server (`memory_mcp.rs`), which leaves
//! one small event file per call in `events_dir()`, written under a temporary name and
//! renamed so the app never reads half a file. Agents with no MCP write the same things as
//! fenced `runo-…` blocks in their reply, which `TagFilter` takes out of the visible text.
//!
//! Crew's own chat is a session too (`CANVAS`), held read-only; it acts by ending a reply
//! with a `runo-actions` block, which `parse_actions` and `within_limits` check before
//! anything runs. This file holds the data and the rules; the app side is `crew_ui.rs`.

use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::store::Project;

/// `Project::mode` of a project that works through Crew. Any other value, the empty
/// default included, is Direct.
pub const MODE_CREW: &str = "crew";

/// Steps a checklist keeps; more are cut off.
pub const MAX_STEPS: usize = 20;
/// Characters kept of one step, a decision option or a link.
pub const MAX_LINE: usize = 200;
/// Characters kept of a report, a question or a message to Crew.
pub const MAX_TEXT: usize = 4000;
/// Reports a thread keeps; the oldest go first.
pub const MAX_REPORTS: usize = 50;
/// Event ids remembered so an event read twice is applied once.
const SEEN_LIMIT: usize = 1000;

pub fn is_crew(project: &Project) -> bool {
    project.mode == MODE_CREW
}

/// What a session that is a Crew thread keeps besides the session itself.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct ThreadInfo {
    #[serde(default)]
    pub origin: Origin,
    /// What Crew asked the thread to do, shown at the top of the thread.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub brief: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checklist: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reports: Vec<Report>,
    /// The decision the thread is waiting on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Decision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    /// Hidden from the open threads; the session itself stays.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
}

/// Who started a thread.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    /// A session the person started that Crew now routes to.
    #[default]
    Person,
    /// Crew, answering the person's message with this id.
    Crew { message: u64 },
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    #[default]
    Todo,
    Doing,
    Done,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Step {
    pub text: String,
    #[serde(default)]
    pub mark: Mark,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Report {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// Seconds since the epoch.
    #[serde(default)]
    pub at: u64,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Choice {
    pub label: String,
    /// What happens if the person picks it.
    #[serde(default)]
    pub consequence: String,
}

/// One question for the person, with two to four answers and the one the thread would pick.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Decision {
    pub question: String,
    pub options: Vec<Choice>,
    #[serde(default)]
    pub recommended: usize,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    /// A decision, permission or question is open.
    NeedsYou,
    /// The work is done and waits for review.
    Review,
}

/// What a thread may spend: dollars where its agent reports cost, otherwise turns. Zero
/// means no limit of that kind.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Budget {
    #[serde(default)]
    pub dollars: f64,
    #[serde(default)]
    pub turns: u32,
}

/// One thing a thread told the app.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Event {
    /// Unique, so an event read twice is applied once.
    pub id: String,
    pub thread: u64,
    /// Seconds since the epoch.
    pub at: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// The whole checklist, replacing the last one, and a short name for the task when
    /// the thread gives one.
    Checklist {
        steps: Vec<Step>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Report {
        summary: String,
        #[serde(default)]
        links: Vec<String>,
    },
    Decision(Decision),
    /// A message for Crew, such as a question for another thread.
    Message {
        text: String,
    },
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Where threads leave events for the app.
pub fn events_dir() -> PathBuf {
    crate::store::data_dir().join("crew").join("events")
}

/// A new event from `thread`, stamped now.
pub fn event(thread: u64, kind: EventKind) -> Event {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Event {
        id: format!("{thread}-{nanos}-{}", std::process::id()),
        thread,
        at: now(),
        kind,
    }
}

/// Leave `event` in `dir`. Written under a temporary name and renamed, so the app never
/// reads half a file; names sort in the order events were sent.
pub fn send_event(dir: &Path, event: &Event) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{nanos:020}-{}", std::process::id());
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(event)?)?;
    std::fs::rename(tmp, dir.join(format!("{name}.json")))
}

/// Ids of events already taken, the newest `SEEN_LIMIT` of them.
#[derive(Default)]
pub struct Seen {
    order: VecDeque<String>,
    ids: HashSet<String>,
}

impl Seen {
    /// Whether `id` is new; it is remembered either way.
    pub fn first(&mut self, id: &str) -> bool {
        if !self.ids.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        if self.order.len() > SEEN_LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
        true
    }
}

/// Take every event waiting in `dir`, oldest first, removing their files. A file that
/// can't be read as an event is renamed to `.bad`, so it is skipped from then on but kept
/// for a look; an event already seen is dropped.
pub fn take_events(dir: &Path, seen: &mut Seen) -> Vec<Event> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let parsed = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Event>(&bytes).ok());
        match parsed {
            Some(event) => {
                let _ = std::fs::remove_file(&path);
                if seen.first(&event.id) {
                    out.push(event);
                }
            }
            None => {
                crate::applog::warn(
                    "crew",
                    format!("Skipped an unreadable event: {}", path.display()),
                );
                let _ = std::fs::rename(&path, path.with_extension("bad"));
            }
        }
    }
    out
}

/// `text` cut to `max` characters, trimmed.
fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].trim_end().to_string(),
        None => text.to_string(),
    }
}

/// A checklist as it is kept: steps clipped, empty ones left out, at most `MAX_STEPS`.
pub fn clean_steps(steps: Vec<Step>) -> Vec<Step> {
    steps
        .into_iter()
        .map(|s| Step {
            text: clip(&s.text, MAX_LINE),
            mark: s.mark,
        })
        .filter(|s| !s.text.is_empty())
        .take(MAX_STEPS)
        .collect()
}

/// A decision as it is kept, or why it can't be: it needs a question, two to four options
/// with labels, and a recommended option among them.
pub fn check_decision(decision: Decision) -> Result<Decision, String> {
    let question = clip(&decision.question, MAX_TEXT);
    if question.is_empty() {
        return Err("The decision needs a question.".into());
    }
    let options: Vec<Choice> = decision
        .options
        .into_iter()
        .map(|c| Choice {
            label: clip(&c.label, MAX_LINE),
            consequence: clip(&c.consequence, MAX_LINE),
        })
        .collect();
    if !(2..=4).contains(&options.len()) || options.iter().any(|c| c.label.is_empty()) {
        return Err("Give two to four options, each with a label.".into());
    }
    if decision.recommended >= options.len() {
        return Err("recommended must be the index of one of the options, from 0.".into());
    }
    Ok(Decision {
        question,
        options,
        recommended: decision.recommended,
    })
}

/// Apply what a thread reported to what the app keeps for it. Returns whether anything
/// changed; a message for Crew changes nothing here.
/// The name a thread gave its task, for a thread the person started: those are named
/// after their prompt until then. Crew's threads keep the titles Crew gave them.
pub fn self_title(info: &ThreadInfo, kind: &EventKind) -> Option<String> {
    let EventKind::Checklist {
        title: Some(title), ..
    } = kind
    else {
        return None;
    };
    let title = title.trim().trim_matches(['"', '.']).trim();
    (info.origin == Origin::Person && !title.is_empty()).then(|| title_from(title))
}

pub fn apply(info: &mut ThreadInfo, kind: &EventKind, at: u64) -> bool {
    match kind {
        EventKind::Checklist { steps, .. } => {
            let steps = clean_steps(steps.clone());
            if info.checklist == steps {
                return false;
            }
            info.checklist = steps;
            true
        }
        EventKind::Report { summary, links } => {
            let summary = clip(summary, MAX_TEXT);
            if summary.is_empty() {
                return false;
            }
            info.reports.push(Report {
                summary,
                links: links.iter().map(|l| clip(l, MAX_LINE)).collect(),
                at,
            });
            if info.reports.len() > MAX_REPORTS {
                info.reports.remove(0);
            }
            true
        }
        EventKind::Decision(decision) => match check_decision(decision.clone()) {
            Ok(decision) => {
                info.decision = Some(decision);
                info.attention = Some(Attention::NeedsYou);
                true
            }
            Err(_) => false,
        },
        EventKind::Message { .. } => false,
    }
}

/// A thread's turn ended cleanly with nothing asked of the person, so its agent is done:
/// steps it never ticked off are taken as done rather than left hanging. Returns whether
/// anything changed.
pub fn settle(info: &mut ThreadInfo) -> bool {
    if info.decision.is_some() {
        return false;
    }
    let mut changed = false;
    for step in &mut info.checklist {
        if step.mark != Mark::Done {
            step.mark = Mark::Done;
            changed = true;
        }
    }
    changed
}

/// Kinds of fenced block an agent with no MCP may write: ```runo-checklist and so on.
pub const BLOCK_KINDS: [&str; 4] = ["checklist", "report", "decision", "actions"];

/// A fenced `runo-…` block taken out of a reply.
#[derive(Clone, PartialEq, Debug)]
pub struct Tagged {
    pub kind: String,
    pub body: String,
}

/// Takes `runo-…` blocks out of a reply as it streams in, passing the rest through.
/// Text inside any other fenced code block is left alone, so an agent showing an example
/// block inside ```markdown doesn't trigger anything. A line that might open a block is
/// held back until it is complete; everything else passes through as soon as it arrives.
#[derive(Default)]
pub struct TagFilter {
    /// The current line, not yet ended by a newline.
    line: String,
    /// Bytes of `line` already passed through.
    shown: usize,
    /// The open block: its opening fence line and body so far.
    block: Option<(String, String, String)>,
    /// Inside a fenced code block that isn't Runo's.
    fenced: bool,
    blocks: Vec<Tagged>,
}

impl TagFilter {
    /// Feed the next piece of the reply; returns the text to show.
    pub fn push(&mut self, chunk: &str) -> String {
        let mut out = String::new();
        for part in chunk.split_inclusive('\n') {
            self.line.push_str(part);
            if self.line.ends_with('\n') {
                let line = std::mem::take(&mut self.line);
                let shown = std::mem::take(&mut self.shown);
                out.push_str(&self.end_line(line, shown));
            }
        }
        // Pass an unfinished line through when it can't be a fence.
        if self.block.is_none() {
            let start = self.line.trim_start();
            if !start.is_empty() && !start.starts_with('`') {
                out.push_str(&self.line[self.shown..]);
                self.shown = self.line.len();
            }
        }
        out
    }

    /// The text still held back, and the blocks found. A block never closed is shown as
    /// text, so nothing the agent wrote is lost.
    pub fn finish(mut self) -> (String, Vec<Tagged>) {
        let mut out = String::new();
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            let shown = self.shown;
            out.push_str(&self.end_line(line, shown));
        }
        if let Some((open, _, body)) = self.block.take() {
            out.push_str(&open);
            out.push_str(&body);
        }
        (out, self.blocks)
    }

    fn end_line(&mut self, line: String, shown: usize) -> String {
        let fence = line.trim();
        if let Some((open, kind, mut body)) = self.block.take() {
            if fence == "```" {
                self.blocks.push(Tagged {
                    kind,
                    body: body.trim_end().to_string(),
                });
            } else {
                body.push_str(&line);
                self.block = Some((open, kind, body));
            }
            return String::new();
        }
        if shown == 0 && fence.starts_with("```") {
            if !self.fenced {
                if let Some(kind) = fence.strip_prefix("```runo-") {
                    if BLOCK_KINDS.contains(&kind) {
                        let kind = kind.to_string();
                        self.block = Some((line, kind, String::new()));
                        return String::new();
                    }
                }
            }
            self.fenced = !self.fenced;
        }
        line[shown..].to_string()
    }
}

/// The event a `checklist`, `report` or `decision` block stands for. A checklist is one
/// step per line, `- [x]` done, `- [~]` doing and `- [ ]` to do; a report is its text, with
/// lines that are only a link kept as links; a decision is JSON like `ask_decision`'s.
pub fn block_event(block: &Tagged) -> Option<EventKind> {
    match block.kind.as_str() {
        "checklist" => {
            let title = block.body.lines().find_map(|l| {
                let l = l.trim();
                l.strip_prefix("title:")
                    .or_else(|| l.strip_prefix("Title:"))
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            });
            let steps = block
                .body
                .lines()
                .filter_map(|line| {
                    let line = line.trim().trim_start_matches(['-', '*']).trim_start();
                    let (mark, text) = [
                        ("[x]", Mark::Done),
                        ("[X]", Mark::Done),
                        ("[~]", Mark::Doing),
                        ("[ ]", Mark::Todo),
                    ]
                    .into_iter()
                    .find_map(|(box_, mark)| line.strip_prefix(box_).map(|t| (mark, t)))?;
                    Some(Step {
                        text: text.trim().to_string(),
                        mark,
                    })
                })
                .collect::<Vec<_>>();
            let steps = clean_steps(steps);
            (!steps.is_empty()).then_some(EventKind::Checklist { steps, title })
        }
        "report" => {
            let (links, text): (Vec<&str>, Vec<&str>) = block.body.lines().partition(|l| {
                let l = l.trim();
                (l.starts_with("https://") || l.starts_with("http://")) && !l.contains(' ')
            });
            let summary = text.join("\n").trim().to_string();
            (!summary.is_empty()).then(|| EventKind::Report {
                summary,
                links: links.iter().map(|l| l.trim().to_string()).collect(),
            })
        }
        "decision" => serde_json::from_str::<Decision>(&block.body)
            .ok()
            .and_then(|d| check_decision(d).ok())
            .map(EventKind::Decision),
        _ => None,
    }
}

// ---------- Crew's own chat ----------

/// `Session::canvas` of a project's Crew chat: a session like the Design and Architecture
/// chats, kept out of the session lists and held read-only.
pub const CANVAS: &str = "crew";
/// Threads Crew may start in one turn; more wait for the person.
pub const MAX_NEW_THREADS: usize = 3;
/// Actions read from one reply; the rest are dropped.
pub const MAX_ACTIONS: usize = 12;
/// Turns Crew takes on its own, to answer threads, between two messages from the person.
pub const MAX_WAKES: u32 = 4;

/// What Crew asks Runo to do, from the `runo-actions` block that ends its reply.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "do", rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)] // the names are the wire format Crew writes
pub enum Action {
    /// Start a thread: a new session that works on `brief`.
    StartThread {
        title: String,
        brief: String,
        /// An agent id such as "codex"; empty for the project's default.
        #[serde(default)]
        agent: String,
        /// Work in its own worktree; unset leaves it to the person's New session default.
        #[serde(default)]
        isolated: Option<bool>,
    },
    /// Send a thread a message, as its next prompt.
    MessageThread { thread: u64, text: String },
    /// Read a thread's latest replies, sent back with Crew's next turn.
    ReadThread { thread: u64 },
    /// Mark a thread finished: it leaves Crew's open threads, the session stays.
    ResolveThread { thread: u64 },
}

/// A card shown with a message in Crew's chat. Threads are drawn live from the sessions,
/// so a card shows where each one stands now, not when it was posted.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "card", rename_all = "snake_case")]
pub enum Card {
    /// Threads started together.
    Started { threads: Vec<u64> },
    /// Threads Crew asked for over its limit, waiting for the person.
    Held {
        actions: Vec<Action>,
        #[serde(default)]
        answered: bool,
    },
    /// News from one thread: it finished, failed, reported or needs the person.
    News { thread: u64 },
}

impl Action {
    /// The thread it is about, for actions on an existing one.
    pub fn thread(&self) -> Option<u64> {
        match self {
            Action::StartThread { .. } => None,
            Action::MessageThread { thread, .. }
            | Action::ReadThread { thread }
            | Action::ResolveThread { thread } => Some(*thread),
        }
    }
}

/// The actions in a `runo-actions` block, checked and clipped, and what was wrong with
/// the ones left out. The block may hold one action, a list, or `{"actions": [...]}`.
/// Nothing here can panic on what an agent writes, and an action that doesn't check out
/// is never returned.
pub fn parse_actions(body: &str) -> (Vec<Action>, Vec<String>) {
    let mut problems = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(body.trim()) {
        Ok(v) => v,
        Err(e) => {
            return (
                Vec::new(),
                vec![format!("The actions aren't valid JSON: {e}")],
            )
        }
    };
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut map) => match map.remove("actions") {
            Some(serde_json::Value::Array(items)) => items,
            Some(_) => return (Vec::new(), vec!["\"actions\" must be a list.".into()]),
            None => vec![serde_json::Value::Object(map)],
        },
        _ => {
            return (
                Vec::new(),
                vec!["The actions must be an object or a list.".into()],
            )
        }
    };
    if items.len() > MAX_ACTIONS {
        problems.push(format!(
            "Only the first {MAX_ACTIONS} of {} actions were read.",
            items.len()
        ));
    }
    let mut out: Vec<Action> = Vec::new();
    for item in items.into_iter().take(MAX_ACTIONS) {
        let action = match serde_json::from_value::<Action>(item) {
            Ok(a) => a,
            Err(e) => {
                problems.push(format!("Skipped an action: {e}"));
                continue;
            }
        };
        let action = match action {
            Action::StartThread {
                title,
                brief,
                agent,
                isolated,
            } => {
                let title = clip(&title, 80);
                let brief = clip(&brief, MAX_TEXT);
                if title.is_empty() || brief.is_empty() {
                    problems.push("Skipped a thread with no title or brief.".into());
                    continue;
                }
                Action::StartThread {
                    title,
                    brief,
                    agent: clip(&agent, 40).to_ascii_lowercase(),
                    isolated,
                }
            }
            Action::MessageThread { thread, text } => {
                let text = clip(&text, MAX_TEXT);
                if text.is_empty() {
                    problems.push(format!("Skipped an empty message to thread #{thread}."));
                    continue;
                }
                Action::MessageThread { thread, text }
            }
            other => other,
        };
        if action.thread() == Some(0) {
            problems.push("Skipped an action with no thread id.".into());
            continue;
        }
        if !out.contains(&action) {
            out.push(action);
        }
    }
    (out, problems)
}

/// Split checked actions into the ones that run now and the ones held for the person:
/// at most `MAX_NEW_THREADS` new threads, one message per thread, and only threads in
/// `threads` (this project's) may be named. Returns (run, held, problems).
pub fn within_limits(
    actions: Vec<Action>,
    threads: &[u64],
) -> (Vec<Action>, Vec<Action>, Vec<String>) {
    let mut run = Vec::new();
    let mut held = Vec::new();
    let mut problems = Vec::new();
    let mut started = 0;
    let mut messaged: Vec<u64> = Vec::new();
    for action in actions {
        if let Some(t) = action.thread() {
            if !threads.contains(&t) {
                problems.push(format!("There's no thread #{t} in this project."));
                continue;
            }
        }
        match &action {
            Action::StartThread { .. } => {
                if started < MAX_NEW_THREADS {
                    started += 1;
                    run.push(action);
                } else {
                    held.push(action);
                }
            }
            Action::MessageThread { thread, .. } => {
                if messaged.contains(thread) {
                    problems.push(format!("Only one message per thread per turn: #{thread}."));
                } else {
                    messaged.push(*thread);
                    run.push(action);
                }
            }
            _ => run.push(action),
        }
    }
    (run, held, problems)
}

/// The routing chip on Crew's composer.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Chip {
    /// Crew decides.
    #[default]
    Auto,
    /// Crew answers and starts nothing.
    Answer,
    /// Start a thread with the message, without asking Crew.
    Thread,
}

impl Chip {
    pub const ALL: [Chip; 3] = [Chip::Auto, Chip::Answer, Chip::Thread];

    pub fn label(self) -> &'static str {
        match self {
            Chip::Auto => "Auto",
            Chip::Answer => "Just answer",
            Chip::Thread => "New thread",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Chip::Auto => "Crew answers, or starts threads when there's work",
            Chip::Answer => "Crew only answers, and starts nothing",
            Chip::Thread => "Starts a thread with your message, without Crew",
        }
    }
}

/// Where a message typed in Crew's chat goes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Route {
    /// To Crew, with a note for this turn when the chip asked for one.
    Crew { answer_only: bool },
    /// Straight to a thread named with @, with what follows the name.
    Thread { id: u64, text: String },
    /// A new thread with the whole message as its brief.
    NewThread,
}

/// Where `text` goes. A message that starts with @ and a thread's title (or `@#12`) goes
/// to that thread and costs Crew nothing; then the chip decides.
pub fn route(text: &str, chip: Chip, threads: &[(u64, String)]) -> Route {
    if let Some(rest) = text.trim_start().strip_prefix('@') {
        let by_id = rest.strip_prefix('#').and_then(|r| {
            let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
            let id = digits.parse::<u64>().ok()?;
            threads
                .iter()
                .any(|(t, _)| *t == id)
                .then(|| (id, r[digits.len()..].to_string()))
        });
        let by_title = || {
            threads
                .iter()
                .filter(|(_, title)| {
                    !title.is_empty()
                        && rest
                            .get(..title.len())
                            .is_some_and(|head| head.to_lowercase() == title.to_lowercase())
                        && rest[title.len()..]
                            .chars()
                            .next()
                            .is_none_or(|c| !c.is_alphanumeric())
                })
                .max_by_key(|(_, title)| title.len())
                .map(|(id, title)| (*id, rest[title.len()..].to_string()))
        };
        if let Some((id, after)) = by_id.or_else(by_title) {
            let text = after.trim_start_matches([':', ',', ' ']).trim().to_string();
            if !text.is_empty() {
                return Route::Thread { id, text };
            }
        }
    }
    match chip {
        Chip::Auto => Route::Crew { answer_only: false },
        Chip::Answer => Route::Crew { answer_only: true },
        Chip::Thread => Route::NewThread,
    }
}

/// A short title for a thread started from a message: its first line, cut at a word.
pub fn title_from(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= 60 {
        return line.to_string();
    }
    let cut: String = line.chars().take(60).collect();
    match cut.rfind(' ') {
        Some(at) if at > 20 => format!("{}…", &cut[..at]),
        _ => format!("{cut}…"),
    }
}

/// A thread's title as shown. Titles used to be the prompt's first 48 characters, cut
/// mid-word; those are cut back to the last whole word.
pub fn shown_title(title: &str) -> String {
    let title = title.trim();
    if title.chars().count() != 48 {
        return title.to_string();
    }
    match title.rfind(' ') {
        Some(at) if at > 20 => format!("{}…", title[..at].trim_end_matches([',', ':', ';'])),
        _ => title.to_string(),
    }
}

/// Markdown as plain text on one line: links keep their text, and code marks, emphasis,
/// headings and list bullets go.
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(mid) = rest[open..].find("](").map(|m| open + m) else {
            break;
        };
        let Some(close) = rest[mid..].find(')').map(|c| mid + c) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..mid]);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out.lines()
        .map(|l| l.trim().trim_start_matches(['#', '>']).trim_start())
        .map(|l| {
            l.strip_prefix("- ")
                .or_else(|| l.strip_prefix("* "))
                .unwrap_or(l)
        })
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace("**", "")
        .replace("__", "")
        .replace('`', "")
}

/// What a finished thread came to, for its row: the first sentence of its last report,
/// without asides in brackets, cut at a word if it runs long.
pub fn outcome(info: &ThreadInfo) -> Option<String> {
    let text = plain(&info.reports.last()?.summary);
    let end = text
        .match_indices(['.', '!', '?'])
        .map(|(at, _)| at)
        .find(|&at| text[at + 1..].starts_with(' ') || at + 1 == text.len())
        .map_or(text.len(), |at| at + 1);
    let first = without_asides(&text[..end]);
    let first = first.trim().trim_end_matches('.');
    if first.is_empty() {
        return None;
    }
    if first.chars().count() <= OUTCOME_LEN {
        return Some(first.to_string());
    }
    let cut: String = first.chars().take(OUTCOME_LEN).collect();
    let cut = cut.rfind(' ').map_or(cut.as_str(), |at| &cut[..at]);
    Some(format!("{}…", cut.trim_end_matches([',', ':', ';'])))
}

/// The number of the pull request a link points at: ".../pull/12" → 12.
pub fn pr_number(link: &str) -> Option<u64> {
    let (_, rest) = link.split_once("/pull/")?;
    rest.split(['/', '#', '?']).next()?.parse().ok()
}

/// How long an outcome line runs before it is cut.
const OUTCOME_LEN: usize = 80;

/// Text without its bracketed asides: "Read cove.md (12 lines) and" → "Read cove.md and".
fn without_asides(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One line about a thread for Crew's board, which starts with `#id ` so changes can be
/// told apart: "#12 Cove setup [working] 3/5 steps. Last report: …".
pub fn board_line(id: u64, title: &str, status: &str, info: &ThreadInfo) -> String {
    let mut line = format!("#{id} {} [{status}]", clip(title, 80));
    if !info.checklist.is_empty() {
        let done = info
            .checklist
            .iter()
            .filter(|s| s.mark == Mark::Done)
            .count();
        line.push_str(&format!(" {done}/{} steps.", info.checklist.len()));
    }
    if let Some(d) = &info.decision {
        line.push_str(&format!(
            " Waiting on the person: {}",
            clip(&d.question, 160)
        ));
    }
    if let Some(r) = info.reports.last() {
        line.push_str(&format!(" Last report: {}", clip(&r.summary, 240)));
    }
    line.replace('\n', " ")
}

/// What changed on the board since `before` (the board last sent, one line per thread):
/// the lines that are new or different, and the threads no longer open. Empty when
/// nothing changed.
pub fn board_changes(before: &str, now: &[String]) -> String {
    let key = |l: &str| l.split_whitespace().next().unwrap_or("").to_string();
    let old: Vec<&str> = before.lines().filter(|l| l.starts_with('#')).collect();
    let mut out = String::new();
    for line in now {
        if !old.contains(&line.as_str()) {
            out.push_str(&format!("- {line}\n"));
        }
    }
    for line in &old {
        let k = key(line);
        if !now.iter().any(|n| key(n) == k) {
            out.push_str(&format!("- {k} is no longer open.\n"));
        }
    }
    out
}

/// Crew's rules, sent with the first message of its conversation.
pub fn rules(project: &str, agents: &[(&str, &str)], default_agent: &str) -> String {
    let roster = agents
        .iter()
        .map(|(id, name)| format!("{id} ({name})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r##"## You are Crew
You run the project "{project}" for the person in this chat. You can read the code here but never change it: threads do the work. A thread is an agent session of its own.

Answer directly when you can: questions, explanations, plans and small lookups need no thread. Start a thread when there is real work: code to change, something to build, fix or investigate at length. Keep replies short and plain.

To act, end your reply with one fenced block like this, and nothing after it:

```runo-actions
[{{"do":"start_thread","title":"App startup time","brief":"What to do, what done looks like, and anything the thread needs to know.","agent":"","isolated":true}}]
```

Actions:
- start_thread: title (2 to 5 words), brief (complete: the thread has not seen this chat), agent (empty for the default, {default_agent}; or one of: {roster}), isolated (true for its own worktree and branch, false to work in the project folder).
- message_thread: thread (its number), text. It becomes that thread's next prompt.
- read_thread: thread. Its latest replies come back with the next message.
- resolve_thread: thread. Use it once its work is merged or no longer needed.

Limits: at most {MAX_NEW_THREADS} new threads and one message per thread per reply. Never start a thread for something you can answer. Say in your reply what you started and why, in a sentence.

Each message may open with a Threads section: what changed in this project's threads since your last turn. Threads report through their own tools; use those reports to answer the person, with links.
"##
    )
}

/// Told to a thread with its first prompt: how it reports back.
pub fn thread_rules(tools: bool) -> String {
    let how = if tools {
        "Use Runo's tools: set_checklist with your plan and, the first time, a 2 to 5 word title for the task (update it as steps start and finish, and once more before you finish so every step shows how it ended; your own todo list shows there too), report when you finish or find something the person should know (open with a short sentence on the outcome, such as \"Fixed the export crash.\" or \"No changes needed.\", then a few sentences, with links), ask_decision when a choice changes what the person gets or can't be undone, and message_crew to ask Crew for something another thread has."
    } else {
        "Write these fenced blocks in your reply; Runo takes them out of the text. ```runo-checklist with a first line such as \"title: Settings spacing pass\" naming the task in 2 to 5 words, then one step per line (- [x] done, - [~] doing, - [ ] to do), written again at the end so every step shows how it ended, ```runo-report opening with a short sentence on the outcome, then a few sentences and any links on lines of their own, and ```runo-decision with JSON {\"question\":…, \"options\":[{\"label\":…, \"consequence\":…}], \"recommended\":0} when a choice changes what the person gets or can't be undone."
    };
    format!(
        "## You are a Crew thread\nCrew, which runs this project for the person, started you for the task below. Work on it until it's done. {how}\n\n## Your task\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_titles_are_cut_back_to_a_word() {
        assert_eq!(
            shown_title("In native/runo-app/src/settings_ui.rs, on the Se"),
            "In native/runo-app/src/settings_ui.rs, on the…"
        );
        assert_eq!(shown_title("Typos in git guide"), "Typos in git guide");
    }

    #[test]
    fn plain_drops_markdown() {
        assert_eq!(
            plain(
                "Created [cove-check.txt](/root/x/cove-check.txt) with `uname -a`.\n\n- **Done**"
            ),
            "Created cove-check.txt with uname -a. Done"
        );
    }

    #[test]
    fn outcome_is_the_reports_first_sentence() {
        let mut info = ThreadInfo::default();
        assert_eq!(outcome(&info), None);
        info.reports.push(Report {
            summary: "No typos found. I read docs/guide/git.md (14 lines).".into(),
            ..Default::default()
        });
        assert_eq!(outcome(&info).as_deref(), Some("No typos found"));
        info.reports[0].summary = "I read docs/guide/cove.md (12 lines: front matter plus three numbered steps) and found no typos. Nothing changed.".into();
        assert_eq!(
            outcome(&info).as_deref(),
            Some("I read docs/guide/cove.md and found no typos")
        );
    }

    #[test]
    fn pr_numbers_come_from_links() {
        assert_eq!(pr_number("https://github.com/a/b/pull/148"), Some(148));
        assert_eq!(pr_number("https://github.com/a/b/pull/7/files"), Some(7));
        assert_eq!(pr_number("docs/guide/git.md"), None);
    }

    #[test]
    fn person_threads_take_the_name_they_give() {
        let kind = EventKind::Checklist {
            steps: vec![],
            title: Some("Settings spacing pass".into()),
        };
        let mut info = ThreadInfo::default();
        assert_eq!(
            self_title(&info, &kind).as_deref(),
            Some("Settings spacing pass")
        );
        info.origin = Origin::Crew { message: 1 };
        assert_eq!(self_title(&info, &kind), None);
        let block = Tagged {
            kind: "checklist".into(),
            body: "title: Cove check\n- [ ] Run it".into(),
        };
        let Some(EventKind::Checklist { title, .. }) = block_event(&block) else {
            panic!("a checklist");
        };
        assert_eq!(title.as_deref(), Some("Cove check"));
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-crew-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn filter_all(chunks: &[&str]) -> (String, Vec<Tagged>) {
        let mut f = TagFilter::default();
        let mut shown: String = chunks.iter().map(|c| f.push(c)).collect();
        let (rest, blocks) = f.finish();
        shown.push_str(&rest);
        (shown, blocks)
    }

    #[test]
    fn projects_are_direct_unless_set_to_crew() {
        let project: Project = serde_json::from_str(r#"{"id":1,"name":"a","path":"/a"}"#).unwrap();
        assert!(!is_crew(&project));
        let crew = Project {
            mode: MODE_CREW.into(),
            ..project.clone()
        };
        assert!(is_crew(&crew));
        // Direct writes nothing, so files stay as older builds wrote them.
        assert!(!serde_json::to_string(&project).unwrap().contains("mode"));
    }

    #[test]
    fn events_round_trip_once_in_order() {
        let dir = temp("events");
        let first = event(7, EventKind::Message { text: "hi".into() });
        let second = event(
            7,
            EventKind::Report {
                summary: "Done".into(),
                links: vec![],
            },
        );
        send_event(&dir, &first).unwrap();
        send_event(&dir, &second).unwrap();
        // A file left half written by a crash isn't read.
        std::fs::write(dir.join("zz.tmp"), "{").unwrap();
        let mut seen = Seen::default();
        assert_eq!(take_events(&dir, &mut seen), vec![first.clone(), second]);
        assert!(take_events(&dir, &mut seen).is_empty());
        // The same event sent again, as after a replay, is applied once.
        send_event(&dir, &first).unwrap();
        assert!(take_events(&dir, &mut seen).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_events_are_set_aside() {
        let dir = temp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.json"), "not json").unwrap();
        let good = event(1, EventKind::Message { text: "ok".into() });
        send_event(&dir, &good).unwrap();
        let mut seen = Seen::default();
        assert_eq!(take_events(&dir, &mut seen), vec![good]);
        assert!(dir.join("1.bad").exists());
        assert!(take_events(&dir, &mut seen).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn seen_forgets_the_oldest() {
        let mut seen = Seen::default();
        for i in 0..=SEEN_LIMIT {
            assert!(seen.first(&i.to_string()));
        }
        assert!(!seen.first(&SEEN_LIMIT.to_string()));
        assert!(seen.first("0"));
    }

    #[test]
    fn a_finished_turn_ticks_off_what_is_left() {
        let step = |text: &str, mark| Step {
            text: text.into(),
            mark,
        };
        let mut info = ThreadInfo {
            checklist: vec![
                step("Read docs", Mark::Done),
                step("Start docker", Mark::Doing),
                step("Verify", Mark::Todo),
            ],
            ..Default::default()
        };
        assert!(settle(&mut info));
        assert!(info.checklist.iter().all(|s| s.mark == Mark::Done));
        assert!(!settle(&mut info));
        // A thread waiting on the person isn't done yet.
        info.checklist[1].mark = Mark::Doing;
        info.decision = Some(Decision::default());
        assert!(!settle(&mut info));
        assert_eq!(info.checklist[1].mark, Mark::Doing);
    }

    #[test]
    fn apply_keeps_what_threads_report() {
        let mut info = ThreadInfo::default();
        let steps = vec![
            Step {
                text: " Measure ".into(),
                mark: Mark::Done,
            },
            Step {
                text: "".into(),
                mark: Mark::Todo,
            },
        ];
        assert!(apply(
            &mut info,
            &EventKind::Checklist {
                steps: steps.clone(),
                title: None,
            },
            1
        ));
        assert_eq!(info.checklist.len(), 1);
        assert_eq!(info.checklist[0].text, "Measure");
        assert!(!apply(
            &mut info,
            &EventKind::Checklist { steps, title: None },
            2
        ));
        let long: Vec<Step> = (0..40)
            .map(|i| Step {
                text: format!("step {i}"),
                mark: Mark::Todo,
            })
            .collect();
        apply(
            &mut info,
            &EventKind::Checklist {
                steps: long,
                title: None,
            },
            3,
        );
        assert_eq!(info.checklist.len(), MAX_STEPS);

        for i in 0..MAX_REPORTS + 3 {
            let kind = EventKind::Report {
                summary: format!("r{i}"),
                links: vec![],
            };
            assert!(apply(&mut info, &kind, i as u64));
        }
        assert_eq!(info.reports.len(), MAX_REPORTS);
        assert_eq!(info.reports[0].summary, "r3");
        let empty = EventKind::Report {
            summary: "  ".into(),
            links: vec![],
        };
        assert!(!apply(&mut info, &empty, 9));

        let ask = |options: usize, recommended: usize| {
            EventKind::Decision(Decision {
                question: "Quit Runo?".into(),
                options: (0..options)
                    .map(|i| Choice {
                        label: format!("o{i}"),
                        consequence: String::new(),
                    })
                    .collect(),
                recommended,
            })
        };
        assert!(!apply(&mut info, &ask(1, 0), 1));
        assert!(!apply(&mut info, &ask(5, 0), 1));
        assert!(!apply(&mut info, &ask(2, 2), 1));
        assert!(info.attention.is_none());
        assert!(apply(&mut info, &ask(2, 1), 1));
        assert_eq!(info.attention, Some(Attention::NeedsYou));
        assert!(!apply(
            &mut info,
            &EventKind::Message { text: "x".into() },
            1
        ));
    }

    #[test]
    fn thread_info_saves_only_what_is_set() {
        assert_eq!(
            serde_json::to_string(&ThreadInfo::default()).unwrap(),
            r#"{"origin":{"kind":"person"}}"#
        );
        let info = ThreadInfo {
            origin: Origin::Crew { message: 4 },
            brief: "Time launches".into(),
            resolved: true,
            ..Default::default()
        };
        let back: ThreadInfo =
            serde_json::from_str(&serde_json::to_string(&info).unwrap()).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn tags_come_out_of_the_reply() {
        let reply = "Working on it.\n```runo-checklist\n- [x] Measure\n- [~] Fix\n- [ ] PR\n```\nDone for now.\n";
        let (shown, blocks) = filter_all(&[reply]);
        assert_eq!(shown, "Working on it.\nDone for now.\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "checklist");
        let Some(EventKind::Checklist { steps, .. }) = block_event(&blocks[0]) else {
            panic!("no checklist");
        };
        let marks: Vec<Mark> = steps.iter().map(|s| s.mark).collect();
        assert_eq!(marks, vec![Mark::Done, Mark::Doing, Mark::Todo]);
    }

    #[test]
    fn tags_split_across_chunks() {
        let reply = "Hi\n```runo-report\nAll done.\nhttps://github.com/a/b/pull/1\n```\nBye";
        // Every split point gives the same result.
        for at in 0..=reply.len() {
            if !reply.is_char_boundary(at) {
                continue;
            }
            let (shown, blocks) = filter_all(&[&reply[..at], &reply[at..]]);
            assert_eq!(shown, "Hi\nBye", "split at {at}");
            assert_eq!(blocks.len(), 1, "split at {at}");
        }
        let (_, blocks) = filter_all(&[reply]);
        assert_eq!(
            block_event(&blocks[0]),
            Some(EventKind::Report {
                summary: "All done.".into(),
                links: vec!["https://github.com/a/b/pull/1".into()],
            })
        );
    }

    #[test]
    fn text_streams_without_waiting_for_the_line() {
        let mut f = TagFilter::default();
        assert_eq!(f.push("Hel"), "Hel");
        assert_eq!(f.push("lo\n``"), "lo\n");
        assert_eq!(f.push("`rust\nfn a() {}\n"), "```rust\nfn a() {}\n");
    }

    #[test]
    fn examples_inside_other_code_blocks_stay() {
        let reply = "```markdown\n```runo-checklist\n- [ ] x\n```\n";
        let (shown, blocks) = filter_all(&[reply]);
        // The inner fence closes the markdown block, as Markdown reads it; nothing is taken.
        assert_eq!(shown, reply);
        assert!(blocks.is_empty());
        let other = "```runo-unknown\nx\n```\n";
        assert_eq!(filter_all(&[other]).0, other);
    }

    #[test]
    fn unclosed_blocks_are_shown() {
        let reply = "Text\n```runo-report\nhalf a report";
        let (shown, blocks) = filter_all(&[reply]);
        assert_eq!(shown, reply);
        assert!(blocks.is_empty());
    }

    #[test]
    fn decision_blocks_are_checked() {
        let good = Tagged {
            kind: "decision".into(),
            body:
                r#"{"question":"Quit?","options":[{"label":"Yes"},{"label":"No"}],"recommended":1}"#
                    .into(),
        };
        assert!(matches!(block_event(&good), Some(EventKind::Decision(_))));
        let bad = Tagged {
            kind: "decision".into(),
            body: r#"{"question":"Quit?","options":[{"label":"Yes"}]}"#.into(),
        };
        assert_eq!(block_event(&bad), None);
        let junk = Tagged {
            kind: "decision".into(),
            body: "{".into(),
        };
        assert_eq!(block_event(&junk), None);
    }

    fn start(title: &str) -> Action {
        Action::StartThread {
            title: title.into(),
            brief: format!("Do {title}"),
            agent: String::new(),
            isolated: None,
        }
    }

    #[test]
    fn actions_come_as_one_a_list_or_an_object() {
        let one = r#"{"do":"start_thread","title":"Startup time","brief":"Measure it","agent":"Codex","isolated":true}"#;
        let (found, problems) = parse_actions(one);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            found,
            [Action::StartThread {
                title: "Startup time".into(),
                brief: "Measure it".into(),
                agent: "codex".into(),
                isolated: Some(true),
            }]
        );
        let list = r#"[{"do":"message_thread","thread":4,"text":"Also the docs"},{"do":"read_thread","thread":4}]"#;
        assert_eq!(parse_actions(list).0.len(), 2);
        let wrapped = r#"{"actions":[{"do":"resolve_thread","thread":9}]}"#;
        assert_eq!(
            parse_actions(wrapped).0,
            [Action::ResolveThread { thread: 9 }]
        );
    }

    #[test]
    fn broken_actions_run_nothing_and_say_why() {
        for bad in [
            "",
            "not json",
            "42",
            r#"{"actions": 3}"#,
            r#"{"do":"delete_everything"}"#,
            r#"{"do":"start_thread","title":"","brief":"x"}"#,
            r#"{"do":"start_thread","title":"x","brief":"   "}"#,
            r#"{"do":"message_thread","thread":0,"text":"hi"}"#,
            r#"{"do":"message_thread","thread":3,"text":""}"#,
            r#"{"do":"read_thread","thread":-1}"#,
            r#"{"do":"read_thread","thread":"3"}"#,
        ] {
            let (found, problems) = parse_actions(bad);
            assert!(found.is_empty(), "{bad} gave {found:?}");
            assert!(!problems.is_empty(), "{bad} said nothing");
        }
        // Good ones beside a bad one still run; repeats run once.
        let mixed =
            r#"[{"do":"read_thread","thread":2},{"do":"nope"},{"do":"read_thread","thread":2}]"#;
        let (found, problems) = parse_actions(mixed);
        assert_eq!(found, [Action::ReadThread { thread: 2 }]);
        assert_eq!(problems.len(), 1);
        // Long text is clipped, and no more than MAX_ACTIONS are read.
        let long = format!(
            r#"{{"do":"start_thread","title":"{}","brief":"b"}}"#,
            "t".repeat(500)
        );
        match &parse_actions(&long).0[0] {
            Action::StartThread { title, .. } => assert_eq!(title.chars().count(), 80),
            other => panic!("{other:?}"),
        }
        let many: Vec<String> = (1..=30)
            .map(|i| format!(r#"{{"do":"read_thread","thread":{i}}}"#))
            .collect();
        let (found, problems) = parse_actions(&format!("[{}]", many.join(",")));
        assert_eq!(found.len(), MAX_ACTIONS);
        assert!(problems[0].contains("Only the first"));
    }

    #[test]
    fn the_action_parser_survives_anything() {
        // Mangled versions of good blocks: never a panic, and whatever comes back checks out.
        let seeds = [
            r#"[{"do":"start_thread","title":"App startup time","brief":"Measure and fix","agent":"","isolated":true}]"#,
            r#"{"actions":[{"do":"message_thread","thread":12,"text":"Ship it"},{"do":"resolve_thread","thread":3}]}"#,
        ];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let noise = [
            '{', '}', '[', ']', '"', ':', ',', '\\', '0', '-', 'é', '\n', ' ', '\u{0}',
        ];
        for _ in 0..4000 {
            let mut text: Vec<char> = seeds[(next() % 2) as usize].chars().collect();
            for _ in 0..(next() % 6) {
                let at = (next() as usize) % (text.len() + 1);
                match next() % 3 {
                    0 if at < text.len() => {
                        text.remove(at);
                    }
                    1 => text.insert(at, noise[(next() as usize) % noise.len()]),
                    _ => text.truncate(at),
                }
            }
            let text: String = text.into_iter().collect();
            let (found, _) = parse_actions(&text);
            assert!(found.len() <= MAX_ACTIONS);
            for action in found {
                match action {
                    Action::StartThread { title, brief, .. } => {
                        assert!(!title.trim().is_empty() && !brief.trim().is_empty())
                    }
                    Action::MessageThread { thread, text } => {
                        assert!(thread != 0 && !text.trim().is_empty())
                    }
                    other => assert_ne!(other.thread(), Some(0)),
                }
            }
        }
    }

    #[test]
    fn crew_starts_three_threads_and_holds_the_rest() {
        let ten: Vec<Action> = (0..10).map(|i| start(&format!("Part {i}"))).collect();
        let (run, held, problems) = within_limits(ten, &[]);
        assert_eq!(
            (run.len(), held.len()),
            (MAX_NEW_THREADS, 10 - MAX_NEW_THREADS)
        );
        assert!(problems.is_empty());
        assert_eq!(run[0], start("Part 0"));
        assert_eq!(held[0], start(&format!("Part {MAX_NEW_THREADS}")));
    }

    #[test]
    fn crew_only_reaches_its_own_threads_once_a_turn() {
        let actions = vec![
            Action::MessageThread {
                thread: 5,
                text: "a".into(),
            },
            Action::MessageThread {
                thread: 5,
                text: "b".into(),
            },
            Action::MessageThread {
                thread: 6,
                text: "c".into(),
            },
            Action::ReadThread { thread: 99 },
            Action::ResolveThread { thread: 6 },
        ];
        let (run, held, problems) = within_limits(actions, &[5, 6]);
        assert!(held.is_empty());
        assert_eq!(
            run,
            [
                Action::MessageThread {
                    thread: 5,
                    text: "a".into()
                },
                Action::MessageThread {
                    thread: 6,
                    text: "c".into()
                },
                Action::ResolveThread { thread: 6 },
            ]
        );
        assert_eq!(problems.len(), 2);
    }

    #[test]
    fn messages_go_where_they_are_meant() {
        let threads = vec![
            (4, "Cove setup".to_string()),
            (7, "Cove setup docs".to_string()),
            (9, "App startup time".to_string()),
        ];
        let to = |id: u64, text: &str| Route::Thread {
            id,
            text: text.into(),
        };
        let crew = Route::Crew { answer_only: false };
        for (text, chip, want) in [
            ("What's left on the release?", Chip::Auto, crew.clone()),
            (
                "What's left?",
                Chip::Answer,
                Route::Crew { answer_only: true },
            ),
            ("Fix the login bug", Chip::Thread, Route::NewThread),
            (
                "@Cove setup: use the beta image",
                Chip::Auto,
                to(4, "use the beta image"),
            ),
            (
                "@cove setup docs add a screenshot",
                Chip::Auto,
                to(7, "add a screenshot"),
            ),
            (
                "@App startup time, how far?",
                Chip::Thread,
                to(9, "how far?"),
            ),
            ("@#9 any news", Chip::Auto, to(9, "any news")),
            // A mention with nothing after it, an unknown thread, or a name that only
            // starts the same, goes by the chip.
            ("@Cove setup", Chip::Auto, crew.clone()),
            ("@Nobody hello", Chip::Auto, crew.clone()),
            ("@#3 hello", Chip::Auto, crew.clone()),
            ("@Cove setupx hello", Chip::Auto, crew.clone()),
            ("email me@Cove setup", Chip::Auto, crew.clone()),
            ("@İstanbul hi", Chip::Auto, crew.clone()),
        ] {
            assert_eq!(route(text, chip, &threads), want, "{text}");
        }
    }

    #[test]
    fn crew_hears_only_what_changed() {
        let info = ThreadInfo::default();
        let a = board_line(4, "Cove setup", "working", &info);
        let b = board_line(9, "Startup", "done", &info);
        let first = board_changes("", &[a.clone(), b.clone()]);
        assert_eq!(first.lines().count(), 2);
        let before = [a.clone(), b.clone()].join("\n");
        assert!(board_changes(&before, &[a.clone(), b.clone()]).is_empty());
        let done = board_line(4, "Cove setup", "done", &info);
        let changed = board_changes(&before, &[done.clone(), b.clone()]);
        assert_eq!(changed, format!("- {done}\n"));
        let gone = board_changes(&before, std::slice::from_ref(&a));
        assert_eq!(gone, "- #9 is no longer open.\n");
        // The line carries progress, what waits on the person, and the last report.
        let mut busy = ThreadInfo {
            checklist: clean_steps(vec![
                Step {
                    text: "Measure".into(),
                    mark: Mark::Done,
                },
                Step {
                    text: "Fix".into(),
                    mark: Mark::Doing,
                },
            ]),
            ..Default::default()
        };
        busy.reports.push(Report {
            summary: "Cold start is\n2.1s".into(),
            links: vec![],
            at: 1,
        });
        let line = board_line(4, "Cove setup", "working", &busy);
        assert_eq!(
            line,
            "#4 Cove setup [working] 1/2 steps. Last report: Cold start is 2.1s"
        );
    }

    #[test]
    fn titles_from_messages_are_short() {
        assert_eq!(
            title_from("\n  Fix the login bug\nmore"),
            "Fix the login bug"
        );
        let long = title_from(&"word ".repeat(30));
        assert!(long.ends_with('…') && long.chars().count() <= 61);
    }

    #[test]
    fn rules_tell_crew_how_to_act() {
        let rules = rules(
            "runo",
            &[("claude", "Claude Code"), ("codex", "Codex")],
            "Claude Code",
        );
        assert!(rules.contains("```runo-actions"));
        assert!(rules.contains("codex (Codex)"));
        assert!(rules.contains(&format!("at most {MAX_NEW_THREADS} new threads")));
        // The example in the rules is itself a valid block.
        let example = rules.split("```runo-actions\n").nth(1).unwrap();
        let example = example.split("\n```").next().unwrap();
        assert_eq!(parse_actions(example).0.len(), 1);
        assert!(thread_rules(true).contains("set_checklist"));
        assert!(thread_rules(false).contains("```runo-checklist"));
    }

    #[test]
    fn cards_save_and_load() {
        let card = Card::Held {
            actions: vec![start("Docs")],
            answered: false,
        };
        let text = serde_json::to_string(&card).unwrap();
        assert_eq!(serde_json::from_str::<Card>(&text).unwrap(), card);
        let news: Card = serde_json::from_str(r#"{"card":"news","thread":3}"#).unwrap();
        assert_eq!(news, Card::News { thread: 3 });
    }
}
