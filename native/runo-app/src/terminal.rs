//! Shell tabs in the terminal panel: real PTYs rendered from a VT100 screen.

use super::*;

/// Text metrics of the terminal grid.
pub(crate) const TERM_FONT: f32 = 12.;
pub(crate) const TERM_LINE: f32 = 16.;

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
        self.next_shell += 1;
        let id = self.next_shell;
        let tx = self.tx.clone();
        let (rows, cols) = self.term_grid();
        match pty::Shell::spawn(id, session, &dir, rows, cols, move |id| {
            let _ = tx.send(Event::Pty(id));
        }) {
            Ok(mut shell) => {
                let n = self.session_shells(session).count() + 1;
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
    }

    /// Rows and columns that fit the terminal body.
    pub(crate) fn term_grid(&self) -> (u16, u16) {
        let height = self.term_height() - 32. - 16.;
        let width = 460. - 24.;
        let rows = (height / TERM_LINE).floor().max(2.) as u16;
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
    pub(crate) fn measure_cell(&mut self, window: &mut Window) {
        if self.cell_measured {
            return;
        }
        self.cell_measured = true;
        let ts = window.text_system();
        let id = ts.resolve_font(&font(self.mono_font.clone()));
        if let Ok(size) = ts.advance(id, px(TERM_FONT), 'm') {
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
        if self.pty_key(shell, event, cx) {
            self.info = Some((
                "Copied the terminal screen.".into(),
                std::time::Instant::now(),
            ));
            cx.notify();
        }
        let _ = window;
    }

    /// Send a keystroke to a PTY program, handling copy and paste. Returns whether it
    /// changed something the UI shows.
    pub(crate) fn pty_key(
        &self,
        shell: &pty::Shell,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let ks = &event.keystroke;
        let m = ks.modifiers;
        // Paste: Cmd-V on macOS, Ctrl-Shift-V elsewhere.
        let paste = (m.platform && ks.key == "v") || (m.control && m.shift && ks.key == "v");
        if paste {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                shell.paste(&text);
            }
            cx.stop_propagation();
            return false;
        }
        let copy = (m.platform && ks.key == "c") || (m.control && m.shift && ks.key == "c");
        if copy {
            cx.write_to_clipboard(ClipboardItem::new_string(shell.text()));
            cx.stop_propagation();
            return true;
        }
        if m.platform {
            return false;
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
        false
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
                el.bg(rgb(c_active())).text_color(rgb(c_text()))
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
                    .hover(|el| el.bg(rgb(c_active())).text_color(rgb(c_text())))
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
        let lines = shell.with_screen(|screen| {
            let mut lines = div().v_flex();
            let (rows, cols) = screen.size();
            let (cur_row, cur_col) = screen.cursor_position();
            let show_cursor = !screen.hide_cursor() && screen.scrollback() == 0 && !shell.exited();
            for row in 0..rows {
                let mut line = div().h(px(TERM_LINE)).flex().whitespace_nowrap();
                // Runs of cells that share a style become one text span.
                let mut run = String::new();
                let mut style: Option<(u32, Option<u32>, bool, bool, bool)> = None;
                let flush =
                    |line: Div,
                     run: &mut String,
                     style: Option<(u32, Option<u32>, bool, bool, bool)>| {
                        let Some((fg, bg, bold, underline, cursor)) = style else {
                            return line;
                        };
                        if run.is_empty() {
                            return line;
                        }
                        let text = std::mem::take(run);
                        line.child(
                            div()
                                .flex_none()
                                .text_color(rgb(fg))
                                .when_some(bg, |el, bg| el.bg(rgb(bg)))
                                .when(bold, |el| el.font_weight(FontWeight::BOLD))
                                .when(underline, |el| el.underline())
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
                    let next = Some((fg, bg, cell.bold(), cell.underline(), cursor));
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
            .font_family(self.mono_font.clone())
            .text_size(px(TERM_FONT))
            .line_height(px(TERM_LINE))
            .cursor_text()
            .on_key_down(
                cx.listener(move |this, ev: &KeyDownEvent, window, cx| match cli {
                    Some(id) => this.cli_key(id, ev, cx),
                    None => this.shell_key(ev, window, cx),
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    let focus = if cli.is_some() {
                        &this.cli_focus
                    } else {
                        &this.term_focus
                    };
                    window.focus(focus, cx);
                    cx.notify();
                }),
            )
            .when(cli.is_none(), |el| {
                el.on_mouse_down(
                    MouseButton::Right,
                    self.on_context(ctxmenu::CtxTarget::Terminal, cx),
                )
            })
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                let dy = f32::from(ev.delta.pixel_delta(px(TERM_LINE)).y);
                let lines = (dy / TERM_LINE).round() as isize;
                if lines != 0 {
                    let shell = match cli {
                        Some(id) => this.cli_runs.get(&id).map(|r| &r.shell),
                        None => this.active_shell(),
                    };
                    if let Some(shell) = shell {
                        shell.scroll_by(lines);
                    }
                    cx.notify();
                }
            }))
            .child(lines)
            .when(scrolled > 0, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_2()
                        .right_3()
                        .px_2()
                        .py(px(1.))
                        .rounded_md()
                        .bg(rgb(c_active()))
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
}
