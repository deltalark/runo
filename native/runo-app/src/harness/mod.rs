//! The agent harness: what a turn of any agent CLI produces, in one typed shape, and how
//! a running turn is kept. `turn::Turn` holds a session's running turn (its parser from
//! `stream`, its event log and how it stands); `apply` puts the events on the session's
//! conversation; `log` appends them to `sessions/<id>/events.jsonl`.

pub mod acp;
pub mod apply;
pub mod codex;
pub mod event;
#[cfg(test)]
mod fixtures;
pub mod host;
pub mod log;
pub mod pi;
pub mod record;
pub mod rpc;
pub mod turn;

pub use event::{AgentEvent, StepState, ToolKind, ToolStep};
