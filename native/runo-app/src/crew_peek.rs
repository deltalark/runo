//! A Crew thread open in the Threads tab, beside Crew's chat: its checklist and decision,
//! its conversation in a compact form, and a composer that writes to it. Crew's chat stays
//! where it was; "Open in main" moves the thread to the main view.

use super::*;
use crate::crew_ui::thread_status;

impl RunoApp {
    /// Open `thread` in the Threads tab.
    pub(crate) fn open_peek(&mut self, thread: u64) {
        if self.peek != Some(thread) {
            self.peek_jump_to_bottom();
        }
        self.peek = Some(thread);
        self.right_tab = RightTab::Threads;
        self.show_panel = true;
    }

    /// Back from the thread to the list of threads.
    fn close_peek(&mut self) {
        self.peek = None;
    }

    /// Send what is typed in the panel's composer to the open thread. A thread at work
    /// gets it when its turn ends.
    pub(crate) fn send_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.peek else {
            return;
        };
        let text = self.peek_prompt.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.peek_prompt
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.message_thread(thread, text, Vec::new(), cx);
        self.peek_jump_to_bottom();
        self.save();
    }

    /// The thread open in the Threads tab, or None once it is gone.
    pub(crate) fn peek_view(&self, thread: u64, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.state.session(thread)?;
        let working = self.handles.contains_key(&Target::Session(thread));
        let status = thread_status(working, session);
        let head = div()
            .flex_none()
            .h(px(38.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .id("peek-back")
                    .flex_none()
                    .px_1()
                    .py(px(2.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_peek();
                        cx.notify();
                    }))
                    .child(icon(IconName::ChevronLeft, 12., c_muted()))
                    .child("Threads"),
            )
            .child(self.thread_glyph(thread, status, "peek"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(crew::shown_title(&session.title)),
            )
            .child(
                div()
                    .id("peek-main")
                    .flex_none()
                    .px_2()
                    .py(px(3.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(tint(c_subtle()))
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(
                            "Open this thread in the main view",
                        )
                        .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_peek();
                        this.select(thread);
                        cx.notify();
                    }))
                    .child(icon(IconName::Maximize, 11., c_secondary()))
                    .child("Open in main"),
            );

        let mut list = div().v_flex().gap_3().px_3().py_3();
        if let Some(strip) = self.thread_strip(session, true, cx) {
            list = list.child(strip);
        }
        for (i, message) in session.messages.iter().enumerate() {
            list = list.child(peek_message(thread, i, message));
        }
        if working {
            let since = self
                .turns
                .get(&thread)
                .map(|t| &t.started)
                .map(|t| elapsed(t.elapsed()));
            list = list.child(div().text_size(px(12.5)).child(working_indicator(since)));
        }
        let queued = session.queue.len();

        let composer = div()
            .flex_none()
            .m_2()
            .px_2()
            .py_1()
            .v_flex()
            .gap_1()
            .rounded_xl()
            .bg(tint(c_raised()))
            .when(queued > 0, |el| {
                el.child(
                    div()
                        .px_1()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(if queued == 1 {
                            "1 message waits for this turn to end".to_string()
                        } else {
                            format!("{queued} messages wait for this turn to end")
                        }),
                )
            })
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().text_size(px(13.)).child(
                            Textarea::new(&self.peek_prompt)
                                .appearance(false)
                                .bordered(false)
                                .h(px(40.)),
                        ),
                    )
                    .child(
                        div()
                            .id("peek-send")
                            .flex_none()
                            .mb_1()
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .bg(rgb(c_blue()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.send_peek(window, cx);
                                cx.notify();
                            }))
                            .child(icon(IconName::ArrowUp, 13., c_canvas())),
                    ),
            );

        Some(
            div()
                .id(SharedString::from(format!("peek-{thread}")))
                .size_full()
                .v_flex()
                .child(head)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .v_flex()
                        .child(
                            div()
                                .id(SharedString::from(format!("peek-scroll-{thread}")))
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .track_scroll(&self.peek_scroll)
                                .on_scroll_wheel(cx.listener(
                                    |this, e: &ScrollWheelEvent, window, _| {
                                        this.peek_wheel(e, window)
                                    },
                                ))
                                .child(list),
                        )
                        .children(self.peek_messages_pill(session, cx)),
                )
                .child(composer)
                .into_any_element(),
        )
    }
}

/// One message of a thread, compact: the person's words on the right, the agent's reply
/// with how many steps it took, and anything else as a quiet line.
fn peek_message(thread: u64, index: usize, message: &Message) -> AnyElement {
    match message.role {
        Role::User => div()
            .flex()
            .justify_end()
            .child(
                div()
                    .max_w(relative(0.88))
                    .px_3()
                    .py(px(6.))
                    .rounded_xl()
                    .bg(tint(c_raised()))
                    .text_size(px(12.5))
                    .child(message.text.clone()),
            )
            .into_any_element(),
        Role::Agent => {
            let n = message.steps.len();
            div()
                .v_flex()
                .gap_1()
                .when(n > 0, |el| {
                    el.child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("{n} step{}", if n == 1 { "" } else { "s" })),
                    )
                })
                .when(!message.text.trim().is_empty(), |el| {
                    el.child(
                        div().text_size(px(12.5)).child(
                            TextView::markdown(
                                SharedString::from(format!("peek-body-{thread}-{index}")),
                                message.text.clone(),
                            )
                            .selectable(true),
                        ),
                    )
                })
                .into_any_element()
        }
        _ => div()
            .text_size(px(11.5))
            .text_color(rgb(c_muted()))
            .child(crew::plain(&message.text))
            .into_any_element(),
    }
}
