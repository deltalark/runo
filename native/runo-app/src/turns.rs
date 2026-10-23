//! Agent turns: queued prompts, per-turn checkpoints with Undo / Keep, image attachments,
//! @file mentions, plans, questions and permission requests.

use super::*;
use crate::store::Queued;
use gpui_kit::component::input;

/// An entry of the composer's `/` menu.
#[derive(Clone)]
pub(crate) enum SlashPick {
    Playbook(u64),
    Command(agentcmds::Command),
}

/// An entry of the composer's `@` menu.
#[derive(Clone)]
pub(crate) enum MentionPick {
    /// A Crew thread, in Crew's chat when the message starts with `@`.
    Thread(u64),
    File(String),
}

/// Most recent conversation, written into follow-ups for CLIs that cannot resume.
/// Marks a message sent to the agent while its turn ran.
pub(crate) const STEER: &str = "sent while working";
const CONTEXT_MESSAGES: usize = 8;
const CONTEXT_CHARS: usize = 1500;
/// Earlier requests summed up in one line each, before the recent conversation.
const EARLIER_REQUESTS: usize = 10;
const EARLIER_CHARS: usize = 120;

impl RunoApp {
    /// Start one agent turn: snapshot the worktree, then launch the CLI.
    pub(crate) fn start_turn(
        &mut self,
        id: u64,
        prompt: String,
        images: Vec<String>,
        opts: &AgentOptions,
    ) {
        // Whatever goes to a Crew thread next answers the decision it waits on: a choice,
        // a reply typed in the thread, or a message from Crew.
        if let Some(info) = self.state.session_mut(id).and_then(|s| s.crew.as_mut()) {
            if info.decision.take().is_some() {
                info.attention = None;
            }
        }
        // The worktree is still being made: the turn starts once it is ready.
        if let Some(waiting) = self.preparing.get_mut(&id) {
            waiting.push(crate::ship::Deferred {
                prompt,
                images,
                opts: opts.clone(),
                tag: None,
            });
            self.info = Some((
                "Starts as soon as the worktree is ready.".into(),
                std::time::Instant::now(),
            ));
            return;
        }
        // One run per session: a prompt that arrives while the agent or the worktree setup
        // command is running waits its turn, unless the agent takes it as it works.
        if self.handles.contains_key(&Target::Session(id)) || self.pending_start.contains_key(&id) {
            if images.is_empty() && self.steer(id, &prompt) {
                return;
            }
            if let Some(s) = self.state.session_mut(id) {
                s.queue.push(Queued::new(prompt, images));
            }
            self.save();
            self.info = Some((
                "Queued. It is sent when the agent finishes.".into(),
                std::time::Instant::now(),
            ));
            return;
        }
        self.stderr.remove(&id);
        // The CLI view owns the conversation while it is open; two writers would fork it.
        if self.cli_runs.contains_key(&id) {
            if let Some(s) = self.state.session_mut(id) {
                s.queue.insert(0, Queued::new(prompt, images));
            }
            self.fail("turn", 
                "This session is open in the CLI. The prompt is queued for when you switch back to Chat."
                    .into(),
            );
            return;
        }
        // A spending limit or the cap on running agents holds the prompt in the queue.
        if let Some(why) = self.hold_turn(id) {
            if let Some(s) = self.state.session_mut(id) {
                s.queue.insert(0, Queued::new(prompt, images));
            }
            self.save();
            self.info = Some((why, std::time::Instant::now()));
            return;
        }
        // Crew's rules and what changed in its threads, or a thread's rules: unseen context.
        let preface = self.crew_preface(id);
        // Notes the person shared with agents open each new conversation in the project.
        let notes = self
            .state
            .session(id)
            .filter(|s| !s.has_conversation())
            .and_then(|s| self.shared_notes(s.project_id));
        // A playbook's own session names it on its first message, however late that starts.
        let playbook = self
            .state
            .session(id)
            .filter(|s| s.playbook != 0 && !s.messages.iter().any(|m| m.role == Role::User))
            .and_then(|s| self.state.automations.iter().find(|a| a.id == s.playbook))
            .map(|a| a.name.clone());
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        // A session a playbook started keeps to its limits on every turn.
        let held = crate::workspace::held_options(session, &opts.for_project(session.project_id));
        let opts = &held;
        let mode = session
            .selections()
            .get("mode")
            .cloned()
            .unwrap_or_default();
        if let Some(error) = agent::mode_error(&session.provider, &mode) {
            crate::applog::warn(
                "turn",
                format!(
                    "session #{id} ({}): not started, mode {mode:?}: {error}",
                    session.provider
                ),
            );
            session.status = Status::Failed;
            session.messages.push(Message::new(Role::System, error));
            self.save();
            return;
        }
        let has = session.has_conversation();
        let provider = session.provider.clone();
        // Resume the CLI's own conversation when its id is known (Claude Code and Codex can
        // also pick up the newest one in the folder). Otherwise the history goes in the prompt.
        let native = has
            && agent::resumes(&provider)
            && (matches!(provider.as_str(), "claude" | "codex")
                || !session.agent_session.is_empty());
        if agent::names_conversation(&provider) && !native {
            session.agent_session = agent::new_uuid();
        }
        let resume = has;
        let dir = session.dir.clone();
        let in_git = git::is_repo(&dir);
        // Agents that cannot resume their own conversation get it written into the prompt.
        let handoff = std::mem::take(&mut session.handoff);
        // Once a message goes out the switch is final.
        session.switched_from = None;
        let sent = if !handoff.is_empty() {
            format!("{handoff}\n\n## Your next instruction\n{prompt}")
        } else if resume && !native {
            let memory = std::path::Path::new(&dir).join(handoff::MEMORY_FILE);
            if memory.is_file() {
                // The memory file holds the history; only the latest exchange is resent.
                let recent = context_prompt_last(
                    &session.messages[session.conversation_start..],
                    &prompt,
                    2,
                );
                let name = agent::provider(&provider).name;
                let from = crate::chatctl::context_from(&session.messages);
                let digest = handoff::digest(&session.messages[from..], name);
                // After a compact, its summary stands in for what came before.
                if from > 0 && !session.carried.is_empty() {
                    format!("{}\n\n{digest}\n{recent}", session.carried)
                } else {
                    format!("{digest}\n{recent}")
                }
            } else {
                let recent =
                    context_prompt(&session.messages[session.conversation_start..], &prompt);
                if session.carried.is_empty() {
                    recent
                } else {
                    format!("{}\n\n{recent}", session.carried)
                }
            }
        } else {
            prompt.clone()
        };
        let sent = match notes {
            Some(notes) => format!("{notes}{sent}"),
            None => sent,
        };
        // Tell the agent its earlier work was undone (or redone) behind its back.
        let undo_note = std::mem::take(&mut session.undo_note);
        let sent = if undo_note.is_empty() {
            sent
        } else {
            format!("{undo_note}\n\n{sent}")
        };
        let sent = match preface {
            Some(preface) => format!("{preface}\n{sent}"),
            None => sent,
        };
        let mut user = Message::now(Role::User, prompt.clone());
        user.images = images.clone();
        user.playbook = playbook.unwrap_or_default();
        if in_git {
            user.checkpoint = git::checkpoint(&dir).unwrap_or_default();
        }
        session.messages.push(user);
        session.messages.push(Message::now(Role::Agent, ""));
        let sel = session.selections();
        let agent_session = session.agent_session.clone();
        let sandbox = session.sandbox();
        // A signed-out agent says so before its turn fails, not after.
        if sandbox.is_none() && crate::signin::get(&provider) == Some(false) {
            let mut hint = Message::new(Role::System, agent::sign_in_help(&provider));
            hint.meta = "warning".into();
            let at = session.messages.len() - 1;
            session.messages.insert(at, hint);
        }
        crate::connections::prepare_run(
            Some((provider.clone(), agent_session.clone())),
            crate::settings::sync_backup(),
        );
        // On a connection the agent reaches Runo's memory server through a relay.
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|_| memory_server() && crate::connections::mcp_reachable(sandbox.as_ref()));
        // Images inside the session folder go by their place in it, which is the same on
        // a connection; others by their full path.
        let attached: Vec<String> = images
            .iter()
            .map(|path| {
                std::path::Path::new(path)
                    .strip_prefix(&dir)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.clone())
            })
            .collect();
        let spec = agent::RunSpec {
            provider: &provider,
            prompt: &sent,
            resume: native,
            sel: &sel,
            opts,
            in_git,
            session_id: &agent_session,
            images: &attached,
            memory: exe.as_deref().map(|exe| (exe, dir.as_str())),
            local: sandbox.is_none(),
            thread: if session.crew.is_some() && sandbox.is_none() {
                id
            } else {
                0
            },
        };
        match agent::run_agent(
            Target::Session(id),
            &spec,
            &dir,
            sandbox.as_ref(),
            self.tx.clone(),
        ) {
            Ok(handle) => {
                session.status = Status::Working;
                session.remote_turn =
                    matches!(sandbox, Some(crate::connections::Sandbox::Remote(_)));
                let turn = session
                    .messages
                    .iter()
                    .rposition(|m| m.role == Role::User)
                    .unwrap_or(0);
                let model = sel.get("model").map(String::as_str).unwrap_or("");
                let mut running =
                    crate::harness::turn::Turn::start(id, &provider, &dir, turn, model);
                if let Some(root) = crate::detach::root() {
                    running.follow(&crate::detach::turn_dir(root, id));
                }
                self.turns.insert(id, running);
                self.handles.insert(Target::Session(id), handle);
                self.turn_files.remove(&id);
                // Counted as it starts, so turns running side by side can't pass the limit.
                self.state.add_turn(&store::today());
                crate::applog::info(
                    "turn",
                    format!(
                        "session #{id} ({provider}): turn started{}{}",
                        match &sandbox {
                            None => "",
                            Some(crate::connections::Sandbox::Remote(_)) => ", on a connection",
                            Some(_) => ", in a sandbox",
                        },
                        if native {
                            ", resuming its conversation"
                        } else {
                            ""
                        }
                    ),
                );
                let first = prompt.lines().next().unwrap_or("").to_string();
                self.state.log(id, format!("Started: {first}"));
            }
            Err(error) => {
                crate::applog::error(
                    "turn",
                    format!("session #{id} ({provider}): could not start the agent: {error}"),
                );
                session.status = Status::Failed;
                session.messages.pop();
                session.messages.push(Message::new(Role::System, error));
            }
        }
        self.save();
        self.chat_scroll.scroll_to_bottom();
        self.keep_awake_update();
    }

    /// The CLI is waiting for a browser sign-in, which never completes without a terminal:
    /// stop the run and say how to sign in.
    pub(crate) fn sign_in_wait(&mut self, id: u64, line: &str) {
        if let Some(turn) = self.turns.get_mut(&id) {
            if std::mem::replace(&mut turn.signed_out, true) {
                return;
            }
        }
        self.stderr.entry(id).or_default().push(line.to_string());
        if let Some(handle) = self.handles.get(&Target::Session(id)) {
            handle.kill();
        }
    }

    /// Copilot without `--session-id` prints no conversation id. After a run, take the
    /// newest saved session written during it, if it ran in this session's folder and
    /// no other session already uses it.
    pub(crate) fn find_copilot_session(&mut self, id: u64, since: std::time::SystemTime) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if session.provider != "copilot" || !session.agent_session.is_empty() {
            return;
        }
        let dir = session.dir.clone();
        let asked = session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.text.clone())
            .unwrap_or_default();
        let taken: std::collections::HashSet<String> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.provider == "copilot" && s.id != id)
            .map(|s| s.agent_session.clone())
            .collect();
        let root = std::env::var_os("COPILOT_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .map(|h| std::path::PathBuf::from(h).join(".copilot"))
            })
            .map(|p| p.join("session-state"));
        if let Some(found) =
            root.and_then(|r| newest_copilot_session(&r, &dir, &asked, since, &taken))
        {
            if let Some(s) = self.state.session_mut(id) {
                s.agent_session = found;
            }
        }
    }

    /// Send `text` to the agent of a running live turn, which reads it as it works. The
    /// message shows in the conversation at once and the reply carries on below it.
    /// Returns false when the turn doesn't take messages, and the prompt should queue.
    pub(crate) fn steer(&mut self, id: u64, text: &str) -> bool {
        if !self.turns.get(&id).is_some_and(|t| t.live && !t.stopping) {
            return false;
        }
        let Some(root) = crate::detach::root() else {
            return false;
        };
        if let Err(e) = crate::harness::host::send(&crate::detach::turn_dir(root, id), text) {
            crate::applog::warn(
                "turn",
                format!("session #{id}: couldn't send a message to the running agent: {e}"),
            );
            return false;
        }
        let Some(session) = self.state.session_mut(id) else {
            return true;
        };
        // Undo still goes back to where the turn started.
        let checkpoint = session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.checkpoint.clone())
            .unwrap_or_default();
        let mut message = Message::now(Role::User, text);
        message.meta = STEER.into();
        message.checkpoint = checkpoint;
        session.messages.push(message);
        crate::applog::info(
            "turn",
            format!("session #{id}: message sent to the running agent"),
        );
        self.save();
        true
    }

    /// The messages a live turn's agent never read, because it finished first: they leave
    /// the conversation and go to the front of the queue, to be sent as the next turn.
    pub(crate) fn requeue_unsent(&mut self, id: u64) {
        let Some(root) = crate::detach::root() else {
            return;
        };
        let unsent = crate::harness::host::unsent(&crate::detach::turn_dir(root, id));
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        take_unsent(session, &unsent);
        if !unsent.is_empty() {
            crate::applog::info(
                "turn",
                format!(
                    "session #{id}: {} message(s) came after the agent finished; queued",
                    unsent.len()
                ),
            );
        }
    }

    /// After a turn: count what it changed, then send the next queued prompt. With `hold`
    /// (the agent isn't signed in) the queue waits for the person.
    pub(crate) fn finish_turn(
        &mut self,
        id: u64,
        ok: bool,
        stopped: bool,
        hold: bool,
        cx: &mut Context<Self>,
    ) {
        self.crew_turn_ended(id, ok, stopped, cx);
        // Servers Runo merged into the agent's project file go before the snapshot.
        if let Some(s) = self.state.session(id) {
            crate::mcp::clean_project(std::path::Path::new(&s.dir));
        }
        // A pull request's state, and a review the agent wrote for it, show right away.
        if self.state.session(id).is_some_and(|s| !s.pr_url.is_empty()) {
            self.load_pr_status(id);
        }
        let checkpoint = self.state.session(id).and_then(|s| {
            s.messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| (m.checkpoint.clone(), s.dir.clone()))
        });
        // How many files the turn changed, and a snapshot of where it ended for Undo.
        // A turn that finished with changed files gets its checks run (Proof of done).
        let prove = ok
            && !stopped
            && self
                .state
                .session(id)
                .is_some_and(|s| s.canvas.is_empty() && !s.cli);
        // The reply the files are recorded on, found now: a queued prompt may add more
        // messages before the comparison is done.
        let reply = self.state.session(id).and_then(|s| {
            let user = s.messages.iter().rposition(|m| m.role == Role::User)?;
            s.messages[user..]
                .iter()
                .rposition(|m| m.role == Role::Agent)
                .map(|i| user + i)
        });
        if let Some((cp, dir)) = checkpoint.filter(|(cp, _)| !cp.is_empty()) {
            self.bg(
                move || (git::changed_since(&dir, &cp), git::checkpoint(&dir)),
                move |this, (changed, end), cx| {
                    let files = changed.len();
                    this.turn_files.insert(id, files);
                    if let Some(s) = this.state.session_mut(id) {
                        if let Some(m) = reply.and_then(|i| s.messages.get_mut(i)) {
                            m.changed = changed;
                        }
                        if let Ok(end) = end {
                            s.snapshot = end;
                        }
                    }
                    this.save();
                    if prove && files > 0 {
                        this.prove_turn(id, true, cx);
                    }
                },
            );
        }
        let title = self
            .state
            .session(id)
            .map(|s| s.title.clone())
            .unwrap_or_default();
        self.state.log(
            id,
            match (ok, stopped) {
                (true, _) => format!("Finished: {title}"),
                (false, true) => format!("Stopped: {title}"),
                (false, false) => format!("Failed: {title}"),
            },
        );
        self.keep_awake_update();
        // Queued prompts go out in order whether the turn finished, failed or was stopped;
        // they wait only when the agent can't run at all.
        if !hold {
            self.send_queued(id, 0, cx);
        }
    }

    /// Send queued prompt `index` of a session now. While the agent works it is stopped
    /// first, and the prompt goes out once it has exited.
    pub(crate) fn send_queued(&mut self, id: u64, index: usize, cx: &mut Context<Self>) {
        if self.handles.contains_key(&Target::Session(id)) {
            if let Some(s) = self.state.session_mut(id) {
                if index < s.queue.len() {
                    let item = s.queue.remove(index);
                    s.queue.insert(0, item);
                }
            }
            self.save();
            self.stop_session(id);
            return;
        }
        let next = self
            .state
            .session_mut(id)
            .filter(|s| index < s.queue.len())
            .map(|s| (s.queue.remove(index), s.provider.clone()));
        if let Some((item, provider)) = next {
            let opts = cx.global::<AppSettings>().agent(&provider);
            self.start_turn(id, item.text, item.images, &opts);
        }
    }

    /// Send the last prompt again, with its images, after a failed or stopped turn.
    pub(crate) fn retry_last(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.handles.contains_key(&Target::Session(id)) {
            return;
        }
        let last = self.state.session(id).and_then(|s| {
            s.messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| (m.text.clone(), m.images.clone(), s.provider.clone()))
        });
        if let Some((text, images, provider)) = last {
            let opts = cx.global::<AppSettings>().agent(&provider);
            self.start_turn(id, text, images, &opts);
        }
    }

    /// Prompts that were still queued when Runo last closed go out now, in order.
    pub(crate) fn send_waiting_queues(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| !s.queue.is_empty() && !s.cli && !s.archived)
            .filter(|s| !self.handles.contains_key(&Target::Session(s.id)))
            .map(|s| s.id)
            .collect();
        for id in ids {
            self.send_queued(id, 0, cx);
        }
    }

    /// Images attached to the selected session's unsent prompt.
    pub(crate) fn attached(&self) -> &[String] {
        self.selected
            .and_then(|id| self.attachments.get(&id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Send the composer's prompt, or queue it while the agent is busy.
    pub(crate) fn submit_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let typed = self.prompt.read(cx).value().trim().to_string();
        let playbook = self
            .picked_playbook
            .and_then(|pb| self.state.automations.iter().find(|a| a.id == pb))
            .map(|a| {
                let prompt = crate::pbfile::fill(&a.prompt, &a.run_inputs(), &self.picked_values);
                (a.name.clone(), prompt)
            });
        if typed.is_empty() && self.attached().is_empty() && playbook.is_none() {
            return;
        }
        // A playbook goes out as its full instructions, with anything typed added after.
        let text = match &playbook {
            Some((_, instructions)) if typed.is_empty() => instructions.clone(),
            Some((_, instructions)) => format!("{instructions}\n\nAlso: {typed}"),
            None => typed.clone(),
        };
        self.picked_values.clear();
        if let Some(pb) = self.picked_playbook.take() {
            if let Some(a) = self.state.automations.iter_mut().find(|a| a.id == pb) {
                a.last_run = workspace::unix_now();
                a.ran = a.last_run;
            }
        }
        self.prompt.update(cx, |p, cx| p.set_value("", window, cx));
        self.drafts.remove(&id);
        self.mention = None;
        self.thread_mention = None;
        let images = self.attachments.remove(&id).unwrap_or_default();
        let text = if text.is_empty() {
            "Look at the attached image.".to_string()
        } else {
            text
        };
        // In a Crew chat an @thread or the New thread chip skips Crew.
        if playbook.is_none() && self.crew_route_submit(id, &text, &images, cx) {
            self.editing = None;
            return;
        }
        // While the agent or its setup command runs, the prompt and its images wait,
        // unless the agent takes messages as it works.
        if self.handles.contains_key(&Target::Session(id)) || self.pending_start.contains_key(&id) {
            self.editing = None;
            if images.is_empty() && playbook.is_none() && self.steer(id, &text) {
                return;
            }
            if let Some(session) = self.state.session_mut(id) {
                session.queue.push(Queued::new(text, images));
            }
            self.save();
            self.info = Some((
                "Queued. It is sent when the agent finishes.".into(),
                std::time::Instant::now(),
            ));
            return;
        }
        let opts = self
            .current()
            .map(|s| cx.global::<AppSettings>().agent(&s.provider))
            .unwrap_or_default();
        // An edited message replaces the one it came from.
        if let Some((_, index)) = self.editing.filter(|(e, _)| *e == id) {
            self.editing = None;
            self.resend_from(id, index, text, images, cx);
            return;
        }
        if playbook.is_none() && images.is_empty() && self.run_chat_command(id, &text, &opts) {
            return;
        }
        // `codex exec` takes prompts as written, so its saved prompts are expanded here.
        let codex_prompt = self
            .current()
            .filter(|s| s.provider == "codex" && playbook.is_none())
            .and_then(|_| agentcmds::expand_codex(&text, agentcmds::home().as_deref()))
            .map(|expanded| (expanded, text.clone()));
        let text = codex_prompt
            .as_ref()
            .map(|(expanded, _)| expanded.clone())
            .unwrap_or(text);
        let before = self.current().map_or(0, |s| s.messages.len());
        self.start_turn(id, text, images, &opts);
        // Only when the turn really started; an open CLI queues it instead.
        let started = self.current().map_or(0, |s| s.messages.len()) > before;
        if let (Some((name, _)), true) = (playbook, started) {
            self.tag_playbook(id, &name, &typed);
        }
        if let (Some((_, typed)), true) = (codex_prompt, started) {
            let (name, note) = typed.trim().split_once(' ').unwrap_or((typed.trim(), ""));
            self.tag_playbook(id, name, note.trim());
        }
    }

    /// Mark the session's latest message as a playbook run, so it shows as the playbook's
    /// name rather than its whole prompt.
    pub(crate) fn tag_playbook(&mut self, id: u64, name: &str, note: &str) {
        if let Some(turn) = self.preparing.get_mut(&id).and_then(|w| w.last_mut()) {
            turn.tag = Some((name.to_string(), note.to_string()));
            return;
        }
        if let Some(m) = self
            .state
            .session_mut(id)
            .and_then(|s| s.messages.iter_mut().rev().find(|m| m.role == Role::User))
        {
            m.playbook = name.to_string();
            m.note = note.to_string();
        }
        self.save();
    }

    /// Put the worktree back as it was before user message `index` ran: files, their
    /// staging, and the branch if the agent committed. A snapshot taken first makes it
    /// redoable, and files changed after the agent's last turn are left alone.
    pub(crate) fn undo_to(&mut self, id: u64, index: usize) {
        if self.handles.contains_key(&Target::Session(id)) {
            self.fail("turn", "Stop the agent before undoing its changes.".into());
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let Some(cp) = session
            .messages
            .get(index)
            .map(|m| m.checkpoint.clone())
            .filter(|c| !c.is_empty())
        else {
            return;
        };
        let dir = session.dir.clone();
        let guard = Some(session.snapshot.clone()).filter(|s| !s.is_empty());
        if self.git_busy.contains_key(&id) {
            return;
        }
        self.git_busy.insert(id, "Undoing.".into());
        self.bg(
            move || {
                let redo = git::checkpoint(&dir)?;
                let done = git::restore(&dir, &cp, guard.as_deref())?;
                let after = git::checkpoint(&dir).unwrap_or_default();
                Ok::<_, String>((redo, done, after))
            },
            move |this, result, _| {
                this.git_busy.remove(&id);
                match result {
                    Ok((redo, done, after)) => this.undone(id, index, redo, done, after),
                    Err(error) => this.fail("turn", format!("Could not undo: {error}")),
                }
            },
        );
    }

    fn undone(&mut self, id: u64, index: usize, redo: String, done: git::Restored, after: String) {
        let n = done.files.len();
        if let Some(session) = self.state.session_mut(id) {
            let mut first = true;
            for m in session.messages.iter_mut().skip(index) {
                if m.role == Role::Agent && m.resolved.is_empty() {
                    m.resolved = "undone".into();
                    // Redo lives on the first undone reply.
                    if first {
                        m.redo = redo.clone();
                        first = false;
                    }
                }
            }
            session.snapshot = after;
            session.undo_note = undo_note(&done, false);
        }
        self.turn_files.insert(id, 0);
        self.state.log(id, format!("Undid changes to {n} files"));
        let mut text = format!("Restored {n} file{}", if n == 1 { "" } else { "s" });
        if done.moved_head {
            text.push_str(" and moved the branch back past the agent's commits");
        }
        if !done.skipped.is_empty() {
            text.push_str(&format!(
                ". Left {} you changed since alone: {}",
                done.skipped.len(),
                done.skipped.join(", ")
            ));
        }
        self.info = Some((format!("{text}."), std::time::Instant::now()));
        self.save();
        self.measure(id);
        self.refresh();
    }

    /// Bring back what an Undo removed, from the snapshot it took.
    pub(crate) fn redo_turn(&mut self, id: u64, index: usize) {
        if self.handles.contains_key(&Target::Session(id)) || self.git_busy.contains_key(&id) {
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let Some(redo) = session
            .messages
            .get(index)
            .map(|m| m.redo.clone())
            .filter(|r| !r.is_empty())
        else {
            return;
        };
        let dir = session.dir.clone();
        let guard = Some(session.snapshot.clone()).filter(|s| !s.is_empty());
        self.git_busy.insert(id, "Redoing.".into());
        self.bg(
            move || {
                let done = git::restore(&dir, &redo, guard.as_deref())?;
                let after = git::checkpoint(&dir).unwrap_or_default();
                Ok::<_, String>((done, after))
            },
            move |this, result, _| {
                this.git_busy.remove(&id);
                let (done, after) = match result {
                    Ok(r) => r,
                    Err(error) => {
                        this.fail("turn", format!("Could not redo: {error}"));
                        return;
                    }
                };
                if let Some(session) = this.state.session_mut(id) {
                    for m in session.messages.iter_mut().skip(index) {
                        if m.role == Role::Agent && m.resolved == "undone" {
                            m.resolved.clear();
                            m.redo.clear();
                        }
                    }
                    session.snapshot = after;
                    session.undo_note = undo_note(&done, true);
                }
                let n = done.files.len();
                this.turn_files.remove(&id);
                this.state.log(id, format!("Redid changes to {n} files"));
                this.info = Some((
                    format!("Put back {n} file{}.", if n == 1 { "" } else { "s" }),
                    std::time::Instant::now(),
                ));
                this.save();
                this.measure(id);
                this.refresh();
            },
        );
    }

    pub(crate) fn keep_turn(&mut self, id: u64, index: usize) {
        if let Some(m) = self
            .state
            .session_mut(id)
            .and_then(|s| s.messages.get_mut(index))
        {
            m.resolved = "kept".into();
        }
        self.save();
    }

    /// Answer the agent: a follow-up prompt, optionally with tools allowed for this turn.
    pub(crate) fn answer(&mut self, prompt: String, allow: Vec<String>, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        if self.handles.contains_key(&Target::Session(id)) {
            return;
        }
        let Some(provider) = self.current().map(|s| s.provider.clone()) else {
            return;
        };
        let mut opts = cx.global::<AppSettings>().agent(&provider);
        if !allow.is_empty() {
            let mut tools: Vec<String> = opts
                .allowed_tools
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            tools.extend(allow);
            opts.allowed_tools = tools.join(",");
        }
        self.start_turn(id, prompt, Vec::new(), &opts);
    }

    /// Always allow these tools for this agent (Settings > agent > Allowed tools).
    pub(crate) fn allow_always(&mut self, tools: &[String], cx: &mut Context<Self>) {
        let Some(provider) = self.current().map(|s| s.provider.clone()) else {
            return;
        };
        self.allow_tools_for(&provider, tools, cx);
    }

    pub(crate) fn allow_tools_for(
        &mut self,
        provider: &str,
        tools: &[String],
        cx: &mut Context<Self>,
    ) {
        let provider = provider.to_string();
        let mut st = cx.global::<AppSettings>().clone();
        let entry = st.agents.entry(provider).or_default();
        let mut list: Vec<String> = entry
            .allowed_tools
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        for tool in tools {
            if !list.contains(tool) {
                list.push(tool.clone());
            }
        }
        entry.allowed_tools = list.join(",");
        st.save();
        cx.set_global(st);
    }

    /// Attach an image from the clipboard, or pick files when the clipboard has none.
    pub(crate) fn attach(&mut self, cx: &mut Context<Self>) {
        let Some((id, dir)) = self.current().map(|s| (s.id, s.dir.clone())) else {
            return;
        };
        if let Some(item) = cx.read_from_clipboard() {
            for entry in item.entries() {
                match entry {
                    ClipboardEntry::Image(image) => {
                        if let Some(path) = save_image(&dir, &image.bytes, image.format) {
                            self.attachments.entry(id).or_default().push(path);
                            return;
                        }
                    }
                    ClipboardEntry::ExternalPaths(paths) => {
                        let before = self.attached().len();
                        self.attach_paths(paths.paths());
                        if self.attached().len() > before {
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |this, cx| {
                    this.attach_paths(&paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Images become attachments, copied into the session folder so the agent can read
    /// them wherever it runs; other files are mentioned in the prompt as @path.
    pub(crate) fn attach_paths(&mut self, paths: &[std::path::PathBuf]) {
        let Some((id, dir, provider)) = self
            .current()
            .map(|s| (s.id, s.dir.clone(), s.provider.clone()))
        else {
            return;
        };
        for path in paths {
            if is_image(path) {
                if !agent::takes_images(&provider) {
                    self.fail(
                        "turn",
                        format!(
                            "{} can't read images, so the image wasn't attached.",
                            agent::provider(&provider).name
                        ),
                    );
                    continue;
                }
                let Some(p) = copy_image(&dir, path) else {
                    self.fail("turn", format!("Could not attach {}.", path.display()));
                    continue;
                };
                let list = self.attachments.entry(id).or_default();
                if !list.contains(&p) {
                    list.push(p);
                }
            } else {
                let rel = path
                    .strip_prefix(&dir)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string_lossy().into_owned());
                self.pending_mentions.push(rel);
            }
        }
    }

    /// The `@query` being typed at the end of the prompt, if any.
    pub(crate) fn update_mention(&mut self, cx: &mut Context<Self>) {
        let text = self.prompt.read(cx).value().to_string();
        self.mention = mention_query(&text);
        self.thread_mention = self
            .current()
            .filter(|s| s.canvas == crew::CANVAS)
            .and_then(|_| text.trim_start().strip_prefix('@'))
            .map(str::to_lowercase);
        self.slash = slash_query(&text);
        if self.slash.is_some() {
            self.load_agent_commands();
        } else {
            // Read again next time, so new command files show up.
            self.agent_commands = None;
        }
    }

    /// Playbooks and the agent's own commands matching the `/query`, best first. A command
    /// typed out in full goes first; otherwise playbooks lead.
    pub(crate) fn slash_matches(&self) -> Vec<SlashPick> {
        let Some(query) = &self.slash else {
            return Vec::new();
        };
        let q = query.to_lowercase();
        let here = self.current().map(|s| s.project_id);
        let mut hits: Vec<(usize, &store::Automation)> = self
            .state
            .automations
            .iter()
            .filter(|a| a.project_id == 0 || Some(a.project_id) == here)
            .filter_map(|a| {
                let slug = a.slug();
                if slug.starts_with(&q) {
                    Some((0, a))
                } else if slug.contains(&q) || a.name.to_lowercase().contains(&q) {
                    Some((1, a))
                } else {
                    None
                }
            })
            .collect();
        hits.sort_by_key(|(rank, a)| (*rank, a.name.len()));
        let mut out: Vec<SlashPick> = hits
            .into_iter()
            .take(8)
            .map(|(_, a)| SlashPick::Playbook(a.id))
            .collect();
        let commands = self
            .agent_commands
            .as_ref()
            .filter(|(sid, _)| Some(*sid) == self.selected)
            .map(|(_, c)| agentcmds::matching(c, &q))
            .unwrap_or_default();
        for c in commands.into_iter().take(12) {
            let pick = SlashPick::Command(c.clone());
            if c.name.to_lowercase() == q {
                out.insert(0, pick);
            } else {
                out.push(pick);
            }
        }
        out
    }

    /// Read the agent's commands for the selected session when the `/` menu opens.
    fn load_agent_commands(&mut self) {
        let Some((id, provider, dir)) = self
            .current()
            .map(|s| (s.id, s.provider.clone(), s.dir.clone()))
        else {
            return;
        };
        if self
            .agent_commands
            .as_ref()
            .is_some_and(|(sid, _)| *sid == id)
        {
            return;
        }
        let home = agentcmds::home();
        let found = agentcmds::discover(&provider, std::path::Path::new(&dir), home.as_deref());
        self.agent_commands = Some((id, found));
    }

    /// Use an entry of the `/` menu. A playbook goes in the box for this session, or starts
    /// as its own session in this project; an agent command is typed into the box.
    pub(crate) fn pick_slash(
        &mut self,
        pick: SlashPick,
        new_session: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.slash = None;
        let playbook = match pick {
            SlashPick::Playbook(id) => id,
            SlashPick::Command(command) => {
                self.prompt.update(cx, |p, cx| {
                    p.set_value(format!("/{} ", command.name), window, cx);
                    p.focus(window, cx);
                });
                return;
            }
        };
        let Some(asks) = self
            .state
            .automations
            .iter()
            .find(|a| a.id == playbook)
            .map(|a| !a.run_inputs().is_empty())
        else {
            return;
        };
        if new_session {
            self.prompt.update(cx, |p, cx| p.set_value("", window, cx));
            let here = self.current().map(|s| s.project_id);
            self.run_playbook(playbook, here, cx);
            return;
        }
        // The playbook rides along as a chip; its instructions go out when the message is
        // sent, so the box stays free for anything to add. Inputs are asked for first.
        self.picked_values.clear();
        if asks {
            self.prompt.update(cx, |p, cx| p.set_value("", window, cx));
            self.ask_playbook_inputs(playbook, None, true);
            return;
        }
        self.picked_playbook = Some(playbook);
        self.prompt.update(cx, |p, cx| {
            p.set_value("", window, cx);
            p.focus(window, cx);
        });
    }

    /// Threads (in Crew's chat) and files matching the `@query`, best first.
    pub(crate) fn mention_matches(&self) -> Vec<MentionPick> {
        let mut out: Vec<MentionPick> = Vec::new();
        if let (Some(q), Some(pid)) = (&self.thread_mention, self.current().map(|s| s.project_id)) {
            out.extend(
                self.crew_threads(pid, false)
                    .iter()
                    .filter(|t| {
                        t.title.to_lowercase().contains(q.as_str())
                            || format!("#{}", t.id).starts_with(q.as_str())
                    })
                    .take(5)
                    .map(|t| MentionPick::Thread(t.id)),
            );
            // Past the first word it is a thread's name, not a file's.
            if q.contains(char::is_whitespace) {
                return out;
            }
        }
        let Some(query) = &self.mention else {
            return out;
        };
        let q = query.to_lowercase();
        let mut hits: Vec<(usize, &String)> = self
            .files
            .iter()
            .filter_map(|f| {
                let lower = f.to_lowercase();
                let name = lower.rsplit('/').next().unwrap_or(&lower);
                if name.starts_with(&q) {
                    Some((0, f))
                } else if name.contains(&q) {
                    Some((1, f))
                } else if lower.contains(&q) {
                    Some((2, f))
                } else {
                    None
                }
            })
            .collect();
        hits.sort_by_key(|(rank, f)| (*rank, f.len()));
        out.extend(
            hits.into_iter()
                .take(8)
                .map(|(_, f)| MentionPick::File(f.clone())),
        );
        out
    }

    pub(crate) fn pick_mention(
        &mut self,
        pick: &MentionPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.prompt.read(cx).value().to_string();
        let new = match pick {
            // A thread's name only routes at the start of the message.
            MentionPick::Thread(id) => match self.state.session(*id) {
                Some(t) => format!("@{} ", t.title),
                None => return,
            },
            MentionPick::File(path) => complete_mention(&text, path),
        };
        let end = new.encode_utf16().count() as u32;
        self.prompt.update(cx, |p, cx| {
            p.set_value(new, window, cx);
            p.set_cursor_position(
                input::Position {
                    line: 0,
                    character: end,
                },
                window,
                cx,
            );
            p.focus(window, cx);
        });
        self.mention = None;
        self.thread_mention = None;
    }

    /// Write a commit message for the current changes with the session's agent.
    pub(crate) fn suggest_commit(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.current() else {
            return;
        };
        let patch = git::patch_for_message(&session.dir, 24_000);
        if patch.trim().is_empty() {
            self.fail("turn", "There are no changes to describe.".into());
            return;
        }
        let prompt = format!(
            "Write a git commit message for the changes below. Reply with only the message: \
             a summary line under 72 characters in the imperative mood, then optionally a \
             blank line and a short body. No code fences, no quotes, no preamble.\n\n{patch}"
        );
        let configured = cx.global::<AppSettings>().agent(&session.provider).binary;
        let command = agent::ask_command(&session.provider, &configured, &prompt);
        agent::oneshot(
            format!("commit-msg:{}", session.id),
            command,
            &session.dir,
            self.tx.clone(),
        );
        self.writing_commit = true;
    }

    /// Results of one-off background commands.
    pub(crate) fn handle_output(&mut self, key: String, result: Result<String, String>) {
        if key.starts_with("guide:") || key.starts_with("split:") {
            self.handle_reviewable(&key, result);
            return;
        }
        if key.starts_with("resolve:") {
            self.handle_resolve(&key, result);
            return;
        }
        if key.starts_with("explain:") {
            self.handle_explain(&key, result);
            return;
        }
        if key.starts_with("assistant:") {
            return self.assistant_reply(key, result);
        }
        if key.starts_with("commit-msg:") {
            self.writing_commit = false;
            match result {
                Ok(text) => {
                    // The commit box is one line: keep the summary.
                    let text = clean_commit_message(&text)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if text.is_empty() {
                        self.fail("turn", "The agent did not write a commit message.".into());
                    } else {
                        self.commit_suggestion = Some(text);
                    }
                }
                Err(error) => {
                    let first = error.lines().next().unwrap_or("").to_string();
                    self.fail("turn", format!("Could not write a commit message: {first}"));
                }
            }
        } else if self.handle_review(&key, result.clone())
            || self.handle_recap(&key, result.clone())
            || self.handle_alert_job(&key, result.clone())
        {
        } else if !self.handle_browser_job(&key, result.clone()) {
            self.handle_job(key, result);
        }
    }

    /// Apply values produced outside of render (they need a window to reach inputs).
    pub(crate) fn apply_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_folder_pending(window, cx);
        // Each session keeps its own unsent prompt while another one is on screen.
        if self.draft_for != self.selected {
            let text = self.prompt.read(cx).value().to_string();
            if let Some(old) = self.draft_for {
                if text.trim().is_empty() {
                    self.drafts.remove(&old);
                } else {
                    self.drafts.insert(old, text.clone());
                }
            }
            let draft = self
                .selected
                .and_then(|id| self.drafts.remove(&id))
                .unwrap_or_default();
            if draft != text {
                self.prompt
                    .update(cx, |p, cx| p.set_value(draft, window, cx));
            }
            self.draft_for = self.selected;
            self.mention = None;
            self.thread_mention = None;
            self.slash = None;
        }
        if let Some(message) = self.commit_suggestion.take() {
            self.commit_msg
                .update(cx, |input, cx| input.set_value(message, window, cx));
        }
        if !self.pending_mentions.is_empty() {
            let add: Vec<String> = std::mem::take(&mut self.pending_mentions)
                .into_iter()
                .map(|p| format!("@{p}"))
                .collect();
            let text = self.prompt.read(cx).value().to_string();
            let sep = if text.is_empty() || text.ends_with(char::is_whitespace) {
                ""
            } else {
                " "
            };
            let new = format!("{text}{sep}{} ", add.join(" "));
            self.prompt.update(cx, |p, cx| p.set_value(new, window, cx));
        }
    }

    /// Prevent system sleep while any agent runs (macOS `caffeinate`, Linux `systemd-inhibit`).
    pub(crate) fn keep_awake_update(&mut self) {
        // A turn on a connection doesn't need this machine awake.
        let want = cx_keep_awake()
            && self
                .turns
                .keys()
                .any(|id| self.state.session(*id).is_some_and(|s| !s.remote_turn));
        match (&mut self.awake, want) {
            (None, true) => {
                let pid = std::process::id().to_string();
                let child = if cfg!(target_os = "macos") {
                    std::process::Command::new("caffeinate")
                        .args(["-i", "-w", &pid])
                        .spawn()
                } else if cfg!(target_os = "linux") {
                    std::process::Command::new("systemd-inhibit")
                        .args([
                            "--what=idle:sleep",
                            "--who=Runo",
                            "--why=Agents are running",
                            "sleep",
                            "infinity",
                        ])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                } else {
                    return;
                };
                self.awake = child.ok();
            }
            (Some(child), false) => {
                let _ = child.kill();
                let _ = child.wait();
                self.awake = None;
            }
            _ => {}
        }
    }

    // ---------- UI ----------

    /// Undo / Keep / Review under the latest finished turn that changed files.
    pub(crate) fn turn_bar(&self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.current()?;
        let message = session.messages.get(index)?;
        if message.resolved == "undone" && !message.redo.is_empty() {
            let id = session.id;
            return Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child("This turn's changes were undone.")
                    .child(div().flex_1())
                    .child(
                        self.button(("redo-turn", index), "Redo", false)
                            .h(px(24.))
                            .px_2()
                            .text_size(px(12.))
                            .child(icon(IconName::Redo2, 12., c_secondary()))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Put the files and commits back as they were before the undo",
                                )
                                .build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.redo_turn(id, index);
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        if !message.resolved.is_empty() {
            return None;
        }
        let user = session.messages[..index]
            .iter()
            .rposition(|m| m.role == Role::User)?;
        if session.messages[user].checkpoint.is_empty() {
            return None;
        }
        // The count is only known for a turn run since launch. Without it the turn may
        // have changed nothing, and Undo would roll back work done since, so say nothing.
        let n = self
            .turn_files
            .get(&session.id)
            .copied()
            .filter(|n| *n > 0)?;
        let id = session.id;
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(c_secondary()))
                .child(format!(
                    "This turn changed {n} file{}.",
                    if n == 1 { "" } else { "s" }
                ))
                .child(div().flex_1())
                .child(
                    self.button(("undo-turn", index), "Undo", false)
                        .h(px(24.))
                        .px_2()
                        .text_size(px(12.))
                        .child(icon(IconName::Undo2, 12., c_secondary()))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Put every file back as it was before this turn",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.undo_to(id, user);
                            cx.notify();
                        })),
                )
                .child(
                    self.button(("review-turn", index), "Review", false)
                        .child(icon(IconName::GitCompare, 12., c_secondary()))
                        .h(px(24.))
                        .px_2()
                        .text_size(px(12.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.right_tab = RightTab::Changes;
                            this.show_panel = true;
                            cx.notify();
                        })),
                )
                .child(
                    self.button(("keep-turn", index), "Keep", true)
                        .child(icon(IconName::Check, 12., c_canvas()))
                        .h(px(24.))
                        .px_2()
                        .text_size(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.keep_turn(id, index);
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    /// Approve the plan and let the agent build it. A playbook that declared it only reads
    /// asks first, unless `widen` says the person already agreed to let it edit.
    pub(crate) fn approve_plan(&mut self, widen: bool, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let Some(s) = self.state.session_mut(id) else {
            return;
        };
        if !s.limits.is_empty() && !s.limits.iter().any(|p| p == "edit") {
            if !widen {
                self.pb_widen = Some(id);
                return;
            }
            s.limits.push("edit".into());
            let name = s.title.clone();
            self.state.log(
                id,
                format!("Allowed edits beyond the playbook's limits: {name}"),
            );
        }
        self.pb_widen = None;
        if let Some(s) = self.state.session_mut(id) {
            s.opts.insert("mode".into(), "edit".into());
        }
        self.answer(
            "The plan is approved. Go ahead and implement it.".into(),
            Vec::new(),
            cx,
        );
    }

    /// The plan the agent proposed, with Approve and Keep planning.
    pub(crate) fn plan_card(
        &self,
        index: usize,
        plan: &str,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self
            .current()
            .is_some_and(|s| self.handles.contains_key(&Target::Session(s.id)));
        let widen = self.pb_widen.is_some() && self.pb_widen == self.selected;
        div()
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .overflow_hidden()
            .child(
                div()
                    .h(px(32.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(tint(c_subtle()))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(icon(IconName::ListChecks, 13., c_secondary()))
                    .child("Plan"),
            )
            .child(
                div().px_4().py_3().text_size(px(13.)).child(
                    TextView::markdown(
                        SharedString::from(format!("plan-{}-{index}", self.selected.unwrap_or(0))),
                        plan.to_string(),
                    )
                    .selectable(true),
                ),
            )
            .when(last && !busy && widen, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(divider())
                        .bg(rgb(settings::mix(c_amber(), c_canvas(), 0.08)))
                        .child(icon(IconName::ShieldAlert, 13., c_amber()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.5))
                                .child("This session's playbook said it only reads files. Let it edit files in this session?"),
                        )
                        .child(
                            self.button(("plan-widen-no", index), "Keep it read-only", false)
                                .h(px(26.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pb_widen = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button(("plan-widen-yes", index), "Allow edits", true)
                                .h(px(26.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.approve_plan(true, cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(last && !busy && !widen, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .border_t_1()
                        .border_color(divider())
                        .child(
                            self.button(("plan-more", index), "Keep planning", false)
                                .child(icon(IconName::Pencil, 12., c_secondary()))
                                .h(px(26.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.prompt.update(cx, |p, cx| p.focus(window, cx));
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button(("plan-go", index), "Approve and build", true)
                                .child(icon(IconName::Play, 12., c_canvas()))
                                .h(px(26.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.approve_plan(false, cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    /// The first of the agent's questions still without an answer, and its position.
    pub(crate) fn next_question(
        &self,
        id: u64,
        index: usize,
        questions: &[store::Question],
    ) -> (usize, store::Question) {
        let picks = self.question_picks.get(&(id, index));
        let qi = (0..questions.len())
            .find(|qi| {
                !questions[*qi].options.is_empty()
                    && picks.and_then(|p| p.get(*qi)).is_none_or(Option::is_none)
            })
            .unwrap_or(0);
        (
            qi,
            questions.get(qi).cloned().unwrap_or(store::Question {
                question: String::new(),
                options: Vec::new(),
            }),
        )
    }

    /// Answer question `qi` of the agent's message `index`. Once every question has an
    /// answer they go back to the agent together.
    pub(crate) fn pick_answer(
        &mut self,
        id: u64,
        index: usize,
        qi: usize,
        option: String,
        cx: &mut Context<Self>,
    ) {
        let Some(questions) = self
            .state
            .session(id)
            .and_then(|s| s.messages.get(index))
            .map(|m| m.questions.clone())
        else {
            return;
        };
        let picks = self
            .question_picks
            .entry((id, index))
            .or_insert_with(|| vec![None; questions.len()]);
        if let Some(pick) = picks.get_mut(qi) {
            *pick = Some(option);
        }
        // Questions without options can't be picked; they are left out of the answer.
        let done = questions
            .iter()
            .zip(picks.iter())
            .all(|(q, p)| q.options.is_empty() || p.is_some());
        if done {
            let picks = self.question_picks.remove(&(id, index)).unwrap_or_default();
            self.answer_session(id, answers_prompt(&questions, &picks), Vec::new(), cx);
        }
    }

    /// The agent's multiple-choice questions. Each pick is kept until every question has
    /// one; then the answers are sent together.
    pub(crate) fn question_card(
        &self,
        index: usize,
        questions: &[store::Question],
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = self.selected.unwrap_or(0);
        let picks = self.question_picks.get(&(id, index));
        let mut card = div()
            .p_3()
            .v_flex()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .bg(rgb(c_raised()));
        for (qi, q) in questions.iter().enumerate() {
            let picked = picks.and_then(|p| p.get(qi)).cloned().flatten();
            let mut options = div().flex().flex_wrap().gap_2();
            for (oi, option) in q.options.iter().enumerate() {
                let chosen = picked.as_deref() == Some(option.as_str());
                let answer = option.clone();
                options = options.child(
                    self.chip(
                        ("answer", index * 100 + qi * 10 + oi),
                        option.clone(),
                        chosen,
                    )
                    .when(!last, |el| el.opacity(0.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if last {
                            this.pick_answer(id, index, qi, answer.clone(), cx);
                        }
                        cx.notify();
                    })),
                );
            }
            card = card.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(icon(IconName::CircleQuestionMark, 13., c_blue()))
                            .child(q.question.clone()),
                    )
                    .child(options),
            );
        }
        if last && questions.len() > 1 {
            card = card.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child("Pick an answer for each question; they're sent together."),
            );
        }
        card.into_any_element()
    }

    /// Tools the agent was not allowed to use, with Allow once / Always allow.
    pub(crate) fn denied_card(
        &self,
        index: usize,
        tools: &[String],
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let list = tools.join(", ");
        let (once, always) = (tools.to_vec(), tools.to_vec());
        div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(settings::mix(c_amber(), c_canvas(), 0.4)))
            .bg(rgb(settings::mix(c_amber(), c_canvas(), 0.08)))
            .text_size(px(12.5))
            .child(icon(IconName::ShieldAlert, 14., c_amber()))
            .child(div().flex_1().min_w_0().child(format!(
                "The agent asked to use {list} and was not allowed."
            )))
            .when(last, |el| {
                el.child(
                    self.button(("allow-once", index), "Allow once", false)
                        .child(icon(IconName::Check, 12., c_secondary()))
                        .h(px(24.))
                        .px_2()
                        .text_size(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.answer(
                                format!(
                                    "You now have permission to use {}. Continue.",
                                    once.join(", ")
                                ),
                                once.clone(),
                                cx,
                            );
                            cx.notify();
                        })),
                )
                .child(
                    self.button(("allow-always", index), "Always allow", true)
                        .child(icon(IconName::ShieldCheck, 12., c_canvas()))
                        .h(px(24.))
                        .px_2()
                        .text_size(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.allow_always(&always, cx);
                            this.answer(
                                format!(
                                    "You now have permission to use {}. Continue.",
                                    always.join(", ")
                                ),
                                Vec::new(),
                                cx,
                            );
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }

    /// Queued prompts, images and @mention suggestions shown above the composer.
    pub(crate) fn composer_extras(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.current()?;
        let id = session.id;
        let matches = self.mention_matches();
        let playbooks = self.slash_matches();
        let attached = self.attached();
        let working = self.handles.contains_key(&Target::Session(id));
        // A turn that failed or was stopped can be sent again as it was.
        let retry = !working
            && matches!(session.status, Status::Failed | Status::Stopped)
            && session
                .messages
                .last()
                .is_some_and(|m| m.role == Role::System)
            && session.messages.iter().any(|m| m.role == Role::User);
        let edit_bar = self.edit_bar(cx);
        let retiring = if working {
            None
        } else {
            self.retiring_bar(id, cx)
        };
        if session.queue.is_empty()
            && attached.is_empty()
            && matches.is_empty()
            && playbooks.is_empty()
            && !retry
            && edit_bar.is_none()
            && retiring.is_none()
        {
            return None;
        }
        let mut col = div().v_flex().gap_1().children(edit_bar).children(retiring);
        if retry {
            let fixes = self.turn_fixes(id, cx);
            let bar = div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .bg(tint(c_subtle()))
                .text_size(px(12.5))
                .child(icon(IconName::TriangleAlert, 12., c_amber()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(c_secondary()))
                        .child("The last turn didn't finish."),
                )
                .child(
                    self.button("retry-turn", "Retry", false)
                        .h(px(22.))
                        .px_2()
                        .text_size(px(12.))
                        .child(icon(IconName::RefreshCw, 11., c_secondary()))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Send the last message again, with its images",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.retry_last(id, cx);
                            cx.notify();
                        })),
                );
            // What would fix the agent goes under it, one click each.
            col = col.child(match fixes {
                Some(fixes) => div()
                    .v_flex()
                    .rounded_md()
                    .bg(tint(c_subtle()))
                    .child(bar.bg(gpui_kit::transparent_black()))
                    .child(fixes)
                    .into_any_element(),
                None => bar.into_any_element(),
            });
        }
        if let Some(line) = self.held_line(id, cx) {
            col = col.child(line);
        }
        for (i, queued) in session.queue.iter().enumerate() {
            let images = queued.images.len();
            col = col.child(
                div()
                    .id(("queued", i))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(tint(c_subtle()))
                    .text_size(px(12.5))
                    .child(icon(IconName::Clock, 12., c_muted()))
                    .child(div().text_color(rgb(c_muted())).child("Queued"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(queued.text.lines().next().unwrap_or("").to_string()),
                    )
                    .when(images > 0, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_color(rgb(c_muted()))
                                .child(icon(IconName::Image, 12., c_muted()))
                                .child(images.to_string()),
                        )
                    })
                    .child(
                        self.ghost_button_sized(
                            ("send-queued", i),
                            IconName::Send,
                            if working {
                                "Send now: stop the agent and send this next"
                            } else {
                                "Send now"
                            },
                            20.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.send_queued(id, i, cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        self.ghost_button_sized(("unqueue", i), IconName::X, "Remove", 20.)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(s) = this.state.session_mut(id) {
                                    if i < s.queue.len() {
                                        s.queue.remove(i);
                                    }
                                }
                                this.save();
                                cx.notify();
                            })),
                    ),
            );
        }
        if !attached.is_empty() {
            let mut row = div().flex().flex_wrap().gap_2();
            for (i, path) in attached.iter().enumerate() {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                row = row.child(
                    div()
                        .h(px(48.))
                        .pl_1()
                        .pr_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .border_1()
                        .border_color(divider())
                        .child(
                            img(std::path::PathBuf::from(path))
                                .size(px(40.))
                                .rounded_sm()
                                .object_fit(ObjectFit::Cover),
                        )
                        .child(
                            div()
                                .max_w(px(140.))
                                .truncate()
                                .text_size(px(12.))
                                .child(name),
                        )
                        .child(
                            self.ghost_button_sized(("unattach", i), IconName::X, "Remove", 20.)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(list) = this.attachments.get_mut(&id) {
                                        if i < list.len() {
                                            list.remove(i);
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
            col = col.child(row);
        }
        if !matches.is_empty() {
            let mut list = div()
                .v_flex()
                .p_1()
                .rounded_lg()
                .border_1()
                .border_color(divider())
                .bg(rgb(c_raised()))
                .shadow_lg();
            for (i, pick) in matches.into_iter().enumerate() {
                let (lead, name, detail) = match &pick {
                    MentionPick::Thread(id) => {
                        let Some(t) = self.state.session(*id) else {
                            continue;
                        };
                        let status = crate::crew_ui::thread_status(
                            self.handles.contains_key(&Target::Session(*id)),
                            t,
                        );
                        (
                            div()
                                .size(px(8.))
                                .mx(px(2.))
                                .rounded_full()
                                .bg(rgb(crate::crew_view::status_tone(status)))
                                .into_any_element(),
                            t.title.clone(),
                            "Thread".to_string(),
                        )
                    }
                    MentionPick::File(path) => {
                        let (dir, name) = match path.rsplit_once('/') {
                            Some((d, n)) => (d.to_string(), n.to_string()),
                            None => (String::new(), path.clone()),
                        };
                        (
                            icon(IconName::File, 12., c_muted()).into_any_element(),
                            name,
                            dir,
                        )
                    }
                };
                list = list.child(
                    div()
                        .id(("mention", i))
                        .h(px(26.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .when(i == 0, |el| el.bg(tint(c_active())))
                        .hover(|el| el.bg(tint(c_active())))
                        .text_size(px(12.5))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick_mention(&pick, window, cx);
                            cx.notify();
                        }))
                        .child(lead)
                        .child(div().truncate().child(name))
                        .child(div().truncate().text_color(rgb(c_muted())).child(detail)),
                );
            }
            col = col.child(list);
        }
        if !playbooks.is_empty() {
            let mut list = div()
                .v_flex()
                .p_1()
                .rounded_lg()
                .border_1()
                .border_color(divider())
                .bg(rgb(c_raised()))
                .shadow_lg();
            let heading = |text: String| {
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(text)
            };
            let mut section = None;
            for (i, pick) in playbooks.into_iter().enumerate() {
                let is_playbook = matches!(pick, SlashPick::Playbook(_));
                if section != Some(is_playbook) {
                    section = Some(is_playbook);
                    list = list.child(heading(if is_playbook {
                        format!(
                            "PLAYBOOKS · Enter adds it to this message · {}↵ starts a new session",
                            if cfg!(target_os = "macos") {
                                "⌘"
                            } else {
                                "Ctrl+"
                            }
                        )
                    } else {
                        format!(
                            "{} COMMANDS · Enter puts it in the message",
                            agent::provider(&session.provider).name.to_uppercase()
                        )
                    }));
                }
                let row = div()
                    .id(("slash", i))
                    .min_h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(i == 0, |el| el.bg(tint(c_active())))
                    .hover(|el| el.bg(tint(c_active())))
                    .text_size(px(12.5));
                let picked = pick.clone();
                let row = row.on_click(cx.listener(move |this, _, window, cx| {
                    this.pick_slash(picked.clone(), false, window, cx);
                    cx.notify();
                }));
                let entry = match pick {
                    SlashPick::Playbook(id) => {
                        let Some(a) = self.state.automations.iter().find(|a| a.id == id) else {
                            continue;
                        };
                        row.child(icon(IconName::BookOpen, 12., c_muted()))
                            .child(div().flex_none().child(format!("/{}", a.slug())))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(c_muted()))
                                    .child(a.summary()),
                            )
                            .child(
                                self.ghost_button_sized(
                                    ("slash-run", i),
                                    IconName::Play,
                                    "Start as a new session",
                                    22.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.pick_slash(SlashPick::Playbook(id), true, window, cx);
                                        cx.notify();
                                    },
                                )),
                            )
                    }
                    SlashPick::Command(c) => row
                        .child(icon(
                            if c.source == agentcmds::Source::Skill {
                                IconName::Sparkles
                            } else {
                                IconName::SquareSlash
                            },
                            12.,
                            c_muted(),
                        ))
                        .child(div().flex_none().child(format!("/{}", c.name)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(c_muted()))
                                .child(c.description.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .child(c.source.label()),
                        ),
                };
                list = list.child(entry);
            }
            col = col.child(list);
        }
        Some(col.into_any_element())
    }
}

/// Strip fences, quotes and chatter around a generated commit message.
/// What the agent is told with its next prompt after an undo or redo.
pub fn undo_note(done: &git::Restored, redo: bool) -> String {
    let mut files: Vec<&str> = done.files.iter().map(String::as_str).take(20).collect();
    if done.files.len() > 20 {
        files.push("…");
    }
    let mut note = if redo {
        format!(
            "(Note: I redid the changes I had undone, so these files are back as you left them: {}.",
            files.join(", ")
        )
    } else {
        format!(
            "(Note: I undid your changes from the last turn. These files are back as they were before it: {}.",
            files.join(", ")
        )
    };
    if done.moved_head {
        note.push_str(if redo {
            " Your commits are back on the branch."
        } else {
            " The commits you made in that turn were removed from the branch too."
        });
    }
    note.push_str(" Check the files before relying on what you remember of them.)");
    note
}

pub fn clean_commit_message(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let joined = lines.join("\n");
    joined
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string()
}

fn cx_keep_awake() -> bool {
    settings::keep_awake()
}

/// The reply to the agent's questions: one answer as before, several as a list.
pub fn answers_prompt(questions: &[store::Question], picks: &[Option<String>]) -> String {
    let answered: Vec<(&str, &str)> = questions
        .iter()
        .zip(picks)
        .filter_map(|(q, p)| p.as_deref().map(|a| (q.question.as_str(), a)))
        .collect();
    match answered.as_slice() {
        [(q, a)] => format!("My answer: {q} {a}"),
        _ => {
            let mut out = String::from("My answers:");
            for (q, a) in answered {
                out.push_str(&format!("\n- {q} {a}"));
            }
            out
        }
    }
}

/// Earlier turns, for CLIs that do not keep a conversation, ahead of the new request.
pub fn context_prompt(messages: &[Message], prompt: &str) -> String {
    context_prompt_last(messages, prompt, CONTEXT_MESSAGES)
}

/// The last `keep` messages of the conversation, then the new prompt.
pub fn context_prompt_last(messages: &[Message], prompt: &str, keep: usize) -> String {
    let recent: Vec<&Message> = messages
        .iter()
        .filter(|m| m.role != Role::System && !m.text.is_empty())
        .collect();
    let start = recent.len().saturating_sub(keep);
    let mut out = String::from("Our conversation so far:\n");
    // What fell out of the window still shows, as one line per earlier request.
    let earlier: Vec<String> = recent[..start]
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| {
            let line = m.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let mut line: String = line.trim().chars().take(EARLIER_CHARS).collect();
            if m.text.trim().chars().count() > line.chars().count() {
                line.push('…');
            }
            line
        })
        .collect();
    if !earlier.is_empty() {
        let skipped = earlier.len().saturating_sub(EARLIER_REQUESTS);
        out.push_str("\nEarlier, I asked you to:\n");
        if skipped > 0 {
            out.push_str(&format!("- ({skipped} more before these)\n"));
        }
        for line in &earlier[skipped..] {
            out.push_str(&format!("- {line}\n"));
        }
    }
    for m in &recent[start..] {
        let who = if m.role == Role::User { "Me" } else { "You" };
        let mut text: String = m.text.chars().take(CONTEXT_CHARS).collect();
        if m.text.chars().count() > CONTEXT_CHARS {
            text.push('…');
        }
        out.push_str(&format!("\n{who}: {text}\n"));
    }
    out.push_str(&format!("\nNow: {prompt}"));
    out
}

/// `foo` when the text ends in `@foo` (a mention being typed), else None.
pub fn mention_query(text: &str) -> Option<String> {
    let word = text.rsplit(|c: char| c.is_whitespace()).next()?;
    let query = word.strip_prefix('@')?;
    if query.contains('@') {
        return None;
    }
    Some(query.to_string())
}

/// `foo` when the whole prompt is `/foo` (a playbook being picked), else None.
pub fn slash_query(text: &str) -> Option<String> {
    let query = text.trim_start().strip_prefix('/')?;
    if query.contains(char::is_whitespace) || query.contains('/') {
        return None;
    }
    Some(query.to_lowercase())
}

/// Replace the trailing `@query` with `@path `.
pub fn complete_mention(text: &str, path: &str) -> String {
    match text.rfind('@') {
        Some(at) => format!("{}@{path} ", &text[..at]),
        None => format!("{text}@{path} "),
    }
}

pub(crate) fn is_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
            )
        })
        .unwrap_or(false)
}

/// Where a session keeps the images attached to its prompts: inside its own folder, so
/// an agent on a connection or in a VM finds them at the same place. Git is told to
/// ignore the folder through the repository's exclude file, not `.gitignore`.
pub const ATTACHMENTS: &str = ".runo/attachments";

/// The session's attachments folder, created and kept out of git.
fn attachments_dir(dir: &str) -> Option<std::path::PathBuf> {
    let folder = std::path::Path::new(dir).join(ATTACHMENTS);
    std::fs::create_dir_all(&folder).ok()?;
    exclude_runo_folder(dir);
    Some(folder)
}

/// Add `/.runo/` to the repository's `info/exclude`, once.
pub(crate) fn exclude_runo_folder(dir: &str) {
    let Ok(out) = std::process::Command::new("git")
        .args(["-C", dir, "rev-parse", "--git-path", "info/exclude"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let path = std::path::Path::new(dir).join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text.lines().any(|l| l.trim() == "/.runo/") {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let sep = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let _ = std::fs::write(&path, format!("{text}{sep}/.runo/\n"));
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S%3f").to_string()
}

/// Save pasted image bytes in the session's attachments folder; returns the file path.
fn save_image(dir: &str, bytes: &[u8], format: ImageFormat) -> Option<String> {
    let ext = match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::Webp => "webp",
        ImageFormat::Bmp => "bmp",
        _ => "png",
    };
    let path = attachments_dir(dir)?.join(format!("paste-{}.{ext}", stamp()));
    std::fs::write(&path, bytes).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// Copy an image into the session's attachments folder, unless it is already there.
fn copy_image(dir: &str, image: &std::path::Path) -> Option<String> {
    let folder = attachments_dir(dir)?;
    if image.parent() == Some(folder.as_path()) {
        return Some(image.to_string_lossy().into_owned());
    }
    let name = image.file_name()?.to_string_lossy().into_owned();
    let target = folder.join(format!("{}-{name}", stamp()));
    std::fs::copy(image, &target).ok()?;
    Some(target.to_string_lossy().into_owned())
}

/// The newest Copilot session written since `since` that ran in `dir` and no other
/// session uses: a folder `<id>/` or a file `<id>.jsonl` under `root`. Of several, the
/// one that holds `prompt` wins.
pub fn newest_copilot_session(
    root: &std::path::Path,
    dir: &str,
    prompt: &str,
    since: std::time::SystemTime,
    taken: &std::collections::HashSet<String>,
) -> Option<String> {
    use std::io::Read;
    let head = |path: &std::path::Path| -> String {
        let files: Vec<std::path::PathBuf> = if path.is_dir() {
            std::fs::read_dir(path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        } else {
            vec![path.to_path_buf()]
        };
        let mut text = String::new();
        for file in files {
            let mut head = Vec::new();
            if std::fs::File::open(&file)
                .and_then(|f| f.take(256 * 1024).read_to_end(&mut head))
                .is_ok()
            {
                text.push_str(&String::from_utf8_lossy(&head));
                text.push('\n');
            }
        }
        text
    };
    // The prompt's first line as it appears in Copilot's JSON log.
    let asked = prompt
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().chars().take(60).collect::<String>())
        .and_then(|l| serde_json::to_string(&l).ok())
        .map(|l| l.trim_matches('"').to_string())
        .filter(|l| !l.is_empty());
    let candidates: Vec<(std::time::SystemTime, String, String)> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().ok()?.modified().ok()?;
            let id = if path.is_dir() {
                e.file_name().to_string_lossy().into_owned()
            } else if path.extension().is_some_and(|x| x == "jsonl") {
                path.file_stem()?.to_string_lossy().into_owned()
            } else {
                return None;
            };
            (modified >= since && !taken.contains(&id)).then_some((modified, id, path))
        })
        .map(|(modified, id, path)| (modified, id, head(&path)))
        .filter(|(_, _, text)| mentions_folder(text, dir))
        .collect();
    // Two sessions working in one folder at once are told apart by what was asked.
    let newest = |list: Vec<&(std::time::SystemTime, String, String)>| {
        list.into_iter()
            .max_by_key(|(modified, ..)| *modified)
            .map(|(_, id, _)| id.clone())
    };
    asked
        .and_then(|asked| newest(candidates.iter().filter(|c| c.2.contains(&asked)).collect()))
        .or_else(|| newest(candidates.iter().collect()))
}

/// Whether `text` names the folder `dir` itself or a path inside it, not merely a folder
/// whose name starts the same ("/work/s-1" is not in "/work/s-12").
fn mentions_folder(text: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches(['/', '\\']);
    if dir.is_empty() {
        return false;
    }
    // JSON doubles the backslashes of Windows paths.
    let escaped = dir.replace('\\', "\\\\");
    [dir.to_string(), escaped].iter().any(|dir| {
        text.match_indices(dir.as_str()).any(|(at, _)| {
            !text[at + dir.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    })
}

/// Move messages the agent never read out of the conversation and to the front of the
/// queue, in the order they were sent.
fn take_unsent(session: &mut store::Session, unsent: &[String]) {
    for text in unsent.iter().rev() {
        if let Some(i) = session
            .messages
            .iter()
            .rposition(|m| m.role == Role::User && m.meta == STEER && m.text == *text)
        {
            session.messages.remove(i);
        }
        session
            .queue
            .insert(0, Queued::new(text.clone(), Vec::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn messages_the_agent_never_read_go_back_to_the_queue() {
        let mut session = store::Session::default();
        session.messages.push(Message::new(Role::User, "Fix it"));
        let mut read = Message::new(Role::User, "also the docs");
        read.meta = STEER.into();
        session.messages.push(read.clone());
        session.messages.push(Message::new(Role::Agent, "Done."));
        let mut late = Message::new(Role::User, "and the tests");
        late.meta = STEER.into();
        session.messages.push(late);
        session.queue.push(Queued::new("later", Vec::new()));
        take_unsent(&mut session, &["and the tests".to_string()]);
        let texts: Vec<&str> = session.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["Fix it", "also the docs", "Done."]);
        let queued: Vec<&str> = session.queue.iter().map(|q| q.text.as_str()).collect();
        assert_eq!(queued, ["and the tests", "later"]);
    }

    #[test]
    fn copilot_sessions_match_their_own_folder_and_prompt() {
        let root = std::env::temp_dir().join(format!("runo-copilot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |id: &str, cwd: &str, prompt: &str| {
            let dir = root.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("workspace.yaml"),
                format!("id: {id}\ncwd: {cwd}\n"),
            )
            .unwrap();
            let event = serde_json::json!({"type": "user.message", "data": {"content": prompt}});
            std::fs::write(dir.join("events.jsonl"), format!("{event}\n")).unwrap();
        };
        write("a", "/work/s-12", "Add docs");
        write("b", "/work/s-1", "Add docs");
        write("c", "/work/s-1", "Fix the \"login\" test");
        let since = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let none = std::collections::HashSet::new();
        let find = |dir, prompt| newest_copilot_session(&root, dir, prompt, since, &none);
        assert_eq!(find("/work/s-1", "Add docs").as_deref(), Some("b"));
        assert_eq!(
            find("/work/s-1", "Fix the \"login\" test").as_deref(),
            Some("c")
        );
        assert_eq!(find("/work/s-12", "Add docs").as_deref(), Some("a"));
        assert_eq!(find("/work/s", "Add docs"), None);
        let taken = ["b".to_string()].into_iter().collect();
        assert_eq!(
            newest_copilot_session(&root, "/work/s-1", "Add docs", since, &taken).as_deref(),
            Some("c")
        );
        assert!(mentions_folder("cwd: /work/s-1\n", "/work/s-1/"));
        assert!(mentions_folder(
            r#"{"path":"C:\\w\\s-1\\a.rs"}"#,
            r"C:\w\s-1"
        ));
        assert!(!mentions_folder(r#"{"path":"C:\\w\\s-12"}"#, r"C:\w\s-1"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn commit_messages_are_cleaned() {
        assert_eq!(
            clean_commit_message("```\nAdd login\n\nWith tests\n```\n"),
            "Add login\n\nWith tests"
        );
        assert_eq!(clean_commit_message("\"Fix bug\""), "Fix bug");
    }

    #[test]
    fn slash_starts_a_playbook_pick() {
        assert_eq!(slash_query("/"), Some(String::new()));
        assert_eq!(slash_query("/Find"), Some("find".into()));
        assert_eq!(slash_query("/find bugs"), None);
        assert_eq!(slash_query("see /usr/bin"), None);
        assert_eq!(slash_query("/usr/bin"), None);
    }

    #[test]
    fn mentions_are_found_and_completed() {
        assert_eq!(mention_query("look at @src/ma"), Some("src/ma".into()));
        assert_eq!(mention_query("@"), Some(String::new()));
        assert_eq!(mention_query("mail me@x.com"), None);
        assert_eq!(mention_query("done @a "), None);
        assert_eq!(
            complete_mention("look at @src/ma", "src/main.rs"),
            "look at @src/main.rs "
        );
    }

    #[test]
    fn stateless_agents_get_recent_turns() {
        let mut messages = vec![
            Message::new(Role::User, "Add a flag"),
            Message::new(Role::Agent, "Added --verbose."),
            Message::new(Role::System, "ignored"),
        ];
        for i in 0..10 {
            messages.push(Message::new(Role::User, format!("turn {i}")));
        }
        let prompt = context_prompt(&messages[..3], "Now document it");
        assert!(prompt.contains("Me: Add a flag"));
        assert!(prompt.contains("You: Added --verbose."));
        assert!(!prompt.contains("ignored"));
        assert!(prompt.ends_with("Now: Now document it"));
        let long = context_prompt(&messages, "x");
        assert!(!long.contains("Me: Add a flag"));
        assert!(long.contains("Me: turn 9"));
        // Requests that fell out of the window are still listed, one line each.
        assert!(long.contains("Earlier, I asked you to:\n- Add a flag\n- turn 0\n"));
        assert!(!long.contains("Added --verbose."));
    }
}
