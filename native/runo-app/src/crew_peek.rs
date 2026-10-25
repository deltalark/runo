//! A Crew thread open in the Threads tab, beside Crew's chat: its checklist and decision,
//! its conversation in a compact form, and a composer that writes to it. Crew's chat stays
//! where it was; "Open in main" moves the thread to the main view.

use super::*;
use crate::chat_list::ChatRow;
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

    /// Open `thread` from a "#34" in Crew's chat: beside the chat as its cards do, or in
    /// the main view when `main` (a ⌘-click) or when the chat isn't open.
    pub(crate) fn open_thread_ref(&mut self, thread: u64, main: bool) {
        if !main && self.current().is_some_and(|s| s.canvas == crew::CANVAS) {
            self.open_peek(thread);
        } else {
            self.close_peek();
            self.select(thread);
        }
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

        let mut rows = Vec::with_capacity(session.messages.len() + 2);
        if let Some(crew) = &session.crew {
            rows.push((ChatRow::Strip, chat_list::hash(&crew.decision)));
        }
        rows.extend(session.messages.iter().enumerate().map(|(i, m)| {
            (
                ChatRow::Message(i),
                (m.text.len() as u64) << 20 | m.steps.len() as u64,
            )
        }));
        if working {
            rows.push((ChatRow::Working, 0));
        } else if self
            .finished
            .get(&thread)
            .is_some_and(|(at, _)| at.elapsed() < working::FINISH)
        {
            rows.push((ChatRow::Finished, 0));
        }
        self.peek_rows.borrow_mut().update(
            &self.peek_list,
            thread,
            true,
            rows,
            chat_list::layout_epoch(cx),
        );
        let view = cx.entity().downgrade();
        let list = list(self.peek_list.clone(), move |ix, _, cx| {
            view.update(cx, |this, cx| this.peek_row(thread, ix, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
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

impl RunoApp {
    /// One message of a thread, compact: the person's words on the right, the agent's reply
    /// with how many steps it took, and anything else as a quiet line. Thread numbers in
    /// the reply link to their thread through `links`.
    fn peek_message(
        &self,
        thread: u64,
        index: usize,
        message: &Message,
        links: Option<&crate::thread_refs::ThreadLinks>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                        el.child(div().text_size(px(12.5)).child(self.markdown_view(
                            thread,
                            index,
                            1,
                            &message.text,
                            links,
                            cx,
                        )))
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

    /// Row `ix` of the thread open in the Threads tab, as `peek_view` listed them last.
    fn peek_row(&self, thread: u64, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let (row, fresh, count) = {
            let rows = self.peek_rows.borrow();
            let Some((row, fresh)) = rows.row(ix) else {
                return div().into_any_element();
            };
            (row, fresh, rows.len())
        };
        let Some(session) = self.state.session(thread) else {
            return div().into_any_element();
        };
        chat_list::ROW_SETTLED.set(!fresh);
        let content = match row {
            ChatRow::Strip => self.thread_strip(session, true, cx),
            ChatRow::Message(i) => {
                let links = self.thread_links(cx);
                session.messages.get(i).map(|m| {
                    let text = self.peek_message(thread, i, m, links.as_ref(), cx);
                    if m.role != Role::User || m.images.is_empty() {
                        return text;
                    }
                    let mut images = div().flex().flex_wrap().justify_end().gap_1();
                    for (k, path) in m.images.iter().enumerate() {
                        images = images.child(
                            self.viewable(
                                SharedString::from(format!("peek-img-{thread}-{i}-{k}")),
                                thread,
                                crate::lightbox::Pick::Message(i, k),
                                img(std::path::PathBuf::from(path))
                                    .size(px(56.))
                                    .rounded_md()
                                    .object_fit(ObjectFit::Cover),
                                cx,
                            ),
                        );
                    }
                    div()
                        .v_flex()
                        .gap_3()
                        .child(images)
                        .child(text)
                        .into_any_element()
                })
            }
            ChatRow::Working => {
                let since = self
                    .turns
                    .get(&thread)
                    .map(|t| &t.started)
                    .map(|t| elapsed(t.elapsed()));
                Some(
                    div()
                        .text_size(px(12.5))
                        .child(working::indicator(working::doing(&session.messages), since))
                        .into_any_element(),
                )
            }
            ChatRow::Finished => self
                .finished_row(thread)
                .map(|done| div().text_size(px(12.5)).child(done).into_any_element()),
            _ => None,
        };
        chat_list::ROW_SETTLED.set(false);
        div()
            .px_3()
            .pt_3()
            .when(ix + 1 == count, |el| el.pb_3())
            .children(content)
            .into_any_element()
    }
}
