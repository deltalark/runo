//! Chat and CLI views of one conversation. The CLI view runs the agent's own interactive
//! CLI in the centre column on the same conversation id the chat resumes, so switching
//! sends nothing to the model: the CLI loads the saved conversation from disk, and going
//! back to Chat only reads that file to show the turns made in the CLI.

use super::*;
use std::path::PathBuf;
use std::time::SystemTime;

/// A CLI still redrawing within this long is treated as working.
const BUSY_MS: u64 = 1500;

/// Whether the agent in the CLI is in the middle of a reply. Claude Code and Codex show
/// "esc to interrupt" only while they work, and Gemini "(esc to cancel, 3s)" with the time
/// so far, so the screen says it for them; an idle CLI can still redraw on its own (focus,
/// resize, its status line), and their dialogs say a plain "Esc to cancel". Other CLIs
/// fall back to "is it redrawing without being typed at".
fn cli_working(run: &CliRun) -> bool {
    if run.shell.exited() {
        return false;
    }
    match run.provider.as_str() {
        "claude" | "codex" | "gemini" => working_on_screen(&run.shell.text()),
        _ => run.shell.busy(BUSY_MS),
    }
}

fn working_on_screen(screen: &str) -> bool {
    let screen = screen.to_lowercase();
    screen.contains("esc to interrupt")
        || screen
            .match_indices("(esc to cancel, ")
            .any(|(i, m)| screen[i + m.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

pub(crate) struct CliRun {
    pub shell: pty::Shell,
    provider: String,
    /// Runs on a server through Runo's runner, which brings the work and conversation
    /// back when the CLI ends.
    remote: bool,
    /// Asked to end; its turns come into the chat once the runner has synced back.
    closing: bool,
    /// The conversation's transcript and its length when the CLI opened: turns after
    /// that point were made in the CLI.
    transcript: Option<PathBuf>,
    offset: u64,
    opened: SystemTime,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The divider's meta: `cli:<provider>:<turns>`, with `-` when the turns stay in the
/// agent's own history because Runo can't read it.
fn divider_meta(provider: &str, turns: Option<usize>) -> String {
    match turns {
        Some(n) => format!("cli:{provider}:{n}"),
        None => format!("cli:{provider}:-"),
    }
}

fn parse_divider(meta: &str) -> Option<(&str, Option<usize>)> {
    let mut parts = meta.strip_prefix("cli:")?.split(':');
    let provider = parts.next()?;
    let turns = parts.next()?;
    Some((
        provider,
        if turns == "-" {
            None
        } else {
            Some(turns.parse().ok()?)
        },
    ))
}

/// Why a session can't show its CLI view without losing context, if it can't.
pub(crate) fn cli_blocker(session: &Session) -> Option<String> {
    let name = agent::provider(&session.provider).name;
    if session.elsewhere() && session.sandbox().is_none() {
        return Some(format!(
            "This session runs on a connection that no longer exists ({}). Move it to this machine to use the CLI.",
            crate::connections::name(session.connection)
        ));
    }
    if !agent::has_cli_view(&session.provider) {
        return Some(format!(
            "{name}'s CLI can't reopen a conversation by id, so there is no CLI view: switching would lose context or cost tokens."
        ));
    }
    let knows_id = agent::imports_cli_turns(&session.provider)
        || agent::names_conversation(&session.provider)
        || !session.agent_session.is_empty();
    (!knows_id).then(|| {
        format!("Send the first message in Chat so Runo learns {name}'s conversation id. Then the CLI can open the same conversation.")
    })
}

impl RunoApp {
    /// Show the Chat or the CLI view of a session.
    pub(crate) fn set_cli_view(&mut self, id: u64, cli: bool, cx: &mut Context<Self>) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if session.cli == cli {
            return;
        }
        if cli {
            if let Some(reason) = cli_blocker(session) {
                self.fail("cli", reason);
                return;
            }
            if self.handles.contains_key(&Target::Session(id)) {
                self.info = Some((
                    "The agent is still answering. You can switch to the CLI when it's done."
                        .into(),
                    std::time::Instant::now(),
                ));
                return;
            }
            if self.cli_runs.get(&id).is_some_and(|r| r.closing) {
                self.info = Some((
                    "Still bringing the CLI's work back. Try again in a moment.".into(),
                    std::time::Instant::now(),
                ));
                return;
            }
            if let Some(s) = self.state.session_mut(id) {
                s.cli = true;
            }
            self.view_changed = Some((id, std::time::Instant::now()));
            self.open_cli(id, "", cx);
        } else {
            if !self.close_cli(id, false) {
                return;
            }
            self.view_changed = Some((id, std::time::Instant::now()));
            // A prompt queued while the CLI was open goes now.
            self.send_queued(id, 0, cx);
        }
        self.save();
        cx.notify();
    }

    /// Start the agent's interactive CLI on the session's conversation.
    pub(crate) fn open_cli(&mut self, id: u64, first_prompt: &str, cx: &mut Context<Self>) {
        if self.cli_runs.contains_key(&id) {
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let provider = session.provider.clone();
        let dir = session.dir.clone();
        let opts = cx.global::<AppSettings>().agent(&provider);
        let home = home();
        let mut conversation = session.agent_session.clone();
        // Codex names its conversations itself; find the one this session's chat used.
        if provider == "codex" && conversation.is_empty() && session.has_conversation() {
            if let Some((found, _)) = history::newest_codex(&home, &dir, SystemTime::UNIX_EPOCH) {
                conversation = found;
            }
        }
        let imports = agent::imports_cli_turns(&provider);
        let mut transcript = if imports {
            history::transcript(&home, &provider, &conversation)
        } else {
            None
        };
        let resume = if imports {
            transcript.is_some()
        } else {
            session.has_conversation() && !conversation.is_empty()
        };
        if !resume && session.has_conversation() && !self.demo {
            // Opening a fresh CLI would drop the context; refuse rather than lose it.
            if let Some(s) = self.state.session_mut(id) {
                s.cli = false;
            }
            self.fail("cli", format!(
                "{}'s saved conversation for this session wasn't found, so the CLI can't reopen it. Staying in Chat.",
                agent::provider(&provider).name
            ));
            return;
        }
        if agent::names_conversation(&provider) && !resume {
            // A new conversation, named up front so the chat can resume it later.
            conversation = agent::new_uuid();
            transcript = None;
        }
        if let Some(s) = self.state.session_mut(id) {
            s.agent_session = conversation.clone();
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let sel = session.selections();
        // On a connection the CLI runs there, reaching Runo's tools through the relay.
        let sandbox = session.sandbox();
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|_| {
                !self.demo && memory_server() && crate::connections::mcp_reachable(sandbox.as_ref())
            });
        let spec = agent::RunSpec {
            provider: &provider,
            prompt: if resume { "" } else { first_prompt },
            resume,
            sel: &sel,
            opts: &opts,
            in_git: git::is_repo(&dir),
            session_id: &conversation,
            images: &[],
            memory: exe.as_deref().map(|exe| (exe, dir.as_str())),
            local: sandbox.is_none(),
        };
        let mut program = pty::Program {
            bin: if opts.binary.trim().is_empty() {
                agent::provider(&provider).bin.to_string()
            } else {
                opts.binary.trim().to_string()
            },
            args: agent::interactive_args(&spec),
            env: opts
                .env
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .filter(|(k, _)| !k.is_empty())
                .collect(),
            // When Runo itself was started from inside an agent, the nesting marker would
            // stop the CLI saving its conversation, and the turns could not come back to Chat.
            unset: vec!["CLAUDE_CODE_CHILD_SESSION".into()],
        };
        let remote = matches!(sandbox, Some(crate::connections::Sandbox::Remote(_)));
        if let Some(sandbox) = &sandbox {
            let mut command = std::process::Command::new(&program.bin);
            command.args(&program.args);
            for (k, v) in &program.env {
                command.env(k, v);
            }
            let carried =
                (!conversation.is_empty()).then(|| (provider.clone(), conversation.clone()));
            match sandbox.pty_command(&command, &dir, carried) {
                Ok(wrapped) => {
                    let unset = std::mem::take(&mut program.unset);
                    program = pty::Program::of(&wrapped);
                    program.unset = unset;
                }
                Err(e) => {
                    if let Some(s) = self.state.session_mut(id) {
                        s.cli = false;
                    }
                    self.fail("cli", format!("Could not start the CLI: {e}"));
                    return;
                }
            }
        }
        self.next_shell += 1;
        let shell_id = self.next_shell;
        let tx = self.tx.clone();
        let (rows, cols) = self.cli_grid();
        match pty::Shell::spawn_program(shell_id, id, &dir, rows, cols, Some(program), move |sid| {
            let _ = tx.send(Event::Pty(sid));
        }) {
            Ok(shell) => {
                let offset = history::length(transcript.as_deref());
                self.cli_runs.insert(
                    id,
                    CliRun {
                        shell,
                        provider: provider.clone(),
                        remote,
                        closing: false,
                        transcript,
                        offset,
                        opened: SystemTime::now(),
                    },
                );
                self.focus_cli = 2;
                let name = agent::provider(&provider).name;
                self.state.log(
                    id,
                    format!(
                        "Opened {name}'s CLI on the {} conversation",
                        if resume { "same" } else { "new" }
                    ),
                );
            }
            Err(e) => {
                if let Some(s) = self.state.session_mut(id) {
                    s.cli = false;
                }
                self.fail("cli", format!("Could not start the CLI: {e}"));
            }
        }
    }

    /// Close the CLI and bring the turns made in it into the chat. Unless `force`, a CLI
    /// that is still working is left alone and `false` is returned.
    pub(crate) fn close_cli(&mut self, id: u64, force: bool) -> bool {
        if let Some(run) = self.cli_runs.get_mut(&id) {
            if !force && !run.closing && cli_working(run) {
                self.info = Some((
                    "The agent is still replying in the CLI. Switch to Chat when it's done, or press Esc there to stop it."
                        .into(),
                    std::time::Instant::now(),
                ));
                return false;
            }
            // On a server the runner ends the CLI and syncs back first; its turns come
            // into the chat when it exits.
            if run.remote && !run.shell.exited() {
                run.shell.finish();
                run.closing = true;
                if let Some(s) = self.state.session_mut(id) {
                    s.cli = false;
                }
                self.info = Some((
                    "Bringing the CLI's work back to this machine…".into(),
                    std::time::Instant::now(),
                ));
                self.save();
                return true;
            }
        }
        if let Some(run) = self.cli_runs.remove(&id) {
            run.shell.kill();
            self.import_cli_turns(id, &run);
            // Files the CLI changed show up in the Changes panel right away.
            if self.selected == Some(id) && !self.demo {
                self.refresh();
            }
        }
        if let Some(s) = self.state.session_mut(id) {
            s.cli = false;
        }
        self.save();
        true
    }

    /// Runo is quitting with CLIs open, and they close with it: bring their turns into
    /// the chat now. The sessions stay in CLI view and reopen on the same conversation.
    pub(crate) fn keep_cli_turns(&mut self) {
        // CLIs on servers sync back before they close, for a few seconds at most.
        for run in self
            .cli_runs
            .values()
            .filter(|r| r.remote && !r.shell.exited())
        {
            run.shell.finish();
        }
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(15)
            && self
                .cli_runs
                .values()
                .any(|r| r.remote && !r.shell.exited())
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        let ids: Vec<u64> = self.cli_runs.keys().copied().collect();
        for id in ids {
            if let Some(run) = self.cli_runs.remove(&id) {
                run.shell.kill();
                self.import_cli_turns(id, &run);
            }
        }
    }

    fn import_cli_turns(&mut self, id: u64, run: &CliRun) {
        let home = home();
        let Some((dir, conversation)) = self
            .state
            .session(id)
            .map(|s| (s.dir.clone(), s.agent_session.clone()))
        else {
            return;
        };
        let (path, offset) = match &run.transcript {
            Some(p) => (Some(p.clone()), run.offset),
            None if run.provider == "codex" => {
                match history::newest_codex(&home, &dir, run.opened) {
                    Some((found, path)) => {
                        if let Some(s) = self.state.session_mut(id) {
                            s.agent_session = found;
                        }
                        (Some(path), 0)
                    }
                    None => (None, 0),
                }
            }
            None => (history::transcript(&home, &run.provider, &conversation), 0),
        };
        let name = agent::provider(&run.provider).name;
        let Some(path) = path else {
            if !agent::imports_cli_turns(&run.provider) {
                // The turns live in the agent's own history; mark where the CLI was used.
                let mut divider = Message::now(Role::System, format!("Used {name}'s CLI"));
                divider.meta = divider_meta(&run.provider, None);
                if let Some(s) = self.state.session_mut(id) {
                    s.messages.push(divider);
                }
            }
            return;
        };
        let mut turns = history::turns(&run.provider, &history::read_from(&path, offset));
        let prompts = turns.iter().filter(|m| m.role == Role::User).count();
        if turns.is_empty() {
            return;
        }
        let time = chrono::Local::now().format("%-I:%M %p").to_string();
        let mut divider = Message::now(Role::System, format!("Worked in {name}'s CLI"));
        divider.meta = divider_meta(&run.provider, Some(prompts));
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        for m in &mut turns {
            m.time = time.clone();
            for step in &mut m.steps {
                *step = step.replace(&prefix, "");
            }
        }
        if let Some(s) = self.state.session_mut(id) {
            s.messages.push(divider);
            s.messages.extend(turns);
            if s.status != Status::Working {
                s.status = Status::Done;
            }
        }
        self.state.log(
            id,
            format!(
                "Brought {prompts} CLI turn{} into the chat",
                if prompts == 1 { "" } else { "s" }
            ),
        );
        self.chat_scroll.scroll_to_bottom();
    }

    /// The CLI quit on its own (for example `/exit`): go back to Chat.
    pub(crate) fn cli_exited(&mut self, shell: u64) {
        let Some((id, closing)) = self
            .cli_runs
            .iter()
            .find(|(_, r)| r.shell.id == shell && r.shell.exited())
            .map(|(id, r)| (*id, r.closing))
        else {
            return;
        };
        self.close_cli(id, true);
        if closing {
            self.info = Some((
                "The CLI's work is back. You're in Chat, on the same conversation.".into(),
                std::time::Instant::now(),
            ));
            return;
        }
        self.view_changed = Some((id, std::time::Instant::now()));
        self.info = Some((
            "The CLI closed. You're back in Chat, on the same conversation.".into(),
            std::time::Instant::now(),
        ));
    }

    pub(crate) fn cli_key(&mut self, id: u64, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let m = ks.modifiers;
        // The view shortcut works from inside the CLI too.
        if (m.control || m.platform) && m.shift && ks.key == "m" {
            self.set_cli_view(id, false, cx);
            cx.stop_propagation();
            return;
        }
        let Some(run) = self.cli_runs.get(&id) else {
            return;
        };
        if let Some(selection) = self.pty_key(&run.shell, event, cx) {
            let what = if selection { "selection" } else { "CLI screen" };
            self.info = Some((format!("Copied the {what}."), std::time::Instant::now()));
        }
        cx.notify();
    }

    /// Rows and columns that fit the CLI view, measured last frame.
    pub(crate) fn cli_grid(&self) -> (u16, u16) {
        match self.tour_rects.get("cli-view") {
            Some(b) => {
                let w = f32::from(b.size.width) - 24.;
                let h = f32::from(b.size.height) - 16.;
                (
                    (h / terminal::term_line()).floor().max(4.) as u16,
                    (w / self.cell_w).floor().max(20.) as u16,
                )
            }
            None => (40, 100),
        }
    }

    /// The segmented Chat / CLI switch in the session header.
    pub(crate) fn view_toggle(&self, session: &Session, cx: &mut Context<Self>) -> AnyElement {
        let id = session.id;
        let blocker = cli_blocker(session);
        let supported = blocker.is_none();
        let name = agent::provider(&session.provider).name;
        let option = |key: &'static str, glyph: IconName, label: &'static str, on: bool| {
            let active = session.cli == on;
            div()
                .id(key)
                .h(px(22.))
                .px_2()
                .flex()
                .items_center()
                .gap(px(5.))
                .rounded_md()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .relative()
                .when(active, |el| {
                    el.child(crate::shell::seg_pill(format!("{key}-fill")))
                        .text_color(rgb(c_text()))
                })
                .when(!active, |el| {
                    el.text_color(rgb(c_muted()))
                        .hover(|el| el.text_color(rgb(c_text())))
                })
                .child(if active {
                    crate::shell::pop(icon(glyph, 12., c_text()), format!("{key}-icon"))
                } else {
                    icon(glyph, 12., c_muted()).into_any_element()
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_cli_view(id, on, cx)))
        };
        crate::shell::seg_track()
            .id("view-toggle")
            .flex_none()
            .p(px(2.))
            .when(!supported, |el| el.opacity(0.5))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(match &blocker {
                    None => format!(
                        "Same conversation, two views. Switching is free: nothing is sent to {name}. {}",
                        keymap::label(cx, "toggle-view")
                    ),
                    Some(reason) => reason.clone(),
                })
                .build(window, cx)
            })
            .child(option("view-chat", IconName::MessageSquare, "Chat", false))
            .child(option("view-cli", IconName::SquareTerminal, "CLI", true))
            .into_any_element()
    }

    /// The centre column in CLI view: a slim banner over the agent's own CLI.
    pub(crate) fn cli_body(
        &self,
        session: &Session,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider = agent::provider(&session.provider);
        let fresh = self
            .view_changed
            .is_some_and(|(id, at)| id == session.id && at.elapsed().as_secs() < 6);
        let banner = div()
            .flex_none()
            .h(px(30.))
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(divider())
            .bg(tint(c_subtle()))
            .text_size(px(12.))
            .text_color(rgb(c_secondary()))
            .child(self.provider_badge(provider, 16.))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_text()))
                    .child(format!("{} CLI", provider.name)),
            )
            .child(div().text_color(rgb(c_muted())).child("·"))
            .child(icon(IconName::Link, 12., c_green()))
            .child("Same conversation as the chat")
            .child(div().flex_1())
            .child(
                div()
                    .text_color(rgb(c_muted()))
                    .child("Switch to Chat any time, nothing is sent"),
            );
        let banner = if fresh {
            rise(
                banner,
                SharedString::from(format!("cli-banner-{}", session.id)),
                360,
            )
        } else {
            banner.into_any_element()
        };
        let body = match self.cli_runs.get(&session.id) {
            Some(run) => self
                .pty_view(&run.shell, self.cli_grid(), Some(session.id), window, cx)
                .size_full()
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(c_muted()))
                .child(format!("Opening {}…", provider.name))
                .into_any_element(),
        };
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .bg(term_fill())
            .child(banner)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(tour::anchor(&self.anchors, "cli-view"))
                    .child(body),
            )
            .into_any_element()
    }

    /// "Worked in Claude Code's CLI · 2 turns" between chat messages.
    pub(crate) fn cli_divider(&self, index: usize, meta: &str) -> Option<AnyElement> {
        let (provider, turns) = parse_divider(meta)?;
        let provider = agent::provider(provider);
        let line = || div().flex_1().h(px(1.)).bg(divider());
        Some(
            rise(
                div()
                    .id(("cli-div", index))
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
                            .bg(rgb(c_raised()))
                            .text_size(px(12.))
                            .child(icon(IconName::SquareTerminal, 13., c_secondary()))
                            .child(div().font_weight(FontWeight::MEDIUM).child(format!(
                                "{} {}'s CLI",
                                if turns.is_some() { "Worked in" } else { "Used" },
                                provider.name
                            )))
                            .child(div().text_color(rgb(c_muted())).child(match turns {
                                Some(n) => format!(
                                    "{n} turn{} · same conversation",
                                    if n == 1 { "" } else { "s" }
                                ),
                                None => format!(
                                    "same conversation · its turns are in {}'s own history",
                                    provider.name
                                ),
                            })),
                    )
                    .child(line()),
                SharedString::from(format!("cli-div-rise-{index}")),
                420,
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
    fn dividers_round_trip() {
        assert_eq!(
            parse_divider(&divider_meta("claude", Some(3))),
            Some(("claude", Some(3)))
        );
        assert_eq!(
            parse_divider(&divider_meta("gemini", None)),
            Some(("gemini", None))
        );
        assert_eq!(parse_divider("switch:claude:codex:1:20"), None);
    }

    #[test]
    fn working_lines_not_dialogs() {
        assert!(working_on_screen("✻ Thinking… (esc to interrupt)"));
        assert!(working_on_screen("Working (12s • esc to interrupt)"));
        assert!(working_on_screen("⠋ Reading files (esc to cancel, 4s)"));
        assert!(!working_on_screen("Enter to retry, or Esc to cancel"));
        assert!(!working_on_screen(
            "New Task: Enter to submit; Esc to cancel"
        ));
        assert!(!working_on_screen("(Press Enter to submit, Esc to cancel)"));
        assert!(!working_on_screen(
            "> Try \"fix lint errors\"\n  accept edits on"
        ));
    }
}
