//! Pieces shared by the Ask AI bars on the Architecture and Design canvases: the composer
//! around the input, the chips in it, and what the canvas shows while an agent works.

use crate::archdraw::view::col;
use crate::*;

/// The composer's frame: a raised card with the input, chips and buttons inside.
pub(crate) fn composer(id: &'static str, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(600.))
        .max_w_full()
        .px_2()
        .pt(px(6.))
        .pb(px(6.))
        .v_flex()
        .gap(px(4.))
        .rounded_xl()
        .border_1()
        .border_color(if active {
            col(c_blue(), 0.7)
        } else {
            rgb(c_border())
        })
        .bg(float(c_canvas()))
        .shadow_lg()
        .text_size(px(12.5))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// What the ask is about: the selection, a screen, a new screen. The caller makes the ×
/// clear it.
pub(crate) fn target_chip(id: impl Into<ElementId>, lead: IconName, text: String) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .max_w(px(420.))
        .h(px(24.))
        .pl_2()
        .pr_1()
        .flex()
        .items_center()
        .gap_1()
        .rounded_md()
        .bg(col(c_blue(), 0.12))
        .text_size(px(11.5))
        .text_color(rgb(c_text()))
        .cursor_pointer()
        .child(icon(lead, 12., c_blue()))
        .child(div().min_w_0().truncate().child(text))
        .child(icon(IconName::X, 11., c_muted()))
}

/// A one-tap starting point for the ask, shown while the input is empty.
pub(crate) fn suggestion(id: impl Into<ElementId>, text: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(divider())
        .text_size(px(11.5))
        .text_color(rgb(c_secondary()))
        .cursor_pointer()
        .hover(|el| el.bg(tint(c_active())).text_color(rgb(c_text())))
        .child(text)
}

/// The round send button; dimmed while there's nothing to send.
pub(crate) fn send_button(id: &'static str, ready: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(if ready {
            rgb(c_blue())
        } else {
            col(c_blue(), 0.35)
        })
        .cursor_pointer()
        .child(icon(IconName::ArrowUp, 15., 0xffffff))
}

/// A spinning loader.
pub(crate) fn spinner(id: &'static str, size: f32) -> AnyElement {
    icon(IconName::LoaderCircle, size, c_blue()).animated(
        id,
        Animation::new(Duration::from_millis(900)).repeat(),
        |i, d| i.rotate(percentage(d)),
    )
}

/// The composer while the agent works: what it's doing, for how long, and a way to stop.
pub(crate) fn working_row(id: &'static str, line: String, task: String, secs: u64) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_1()
        .min_h(px(32.))
        .child(spinner(id, 15.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .v_flex()
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(div().text_color(rgb(c_text())).child(line))
                        .child(div().text_color(rgb(c_muted())).child(format!("{secs}s"))),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(task),
                ),
        )
}

/// 0 to 1 and back again over one turn of an animation.
pub(crate) fn pulse(d: f32) -> f32 {
    1. - (d * 2. - 1.).abs()
}

/// A soft glow round the canvas and a line at its top, while an agent changes what's on it.
pub(crate) fn busy_edge(line: String, secs: Option<u64>) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .border_2()
                .border_color(col(c_blue(), 0.5))
                .animated(
                    "ask-busy-glow",
                    Animation::new(Duration::from_millis(1800)).repeat(),
                    |el, d| el.opacity(0.25 + 0.75 * pulse(d)),
                ),
        )
        .child(
            div()
                .absolute()
                .top(px(12.))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py(px(6.))
                        .rounded_full()
                        .border_1()
                        .border_color(col(c_blue(), 0.4))
                        .bg(float(c_canvas()))
                        .shadow_md()
                        .text_size(px(12.))
                        .child(spinner("ask-busy-spin", 14.))
                        .child(div().text_color(rgb(c_text())).child(line))
                        .children(
                            secs.map(|s| div().text_color(rgb(c_muted())).child(format!("{s}s"))),
                        ),
                ),
        )
        .into_any_element()
}

/// A card on an empty canvas while an agent draws: a sketch of boxes and lines lighting up
/// in turn, a title, and what the agent is likely doing.
pub(crate) fn busy_card(title: &'static str, line: String, secs: Option<u64>) -> AnyElement {
    const BOXES: [(f32, f32); 5] = [(0., 0.), (104., 0.), (208., 0.), (52., 78.), (156., 78.)];
    const LINES: [(f32, f32, f32, f32); 4] = [
        (72., 19., 32., 2.),
        (176., 19., 32., 2.),
        (87., 40., 2., 38.),
        (191., 40., 2., 38.),
    ];
    let mut sketch = div().relative().w(px(280.)).h(px(118.));
    for (i, (x, y, w, h)) in LINES.into_iter().enumerate() {
        sketch = sketch.child(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w))
                .h(px(h))
                .bg(rgb(c_blue()))
                .animated(
                    ("ask-busy-line", i),
                    Animation::new(Duration::from_millis(2400)).repeat(),
                    move |el, d| el.opacity(0.15 + 0.6 * pulse((d + 0.1 + i as f32 * 0.2).fract())),
                ),
        );
    }
    for (i, (x, y)) in BOXES.into_iter().enumerate() {
        sketch = sketch.child(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(72.))
                .h(px(40.))
                .rounded_md()
                .border_1()
                .border_color(rgb(c_blue()))
                .bg(col(c_blue(), 0.14))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(36.))
                        .h(px(5.))
                        .rounded_full()
                        .bg(col(c_blue(), 0.6)),
                )
                .animated(
                    ("ask-busy-box", i),
                    Animation::new(Duration::from_millis(2400)).repeat(),
                    move |el, d| el.opacity(0.25 + 0.75 * pulse((d + i as f32 * 0.2).fract())),
                ),
        );
    }
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(400.))
                .p_5()
                .v_flex()
                .items_center()
                .gap_4()
                .rounded_xl()
                .border_1()
                .border_color(divider())
                .bg(float(c_canvas()))
                .shadow_lg()
                .child(sketch)
                .child(
                    div()
                        .v_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(spinner("ask-busy-card-spin", 14.))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .child(match secs {
                                    Some(s) => format!("{line} · {s}s"),
                                    None => line,
                                }),
                        ),
                ),
        )
        .into_any_element()
}

/// A screen still being drawn: grey bars that shimmer where its content will be.
pub(crate) fn skeleton(id: impl Into<ElementId> + Clone, scale: f32) -> AnyElement {
    let s = |v: f32| px((v * scale).max(2.));
    let bar = |w: f32, h: f32| {
        div()
            .w(relative(w))
            .h(s(h))
            .rounded(s(6.))
            .bg(col(c_muted(), 0.18))
    };
    div()
        .size_full()
        .p(s(24.))
        .v_flex()
        .gap(s(14.))
        .child(bar(0.45, 22.))
        .child(bar(0.8, 12.))
        .child(bar(1., 120.))
        .child(
            div()
                .flex()
                .gap(s(12.))
                .child(bar(0.5, 70.))
                .child(bar(0.5, 70.)),
        )
        .child(bar(0.9, 12.))
        .child(bar(0.7, 12.))
        .child(bar(1., 44.))
        .animated(
            id,
            Animation::new(Duration::from_millis(1400)).repeat(),
            |el, d| el.opacity(0.45 + 0.55 * pulse(d)),
        )
}
