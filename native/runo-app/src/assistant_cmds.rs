//! The assistant's shortcuts and nudges. Slash commands answer at once, without an
//! agent: `/status` sums up what is happening, `/new`, `/race`, `/playbook`, `/stop` and
//! `/approve` lay out action cards. `@` names a session, project or playbook, and a
//! session named that way goes to the agent in more detail. While the panel is closed, the
//! mark offers the one next step when a run finishes, a race is ready to compare or the
//! radar finds a clash.

use super::*;
use crate::assistant_act::{Action, Outcome, MAX_ACTIONS};

/// The slash commands, with what each does, as the input suggests them.
pub(crate) const COMMANDS: [(&str, &str); 10] = [
    ("/status", "What's running, waiting and done"),
    ("/new", "Start an agent on a task"),
    ("/race", "Race your agents on a task"),
    ("/playbook", "Run a playbook"),
    ("/stop", "Stop running agents"),
    ("/approve", "Answer what's waiting for permission"),
    ("/remember", "Keep a preference for later chats"),
    ("/forget", "Drop something remembered"),
    ("/clear", "Start a new chat"),
    ("/help", "What the assistant can do"),
];

/// How many suggestions show over the input.
const SUGGESTIONS: usize = 6;

/// A slash command and what follows it, when `text` is one.
pub(crate) fn command(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    if !text.starts_with('/') {
        return None;
    }
    Some(match text.split_once(char::is_whitespace) {
        Some((cmd, rest)) => (cmd, rest.trim()),
        None => (text, ""),
    })
}

/// The `@` word being typed at the end of `text`, without the `@`.
pub(crate) fn mention_typed(text: &str) -> Option<&str> {
    let at = text.rfind('@')?;
    let before = &text[..at];
    if !(before.is_empty() || before.ends_with(char::is_whitespace)) {
        return None;
    }
    let word = &text[at + 1..];
    // A finished mention is followed by a space; titles may hold spaces, so only a
    // trailing one ends it.
    (!word.ends_with(' ')).then_some(word)
}

/// Something the input suggests: what to show, and the text it puts in the input.
pub(crate) struct Suggestion {
    pub label: String,
    pub detail: String,
    pub fill: String,
}

/// What waits on the mark while the panel is closed: the line it says, the step it
/// offers, and the key that dismisses it for good.
pub(crate) struct Nudge {
    pub key: String,
    pub text: String,
    pub label: &'static str,
    pub action: Action,
}

impl RunoApp {
    /// What the input suggests for `text`: commands after `/`, and sessions, projects
    /// and playbooks after `@`.
    pub(crate) fn assistant_suggestions(&self, text: &str) -> Vec<Suggestion> {
        if text.starts_with('/') && !text.contains(' ') {
            return COMMANDS
                .iter()
                .filter(|(c, _)| c.starts_with(text))
                .map(|(c, what)| Suggestion {
                    label: c.to_string(),
                    detail: what.to_string(),
                    fill: format!("{c} "),
                })
                .collect();
        }
        let Some(word) = mention_typed(text) else {
            return Vec::new();
        };
        let word = word.to_lowercase();
        let head = &text[..text.len() - word.len() - 1];
        let fill = |name: &str| format!("{head}@{name} ");
        let mut sessions: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| !s.archived && s.title.to_lowercase().contains(&word))
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.id));
        let mut out: Vec<Suggestion> = sessions
            .into_iter()
            .map(|s| Suggestion {
                label: s.title.clone(),
                detail: format!("{} · session", agent::provider(&s.provider).name),
                fill: fill(&s.title),
            })
            .collect();
        out.extend(
            self.state
                .projects
                .iter()
                .filter(|p| p.name.to_lowercase().contains(&word))
                .map(|p| Suggestion {
                    label: p.name.clone(),
                    detail: "project".into(),
                    fill: fill(&p.name),
                }),
        );
        out.extend(
            self.state
                .automations
                .iter()
                .filter(|a| a.name.to_lowercase().contains(&word))
                .map(|a| Suggestion {
                    label: a.name.clone(),
                    detail: "playbook".into(),
                    fill: fill(&a.name),
                }),
        );
        out.truncate(SUGGESTIONS);
        out
    }

    /// The sessions `@`-named in `question`, in more detail than the snapshot: the last
    /// few messages and steps, so the agent can say how one is really doing.
    pub(crate) fn assistant_mentioned(&self, question: &str) -> String {
        let mut out = String::new();
        for s in self.state.sessions.iter().filter(|s| !s.archived) {
            if !question.contains(&format!("@{}", s.title)) {
                continue;
            }
            out.push_str(&format!(
                "Session #{} \"{}\", most recent last:\n",
                s.id, s.title
            ));
            for m in s.messages.iter().rev().take(6).rev() {
                let who = match m.role {
                    Role::User => "Person",
                    Role::Agent => "Agent",
                    _ => "Runo",
                };
                let text: String = m.text.split_whitespace().collect::<Vec<_>>().join(" ");
                let text: String = text.chars().take(400).collect();
                out.push_str(&format!("- {who}: {text}\n"));
                for step in m.steps.iter().rev().take(5).rev() {
                    out.push_str(&format!("  · {step}\n"));
                }
            }
        }
        out
    }

    /// Answer a slash command at once. False when `question` isn't one.
    pub(crate) fn assistant_command(&mut self, question: &str, cx: &mut Context<Self>) -> bool {
        let Some((cmd, rest)) = command(question) else {
            return false;
        };
        let (text, actions) = match cmd {
            "/clear" => {
                self.assistant_new_chat();
                return true;
            }
            "/status" => (self.assistant_status(), Vec::new()),
            "/remember" if rest.is_empty() => {
                let notes = crate::assistant_mem::notes_for(
                    &self.assistant.kept.notes,
                    &self.assistant_project_name(),
                );
                let text = if notes.is_empty() {
                    "I don't remember anything here yet. Say what to keep, like `/remember use worktrees for Codex`.".into()
                } else {
                    format!("Here I remember:\n\n- {}", notes.join("\n- "))
                };
                (text, Vec::new())
            }
            "/remember" => {
                let text = match self.assistant_remember(rest) {
                    Ok(done) => format!("{done}. I'll keep it in mind in later chats."),
                    Err(why) => why,
                };
                (text, Vec::new())
            }
            "/forget" => {
                let word = rest.to_lowercase();
                let found: Vec<Action> = self
                    .assistant
                    .kept
                    .notes
                    .iter()
                    .filter(|n| n.text.to_lowercase().contains(&word))
                    .take(MAX_ACTIONS)
                    .map(|n| Action::Forget {
                        note: n.text.clone(),
                    })
                    .collect();
                let text = if found.is_empty() {
                    "I don't remember anything like that.".into()
                } else {
                    "Which one?".into()
                };
                (text, found)
            }
            "/new" | "/race" if rest.is_empty() => (
                format!("Say what to work on, like `{cmd} fix the flaky login test`."),
                Vec::new(),
            ),
            "/new" => (
                "Ready when you are:".into(),
                vec![Action::Start {
                    task: rest.into(),
                    agent: String::new(),
                    isolated: None,
                    project: String::new(),
                }],
            ),
            "/race" => {
                let st = cx.global::<AppSettings>();
                let mut agents: Vec<String> = vec![st.default_provider.clone()];
                agents.extend(
                    agent::PROVIDERS
                        .iter()
                        .filter(|p| {
                            p.id != st.default_provider
                                && agent::available(p.bin, &st.agent(p.id).binary)
                        })
                        .take(2)
                        .map(|p| p.id.to_string()),
                );
                (
                    "Each gets its own worktree; keep the best one.".into(),
                    vec![Action::Race {
                        task: rest.into(),
                        agents,
                        project: String::new(),
                    }],
                )
            }
            "/playbook" => {
                let name = rest.to_lowercase();
                let found: Vec<Action> = self
                    .state
                    .automations
                    .iter()
                    .filter(|a| a.name.to_lowercase().contains(&name))
                    .take(MAX_ACTIONS)
                    .map(|a| Action::Playbook {
                        name: a.name.clone(),
                    })
                    .collect();
                let text = if found.is_empty() {
                    "No playbook matches that.".into()
                } else {
                    "Which one?".into()
                };
                (text, found)
            }
            "/stop" => {
                let running: Vec<Action> = self
                    .state
                    .sessions
                    .iter()
                    .filter(|s| !s.archived && self.is_running(s))
                    .take(MAX_ACTIONS)
                    .map(|s| Action::Stop { session: s.id })
                    .collect();
                let text = if running.is_empty() {
                    "Nothing is running.".into()
                } else {
                    "Running now:".into()
                };
                (text, running)
            }
            "/approve" => {
                let waiting: Vec<Action> = self
                    .approvals
                    .iter()
                    .take(MAX_ACTIONS)
                    .map(|r| Action::Approve {
                        request: r.id.clone(),
                    })
                    .collect();
                let text = if waiting.is_empty() {
                    "No agent is waiting for permission.".into()
                } else {
                    "Waiting for your permission:".into()
                };
                (text, waiting)
            }
            _ => {
                let list: Vec<String> = COMMANDS
                    .iter()
                    .map(|(c, what)| format!("- `{c}` {what}"))
                    .collect();
                (
                    format!(
                        "{}\n\nType `@` to name a session, project or playbook.",
                        list.join("\n")
                    ),
                    Vec::new(),
                )
            }
        };
        self.assistant_say(question, text, actions);
        true
    }

    /// A summary of what is happening, from Runo alone.
    fn assistant_status(&self) -> String {
        let live: Vec<&Session> = self.state.sessions.iter().filter(|s| !s.archived).collect();
        let running: Vec<String> = live
            .iter()
            .filter(|s| self.is_running(s))
            .map(|s| format!("**{}** ({})", s.title, agent::provider(&s.provider).name))
            .collect();
        let done: Vec<String> = live
            .iter()
            .filter(|s| s.unread && !self.is_running(s))
            .map(|s| {
                format!(
                    "**{}** ({})",
                    s.title,
                    Self::status_label(s.status).to_lowercase()
                )
            })
            .collect();
        let waiting = self.approvals.len() + self.waiting_sessions().len();
        let mut lines = Vec::new();
        lines.push(if running.is_empty() {
            "Nothing is running.".to_string()
        } else {
            format!("Running: {}.", running.join(", "))
        });
        if waiting > 0 {
            lines.push(format!(
                "{waiting} thing{} waiting on you; `/approve` lists the permission requests.",
                if waiting == 1 { " is" } else { "s are" }
            ));
        }
        if !done.is_empty() {
            lines.push(format!(
                "Finished since you last looked: {}.",
                done.join(", ")
            ));
        }
        if let Some(hits) = self.git_radar.as_ref().filter(|h| !h.is_empty()) {
            lines.push(format!(
                "The clash radar found {} place{} where two lines of work change the same lines.",
                hits.len(),
                if hits.len() == 1 { "" } else { "s" }
            ));
        }
        if lines.len() == 1 && running.is_empty() {
            return "Nothing is running, and nothing needs you.".to_string();
        }
        lines.join("\n\n")
    }

    /// Put a question and Runo's own answer in the chat.
    fn assistant_say(&mut self, question: &str, text: String, actions: Vec<Action>) {
        self.assistant.chat.push(crate::assistant::Msg::new(
            true,
            question.to_string(),
            Vec::new(),
        ));
        self.assistant.chat.push(crate::assistant::Msg::new(
            false,
            text,
            actions.into_iter().map(|a| (a, Outcome::Ready)).collect(),
        ));
        self.assistant.scroll.scroll_to_bottom();
        self.assistant_keep();
    }

    /// The one thing worth a nudge now, if any: a race ready to compare, a run that
    /// finished while the person was elsewhere, or a clash. Things waiting on the person
    /// have their own cards in the corner, so they aren't repeated here.
    pub(crate) fn assistant_nudge(&self, cx: &App) -> Option<Nudge> {
        if !cx.global::<AppSettings>().assistant_nudges || self.assistant.open {
            return None;
        }
        let seen = |key: &str| self.assistant.nudged.contains(key);
        let live = || self.state.sessions.iter().filter(|s| !s.archived);
        // A race whose agents have all finished, at least one unseen.
        for s in live().filter(|s| s.race != 0 && s.race == s.id) {
            let racers: Vec<&Session> = live().filter(|r| r.race == s.id).collect();
            if racers.len() > 1
                && racers.iter().all(|r| !self.is_running(r))
                && racers.iter().any(|r| r.unread)
            {
                let key = format!(
                    "race:{}:{}",
                    s.id,
                    racers.iter().map(|r| r.messages.len()).sum::<usize>()
                );
                if !seen(&key) {
                    return Some(Nudge {
                        key,
                        text: format!(
                            "The race on \"{}\" is done. Compare and keep the best.",
                            s.title
                        ),
                        label: "Compare",
                        action: Action::Session { id: s.id },
                    });
                }
            }
        }
        let on_screen = (self.view == View::Session)
            .then_some(self.selected)
            .flatten();
        for s in live().filter(|s| s.race == 0 && s.unread && Some(s.id) != on_screen) {
            if self.is_running(s) {
                continue;
            }
            let key = format!("done:{}:{}", s.id, s.messages.len());
            if seen(&key) {
                continue;
            }
            let failed = s.status == Status::Failed;
            return Some(Nudge {
                key,
                text: format!(
                    "{} {} \"{}\".",
                    agent::provider(&s.provider).name,
                    if failed {
                        "stopped with an error on"
                    } else {
                        "finished"
                    },
                    s.title
                ),
                label: if failed { "Look" } else { "Review" },
                action: Action::Session { id: s.id },
            });
        }
        if let Some(hits) = self.git_radar.as_ref().filter(|h| !h.is_empty()) {
            let key = format!("clash:{}", hits.len());
            if !seen(&key) {
                return Some(Nudge {
                    key,
                    text: format!(
                        "Two lines of work change the same lines in {} place{}.",
                        hits.len(),
                        if hits.len() == 1 { "" } else { "s" }
                    ),
                    label: "Show me",
                    action: Action::Open {
                        place: "review".into(),
                    },
                });
            }
        }
        None
    }

    /// The nudge over the mark: one line, the step it offers, and a way to wave it off.
    pub(crate) fn assistant_nudge_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let nudge = self.assistant_nudge(cx)?;
        let key = nudge.key.clone();
        let action = nudge.action.clone();
        let dismiss = nudge.key.clone();
        Some(
            div()
                .id("assistant-nudge")
                .absolute()
                .right(px(76.))
                .bottom(px(38.))
                .w(px(300.))
                .occlude()
                .v_flex()
                .gap_2()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(float_edge())
                .bg(float(c_raised()))
                .shadow_lg()
                .child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .child(nudge.text),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_1()
                        .child(
                            self.button("assistant-nudge-later", "Later", false)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.assistant.nudged.insert(dismiss.clone());
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button("assistant-nudge-go", nudge.label, true)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.assistant.nudged.insert(key.clone());
                                    let _ = this.run_action(&action, window, cx);
                                    cx.notify();
                                })),
                        ),
                )
                .animated(
                    "assistant-nudge-in",
                    Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                    |el, d| el.opacity(d).bottom(px(38. - 8. * (1. - d))),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn commands_split_from_what_follows() {
        assert_eq!(
            command("/race fix the test"),
            Some(("/race", "fix the test"))
        );
        assert_eq!(command("  /status "), Some(("/status", "")));
        assert_eq!(command("how do I race?"), None);
    }

    #[test]
    fn mentions_are_found_while_typed() {
        assert_eq!(mention_typed("how is @auth"), Some("auth"));
        assert_eq!(mention_typed("@"), Some(""));
        assert_eq!(
            mention_typed("how is @auth refactor going"),
            Some("auth refactor going")
        );
        // A mention ended with a space, or an @ inside a word, isn't one being typed.
        assert_eq!(mention_typed("ask @Fix login "), None);
        assert_eq!(mention_typed("mail me@example"), None);
        assert_eq!(mention_typed("no mention"), None);
    }

    #[test]
    fn every_command_is_answered() {
        for (c, what) in COMMANDS {
            assert!(c.starts_with('/') && !what.is_empty());
        }
    }
}
