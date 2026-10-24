//! The events a turn produces, whichever agent runs it.

use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::crew::Step;
use crate::store::{Question, Usage};

/// One thing that happened in an agent's turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The turn started: which agent, and the model picked ("" for its default).
    TurnStarted { provider: String, model: String },
    /// Reply text to append to the agent's message.
    Text(String),
    /// A tool call. One that is `Running` gets a `ToolDone` with the same id later;
    /// one that is already `Done` (the CLI reports calls only once) gets nothing more.
    Tool(ToolStep),
    /// The result of a running tool call: `ok` is false when the tool failed, `output`
    /// is the end of what it printed, when the CLI passes that on.
    ToolDone {
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        output: String,
    },
    /// The run's summary line, such as "12s · 3 turns · $0.04".
    Summary(String),
    /// The run failed; the CLI's own explanation.
    Error(String),
    /// A warning from the CLI that doesn't end the run, such as an unknown model.
    Notice(String),
    /// Token use: the context after the latest model call, then totals from the result.
    Usage(Usage),
    /// Tools the agent wanted but its permission mode did not allow.
    Denied(Vec<String>),
    /// A plan proposed in plan mode.
    Plan(String),
    /// Multiple-choice questions for the person.
    Questions(Vec<Question>),
    /// The CLI's own id for the conversation, so follow-ups resume exactly this one.
    Conversation(String),
    /// The agent's own todo list, whole, as its todo tool last wrote it: Claude's
    /// TodoWrite, Codex's plan and the like. A Crew thread shows it as its checklist.
    Todos(Vec<Step>),
    /// The turn's process ended; `ok` is how Runo judged it.
    TurnEnded { ok: bool },
    /// Where the agent's plan stands: a usage window near or past its limit.
    Limit(crate::planlimit::PlanLimit),
}

/// What kind of work a tool call does, whatever the CLI calls the tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Write,
    Delete,
    Shell,
    Search,
    List,
    WebSearch,
    Fetch,
    Mcp,
    Subagent,
    Todo,
    Plan,
    Question,
    Think,
    #[default]
    Other,
}

impl ToolKind {
    /// The kind of a tool, by the names the CLIs give their tools.
    pub fn of(tool: &str) -> ToolKind {
        match tool {
            "Read" | "read" | "view" | "read_file" | "ReadFile" => ToolKind::Read,
            "Edit" | "MultiEdit" | "edit" | "edit_file" | "replace" | "str_replace_editor"
            | "ApplyPatch" | "apply_patch" | "NotebookEdit" => ToolKind::Edit,
            "Write" | "write" | "create" | "Create" | "write_file" | "create_file" => {
                ToolKind::Write
            }
            "Delete" | "delete" | "delete_file" => ToolKind::Delete,
            "Bash" | "bash" | "shell" | "Execute" | "run_shell_command" | "exec_command" => {
                ToolKind::Shell
            }
            "Grep" | "grep" | "search_file_content" | "search" => ToolKind::Search,
            "Glob" | "glob" | "LS" | "ls" | "list" | "list_directory" => ToolKind::List,
            "WebSearch" | "web_search" | "google_web_search" | "websearch" => ToolKind::WebSearch,
            "WebFetch" | "web_fetch" | "webfetch" | "fetch" => ToolKind::Fetch,
            "Task" | "Agent" | "task" => ToolKind::Subagent,
            "TodoWrite" | "todowrite" | "write_todos" | "update_todo" | "todo_write"
            | "update_plan" => ToolKind::Todo,
            "ExitPlanMode" => ToolKind::Plan,
            "AskUserQuestion" => ToolKind::Question,
            t if t.starts_with("mcp__") => ToolKind::Mcp,
            _ => ToolKind::Other,
        }
    }

    pub fn is_other(&self) -> bool {
        *self == ToolKind::Other
    }
}

/// How a tool step in an agent turn stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Running,
    #[default]
    Done,
    Failed,
}

impl StepState {
    fn is_done(&self) -> bool {
        *self == StepState::Done
    }
}

/// One tool call of an agent turn, as the conversation shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolStep {
    /// The CLI's id for the call, when it gives one; its result comes back under it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "ToolKind::is_other")]
    pub kind: ToolKind,
    /// The tool's own name, such as "Bash" or "read_file".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    /// A short line for the conversation, such as "Read src/main.rs".
    pub label: String,
    #[serde(default, skip_serializing_if = "StepState::is_done")]
    pub state: StepState,
    /// How long the call took, once its result arrived.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ms: u64,
    /// When the app saw the call start, to time it.
    #[serde(skip)]
    pub started: Option<Instant>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl ToolStep {
    /// A finished step that only has a label, such as one read from a CLI's transcript.
    pub fn done(label: impl Into<String>) -> ToolStep {
        ToolStep {
            label: label.into(),
            ..ToolStep::default()
        }
    }

    /// A call of `tool`, already finished.
    pub fn of(tool: &str, label: impl Into<String>) -> ToolStep {
        ToolStep {
            kind: ToolKind::of(tool),
            tool: tool.to_string(),
            label: label.into(),
            ..ToolStep::default()
        }
    }

    /// A call of `tool` whose result will follow under `id`.
    pub fn running(id: &str, tool: &str, label: impl Into<String>) -> ToolStep {
        ToolStep {
            id: id.to_string(),
            state: StepState::Running,
            ..ToolStep::of(tool, label)
        }
    }

    /// The step with its kind set, for tools whose name doesn't tell it.
    pub fn kind(mut self, kind: ToolKind) -> ToolStep {
        self.kind = kind;
        self
    }

    /// The result arrived: no longer running, failed when `ok` is false, and timed when
    /// the start was seen.
    pub fn finish(&mut self, ok: bool) {
        self.state = if ok {
            StepState::Done
        } else {
            StepState::Failed
        };
        if let Some(start) = self.started.take() {
            self.ms = start.elapsed().as_millis() as u64;
        }
    }
}

impl From<&str> for ToolStep {
    fn from(label: &str) -> ToolStep {
        ToolStep::done(label)
    }
}

impl From<String> for ToolStep {
    fn from(label: String) -> ToolStep {
        ToolStep::done(label)
    }
}

/// The end of a tool's output, at most `max` bytes, cut at a character boundary.
pub fn tail(text: &str, max: usize) -> String {
    let text = text.trim_end();
    if text.len() <= max {
        return text.to_string();
    }
    let mut at = text.len() - max;
    while !text.is_char_boundary(at) {
        at += 1;
    }
    format!("…{}", &text[at..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_tool_names_across_clis() {
        assert_eq!(ToolKind::of("Bash"), ToolKind::Shell);
        assert_eq!(ToolKind::of("run_shell_command"), ToolKind::Shell);
        assert_eq!(ToolKind::of("read_file"), ToolKind::Read);
        assert_eq!(ToolKind::of("apply_patch"), ToolKind::Edit);
        assert_eq!(ToolKind::of("mcp__github__get_me"), ToolKind::Mcp);
        assert_eq!(ToolKind::of("TodoWrite"), ToolKind::Todo);
        assert_eq!(ToolKind::of("frobnicate"), ToolKind::Other);
    }

    #[test]
    fn steps_save_only_what_they_have() {
        let plain = serde_json::to_string(&ToolStep::done("Read a.rs")).unwrap();
        assert_eq!(plain, r#"{"label":"Read a.rs"}"#);
        let mut step = ToolStep::running("t1", "Bash", "Ran false");
        step.finish(false);
        let json = serde_json::to_string(&step).unwrap();
        assert_eq!(
            json,
            r#"{"id":"t1","kind":"shell","tool":"Bash","label":"Ran false","state":"failed"}"#
        );
        let back: ToolStep = serde_json::from_str(&json).unwrap();
        assert_eq!(back, step);
    }

    #[test]
    fn finishing_times_a_step_seen_starting() {
        let mut step = ToolStep::running("t1", "Bash", "Ran sleep 1");
        step.started = Some(Instant::now() - std::time::Duration::from_millis(1500));
        step.finish(true);
        assert_eq!(step.state, StepState::Done);
        assert!(step.ms >= 1500, "{}", step.ms);
        assert!(step.started.is_none());
    }

    #[test]
    fn events_round_trip_as_json() {
        let events = vec![
            AgentEvent::TurnStarted {
                provider: "claude".into(),
                model: "".into(),
            },
            AgentEvent::Text("Hello".into()),
            AgentEvent::Tool(ToolStep::running("t1", "Read", "Read a.rs")),
            AgentEvent::ToolDone {
                id: "t1".into(),
                ok: true,
                output: "fn main() {}".into(),
            },
            AgentEvent::Usage(Usage {
                input: 10,
                output: 5,
                ..Usage::default()
            }),
            AgentEvent::Todos(vec![Step {
                text: "Build".into(),
                mark: crate::crew::Mark::Doing,
            }]),
            AgentEvent::TurnEnded { ok: true },
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let back: AgentEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event, "{json}");
        }
    }

    #[test]
    fn tails_keep_the_end_on_a_char_boundary() {
        assert_eq!(tail("short\n", 10), "short");
        assert_eq!(tail("abcdef", 3), "…def");
        assert_eq!(tail("ééé", 3), "…é");
    }
}
