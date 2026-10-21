//! What Crew adds to the screen: its row in the sidebar, the cards in its chat, the
//! routing chip over its composer, the strip at the top of a thread, the Threads panel and
//! the Crew / Direct switch. The logic is in `crew_ui.rs`.

use super::*;
use crate::crew::{self, Action, Card, Chip, Mark};
use crate::crew_ui::thread_status;

/// Crew's mark wherever it shows.
pub(crate) const CREW_ICON: IconName = IconName::Workflow;

fn status_tone(status: &str) -> u32 {
    match status {
        "working" => c_blue(),
        "needs you" => c_amber(),
        "failed" => c_red(),
        "done" => c_green(),
        _ => c_muted(),
    }
}

fn small(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(c_muted()))
        .child(text.into())
}

impl RunoApp {
    fn thread_working(&self, id: u64) -> bool {
        self.handles.contains_key(&Target::Session(id))
    }

    /// The pinned Crew row at the top of a Crew project's sessions.
    pub(crate) fn crew_row(&self, pid: u64, cx: &mut Context<Self>) -> AnyElement {
        let chat = self.crew_chat(pid);
        let on = chat.is_some_and(|c| self.selected == Some(c.id) && self.view == View::Session);
        let working = chat.is_some_and(|c| self.thread_working(c.id));
        let unread = chat.is_some_and(|c| c.unread);
        let open = self.crew_threads(pid, false).len();
        div()
            .id(("crew-row", pid as usize))
            .relative()
            .h(px(30.))
            .px_2()
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded_md()
            .cursor_pointer()
            .when(on, |el| el.bg(tint(c_active())))
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_crew(pid, cx);
                this.right_tab = RightTab::Threads;
                cx.notify();
            }))
            .child(icon(CREW_ICON, 13., c_blue()))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Crew"),
            )
            .when(working, |el| {
                el.child(self.status_dot(Status::Working, ("crew-dot", pid as usize), 7.))
            })
            .when(unread && !working, |el| {
                el.child(div().size(px(7.)).rounded_full().bg(rgb(c_blue())))
            })
            .when(open > 0, |el| el.child(small(format!("{open}"))))
            .into_any_element()
    }

    /// Crew or Direct for project `pid`, as a two-way switch.
    pub(crate) fn crew_mode_switch(&self, pid: u64, cx: &mut Context<Self>) -> AnyElement {
        let on = self.crew_mode(pid);
        let choice = |id: &'static str, label: &'static str, crew: bool| {
            shell::seg_choice(
                (id, pid as usize),
                format!("mode-{pid}-{}", crew == on),
                crew == on,
            )
            .flex_1()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_project_mode(pid, crew);
                cx.notify();
            }))
        };
        shell::seg_track()
            .child(choice("mode-crew", "Crew", true))
            .child(choice("mode-direct", "Direct", false))
            .into_any_element()
    }

    /// One thread as a compact live card: where it stands, its checklist and its last word.
    pub(crate) fn thread_card(&self, thread: u64, key: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(s) = self.state.session(thread) else {
            return div()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(divider())
                .child(small("This thread was deleted."))
                .into_any_element();
        };
        let status = thread_status(self.thread_working(thread), s);
        let info = s.crew.clone().unwrap_or_default();
        let done = info
            .checklist
            .iter()
            .filter(|st| st.mark == Mark::Done)
            .count();
        let now = info
            .checklist
            .iter()
            .find(|st| st.mark == Mark::Doing)
            .map(|st| st.text.clone());
        let last = info
            .decision
            .as_ref()
            .map(|d| d.question.clone())
            .or_else(|| info.reports.last().map(|r| r.summary.clone()))
            .or(now);
        let provider = agent::provider(&s.provider);
        let id = SharedString::from(format!("thread-card-{key}-{thread}"));
        div()
            .id(id)
            .px_3()
            .py_2()
            .v_flex()
            .gap_1()
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .bg(tint(c_subtle()))
            .cursor_pointer()
            .hover(|el| el.border_color(rgb(c_border())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(thread);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(status_tone(status))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(s.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(provider.color))
                            .child(provider.mark),
                    )
                    .child(small(status)),
            )
            .when(!info.checklist.is_empty(), |el| {
                el.child(small(format!("{done} of {} steps", info.checklist.len())))
            })
            .children(last.map(|text| {
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .line_clamp(2)
                    .child(text)
            }))
            .into_any_element()
    }

    /// A card drawn with message `index` of Crew's chat `chat`.
    pub(crate) fn crew_card(
        &self,
        chat: u64,
        index: usize,
        message: &Message,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let card = message.crew.as_ref()?;
        let line = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.5))
            .text_color(rgb(c_secondary()))
            .child(icon(CREW_ICON, 12., c_muted()))
            .child(message.text.clone());
        let body = match card {
            Card::Started { threads } => div().v_flex().gap_2().child(line).children(
                threads
                    .iter()
                    .map(|t| self.thread_card(*t, &format!("m{index}"), cx)),
            ),
            Card::News { thread } => {
                let thread = *thread;
                div().v_flex().gap_2().child(line).child(self.thread_card(
                    thread,
                    &format!("m{index}"),
                    cx,
                ))
            }
            Card::Held { actions, answered } => {
                let titles: Vec<String> = actions
                    .iter()
                    .filter_map(|a| match a {
                        Action::StartThread { title, .. } => Some(title.clone()),
                        _ => None,
                    })
                    .collect();
                let answered = *answered;
                div()
                    .px_3()
                    .py_2()
                    .v_flex()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(if answered { c_border() } else { c_amber() }))
                    .child(line)
                    .children(titles.into_iter().map(|t| {
                        div()
                            .flex()
                            .gap_2()
                            .text_size(px(12.5))
                            .child(icon(IconName::Plus, 12., c_muted()))
                            .child(t)
                    }))
                    .when(!answered, |el| {
                        el.child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    self.button(("crew-held-start", index), "Start them", true)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.start_held(chat, index, cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    self.button(("crew-held-drop", index), "Not now", false)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.drop_held(chat, index);
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
            }
        };
        Some(body.into_any_element())
    }

    /// Over Crew's composer: where the next message goes.
    pub(crate) fn crew_route_bar(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if session.canvas != crew::CANVAS {
            return None;
        }
        let pid = session.project_id;
        let current = self.crew.chips.get(&pid).copied().unwrap_or_default();
        let mut track = shell::seg_track();
        for (i, chip) in Chip::ALL.into_iter().enumerate() {
            let on = chip == current;
            track = track.child(
                shell::seg_choice(("crew-chip", i), format!("crew-chip-{pid}-{on}-{i}"), on)
                    .child(chip.label())
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(chip.detail()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.crew.chips.insert(pid, chip);
                        cx.notify();
                    })),
            );
        }
        Some(
            div()
                .w_full()
                .when_some(cx.global::<AppSettings>().content_max_width(), |el, w| {
                    el.max_w(px(w))
                })
                .mx_auto()
                .px_8()
                .pt_2()
                .flex()
                .items_center()
                .gap_2()
                .child(track)
                .child(small(
                    "Type @ and a thread's name to send it straight there.",
                ))
                .into_any_element(),
        )
    }

    /// At the top of a thread: who started it and why, its checklist, and its open decision.
    pub(crate) fn thread_strip(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let info = session.crew.as_ref()?;
        let thread = session.id;
        let pid = session.project_id;
        let decision = info.decision.clone();
        Some(
            div()
                .p_3()
                .v_flex()
                .gap_2()
                .rounded_xl()
                .border_1()
                .border_color(divider())
                .bg(tint(c_subtle()))
                .child(
                    div()
                        .id("thread-from")
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .cursor_pointer()
                        .hover(|el| el.text_color(rgb(c_text())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_crew(pid, cx);
                            cx.notify();
                        }))
                        .child(icon(CREW_ICON, 12., c_blue()))
                        .child(match info.origin {
                            crew::Origin::Crew { .. } => "Started by Crew. Back to Crew",
                            crew::Origin::Person => "A Crew thread. Back to Crew",
                        }),
                )
                .when(!info.brief.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .line_clamp(3)
                            .child(info.brief.clone()),
                    )
                })
                .when(!info.checklist.is_empty(), |el| {
                    el.child(
                        div()
                            .v_flex()
                            .gap_1()
                            .children(info.checklist.iter().map(|st| {
                                let (glyph, tone) = match st.mark {
                                    Mark::Done => (IconName::CircleCheck, c_green()),
                                    Mark::Doing => (IconName::CircleDot, c_blue()),
                                    Mark::Todo => (IconName::Circle, c_muted()),
                                };
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(12.5))
                                    .child(icon(glyph, 12., tone))
                                    .child(st.text.clone())
                            })),
                    )
                })
                .children(decision.map(|d| {
                    div()
                        .v_flex()
                        .gap_2()
                        .pt_1()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(d.question.clone()),
                        )
                        .children(d.options.iter().enumerate().map(|(i, o)| {
                            let recommended = i == d.recommended;
                            div()
                                .id(("thread-choice", i))
                                .px_3()
                                .py_2()
                                .v_flex()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(if recommended { c_blue() } else { c_border() }))
                                .cursor_pointer()
                                .hover(|el| el.bg(tint(c_active())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer_decision(thread, i, cx);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(if recommended {
                                            format!("{}  (recommended)", o.label)
                                        } else {
                                            o.label.clone()
                                        }),
                                )
                                .when(!o.consequence.is_empty(), |el| {
                                    el.child(small(o.consequence.clone()))
                                })
                        }))
                }))
                .into_any_element(),
        )
    }

    /// The right panel's Threads tab beside Crew: the project's threads by who acts next.
    pub(crate) fn threads_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(pid) = self.current().map(|s| s.project_id) else {
            return div().into_any_element();
        };
        let threads = self.crew_threads(pid, false);
        if threads.is_empty() {
            return div()
                .p_4()
                .child(small(
                    "No threads yet. When Crew starts one for your work, it shows here.",
                ))
                .into_any_element();
        }
        let groups = [
            ("Needs you", "needs you"),
            ("Working", "working"),
            ("Done", "done"),
            ("Other", ""),
        ];
        let mut list = div().v_flex().gap_3().p_3();
        let mut shown: Vec<u64> = Vec::new();
        for (title, status) in groups {
            let members: Vec<u64> = threads
                .iter()
                .filter(|t| !shown.contains(&t.id))
                .filter(|t| {
                    status.is_empty() || thread_status(self.thread_working(t.id), t) == status
                })
                .map(|t| t.id)
                .collect();
            if members.is_empty() {
                continue;
            }
            shown.extend(&members);
            list = list.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
                            .child(format!("{} {}", title.to_uppercase(), members.len())),
                    )
                    .children(
                        members
                            .into_iter()
                            .map(|t| self.thread_card(t, "panel", cx)),
                    ),
            );
        }
        div()
            .id("threads-tab")
            .size_full()
            .overflow_y_scroll()
            .child(list)
            .into_any_element()
    }
}
