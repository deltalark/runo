//! Applying a turn's events to its session: reply text, tool steps and their results,
//! warnings, errors, token use, plans, questions and the conversation id. Kept apart from
//! the window so it can be tested on its own.

use super::AgentEvent;
use crate::store::{Message, Role, Session};

/// What the events are applied with.
pub struct Context<'a> {
    /// The agent prints plain text, so each event is a line of it.
    pub plain: bool,
    /// Tools the person already refused this turn; they aren't offered again.
    pub refused: &'a [String],
    /// The model the turn ran on, named in front of its run summary.
    pub model: Option<String>,
}

/// Apply `events` to `session`. Returns true when the agent reported a failure.
pub fn apply(session: &mut Session, events: Vec<AgentEvent>, cx: &Context) -> bool {
    let mut failed = false;
    let id = session.id;
    for event in events {
        if !matches!(session.messages.last(), Some(m) if m.role == Role::Agent) {
            session.messages.push(Message::new(Role::Agent, ""));
        }
        let Some(m) = session.messages.last_mut() else {
            continue;
        };
        match event {
            AgentEvent::Text(text) => {
                if cx.plain && !m.text.is_empty() {
                    m.text.push('\n');
                }
                m.text.push_str(&text);
            }
            AgentEvent::Tool(mut step) => {
                step.started = Some(std::time::Instant::now());
                m.steps.push(step);
            }
            AgentEvent::ToolDone { id, ok, .. } => {
                // The step may sit in an earlier reply when an error message came between.
                for m in session.messages.iter_mut().rev() {
                    if m.finish_step(&id, ok) {
                        break;
                    }
                }
            }
            // Written to the log by whoever starts and ends the turn.
            AgentEvent::TurnStarted { .. } | AgentEvent::TurnEnded { .. } => {}
            // The window holds or warns about the agent's turns; see `note_plan_limit`.
            AgentEvent::Limit(_) => {}
            AgentEvent::Summary(summary) => {
                m.meta = match &cx.model {
                    Some(model) => format!("{model} · {summary}"),
                    None => summary,
                }
            }
            AgentEvent::Error(error) => {
                crate::applog::warn(
                    "agent",
                    format!(
                        "session #{id} ({}): the agent reported an error: {}",
                        session.provider,
                        crate::applog::gist(&error)
                    ),
                );
                failed = true;
                session.messages.push(Message::new(Role::System, error))
            }
            AgentEvent::Notice(text) => {
                let turn = session
                    .messages
                    .iter()
                    .rposition(|m| m.role == Role::User)
                    .unwrap_or(0);
                let seen = session.messages[turn..]
                    .iter()
                    .any(|m| m.meta == "warning" && m.text == text);
                if !seen {
                    crate::applog::warn(
                        "agent",
                        format!(
                            "session #{id} ({}): {}",
                            session.provider,
                            crate::applog::gist(&text)
                        ),
                    );
                    // Ahead of the reply, which carries on below it.
                    let mut warning = Message::new(Role::System, text);
                    warning.meta = "warning".into();
                    let at = session.messages.len() - 1;
                    session.messages.insert(at, warning);
                }
            }
            AgentEvent::Usage(usage) => m.usage = usage,
            AgentEvent::Denied(tools) => {
                m.denied = tools
                    .into_iter()
                    .filter(|t| !cx.refused.contains(t))
                    .collect()
            }
            AgentEvent::Plan(plan) => m.plan = plan,
            AgentEvent::Questions(questions) => m.questions = questions,
            AgentEvent::Conversation(conversation) => session.agent_session = conversation,
            // A thread's checklist follows the agent's own todo list too, so steps it ticks
            // off there show without a second report.
            AgentEvent::Todos(steps) => {
                if session.canvas != crate::crew::CANVAS {
                    if let Some(info) = session.crew.as_mut() {
                        let kind = crate::crew::EventKind::Checklist { steps, title: None };
                        crate::crew::apply(info, &kind, 0);
                    }
                }
            }
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{StepState, ToolStep};
    use crate::store::Usage;

    fn session() -> Session {
        let mut s = Session {
            id: 9,
            provider: "claude".into(),
            ..Session::default()
        };
        s.messages.push(Message::new(Role::User, "go"));
        s
    }

    fn cx() -> Context<'static> {
        Context {
            plain: false,
            refused: &[],
            model: None,
        }
    }

    #[test]
    fn text_and_steps_build_the_reply() {
        let mut s = session();
        let failed = apply(
            &mut s,
            vec![
                AgentEvent::Text("Look".into()),
                AgentEvent::Tool(ToolStep::running("t1", "Bash", "Ran ls")),
                AgentEvent::Text("ing.".into()),
                AgentEvent::ToolDone {
                    id: "t1".into(),
                    ok: false,
                    output: String::new(),
                },
                AgentEvent::Conversation("conv-1".into()),
            ],
            &cx(),
        );
        assert!(!failed);
        let reply = s.messages.last().unwrap();
        assert_eq!(reply.role, Role::Agent);
        assert_eq!(reply.text, "Looking.");
        assert_eq!(reply.steps[0].state, StepState::Failed);
        assert_eq!(s.agent_session, "conv-1");
    }

    #[test]
    fn plain_lines_go_on_their_own_lines() {
        let mut s = session();
        let plain = Context {
            plain: true,
            ..cx()
        };
        apply(
            &mut s,
            vec![AgentEvent::Text("a".into()), AgentEvent::Text("b".into())],
            &plain,
        );
        assert_eq!(s.messages.last().unwrap().text, "a\nb");
    }

    #[test]
    fn an_error_is_its_own_message_and_a_result_finds_its_step_above_it() {
        let mut s = session();
        let failed = apply(
            &mut s,
            vec![
                AgentEvent::Tool(ToolStep::running("t1", "Bash", "Ran make")),
                AgentEvent::Error("overloaded".into()),
                AgentEvent::ToolDone {
                    id: "t1".into(),
                    ok: true,
                    output: String::new(),
                },
            ],
            &cx(),
        );
        assert!(failed);
        let roles: Vec<Role> = s.messages.iter().map(|m| m.role).collect();
        // The result after the error starts a new, empty reply; the step it closes is in
        // the first one.
        assert_eq!(roles, [Role::User, Role::Agent, Role::System, Role::Agent]);
        assert_eq!(s.messages[1].steps[0].state, StepState::Done);
        assert_eq!(s.messages[2].text, "overloaded");
    }

    #[test]
    fn a_warning_shows_once_ahead_of_the_reply() {
        let mut s = session();
        let notice = AgentEvent::Notice("Unknown model".into());
        apply(
            &mut s,
            vec![notice.clone(), AgentEvent::Text("Hi".into()), notice],
            &cx(),
        );
        let warnings: Vec<&Message> = s.messages.iter().filter(|m| m.meta == "warning").collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(s.messages[1].meta, "warning");
        assert_eq!(s.messages[2].text, "Hi");
    }

    #[test]
    fn summaries_usage_and_denials_land_on_the_reply() {
        let mut s = session();
        let refused = vec!["Bash".to_string()];
        let cx = Context {
            plain: false,
            refused: &refused,
            model: Some("Opus".into()),
        };
        let usage = Usage {
            input: 5,
            output: 7,
            ..Usage::default()
        };
        apply(
            &mut s,
            vec![
                AgentEvent::Usage(usage),
                AgentEvent::Denied(vec!["Bash".into(), "Write".into()]),
                AgentEvent::Summary("3s".into()),
                AgentEvent::Plan("1. Do it".into()),
            ],
            &cx,
        );
        let reply = s.messages.last().unwrap();
        assert_eq!(reply.meta, "Opus · 3s");
        assert_eq!(reply.usage, usage);
        assert_eq!(reply.denied, ["Write"]);
        assert_eq!(reply.plan, "1. Do it");
    }
}
