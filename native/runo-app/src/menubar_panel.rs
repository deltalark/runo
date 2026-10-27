//! The panel that hangs under the menu bar glyph. Activity: what needs the person, with
//! Allow and Deny for a waiting permission, and the agents working, with Stop. Ask Runo:
//! the same chat as the assistant in the window. It closes when it loses focus.

use super::{Anchor, Dot, Go, Row, Snapshot};
use crate::assistant::{glow, Mood};
use crate::*;
use std::cell::{Cell, RefCell};
use std::time::Instant;

const WIDTH: f32 = 376.;
const HEIGHT: f32 = 620.;

/// The tile the person tapped to filter the list below.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Filter {
    Working,
    Needs,
}

thread_local! {
    static OPEN: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
    /// When the panel last closed on losing focus, so the click on the glyph that took
    /// its focus doesn't open it again straight away.
    static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The tab it showed last, so it opens the same way.
    static ASKING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn close(cx: &mut App) {
    if let Some(window) = OPEN.take() {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

pub(super) fn toggle(anchor: Option<Anchor>, cx: &mut App) {
    if OPEN.with_borrow(Option::is_some) {
        close(cx);
        return;
    }
    if CLOSED_AT
        .get()
        .is_some_and(|at| at.elapsed() < Duration::from_millis(400))
    {
        return;
    }
    let origin = anchor
        .map(|a| point(px(a.x - WIDTH / 2.), px(a.y + 6.)))
        .unwrap_or_default();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            origin,
            size(px(WIDTH), px(HEIGHT)),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        focus: true,
        show: true,
        display_id: anchor
            .and_then(|a| a.display)
            .map(|id| DisplayId::new(id as u64)),
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let app = cx.try_global::<MainApp>().map(|m| m.0.clone());
    let Ok((handle, _)) = open_window(options, cx, |window, cx| {
        cx.new(|cx| Panel::new(app, window, cx))
    }) else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        // The component library's root paints an opaque background; the panel draws its
        // own rounded card.
        gpui_kit::base::Root::update(window, cx, |root, _, cx| {
            root.style().background = Some(gpui_kit::transparent_black().into());
            cx.notify();
        });
        super::platform::place(window, WIDTH, HEIGHT);
    });
    OPEN.set(Some(handle));
}

struct Panel {
    app: Option<WeakEntity<RunoApp>>,
    input: Entity<InputState>,
    /// Ask Runo is showing rather than Activity.
    asking: bool,
    /// Which tile is currently narrowing the list below. None shows everything.
    filter: Option<Filter>,
    /// The row opened to show its detail, and the one under the pointer.
    opened: Option<u64>,
    hovered: Option<u64>,
    /// Which stat tile is under the pointer, to lift its wash.
    hovered_tile: Option<Filter>,
    scroll: ScrollHandle,
    /// How many messages the chat had when last drawn, to follow new ones down.
    seen: usize,
    _subs: Vec<Subscription>,
}

impl Panel {
    fn new(app: Option<WeakEntity<RunoApp>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Ask, or tell Runo what to do…"));
        let mut subs = vec![
            cx.subscribe_in(&input, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let question = input.read(cx).value().trim().to_string();
                    if !question.is_empty() {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                        this.ask(question, cx);
                    }
                }
            }),
            // Clicking anywhere else closes it, as a menu would.
            cx.observe_window_activation(window, |_, window, _| {
                if !window.is_window_active() {
                    CLOSED_AT.set(Some(Instant::now()));
                    OPEN.take();
                    window.remove_window();
                }
            }),
        ];
        if let Some(app) = app.as_ref().and_then(WeakEntity::upgrade) {
            subs.push(cx.observe(&app, |_, _, cx| cx.notify()));
        }
        // Running times tick over while it is open.
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                return;
            }
        })
        .detach();
        let asking = ASKING.get();
        if asking {
            input.update(cx, |i, cx| i.focus(window, cx));
        }
        Self {
            app,
            input,
            asking,
            filter: None,
            opened: None,
            hovered: None,
            hovered_tile: None,
            scroll: ScrollHandle::new(),
            seen: 0,
            _subs: subs,
        }
    }

    fn toggle_filter(&mut self, f: Filter, cx: &mut Context<Self>) {
        self.filter = (self.filter != Some(f)).then_some(f);
        cx.notify();
    }

    fn with_app(&self, cx: &mut App, f: impl FnOnce(&mut RunoApp, &mut Context<RunoApp>)) {
        if let Some(app) = self.app.as_ref().and_then(WeakEntity::upgrade) {
            app.update(cx, |this, cx| {
                f(this, cx);
                cx.notify();
            });
        }
    }

    /// Ask the assistant, showing its chat.
    fn ask(&mut self, question: String, cx: &mut Context<Self>) {
        self.show_ask(true, cx);
        self.with_app(cx, |app, cx| app.ask_runo(question, cx));
    }

    fn show_ask(&mut self, on: bool, cx: &mut Context<Self>) {
        self.asking = on;
        ASKING.set(on);
        cx.notify();
    }

    fn header(&self, snap: &Snapshot, mood: Mood) -> Div {
        // "Good evening." becomes "Good evening" so it joins other lines without a stop.
        let hello = crate::home::greeting().trim_end_matches('.');
        let status = if snap.closed {
            "Window closed".to_string()
        } else {
            summary_line(snap)
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .pt(px(13.))
            .pb(px(10.))
            .child(crate::assistant::live_mark_sized(mood, 30.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap(px(1.))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(hello),
                            )
                            .child(
                                div()
                                    .text_size(px(14.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(c_text()))
                                    .child("Runo"),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(status),
                    ),
            )
            .when(!snap.closed, |el| {
                el.child(
                    icon_button("mb-settings", IconName::Settings, "Settings")
                        .on_click(|_, _, cx| super::go(Go::Settings, cx)),
                )
            })
            .child(
                icon_button("mb-open-runo", IconName::ExternalLink, "Open Runo")
                    .on_click(|_, _, cx| super::go(Go::OpenRuno, cx)),
            )
    }

    /// The three counts under the header; working and needs filter the list below, review
    /// opens the Review page in the window.
    fn stats(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let working = self.stat_tile(
            "mb-stat-working",
            snap.running,
            "Working",
            c_blue(),
            Some(Filter::Working),
            cx,
        );
        let needs_label = if snap.needs_you == 1 {
            "Needs you"
        } else {
            "Need you"
        };
        let needs = self.stat_tile(
            "mb-stat-needs",
            snap.needs_you,
            needs_label,
            c_amber(),
            Some(Filter::Needs),
            cx,
        );
        let review = self.stat_tile(
            "mb-stat-review",
            snap.review,
            "To review",
            c_green(),
            None,
            cx,
        );
        div()
            .flex()
            .gap(px(8.))
            .px(px(14.))
            .pb(px(12.))
            .child(working)
            .child(needs)
            .child(review)
    }

    fn stat_tile(
        &self,
        id: &'static str,
        n: usize,
        what: &'static str,
        color: u32,
        filter: Option<Filter>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let lit = n > 0;
        let active = filter.is_some() && self.filter == filter;
        let hovered = filter.is_some() && self.hovered_tile == filter;
        // Tint strength: a quiet wash at rest, a fuller one for the chosen tile. Unlit
        // tiles sit near canvas so a count of 0 does not pull the eye.
        let (wash_a, wash_b) = match (lit, active) {
            (false, _) => (0.045, 0.025),
            (true, false) => (0.11, 0.06),
            (true, true) => (0.20, 0.12),
        };
        let number_color = if lit { c_text() } else { c_muted() };
        div()
            .id(id)
            .flex_1()
            .min_w_0()
            .relative()
            .px(px(10.))
            .py(px(8.))
            .rounded(px(12.))
            .cursor_pointer()
            .bg(linear_gradient(
                160.,
                linear_color_stop(glow(color, wash_a), 0.),
                linear_color_stop(glow(color, wash_b), 1.),
            ))
            .when(hovered && !active, |el| {
                el.bg(linear_gradient(
                    160.,
                    linear_color_stop(glow(color, wash_a + 0.04), 0.),
                    linear_color_stop(glow(color, wash_b + 0.03), 1.),
                ))
            })
            .active(|el| el.opacity(0.85))
            .when_some(filter, |el, f| {
                el.on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    this.hovered_tile = if *over { Some(f) } else { None };
                    cx.notify();
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_filter(f, cx)))
            })
            .when(filter.is_none(), |el| {
                el.on_click(|_, _, cx| super::go(Go::Review, cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(
                        div()
                            .text_size(px(21.))
                            .line_height(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(number_color))
                            .font_features(tnum())
                            .child(n.to_string()),
                    )
                    .child(div().flex_1())
                    // Top-right glyph: a lit dot for a non-zero count, a hollow ring
                    // otherwise, in the tile's own colour.
                    .child(if lit {
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(rgb(color))
                            .shadow(vec![crate::assistant::halo(color, 0.75, 7., 0., 0.)])
                    } else {
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .border_1()
                            .border_color(glow(color, 0.45))
                    }),
            )
            .child(
                div()
                    .pt(px(2.))
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(what),
            )
    }

    fn tabs(&self, cx: &mut Context<Self>) -> Div {
        div().px(px(14.)).pb(px(6.)).child(
            crate::shell::seg_track()
                .w_full()
                .child(
                    crate::shell::seg_choice("mb-tab-activity", "mb-tab", !self.asking)
                        .flex_1()
                        .child("Activity")
                        .on_click(cx.listener(|this, _, _, cx| this.show_ask(false, cx))),
                )
                .child(
                    crate::shell::seg_choice("mb-tab-ask", "mb-tab", self.asking)
                        .flex_1()
                        .child(icon(
                            IconName::Sparkles,
                            12.,
                            if self.asking { c_blue() } else { c_muted() },
                        ))
                        .child("Ask Runo")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_ask(true, cx);
                            this.input.update(cx, |i, cx| i.focus(window, cx));
                        })),
                ),
        )
    }

    /// A session that needs the person, as a card tinted by why.
    fn card(&self, row: &Row, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = row.id;
        let tone = match row.dot {
            Dot::Waiting => c_amber(),
            Dot::Failed => c_red(),
            _ => c_green(),
        };
        let mut body = div().v_flex().gap(px(8.)).pl(px(31.));
        if let Some((wants, code)) = &row.approval {
            body = body.child(
                div()
                    .v_flex()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .child(wants.clone()),
                    )
                    .when_some(code.clone(), |el, code| {
                        el.child(
                            div()
                                .px(px(8.))
                                .py(px(6.))
                                .rounded(px(7.))
                                .bg(rgba(if is_dark() { 0x00000059 } else { 0x0000000d }))
                                .font_family("JetBrains Mono")
                                .text_size(px(11.5))
                                .text_color(rgb(c_text()))
                                .child(code),
                        )
                    }),
            );
            let mut buttons = div().flex().items_center().gap(px(6.));
            for (n, (label, allow, primary)) in [
                ("Allow", Some(false), true),
                ("Always", Some(true), false),
                ("Deny", None, false),
            ]
            .into_iter()
            .enumerate()
            {
                buttons = buttons.child(
                    button(("mb-answer", id * 4 + n as u64), label, primary).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.with_app(cx, |app, cx| {
                                if let Some(request) = app.approval_for(id).cloned() {
                                    app.answer_approval(&request, allow, cx);
                                }
                            });
                        },
                    )),
                );
            }
            body = body.child(
                buttons.child(div().flex_1()).child(
                    ghost(("mb-open", id), "Open", IconName::ArrowUpRight)
                        .on_click(move |_, _, cx| super::go(Go::Open(id), cx)),
                ),
            );
        } else {
            if let Some(last) = &row.last {
                body = body.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .line_clamp(2)
                        .child(format!("“{last}”")),
                );
            }
            let title = row.title.clone();
            let failed = row.dot == Dot::Failed;
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        button(("mb-open", id), "Open", true)
                            .on_click(move |_, _, cx| super::go(Go::Open(id), cx)),
                    )
                    .when(failed, |el| {
                        el.child(button(("mb-why", id), "Ask Runo why", false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.ask(format!("Why did “{title}” fail?"), cx);
                            }),
                        ))
                    }),
            );
        }
        div()
            .id(("mb-card", id))
            .mx(px(10.))
            .mb(px(8.))
            .px(px(12.))
            .py(px(11.))
            .rounded(px(12.))
            .border_1()
            .border_color(glow(tone, if row.dot == Dot::Waiting { 0.45 } else { 0.3 }))
            .bg(glow(tone, 0.07))
            .v_flex()
            .gap(px(8.))
            .child(head_line(row, Some(tone)))
            .child(body)
    }

    /// A session row: a status glyph on the left (ring for Working, dot for the rest),
    /// title with project · branch under it, elapsed on the right. Click opens the row
    /// for more; click Open to jump to it in the window. Dense but with air.
    fn working(&self, row: &Row, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = row.id;
        let opened = self.opened == Some(id);
        let hovered = self.hovered == Some(id);
        let (tone, running) = match row.dot {
            Dot::Working => (c_blue(), true),
            Dot::Waiting => (c_amber(), false),
            Dot::Failed => (c_red(), false),
            Dot::Finished => (c_green(), false),
        };
        let meta = {
            let mut m = Vec::new();
            if !row.project.is_empty() {
                m.push(row.project.clone());
            }
            if !row.branch.is_empty() {
                m.push(row.branch.clone());
            }
            m.join(" · ")
        };
        let indicator: AnyElement = if running {
            working::ring(tone)
        } else {
            div()
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(7.))
                        .rounded_full()
                        .bg(rgb(tone))
                        .shadow(vec![crate::assistant::halo(tone, 0.75, 7., 0., 0.)]),
                )
                .into_any_element()
        };

        let top = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(indicator)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap(px(1.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_text()))
                            .child(row.title.clone()),
                    )
                    .when(!meta.is_empty(), |el| {
                        el.child(
                            div()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(meta),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(rgb(if row.dot == Dot::Failed {
                        c_red()
                    } else {
                        c_muted()
                    }))
                    .font_features(tnum())
                    .child(row.status.clone()),
            );

        let mut out = div()
            .id(("mb-work", id))
            .relative()
            .mx(px(8.))
            .px(px(10.))
            .py(px(7.))
            .rounded(px(10.))
            .cursor_pointer()
            .when(hovered || opened, |el| el.bg(tint(c_subtle())))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                this.hovered = if *over { Some(id) } else { None };
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.opened = (this.opened != Some(id)).then_some(id);
                cx.notify();
            }))
            .v_flex()
            .gap(px(4.))
            .child(top);
        if opened {
            if let Some(last) = &row.last {
                out = out.child(
                    div()
                        .pl(px(26.))
                        .pt(px(3.))
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .line_clamp(3)
                        .child(format!("“{last}”")),
                );
            }
            out = out.child(
                div()
                    .pl(px(26.))
                    .pt(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(!row.facts.is_empty(), |el| {
                        el.child(
                            div()
                                .flex_1()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(row.facts.clone()),
                        )
                    })
                    .when(row.facts.is_empty(), |el| el.child(div().flex_1()))
                    .child(
                        ghost(("mb-open", id), "Open", IconName::ArrowUpRight)
                            .on_click(move |_, _, cx| super::go(Go::Open(id), cx)),
                    ),
            );
        }
        if row.stoppable && (hovered || opened) {
            out = out.child(
                div().absolute().top(px(6.)).right(px(8.)).child(
                    button(("mb-stop", id), "Stop", false)
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.with_app(cx, |app, _| app.stop_session(id));
                        })),
                ),
            );
        }
        out
    }

    fn activity(&self, snap: &Snapshot, acts: bool, cx: &mut Context<Self>) -> Div {
        let mut out = div().v_flex();
        let show_needs = self.filter != Some(Filter::Working);
        let show_working = self.filter != Some(Filter::Needs);
        // Waiting and Failed want your eye; Finished is just a result to read.
        let (attention, done): (Vec<&Row>, Vec<&Row>) =
            snap.needs.iter().partition(|r| r.dot != Dot::Finished);

        if show_needs && !attention.is_empty() {
            out = out.child(heading("NEEDS YOU"));
            for row in &attention {
                out = out.child(self.card(row, cx));
            }
        }
        if show_working && !snap.working.is_empty() {
            out = out.child(heading("WORKING"));
            for row in &snap.working {
                out = out.child(self.working(row, cx));
            }
            if snap.more_working > 0 {
                out = out.child(
                    link("mb-more-working", format!("{} more", snap.more_working))
                        .on_click(|_, _, cx| super::go(Go::Sessions, cx)),
                );
            }
        }
        if show_needs && !done.is_empty() {
            out = out.child(heading("DONE"));
            for row in &done {
                out = out.child(self.working(row, cx));
            }
        }
        if show_needs && snap.more_needs > 0 {
            out = out.child(
                link(
                    "mb-more-needs",
                    format!("{} more in the Inbox", snap.more_needs),
                )
                .on_click(|_, _, cx| super::go(Go::Inbox, cx)),
            );
        }
        if self.filter.is_some()
            && attention.is_empty()
            && done.is_empty()
            && snap.working.is_empty()
        {
            out = out.child(
                div()
                    .px(px(16.))
                    .pt(px(14.))
                    .pb(px(10.))
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child("Nothing here right now."),
            );
        }
        if snap.needs.is_empty() && snap.working.is_empty() {
            out = out
                .child(
                    div()
                        .pt(px(26.))
                        .pb(px(18.))
                        .v_flex()
                        .items_center()
                        .gap(px(4.))
                        .child(crate::assistant::live_mark_sized(Mood::Idle, 28.))
                        .child(
                            div()
                                .pt(px(6.))
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Nothing running"),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_muted()))
                                .child("Start something, or ask Runo."),
                        ),
                )
                .child(heading("TRY"))
                .child(self.starters(acts, cx));
        }
        out
    }

    /// Questions to start the chat with.
    fn starters(&self, acts: bool, cx: &mut Context<Self>) -> Div {
        let starters: &'static [&str] = if acts {
            &crate::assistant::AGENT_STARTERS
        } else {
            &crate::assistant::STARTERS
        };
        let mut out = div().flex().flex_wrap().gap(px(6.)).px(px(14.)).pb(px(8.));
        for (i, q) in starters.iter().enumerate() {
            out = out.child(
                div()
                    .id(("mb-starter", i))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded_full()
                    .border_1()
                    .border_color(divider())
                    .bg(tint(c_subtle()))
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_border())))
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .child(*q)
                    .on_click(cx.listener(move |this, _, _, cx| this.ask(q.to_string(), cx))),
            );
        }
        out
    }

    /// The assistant's chat, with Runo's mark beside each answer.
    fn chat(&mut self, app: &Entity<RunoApp>, acts: bool, cx: &mut Context<Self>) -> Div {
        let (messages, empty) = app.update(cx, |this, cx| {
            // Answers read here are read in the window too.
            if this.assistant.unread > 0 {
                this.assistant.unread = 0;
                cx.notify();
            }
            (this.assistant_messages(cx), this.assistant.chat.is_empty())
        });
        if messages.len() != self.seen {
            self.seen = messages.len();
            self.scroll.scroll_to_bottom();
        }
        if empty {
            return div()
                .v_flex()
                .gap(px(10.))
                .child(
                    div()
                        .px(px(14.))
                        .pt(px(6.))
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(rgb(c_secondary()))
                        .child(if acts {
                            "Ask what's happening, or tell me what to do. I'll ask before changing anything."
                        } else {
                            "Ask how to do something in Runo and I'll explain it."
                        }),
                )
                .child(self.starters(acts, cx));
        }
        div()
            .px(px(14.))
            .pt(px(4.))
            .pb(px(8.))
            .v_flex()
            .gap(px(12.))
            .children(messages)
    }

    fn composer(&self, acts: bool) -> Div {
        let accent = c_blue();
        let ring_to = settings::mix(crate::assistant::RING_VIOLET, accent, 0.6);
        div()
            .px(px(12.))
            .pt(px(8.))
            .v_flex()
            .gap(px(6.))
            .child(
                div()
                    .p(px(1.))
                    .rounded(px(12.))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(glow(accent, 0.9), 0.),
                        linear_color_stop(glow(ring_to, 0.85), 1.),
                    ))
                    .shadow(vec![crate::assistant::halo(accent, 0.25, 16., 0., 2.)])
                    .child(
                        div()
                            .rounded(px(11.))
                            .bg(rgb(c_canvas()))
                            .child(Input::new(&self.input).appearance(false).h(px(38.)).px_2()),
                    ),
            )
            .child(
                div()
                    .px(px(4.))
                    .text_size(px(10.5))
                    .text_color(rgb(c_muted()))
                    .child(if acts {
                        "Asks before changing anything · Enter to send"
                    } else {
                        "Answers from Runo's guide · Enter to send"
                    }),
            )
    }
}

/// A session's first line: agent badge, title, project and status.
fn head_line(row: &Row, tone: Option<u32>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(9.))
        .child(badge(row.mark, row.color))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_text()))
                .child(row.title.clone()),
        )
        .when(!row.project.is_empty() && tone.is_some(), |el| {
            el.child(
                div()
                    .flex_none()
                    .px(px(6.))
                    .rounded(px(5.))
                    .bg(tint(c_subtle()))
                    .text_size(px(10.5))
                    .text_color(rgb(c_secondary()))
                    .child(row.project.clone()),
            )
        })
        .child(div().flex_1())
        .child(
            div()
                .flex_none()
                .text_size(px(11.5))
                .text_color(rgb(match (row.dot, tone) {
                    (Dot::Failed, _) => c_red(),
                    (Dot::Waiting, Some(t)) => t,
                    _ => c_muted(),
                }))
                .child(row.status.clone()),
        )
}

/// The agent's glyph on a tile of its colour.
fn badge(mark: &'static str, color: u32) -> Div {
    div()
        .size(px(22.))
        .flex_none()
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(settings::mix(
            color,
            c_canvas(),
            if is_dark() { 0.22 } else { 0.14 },
        )))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(color))
        .child(mark)
}

/// A small grey heading over a section: muted, borderless, matches the window.
fn heading(text: &'static str) -> Div {
    div()
        .px(px(16.))
        .pt(px(10.))
        .pb(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_muted()))
        .child(text)
}

/// Tabular figures: the time ticks over without the row shifting.
fn tnum() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

/// The one-line summary under the greeting: "1 working · 2 need you". Zeros are hidden,
/// so a quiet moment reads as "All quiet" rather than "0 working · 0 need you".
fn summary_line(snap: &Snapshot) -> String {
    let mut parts = Vec::new();
    if snap.running > 0 {
        parts.push(format!("{} working", snap.running));
    }
    if snap.needs_you > 0 {
        parts.push(format!(
            "{} need{} you",
            snap.needs_you,
            if snap.needs_you == 1 { "s" } else { "" }
        ));
    }
    if snap.review > 0 {
        parts.push(format!("{} to review", snap.review));
    }
    if parts.is_empty() {
        "All quiet".into()
    } else {
        parts.join(" · ")
    }
}

/// The footer's primary call: a solid accent pill with a small plus chip on the left.
fn new_session_button() -> Stateful<Div> {
    let accent = c_blue();
    div()
        .id("mb-new")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(7.))
        .pl(px(5.))
        .pr(px(11.))
        .py(px(4.))
        .rounded(px(8.))
        .cursor_pointer()
        .bg(rgb(accent))
        .text_color(rgb(0xffffff))
        .shadow(vec![crate::assistant::halo(accent, 0.5, 10., 0., 1.)])
        .hover(|el| el.bg(rgb(settings::mix(accent, 0xffffff, 0.9))))
        .active(|el| el.opacity(0.85))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| super::go(Go::NewSession, cx))
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new("New session").build(window, cx)
        })
        .child(
            div()
                .size(px(18.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(rgba(0xffffff33))
                .child(icon(IconName::Plus, 12., 0xffffff)),
        )
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .child("New session"),
        )
}

fn button(id: impl Into<ElementId>, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(11.))
        .py(px(4.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .when(primary, |el| {
            el.bg(rgb(c_blue()))
                .text_color(rgb(0xffffff))
                .shadow(vec![crate::assistant::halo(c_blue(), 0.45, 10., 0., 1.)])
        })
        .when(!primary, |el| {
            el.bg(tint(c_subtle()))
                .text_color(rgb(c_text()))
                .hover(|el| el.bg(tint(c_border())))
        })
        .active(|el| el.opacity(0.8))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label)
}

/// A text button with an icon after it, for Open.
fn ghost(id: impl Into<ElementId>, label: &'static str, after: IconName) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(3.))
        .px(px(6.))
        .py(px(4.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(rgb(c_secondary()))
        .hover(|el| el.text_color(rgb(c_text())).bg(tint(c_subtle())))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label)
        .child(icon(after, 12., c_muted()))
}

fn icon_button(id: &'static str, name: IconName, tip: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .cursor_pointer()
        .bg(tint(c_subtle()))
        .hover(|el| el.bg(tint(c_border())))
        .child(icon(name, 13., c_secondary()))
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
        })
}

/// A quiet text button for the footer and the "more" lines.
fn link(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(16.))
        .py(px(5.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(rgb(c_secondary()))
        .hover(|el| el.text_color(rgb(c_text())))
        .child(label.into())
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.as_ref().and_then(WeakEntity::upgrade);
        let snap = match &app {
            Some(app) => app.read(cx).menubar_snapshot(),
            None => Snapshot::closed(),
        };
        let thinking = app
            .as_ref()
            .is_some_and(|a| a.read(cx).assistant.asking.is_some());
        let acts = app.as_ref().is_some_and(|a| a.read(cx).assistant_acts(cx));
        let assistant_on = RunoApp::assistant_enabled(cx);
        let mood = if thinking {
            Mood::Thinking
        } else if snap.needs_you > 0 {
            Mood::NeedsYou
        } else if self.asking {
            Mood::Listening
        } else {
            Mood::Idle
        };

        let mut body = div()
            .id("mb-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb(px(6.))
            .v_flex();
        let mut composer = None;
        match &app {
            None => {
                body = body.child(
                    div()
                        .px(px(16.))
                        .pt(px(10.))
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("Open Runo's window to see your sessions here."),
                );
            }
            Some(_) if !self.asking => body = body.child(self.activity(&snap, acts, cx)),
            Some(_) if !assistant_on => {
                body = body.child(
                    div()
                        .pt(px(30.))
                        .px(px(24.))
                        .v_flex()
                        .items_center()
                        .gap(px(10.))
                        .child(crate::assistant::live_mark_sized(Mood::Idle, 28.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .text_center()
                                .child("The Runo assistant is off. Turn it on to ask questions and have Runo start, stop and answer agents for you."),
                        )
                        .child(button("mb-assistant-on", "Turn on the assistant", true).on_click(
                            |_, _, cx| settings::update(cx, |s| s.assistant = true),
                        )),
                );
            }
            Some(app) => {
                body = body.child(self.chat(app, acts, cx));
                composer = Some(self.composer(acts));
            }
        }

        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(10.))
            .mt(px(4.))
            .when(!snap.closed, |el| el.child(new_session_button()))
            .child(div().flex_1())
            .child(
                link("mb-quit", "Quit Runo")
                    .px(px(4.))
                    .on_click(|_, _, cx| super::quit(cx)),
            );

        // The ring Home's prompt and the assistant wear: the accent turning violet.
        let accent = c_blue();
        let ring_to = settings::mix(crate::assistant::RING_VIOLET, accent, 0.6);
        div()
            .size_full()
            .p(px(1.))
            .rounded(px(16.))
            .bg(linear_gradient(
                160.,
                linear_color_stop(glow(accent, 0.55), 0.),
                linear_color_stop(glow(ring_to, 0.35), 1.),
            ))
            .child(
                div()
                    .size_full()
                    .v_flex()
                    .rounded(px(15.))
                    .overflow_hidden()
                    .bg(float(c_canvas()))
                    .text_color(rgb(c_text()))
                    .child(self.header(&snap, mood))
                    .when(!snap.closed, |el| {
                        el.child(self.stats(&snap, cx)).child(self.tabs(cx))
                    })
                    .child(body)
                    .children(composer)
                    .child(footer),
            )
    }
}
