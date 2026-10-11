//! Agent turns: queued prompts, per-turn checkpoints with Undo / Keep, image attachments,
//! @file mentions, plans, questions and permission requests.

use super::*;

/// Most recent conversation, written into follow-ups for CLIs that cannot resume.
const CONTEXT_MESSAGES: usize = 8;
const CONTEXT_CHARS: usize = 1500;

impl RunoApp {
    /// Start one agent turn: snapshot the worktree, then launch the CLI.
    pub(crate) fn start_turn(
        &mut self,
        id: u64,
        prompt: String,
        images: Vec<String>,
        opts: &AgentOptions,
    ) {
        self.stderr.remove(&id);
        // The CLI view owns the conversation while it is open; two writers would fork it.
        if self.cli_runs.contains_key(&id) {
            if let Some(s) = self.state.session_mut(id) {
                s.queue.insert(0, prompt);
            }
            self.notice = Some(
                "This session is open in the CLI. The prompt is queued for when you switch back to Chat."
                    .into(),
            );
            return;
        }
        let demo = self.demo;
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
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
        self.json_text.remove(&id);
        let dir = session.dir.clone();
        let in_git = !demo && git::is_repo(&dir);
        // Agents that cannot resume their own conversation get it written into the prompt.
        let handoff = std::mem::take(&mut session.handoff);
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
                format!("{}\n{recent}", handoff::digest(&session.messages, name))
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
        let mut user = Message::now(Role::User, prompt.clone());
        user.images = images.clone();
        if in_git {
            user.checkpoint = git::checkpoint(&dir).unwrap_or_default();
        }
        session.messages.push(user);
        session.messages.push(Message::now(Role::Agent, ""));
        let sel = session.selections();
        let agent_session = session.agent_session.clone();
        let sandbox = session.sandbox();
        // Runo's memory server is a program on this machine, which a VM can't start.
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|_| !demo && memory_server() && sandbox.is_none());
        let spec = agent::RunSpec {
            provider: &provider,
            prompt: &sent,
            resume: native,
            sel: &sel,
            opts,
            in_git,
            session_id: &agent_session,
            images: &images,
            memory: exe.as_deref().map(|exe| (exe, dir.as_str())),
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
                if provider == "claude" {
                    self.streams.insert(id, stream::ClaudeStream::new(&dir));
                }
                self.handles.insert(Target::Session(id), handle);
                self.started.insert(id, std::time::Instant::now());
                self.turn_files.remove(&id);
                let first = prompt.lines().next().unwrap_or("").to_string();
                self.state.log(id, format!("Started: {first}"));
            }
            Err(error) => {
                session.status = Status::Failed;
                session.messages.pop();
                session.messages.push(Message::new(Role::System, error));
            }
        }
        self.save();
        self.chat_scroll.scroll_to_bottom();
        self.keep_awake_update();
    }

    /// Updates from one line of JSON output, remembering the conversation id it names.
    pub(crate) fn json_updates(&mut self, id: u64, line: &str) -> Vec<stream::Update> {
        let Some(parsed) = agent::json_line(line) else {
            return if line.trim().is_empty() {
                Vec::new()
            } else {
                vec![stream::Update::Text(line.to_string())]
            };
        };
        if let Some(conversation) = parsed.conversation {
            if let Some(s) = self.state.session_mut(id) {
                s.agent_session = conversation;
            }
        }
        let mut out: Vec<stream::Update> =
            parsed.steps.into_iter().map(stream::Update::Step).collect();
        if let Some(text) = parsed.text {
            self.json_text.insert(id);
            out.push(stream::Update::Text(text));
        }
        if let Some(result) = parsed.result.filter(|_| !self.json_text.contains(&id)) {
            out.push(stream::Update::Text(result));
        }
        if let Some(error) = parsed.error {
            out.push(stream::Update::Error(error));
        }
        out
    }

    /// Copilot prints no conversation id; after a run, its newest saved session is this one.
    pub(crate) fn find_copilot_session(&mut self, id: u64, since: std::time::SystemTime) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if session.provider != "copilot" || !session.agent_session.is_empty() {
            return;
        }
        let root = std::env::var_os("COPILOT_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .map(|h| std::path::PathBuf::from(h).join(".copilot"))
            })
            .map(|p| p.join("session-state"));
        let newest = root
            .and_then(|r| std::fs::read_dir(r).ok())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let modified = e.metadata().ok()?.modified().ok()?;
                (e.path().is_dir() && modified >= since)
                    .then(|| (modified, e.file_name().to_string_lossy().into_owned()))
            })
            .max();
        if let Some((_, found)) = newest {
            if let Some(s) = self.state.session_mut(id) {
                s.agent_session = found;
            }
        }
    }

    /// After a turn: count what it changed, then send the next queued prompt.
    pub(crate) fn finish_turn(&mut self, id: u64, ok: bool, stopped: bool, cx: &mut Context<Self>) {
        let checkpoint = self.state.session(id).and_then(|s| {
            s.messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| (m.checkpoint.clone(), s.dir.clone()))
        });
        if let Some((cp, dir)) = checkpoint.filter(|(cp, _)| !cp.is_empty()) {
            let files = git::changed_since(&dir, &cp).len();
            self.turn_files.insert(id, files);
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
        if !ok {
            return;
        }
        let next = self
            .state
            .session_mut(id)
            .filter(|s| !s.queue.is_empty())
            .map(|s| (s.queue.remove(0), s.provider.clone()));
        if let Some((prompt, provider)) = next {
            let opts = cx.global::<AppSettings>().agent(&provider);
            self.start_turn(id, prompt, Vec::new(), &opts);
        }
    }

    /// Send the composer's prompt, or queue it while the agent is busy.
    pub(crate) fn submit_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some(id) = self.selected else { return };
        let text = self.prompt.read(cx).value().trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        self.prompt.update(cx, |p, cx| p.set_value("", window, cx));
        self.mention = None;
        if self.handles.contains_key(&Target::Session(id)) {
            if let Some(session) = self.state.session_mut(id) {
                session.queue.push(text);
            }
            self.save();
            self.info = Some((
                "Queued. It is sent when the agent finishes.".into(),
                std::time::Instant::now(),
            ));
            return;
        }
        let images = std::mem::take(&mut self.attachments);
        let text = if text.is_empty() {
            "Look at the attached image.".to_string()
        } else {
            text
        };
        let opts = self
            .current()
            .map(|s| cx.global::<AppSettings>().agent(&s.provider))
            .unwrap_or_default();
        self.start_turn(id, text, images, &opts);
    }

    /// Put the worktree back as it was before user message `index` ran.
    pub(crate) fn undo_to(&mut self, id: u64, index: usize) {
        if self.handles.contains_key(&Target::Session(id)) {
            self.notice = Some("Stop the agent before undoing its changes.".into());
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
        match git::restore(&session.dir, &cp) {
            Ok(n) => {
                if let Some(session) = self.state.session_mut(id) {
                    for m in session.messages.iter_mut().skip(index) {
                        if m.role == Role::Agent && m.resolved.is_empty() {
                            m.resolved = "undone".into();
                        }
                    }
                }
                self.turn_files.insert(id, 0);
                self.state.log(id, format!("Undid changes to {n} files"));
                self.info = Some((
                    format!("Restored {n} file{}.", if n == 1 { "" } else { "s" }),
                    std::time::Instant::now(),
                ));
                self.save();
                self.measure(id);
                self.refresh();
            }
            Err(error) => self.notice = Some(format!("Could not undo: {error}")),
        }
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
        if let Some(item) = cx.read_from_clipboard() {
            for entry in item.entries() {
                match entry {
                    ClipboardEntry::Image(image) => {
                        if let Some(path) = save_image(&image.bytes, image.format) {
                            self.attachments.push(path);
                            return;
                        }
                    }
                    ClipboardEntry::ExternalPaths(paths) => {
                        let before = self.attachments.len();
                        self.attach_paths(paths.paths());
                        if self.attachments.len() > before {
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

    /// Images become attachments; other files are mentioned in the prompt as @path.
    pub(crate) fn attach_paths(&mut self, paths: &[std::path::PathBuf]) {
        let dir = self.current().map(|s| s.dir.clone()).unwrap_or_default();
        for path in paths {
            if is_image(path) {
                let p = path.to_string_lossy().into_owned();
                if !self.attachments.contains(&p) {
                    self.attachments.push(p);
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
        self.slash = slash_query(&text);
    }

    /// Playbooks matching the `/query`, best first.
    pub(crate) fn slash_matches(&self) -> Vec<u64> {
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
        hits.into_iter().take(8).map(|(_, a)| a.id).collect()
    }

    /// Use a playbook from the `/` menu: put its prompt in the box for this session, or
    /// start it as its own session in this project.
    pub(crate) fn pick_slash(
        &mut self,
        playbook: u64,
        new_session: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.slash = None;
        let Some(prompt) = self
            .state
            .automations
            .iter()
            .find(|a| a.id == playbook)
            .map(|a| a.prompt.clone())
        else {
            return;
        };
        if new_session {
            self.prompt.update(cx, |p, cx| p.set_value("", window, cx));
            let here = self.current().map(|s| s.project_id);
            self.run_playbook(playbook, here, cx);
            return;
        }
        if let Some(a) = self.state.automations.iter_mut().find(|a| a.id == playbook) {
            a.last_run = workspace::unix_now();
        }
        self.prompt.update(cx, |p, cx| {
            p.set_value(prompt, window, cx);
            p.focus(window, cx);
        });
    }

    /// Files matching the `@query`, best first.
    pub(crate) fn mention_matches(&self) -> Vec<String> {
        let Some(query) = &self.mention else {
            return Vec::new();
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
        hits.into_iter().take(8).map(|(_, f)| f.clone()).collect()
    }

    pub(crate) fn pick_mention(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.prompt.read(cx).value().to_string();
        let new = complete_mention(&text, path);
        self.prompt.update(cx, |p, cx| {
            p.set_value(new, window, cx);
            p.focus(window, cx);
        });
        self.mention = None;
    }

    /// Write a commit message for the current changes with the session's agent.
    pub(crate) fn suggest_commit(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.current() else {
            return;
        };
        if self.demo {
            self.commit_suggestion = Some("Refine sidebar navigation and nav items".into());
            return;
        }
        let patch = git::patch_for_message(&session.dir, 24_000);
        if patch.trim().is_empty() {
            self.notice = Some("There are no changes to describe.".into());
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
                        self.notice = Some("The agent did not write a commit message.".into());
                    } else {
                        self.commit_suggestion = Some(text);
                    }
                }
                Err(error) => {
                    let first = error.lines().next().unwrap_or("").to_string();
                    self.notice = Some(format!("Could not write a commit message: {first}"));
                }
            }
        } else if self.handle_alert_job(&key, result.clone()) {
        } else if !self.handle_browser_job(&key, result.clone()) {
            self.handle_job(key, result);
        }
    }

    /// Apply values produced outside of render (they need a window to reach inputs).
    pub(crate) fn apply_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let want = cx_keep_awake() && !self.started.is_empty();
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
        if !message.resolved.is_empty() {
            return None;
        }
        let user = session.messages[..index]
            .iter()
            .rposition(|m| m.role == Role::User)?;
        if session.messages[user].checkpoint.is_empty() {
            return None;
        }
        let files = self.turn_files.get(&session.id).copied();
        if files == Some(0) {
            return None;
        }
        let id = session.id;
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(c_secondary()))
                .child(match files {
                    Some(n) => format!(
                        "This turn changed {n} file{}.",
                        if n == 1 { "" } else { "s" }
                    ),
                    None => "Changes from this turn.".into(),
                })
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
        div()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
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
            .when(last && !busy, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .border_t_1()
                        .border_color(rgb(c_border()))
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
                                    if let Some(id) = this.selected {
                                        if let Some(s) = this.state.session_mut(id) {
                                            s.opts.insert("mode".into(), "edit".into());
                                        }
                                    }
                                    this.answer(
                                        "The plan is approved. Go ahead and implement it.".into(),
                                        Vec::new(),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    /// The agent's multiple-choice questions; picking an option sends it as the answer.
    pub(crate) fn question_card(
        &self,
        index: usize,
        questions: &[store::Question],
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut card = div()
            .p_3()
            .v_flex()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()));
        for (qi, q) in questions.iter().enumerate() {
            let mut options = div().flex().flex_wrap().gap_2();
            for (oi, option) in q.options.iter().enumerate() {
                let answer = format!("{} {}", q.question, option);
                options = options.child(
                    self.chip(
                        ("answer", index * 100 + qi * 10 + oi),
                        option.clone(),
                        false,
                    )
                    .when(!last, |el| el.opacity(0.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if last {
                            this.answer(format!("My answer: {answer}"), Vec::new(), cx);
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
        if session.queue.is_empty()
            && self.attachments.is_empty()
            && matches.is_empty()
            && playbooks.is_empty()
        {
            return None;
        }
        let mut col = div().v_flex().gap_1();
        for (i, queued) in session.queue.iter().enumerate() {
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
                            .child(queued.lines().next().unwrap_or("").to_string()),
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
        if !self.attachments.is_empty() {
            let mut row = div().flex().flex_wrap().gap_2();
            for (i, path) in self.attachments.iter().enumerate() {
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
                        .border_color(rgb(c_border()))
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
                                    if i < this.attachments.len() {
                                        this.attachments.remove(i);
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
                .border_color(rgb(c_border()))
                .bg(rgb(c_raised()))
                .shadow_lg();
            for (i, path) in matches.into_iter().enumerate() {
                let (dir, name) = match path.rsplit_once('/') {
                    Some((d, n)) => (d.to_string(), n.to_string()),
                    None => (String::new(), path.clone()),
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
                            this.pick_mention(&path, window, cx);
                            cx.notify();
                        }))
                        .child(icon(IconName::File, 12., c_muted()))
                        .child(name)
                        .child(div().truncate().text_color(rgb(c_muted())).child(dir)),
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
                .border_color(rgb(c_border()))
                .bg(rgb(c_raised()))
                .shadow_lg()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(format!(
                            "PLAYBOOKS · Enter uses it here · {}↵ starts a new session",
                            if cfg!(target_os = "macos") {
                                "⌘"
                            } else {
                                "Ctrl+"
                            }
                        )),
                );
            for (i, id) in playbooks.into_iter().enumerate() {
                let Some(a) = self.state.automations.iter().find(|a| a.id == id) else {
                    continue;
                };
                list = list.child(
                    div()
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
                        .text_size(px(12.5))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick_slash(id, false, window, cx);
                            cx.notify();
                        }))
                        .child(icon(IconName::BookOpen, 12., c_muted()))
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
                                    this.pick_slash(id, true, window, cx);
                                    cx.notify();
                                },
                            )),
                        ),
                );
            }
            col = col.child(list);
        }
        Some(col.into_any_element())
    }
}

/// Strip fences, quotes and chatter around a generated commit message.
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

fn is_image(path: &std::path::Path) -> bool {
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

/// Save pasted image bytes under Runo's data folder; returns the file path.
fn save_image(bytes: &[u8], format: ImageFormat) -> Option<String> {
    let ext = match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::Webp => "webp",
        ImageFormat::Bmp => "bmp",
        _ => "png",
    };
    let dir = store::data_dir().join("images");
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%3f");
    let path = dir.join(format!("paste-{stamp}.{ext}"));
    std::fs::write(&path, bytes).ok()?;
    Some(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

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
        assert!(!long.contains("Add a flag"));
        assert!(long.contains("turn 9"));
    }
}
