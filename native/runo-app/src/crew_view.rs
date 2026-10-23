//! What Crew adds to the screen: its row in the sidebar, the cards in its chat, the
//! routing chip over its composer, the strip at the top of a thread, the Threads panel and
//! the Crew / Direct switch. The logic is in `crew_ui.rs`.

use super::*;
use crate::crew::{self, Action, Card, Chip, Mark};
use crate::crew_ui::thread_status;

/// Crew's mark wherever it shows.
pub(crate) const CREW_ICON: IconName = IconName::Workflow;

pub(crate) fn status_tone(status: &str) -> u32 {
    match status {
        "working" => c_blue(),
        "needs you" => c_amber(),
        "failed" => c_red(),
        "done" => c_green(),
        _ => c_muted(),
    }
}

/// "now", "12m", "3h", "2d": how long ago, for a row's corner.
fn short_age(then: u64, now: u64) -> String {
    if then == 0 {
        return String::new();
    }
    match now.saturating_sub(then) {
        0..=59 => "now".into(),
        s @ 60..=3599 => format!("{}m", s / 60),
        s @ 3600..=86_399 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// How many finished threads the Threads tab lists before "Show all".
const DONE_SHOWN: usize = 5;

fn small(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(c_muted()))
        .child(text.into())
}

impl RunoApp {
    /// A thread's decision as one chip per answer, the recommended one filled, for the
    /// corner cards and the Inbox.
    pub(crate) fn decision_chips(
        &self,
        key: &'static str,
        n: usize,
        thread: u64,
        d: &crew::Decision,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        d.options
            .iter()
            .take(4)
            .enumerate()
            .map(|(i, o)| {
                self.chip((key, n * 10 + i), o.label.clone(), i == d.recommended)
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.answer_decision(thread, i, cx);
                        cx.notify();
                    }))
            })
            .collect()
    }

    fn thread_working(&self, id: u64) -> bool {
        self.handles.contains_key(&Target::Session(id))
    }

    /// The pinned Crew row at the top of a Crew project's sessions.
    pub(crate) fn crew_row(&self, pid: u64, cx: &mut Context<Self>) -> AnyElement {
        let chat = self.crew_chat(pid);
        let on = chat.is_some_and(|c| self.selected == Some(c.id) && self.view == View::Session);
        let working = chat.is_some_and(|c| self.thread_working(c.id));
        let unread = chat.is_some_and(|c| c.unread);
        let threads = self.crew_threads(pid, false);
        let open = threads.len();
        let waiting = threads
            .iter()
            .filter(|t| thread_status(self.thread_working(t.id), t) == "needs you")
            .count();
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
            .when(waiting > 0, |el| {
                el.child(
                    div()
                        .px(px(7.))
                        .rounded_full()
                        .bg(rgba((c_amber() << 8) | 0x28))
                        .text_size(px(10.5))
                        .text_color(rgb(c_amber()))
                        .child(format!("{waiting} needs you")),
                )
            })
            .when(open > 0 && waiting == 0, |el| {
                el.child(small(format!("{open}")))
            })
            .into_any_element()
    }

    /// Crew's open threads under its row in the sidebar, each on one line. Finished ones
    /// fold into a single "N done" row until it is opened. Selecting a thread only
    /// highlights it: rows keep their place.
    pub(crate) fn crew_children(
        &self,
        pid: u64,
        threads: &[&Session],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let unfolded = self.sidebar_done_open.contains(&pid);
        let mut open: Vec<AnyElement> = Vec::new();
        let mut finished: Vec<AnyElement> = Vec::new();
        // The open thread stays in view even once finished and folded: it shows just
        // under the "N done" row rather than jumping above it.
        let mut selected_done: Option<AnyElement> = None;
        for t in threads {
            let status = thread_status(self.thread_working(t.id), t);
            let done = status == "done";
            let row = self
                .child_row(
                    t,
                    self.thread_glyph(t.id, status, "side"),
                    crew::shown_title(&t.title),
                    cx,
                )
                .into_any_element();
            if !done {
                open.push(row);
            } else if !unfolded && self.selected == Some(t.id) {
                selected_done = Some(row);
            } else {
                finished.push(row);
            }
        }
        let n = finished.len() + usize::from(selected_done.is_some());
        if n > 0 {
            open.push(
                div()
                    .id(("crew-done-fold", pid as usize))
                    .h(px(24.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.sidebar_done_open.remove(&pid) {
                            this.sidebar_done_open.insert(pid);
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .flex()
                            .justify_center()
                            .child(icon(IconName::Check, 12., c_green())),
                    )
                    .child(div().flex_1().child(format!("{n} done")))
                    .child(icon(
                        if unfolded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        11.,
                        c_muted(),
                    ))
                    .into_any_element(),
            );
            if unfolded {
                open.extend(finished);
            } else {
                open.extend(selected_done);
            }
        }
        Self::child_rail(open).into_any_element()
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

    /// The glyph before a thread's title: a check once done, the working pulse, or a dot
    /// in its status colour.
    pub(crate) fn thread_glyph(&self, thread: u64, status: &str, key: &str) -> AnyElement {
        let slot = div().w(px(14.)).flex_none().flex().justify_center();
        match status {
            "done" | "resolved" => slot.child(icon(IconName::Check, 12., c_green())),
            "working" => slot.child(self.status_dot(
                Status::Working,
                SharedString::from(format!("thread-dot-{key}-{thread}")),
                7.,
            )),
            _ => slot.child(
                div()
                    .size(px(7.))
                    .rounded_full()
                    .bg(rgb(status_tone(status))),
            ),
        }
        .into_any_element()
    }

    /// What a finished thread changed, as a chip: the pull request it opened, its diff,
    /// or "No changes".
    fn thread_result(&self, thread: u64, info: &crew::ThreadInfo) -> AnyElement {
        let chip = || {
            div()
                .flex_none()
                .px(px(7.))
                .rounded_full()
                .bg(tint(c_subtle()))
                .text_size(px(11.))
        };
        let pr = info
            .reports
            .iter()
            .rev()
            .flat_map(|r| r.links.iter())
            .find_map(|l| crew::pr_number(l));
        if let Some(n) = pr {
            return chip()
                .text_color(rgb(c_blue()))
                .child(format!("PR #{n}"))
                .into_any_element();
        }
        match self.stats.get(&thread).copied().filter(|s| s.0 > 0) {
            Some((_, a, r)) => chip()
                .flex()
                .gap_1()
                .font_family(self.mono_font.clone())
                .child(div().text_color(rgb(c_add())).child(format!("+{a}")))
                .child(div().text_color(rgb(c_del())).child(format!("−{r}")))
                .into_any_element(),
            None => chip()
                .text_color(rgb(c_muted()))
                .child("No changes")
                .into_any_element(),
        }
    }

    /// One thread as a borderless row: its glyph, title, agent and how far along or how
    /// long ago, then one line under it: the decision it waits on with its answers, the
    /// step it is on, or what it came to. `note` takes the place of that line.
    pub(crate) fn thread_card(
        &self,
        thread: u64,
        key: &str,
        note: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Shown only with "Show deleted threads" on, as one quiet line.
        let Some(s) = self.state.session(thread) else {
            return div()
                .px_2()
                .py(px(2.))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(14.))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .child(icon(IconName::Trash, 10., c_muted())),
                )
                .child(small("Deleted thread").text_size(px(11.)))
                .into_any_element();
        };
        let status = thread_status(self.thread_working(thread), s);
        let info = s.crew.clone().unwrap_or_default();
        let finished = matches!(status, "done" | "resolved");
        let steps = info.checklist.len();
        let done = info
            .checklist
            .iter()
            .filter(|st| st.mark == Mark::Done)
            .count();
        let meta = if status == "working" && steps > 0 {
            format!("{done}/{steps}")
        } else if status == "failed" {
            "failed".into()
        } else {
            info.reports
                .last()
                .map(|r| short_age(r.at, crate::workspace::unix_now()))
                .unwrap_or_default()
        };
        let decision = info.decision.clone();
        let line = note.or_else(|| match status {
            "needs you" => decision.as_ref().map(|d| crew::plain(&d.question)),
            "working" => info
                .checklist
                .iter()
                .find(|st| st.mark == Mark::Doing)
                .map(|st| crew::plain(&st.text)),
            _ => crew::outcome(&info),
        });
        let selected = (self.selected == Some(thread) && self.view == View::Session)
            || self.peek == Some(thread);
        // A finished row says what changed with a chip; its outcome shows when it is open.
        let result = finished.then(|| self.thread_result(thread, &info));
        let line = if finished && !selected { None } else { line };
        let provider = agent::provider(&s.provider);
        let id = SharedString::from(format!("thread-card-{key}-{thread}"));
        let row = div()
            .id(id.clone())
            .relative()
            .px_2()
            .py(px(6.))
            .v_flex()
            .gap(px(3.))
            .rounded_lg()
            .cursor_pointer()
            // Beside Crew's chat a thread opens in the panel, so the chat stays put.
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.current().is_some_and(|s| s.canvas == crew::CANVAS) {
                    this.open_peek(thread);
                } else {
                    this.select(thread);
                }
                cx.notify();
            }));
        let row = if selected {
            row.bg(tint(c_active()))
                .child(shell::accent_bracket(format!("{id}-mark"), 8.))
        } else {
            shell::hover_wash(row, format!("{id}-wash"), tint(c_subtle()), 8.)
        };
        row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(self.thread_glyph(thread, status, key))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .when(finished, |el| el.text_color(rgb(c_secondary())))
                        .when(!finished, |el| el.font_weight(FontWeight::MEDIUM))
                        .child(crew::shown_title(&s.title)),
                )
                .when(result.is_none(), |el| {
                    el.child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(rgb(provider.color))
                            .child(provider.mark),
                    )
                })
                .children(result)
                .when(!meta.is_empty(), |el| {
                    el.child(
                        small(meta)
                            .flex_none()
                            .text_color(rgb(if status == "failed" {
                                c_red()
                            } else {
                                c_muted()
                            })),
                    )
                }),
        )
        .children(line.filter(|l| !l.is_empty()).map(|text| {
            div()
                .pl(px(22.))
                .text_size(px(12.))
                .text_color(rgb(c_secondary()))
                .when(status == "needs you" || finished, |el| el.line_clamp(2))
                .when(status != "needs you" && !finished, |el| el.truncate())
                .child(text)
        }))
        .when_some(decision.filter(|_| status == "needs you"), |el, d| {
            el.child(
                div()
                    .pl(px(22.))
                    .pt_1()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(self.decision_chips(
                        "thread-row-choice",
                        thread as usize,
                        thread,
                        &d,
                        cx,
                    )),
            )
        })
        .into_any_element()
    }

    /// Whether deleted threads keep their rows in Crew's chat.
    fn show_deleted(cx: &App) -> bool {
        cx.global::<AppSettings>().show_deleted_threads
    }

    /// Whether message `index` of Crew's chat is left out: news about a thread an earlier
    /// card already shows (those cards are live, so the thread shows once), or a card
    /// whose threads were all deleted while deleted threads are hidden.
    pub(crate) fn crew_card_hidden(&self, chat: &Session, index: usize, cx: &App) -> bool {
        let gone = |t: &u64| self.state.session(*t).is_none();
        let thread = match chat.messages.get(index).and_then(|m| m.crew.as_ref()) {
            Some(Card::News { thread }) => thread,
            Some(Card::Started { threads }) => {
                return !Self::show_deleted(cx) && threads.iter().all(gone);
            }
            _ => return false,
        };
        if gone(thread) && !Self::show_deleted(cx) {
            return true;
        }
        chat.messages[..index]
            .iter()
            .filter_map(|m| m.crew.as_ref())
            .any(|c| match c {
                Card::Started { threads } => threads.contains(thread),
                Card::News { thread: t } => t == thread,
                Card::Held { .. } => false,
            })
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
            Card::Started { threads } => {
                let threads: Vec<u64> = threads
                    .iter()
                    .copied()
                    .filter(|t| self.state.session(*t).is_some() || Self::show_deleted(cx))
                    .collect();
                div()
                    .v_flex()
                    .gap_2()
                    .child(line.child({
                        let finished = threads
                            .iter()
                            .filter_map(|t| self.state.session(*t))
                            .filter(|t| {
                                matches!(
                                    thread_status(self.thread_working(t.id), t),
                                    "done" | "resolved"
                                )
                            })
                            .count();
                        div().text_color(rgb(c_muted())).child(match finished {
                            0 => String::new(),
                            n if n == threads.len() => "· all finished".into(),
                            n => format!("· {n} of {} finished", threads.len()),
                        })
                    }))
                    .child(
                        div()
                            .p_1()
                            .v_flex()
                            .rounded_lg()
                            .bg(tint(c_subtle()))
                            .children(
                                threads
                                    .iter()
                                    .map(|t| self.thread_card(*t, &format!("m{index}"), None, cx)),
                            ),
                    )
            }
            // The news is the row's line, so the thread shows once, with what happened.
            Card::News { thread } => {
                let thread = *thread;
                let title = self
                    .state
                    .session(thread)
                    .map(|s| s.title.clone())
                    .unwrap_or_default();
                let news = crew::plain(
                    message
                        .text
                        .strip_prefix(title.as_str())
                        .map_or(message.text.as_str(), |rest| {
                            rest.trim_start_matches([':', '.', ' '])
                        }),
                );
                // Once finished, the row's chip and outcome say more than "finished."
                let done = self.state.session(thread).is_some_and(|s| {
                    matches!(
                        thread_status(self.thread_working(thread), s),
                        "done" | "resolved"
                    )
                });
                let news = (!done).then_some(news);
                div().child(self.thread_card(thread, &format!("m{index}"), news, cx))
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
    /// `panel` leaves out the way back to Crew, for a thread open beside Crew's chat.
    pub(crate) fn thread_strip(
        &self,
        session: &Session,
        panel: bool,
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
                .when(panel, |el| el.p_2().rounded_lg())
                .when(!panel, |el| {
                    el.child(
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
                })
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
        if let Some(view) = self
            .peek
            .filter(|t| self.state.session(*t).is_some_and(|s| s.project_id == pid))
            .and_then(|t| self.peek_view(t, cx))
        {
            return view;
        }
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
        let mut list = div().v_flex().gap_2().p_2();
        let mut shown: Vec<u64> = Vec::new();
        for (title, status) in groups {
            let mut members: Vec<&Session> = threads
                .iter()
                .copied()
                .filter(|t| !shown.contains(&t.id))
                .filter(|t| {
                    status.is_empty() || thread_status(self.thread_working(t.id), t) == status
                })
                .collect();
            if members.is_empty() {
                continue;
            }
            shown.extend(members.iter().map(|t| t.id));
            // The latest finished first: by its last report, else by when it started.
            members.sort_by_key(|t| {
                std::cmp::Reverse((
                    t.crew
                        .as_ref()
                        .and_then(|i| i.reports.last())
                        .map_or(0, |r| r.at),
                    t.id,
                ))
            });
            let count = members.len();
            let mut members: Vec<u64> = members.into_iter().map(|t| t.id).collect();
            // Past the latest few, finished threads wait behind "Show all", except the one
            // that is open.
            let hidden = if status == "done" && !self.threads_done_all && count > DONE_SHOWN {
                let keep: Vec<u64> = members
                    .iter()
                    .enumerate()
                    .filter(|(i, t)| *i < DONE_SHOWN || self.selected == Some(**t))
                    .map(|(_, t)| *t)
                    .collect();
                let n = count - keep.len();
                members = keep;
                n
            } else {
                0
            };
            // Finished threads fold away, so what is still open stays in view.
            let foldable = status == "done";
            let shut = foldable && self.threads_done_shut;
            list = list.child(
                div()
                    .v_flex()
                    .child(
                        div()
                            .id(SharedString::from(format!("threads-group-{status}")))
                            .px_2()
                            .pt_2()
                            .pb_1()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
                            .child(title.to_uppercase())
                            .child(div().opacity(0.8).child(format!("{count}")))
                            .when(foldable, |el| {
                                el.cursor_pointer()
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .font_weight(FontWeight::NORMAL)
                                            .hover(|el| el.text_color(rgb(c_secondary())))
                                            .child(if shut { "Show" } else { "Hide" }),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.threads_done_shut = !this.threads_done_shut;
                                        cx.notify();
                                    }))
                            }),
                    )
                    .when(!shut, |el| {
                        el.children(
                            members
                                .into_iter()
                                .map(|t| self.thread_card(t, "panel", None, cx)),
                        )
                        .when(hidden > 0, |el| {
                            el.child(
                                div()
                                    .id("threads-show-all")
                                    .px_2()
                                    .py(px(6.))
                                    .flex()
                                    .gap_1()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.threads_done_all = true;
                                        cx.notify();
                                    }))
                                    .child(format!("{hidden} more ·"))
                                    .child(
                                        div()
                                            .text_color(rgb(c_blue()))
                                            .hover(|el| el.underline())
                                            .child("Show all"),
                                    ),
                            )
                        })
                    }),
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
