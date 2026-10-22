//! The panel that hangs under the menu bar glyph: what needs the person, with Allow and
//! Deny for a waiting permission; the agents working, with Stop; and Ask Runo, the same
//! chat as the assistant in the window. It closes when it loses focus.

use super::{Anchor, Dot, Go, Row, Snapshot};
use crate::*;
use std::cell::{Cell, RefCell};
use std::time::Instant;

const WIDTH: f32 = 380.;
const HEIGHT: f32 = 600.;

thread_local! {
    static OPEN: RefCell<Option<AnyWindowHandle>> = const { RefCell::new(None) };
    /// When the panel last closed on losing focus, so the click on the glyph that took
    /// its focus doesn't open it again straight away.
    static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
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
    /// The row opened to show its detail.
    opened: Option<u64>,
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
                        this.with_app(cx, |app, cx| app.ask_runo(question, cx));
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
        input.update(cx, |i, cx| i.focus(window, cx));
        Self {
            app,
            input,
            opened: None,
            scroll: ScrollHandle::new(),
            seen: 0,
            _subs: subs,
        }
    }

    fn with_app(&self, cx: &mut App, f: impl FnOnce(&mut RunoApp, &mut Context<RunoApp>)) {
        if let Some(app) = self.app.as_ref().and_then(WeakEntity::upgrade) {
            app.update(cx, |this, cx| {
                f(this, cx);
                cx.notify();
            });
        }
    }

    fn row(&self, row: &Row, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = row.id;
        let opened = self.opened == Some(id);
        let dot = match row.dot {
            Dot::Waiting => c_amber(),
            Dot::Failed => c_red(),
            Dot::Finished => c_green(),
            Dot::Working => c_blue(),
        };
        let mut text = div()
            .flex_1()
            .min_w_0()
            .v_flex()
            .gap(px(2.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_text()))
                    .truncate()
                    .child(row.title.clone()),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .truncate()
                    .child(row.approval.clone().unwrap_or_else(|| row.line.clone())),
            );
        if opened {
            for line in &row.detail {
                text = text.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_secondary()))
                        .child(line.clone()),
                );
            }
        }
        let mut buttons = div().flex().flex_wrap().gap(px(6.)).pt(px(6.));
        let mut any = false;
        if row.approval.is_some() {
            any = true;
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
        }
        if opened || row.approval.is_some() {
            any = true;
            buttons = buttons.child(
                button(("mb-open", id), "Open", false)
                    .on_click(move |_, _, cx| super::go(Go::Open(id), cx)),
            );
        }
        let stop = row.stoppable.then(|| {
            button(("mb-stop", id), "Stop", false).on_click(cx.listener(move |this, _, _, cx| {
                this.with_app(cx, |app, _| app.stop_session(id));
            }))
        });
        div()
            .id(("mb-row", id))
            .mx_2()
            .px_2()
            .py(px(7.))
            .rounded_lg()
            .flex()
            .items_start()
            .gap(px(10.))
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())))
            .when(opened || row.approval.is_some(), |el| {
                el.bg(tint(c_subtle()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.opened = (this.opened != Some(id)).then_some(id);
                cx.notify();
            }))
            .child(
                div()
                    .mt(px(5.))
                    .size(px(8.))
                    .flex_none()
                    .rounded_full()
                    .bg(rgb(dot)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(text)
                    .when(any, |el| el.child(buttons)),
            )
            .children(stop)
    }

    fn section(
        &self,
        title: &'static str,
        rows: &[Row],
        more: usize,
        go: Go,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if rows.is_empty() {
            return None;
        }
        let mut out = div().v_flex().gap(px(2.)).child(heading(title));
        for row in rows {
            out = out.child(self.row(row, cx));
        }
        if more > 0 {
            out = out.child(
                link(title, format!("{more} more…")).on_click(move |_, _, cx| super::go(go, cx)),
            );
        }
        Some(out)
    }
}

/// A small grey heading over a section.
fn heading(text: &'static str) -> Div {
    div()
        .px_4()
        .pt_3()
        .pb_1()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_muted()))
        .child(text)
}

fn button(id: impl Into<ElementId>, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(10.))
        .py(px(3.))
        .rounded_md()
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .when(primary, |el| el.bg(rgb(c_blue())).text_color(rgb(0xffffff)))
        .when(!primary, |el| {
            el.bg(tint(c_subtle()))
                .text_color(rgb(c_text()))
                .hover(|el| el.bg(tint(c_border())))
        })
        .active(|el| el.opacity(0.8))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label)
}

/// A quiet text button for the footer and the "more" lines.
fn link(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_4()
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
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child("Runo"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(snap.summary.clone()),
            )
            .child(
                div()
                    .id("mb-open-runo")
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .bg(tint(c_subtle()))
                    .hover(|el| el.bg(tint(c_border())))
                    .child(icon(IconName::ExternalLink, 13., c_secondary()))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Open Runo").build(window, cx)
                    })
                    .on_click(|_, _, cx| super::go(Go::OpenRuno, cx)),
            );

        let mut body = div()
            .id("mb-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb_2()
            .v_flex();
        if !snap.closed {
            body = body
                .children(self.section("NEEDS YOU", &snap.needs, snap.more_needs, Go::Inbox, cx))
                .children(self.section(
                    "WORKING",
                    &snap.working,
                    snap.more_working,
                    Go::Sessions,
                    cx,
                ));
            if snap.needs.is_empty() && snap.working.is_empty() {
                body = body.child(
                    div()
                        .px_4()
                        .pt_3()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("No agents running and nothing waiting on you."),
                );
            }
            if snap.review > 0 {
                body = body.child(
                    link(
                        "mb-review",
                        if snap.review == 1 {
                            "1 session has changes to review".to_string()
                        } else {
                            format!("{} sessions have changes to review", snap.review)
                        },
                    )
                    .pt_2()
                    .on_click(|_, _, cx| super::go(Go::Review, cx)),
                );
            }
        }

        // Ask Runo: the assistant's own chat, so it carries on in the window.
        let mut ask = None;
        if let Some(app) = &app {
            if RunoApp::assistant_enabled(cx) {
                let messages = app.update(cx, |this, cx| this.assistant_messages(cx));
                if messages.len() != self.seen {
                    self.seen = messages.len();
                    self.scroll.scroll_to_bottom();
                }
                body = body
                    .child(heading("ASK RUNO"))
                    .child(div().px_4().v_flex().gap_3().children(messages));
                ask = Some(
                    div()
                        .px_3()
                        .pb_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(divider())
                        .child(Input::new(&self.input).bordered(true).h(px(34.))),
                );
            } else {
                body = body.child(heading("ASK RUNO")).child(
                    div()
                        .px_4()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(rgb(c_muted()))
                                .child("The assistant is off."),
                        )
                        .child(button("mb-assistant-on", "Turn it on", true).on_click(
                            |_, _, cx| {
                                settings::update(cx, |s| s.assistant = true);
                            },
                        )),
                );
            }
        }

        let footer = div()
            .flex()
            .items_center()
            .py_1()
            .border_t_1()
            .border_color(divider())
            .when(!snap.closed, |el| {
                el.child(
                    link("mb-new", "New session")
                        .on_click(|_, _, cx| super::go(Go::NewSession, cx)),
                )
            })
            .child(link("mb-runo", "Open Runo").on_click(|_, _, cx| super::go(Go::OpenRuno, cx)))
            .child(div().flex_1())
            .child(link("mb-quit", "Quit Runo").on_click(|_, _, cx| super::quit(cx)));

        div()
            .size_full()
            .v_flex()
            .rounded_xl()
            .overflow_hidden()
            .border_1()
            .border_color(divider())
            .bg(rgb(c_canvas()))
            .text_color(rgb(c_text()))
            .child(header)
            .child(body)
            .children(ask)
            .child(footer)
    }
}
