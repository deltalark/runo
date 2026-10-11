//! Split view: a second session beside the selected one, live, with its own reply box.

use super::*;

impl RunoApp {
    /// The session shown in the split pane, if it is open and not the selected one.
    pub(crate) fn split_session(&self) -> Option<&Session> {
        let id = self.split.filter(|id| Some(*id) != self.selected)?;
        self.state.session(id).filter(|s| !s.archived)
    }

    /// Open the split with the most relevant other session, or close it.
    pub(crate) fn toggle_split(&mut self, cx: &mut Context<Self>) {
        if self.split_session().is_some() {
            self.split = None;
        } else {
            self.split = self.split_candidates().first().copied();
            if self.split.is_none() {
                self.info = Some((
                    "Start another session to see two side by side.".into(),
                    std::time::Instant::now(),
                ));
            }
        }
        self.split_menu = false;
        self.split_scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Other sessions, open tabs first, then the most recently active.
    pub(crate) fn split_candidates(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .tabs
            .iter()
            .rev()
            .copied()
            .filter(|id| Some(*id) != self.selected)
            .collect();
        let mut rest: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| !s.archived && Some(s.id) != self.selected && !ids.contains(&s.id))
            .collect();
        rest.sort_by_key(|s| std::cmp::Reverse(s.id));
        ids.extend(rest.into_iter().map(|s| s.id));
        ids.retain(|id| self.state.session(*id).is_some_and(|s| !s.archived));
        ids
    }

    /// Make the split session the main one, and the main one the split.
    pub(crate) fn swap_split(&mut self, cx: &mut Context<Self>) {
        let Some(other) = self.split_session().map(|s| s.id) else {
            return;
        };
        let main = self.selected;
        self.select(other);
        self.split = main;
        self.split_scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Send the split pane's reply box to its session, queueing while it works.
    pub(crate) fn send_split(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(id) = self.split_session().map(|s| s.id) else {
            return;
        };
        let text = text.trim().to_string();
        if text.is_empty() || self.demo_guard(cx) {
            return;
        }
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
        let opts = self
            .state
            .session(id)
            .map(|s| cx.global::<AppSettings>().agent(&s.provider))
            .unwrap_or_default();
        self.start_turn(id, text, Vec::new(), &opts);
        self.split_scroll.scroll_to_bottom();
    }

    pub(crate) fn split_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.split_session()?;
        let id = session.id;
        let provider = agent::provider(&session.provider);
        let working = self.handles.contains_key(&Target::Session(id));
        let status = match self.started.get(&id).filter(|_| working) {
            Some(start) => format!("Working · {}", elapsed(start.elapsed())),
            None => Self::status_label(session.status).to_string(),
        };
        let header = div()
            .h(px(44.))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(c_border()))
            .child(
                div()
                    .id("split-pick")
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .flex_1()
                    .px_1()
                    .h(px(28.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.split_menu = !this.split_menu;
                        cx.notify();
                    }))
                    .child(self.status_dot(session.status, "split-dot", 6.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(session.title.clone()),
                    )
                    .child(icon(IconName::ChevronDown, 11., c_muted())),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(rgb(c_muted()))
                    .child(status),
            )
            .child(
                self.ghost_button_sized(
                    "split-swap",
                    IconName::ArrowLeft,
                    "Make this the main session",
                    26.,
                )
                .on_click(cx.listener(|this, _, _, cx| this.swap_split(cx))),
            )
            .child(
                self.ghost_button_sized("split-close", IconName::X, "Close split view", 26.)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.split = None;
                        cx.notify();
                    })),
            );
        let mut list = div().v_flex().gap_4().px_4().py_4();
        let skip = session.messages.len().saturating_sub(40);
        for (i, m) in session.messages.iter().enumerate().skip(skip) {
            let live = working && i + 1 == session.messages.len() && m.role == Role::Agent;
            list = list.child(match m.role {
                Role::User => div()
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .max_w(px(360.))
                            .px_3()
                            .py_2()
                            .rounded_xl()
                            .bg(rgb(c_active()))
                            .text_size(px(13.))
                            .child(m.text.clone()),
                    )
                    .into_any_element(),
                _ => {
                    let text = if m.text.trim().is_empty() && live {
                        "Working…".to_string()
                    } else {
                        m.text.clone()
                    };
                    div()
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .text_color(rgb(c_muted()))
                                .child(provider.name)
                                .when(!m.steps.is_empty(), |el| {
                                    let n = m.steps.len();
                                    el.child(format!("· {n} step{}", if n == 1 { "" } else { "s" }))
                                })
                                .when(!m.time.is_empty(), |el| el.child(format!("· {}", m.time))),
                        )
                        .child(
                            div().text_size(px(13.)).child(
                                TextView::markdown(
                                    SharedString::from(format!("split-{id}-{i}")),
                                    text,
                                )
                                .selectable(true),
                            ),
                        )
                        .into_any_element()
                }
            });
        }
        if session.messages.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(c_muted()))
                    .child("No messages yet."),
            );
        }
        let menu = self.split_menu.then(|| {
            let mut items = div().v_flex().p_1();
            for (n, other) in self.split_candidates().into_iter().take(12).enumerate() {
                let Some(s) = self.state.session(other) else {
                    continue;
                };
                items = items.child(
                    div()
                        .id(("split-item", n))
                        .px_2()
                        .py(px(5.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_size(px(13.))
                        .hover(|el| el.bg(rgb(c_active())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.split = Some(other);
                            this.split_menu = false;
                            this.split_scroll.scroll_to_bottom();
                            cx.notify();
                        }))
                        .child(self.status_dot(s.status, ("split-item-dot", n), 6.))
                        .child(div().flex_1().truncate().child(s.title.clone()))
                        .when(other == id, |el| {
                            el.child(icon(IconName::Check, 13., c_text()))
                        }),
                );
            }
            div()
                .absolute()
                .top(px(40.))
                .left_2()
                .w(px(300.))
                .max_h(px(360.))
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(rgb(c_raised()))
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.split_menu = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("split-menu-scroll")
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .child(items),
                )
        });
        Some(slide_in(
            div()
                .flex_1()
                .min_w(px(320.))
                .h_full()
                .relative()
                .v_flex()
                .border_l_1()
                .border_color(rgb(c_border()))
                .child(header)
                .child(
                    div()
                        .id("split-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.split_scroll)
                        .child(list),
                )
                .child(
                    div()
                        .p_3()
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .child(
                            div()
                                .px_3()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(c_border()))
                                .bg(rgb(c_subtle()))
                                .child(Input::new(&self.split_input).appearance(false).h(px(34.))),
                        ),
                )
                .children(menu.map(|m| rise(m, "split-menu-rise", 180))),
            SharedString::from(format!("split-pane-{id}")),
            260,
        ))
    }
}
