//! The agent harness: what a turn of any agent CLI produces, in one typed shape, and the
//! per-session log those events are kept in. Parsers in `stream` turn each CLI's output
//! into `AgentEvent`s; the app applies them to the conversation and `log` appends them to
//! `sessions/<id>/events.jsonl`.

pub mod event;
pub mod log;

pub use event::{AgentEvent, StepState, ToolKind, ToolStep};
