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

    /// Remove a connection; its sessions move back to this machine, and what they and
    /// agents left there is deleted.
    pub(crate) fn remove_connection(&mut self, id: u64, cx: &mut Context<Self>) {
        let busy = self
            .state
            .sessions
            .iter()
            .any(|s| s.connection == id && self.handles.contains_key(&Target::Session(s.id)));
        if busy {
            self.fail("connection", format!(
                "An agent is still working on {}. Let it finish or stop it, then remove the connection.",
                connections::name(id)
            ));
            cx.notify();
            return;
        }
        if let Some(conn) = connections::get(id) {
            for s in self
                .state
                .sessions
                .iter()
                .filter(|s| s.connection == id || s.left_on.contains(&id))
            {
                let folder = std::path::Path::new(&s.dir)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                connections::cleanup(&conn, &folder, s.id);
            }
            connections::cleanup_scratch(&conn);
        }
        for s in self.state.sessions.iter_mut() {
            s.left_on.retain(|c| *c != id);
        }
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

    /// Install Cove on this machine, or start its server, in the background, and
    /// repaint as it goes.
    pub(crate) fn set_up_cove(&mut self, start_only: bool, cx: &mut Context<Self>) {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let done = move || {
            let _ = tx.send(());
        };
        if start_only {
            crate::cove::start(done);
        } else {
            crate::cove::install(done);
        }
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            let finished = !matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
            let _ = this.update(cx, |_, cx| cx.notify());
            if finished {
                break;
            }
        })
        .detach();
        cx.notify();
    }

    /// Add a Cove connection for a server and say what to do next.
    pub(crate) fn add_cove_server(&mut self, cx: &mut Context<Self>) {
        let id = self.add_connection(Kind::Cove, cx);
        connections::set_pair_result(
            id,
            (
                None,
                "Type the server's SSH login above, such as ubuntu@203.0.113.7, and press Set up."
                    .into(),
            ),
        );
        cx.notify();
    }

    /// Set a Cove server up from its SSH login, in the background: install or update Cove
    /// there, open the tunnel and pair. Settings shows each step.
    pub(crate) fn set_up_cove_server(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(conn) = connections::get(id) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        crate::coveserver::set_up(conn, move || {
            let _ = tx.send(());
        });
        self.watch_server_setup(id, rx, cx);
    }

    /// The server's sudo wants a password: do the root steps in a terminal window.
    pub(crate) fn set_up_cove_in_terminal(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(conn) = connections::get(id) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        match crate::coveserver::set_up_in_terminal(conn, move || {
            let _ = tx.send(());
        }) {
            Ok(()) => self.watch_server_setup(id, rx, cx),
            Err(e) => {
                self.fail("connection", format!("Couldn't open a terminal: {e}"));
                cx.notify();
            }
        }
    }

    /// Repaint while a server setup runs; once it is paired, keep the URL, tunnel port and
    /// key on the connection, and check it.
    fn watch_server_setup(
        &mut self,
        id: u64,
        rx: std::sync::mpsc::Receiver<()>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            let finished = !matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
            let _ = this.update(cx, |this, cx| {
                if finished {
                    if let Some(crate::coveserver::Step::Done { url, key, port }) =
                        crate::coveserver::step(id)
                    {
                        settings::update(cx, move |s| {
                            if let Some(c) = s.connections.iter_mut().find(|c| c.id == id) {
                                c.url = url;
                                c.api_key = key;
                                c.tunnel_port = port;
                                c.pair_code.clear();
                            }
                        });
                        this.save();
                        crate::coveserver::clear_step(id);
                        crate::cove::forget_logins();
                        this.info = Some((
                            format!(
                                "{} is set up and paired. Sign the agents in under it in Settings.",
                                connections::name(id)
                            ),
                            std::time::Instant::now(),
                        ));
                        this.test_connection(id, cx);
                    }
                }
                cx.notify();
            });
            if finished {
                break;
            }
        })
        .detach();
        cx.notify();
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

    /// Trade the pairing code typed for a connection for a key of Runo's own, in the
    /// background, and keep the key in the connection's settings.
    pub(crate) fn pair_connection(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(conn) = connections::get(id) else {
            return;
        };
        let code = cx
            .global::<settings::AppSettings>()
            .connections
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.pair_code.clone())
            .unwrap_or_default();
        if code.trim().is_empty() {
            connections::set_pair_result(
                id,
                (
                    Some(false),
                    "Run cove pair on the server and type its code first.".into(),
                ),
            );
            cx.notify();
            return;
        }
        connections::set_pair_result(id, (None, "Pairing…".into()));
        let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let _ = tx.send(connections::pair(&conn, &code));
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let answer = match rx.try_recv() {
                Ok(a) => Some(a),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("Pairing stopped.".into())),
            };
            let Some(answer) = answer else {
                let _ = this.update(cx, |_, cx| cx.notify());
                continue;
            };
            let _ = this.update(cx, |this, cx| {
                match answer {
                    Ok(key) => {
                        settings::update(cx, move |s| {
                            if let Some(c) = s.connections.iter_mut().find(|c| c.id == id) {
                                c.api_key = key;
                                c.pair_code.clear();
                            }
                        });
                        this.save();
                        connections::set_pair_result(
                            id,
                            (
                                Some(true),
                                "Paired: Runo has a key of its own for this server.".into(),
                            ),
                        );
                        this.test_connection(id, cx);
                    }
                    Err(e) => connections::set_pair_result(id, (Some(false), e)),
                }
                cx.notify();
            });
            break;
        })
        .detach();
        cx.notify();
    }

    /// Move a session to another connection. Its files follow on the next turn, and its
    /// conversation too when the agent's can travel; otherwise the next message carries
    /// a handoff brief. It can't move while the agent is working.
    pub(crate) fn move_session(&mut self, session: u64, to: u64, cx: &mut Context<Self>) {
        let busy = self.handles.contains_key(&Target::Session(session));
        let Some(s) = self.state.session(session) else {
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
        let target = connections::get(to);
        // Remote copies are kept in step with git; a plain folder has no way there.
        if target.as_ref().is_some_and(|c| c.remote()) && !git::is_repo(&s.dir) {
            self.fail("connection", format!(
                "{} can't run this session: its folder isn't a git repository, and Runo syncs files to {} with git. Run `git init` in the project (or start a session in a repository), then move it.",
                connections::name(to),
                connections::name(to)
            ));
            cx.notify();
            return;
        }
        let from_conn = connections::get(s.connection);
        let carries = connections::carries_conversation(
            &s.provider,
            !s.agent_session.is_empty(),
            from_conn.as_ref(),
            target.as_ref(),
        );
        let agent_name = agent::provider(&s.provider).name;
        let to_name = connections::name(to);
        // The conversation stays behind: the next message gets a brief of it instead.
        let brief = (s.has_conversation() && !carries).then(|| {
            let (files, commits) = if git::is_repo(&s.dir) {
                (git::changes(&s.dir), git::recent_commits(&s.dir, 5))
            } else {
                (Vec::new(), String::new())
            };
            let notes =
                std::fs::read_to_string(std::path::Path::new(&s.dir).join(handoff::NOTES_FILE))
                    .unwrap_or_default();
            let full = handoff::brief(&handoff::BriefInput {
                from: agent_name,
                dir: &s.dir,
                branch: &s.branch,
                messages: &s.messages,
                files: &files,
                commits: &commits,
                notes: &notes,
            });
            let body = full.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            format!(
                "You are continuing an existing session that just moved to another machine ({to_name}). Your own record of the conversation stayed on the machine it ran on, so this is a summary of it. This is not a new session: do not say you have no prior context. The session's files came along, changes included.\n{body}"
            )
        });
        let from = connections::name(s.connection);
        let Some(s) = self.state.session_mut(session) else {
            return;
        };
        if s.connection != connections::LOCAL && !s.left_on.contains(&s.connection) {
            s.left_on.push(s.connection);
        }
        s.left_on.retain(|c| *c != to);
        s.connection = to;
        let text = match &brief {
            Some(brief) => {
                s.handoff = brief.clone();
                s.carried = brief.clone();
                s.agent_session.clear();
                format!(
                    "Moved from {from} to {to_name}. {agent_name} can't take its own conversation there, so the next message carries a summary of it, with the latest files."
                )
            }
            None => format!(
                "Moved from {from} to {to_name}. The next message runs there with the latest files and the same conversation."
            ),
        };
        let mut note = Message::new(Role::System, text);
        note.meta = "moved".into();
        s.messages.push(note);
        if brief.is_some() {
            s.conversation_start = s.messages.len();
        }
        self.save();
        cx.notify();
    }

    /// The runner's warnings from a turn on a connection, as a message for the chat: work
    /// that stayed there or was kept aside must not pass as "Agent finished".
    pub(crate) fn sync_warning(&self, id: u64) -> Option<Message> {
        let lines = self.stderr.get(&id)?;
        let warnings = connections::warnings(lines);
        if warnings.is_empty() {
            return None;
        }
        let mut note = Message::now(Role::System, warnings.join("\n\n"));
        note.meta = "sync".into();
        Some(note)
    }

    /// Put a kept version of the files back, from the warning card that named it.
    pub(crate) fn restore_kept(&mut self, id: u64, index: usize, kept: &str) {
        if self.handles.contains_key(&Target::Session(id)) {
            self.fail(
                "connection",
                "Wait for the agent to finish, then restore the kept files.".into(),
            );
            return;
        }
        let Some(dir) = self.state.session(id).map(|s| s.dir.clone()) else {
            return;
        };
        match connections::restore_kept(&dir, id, kept) {
            Ok(saved) => {
                if let Some(m) = self
                    .state
                    .session_mut(id)
                    .and_then(|s| s.messages.get_mut(index))
                {
                    m.resolved = format!("restored {saved}");
                }
                self.state.log(id, "Restored kept changes");
                self.info = Some((
                    "Restored the kept files. What was here before is saved too.".into(),
                    std::time::Instant::now(),
                ));
                self.save();
                self.measure(id);
                self.refresh();
            }
            Err(e) => self.fail(
                "connection",
                format!("Couldn't restore the kept files: {e}"),
            ),
        }
    }

    /// A warning from a turn on a connection, with a way to restore kept files.
    pub(crate) fn sync_card(
        &self,
        index: usize,
        message: &Message,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sid = self.selected.unwrap_or(0);
        let kept = connections::kept_refs(&message.text);
        let restored = message.resolved.strip_prefix("restored ").map(String::from);
        let mut actions = div().flex().flex_wrap().items_center().gap_2();
        match &restored {
            Some(saved) => {
                actions = actions.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .child(format!(
                            "Restored. What was here before is saved as {saved}."
                        )),
                )
            }
            None => {
                for (k, r) in kept.into_iter().enumerate() {
                    actions = actions.child(
                        self.button(
                            ("sync-restore", index * 8 + k),
                            "Restore kept changes",
                            false,
                        )
                        .h(px(26.))
                        .child(icon(IconName::RotateCcw, 12., c_secondary()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.restore_kept(sid, index, &r);
                            cx.notify();
                        })),
                    );
                }
            }
        }
        fade(
            div()
                .id(("message", index))
                .px_3()
                .py(px(10.))
                .flex()
                .items_start()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(rgb(settings::mix(c_amber(), c_canvas(), 0.4)))
                .bg(rgb(settings::mix(c_amber(), c_canvas(), 0.08)))
                .text_size(px(13.))
                .child(
                    div()
                        .pt(px(2.))
                        .child(icon(IconName::TriangleAlert, 14., c_amber())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .v_flex()
                        .gap_2()
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("sync-body-{sid}-{index}")),
                                message.text.clone(),
                            )
                            .selectable(true),
                        )
                        .child(actions),
                ),
            ("sync-card", index),
            200,
        )
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
                    let turn = session
                        .messages
                        .iter()
                        .rposition(|m| m.role == Role::User)
                        .unwrap_or(0);
                    self.logs.insert(
                        id,
                        crate::harness::log::TurnLog::resume(crate::harness::log::root(), id, turn),
                    );
                    if let Some(parser) = stream::Parser::for_provider(&provider, &dir) {
                        self.streams.insert(id, parser);
                    }
                    self.handles.insert(Target::Session(id), handle);
                    self.started.insert(id, std::time::Instant::now());
                    picked += 1;
                }
                Err(e) => {
                    crate::applog::warn(
                        "connection",
                        format!(
                            "session #{id} ({}): couldn't pick up the turn left running: {e}",
                            session.provider
                        ),
                    );
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
