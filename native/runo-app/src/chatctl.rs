//! Chat context control: compact or clear the agent's conversation, fork a session from an
//! earlier turn, and edit a sent message to send it again.

use super::*;

/// Meta of the divider left where the conversation was cleared or compacted.
const CLEARED: &str = "context:clear";
const COMPACTED: &str = "context:compact:";

/// A slash command Runo answers itself instead of passing it to the agent.
#[derive(PartialEq, Debug)]
pub(crate) enum ChatCommand {
    Clear,
    /// Compact, with optional instructions for what the summary keeps.
    Compact(String),
}

/// The Runo command a sent message stands for, if any.
pub(crate) fn chat_command(provider: &str, text: &str) -> Option<ChatCommand> {
    let rest = text.trim().strip_prefix('/')?;
    let (name, args) = rest
        .split_once(char::is_whitespace)
        .map(|(n, a)| (n, a.trim()))
        .unwrap_or((rest, ""));
    match name {
        "clear" | "new" if args.is_empty() => Some(ChatCommand::Clear),
        "compact" => Some(ChatCommand::Compact(args.to_string())),
        // Gemini and Qwen call it compress.
        "compress" if matches!(provider, "gemini" | "qwen") => {
            Some(ChatCommand::Compact(args.to_string()))
        }
        _ => None,
    }
}

/// Whether the CLI compacts its own conversation when sent `/compact` as a prompt.
pub(crate) fn compacts_itself(provider: &str) -> bool {
    provider == "claude"
}

/// Where the context the agent should still know about starts: after the last clear or
/// compact, else the start.
pub(crate) fn context_from(messages: &[Message]) -> usize {
    messages
        .iter()
        .rposition(|m| m.role == Role::System && m.meta.starts_with("context:"))
        .map_or(0, |i| i + 1)
}

/// The end of the exchange that message `index` belongs to: the next request, or the end.
pub(crate) fn exchange_end(messages: &[Message], index: usize) -> usize {
    messages
        .iter()
        .enumerate()
        .skip(index + 1)
        .find(|(_, m)| m.role == Role::User)
        .map_or(messages.len(), |(i, _)| i)
}

/// Messages copied into a fork: the same conversation, without Undo points of the original.
pub(crate) fn fork_messages(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .cloned()
        .map(|mut m| {
            m.checkpoint.clear();
            m.redo.clear();
            if m.role == Role::Agent && m.resolved.is_empty() {
                m.resolved = "kept".into();
            }
            m
        })
        .collect()
}

/// A summary of `messages` for a fresh conversation, opening with `intro`.
pub(crate) fn summary(session: &Session, messages: &[Message], intro: &str) -> String {
    let in_git = git::is_repo(&session.dir);
    let files: Vec<git::Change> = if in_git {
        git::changes(&session.dir)
            .into_iter()
            .filter(|f| !f.path.starts_with(".runo/"))
            .collect()
    } else {
        Vec::new()
    };
    let commits = if in_git {
        git::recent_commits(&session.dir, 5)
    } else {
        String::new()
    };
    let full = handoff::brief(&handoff::BriefInput {
        from: agent::provider(&session.provider).name,
        dir: &session.dir,
        branch: &session.branch,
        messages,
        files: &files,
        commits: &commits,
        notes: &handoff::read_notes(&session.dir),
    });
    let body = full.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
    format!("{intro}\n{body}")
}

/// Whether the messages hold any request worth summarizing.
fn has_requests(messages: &[Message]) -> bool {
    messages
        .iter()
        .any(|m| m.role == Role::User && !m.text.trim().is_empty())
}

impl RunoApp {
    /// Answer `/clear` or `/compact` typed in the composer. Returns whether it was handled.
    pub(crate) fn run_chat_command(&mut self, id: u64, text: &str, opts: &AgentOptions) -> bool {
        let Some(provider) = self.state.session(id).map(|s| s.provider.clone()) else {
            return false;
        };
        match chat_command(&provider, text) {
            Some(ChatCommand::Clear) => self.clear_context(id),
            Some(ChatCommand::Compact(extra)) => self.compact_context(id, &extra, opts),
            None => return false,
        }
        true
    }

    /// Why the conversation can't be changed right now, if it can't.
    fn context_blocked(&self, id: u64) -> Option<&'static str> {
        if self.turn_running(id) || self.pending_start.contains_key(&id) {
            Some("Wait for the agent to finish first.")
        } else if self.cli_runs.contains_key(&id) {
            Some("This session is open in the CLI. Switch back to Chat first.")
        } else {
            None
        }
    }

    /// Start a fresh agent conversation in the same session and folder.
    pub(crate) fn clear_context(&mut self, id: u64) {
        if let Some(why) = self.context_blocked(id) {
            self.fail("session", why.into());
            return;
        }
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        session.agent_session.clear();
        session.handoff.clear();
        session.carried.clear();
        session.switched_from = None;
        let mut divider = Message::now(Role::System, "");
        divider.meta = CLEARED.into();
        session.messages.push(divider);
        session.conversation_start = session.messages.len();
        self.state.log(id, "Cleared the context".to_string());
        self.save();
        self.info = Some((
            "Context cleared. The next message starts a fresh conversation.".into(),
            std::time::Instant::now(),
        ));
        self.chat_scroll.scroll_to_bottom();
    }

    /// Shrink the conversation: the agent's own `/compact` where its CLI has one, else a
    /// fresh conversation that opens with a summary of this one.
    pub(crate) fn compact_context(&mut self, id: u64, extra: &str, opts: &AgentOptions) {
        if let Some(why) = self.context_blocked(id) {
            self.fail("session", why.into());
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let from = context_from(&session.messages).max(session.conversation_start);
        let recent = &session.messages[from.min(session.messages.len())..];
        if !session.has_conversation() || !has_requests(recent) {
            self.info = Some((
                "There is nothing to compact yet.".into(),
                std::time::Instant::now(),
            ));
            return;
        }
        if compacts_itself(&session.provider) {
            let prompt = format!("/compact {extra}").trim_end().to_string();
            self.start_turn(id, prompt, Vec::new(), opts);
            return;
        }
        let mut brief = summary(
            session,
            &session.messages[context_from(&session.messages)..],
            "You are continuing an existing session whose conversation was compacted to save context. This is not a new session: do not say you have no prior context. This summary replaces the earlier conversation, and its changes are already on disk.",
        );
        if !extra.is_empty() {
            brief.push_str(&format!("\n\nWhen compacting, the person asked: {extra}"));
        }
        let cost = handoff::tokens(&brief);
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        session.agent_session.clear();
        session.handoff = brief.clone();
        session.carried = brief;
        session.switched_from = None;
        let mut divider = Message::now(Role::System, "");
        divider.meta = format!("{COMPACTED}{cost}");
        session.messages.push(divider);
        session.conversation_start = session.messages.len();
        self.state.log(id, "Compacted the context".to_string());
        self.save();
        self.info = Some((
            format!(
                "Compacted to a summary of about {} tokens. The next message starts a fresh conversation with it.",
                handoff::short_count(cost)
            ),
            std::time::Instant::now(),
        ));
        self.chat_scroll.scroll_to_bottom();
    }

    /// Put sent message `index` back in the composer; sending replaces it and what follows.
    pub(crate) fn start_edit(
        &mut self,
        id: u64,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(why) = self.context_blocked(id) {
            self.fail("session", why.into());
            return;
        }
        let Some(m) = self
            .state
            .session(id)
            .and_then(|s| s.messages.get(index))
            .filter(|m| m.role == Role::User)
        else {
            return;
        };
        // A playbook run is edited as the note typed with it.
        let text = if m.playbook.is_empty() {
            m.text.clone()
        } else {
            m.note.clone()
        };
        let images = m.images.clone();
        if self.selected != Some(id) {
            self.select(id);
        }
        self.picked_playbook = None;
        self.editing = Some((id, index));
        self.attachments.insert(id, images);
        self.prompt.update(cx, |p, cx| {
            p.set_value(text, window, cx);
            p.focus(window, cx);
        });
    }

    pub(crate) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, _)) = self.editing.take() else {
            return;
        };
        self.attachments.remove(&id);
        self.prompt.update(cx, |p, cx| p.set_value("", window, cx));
    }

    /// Send an edited message in place of message `index`: files go back to how they were
    /// before it, later messages are dropped, and a fresh conversation picks up from a
    /// summary of the ones before it.
    pub(crate) fn resend_from(
        &mut self,
        id: u64,
        index: usize,
        text: String,
        images: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(why) = self.context_blocked(id) {
            self.fail("session", why.into());
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let Some(cp) = session.messages.get(index).map(|m| m.checkpoint.clone()) else {
            return;
        };
        if cp.is_empty() {
            self.rewound(id, index, None, text, images, cx);
            return;
        }
        if self.git_busy.contains_key(&id) {
            self.fail("session", "Wait for the git step to finish.".into());
            return;
        }
        let dir = session.dir.clone();
        let guard = Some(session.snapshot.clone()).filter(|s| !s.is_empty());
        self.git_busy.insert(id, "Rewinding.".into());
        self.bg(
            move || {
                git::restore(&dir, &cp, guard.as_deref())?;
                Ok::<_, String>(git::checkpoint(&dir).unwrap_or_default())
            },
            move |this, result, cx| {
                this.git_busy.remove(&id);
                match result {
                    Ok(after) => this.rewound(id, index, Some(after), text, images, cx),
                    Err(error) => this.fail("session", format!("Could not rewind: {error}")),
                }
            },
        );
    }

    fn rewound(
        &mut self,
        id: u64,
        index: usize,
        snapshot: Option<String>,
        text: String,
        images: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let index = index.min(session.messages.len());
        let earlier = &session.messages[context_from(&session.messages[..index])..index];
        let brief = has_requests(earlier).then(|| {
            summary(
                session,
                earlier,
                "You are continuing an existing session. The person went back to an earlier point of it and is sending an edited message; anything you remember from after that point no longer applies. This is not a new session: do not say you have no prior context. The files on disk are as they were at that point.",
            )
        });
        let provider = session.provider.clone();
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        let dropped = session.messages.len() - index;
        session.messages.truncate(index);
        session.agent_session.clear();
        session.switched_from = None;
        session.undo_note.clear();
        session.handoff = brief.clone().unwrap_or_default();
        session.carried = brief.unwrap_or_default();
        if let Some(after) = snapshot {
            session.snapshot = after;
        }
        let mut note = Message::now(
            Role::System,
            "Rewound to here. The edited message below replaced the one sent before, and what followed it.",
        );
        note.meta = "note".into();
        session.messages.push(note);
        session.conversation_start = session.messages.len();
        self.turn_files.remove(&id);
        self.state.log(
            id,
            format!("Rewound {dropped} messages to send an edited one"),
        );
        self.save();
        let opts = cx.global::<AppSettings>().agent(&provider);
        self.start_turn(id, text, images, &opts);
    }

    /// A new session that continues from message `index`'s exchange, with the same files.
    pub(crate) fn fork_from(&mut self, id: u64, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let cut = exchange_end(&session.messages, index);
        self.fork_at(id, cut, None, cx);
    }

    /// A new session holding the conversation before message `cut` and the files as they
    /// were then. `first` is sent in it straight away.
    pub(crate) fn fork_at(
        &mut self,
        id: u64,
        cut: usize,
        first: Option<(String, Vec<String>)>,
        cx: &mut Context<Self>,
    ) {
        let Some(src) = self.state.session(id).cloned() else {
            return;
        };
        if src.elsewhere() {
            self.fail(
                "session",
                "Forking works for sessions on this machine.".into(),
            );
            return;
        }
        if cut >= src.messages.len() && self.turn_running(id) {
            self.notice =
                Some("Wait for the agent to finish, or fork from an earlier turn.".into());
            return;
        }
        let Some(project) = self.project_of(&src) else {
            return;
        };
        let cut = cut.min(src.messages.len());
        let in_git = git::is_repo(&src.dir) && git::is_repo(&project.path);
        let new_id = self.state.next_id();
        let branch = if in_git {
            format!("runo/fork-{new_id}")
        } else {
            src.branch.clone()
        };
        let dir = if in_git {
            git::worktree_path(&project.path, &branch)
                .to_string_lossy()
                .into_owned()
        } else {
            src.dir.clone()
        };
        let mut messages = fork_messages(&src.messages[..cut]);
        let mut note = Message::now(
            Role::System,
            format!(
                "Forked from “{}” at this point. The next message starts a new conversation with a summary of the one above.",
                src.title
            ),
        );
        note.meta = "note".into();
        messages.push(note);
        let title: String = format!("{} (fork)", src.title).chars().take(56).collect();
        self.state.sessions.push(Session {
            id: new_id,
            project_id: src.project_id,
            title,
            provider: src.provider.clone(),
            branch: branch.clone(),
            dir,
            isolated: in_git,
            opts: src.opts.clone(),
            status: Status::Idle,
            conversation_start: messages.len(),
            messages,
            base: src.base.clone(),
            limits: src.limits.clone(),
            playbook: src.playbook,
            ..Default::default()
        });
        // The conversation the fork continues: since the last clear, up to the cut.
        let start = context_from(&src.messages[..cut]);
        let context = src.messages[start..cut].to_vec();
        if in_git {
            // The files as they were at the cut: the next request's checkpoint, or now.
            let cp = src
                .messages
                .get(cut)
                .map(|m| m.checkpoint.clone())
                .unwrap_or_default();
            let (repo, src_dir) = (project.path.clone(), src.dir.clone());
            self.preparing.insert(new_id, Vec::new());
            self.terminal.entry(new_id).or_default().push(format!(
                "Creating the worktree for {branch} from “{}”…",
                src.title
            ));
            self.bg(
                move || {
                    let cp = if cp.is_empty() {
                        git::checkpoint(&src_dir)?
                    } else {
                        cp
                    };
                    let dir = git::fork_worktree(&repo, &branch, &cp)?;
                    let mut log = Vec::new();
                    if let Ok(copied) = git::copy_included(&repo, &dir) {
                        if !copied.is_empty() {
                            log.push(format!(
                                "Copied from .worktreeinclude: {}",
                                copied.join(", ")
                            ));
                        }
                    }
                    Ok((dir, branch, log))
                },
                move |this, result: Result<(String, String, Vec<String>), String>, cx| {
                    if let Ok((dir, branch, _)) = &result {
                        if let Some(s) = this.state.session_mut(new_id) {
                            s.dir = dir.clone();
                            s.branch = branch.clone();
                        }
                        this.seed_fork(new_id, &context);
                    }
                    this.worktree_ready(new_id, result, cx);
                },
            );
        } else {
            self.seed_fork(new_id, &context);
        }
        if let Some(pid) = Some(src.project_id).filter(|p| !self.expanded.contains(p)) {
            self.expanded.push(pid);
        }
        self.state.log(new_id, format!("Forked from {}", src.title));
        self.save();
        self.select(new_id);
        if let Some((text, images)) = first {
            let opts = cx.global::<AppSettings>().agent(&src.provider);
            self.start_turn(new_id, text, images, &opts);
        }
        self.info = Some((
            "Forked into a new session. The original is unchanged.".into(),
            std::time::Instant::now(),
        ));
    }

    /// Give a fork the summary its first message carries.
    fn seed_fork(&mut self, id: u64, context: &[Message]) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if !has_requests(context) {
            return;
        }
        let brief = summary(
            session,
            context,
            "You are continuing a session forked from an earlier point of another one. This is not a new session: do not say you have no prior context. The files on disk are as they were at that point.",
        );
        if let Some(s) = self.state.session_mut(id) {
            s.handoff = brief.clone();
            s.carried = brief;
        }
        self.save();
    }

    /// The divider in the chat where the context was cleared or compacted.
    pub(crate) fn context_divider(&self, index: usize, meta: &str) -> Option<AnyElement> {
        let label = if meta == CLEARED {
            "context cleared · fresh conversation".to_string()
        } else {
            let tokens: usize = meta.strip_prefix(COMPACTED)?.parse().ok()?;
            format!(
                "context compacted · ~{} token summary",
                handoff::short_count(tokens)
            )
        };
        let line = || div().flex_1().h(px(1.)).bg(divider());
        Some(
            div()
                .id(("context-div", index))
                .flex()
                .items_center()
                .gap_3()
                .py_1()
                .child(line())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py(px(5.))
                        .rounded_full()
                        .border_1()
                        .border_color(divider())
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(icon(
                            if meta == CLEARED {
                                IconName::Eraser
                            } else {
                                IconName::FoldVertical
                            },
                            12.,
                            c_muted(),
                        ))
                        .child(label),
                )
                .child(line())
                .into_any_element(),
        )
    }

    /// The bar above the composer while a sent message is being edited.
    pub(crate) fn edit_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, index) = self.editing.filter(|(id, _)| Some(*id) == self.selected)?;
        let session = self.state.session(id)?;
        let has_checkpoint = session
            .messages
            .get(index)
            .is_some_and(|m| !m.checkpoint.is_empty());
        let last = session.messages.iter().rposition(|m| m.role == Role::User) == Some(index);
        let text = match (last, has_checkpoint) {
            (true, true) => "Editing your last message. Sending replaces it and its reply, and undoes that turn's file changes.",
            (true, false) => "Editing your last message. Sending replaces it and its reply.",
            (false, true) => "Editing an earlier message. Sending drops it and everything after it, and puts the files back as they were before it.",
            (false, false) => "Editing an earlier message. Sending drops it and everything after it.",
        };
        Some(
            div()
                .min_h(px(28.))
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .bg(tint(c_subtle()))
                .text_size(px(12.5))
                .child(icon(IconName::Pencil, 12., c_blue()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(rgb(c_secondary()))
                        .child(text),
                )
                .child(
                    self.button("edit-fork", "Send as a fork", false)
                        .h(px(22.))
                        .px_2()
                        .text_size(px(12.))
                        .child(icon(IconName::GitFork, 11., c_secondary()))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Leave this session as it is and send the edited message in a new one",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let text = this.prompt.read(cx).value().trim().to_string();
                            if text.is_empty() {
                                return;
                            }
                            this.editing = None;
                            let images = this.attachments.remove(&id).unwrap_or_default();
                            this.prompt.update(cx, |p, cx| p.set_value("", window, cx));
                            this.fork_at(id, index, Some((text, images)), cx);
                            cx.notify();
                        })),
                )
                .child(
                    self.ghost_button_sized("edit-cancel", IconName::X, "Cancel editing", 20.)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_edit(window, cx);
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn msg(role: Role, text: &str) -> Message {
        Message::new(role, text)
    }

    #[test]
    fn reads_runo_commands() {
        assert_eq!(chat_command("claude", "/clear"), Some(ChatCommand::Clear));
        assert_eq!(chat_command("codex", " /new "), Some(ChatCommand::Clear));
        assert_eq!(chat_command("codex", "/clear the cache"), None);
        assert_eq!(
            chat_command("codex", "/compact keep the plan"),
            Some(ChatCommand::Compact("keep the plan".into()))
        );
        assert_eq!(
            chat_command("gemini", "/compress"),
            Some(ChatCommand::Compact(String::new()))
        );
        assert_eq!(chat_command("claude", "/compress"), None);
        assert_eq!(chat_command("claude", "/review 12"), None);
        assert_eq!(chat_command("claude", "please /clear"), None);
        assert!(compacts_itself("claude"));
        assert!(!compacts_itself("codex"));
    }

    #[test]
    fn finds_exchanges_and_context() {
        let mut cleared = msg(Role::System, "");
        cleared.meta = CLEARED.into();
        let messages = vec![
            msg(Role::User, "one"),
            msg(Role::Agent, "a"),
            cleared,
            msg(Role::User, "two"),
            msg(Role::Agent, "b"),
            msg(Role::System, "error"),
            msg(Role::User, "three"),
            msg(Role::Agent, "c"),
        ];
        assert_eq!(exchange_end(&messages, 0), 3);
        assert_eq!(exchange_end(&messages, 1), 3);
        assert_eq!(exchange_end(&messages, 3), 6);
        assert_eq!(exchange_end(&messages, 7), 8);
        assert_eq!(context_from(&messages), 3);
        assert_eq!(context_from(&messages[..2]), 0);
    }

    #[test]
    fn forks_drop_undo_points() {
        let mut user = msg(Role::User, "do it");
        user.checkpoint = "abc".into();
        let mut reply = msg(Role::Agent, "done");
        reply.redo = "def".into();
        let copied = fork_messages(&[user, reply]);
        assert!(copied
            .iter()
            .all(|m| m.checkpoint.is_empty() && m.redo.is_empty()));
        assert_eq!(copied[1].resolved, "kept");
        assert_eq!(copied[0].text, "do it");
    }

    #[test]
    fn summaries_open_with_the_intro() {
        let session = Session {
            provider: "codex".into(),
            dir: "/nowhere".into(),
            ..Default::default()
        };
        let messages = vec![msg(Role::User, "Fix the login"), msg(Role::Agent, "Fixed")];
        let text = summary(&session, &messages, "Compacted.");
        assert!(text.starts_with("Compacted.\nFolder: /nowhere"));
        assert!(text.contains("Fix the login"));
        assert!(!text.contains("handed over"));
    }
}
