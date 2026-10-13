//! Adding, testing and removing connections, and moving a session between them.

use super::*;
use crate::connections::{self, Connection, Kind};

impl RunoApp {
    /// Add a connection with sensible defaults and return its id.
    pub(crate) fn add_connection(&mut self, kind: Kind, cx: &mut Context<Self>) -> u64 {
        let all = &cx.global::<AppSettings>().connections;
        let id = connections::next_id(all);
        let mut conn = match kind {
            Kind::Ssh => Connection::new_ssh(id),
            _ => Connection::new_cove(id),
        };
        // "Cove", "Cove 2"…: names stay unique so Runs on is unambiguous.
        let base = conn.name.clone();
        let mut n = 1;
        while all.iter().any(|c| c.name == conn.name) {
            n += 1;
            conn.name = format!("{base} {n}");
        }
        settings::update(cx, move |s| s.connections.push(conn));
        id
    }

    /// Remove a connection; its sessions move back to this machine.
    pub(crate) fn remove_connection(&mut self, id: u64, cx: &mut Context<Self>) {
        let prefix = format!("conn.{id}.");
        self.settings_inputs.retain(|k, _| !k.starts_with(&prefix));
        for s in self
            .state
            .sessions
            .iter_mut()
            .filter(|s| s.connection == id)
        {
            s.connection = connections::LOCAL;
        }
        for p in self
            .state
            .projects
            .iter_mut()
            .filter(|p| p.connection == id)
        {
            p.connection = connections::LOCAL;
        }
        if self.new_connection == id {
            self.new_connection = connections::LOCAL;
        }
        settings::update(cx, move |s| s.connections.retain(|c| c.id != id));
        self.save();
    }

    /// Check a connection in the background and repaint when the answer is in.
    pub(crate) fn test_connection(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(conn) = connections::get(id) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        connections::test(conn, move || {
            let _ = tx.send(());
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let done = !matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
            let _ = this.update(cx, |_, cx| cx.notify());
            if done {
                break;
            }
        })
        .detach();
        cx.notify();
    }

    /// Move a session to another connection. Its files and conversation follow on the
    /// next turn; it can't move while the agent is working.
    pub(crate) fn move_session(&mut self, session: u64, to: u64, cx: &mut Context<Self>) {
        let busy = self.handles.contains_key(&Target::Session(session));
        let Some(s) = self.state.session_mut(session) else {
            return;
        };
        if s.connection == to {
            return;
        }
        if busy {
            self.info = Some((
                "Wait for the agent to finish this turn, then move the session.".into(),
                std::time::Instant::now(),
            ));
            cx.notify();
            return;
        }
        let from = connections::name(s.connection);
        s.connection = to;
        let to_name = connections::name(to);
        let mut note = Message::new(
            Role::System,
            format!(
                "Moved from {from} to {to_name}. The next message runs there with the latest files and the same conversation."
            ),
        );
        note.meta = "moved".into();
        s.messages.push(note);
        self.save();
        cx.notify();
    }

    /// The session header's "Runs on" control: where it runs; click to move it.
    pub(crate) fn runs_on_badge(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if connections::all().is_empty() && session.connection == connections::LOCAL {
            return None;
        }
        let local = session.connection == connections::LOCAL;
        Some(
            div()
                .id("hdr-runs-on")
                .h(px(26.))
                .px_2()
                .mr_2()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(rgb(if local { c_secondary() } else { c_blue() }))
                .hover(|el| el.bg(tint(c_subtle())))
                .on_mouse_down(
                    MouseButton::Left,
                    self.on_context(ctxmenu::CtxTarget::RunsOn(session.id), cx),
                )
                .child(icon(
                    if local {
                        IconName::Cpu
                    } else {
                        IconName::Server
                    },
                    11.,
                    if local { c_secondary() } else { c_blue() },
                ))
                .child(if local {
                    "This machine".to_string()
                } else {
                    connections::name(session.connection)
                })
                .into_any_element(),
        )
    }

    /// Turns that were running on connections when Runo closed kept going there. Show
    /// them as running again: their output replays from the start and their work comes
    /// back when they end.
    pub(crate) fn resume_remote_turns(&mut self) {
        let ids: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.status == Status::Working && s.remote_turn)
            .map(|s| s.id)
            .collect();
        let mut picked = 0;
        for id in ids {
            let backup = settings::sync_backup();
            let Some(session) = self.state.session_mut(id) else {
                continue;
            };
            let conversation = (!session.agent_session.is_empty())
                .then(|| (session.provider.clone(), session.agent_session.clone()));
            let dir = session.dir.clone();
            let provider = session.provider.clone();
            let started = match session.sandbox() {
                Some(connections::Sandbox::Remote(r)) => {
                    connections::resume_command(&r, conversation, backup)
                        .and_then(|c| agent::spawn(Target::Session(id), c, &dir, self.tx.clone()))
                }
                _ => Err("its connection was removed".into()),
            };
            match started {
                Ok(handle) => {
                    // The replay rebuilds the reply from the start.
                    if let Some(m) = session
                        .messages
                        .last_mut()
                        .filter(|m| m.role == Role::Agent)
                    {
                        *m = Message::now(Role::Agent, "");
                    }
                    if let Some(parser) = stream::Parser::for_provider(&provider, &dir) {
                        self.streams.insert(id, parser);
                    }
                    self.handles.insert(Target::Session(id), handle);
                    self.started.insert(id, std::time::Instant::now());
                    picked += 1;
                }
                Err(e) => {
                    session.status = Status::Failed;
                    session.remote_turn = false;
                    session.messages.push(Message::new(
                        Role::System,
                        format!("Couldn't pick up the running turn: {e}"),
                    ));
                }
            }
        }
        if picked > 0 {
            self.info = Some((
                format!(
                    "Picked up {picked} turn{} that kept running on a connection while Runo was closed.",
                    if picked == 1 { "" } else { "s" }
                ),
                std::time::Instant::now(),
            ));
        }
        self.save();
    }
}
