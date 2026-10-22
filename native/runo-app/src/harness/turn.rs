//! One running turn of a session: its parser, its event log and how it stands. The app
//! keeps one per session that has an agent running, from the moment the turn starts (or
//! is picked up again after a restart) until its process ends, instead of a map per
//! piece of state.

use std::path::Path;
use std::time::Instant;

use super::log::TurnLog;
use super::AgentEvent;
use crate::stream::Parser;

pub struct Turn {
    session: u64,
    provider: String,
    /// `None` for agents whose output is plain text.
    parser: Option<Parser>,
    log: TurnLog,
    /// When the turn started, or was picked up again.
    pub started: Instant,
    /// The person (or Runo, for a race it settled) stopped the turn.
    pub stopping: bool,
    /// The CLI waited for a browser sign-in, so Runo stopped it.
    pub signed_out: bool,
    /// The agent reported a failure in its output, whatever its exit code says.
    pub failed: bool,
    /// The agent takes messages while it works (see `host`), so a prompt sent now goes
    /// to it instead of waiting for the turn to end.
    pub live: bool,
}

impl Turn {
    /// A new turn of `session` whose prompt is message `turn`, logged from its start.
    pub fn start(session: u64, provider: &str, dir: &str, turn: usize, model: &str) -> Turn {
        Turn::start_in(super::log::root(), session, provider, dir, turn, model)
    }

    fn start_in(
        root: Option<&Path>,
        session: u64,
        provider: &str,
        dir: &str,
        turn: usize,
        model: &str,
    ) -> Turn {
        let log = TurnLog::start(root, session, turn, provider, model);
        Turn::new(session, provider, dir, log)
    }

    /// A turn picked up after Runo restarted. Its output replays from the start; what
    /// the log already has of it isn't written again.
    pub fn resume(session: u64, provider: &str, dir: &str, turn: usize) -> Turn {
        let log = TurnLog::resume(super::log::root(), session, turn);
        Turn::new(session, provider, dir, log)
    }

    /// A turn that isn't part of a session and keeps no log, such as a scheduled
    /// playbook run while Runo is closed, which reports through its own file.
    pub fn unlogged(provider: &str, dir: &str) -> Turn {
        Turn::new(0, provider, dir, TurnLog::off())
    }

    fn new(session: u64, provider: &str, dir: &str, log: TurnLog) -> Turn {
        Turn {
            session,
            provider: provider.to_string(),
            parser: Parser::for_provider(provider, dir),
            log,
            started: Instant::now(),
            stopping: false,
            signed_out: false,
            failed: false,
            live: false,
        }
    }

    /// Whether the agent's output is plain text rather than structured events.
    pub fn plain(&self) -> bool {
        self.parser.is_none()
    }

    /// One line of the agent's output: the events it gives, also written to the log.
    pub fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        let Some(parser) = self.parser.as_mut() else {
            // Plain output is kept line by line.
            self.log.record(&AgentEvent::Text(format!("{line}\n")));
            return vec![AgentEvent::Text(line.to_string())];
        };
        super::record::line(&self.provider, self.session, self.log.turn(), line);
        let events = parser.feed(line);
        for event in &events {
            self.log.record(event);
        }
        events
    }

    /// The process ended: the log gets how Runo judged the turn.
    pub fn end(mut self, ok: bool) {
        self.log.end(ok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::log;

    fn root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-turn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_structured_turn_parses_and_logs_each_line() {
        let dir = root("claude");
        let mut turn = Turn::start_in(Some(&dir), 3, "claude", "/w", 0, "");
        assert!(!turn.plain());
        let events = turn.feed(
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Hi"}]}}"#,
        );
        assert_eq!(events, [AgentEvent::Text("Hi".into())]);
        turn.end(true);
        let logged: Vec<AgentEvent> = log::read(&dir, 3).into_iter().map(|r| r.event).collect();
        assert_eq!(logged.len(), 3, "{logged:?}");
        assert_eq!(logged[1], AgentEvent::Text("Hi".into()));
        assert_eq!(logged[2], AgentEvent::TurnEnded { ok: true });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_plain_turn_passes_lines_through_and_logs_them_whole() {
        let dir = root("plain");
        // Grok prints plain text.
        let mut turn = Turn::start_in(Some(&dir), 4, "grok", "/w", 2, "");
        assert!(turn.plain());
        assert_eq!(turn.feed("one"), [AgentEvent::Text("one".into())]);
        assert_eq!(turn.feed("two"), [AgentEvent::Text("two".into())]);
        turn.end(false);
        let logged: Vec<AgentEvent> = log::read(&dir, 4).into_iter().map(|r| r.event).collect();
        assert_eq!(logged[1], AgentEvent::Text("one\ntwo\n".into()));
        assert_eq!(logged[2], AgentEvent::TurnEnded { ok: false });
        let _ = std::fs::remove_dir_all(dir);
    }
}
