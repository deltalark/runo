//! Shell tabs in the terminal panel: real PTYs rendered from a VT100 screen.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static FONT_SIZE: AtomicU32 = AtomicU32::new(12);
static SCROLL_SPEED: AtomicU32 = AtomicU32::new(1);
static RIGHT_CLICK_PASTE: AtomicBool = AtomicBool::new(false);

/// Apply the Terminal settings (size in points, wheel multiplier, right-click paste).
pub(crate) fn configure(size: u32, scroll: u32, right_paste: bool) {
    FONT_SIZE.store(size.clamp(9, 24), Ordering::Relaxed);
    SCROLL_SPEED.store(scroll.max(1), Ordering::Relaxed);
    RIGHT_CLICK_PASTE.store(right_paste, Ordering::Relaxed);
}

/// Terminal text size.
pub(crate) fn term_font() -> f32 {
    FONT_SIZE.load(Ordering::Relaxed) as f32
}

/// Terminal row height: a third taller than the text, on whole pixels.
pub(crate) fn term_line() -> f32 {
    (term_font() * 4. / 3.).round()
}

impl RunoApp {
    pub(crate) fn session_shells(&self, session: u64) -> impl Iterator<Item = &pty::Shell> {
        self.shells.iter().filter(move |s| s.session == session)
    }

    /// The shell shown in the terminal panel, if a shell tab is selected.
    pub(crate) fn active_shell(&self) -> Option<&pty::Shell> {
        let session = self.selected?;
        let id = self.shell_tab?;
        self.shells
            .iter()
            .find(|s| s.id == id && s.session == session)
    }

    pub(crate) fn new_shell(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected else {
            return;
        };
        let dir = self.current().map(|s| s.dir.clone()).unwrap_or_default();
        // A Cove session's terminal is a shell inside its VM.
        // A session on a connection gets its terminal there: a shell in its VM, or on
        // the server in the session's copy.
        let program = match self.current().and_then(|s| s.sandbox()) {
            Some(sandbox) => match sandbox.shell_program(&dir) {
                Ok((bin, args)) => Some(pty::Program {
                    bin,
                    args,
                    env: Vec::new(),
                    unset: Vec::new(),
                }),
                Err(error) => {
                    self.info = Some((error, std::time::Instant::now()));
                    cx.notify();
                    return;
                }
            },
            None => None,
        };
        // Named after where it runs: "Build box 1".
        let place = program.as_ref().and_then(|_| {
            self.current()
                .map(|s| crate::connections::name(s.connection))
        });
        self.next_shell += 1;
        let id = self.next_shell;
        let tx = self.tx.clone();
        let (rows, cols) = self.term_grid();
        match pty::Shell::spawn_program(id, session, &dir, rows, cols, program, move |id| {
            let _ = tx.send(Event::Pty(id));
        }) {
            Ok(mut shell) => {
                let n = self.session_shells(session).count() + 1;
                if let Some(place) = place {
                    shell.title = place;
                }
                shell.title = format!("{} {n}", shell.title);
                self.shells.push(shell);
                self.shell_tab = Some(id);
                self.cli_tab = false;
                // The shell's element only exists from the next render on.
                self.focus_shell = 2;
            }
            Err(e) => {
                self.info = Some((
                    format!("Could not start a shell: {e}"),
                    std::time::Instant::now(),
                ))
            }
        }
        cx.notify();
    }

    /// Run the worktree setup command in a terminal tab of session `id`, where it can
    /// ask questions; the agent starts when it exits. Returns whether it started.
    pub(crate) fn run_setup_in(&mut self, id: u64, setup: &str, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.state.session(id) else {
            return false;
        };
        let dir = session.dir.clone();
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", setup]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", setup]);
            c
        };
        // On a connection it runs there, in the session's files.
        if let Some(sandbox) = session.sandbox() {
            match sandbox.pty_command(&command, &dir, None) {
                Ok(wrapped) => command = wrapped,
                Err(error) => {
                    self.notice = Some(format!("The setup command could not start: {error}"));
                    return false;
                }
            }
        }
        self.next_shell += 1;
        let shell_id = self.next_shell;
        let tx = self.tx.clone();
        let (rows, cols) = self.term_grid();
        match pty::Shell::spawn_program(
            shell_id,
            id,
            &dir,
            rows,
            cols,
            Some(pty::Program::of(&command)),
            move |sid| {
                let _ = tx.send(Event::Pty(sid));
            },
        ) {
            Ok(mut shell) => {
                shell.title = "Setup".into();
                self.shells.push(shell);
                self.setups.insert(shell_id, id);
                if self.selected == Some(id) {
                    self.shell_tab = Some(shell_id);
                    self.cli_tab = false;
                    self.focus_shell = 2;
                }
                cx.notify();
                true
            }
            Err(e) => {
                self.notice = Some(format!("The setup command could not start: {e}"));
                false
            }
        }
    }

    /// A setup command's terminal ended (or was closed): start the agent it held back.
    pub(crate) fn setup_exited(&mut self, shell: u64, cx: &mut Context<Self>) {
        let ok = match self.shells.iter().find(|s| s.id == shell) {
            Some(s) if s.exited() => s.success().unwrap_or(true),
            Some(_) => return,
            None => false,
        };
        let Some(id) = self.setups.remove(&shell) else {
            return;
        };
        self.setup_done(id, ok, cx);
    }

    pub(crate) fn close_shell(&mut self, id: u64, cx: &mut Context<Self>) {
        self.shells.retain(|s| s.id != id);
        if self.shell_tab == Some(id) {
            let session = self.selected.unwrap_or(0);
            self.shell_tab = self.session_shells(session).last().map(|s| s.id);
        }
        cx.notify();
    }

    /// Shells of deleted sessions are closed with them.
    pub(crate) fn prune_shells(&mut self) {
        let state = &self.state;
        self.shells.retain(|s| state.session(s.session).is_some());
        self.cli_runs.retain(|id, _| state.session(*id).is_some());
        self.forwards
            .retain(|(id, _), _| state.session(*id).is_some());
    }

    /// Rows and columns that fit the terminal body.
    pub(crate) fn term_grid(&self) -> (u16, u16) {
        let height = self.term_height() - 32. - 16.;
        let width = self.panel_width - 24.;
        let rows = (height / term_line()).floor().max(2.) as u16;
        let cols = (width / self.cell_w).floor().max(10.) as u16;
        (rows, cols)
    }

    pub(crate) fn term_height(&self) -> f32 {
        if self.term_tall {
            560.
        } else {
            230.
        }
    }

    /// Measure the monospace cell once a window is available.
    /// The terminal's font: the Terminal setting, or the theme's monospace font.
    pub(crate) fn term_family(&self, cx: &App) -> SharedString {
        let family = cx.global::<AppSettings>().term_font.trim();
        if family.is_empty() {
            self.mono_font.clone()
        } else {
            SharedString::from(family.to_string())
        }
    }

    pub(crate) fn measure_cell(&mut self, window: &mut Window, cx: &App) {
        let family = self.term_family(cx);
        let key = (family.clone(), FONT_SIZE.load(Ordering::Relaxed));
        if self.cell_key.as_ref() == Some(&key) {
            return;
        }
        self.cell_key = Some(key);
        let ts = window.text_system();
        let id = ts.resolve_font(&font(family));
        if let Ok(size) = ts.advance(id, px(term_font()), 'm') {
            let w = f32::from(size.width);
            if w > 3. {
                self.cell_w = w;
            }
        }
    }

    pub(crate) fn shell_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shell) = self.active_shell() else {
            return;
        };
        if let Some(selection) = self.pty_key(shell, event, cx) {
            let what = if selection {
                "selection"
            } else {
                "terminal screen"
            };
            self.info = Some((format!("Copied the {what}."), std::time::Instant::now()));
        }
        cx.notify();
        let _ = window;
    }

    /// Send a keystroke to a PTY program, handling copy, paste and the find bar.
    /// Returns `Some(true)` when the selection was copied, `Some(false)` for the screen.
    pub(crate) fn pty_key(
        &self,
        shell: &pty::Shell,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let ks = &event.keystroke;
        let m = ks.modifiers;
        // Find in the scrollback: Cmd-F on macOS, Ctrl-Shift-F elsewhere.
        if (m.platform && ks.key == "f") || (m.control && m.shift && ks.key == "f") {
            shell.view(|v| {
                if v.find.is_none() {
                    v.find = Some(String::new());
                    v.find_at = 0;
                }
            });
            cx.stop_propagation();
            return None;
        }
        if shell.view(|v| v.find.is_some()) && !m.platform {
            match ks.key.as_str() {
                "escape" => shell.view(|v| {
                    v.find = None;
                    v.find_hit = None;
                    v.find_count = 0;
                }),
                "enter" => shell.find_step(if m.shift { -1 } else { 1 }),
                "backspace" => {
                    shell.view(|v| {
                        if let Some(q) = v.find.as_mut() {
                            q.pop();
                        }
                        v.find_at = 0;
                    });
                    shell.find_step(0);
                }
                key if !m.control => {
                    let text = ks
                        .key_char
                        .clone()
                        .or_else(|| (key == "space").then(|| " ".to_string()));
                    if let Some(text) = text {
                        shell.view(|v| {
                            if let Some(q) = v.find.as_mut() {
                                q.push_str(&text);
                            }
                            v.find_at = 0;
                        });
                        shell.find_step(0);
                    }
                }
                _ => {}
            }
            cx.stop_propagation();
            return None;
        }
        // Paste: Cmd-V on macOS, Ctrl-Shift-V elsewhere.
        let paste = (m.platform && ks.key == "v") || (m.control && m.shift && ks.key == "v");
        if paste {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                shell.paste(&text);
            }
            cx.stop_propagation();
            return None;
        }
        let copy = (m.platform && ks.key == "c") || (m.control && m.shift && ks.key == "c");
        if copy {
            let selection = shell.selection_text();
            let found = selection.is_some();
            cx.write_to_clipboard(ClipboardItem::new_string(
                selection.unwrap_or_else(|| shell.text()),
            ));
            cx.stop_propagation();
            return Some(found);
        }
        if m.platform {
            return None;
        }
        let app_cursor = shell.with_screen(|s| s.application_cursor());
        if let Some(bytes) = pty::key_bytes(
            &ks.key,
            ks.key_char.as_deref(),
            m.control,
            m.alt,
            m.shift,
            app_cursor,
        ) {
            shell.write(&bytes);
            cx.stop_propagation();
        }
        None
    }

    pub(crate) fn shell_tab_button(&self, shell: &pty::Shell, cx: &mut Context<Self>) -> Div {
        let id = shell.id;
        let selected = self.shell_tab == Some(id) && !self.cli_tab;
        div()
            .h(px(22.))
            .pl_2()
            .pr_1()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .when(selected, |el| {
                el.bg(tint(c_active())).text_color(rgb(c_text()))
            })
            .when(!selected, |el| el.text_color(rgb(c_muted())))
            .child(
                div()
                    .id(SharedString::from(format!("shell-tab-{id}")))
                    .cursor_pointer()
                    .when(!selected, |el| el.hover(|el| el.text_color(rgb(c_text()))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.shell_tab = Some(id);
                        this.cli_tab = false;
                        window.focus(&this.term_focus, cx);
                        cx.notify();
                    }))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(icon(
                        IconName::Terminal,
                        12.,
                        if shell.exited() { c_muted() } else { c_green() },
                    ))
                    .child(if shell.exited() {
                        format!("{} (exited)", shell.title)
                    } else {
                        shell.title.clone()
                    }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("shell-close-{id}")))
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(c_muted()))
                    .hover(|el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                    .on_click(cx.listener(move |this, _, _, cx| this.close_shell(id, cx)))
                    .child(Icon::new(IconName::X).size(px(11.))),
            )
    }

    /// The live screen of a shell, drawn cell run by cell run.
    pub(crate) fn shell_view(
        &self,
        shell: &pty::Shell,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (rows, cols) = self.term_grid();
        self.pty_view(shell, (rows, cols), None, window, cx)
    }

    /// A PTY screen. `cli` is the session whose agent CLI this is, or `None` for a shell
    /// tab in the terminal panel.
    pub(crate) fn pty_view(
        &self,
        shell: &pty::Shell,
        (rows, cols): (u16, u16),
        cli: Option<u64>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        shell.resize(rows, cols);
        shell.take_dirty();
        let dark = is_dark();
        let focus = if cli.is_some() {
            &self.cli_focus
        } else {
            &self.term_focus
        };
        let focused = focus.is_focused(window);
        let default_fg = c_text();
        let color = |c: vt100::Color, default: u32| match c {
            vt100::Color::Default => default,
            vt100::Color::Idx(i) => pty::palette(i, dark),
            vt100::Color::Rgb(r, g, b) => ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
        };
        let view = shell.view(|v| v.clone());
        let query = view.find.clone().filter(|q| !q.is_empty());
        let selection_bg = rgb(c_blue()).alpha(0.35);
        let match_bg = rgb(c_amber()).alpha(0.35);
        let lines = shell.with_screen(|screen| {
            let mut lines = div().v_flex();
            let (rows, cols) = screen.size();
            let scrolled = screen.scrollback();
            let (cur_row, cur_col) = screen.cursor_position();
            let show_cursor = !screen.hide_cursor() && scrolled == 0 && !shell.exited();
            for row in 0..rows {
                let back = scrolled + (rows - 1 - row) as usize;
                // One character per column, so links and matches map onto cells.
                let text: Vec<String> = (0..cols)
                    .map(|col| match screen.cell(row, col) {
                        Some(cell) if !cell.is_wide_continuation() => {
                            cell.contents().chars().next().unwrap_or(' ').to_string()
                        }
                        _ => " ".to_string(),
                    })
                    .collect();
                let text = text.concat();
                let links = pty::links(&text);
                let found = query
                    .as_deref()
                    .map(|q| pty::find_in(std::slice::from_ref(&text), q))
                    .unwrap_or_default();
                let mut line = div().h(px(term_line())).flex().whitespace_nowrap();
                // Runs of cells that share a style become one text span.
                let mut run = String::new();
                type Style = (Rgba, Option<Rgba>, bool, bool, bool, bool, bool);
                let mut style: Option<Style> = None;
                let flush = |line: Div, run: &mut String, style: Option<Style>| {
                    let Some((fg, bg, bold, italic, underline, strike, cursor)) = style else {
                        return line;
                    };
                    if run.is_empty() {
                        return line;
                    }
                    let text = std::mem::take(run);
                    line.child(
                        div()
                            .flex_none()
                            .text_color(fg)
                            .when_some(bg, |el, bg| el.bg(bg))
                            .when(bold, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .when(italic, |el| el.italic())
                            .when(underline, |el| el.underline())
                            .when(strike, |el| el.line_through())
                            .when(cursor && !focused, |el| {
                                el.border_1().border_color(rgb(c_secondary()))
                            })
                            .child(text),
                    )
                };
                let mut col = 0;
                while col < cols {
                    let Some(cell) = screen.cell(row, col) else {
                        break;
                    };
                    if cell.is_wide_continuation() {
                        col += 1;
                        continue;
                    }
                    let mut fg = color(cell.fgcolor(), default_fg);
                    let mut bg = match cell.bgcolor() {
                        vt100::Color::Default => None,
                        c => Some(color(c, 0)),
                    };
                    if cell.inverse() {
                        let back = bg.unwrap_or(c_term_bg());
                        bg = Some(fg);
                        fg = back;
                    }
                    let cursor = show_cursor && row == cur_row && col == cur_col;
                    if cursor && focused {
                        let back = bg.unwrap_or(c_term_bg());
                        bg = Some(fg);
                        fg = back;
                    }
                    let c = col as usize;
                    let current = view
                        .find_hit
                        .is_some_and(|(b, s, e)| b == back && (s..e).contains(&c));
                    let mut fg = rgb(fg);
                    if cell.dim() {
                        fg = fg.alpha(0.6);
                    }
                    let mut bg = bg.map(rgb);
                    if current {
                        bg = Some(rgb(c_amber()));
                        fg = rgb(0x1a1a1a);
                    } else if found.iter().any(|&(_, s, e)| (s..e).contains(&c)) {
                        bg = Some(match_bg);
                    } else if view.selected(back, col) {
                        bg = Some(selection_bg);
                    }
                    let link = links.iter().any(|(s, e, _)| (*s..*e).contains(&c));
                    let next = Some((
                        fg,
                        bg,
                        cell.bold(),
                        cell.italic(),
                        cell.underline() || link,
                        cell.strikethrough(),
                        cursor,
                    ));
                    if next != style || cursor {
                        line = flush(line, &mut run, style);
                        style = next;
                    }
                    let text = cell.contents();
                    run.push_str(if text.is_empty() { " " } else { &text });
                    if cursor {
                        line = flush(line, &mut run, style);
                        style = None;
                    }
                    col += 1;
                }
                line = flush(line, &mut run, style);
                lines = lines.child(line);
            }
            lines
        });
        let scrolled = shell.with_screen(|s| s.scrollback());
        let shared = shell.view.clone();
        let cell_size = (self.cell_w, term_line());
        // Records where the grid is drawn, to turn mouse positions into cells.
        let measure = canvas(
            move |b, _, _| {
                let mut v = shared.lock().unwrap_or_else(|e| e.into_inner());
                v.origin = (f32::from(b.origin.x), f32::from(b.origin.y));
                v.cell = cell_size;
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let lines = div().relative().child(measure).child(lines);
        let find_bar = view.find.as_ref().map(|q| {
            let count = match (view.find_count, q.is_empty()) {
                (_, true) => "Type to find".to_string(),
                (0, false) => "No matches".to_string(),
                (n, false) => format!("{} of {n}", view.find_at + 1),
            };
            div()
                .absolute()
                .top_2()
                .right_3()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .bg(float(c_raised()))
                .border_1()
                .border_color(rgb(c_border()))
                .text_size(px(12.))
                .child(icon(IconName::Search, 12., c_muted()))
                .child(
                    div()
                        .min_w(px(80.))
                        .text_color(rgb(c_text()))
                        .child(format!("{q}▏")),
                )
                .child(div().text_color(rgb(c_muted())).child(count))
                .child(
                    div()
                        .text_color(rgb(c_muted()))
                        .text_size(px(11.))
                        .child("Enter: older · Shift-Enter: newer · Esc"),
                )
        });
        div()
            .id(SharedString::from(format!("shell-{}", shell.id)))
            .key_context("RunoTerminal")
            .track_focus(focus)
            .flex_1()
            .min_h_0()
            .px_3()
            .py_2()
            .overflow_hidden()
            .relative()
            .font_family(self.term_family(cx))
            .text_size(px(term_font()))
            .line_height(px(term_line()))
            .cursor_text()
            .on_key_down(
                cx.listener(move |this, ev: &KeyDownEvent, window, cx| match cli {
                    Some(id) => this.cli_key(id, ev, cx),
                    None => this.shell_key(ev, window, cx),
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    let focus = if cli.is_some() {
                        &this.cli_focus
                    } else {
                        &this.term_focus
                    };
                    window.focus(focus, cx);
                    if let Some(url) = this.term_mouse(cli, TermMouse::Down(ev), (rows, cols)) {
                        cx.open_url(&url);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if this
                    .term_mouse(cli, TermMouse::Move(ev), (rows, cols))
                    .is_some()
                {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseUpEvent, _, cx| {
                    this.term_mouse(cli, TermMouse::Up(ev), (rows, cols));
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.term_mouse(cli, TermMouse::Down(ev), (rows, cols));
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, ev: &MouseUpEvent, _, _| {
                    this.term_mouse(cli, TermMouse::Up(ev), (rows, cols));
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseUpEvent, _, _| {
                    this.term_mouse(cli, TermMouse::Up(ev), (rows, cols));
                }),
            )
            // A program that asked for the mouse gets right clicks too; otherwise they
            // paste or open the menu.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    let shell = match cli {
                        Some(id) => this.cli_runs.get(&id).map(|r| &r.shell),
                        None => this.active_shell(),
                    };
                    let Some(shell) = shell else {
                        return;
                    };
                    if reports_mouse(shell, ev.modifiers.shift) {
                        this.term_mouse(cli, TermMouse::Down(ev), (rows, cols));
                    } else if RIGHT_CLICK_PASTE.load(Ordering::Relaxed) {
                        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            shell.paste(&text);
                        }
                    } else if cli.is_none() {
                        this.ctx_menu = Some((ev.position, ctxmenu::CtxTarget::Terminal));
                        this.ctx_index = None;
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                let dy = f32::from(ev.delta.pixel_delta(px(term_line())).y);
                let lines = (dy / term_line()).round() as isize
                    * SCROLL_SPEED.load(Ordering::Relaxed) as isize;
                if lines != 0 {
                    let shell = match cli {
                        Some(id) => this.cli_runs.get(&id).map(|r| &r.shell),
                        None => this.active_shell(),
                    };
                    if let Some(shell) = shell {
                        wheel(shell, ev, lines, (rows, cols));
                    }
                    cx.notify();
                }
            }))
            .child(lines)
            .children(find_bar)
            .when(scrolled > 0 && view.find.is_none(), |el| {
                el.child(
                    div()
                        .absolute()
                        .top_2()
                        .right_3()
                        .px_2()
                        .py(px(1.))
                        .rounded_md()
                        .bg(tint(c_active()))
                        .text_color(rgb(c_secondary()))
                        .text_size(px(11.))
                        .child(format!("{scrolled} lines up")),
                )
            })
            .when(shell.exited(), |el| {
                el.child(div().text_color(rgb(c_muted())).child(if cli.is_some() {
                    "The CLI closed. Switching back to Chat."
                } else {
                    "Process exited. Close this tab or open a new shell."
                }))
            })
    }

    /// Handle the mouse over a PTY: report it to a program that asked for the mouse
    /// (Shift bypasses that), else select text. A Cmd/Ctrl-click on a link returns it.
    fn term_mouse(
        &mut self,
        cli: Option<u64>,
        event: TermMouse,
        (rows, cols): (u16, u16),
    ) -> Option<String> {
        let shell = match cli {
            Some(id) => self.cli_runs.get(&id).map(|r| &r.shell),
            None => self.active_shell(),
        }?;
        let (position, m) = match event {
            TermMouse::Down(e) => (e.position, e.modifiers),
            TermMouse::Move(e) => (e.position, e.modifiers),
            TermMouse::Up(e) => (e.position, e.modifiers),
        };
        let (row, col) = shell.view(|v| cell_at(v, position, (rows, cols)));
        let (mode, encoding, scrolled) = shell.with_screen(|s| {
            (
                s.mouse_protocol_mode(),
                s.mouse_protocol_encoding(),
                s.scrollback(),
            )
        });
        let mods = (m.shift, m.alt, m.control);
        if mode != vt100::MouseProtocolMode::None && !m.shift {
            let act = match event {
                TermMouse::Down(e) => button_code(e.button).map(pty::MouseAct::Press),
                TermMouse::Up(e) => button_code(e.button).map(pty::MouseAct::Release),
                TermMouse::Move(e) => {
                    Some(pty::MouseAct::Move(e.pressed_button.and_then(button_code)))
                }
            };
            let (row, col) = shell.view(|v| report_cell(v, position, (rows, cols)));
            if let Some(bytes) =
                act.and_then(|act| pty::mouse_report(mode, encoding, act, row, col, mods))
            {
                shell.write_raw(&bytes);
            }
            return None;
        }
        // Only the left button selects; the others mean something only to the program.
        let left = match event {
            TermMouse::Down(e) => e.button == MouseButton::Left,
            TermMouse::Up(e) => e.button == MouseButton::Left,
            TermMouse::Move(_) => true,
        };
        if !left {
            return None;
        }
        let back = scrolled + (rows.saturating_sub(1).saturating_sub(row)) as usize;
        let spot = pty::Spot { back, col };
        match event {
            TermMouse::Down(e) => {
                let text = shell.with_screen(|s| s.rows(0, cols).nth(row as usize));
                let text = text.unwrap_or_default();
                if m.platform || m.control {
                    let c = col as usize;
                    if let Some((_, _, url)) = pty::links(&text)
                        .into_iter()
                        .find(|(s, e, _)| (*s..*e).contains(&c))
                    {
                        return Some(url);
                    }
                }
                shell.view(|v| {
                    v.selecting = true;
                    v.selection = Some(match e.click_count {
                        1 => (spot, spot),
                        2 => {
                            let (s, e) = word_at(&text, col as usize);
                            (
                                pty::Spot {
                                    back,
                                    col: s as u16,
                                },
                                pty::Spot {
                                    back,
                                    col: e as u16,
                                },
                            )
                        }
                        _ => (pty::Spot { back, col: 0 }, pty::Spot { back, col: cols }),
                    });
                });
                None
            }
            TermMouse::Move(e) => {
                let held = e.pressed_button == Some(MouseButton::Left);
                shell
                    .view(|v| {
                        if !(v.selecting && held) {
                            return false;
                        }
                        if let Some((start, _)) = v.selection {
                            v.selection = Some((start, spot));
                        }
                        true
                    })
                    .then(String::new)
            }
            TermMouse::Up(_) => {
                shell.view(|v| v.selecting = false);
                None
            }
        }
    }
}

/// A mouse event over a PTY view.
#[derive(Clone, Copy)]
enum TermMouse<'a> {
    Down(&'a MouseDownEvent),
    Move(&'a MouseMoveEvent),
    Up(&'a MouseUpEvent),
}

/// The cell under a window position, clamped to the grid.
fn cell_at(v: &pty::ViewState, position: Point<Pixels>, (rows, cols): (u16, u16)) -> (u16, u16) {
    let (w, h) = (v.cell.0.max(1.), v.cell.1.max(1.));
    let x = (f32::from(position.x) - v.origin.0) / w;
    let y = (f32::from(position.y) - v.origin.1) / h;
    let clamp = |n: f32, max: u16| (n.max(0.) as u16).min(max.saturating_sub(1));
    (clamp(y, rows), clamp(x.round(), cols + 1))
}

/// The cell a mouse report names: the one under the pointer, inside the grid.
fn report_cell(
    v: &pty::ViewState,
    position: Point<Pixels>,
    (rows, cols): (u16, u16),
) -> (u16, u16) {
    let (w, h) = (v.cell.0.max(1.), v.cell.1.max(1.));
    let x = (f32::from(position.x) - v.origin.0) / w;
    let y = (f32::from(position.y) - v.origin.1) / h;
    let clamp = |n: f32, max: u16| (n.max(0.) as u16).min(max.saturating_sub(1));
    (clamp(y, rows), clamp(x, cols))
}

/// The button number a mouse report uses, for the buttons a terminal reports.
fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    }
}

/// Whether a program in the PTY gets the mouse: it asked for it and Shift isn't held.
fn reports_mouse(shell: &pty::Shell, shift: bool) -> bool {
    !shift && shell.with_screen(|s| s.mouse_protocol_mode()) != vt100::MouseProtocolMode::None
}

/// The word around character `col`, as `[start, end)`.
fn word_at(text: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let word = |c: char| !c.is_whitespace() && !"\"'`()[]{}<>|,;".contains(c);
    if !chars.get(col).is_some_and(|&c| word(c)) {
        return (col, col + 1);
    }
    let mut start = col;
    while start > 0 && word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && word(chars[end]) {
        end += 1;
    }
    (start, end)
}

/// Scroll a PTY, or pass the wheel on: as mouse reports to a program that asked for the
/// mouse, or as arrow keys to a full-screen program without scrollback.
fn wheel(shell: &pty::Shell, ev: &ScrollWheelEvent, lines: isize, (rows, cols): (u16, u16)) {
    let (mode, encoding, alternate, app_cursor) = shell.with_screen(|s| {
        (
            s.mouse_protocol_mode(),
            s.mouse_protocol_encoding(),
            s.alternate_screen(),
            s.application_cursor(),
        )
    });
    let m = ev.modifiers;
    if mode != vt100::MouseProtocolMode::None && !m.shift {
        let (row, col) = shell.view(|v| report_cell(v, ev.position, (rows, cols)));
        let act = pty::MouseAct::Wheel(lines > 0);
        for _ in 0..lines.unsigned_abs().min(10) {
            if let Some(bytes) =
                pty::mouse_report(mode, encoding, act, row, col, (false, m.alt, m.control))
            {
                shell.write_raw(&bytes);
            }
        }
    } else if alternate {
        let key = if lines > 0 { "up" } else { "down" };
        if let Some(bytes) = pty::key_bytes(key, None, false, false, false, app_cursor) {
            for _ in 0..lines.unsigned_abs().min(10) {
                shell.write_raw(&bytes);
            }
        }
    } else {
        shell.scroll_by(lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn words_and_cells_under_the_mouse() {
        assert_eq!(word_at("cargo test --release", 8), (6, 10));
        assert_eq!(word_at("a  b", 1), (1, 2));
        let v = pty::ViewState {
            origin: (10., 20.),
            cell: (8., 16.),
            ..Default::default()
        };
        assert_eq!(cell_at(&v, point(px(10.), px(20.)), (24, 80)), (0, 0));
        assert_eq!(cell_at(&v, point(px(51.), px(53.)), (24, 80)), (2, 5));
        assert_eq!(cell_at(&v, point(px(9000.), px(9000.)), (24, 80)), (23, 80));
        // Reports name the cell under the pointer and stay inside the grid.
        assert_eq!(report_cell(&v, point(px(17.), px(20.)), (24, 80)), (0, 0));
        assert_eq!(report_cell(&v, point(px(51.), px(53.)), (24, 80)), (2, 5));
        assert_eq!(
            report_cell(&v, point(px(9000.), px(9000.)), (24, 80)),
            (23, 79)
        );
        assert_eq!(report_cell(&v, point(px(0.), px(0.)), (24, 80)), (0, 0));
    }

    #[test]
    fn mouse_buttons_map_to_report_codes() {
        assert_eq!(button_code(MouseButton::Left), Some(0));
        assert_eq!(button_code(MouseButton::Middle), Some(1));
        assert_eq!(button_code(MouseButton::Right), Some(2));
        assert_eq!(
            button_code(MouseButton::Navigate(NavigationDirection::Back)),
            None
        );
        // A right-button drag and release in SGR, and a middle click in the default encoding.
        use vt100::{MouseProtocolEncoding as E, MouseProtocolMode as M};
        let none = (false, false, false);
        assert_eq!(
            pty::mouse_report(
                M::ButtonMotion,
                E::Sgr,
                pty::MouseAct::Move(Some(2)),
                0,
                0,
                none
            ),
            Some(b"\x1b[<34;1;1M".to_vec())
        );
        assert_eq!(
            pty::mouse_report(
                M::PressRelease,
                E::Sgr,
                pty::MouseAct::Release(2),
                1,
                2,
                none
            ),
            Some(b"\x1b[<2;3;2m".to_vec())
        );
        assert_eq!(
            pty::mouse_report(
                M::PressRelease,
                E::Default,
                pty::MouseAct::Press(1),
                0,
                0,
                none
            ),
            Some(vec![0x1b, b'[', b'M', 33, 33, 33])
        );
        assert_eq!(
            pty::mouse_report(
                M::PressRelease,
                E::Sgr,
                pty::MouseAct::Press(0),
                0,
                0,
                (true, false, true)
            ),
            Some(b"\x1b[<20;1;1M".to_vec())
        );
    }
}
