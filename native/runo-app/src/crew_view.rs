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
        "working" | "watching" => c_blue(),
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

/// The accent at `alpha` (0 to 0xff), for Crew's wash and chip.
fn accent_wash(alpha: u32) -> Rgba {
    rgba((c_blue() << 8) | alpha)
}

fn small(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(c_muted()))
        .child(text.into())
}

/// A small label over a group of Crew's threads in the sidebar, with how many it holds.
fn group_label(label: &'static str, n: usize, first: bool) -> AnyElement {
    div()
        .h(px(if first { 18. } else { 22. }))
        .px_2()
        .pb(px(3.))
        .flex()
        .items_end()
        .gap(px(5.))
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(c_muted()))
        .child(label)
        .child(
            div()
                .opacity(0.8)
                .font_weight(FontWeight::NORMAL)
                .child(n.to_string()),
        )
        .into_any_element()
}

/// What the card on a Crew thread's sidebar row shows, gathered when the row is drawn.
#[derive(Clone)]
struct ThreadCard {
    title: String,
    status: String,
    tone: u32,
    mark: &'static str,
    color: u32,
    agent: String,
    branch: Option<String>,
    pr: Option<u64>,
    size: Option<String>,
    stats: Option<(usize, usize)>,
    mono: SharedString,
}

impl ThreadCard {
    fn element(&self) -> Div {
        let line = |key: &'static str, value: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .text_color(rgb(c_muted()))
                        .child(key),
                )
                .child(value)
        };
        let text = |s: String, color: u32| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(rgb(color))
                .child(s)
                .into_any_element()
        };
        div()
            .w(px(250.))
            .v_flex()
            .gap(px(4.))
            .text_size(px(11.5))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .mb_1()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_color(rgb(c_muted()))
                    .child(div().size(px(7.)).rounded_full().bg(rgb(self.tone)))
                    .child(self.status.clone()),
            )
            .child(line(
                "Agent",
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(self.color))
                            .child(self.mark),
                    )
                    .child(text(self.agent.clone(), c_secondary()))
                    .into_any_element(),
            ))
            .when_some(self.branch.clone(), |el, b| {
                el.child(line("Branch", text(b, c_secondary())))
            })
            .when_some(self.pr, |el, n| {
                el.child(line("Pull request", text(format!("#{n}"), c_blue())))
            })
            .when_some(self.size.clone(), |el, size| {
                el.child(line("Worktree", text(size, c_secondary())))
            })
            .when_some(self.stats, |el, (a, r)| {
                el.child(line(
                    "Changes",
                    div()
                        .flex()
                        .gap_1()
                        .font_family(self.mono.clone())
                        .child(div().text_color(rgb(c_add())).child(format!("+{a}")))
                        .child(div().text_color(rgb(c_del())).child(format!("−{r}")))
                        .into_any_element(),
                ))
            })
    }
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

    pub(crate) fn thread_working(&self, id: u64) -> bool {
        self.handles.contains_key(&Target::Session(id))
    }

    /// The pinned Crew row at the top of a Crew project's sessions. It is Crew's own
    /// place, not one more session, so it always sits on a soft wash of the accent, its
    /// mark in a small accent chip; picked, the wash grows fuller.
    /// While Crew works the chip glows, steady with Animations off.
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
        let dark = is_dark();
        let (wash, hover) = match (on, dark) {
            (true, true) => (0x3a, 0x3a),
            (true, false) => (0x2e, 0x2e),
            (false, true) => (0x16, 0x24),
            (false, false) => (0x10, 0x1c),
        };
        let mut chip = div()
            .relative()
            .size(px(22.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .bg(accent_wash(if on { 0x40 } else { 0x26 }))
            .border_1()
            .border_color(accent_wash(if working { 0xcc } else { 0x00 }));
        if working {
            // A soft halo just outside the chip, breathing while Crew works.
            let halo = div()
                .absolute()
                .top(px(-3.))
                .left(px(-3.))
                .size(px(28.))
                .rounded(px(8.))
                .bg(accent_wash(0x30));
            chip = chip.child(
                halo.opacity(0.7).animated(
                    ("crew-glow", pid as usize),
                    Animation::new(Duration::from_millis(1800))
                        .repeat()
                        .with_easing(pulsating_between(0.15, 1.)),
                    |el, d| el.opacity(d),
                ),
            );
        }
        let chip = chip.child(icon(CREW_ICON, 14., c_blue()));
        div()
            .id(("crew-row", pid as usize))
            .relative()
            .h(px(34.))
            .pl(px(6.))
            .pr_2()
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_md()
            .cursor_pointer()
            .bg(accent_wash(wash))
            .hover(move |el| el.bg(accent_wash(hover)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_crew(pid, cx);
                this.right_tab = RightTab::Threads;
                cx.notify();
            }))
            .child(chip)
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child("Crew"),
            )
            .when(unread && !working, |el| {
                el.child(div().size(px(7.)).rounded_full().bg(rgb(c_blue())))
            })
            .when(waiting > 0, |el| {
                el.child(
                    div()
                        .px(px(7.))
                        .rounded_full()
                        .bg(rgba((c_amber() << 8) | if dark { 0x33 } else { 0x2a }))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_amber()))
                        .child(format!("{waiting} needs you")),
                )
            })
            .when(open > 0 && waiting == 0, |el| {
                el.child(
                    div()
                        .min_w(px(18.))
                        .px(px(5.))
                        .flex()
                        .justify_center()
                        .rounded_full()
                        .bg(accent_wash(if on { 0x30 } else { 0x1c }))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_blue()))
                        .child(format!("{open}")),
                )
            })
            .into_any_element()
    }

    /// Crew's threads under its row in the sidebar, and only there, grouped by what they
    /// need: the person, then working, then watching CI, then waiting on something else. Finished and
    /// resolved ones fold into a single "N done" row until it is opened. Each thread is
    /// one line with its time; the selected one also shows its agent, branch and worktree
    /// size, and hovering one shows all its details.
    pub(crate) fn crew_children(
        &self,
        pid: u64,
        threads: &[&Session],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut groups: [Vec<AnyElement>; 4] = Default::default();
        let mut finished: Vec<(&Session, &'static str)> = Vec::new();
        for t in threads {
            let status = thread_status(self.thread_working(t.id), t);
            let group = match status {
                "needs you" | "failed" => 0,
                "working" => 1,
                "watching" => 2,
                "done" | "resolved" => {
                    finished.push((t, status));
                    continue;
                }
                _ => 3,
            };
            groups[group].push(self.thread_row(t, status, cx));
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        for (label, group) in ["Needs you", "Working", "Watching CI", "Waiting"]
            .into_iter()
            .zip(groups)
        {
            if !group.is_empty() {
                rows.push(group_label(label, group.len(), rows.is_empty()));
                rows.extend(group);
            }
        }
        if !finished.is_empty() {
            finished.sort_by_key(|(t, _)| std::cmp::Reverse(t.id));
            let unfolded = self.sidebar_done_open.contains(&pid);
            let n = finished.len();
            rows.push(
                div()
                    .id(("crew-done-fold", pid as usize))
                    .when(!rows.is_empty(), |el| el.mt(px(4.)))
                    .h(px(24.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    // A quiet text action: hovering brightens the words, no box.
                    .hover(|el| el.text_color(rgb(c_secondary())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.sidebar_done_open.remove(&pid) {
                            this.sidebar_done_all.remove(&pid);
                        } else {
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
            // The newest few, then the rest on request. The open thread stays in view
            // even when it is folded away, just under the "N done" row.
            let shown = match (unfolded, self.sidebar_done_all.contains(&pid)) {
                (false, _) => 0,
                (true, false) => n.min(DONE_SHOWN),
                (true, true) => n,
            };
            for (i, (t, status)) in finished.iter().enumerate() {
                if i < shown || self.selected == Some(t.id) {
                    rows.push(self.thread_row(t, status, cx));
                }
            }
            if unfolded && shown < n {
                rows.push(
                    div()
                        .id(("crew-done-all", pid as usize))
                        .h(px(22.))
                        .pl(px(30.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .hover(|el| el.text_color(rgb(c_secondary())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sidebar_done_all.insert(pid);
                            cx.notify();
                        }))
                        .child(format!("Show {} more", n - shown))
                        .into_any_element(),
                );
            }
        }
        // Crew's threads sit a step in, under Crew's chip, so they read as Crew's and not
        // as the project's other sessions; indentation does that without a guide line.
        div()
            .ml(px(18.))
            .mt(px(2.))
            .v_flex()
            .gap(px(1.))
            .children(rows)
            .into_any_element()
    }

    /// One of Crew's threads in the sidebar: its status, its title and, in the corner,
    /// how long it has worked or when it last spoke (a finished one's pull request).
    /// Selected, a second line gives its agent, branch and worktree size; hovering it
    /// shows all of that and more on a card.
    fn thread_row(&self, t: &Session, status: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let id = t.id;
        let selected = self.selected == Some(id) && self.view == View::Session;
        let finished = matches!(status, "done" | "resolved");
        let provider = agent::provider(&t.provider);
        let model = t
            .selections()
            .get("model")
            .filter(|m| !m.is_empty())
            .cloned();
        let branch = (!t.branch.is_empty() && t.branch != "local folder").then(|| t.branch.clone());
        let size = t.isolated.then(|| self.space.chip_label(&t.dir)).flatten();
        let pr = crew::pr_number(&t.pr_url).or_else(|| {
            t.crew.as_ref().and_then(|i| {
                i.reports
                    .iter()
                    .rev()
                    .flat_map(|r| r.links.iter())
                    .find_map(|l| crew::pr_number(l))
            })
        });
        let when = match self.turns.get(&id) {
            Some(turn) if status == "working" => elapsed(turn.started.elapsed()),
            _ => t
                .messages
                .last()
                .map(|m| m.time.clone())
                .unwrap_or_default(),
        };
        let tone = match status {
            "working" | "watching" => c_blue(),
            "needs you" => c_amber(),
            "failed" => c_red(),
            _ => c_muted(),
        };
        // A watched pull request: "Watching CI #12", "CI failed, fixing #12" and so on.
        let watch = self.watch_line(t);
        let watch_chip = self.watch_chip(t, "side", cx);
        let corner = match pr.filter(|_| finished) {
            Some(n) => div()
                .flex_none()
                .whitespace_nowrap()
                .px(px(5.))
                .rounded(px(4.))
                .bg(accent_wash(0x1c))
                .text_size(px(10.5))
                .text_color(rgb(c_blue()))
                .child(format!("#{n}")),
            None => div()
                .flex_none()
                .whitespace_nowrap()
                .text_size(px(11.))
                .text_color(rgb(tone))
                .child(when.clone()),
        };
        let corner = match watch_chip {
            Some(chip) => chip,
            None => corner.into_any_element(),
        };
        let agent = match &model {
            Some(m) => format!("{} · {m}", provider.name),
            None => provider.name.to_string(),
        };
        let card = ThreadCard {
            title: crew::shown_title(&t.title),
            status: match (status, &watch) {
                (_, Some((text, _, n, _))) => format!("{text} · #{n}"),
                ("working", _) if !when.is_empty() => format!("Working · {when}"),
                ("resolved", _) => "Done".into(),
                (s, _) => {
                    let mut c = s.chars();
                    c.next()
                        .map(|f| f.to_uppercase().chain(c).collect())
                        .unwrap_or_default()
                }
            },
            tone: if finished { c_green() } else { tone },
            mark: provider.mark,
            color: provider.color,
            agent: agent.clone(),
            branch: branch.clone(),
            pr,
            size: size.clone(),
            stats: self
                .stats
                .get(&id)
                .copied()
                .filter(|s| s.0 > 0)
                .map(|(_, a, r)| (a, r)),
            mono: self.mono_font.clone(),
        };
        let title = div()
            .w_full()
            .min_w_0()
            .h(px(18.))
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(selected || t.unread, |el| {
                        el.font_weight(FontWeight::MEDIUM)
                    })
                    .text_color(rgb(if selected || t.unread {
                        c_text()
                    } else {
                        c_secondary()
                    }))
                    .child(crew::shown_title(&t.title)),
            )
            .child(corner);
        let details = selected.then(|| {
            div()
                .w_full()
                .min_w_0()
                .h(px(15.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(11.))
                .text_color(rgb(c_muted()))
                .child(self.provider_mark(provider, 10.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(branch.unwrap_or(agent)),
                )
                .when_some(size, |el, size| {
                    el.child(
                        div()
                            .flex_none()
                            .px(px(5.))
                            .rounded(px(4.))
                            .bg(tint(c_subtle()))
                            .text_size(px(10.5))
                            .child(size),
                    )
                })
        });
        let row = div()
            .id(("crew-thread", id as usize))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::Session(id), cx),
            )
            .relative()
            .py(px(5.))
            .px_2()
            .flex()
            .items_start()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.5))
            .when(self.cursor == Some(id), |el| el.bg(tint(c_selected())))
            .tooltip(move |window, cx| {
                let card = card.clone();
                gpui_kit::component::tooltip::Tooltip::element(move |_, _| card.element())
                    .px_3()
                    .py_2()
                    .build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(id);
                cx.notify();
            }));
        let row = if selected {
            row.bg(tint(c_active()))
                .child(shell::accent_bracket(format!("child-mark-{id}"), 6.))
        } else {
            row.hover(|el| el.bg(tint(c_subtle())))
        };
        row.child(
            div()
                .h(px(18.))
                .flex()
                .items_center()
                .child(self.thread_glyph(id, status, "side")),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .v_flex()
                .gap(px(1.))
                .child(title)
                .children(details),
        )
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

    /// A thread's message to Crew, where the person's own messages go: "From", the
    /// thread's number and title as a link to it, and its status, with the first line of
    /// what it said below. A click on the chip shows the whole message as `body`.
    pub(crate) fn thread_message(
        &self,
        index: usize,
        thread: u64,
        said: &str,
        body: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = (self.selected.unwrap_or(0), index);
        let open = body.is_some();
        let found = self.state.session(thread);
        let title = found
            .map(|t| crew::shown_title(&t.title))
            .unwrap_or_default();
        let link = div()
            .id(("wake-link", index))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .text_size(px(13.))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("#{thread}")),
            )
            .when(!title.is_empty(), |el| {
                el.child(div().truncate().child(title))
            })
            .map(|el| match found {
                Some(_) => el
                    .text_color(rgb(c_blue()))
                    .cursor_pointer()
                    .hover(|el| el.underline())
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Open this thread")
                            .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_thread_ref(thread, event.modifiers().platform);
                        cx.notify();
                    })),
                None => el
                    .text_color(rgb(c_muted()))
                    .child(div().flex_none().text_size(px(11.5)).child("deleted")),
            });
        let status = found.map(|t| thread_status(self.thread_working(thread), t));
        let preview = said.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        div()
            .max_w(px(560.))
            .min_w(px(220.))
            .v_flex()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .overflow_hidden()
            .child(
                div()
                    .id(("wake-head", index))
                    .px_3()
                    .py(px(8.))
                    .v_flex()
                    .gap(px(2.))
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.open_wakes.remove(&key) {
                            this.open_wakes.insert(key);
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon(IconName::MessagesSquare, 13., c_muted()))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .child("From"),
                            )
                            .child(link)
                            .when_some(status, |el, status| {
                                el.child(self.thread_glyph(thread, status, "wake"))
                            })
                            .child(div().flex_1())
                            .child(icon(
                                if open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                },
                                12.,
                                c_muted(),
                            )),
                    )
                    .when(!open && !preview.is_empty(), |el| {
                        el.child(
                            div()
                                .pl(px(21.))
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(rgb(c_muted()))
                                .child(preview.trim().to_string()),
                        )
                    }),
            )
            .when_some(body, |el, body| {
                el.child(
                    div()
                        .px(px(14.))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(divider())
                        .text_size(px(13.5))
                        .text_color(rgb(c_text()))
                        .whitespace_normal()
                        .child(body),
                )
            })
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
        let subagents = crate::health::running_subagents(s);
        let meta = if status == "working" && (steps > 0 || subagents > 0) {
            let mut parts = Vec::new();
            if steps > 0 {
                parts.push(format!("{done}/{steps}"));
            }
            match subagents {
                0 => {}
                1 => parts.push("1 helper agent".into()),
                n => parts.push(format!("{n} helper agents")),
            }
            parts.join(" · ")
        } else if s.queue.iter().any(|q| q.text == crate::planlimit::CONTINUE) {
            "paused".into()
        } else if self.waiting_for_slot(thread) {
            "waiting".into()
        } else if status == "failed" {
            "failed".into()
        } else {
            info.reports
                .last()
                .map(|r| short_age(r.at, crate::workspace::unix_now()))
                .unwrap_or_default()
        };
        let decision = info.decision.clone();
        let noted = note.is_some();
        let line = note.or_else(|| match status {
            "needs you" => decision.as_ref().map(|d| crew::plain(&d.question)),
            "working" => info
                .checklist
                .iter()
                .find(|st| st.mark == Mark::Doing)
                .map(|st| crew::plain(&st.text)),
            _ => crew::outcome(&info),
        });
        // The PR watcher gave up: its reason is what the person needs to read.
        let line = match &info.watch {
            Some(w) if !noted && status == "needs you" && decision.is_none() => {
                w.needs.clone().or(line)
            }
            _ => line,
        };
        let watch = self.watch_chip(s, key, cx);
        // Crew answered the thread's last decision, and the thread hasn't reported since.
        let crew_answered = info.crew_answer.as_ref().is_some_and(|a| {
            status != "needs you" && info.reports.last().is_none_or(|r| r.at <= a.at)
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
                .children(watch)
                .when(crew_answered, |el| {
                    el.child(small("Crew answered").flex_none().text_color(rgb(c_blue())))
                })
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

    /// For each message of Crew's chat, whether it is left out: news about a thread an
    /// earlier card already shows (those cards are live, so the thread shows once), or a
    /// card whose threads were all deleted while deleted threads are hidden. One pass over
    /// the chat, not one over the messages before each.
    pub(crate) fn crew_cards_hidden(&self, chat: &Session, cx: &App) -> Vec<bool> {
        let show_deleted = Self::show_deleted(cx);
        // One pass over the sessions too, rather than a search of them per card.
        let live: std::collections::HashSet<u64> = if show_deleted {
            Default::default()
        } else {
            self.state.sessions.iter().map(|s| s.id).collect()
        };
        let gone = |t: &u64| !live.contains(t);
        let mut shown = std::collections::HashSet::new();
        chat.messages
            .iter()
            .map(|m| match m.crew.as_ref() {
                Some(Card::News { thread }) => {
                    let hidden = (gone(thread) && !show_deleted) || shown.contains(thread);
                    shown.insert(*thread);
                    hidden
                }
                Some(Card::Started { threads }) => {
                    shown.extend(threads.iter().copied());
                    !show_deleted && threads.iter().all(gone)
                }
                _ => false,
            })
            .collect()
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
                        let cents: f64 = threads
                            .iter()
                            .filter_map(|t| self.state.session(*t))
                            .map(crate::health::session_cost)
                            .sum();
                        let mut parts: Vec<String> = Vec::new();
                        match finished {
                            0 => {}
                            n if n == threads.len() => parts.push("all finished".into()),
                            n => parts.push(format!("{n} of {} finished", threads.len())),
                        }
                        if cents > 0. {
                            parts.push(format!("${:.2} so far", cents / 100.));
                        }
                        div().text_color(rgb(c_muted())).child(if parts.is_empty() {
                            String::new()
                        } else {
                            format!("· {}", parts.join(" · "))
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

    /// Over Crew's composer, inside its column so the two share a left edge: where the next
    /// message goes.
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
        let crew_answer = info.crew_answer.clone().filter(|_| decision.is_none());
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
                .children(crew_answer.map(|a| {
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child(icon(CREW_ICON, 12., c_blue()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .line_clamp(3)
                                .child(format!("Crew answered \"{}\": {}", a.question, a.answer)),
                        )
                }))
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
            ("Watching CI", "watching"),
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
