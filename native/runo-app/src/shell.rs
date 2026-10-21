//! The Workbench frame around every screen: the mode rail on the left, the top bar
//! with the project switcher, search and live counters, the tab strip over a session,
//! and the Home page. Design and Architecture have their places on the rail; their pages
//! live in `design` and `architecture`.

use super::*;
use gpui_kit::component::tooltip::Tooltip;
use std::cell::Cell;

/// Width of the mode rail.
pub(crate) const RAIL_WIDTH: f32 = 52.;

/// Where the open place's tab sits on its 34px rail item: starting just outside the
/// item on the left, 8px in from the rail's edge, reaching 9px past it to the rail's
/// other edge on the right, and a little past it at the top and bottom.
const TAB_INSET: f32 = -1.;
const TAB_PAD: f32 = 4.;
/// Radius of the curves where a tab or a selected row flows into the sheet beside it.
const FUSE_R: f32 = 12.;
/// Radius of a sheet's top corner where it meets the rail under the top bar.
const SHEET_R: f32 = 14.;
/// Radius of the rail's own corners where it turns in above and below the open tab.
const RAIL_R: f32 = 14.;

/// How long the shell's small transitions take: quick enough to feel immediate, long
/// enough that a change reads as a change rather than a jump.
const QUICK: u64 = 160;

/// Hold still for the first `lead` of an animation, then ease out over the rest: lets a
/// detail land after the motion it belongs to.
fn after(lead: f32) -> impl Fn(f32) -> f32 {
    move |t| ease_out_quint()(((t - lead) / (1. - lead)).clamp(0., 1.))
}

/// The curve where an edge turns into the sheet beside it: the corner of an `r`
/// square outside a quarter circle, filled with `fill`, so whatever is behind shows in
/// the rest. Drawn as a path, not as a square painted over in a guessed colour, so no
/// corner shows darker than what is around it. It sits beside its shape, never over it,
/// so it matches in glass mode too.
fn inner_curve(fill: Rgba, top: bool, r: f32) -> Div {
    let fill = Hsla::from(fill);
    div().absolute().right_0().size(px(r)).child(
        canvas(
            |_, _, _| (),
            move |b, _, window, _| {
                let k = 0.5523;
                let p = |x: f32, y: f32| point(b.origin.x + px(x), b.origin.y + px(y));
                // The top curve turns round the square's top-left corner, the bottom
                // one round its bottom-left.
                let mut f = PathBuilder::fill();
                if top {
                    f.move_to(p(r, 0.));
                    f.line_to(p(r, r));
                    f.line_to(p(0., r));
                    f.cubic_bezier_to(p(r, 0.), p(k * r, r), p(r, k * r));
                } else {
                    f.move_to(p(0., 0.));
                    f.line_to(p(r, 0.));
                    f.line_to(p(r, r));
                    f.cubic_bezier_to(p(0., 0.), p(r, r - k * r), p(k * r, 0.));
                }
                f.close();
                if let Ok(path) = f.build() {
                    window.paint_path(path, fill);
                }
            },
        )
        .size_full(),
    )
}

/// The one touch of accent on a selected row: a thin bracket in the accent that hugs the
/// row's left edge and wraps round its rounded corners, like "(". It draws in once the
/// row has landed.
pub(crate) fn accent_bracket(key: impl Into<SharedString>) -> AnyElement {
    div()
        .absolute()
        .top(px(1.))
        .bottom(px(1.))
        .left(px(-5.))
        .w(px(FUSE_R + 2.))
        .border_l(px(2.))
        .rounded_l(px(FUSE_R + 2.))
        .border_color(rgb(c_blue()))
        .animated(
            ElementId::Name(key.into()),
            Animation::new(Duration::from_millis(QUICK + 120)).with_easing(after(0.4)),
            |el, d| el.opacity(d),
        )
        .into_any_element()
}

/// The track behind a row of connected choices, such as the sidebar's filter or a
/// setting's options: a shallow well a step darker than what it sits on. Every
/// segmented control uses it, so they all read the same.
pub(crate) fn seg_track() -> Div {
    div()
        .flex()
        .items_center()
        .p(px(3.))
        .gap(px(2.))
        .rounded(px(9.))
        .bg(rgba(if is_dark() { 0x00000047 } else { 0x0000000d }))
}

/// The raised pill under the chosen choice in a `seg_track`, which fades in when it is
/// chosen. A child of the choice, which must be `relative` and add it first.
pub(crate) fn seg_pill(key: impl Into<SharedString>) -> AnyElement {
    chosen_fill_of(
        key,
        7.,
        div()
            // A touch more lift through glass, where the desktop behind thins it.
            .bg(rgba(match (is_dark(), glass_on()) {
                (true, false) => 0xffffff17,
                (true, true) => 0xffffff21,
                (false, false) => 0xffffffff,
                (false, true) => 0xffffffd9,
            }))
            .shadow_sm(),
    )
}

/// One choice in a `seg_track`: the chosen one on its pill in full text colour, the
/// rest quieter until hovered.
pub(crate) fn seg_choice(
    id: impl Into<ElementId>,
    key: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .h(px(26.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .when(on, |el| el.child(seg_pill(key)).text_color(rgb(c_text())))
        .when(!on, |el| {
            el.text_color(rgb(c_secondary()))
                .hover(|el| el.text_color(rgb(c_text())))
                .active(|el| el.opacity(0.7))
        })
}

thread_local! {
    /// Where the open tab sits this frame, so the rail can leave a hole for it.
    static TAB_CUT: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) };
    /// The layer the open tab is cut from, set while the rail is built.
    static TAB_SHEET: Cell<Option<Rgba>> = const { Cell::new(None) };
}

/// The open place's tab on the rail: a piece of what sits beside the rail, the same
/// colour, that reaches out around the icon, with the rail's edge curving into it above
/// and below, so the place and its list (or page) read as one sheet. It slides out of
/// the sheet when the place opens. `sheet` is the fill of what it is cut from.
///
/// The tab paints nothing itself. The rail lays the sheet under its whole column and
/// its own shade over that, cut round the tab (see `rail_fill`), so the tab is the very
/// layer it joins and no two see-through edges ever meet in glass mode.
fn rail_tab(key: &'static str, sheet: Rgba) -> AnyElement {
    TAB_SHEET.set(Some(sheet));
    let reach = (RAIL_WIDTH - 34.) / 2.;
    let height = 34. + 2. * TAB_PAD;
    div()
        .absolute()
        .top(px(-TAB_PAD))
        .left(px(TAB_INSET))
        .right(px(-reach))
        .h(px(height))
        .child(canvas(|b, _, _| TAB_CUT.set(Some(b)), |_, _, _, _| {}).size_full())
        // The accent bracket on the tab's leading corner, as on every open tab.
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size(px(FUSE_R + 2.))
                .border_l(px(2.))
                .border_t(px(2.))
                .rounded_tl(px(FUSE_R))
                .border_color(rgb(c_blue()))
                .animated(
                    SharedString::from(format!("rail-bracket-{key}")),
                    Animation::new(Duration::from_millis(QUICK + 120)).with_easing(after(0.4)),
                    |el, d| el.opacity(d),
                ),
        )
        .animated(
            SharedString::from(format!("rail-tab-{key}")),
            Animation::new(Duration::from_millis(QUICK)).with_easing(ease_out_quint()),
            move |el, d| el.left(px(TAB_INSET + 6. * (1. - d))),
        )
}

/// The rail's own fill: the sheet under the whole column, then the rail's shade over it
/// as one path, its whole column less the tab and the curves into it. Where the shade's
/// edge is antialiased it blends into the sheet beneath, never into the desktop, so the
/// outline stays one smooth curve in every glass style. The rest of the frame is this
/// same pair flattened (see `settings::frame`), so the rail and the bars match.
///
/// A tab cut from the page rather than a list (`tab`) has the difference between the
/// page and the sheet laid in the hole too, so it is the very shade of the page.
fn rail_fill(tab: Rgba) -> AnyElement {
    let base = pane(c_sheet());
    let fill = Hsla::from(rail_shade());
    let lift = (tab != base).then(|| Hsla::from(settings::over(tab, base)));
    let shade = canvas(
        |_, _, _| TAB_CUT.set(None),
        move |b, _, window, _| {
            let (r, k) = (FUSE_R, 0.5523);
            let at = |x: Pixels, y: Pixels| point(x, y);
            // A quarter turn from `from` to `to` round the square corner `c`.
            let turn =
                |f: &mut PathBuilder, from: Point<Pixels>, c: Point<Pixels>, to: Point<Pixels>| {
                    f.cubic_bezier_to(to, from + (c - from) * k, to + (c - to) * k);
                };
            let (left, top, right, bottom) = (b.origin.x, b.origin.y, b.right(), b.bottom());
            // Where the islands end, above the gutter along the bottom.
            let floor = bottom - px(GUTTER);
            // The tab's outline, from the rail's edge above it round to the edge below.
            let outline = |f: &mut PathBuilder, t: Bounds<Pixels>| {
                let (tx, ty, tb, r, o) = (t.origin.x, t.origin.y, t.bottom(), px(r), px(RAIL_R));
                f.line_to(at(right, ty - o));
                turn(f, at(right, ty - o), at(right, ty), at(right - o, ty));
                f.line_to(at(tx + r, ty));
                turn(f, at(tx + r, ty), at(tx, ty), at(tx, ty + r));
                if tb + o > floor {
                    // A tab at the foot, such as Settings, has no room to curve out below:
                    // it runs level with the island's square bottom edge instead.
                    let tb = tb.min(floor);
                    f.line_to(at(tx, tb - r));
                    turn(f, at(tx, tb - r), at(tx, tb), at(tx + r, tb));
                    f.line_to(at(right, tb));
                } else {
                    f.line_to(at(tx, tb - r));
                    turn(f, at(tx, tb - r), at(tx, tb), at(tx + r, tb));
                    f.line_to(at(right - o, tb));
                    turn(f, at(right - o, tb), at(right, tb), at(right, tb + o));
                }
            };
            let cut = TAB_CUT.get();
            let mut f = PathBuilder::fill();
            f.move_to(at(left, top));
            f.line_to(at(right, top));
            if let Some(t) = cut {
                outline(&mut f, t);
            }
            f.line_to(at(right, bottom));
            f.line_to(at(left, bottom));
            f.close();
            if let Ok(path) = f.build() {
                window.paint_path(path, fill);
            }
            if let (Some(t), Some(lift)) = (cut, lift) {
                let mut f = PathBuilder::fill();
                f.move_to(at(right, t.origin.y - px(RAIL_R)));
                outline(&mut f, t);
                f.close();
                if let Ok(path) = f.build() {
                    window.paint_path(path, lift);
                }
            }
        },
    )
    .size_full();
    div()
        .absolute()
        .inset_0()
        .bg(base)
        .child(shade)
        .into_any_element()
}

/// The open place on a page with no list beside the rail, such as the More menu: a quiet
/// tile under the icon, which fades up when the place opens.
fn rail_tile(key: &'static str) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .rounded(px(8.))
        .bg(tint(c_active()))
        .animated(
            SharedString::from(format!("rail-tile-{key}")),
            Animation::new(Duration::from_millis(QUICK)).with_easing(ease_out_quint()),
            |el, d| el.opacity(d),
        )
}

/// A rail icon: the open place's at full strength, the rest muted.
fn rail_icon(glyph: IconName, on: bool) -> AnyElement {
    div()
        .relative()
        .child(icon(glyph, 18., if on { c_text() } else { c_muted() }))
        .into_any_element()
}

/// A list's selected row joined to the page beside the list: a fill in the page's colour
/// from the row out to the list's edge, with the list curving into it above and below,
/// the same way the rail's tab joins the list, and a short accent rule on its left edge.
/// `reach` is how far the list's edge is past the row's right side. It slides in from the
/// page when the row is picked. A child of the row, which must be `relative` and draw it
/// first, under its content.
pub(crate) fn row_fuse(key: impl Into<SharedString>, reach: f32) -> AnyElement {
    let key = key.into();
    // The row sits on the list's sheet: in glass mode only the difference is painted, so
    // the row and its curves are the very shade of the page they join.
    let page = settings::over(glass_fill(c_canvas()), pane(c_sheet()));
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right(px(-reach))
        .child(div().absolute().inset_0().rounded_l(px(FUSE_R)).bg(page))
        .child(inner_curve(page, true, FUSE_R).top(px(-FUSE_R)))
        .child(inner_curve(page, false, FUSE_R).bottom(px(-FUSE_R)))
        .child(accent_bracket(format!("{key}-rule")))
        .animated(
            ElementId::Name(key),
            Animation::new(Duration::from_millis(QUICK)).with_easing(ease_out_quint()),
            |el, d| el.left(px(6. * (1. - d))).opacity(d),
        )
}

/// Let a small detail, such as a count or a badge, settle in with a quick fade when it
/// appears or its value changes. Key it by its value.
pub(crate) fn pop(el: impl IntoElement + 'static, key: impl Into<SharedString>) -> AnyElement {
    div().relative().child(el).animated(
        ElementId::Name(key.into()),
        Animation::new(Duration::from_millis(QUICK)).with_easing(ease_out_quint()),
        |el, d| el.opacity(0.3 + 0.7 * d),
    )
}

/// A live status dot: the one thing in the shell that moves on its own, a slow, soft pulse
/// while an agent is working.
pub(crate) fn live_dot(color: u32, size: f32, key: impl Into<SharedString>) -> AnyElement {
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(rgb(color))
        .animated(
            ElementId::Name(key.into()),
            Animation::new(Duration::from_millis(1800))
                .repeat()
                .with_easing(pulsating_between(0.4, 1.)),
            |el, d| el.opacity(d),
        )
}

/// The fill under a chosen item, such as a segment, that fades in when the item is
/// chosen, with its own look. A child of the item, which must be `relative` and add it
/// first, under its content.
pub(crate) fn chosen_fill_of(key: impl Into<SharedString>, radius: f32, look: Div) -> AnyElement {
    look.absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right_0()
        .rounded(px(radius))
        .animated(
            ElementId::Name(key.into()),
            Animation::new(Duration::from_millis(QUICK)).with_easing(ease_out_quint()),
            |el, d| el.opacity(d),
        )
}

/// The rounded top corner of a sheet beside the rail, where it meets the rail under the
/// top bar: the corner of a square outside a quarter circle, in the rail's colour, laid
/// over the sheet's top-left. Empty in glass mode.
pub(crate) fn sheet_corner() -> Div {
    let fill = Hsla::from(rail_shade());
    div().absolute().top_0().left_0().size(px(SHEET_R)).child(
        canvas(
            |_, _, _| (),
            move |b, _, window, _| {
                let (r, k) = (SHEET_R, 0.5523);
                let p = |x: f32, y: f32| point(b.origin.x + px(x), b.origin.y + px(y));
                let mut f = PathBuilder::fill();
                f.move_to(p(0., 0.));
                f.line_to(p(r, 0.));
                f.cubic_bezier_to(p(0., r), p(r - k * r, 0.), p(0., r - k * r));
                f.close();
                if let Ok(path) = f.build() {
                    window.paint_path(path, fill);
                }
            },
        )
        .size_full(),
    )
}

/// The gap between the window's islands, in the frame's grey.
pub(crate) const GUTTER: f32 = 8.;
/// Radius of an island's rounded corners.
pub(crate) const ISLAND_R: f32 = 14.;
/// How far the frame's curves reach past an island's or tab's rounded edge, so their
/// antialiased rims overlap.
const SEAM: f32 = 0.75;
/// Radius of an open tab's top corners, and of the curves where it flows into its island.
const STRIP_TAB_R: f32 = 9.;
const STRIP_FLARE_R: f32 = 8.;

thread_local! {
    /// Where each fused strip's open tab sits this frame, keyed by strip.
    static STRIP_CUTS: std::cell::RefCell<HashMap<&'static str, Bounds<Pixels>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// A quarter turn from `from` to `to` round the square corner `c`.
fn turn(f: &mut PathBuilder, from: Point<Pixels>, c: Point<Pixels>, to: Point<Pixels>) {
    let k = 0.5523;
    f.cubic_bezier_to(to, from + (c - from) * k, to + (c - to) * k);
}

/// Marks the open tab of a fused strip, so the strip leaves it out of the frame's grey.
/// A child of the open tab, which must be `relative`.
pub(crate) fn strip_cut(strip: &'static str) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .child(
            canvas(
                move |b, _, _| STRIP_CUTS.with_borrow_mut(|c| c.insert(strip, b)),
                |_, _, _, _| {},
            )
            .size_full(),
        )
        .into_any_element()
}

/// The fill of a strip of tabs over an island: the frame's grey across the strip, less
/// the open tab, which is the island's own colour and flows into it with curves on both
/// sides, and the accent bracket on its leading corner. The grey and the tab are two
/// paths that never overlap, so the join looks the same in every glass style. A child
/// of the strip, which must be `relative`, added first so it sits under the tabs.
pub(crate) fn strip_fill(strip: &'static str, island: Rgba) -> AnyElement {
    let frame = Hsla::from(settings::frame());
    let island = Hsla::from(island);
    let accent = Hsla::from(rgb(c_blue()));
    div()
        .absolute()
        .inset_0()
        .child(
            canvas(
                move |_, _, _| STRIP_CUTS.with_borrow_mut(|c| c.remove(strip)),
                move |b, _, window, _| {
                    let at = |x: Pixels, y: Pixels| point(x, y);
                    let (left, top, right, bottom) =
                        (b.origin.x, b.origin.y, b.right(), b.bottom());
                    let (fr, tr) = (px(STRIP_FLARE_R), px(STRIP_TAB_R));
                    // A tab scrolled partly out of the strip is not cut.
                    let cut = STRIP_CUTS
                        .with_borrow(|c| c.get(strip).copied())
                        .filter(|t| t.origin.x - fr >= left && t.right() + fr <= right);
                    let mut f = PathBuilder::fill();
                    f.move_to(at(left, top));
                    f.line_to(at(right, top));
                    f.line_to(at(right, bottom));
                    if let Some(t) = cut {
                        let s = px(SEAM);
                        let (x0, x1, y0) = (t.origin.x + s, t.right() - s, t.origin.y.max(top) + s);
                        let (fr, tr) = (fr + s, tr - s);
                        f.line_to(at(x1 + fr, bottom));
                        turn(
                            &mut f,
                            at(x1 + fr, bottom),
                            at(x1, bottom),
                            at(x1, bottom - fr),
                        );
                        f.line_to(at(x1, y0 + tr));
                        turn(&mut f, at(x1, y0 + tr), at(x1, y0), at(x1 - tr, y0));
                        f.line_to(at(x0 + tr, y0));
                        turn(&mut f, at(x0 + tr, y0), at(x0, y0), at(x0, y0 + tr));
                        f.line_to(at(x0, bottom - fr));
                        turn(
                            &mut f,
                            at(x0, bottom - fr),
                            at(x0, bottom),
                            at(x0 - fr, bottom),
                        );
                    }
                    f.line_to(at(left, bottom));
                    f.close();
                    let frame_path = f.build();
                    let Some(t) = cut else {
                        if let Ok(path) = frame_path {
                            window.paint_path(path, frame);
                        }
                        return;
                    };
                    let (x0, x1, y0) = (t.origin.x, t.right(), t.origin.y.max(top));
                    // The tab goes down first and the frame over it, its hole a little
                    // inside the tab's outline: the two antialiased rims then overlap
                    // instead of meeting, which would leave a light seam round the tab in
                    // glass mode.
                    let tab = |mut f: PathBuilder| {
                        f.move_to(at(x0 - fr, bottom));
                        turn(
                            &mut f,
                            at(x0 - fr, bottom),
                            at(x0, bottom),
                            at(x0, bottom - fr),
                        );
                        f.line_to(at(x0, y0 + tr));
                        turn(&mut f, at(x0, y0 + tr), at(x0, y0), at(x0 + tr, y0));
                        f.line_to(at(x1 - tr, y0));
                        turn(&mut f, at(x1 - tr, y0), at(x1, y0), at(x1, y0 + tr));
                        f.line_to(at(x1, bottom - fr));
                        turn(
                            &mut f,
                            at(x1, bottom - fr),
                            at(x1, bottom),
                            at(x1 + fr, bottom),
                        );
                        f.close();
                        f.build()
                    };
                    if let Ok(path) = tab(PathBuilder::fill()) {
                        window.paint_path(path, island);
                    }
                    if let Ok(path) = frame_path {
                        window.paint_path(path, frame);
                    }
                    // The bracket: up the tab's left edge and round its top corner.
                    let (i, reach) = (px(1.), px(9.));
                    let mut f = PathBuilder::stroke(px(2.));
                    f.move_to(at(x0 + i, y0 + tr + reach));
                    f.line_to(at(x0 + i, y0 + tr));
                    turn(
                        &mut f,
                        at(x0 + i, y0 + tr),
                        at(x0 + i, y0 + i),
                        at(x0 + tr, y0 + i),
                    );
                    f.line_to(at(x0 + tr + reach, y0 + i));
                    if let Ok(path) = f.build() {
                        window.paint_path(path, accent);
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// One rounded corner of an island: the corner of a square outside a quarter circle, in
/// the frame's grey, so the island reads as rounded over whatever it holds. The island's
/// own fill is rounded too, so the two never overlap in glass mode.
pub(crate) fn frame_corner(top: bool, left: bool) -> AnyElement {
    let fill = Hsla::from(settings::frame());
    let r = ISLAND_R;
    div()
        .absolute()
        .size(px(r))
        .map(|el| if top { el.top_0() } else { el.bottom_0() })
        .map(|el| if left { el.left_0() } else { el.right_0() })
        .child(
            canvas(
                |_, _, _| (),
                move |b, _, window, _| {
                    let k = 0.5523;
                    // Drawn as the top-left corner, then mirrored into place.
                    let p = |x: f32, y: f32| {
                        let x = if left { x } else { r - x };
                        let y = if top { y } else { r - y };
                        point(b.origin.x + px(x), b.origin.y + px(y))
                    };
                    // The curve runs a little inside the island's own rounded edge, so
                    // the two antialiased rims overlap instead of meeting. Where they
                    // only meet, both are part-clear along the curve and the desktop
                    // shows through as a light seam in glass mode.
                    let q = r - SEAM;
                    let mut f = PathBuilder::fill();
                    f.move_to(p(0., 0.));
                    f.line_to(p(r, 0.));
                    f.line_to(p(r, r - q));
                    f.cubic_bezier_to(p(r - q, r), p(r - k * q, r - q), p(r - q, r - k * q));
                    f.line_to(p(0., r));
                    f.close();
                    if let Ok(path) = f.build() {
                        window.paint_path(path, fill);
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// An island on the frame: `content` on a fill with the given corners rounded, as
/// top-left, top-right, bottom-right and bottom-left. A transparent fill leaves the
/// layers to the content, as on pages that paint their own list beside the page.
pub(crate) fn island(content: impl IntoElement, fill: Rgba, round: [bool; 4]) -> Div {
    let r = px(ISLAND_R);
    let [tl, tr, br, bl] = round;
    div()
        .relative()
        .flex()
        .flex_col()
        .bg(fill)
        .when(tl, |el| el.rounded_tl(r))
        .when(tr, |el| el.rounded_tr(r))
        .when(br, |el| el.rounded_br(r))
        .when(bl, |el| el.rounded_bl(r))
        .child(content)
        .when(tl, |el| el.child(frame_corner(true, true)))
        .when(tr, |el| el.child(frame_corner(true, false)))
        .when(br, |el| el.child(frame_corner(false, false)))
        .when(bl, |el| el.child(frame_corner(false, true)))
}

/// One tab in a strip over an island. The open one is cut from the island below
/// (see `strip_fill`); the rest sit quietly on the frame until hovered.
pub(crate) fn strip_tab(
    id: impl Into<ElementId>,
    strip: &'static str,
    selected: bool,
) -> Stateful<Div> {
    let id = id.into();
    let tab = div()
        .id(id.clone())
        .relative()
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM);
    if selected {
        tab.h(px(32.))
            .child(strip_cut(strip))
            .text_color(rgb(c_text()))
    } else {
        let (rest, hover) = tab_chip();
        hover_wash(tab, id, hover, 8.)
            .h(px(28.))
            .mb(px(3.))
            .rounded(px(8.))
            .bg(rest)
            .text_color(rgb(c_secondary()))
            .hover(|el| el.text_color(rgb(c_text())))
            .active(|el| el.opacity(0.75))
    }
}

/// The header of a list beside the rail: a large title over a quiet summary line,
/// with room on the right for the list's add button.
pub(crate) fn list_header(title: &'static str, summary: impl IntoElement) -> Div {
    div()
        .pt(px(16.))
        .px(px(16.))
        .flex()
        .items_start()
        .justify_between()
        .child(
            div()
                .v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(19.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_text()))
                        .child(title),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(summary),
                ),
        )
}

/// The round, filled add button in a `list_header`.
pub(crate) fn add_button(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .size(px(30.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(c_text()))
        .cursor_pointer()
        .hover(|el| el.opacity(0.85))
        .active(|el| el.opacity(0.7))
        .child(icon(IconName::Plus, 16., c_canvas()))
}

/// A strip of the frame's grey, such as the gutter between two islands.
pub(crate) fn gutter() -> Div {
    div().flex_none().bg(settings::frame())
}

/// A card on an island: a borderless step lighter than the page, so groups read by
/// tone rather than by lines.
pub(crate) fn card() -> Div {
    div().rounded(px(12.)).bg(tint(c_subtle()))
}

/// A hairline between rows inside a `card`, inset from its edges.
pub(crate) fn card_rule() -> Div {
    div().mx_4().h(px(1.)).flex_none().bg(divider())
}

/// Let a list's rows fade in one after another when the list opens, each a beat after
/// the one above it.
pub(crate) fn cascade(
    el: impl IntoElement + 'static,
    key: impl Into<SharedString>,
    i: usize,
) -> AnyElement {
    let lead = i.min(8) as f32 * 0.04;
    div().relative().child(el).animated(
        ElementId::Name(key.into()),
        Animation::new(Duration::from_millis(QUICK + 20 * i.min(8) as u64))
            .with_easing(after(lead)),
        |el, d| el.opacity(d),
    )
}

thread_local! {
    /// Whether the pointer was over each hover-washed element when it last painted, and
    /// since when, keyed by id.
    static HOVERS: std::cell::RefCell<HashMap<ElementId, (bool, std::time::Instant)>> =
        std::cell::RefCell::new(HashMap::new());
}

/// How long a hover wash takes to ease in or out.
const HOVER_MS: f32 = 120.;

/// Give an element a hover fill that eases in and out instead of snapping: `color`
/// washes up under its content while the pointer is over it. Call it before adding the
/// element's content, so the wash sits beneath it, and keep `on_hover` free for it. The
/// element must be `relative`.
///
/// Whether the pointer is over it is read from the pointer itself as the wash paints,
/// so an element that went away while hovered never comes back lit.
pub(crate) fn hover_wash(
    el: Stateful<Div>,
    id: impl Into<ElementId>,
    color: Rgba,
    radius: f32,
) -> Stateful<Div> {
    let id: ElementId = id.into();
    let color = Hsla::from(color);
    el.on_hover(|_, window, _| window.refresh()).child(
        canvas(
            |_, _, _| (),
            move |b, _, window, _| {
                let over = b.contains(&window.mouse_position());
                let now = std::time::Instant::now();
                let since = HOVERS.with_borrow_mut(|h| {
                    // First seen: already settled, so nothing fades on arrival.
                    let seen = h
                        .entry(id.clone())
                        .or_insert((over, now - Duration::from_secs(1)));
                    if seen.0 != over {
                        *seen = (over, now);
                    }
                    seen.1.elapsed().as_secs_f32() * 1000.
                });
                let t = if anim_on() {
                    (since / HOVER_MS).min(1.)
                } else {
                    1.
                };
                if t < 1. {
                    window.request_animation_frame();
                }
                let t = 1. - (1. - t).powi(3);
                let strength = if over { t } else { 1. - t };
                if strength > 0. && color.a > 0. {
                    window.paint_quad(fill(b, color.opacity(strength)).corner_radii(px(radius)));
                }
            },
        )
        .absolute()
        .inset_0(),
    )
}

thread_local! {
    /// Whether the pointer is over each hover group that reveals actions, and since when.
    static GROUPS: std::cell::RefCell<HashMap<ElementId, (bool, std::time::Instant)>> =
        std::cell::RefCell::new(HashMap::new());
}

/// How long actions take to fade in when their row is hovered.
const REVEAL_MS: u64 = 140;

/// Mark a row whose actions show on hover, so `reveal` can fade them in rather than
/// have them pop. Keep `on_hover` free on the row for it.
pub(crate) fn reveal_group(el: Stateful<Div>, id: impl Into<ElementId>) -> Stateful<Div> {
    let id = id.into();
    el.on_hover(move |hovered, window, _| {
        GROUPS.with_borrow_mut(|g| {
            g.insert(id.clone(), (*hovered, std::time::Instant::now()));
        });
        window.refresh();
    })
}

/// An action that shows while its row is hovered (with `group_hover`, which still decides
/// whether it shows at all), faded in over a moment each time the pointer arrives.
/// `group` is the id the row was given in `reveal_group`.
pub(crate) fn reveal(el: impl IntoElement + 'static, group: impl Into<ElementId>) -> AnyElement {
    let group = group.into();
    let since = GROUPS.with_borrow(|g| g.get(&group).filter(|(on, _)| *on).map(|(_, at)| *at));
    div().flex().child(el).animated(
        ElementId::Name(SharedString::from(format!("reveal-{group}-{since:?}"))),
        Animation::new(Duration::from_millis(REVEAL_MS)).with_easing(ease_out_quint()),
        move |el, d| if since.is_some() { el.opacity(d) } else { el },
    )
}

/// The places on the mode rail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Home,
    Sessions,
    Design,
    Architecture,
    /// What waits to ship and the project's GitHub, as two tabs of one place.
    Git,
    Playbooks,
    History,
    Inbox,
    Notes,
    Settings,
}

impl Mode {
    /// The modes at the top of the rail, in order. The first `WORK` are the day's work;
    /// the rest, what's been kept, sit apart.
    pub(crate) const MAIN: [Mode; 5] = [
        Mode::Home,
        Mode::Sessions,
        Mode::Git,
        Mode::Playbooks,
        Mode::History,
    ];

    /// How many of `MAIN` stay out on the rail; the rest wait in its More menu.
    const WORK: usize = 3;

    /// The places in the rail's More menu, used less than the ones out on the rail.
    pub(crate) const MORE: [Mode; 3] = [Mode::Playbooks, Mode::History, Mode::Notes];

    /// The places Settings > General > Rail can pin out on the rail, in rail order.
    pub(crate) const PINNABLE: [Mode; 4] =
        [Mode::Playbooks, Mode::History, Mode::Notes, Mode::Inbox];

    /// The current project's own modes, grouped under its badge below the main ones; ⌘7
    /// and ⌘8.
    pub(crate) const PLAN: [Mode; 2] = [Mode::Design, Mode::Architecture];

    /// Whether the mode shows one project at a time, or a chooser under All projects.
    pub(crate) fn per_project(self) -> bool {
        Self::PLAN.contains(&self)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Home => "Home",
            Mode::Sessions => "Sessions",
            Mode::Design => "Design",
            Mode::Architecture => "Architecture",
            Mode::Git => "Git",
            Mode::Playbooks => "Playbooks",
            Mode::History => "History",
            Mode::Inbox => "Inbox",
            Mode::Notes => "Notes",
            Mode::Settings => "Settings",
        }
    }

    /// The shortcut that opens this mode, as Settings > Keybindings names it.
    fn shortcut(self) -> &'static str {
        match self {
            Mode::Home => "go-home",
            Mode::Sessions => "go-sessions",
            Mode::Design => "go-design",
            Mode::Architecture => "go-architecture",
            // Git's Changes tab; `go-reviews` opens its GitHub tab.
            Mode::Git => "go-review",
            Mode::Playbooks => "go-playbooks",
            Mode::History => "go-history",
            Mode::Inbox => "go-inbox",
            Mode::Notes => "go-notes",
            Mode::Settings => "settings",
        }
    }

    pub(crate) fn glyph(self) -> IconName {
        match self {
            Mode::Home => IconName::House,
            Mode::Sessions => IconName::CodeXml,
            Mode::Design => IconName::PanelsTopLeft,
            Mode::Architecture => IconName::Network,
            Mode::Git => IconName::GitPullRequest,
            Mode::Playbooks => IconName::BookOpen,
            Mode::History => IconName::RotateCcwClock,
            Mode::Inbox => IconName::Inbox,
            Mode::Notes => IconName::NotebookPen,
            Mode::Settings => IconName::Settings,
        }
    }

    /// The mode a view belongs to.
    pub(crate) fn of(view: View) -> Mode {
        match view {
            View::Home => Mode::Home,
            View::Design => Mode::Design,
            View::Architecture => Mode::Architecture,
            View::Automations => Mode::Playbooks,
            View::Review | View::GitHistory | View::Branches | View::GitHub => Mode::Git,
            View::History => Mode::History,
            View::Inbox => Mode::Inbox,
            View::Notes => Mode::Notes,
            View::Settings => Mode::Settings,
            View::Session
            | View::NewSession
            | View::AllSessions
            | View::Worktrees
            | View::Activity
            | View::Editor => Mode::Sessions,
        }
    }
}

/// Where a session sits on Home and in the sidebar's status groups. The same four words
/// are used everywhere: Needs you, Running, Ready to review and Done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Bucket {
    NeedsYou,
    Running,
    Review,
    Done,
}

impl Bucket {
    pub(crate) const ALL: [Bucket; 4] = [
        Bucket::NeedsYou,
        Bucket::Running,
        Bucket::Review,
        Bucket::Done,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Bucket::NeedsYou => "Needs you",
            Bucket::Running => "Running",
            Bucket::Review => "Ready to review",
            Bucket::Done => "Done",
        }
    }
}

/// Why a session needs the person.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Need {
    /// Paused on a permission, a question or a plan to approve.
    Blocked,
    /// Failed while the person was looking elsewhere.
    Failed,
    /// Finished while the person was looking elsewhere.
    Finished,
}

impl Need {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Need::Blocked => "Waiting on you",
            Need::Failed => "Failed",
            Need::Finished => "Finished",
        }
    }
}

/// The one rule for "needs you", shared by the bell, the rail, Home, the sidebar and the
/// Inbox. A blocked agent needs the person until it is answered or its card dismissed; a
/// finished or failed one until its session is opened, which clears `unread`.
pub(crate) fn need_of(
    s: &Session,
    running: bool,
    hidden: &std::collections::HashSet<String>,
) -> Option<Need> {
    if s.archived || running {
        return None;
    }
    if let Some((index, _)) = alerts::waiting_on(s) {
        if !hidden.contains(&format!("s:{}:{index}", s.id)) {
            return Some(Need::Blocked);
        }
    }
    match (s.unread, s.status) {
        (true, Status::Failed) => Some(Need::Failed),
        (true, _) => Some(Need::Finished),
        _ => None,
    }
}

/// The id `step` places after `at` in `list`; with `wrap`, past either end comes round.
/// With nothing picked yet, moving down picks the first and moving up the last.
pub(crate) fn step_in(list: &[u64], at: Option<u64>, step: isize, wrap: bool) -> Option<u64> {
    if list.is_empty() {
        return None;
    }
    let n = list.len() as isize;
    let next = match at.and_then(|id| list.iter().position(|x| *x == id)) {
        None if step < 0 => n - 1,
        None => 0,
        Some(i) if wrap => (i as isize + step).rem_euclid(n),
        Some(i) => (i as isize + step).clamp(0, n - 1),
    };
    list.get(next as usize).copied()
}

/// Move tab `id` to where `target` is, shifting the others along.
pub(crate) fn move_tab(tabs: &mut Vec<u64>, id: u64, target: u64) {
    let (Some(from), Some(to)) = (
        tabs.iter().position(|t| *t == id),
        tabs.iter().position(|t| *t == target),
    ) else {
        return;
    };
    let tab = tabs.remove(from);
    tabs.insert(to, tab);
}

/// Totals shown in the top bar and on Home.
#[derive(Default)]
pub(crate) struct Counts {
    pub running: usize,
    /// Sessions blocked on the person, or finished or failed unseen.
    pub needs_you: usize,
    /// Idle sessions with uncommitted changes.
    pub review: usize,
    /// Spend reported by the agents today.
    pub spend_today: f64,
    /// Spend over every session ever, for the tooltip.
    pub spend_cents: f64,
}

/// A tab being dragged to a new place in the strip.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub id: u64,
    pub title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(divider())
            .bg(rgb(c_raised()))
            .shadow_md()
            .text_size(px(12.5))
            .text_color(rgb(c_text()))
            .child(self.title.clone())
    }
}

impl RunoApp {
    /// The mode of what is on screen; the New session dialog keeps the mode behind it.
    pub(crate) fn mode(&self) -> Mode {
        match self.view {
            View::NewSession => Mode::of(self.new_back),
            view => Mode::of(view),
        }
    }

    pub(crate) fn go(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.cursor = None;
        match mode {
            Mode::Home => self.view = View::Home,
            Mode::Sessions => {
                self.view = if self.current().is_some() {
                    View::Session
                } else {
                    View::AllSessions
                }
            }
            Mode::Design => self.view = View::Design,
            Mode::Architecture => self.view = View::Architecture,
            Mode::Git => match self.git_tab {
                View::GitHub => self.load_github(),
                View::Branches => self.open_branches(),
                View::GitHistory => self.open_git_history(),
                _ => self.open_review(),
            },
            Mode::Playbooks => self.view = View::Automations,
            Mode::History => self.open_history(cx),
            Mode::Inbox => self.view = View::Inbox,
            Mode::Notes => self.open_notes(window, cx),
            Mode::Settings => self.open_settings(),
        }
        self.save();
        window.focus(&self.focus, cx);
    }

    /// Whether the session's agent is running now.
    pub(crate) fn is_running(&self, s: &Session) -> bool {
        self.started.contains_key(&s.id)
            || s.status == Status::Working
            || self.handles.contains_key(&Target::Session(s.id))
    }

    pub(crate) fn need(&self, s: &Session) -> Option<Need> {
        // A running agent paused on a permission request needs the person now.
        if !s.archived
            && self
                .approval_for(s.id)
                .is_some_and(|r| !self.alerts_hidden.contains(&format!("a:{}", r.id)))
        {
            return Some(Need::Blocked);
        }
        need_of(s, self.is_running(s), &self.alerts_hidden)
    }

    /// Sessions that need the person, blocked ones first, then the newest.
    pub(crate) fn needs_you(&self) -> Vec<(&Session, Need)> {
        let mut out: Vec<(&Session, Need)> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.canvas.is_empty())
            .filter_map(|s| self.need(s).map(|n| (s, n)))
            .collect();
        out.sort_by_key(|(s, n)| (*n != Need::Blocked, std::cmp::Reverse(s.id)));
        out
    }

    /// Where a session sits on Home and in the counts. Canvas chats sit on their canvas.
    pub(crate) fn bucket(&self, s: &Session) -> Option<Bucket> {
        if s.archived || !s.canvas.is_empty() {
            None
        } else if self.need(s).is_some() {
            Some(Bucket::NeedsYou)
        } else if self.is_running(s) {
            Some(Bucket::Running)
        } else if self.stats.get(&s.id).is_some_and(|st| st.0 > 0) {
            Some(Bucket::Review)
        } else {
            Some(Bucket::Done)
        }
    }

    /// Whether Home shows this session: every project's with "All projects", else only the
    /// top bar's project.
    pub(crate) fn on_home(&self, s: &Session) -> bool {
        self.home_all || Some(s.project_id) == self.current_project()
    }

    pub(crate) fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for s in &self.state.sessions {
            match self.bucket(s) {
                Some(Bucket::NeedsYou) => counts.needs_you += 1,
                Some(Bucket::Running) => counts.running += 1,
                Some(Bucket::Review) => counts.review += 1,
                _ => {}
            }
        }
        counts.spend_cents = self
            .state
            .sessions
            .iter()
            .flat_map(|s| s.messages.iter())
            .map(|m| m.usage.cost_cents)
            .sum();
        counts.spend_today = self
            .state
            .spend
            .get(&store::today())
            .copied()
            .unwrap_or_default();
        counts
    }

    /// Note a finished turn's cost and tokens under today, for the top bar's spend and
    /// the daily limits.
    pub(crate) fn record_spend(&mut self, id: u64) {
        let usage = self
            .state
            .session(id)
            .and_then(|s| s.messages.iter().rev().find(|m| m.role == Role::Agent))
            .map(|m| m.usage)
            .unwrap_or_default();
        let today = store::today();
        self.state.add_spend(&today, usage.cost_cents);
        self.state
            .add_tokens(&today, crate::health::turn_tokens(&usage));
    }

    // ---------- keyboard ----------

    /// Ctrl+Tab and friends: the next or previous tab, coming round at the ends.
    pub(crate) fn step_tab(&mut self, step: isize) {
        let at = self.selected.filter(|_| self.view == View::Session);
        if let Some(id) = step_in(&self.tabs, at.or(self.selected), step, at.is_some()) {
            self.select(id);
        }
    }

    pub(crate) fn toggle_panel(&mut self) {
        self.show_panel = !self.show_panel;
        self.panel_auto_hidden = false;
        self.save();
    }

    /// Put the cursor in the open session's message box, opening Sessions if needed.
    pub(crate) fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current().is_none() {
            return;
        }
        if self.view != View::Session {
            self.view = View::Session;
        }
        self.prompt.update(cx, |p, cx| p.focus(window, cx));
    }

    /// Shift+F10: the right-click menu of the highlighted or open session, from the keyboard.
    pub(crate) fn keyboard_context_menu(&mut self, window: &Window) {
        let target = match (self.cursor, self.selected) {
            (Some(id), _) => ctxmenu::CtxTarget::Session(id),
            (None, Some(id)) if self.view == View::Session => ctxmenu::CtxTarget::Tab(id),
            (None, Some(id)) => ctxmenu::CtxTarget::Session(id),
            _ => return,
        };
        let size = window.viewport_size();
        let at = point(size.width / 2. - px(110.), px(120.));
        self.ctx_menu = Some((at, target));
        self.ctx_index = Some(0);
    }

    /// Esc twice in a terminal or agent CLI hands the keyboard back to Runo; the first Esc
    /// still reaches the program. Returns whether the key was used.
    pub(crate) fn leave_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !(self.term_focus.is_focused(window) || self.cli_focus.is_focused(window)) {
            self.term_escape = None;
            return false;
        }
        let now = std::time::Instant::now();
        match self.term_escape.take() {
            Some(at) if now.duration_since(at) < Duration::from_millis(600) => {
                window.focus(&self.focus, cx);
                self.info = Some((
                    "Left the terminal. Shortcuts work again; click it to type there.".into(),
                    now,
                ));
                cx.notify();
                true
            }
            _ => {
                self.term_escape = Some(now);
                false
            }
        }
    }

    /// The rows the arrow keys move through on this screen, top to bottom.
    pub(crate) fn nav_list(&self, cx: &App) -> Vec<u64> {
        match self.view {
            View::Inbox => self.needs_you().into_iter().map(|(s, _)| s.id).collect(),
            View::Home => self.home_recent().into_iter().map(|s| s.id).collect(),
            _ if self.mode() == Mode::Sessions && self.show_sidebar => {
                let filter = self.search.read(cx).value().to_lowercase();
                self.sidebar_rows(&filter, cx)
                    .into_iter()
                    .flat_map(|(_, rows)| rows.into_iter().map(|s| s.id))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Arrow keys, Enter and the menu key on lists, when no text box has the keyboard.
    /// Returns whether the key was used.
    pub(crate) fn list_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ctx_menu.is_some() {
            return self.ctx_key(&keystroke.key, window, cx);
        }
        if !self.focus.is_focused(window)
            || self.palette.is_some()
            || self.tour.is_some()
            || keystroke.modifiers.modified()
        {
            return false;
        }
        match keystroke.key.as_str() {
            "up" | "down" => {
                let list = self.nav_list(cx);
                let at = self.cursor.or(self.selected.filter(|id| list.contains(id)));
                let step = if keystroke.key == "up" { -1 } else { 1 };
                self.cursor = step_in(&list, at, step, false);
                self.cursor.is_some()
            }
            "enter" => match self.cursor.take() {
                Some(id) => {
                    self.select(id);
                    true
                }
                None => false,
            },
            "menu" => {
                self.keyboard_context_menu(window);
                true
            }
            _ => false,
        }
    }

    // ---------- mode rail ----------

    fn rail_item(&self, mode: Mode, badge: Option<u32>, cx: &mut Context<Self>) -> AnyElement {
        let on = self.mode() == mode;
        let name = match self.scoped_project() {
            Some(p) if mode.per_project() => format!("{} · {}", mode.label(), p.name),
            _ => mode.label().to_string(),
        };
        let tip = match keymap::label(cx, mode.shortcut()) {
            keys if keys.is_empty() => name,
            keys => format!("{name}  {keys}"),
        };
        let id = SharedString::from(format!("rail-{}", mode.label()));
        let item = div()
            .id(id.clone())
            .relative()
            .size(px(34.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .active(|el| el.opacity(0.7));
        // The open place shows its tab instead, so its wash stays clear.
        let wash = if on { rgba(0) } else { tint(c_subtle()) };
        hover_wash(item, id, wash, 8.)
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.go(mode, window, cx);
                cx.notify();
            }))
            .when(on, |el| {
                // The tab is cut from the list beside the rail, or from the page itself
                // where nothing sits between them.
                el.child(if self.has_panel(cx) {
                    rail_tab(mode.label(), pane(c_sheet()))
                } else {
                    rail_tab(mode.label(), glass_fill(c_canvas()))
                })
            })
            .child(rail_icon(mode.glyph(), on))
            // A small dot cut into the icon's corner, ringed in the rail's own colour.
            .when_some(badge.filter(|_| !on), |el, color| {
                el.child(
                    div()
                        .absolute()
                        .top(px(5.))
                        .right(px(5.))
                        .size(px(8.))
                        .rounded_full()
                        .border_2()
                        .border_color(rgb(c_rail()))
                        .bg(rgb(color)),
                )
            })
            .into_any_element()
    }

    /// Whether the open place has a list beside the rail for its tab to join: the
    /// sessions sidebar, the review list, Settings' pages, or Design's layers.
    pub(crate) fn has_panel(&self, cx: &App) -> bool {
        match self.mode() {
            Mode::Sessions => self.show_sidebar,
            Mode::Git => matches!(self.view, View::Review | View::GitHistory | View::Branches),
            Mode::Design => self
                .design
                .project
                .and_then(|p| self.design.canvases.get(&p))
                .is_some_and(|canvas| canvas.read(cx).layers_shown()),
            // The diagram editor's shape palette sits against the rail; the code map and
            // the project chooser have nothing there.
            Mode::Architecture => {
                !self.all_scope()
                    && !self.arch.code_map
                    && self
                        .arch
                        .project
                        .is_some_and(|p| self.arch.editors.contains_key(&p))
            }
            _ => false,
        }
    }

    pub(crate) fn mode_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        TAB_SHEET.set(None);
        let counts = self.counts();
        // The groups part by space alone, so the rail reads as a few calm clusters.
        // Room between the items for the open tab's curves, so they never crowd a neighbour.
        let group = || div().v_flex().items_center().gap(px(8.));
        let mut work = group().relative().child(tour::anchor(&self.anchors, "nav"));
        for mode in &Mode::MAIN[..Mode::WORK] {
            // Amber: something there needs you. Blue: agents are working, or work waits
            // to be shipped.
            let badge = match mode {
                Mode::Sessions if counts.needs_you > 0 => Some(c_amber()),
                Mode::Sessions if counts.running > 0 => Some(c_blue()),
                Mode::Git if self.gh.needs_you() > 0 => Some(c_amber()),
                Mode::Git if counts.review > 0 => Some(c_blue()),
                _ => None,
            };
            work = work.child(self.rail_item(*mode, badge, cx));
        }
        // Places pinned from Settings follow the main ones, in a fixed order.
        for mode in Mode::PINNABLE {
            if cx.global::<AppSettings>().pinned(mode.label()) {
                work = work.child(self.rail_item(mode, None, cx));
            }
        }
        let foot = group().child(self.rail_more(cx)).child(
            div()
                .relative()
                .child(tour::anchor(&self.anchors, "settings"))
                .child(self.rail_item(Mode::Settings, None, cx)),
        );
        // The Inbox opens from the bell in the top bar, so the rail doesn't repeat it.
        div()
            .w(px(RAIL_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .items_center()
            .pt(px(10.))
            // The foot's open tab ends level with the islands' bottom edge.
            .pb(px(GUTTER + TAB_PAD))
            .gap(px(18.))
            .relative()
            // Keeps the open tab inside the rail, under the top bar.
            .overflow_hidden()
            .child(rail_fill(
                TAB_SHEET.get().unwrap_or_else(|| pane(c_sheet())),
            ))
            .child(work)
            .child(self.project_group(cx))
            .child(div().flex_1())
            .child(foot)
            .into_any_element()
    }

    /// The ⋯ at the foot of the rail: Playbooks, History and Notes in a menu beside it.
    /// It lights up while one of them is open.
    fn rail_more(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.menu.as_deref() == Some("rail-more");
        let st = cx.global::<AppSettings>();
        let unpinned: Vec<Mode> = Mode::MORE
            .into_iter()
            .filter(|m| !st.pinned(m.label()))
            .collect();
        // Everything it held is pinned out on the rail.
        if unpinned.is_empty() {
            return div().into_any_element();
        }
        let on = unpinned.contains(&self.mode());
        let tip: SharedString = unpinned
            .iter()
            .map(|m| m.label())
            .collect::<Vec<_>>()
            .join(", ")
            .into();
        let mut list = div().v_flex().p_1();
        for mode in unpinned {
            let here = self.mode() == mode;
            list = list.child(
                div()
                    .id(SharedString::from(format!("rail-more-{}", mode.label())))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(here, |el| el.bg(tint(c_active())))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.go(mode, window, cx);
                        cx.notify();
                    }))
                    .child(icon(mode.glyph(), 15., c_secondary()))
                    .child(div().flex_1().child(mode.label()))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(keymap::label(cx, mode.shortcut())),
                    ),
            );
        }
        div()
            .relative()
            .child(
                div()
                    .id("rail-more")
                    .size(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(open, |el| el.bg(tint(c_active())))
                    .when(!(on || open), |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .when(on, |el| el.child(rail_tile("more")))
                    .when(!open, |el| {
                        el.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = match this.menu.as_deref() {
                            Some("rail-more") => None,
                            _ => Some("rail-more".into()),
                        };
                        cx.notify();
                    }))
                    .child(rail_icon(IconName::Ellipsis, on)),
            )
            .when(open, |el| {
                el.child(
                    div().absolute().left(px(46.)).bottom_0().child(
                        deferred(
                            anchored()
                                .anchor(Anchor::BottomLeft)
                                .snap_to_window_with_margin(px(8.))
                                .child(
                                    div()
                                        .id("rail-more-menu")
                                        .w(px(200.))
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(float_edge())
                                        .bg(float(c_raised()))
                                        .shadow_lg()
                                        .occlude()
                                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                            this.menu = None;
                                            cx.notify();
                                        }))
                                        .child(list),
                                ),
                        )
                        .with_priority(2),
                    ),
                )
            })
            .into_any_element()
    }

    /// Design and Architecture under the badge of the project they show, so they read as
    /// that project's pages. The badge opens the project switcher.
    fn project_group(&self, cx: &mut Context<Self>) -> AnyElement {
        let project = self.scoped_project();
        let tip: SharedString = match project {
            Some(p) => format!("{}: switch project", p.name).into(),
            None => "All projects: switch project".into(),
        };
        let badge = div()
            .id("rail-project")
            .size(px(34.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| {
                this.menu = Some("project-switch".into());
                cx.notify();
            }))
            .child(match project {
                Some(p) => project_mark(p, 18.).into_any_element(),
                None => icon(IconName::LayoutGrid, 15., c_muted()).into_any_element(),
            });
        // A short rule above the badge marks where the project's own pages begin.
        let mut group = div()
            .v_flex()
            .items_center()
            .gap(px(4.))
            .child(div().mb(px(12.)).w(px(18.)).h(px(1.)).bg(divider()))
            .child(badge);
        for mode in Mode::PLAN {
            group = group.child(self.rail_item(mode, None, cx));
        }
        group.into_any_element()
    }

    /// The project the per-project modes show: none under All projects.
    pub(crate) fn scoped_project(&self) -> Option<&Project> {
        if self.all_scope() {
            return None;
        }
        self.current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
    }

    /// All projects is picked and the page on screen has a view of every project: Home's
    /// lists, or the project chooser on Design and Architecture.
    pub(crate) fn all_scope(&self) -> bool {
        self.home_all && (self.mode() == Mode::Home || self.mode().per_project())
    }

    // ---------- top bar ----------

    fn bar_pill(&self, id: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(26.))
            .px(px(9.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .rounded_full()
            .border_1()
            .border_color(divider())
            .text_size(px(12.))
            .text_color(rgb(c_secondary()))
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
    }

    /// The project switcher's list, opened under its chip.
    pub(crate) fn project_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.current_project();
        let mut list = div().v_flex().p_1().child(
            div()
                .id("project-all")
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(13.))
                .hover(|el| el.bg(tint(c_active())))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.menu = None;
                    this.home_all = true;
                    // Design and Architecture stay put and offer every project to pick.
                    if this.mode().per_project() {
                        this.save();
                    } else {
                        this.go(Mode::Home, window, cx);
                    }
                    cx.notify();
                }))
                .child(icon(IconName::LayoutGrid, 15., c_secondary()))
                .child(div().flex_1().child("All projects"))
                .when(self.all_scope(), |el| {
                    el.child(icon(IconName::Check, 13., c_text()))
                }),
        );
        for project in &self.state.projects {
            let pid = project.id;
            let missing = !self.demo && !std::path::Path::new(&project.path).is_dir();
            let sessions = self
                .state
                .sessions
                .iter()
                .filter(|s| s.project_id == pid && !s.archived && s.canvas.is_empty())
                .count();
            list = list.child(
                div()
                    .id(("project-pick", pid as usize))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        this.switch_project(pid, window, cx);
                        cx.notify();
                    }))
                    .child(project_mark(project, 18.))
                    .child(div().flex_1().truncate().child(project.name.clone()))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(if missing { c_amber() } else { c_muted() }))
                            .child(match sessions {
                                _ if missing => "folder missing".into(),
                                0 => String::new(),
                                1 => "1 session".into(),
                                n => format!("{n} sessions"),
                            }),
                    )
                    .when(current == Some(pid) && !self.all_scope(), |el| {
                        el.child(icon(IconName::Check, 13., c_text()))
                    }),
            );
        }
        // How the person works in the project the top bar shows: through Crew, or directly.
        if let Some(pid) = current.filter(|_| !self.all_scope()) {
            list = list.child(div().h(px(1.)).my_1().bg(divider())).child(
                div()
                    .px_2()
                    .py_1()
                    .v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child("How you work in this project"),
                    )
                    .child(self.crew_mode_switch(pid, cx)),
            );
        }
        list = list.child(div().h(px(1.)).my_1().bg(divider())).child(
            div()
                .id("project-add")
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(rgb(c_secondary()))
                .hover(|el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    this.pick_project(cx);
                }))
                .child(icon(IconName::FolderPlus, 14., c_muted()))
                .child("New project…"),
        );
        div()
            .absolute()
            .top(px(34.))
            .left_0()
            .child(
                deferred(
                    anchored().snap_to_window_with_margin(px(8.)).child(
                        div()
                            .id("project-menu")
                            .w(px(280.))
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .rounded_lg()
                            .border_1()
                            .border_color(float_edge())
                            .bg(float(c_raised()))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            }))
                            .child(list),
                    ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    /// Switch the top bar to a project. Sessions opens its latest session; every other
    /// mode stays where it is and shows that project. A project without sessions lands on
    /// its Home with the New session form open.
    pub(crate) fn switch_project(&mut self, pid: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.project = Some(pid);
        self.new_project = Some(pid);
        self.home_all = false;
        self.cursor = None;
        if !self.expanded.contains(&pid) {
            self.expanded.push(pid);
        }
        let latest = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid && !s.archived && s.canvas.is_empty())
            .map(|s| s.id)
            .max();
        match (self.mode(), latest) {
            (Mode::Git, _) if self.view == View::GitHub => self.load_github(),
            (Mode::Git, _) if self.view == View::GitHistory => {
                self.git_commit_sel = None;
                self.git_log_filter.path.clear();
                self.open_git_history();
            }
            (Mode::Git, _) if self.view == View::Branches => {
                self.git_branch_sel = None;
                self.open_branches();
            }
            (Mode::Git, _) => self.load_repo(),
            (Mode::Sessions, Some(id)) => self.select(id),
            (Mode::Sessions, None) => {
                self.view = View::Home;
                self.open_new_session();
            }
            _ => {}
        }
        self.save();
        window.focus(&self.focus, cx);
    }

    /// Where the open session's agent runs, as the top bar and status bar show it: the
    /// name and a dot that is green when reachable, amber while unknown, red when not.
    pub(crate) fn runs_on(&self) -> (String, u32, String) {
        let conn = match self.current() {
            Some(s) => s.connection,
            None => self
                .current_project()
                .map(|pid| self.project_connection(pid))
                .unwrap_or(connections::LOCAL),
        };
        if conn == connections::LOCAL {
            return (
                "Local".into(),
                c_green(),
                "Agents run on this computer".into(),
            );
        }
        let Some(c) = connections::get(conn) else {
            return (
                "Removed connection".into(),
                c_red(),
                "This session's connection was removed. Move it somewhere else.".into(),
            );
        };
        let (color, state) = match connections::test_result(conn) {
            Some((Some(true), _)) => (c_green(), "reachable at the last test"),
            Some((Some(false), _)) => (c_red(), "failed its last test"),
            Some((None, _)) => (c_amber(), "being tested"),
            None => (c_amber(), "not tested yet"),
        };
        let tip = format!("Runs on {}, {state}. Click to move it.", c.name);
        (c.name, color, tip)
    }

    /// The popover under the Agents pill: what runs now, to open or stop.
    fn agents_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().v_flex().p_1();
        let running: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| self.is_running(s))
            .collect();
        if running.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child("No agents are working right now."),
            );
        }
        for s in running {
            let id = s.id;
            let provider = agent::provider(&s.provider);
            let when = self
                .started
                .get(&id)
                .map_or("Working".to_string(), |t| elapsed(t.elapsed()));
            list = list.child(
                div()
                    .id(("agents-row", id as usize))
                    .h(px(34.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.select(id);
                        cx.notify();
                    }))
                    .child(self.provider_mark(provider, 13.))
                    .child(div().flex_1().min_w_0().truncate().child(s.title.clone()))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(when),
                    )
                    .when(self.handles.contains_key(&Target::Session(id)), |el| {
                        el.child(
                            self.ghost_button_sized(
                                ("agents-stop", id as usize),
                                IconName::CircleStop,
                                "Stop",
                                24.,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.stop_session(id);
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            );
        }
        self.popover("agents-menu", 320., list, cx)
    }

    /// A floating list under a top-bar control that closes when clicked outside.
    fn popover(
        &self,
        id: &'static str,
        width: f32,
        list: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .absolute()
            .top(px(34.))
            .left_0()
            .child(
                deferred(
                    anchored().snap_to_window_with_margin(px(8.)).child(
                        div()
                            .id(id)
                            .w(px(width))
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .rounded_lg()
                            .border_1()
                            .border_color(float_edge())
                            .bg(float(c_raised()))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            }))
                            .child(list),
                    ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    pub(crate) fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self.counts();
        let inbox = counts.needs_you + self.gh.needs_you();
        let lead = if cfg!(target_os = "macos") { 80. } else { 8. };
        let project = self.scoped_project();
        let mode = self.mode();
        // The branch and session crumb belong to the open session, when it is this project's.
        let session = self
            .current()
            .filter(|s| mode == Mode::Sessions && Some(s.project_id) == project.map(|p| p.id));
        let branch = session
            .map(|s| (s.id, s.branch.clone()))
            .filter(|(_, b)| !b.is_empty());
        let crumb = session
            .filter(|_| self.view == View::Session)
            .map(|s| s.title.clone());
        let menu_open = self.menu.as_deref() == Some("project-switch");
        let agents_open = self.menu.as_deref() == Some("agents");
        let (where_name, where_color, where_tip) = self.runs_on();
        let where_session = self.current().map(|s| s.id);
        div()
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(lead))
            .pr_2()
            // The frame's grey, the same as the rail and the gutters between islands.
            .bg(settings::frame())
            .window_control_area(WindowControlArea::Drag)
            // Three zones: the two sides share what the search box leaves, so the box
            // sits in the middle of the window on every page instead of moving with the
            // breadcrumb's length.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .overflow_hidden()
                    .when(mode == Mode::Sessions, |el| {
                        el.child(
                            self.ghost_button("toggle-sidebar", IconName::PanelLeft, "Toggle sidebar")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_sidebar = !this.show_sidebar;
                                    this.save();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .child(
                                div()
                                    .id("project-switch")
                                    .h(px(30.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .when(menu_open, |el| el.bg(tint(c_active())))
                                    .hover(|el| el.bg(tint(c_subtle())))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.menu = if this.menu.as_deref() == Some("project-switch") {
                                            None
                                        } else {
                                            Some("project-switch".into())
                                        };
                                        cx.notify();
                                    }))
                                    .when_some(project, |el, p| el.child(project_mark(p, 20.)))
                                    .when(self.all_scope(), |el| {
                                        el.child(icon(IconName::LayoutGrid, 15., c_secondary()))
                                    })
                                    .child(div().max_w(px(200.)).truncate().child(match project {
                                        Some(p) => p.name.clone(),
                                        None if self.all_scope() => "All projects".to_string(),
                                        None => "No project".to_string(),
                                    }))
                                    .child(icon(IconName::ChevronDown, 12., c_muted())),
                            )
                            .when(menu_open, |el| el.child(self.project_menu(cx))),
                    )
                    .when_some(branch, |el, (sid, branch)| {
                        el.child(
                            div()
                                .id("top-branch")
                                .h(px(24.))
                                .px(px(7.))
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(5.))
                                .rounded_md()
                                .border_1()
                                .border_color(divider())
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .cursor_pointer()
                                .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                                .tooltip(|window, cx| {
                                    Tooltip::new("Branch: changes, terminal, worktrees").build(window, cx)
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    self.on_context(ctxmenu::CtxTarget::Branch(sid), cx),
                                )
                                .child(icon(IconName::GitBranch, 12., c_muted()))
                                .child(div().max_w(px(180.)).truncate().child(branch)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_shrink(1.)
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .child(icon(IconName::ChevronRight, 12., c_muted()))
                            .child(
                                div()
                                    .id("crumb-mode")
                                    .flex_none()
                                    .px_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        // The mode's own top page: Sessions lists every session.
                                        if mode == Mode::Sessions {
                                            this.view = View::AllSessions;
                                            this.save();
                                        } else {
                                            this.go(mode, window, cx);
                                        }
                                        cx.notify();
                                    }))
                                    .child(mode.label()),
                            )
                            .when_some(crumb, |el, title| {
                                el.child(icon(IconName::ChevronRight, 12., c_muted())).child(
                                    div()
                                        .id("crumb-session")
                                        .min_w_0()
                                        .max_w(px(220.))
                                        .truncate()
                                        .px_1()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .text_color(rgb(c_text()))
                                        .hover(|el| el.bg(tint(c_subtle())))
                                        .tooltip(|window, cx| {
                                            Tooltip::new("Rename this session").build(window, cx)
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.start_rename(window, cx);
                                            cx.notify();
                                        }))
                                        .child(title),
                                )
                            }),
                    )
            )
            .child(
                div().flex_shrink(1.).min_w(px(160.)).flex().justify_center().child(
                    div()
                        .id("top-search")
                        .relative()
                        .child(tour::anchor(&self.anchors, "palette"))
                        .w(px(420.))
                        .min_w(px(160.))
                        .flex_shrink(1.)
                        .h(px(28.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .border_1()
                        .border_color(divider())
                        .bg(tint(c_subtle()))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .hover(|el| el.border_color(rgb(c_secondary())))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_palette(window, cx);
                            cx.notify();
                        }))
                        .child(icon(IconName::Search, 13., c_muted()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child("Search sessions, projects, files, commands"),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px(px(5.))
                                .rounded_sm()
                                .border_1()
                                .border_color(divider())
                                .text_size(px(11.))
                                .child(keymap::label(cx, "quick-open")),
                        ),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .relative()
                            .child(
                                self.bar_pill("top-running")
                                    .when(agents_open, |el| el.bg(tint(c_active())))
                                    .tooltip(|window, cx| {
                                        Tooltip::new("Agents working now: open or stop them")
                                            .build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.menu = if this.menu.as_deref() == Some("agents") {
                                            None
                                        } else {
                                            Some("agents".into())
                                        };
                                        cx.notify();
                                    }))
                                    .child(if counts.running > 0 {
                                        self.status_dot(Status::Working, "top-run-dot", 7.)
                                    } else {
                                        div()
                                            .size(px(7.))
                                            .rounded_full()
                                            .bg(rgb(c_muted()))
                                            .into_any_element()
                                    })
                                    .child(match counts.running {
                                        0 => "No agents running".to_string(),
                                        1 => "1 agent running".to_string(),
                                        n => format!("{n} agents running"),
                                    }),
                            )
                            .when(agents_open, |el| el.child(self.agents_menu(cx))),
                    )
                    .when(counts.spend_cents > 0.0, |el| {
                        let total = counts.spend_cents;
                        el.child(
                            self.bar_pill("top-spend")
                                .cursor_default()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(format!(
                                        "Spend reported by the agents today. ${:.2} across every session. {}",
                                        total / 100.0,
                                        agent::cost_note()
                                    ))
                                    .build(window, cx)
                                })
                                .child(format!("${:.2} today", counts.spend_today / 100.0)),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .child(
                                self.ghost_button("top-inbox", IconName::Bell, "Inbox")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.go(Mode::Inbox, window, cx);
                                        cx.notify();
                                    })),
                            )
                            .when(inbox > 0, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .top(px(1.))
                                        .right(px(0.))
                                        .min_w(px(14.))
                                        .h(px(14.))
                                        .px(px(3.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(rgb(c_amber()))
                                        .text_size(px(9.5))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0xffffff))
                                        .child(pop(inbox.to_string(), format!("top-inbox-{inbox}"))),
                                )
                            }),
                    )
                    .when(
                        self.view == View::Session && self.current().is_some(),
                        |el| {
                            el.child(
                                self.ghost_button(
                                    "toggle-panel",
                                    IconName::PanelRight,
                                    "Toggle the Changes panel",
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.toggle_panel();
                                        cx.notify();
                                    },
                                )),
                            )
                        },
                    )
                    .child(
                        self.bar_pill("top-where")
                            .text_color(rgb(c_text()))
                            .tooltip(move |window, cx| {
                                Tooltip::new(where_tip.clone()).build(window, cx)
                            })
                            .map(|el| match where_session {
                                Some(sid) => el.on_mouse_down(
                                    MouseButton::Left,
                                    self.on_context(ctxmenu::CtxTarget::RunsOn(sid), cx),
                                ),
                                None => el.on_click(cx.listener(|this, _, _, cx| {
                                    this.open_settings();
                                    this.settings_page = SettingsPage::Connections;
                                    cx.notify();
                                })),
                            })
                            .child(div().size(px(7.)).rounded_full().bg(rgb(where_color)))
                            .child(where_name),
                    ),
            )
    }

    /// The list of every open tab, from the strip's overflow button.
    fn tabs_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().v_flex().p_1();
        for &id in &self.tabs {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let active = self.selected == Some(id) && self.view == View::Session;
            list = list.child(
                div()
                    .id(("tabs-row", id as usize))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(active, |el| el.bg(tint(c_active())))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.select(id);
                        cx.notify();
                    }))
                    .child(self.provider_mark(provider, 12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(session.title.clone()),
                    )
                    .child(
                        self.ghost_button_sized(
                            ("tabs-x", id as usize),
                            IconName::X,
                            "Close tab",
                            20.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_tab(id);
                            cx.notify();
                        })),
                    ),
            );
        }
        div()
            .absolute()
            .top(px(30.))
            .right_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id("tabs-menu")
                                .w(px(300.))
                                .max_h(px(400.))
                                .overflow_y_scroll()
                                .rounded_lg()
                                .border_1()
                                .border_color(float_edge())
                                .bg(float(c_raised()))
                                .shadow_lg()
                                .occlude()
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.menu = None;
                                    cx.notify();
                                }))
                                .child(list),
                        ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    /// Open sessions as tabs, over the session they belong to. The strip scrolls when the
    /// tabs don't fit, the button at its end lists them all, and tabs can be dragged into
    /// a new order.
    pub(crate) fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs = div()
            .id("tab-row")
            .flex()
            .items_end()
            .h_full()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll);
        for &id in &self.tabs {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let active =
                self.selected == Some(id) && self.view == View::Session && !self.browser_main;
            let working = self.handles.contains_key(&Target::Session(id));
            let dot = match self.need(session) {
                _ if working => Some(c_blue()),
                Some(Need::Blocked) => Some(c_amber()),
                Some(Need::Failed) => Some(c_red()),
                Some(Need::Finished) => Some(c_blue()),
                None => None,
            };
            let dragged = DraggedTab {
                id,
                title: session.title.clone().into(),
            };
            tabs = tabs.child(tab_slot(
                active,
                div()
                    .id(("tab", id as usize))
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Tab(id), cx),
                    )
                    .on_drag(dragged, |tab, _, _, cx| cx.new(|_| tab.clone()))
                    .drag_over::<DraggedTab>(|el, _, _, _| el.border_color(rgb(c_blue())))
                    .on_drop(cx.listener(move |this, tab: &DraggedTab, _, cx| {
                        move_tab(&mut this.tabs, tab.id, id);
                        this.save();
                        cx.notify();
                    }))
                    .group("tab")
                    .flex_none()
                    .w(px(180.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .map(|el| tab_shape(el, active))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(id);
                        cx.notify();
                    }))
                    .child(self.provider_mark(provider, 12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .id(("tab-x", id as usize))
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .hover(|el| el.bg(divider()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(id);
                                cx.notify();
                            }))
                            .child(match dot {
                                Some(color) => div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(rgb(color))
                                    .group_hover("tab", |el| el.opacity(0.))
                                    .into_any_element(),
                                None => icon(IconName::X, 11., c_muted()).into_any_element(),
                            }),
                    ),
            ));
        }
        if self.browser_open {
            let active = self.browser_main && self.view == View::Session;
            tabs = tabs.child(tab_slot(
                active,
                div()
                    .id("tab-browser")
                    .group("tab")
                    .flex_none()
                    .w(px(140.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .map(|el| tab_shape(el, active))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_browser();
                        cx.notify();
                    }))
                    .child(icon(IconName::Globe, 12., c_secondary()))
                    .child(div().flex_1().min_w_0().truncate().child("Browser"))
                    .child(
                        div()
                            .id("tab-browser-x")
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .hover(|el| el.bg(divider()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.browser_open = false;
                                this.browser_main = false;
                                cx.notify();
                            }))
                            .child(icon(IconName::X, 11., c_muted())),
                    ),
            ));
        }
        let (added, removed) = self
            .changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        let show_stats = self.view == View::Session && !self.changes.is_empty();
        let menu_open = self.menu.as_deref() == Some("tabs");
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_end()
            .relative()
            // The frame's grey, with the open tab cut out of it and joined to the page.
            .child(strip_fill("tabs", glass_fill(c_canvas())))
            .child(strip_edge(div().w(px(10.))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_end()
                    .child(tabs.relative().child(tour::anchor(&self.anchors, "tabs")))
                    .child(strip_edge(div().flex_1())),
            )
            .when(show_stats, |el| {
                el.child(
                    strip_edge(div())
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .text_size(px(12.))
                        .font_family(self.mono_font.clone())
                        .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                        .child(div().text_color(rgb(c_del())).child(format!("−{removed}"))),
                )
            })
            .when(self.tabs.len() > 1, |el| {
                el.child(
                    strip_edge(div())
                        .flex()
                        .items_center()
                        .relative()
                        .child(
                            self.ghost_button_sized(
                                "tabs-all",
                                IconName::ChevronDown,
                                "All open tabs",
                                26.,
                            )
                            .when(menu_open, |el| el.bg(tint(c_active())))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.menu = if this.menu.as_deref() == Some("tabs") {
                                    None
                                } else {
                                    Some("tabs".into())
                                };
                                cx.notify();
                            })),
                        )
                        .when(menu_open, |el| el.child(self.tabs_menu(cx))),
                )
            })
            .child(strip_edge(div().w(px(8.))))
    }

    // ---------- Home ----------

    /// A small bordered button with an icon, for Home's actions.
    pub(crate) fn home_button(
        &self,
        id: &'static str,
        glyph: IconName,
        label: &'static str,
        hint: String,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(32.))
            .px_3()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(divider())
            .bg(rgb(c_raised()))
            .cursor_pointer()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .hover(|el| el.bg(tint(c_subtle())))
            .child(icon(glyph, 14., c_text()))
            .child(label)
            .when(!hint.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(hint),
                )
            })
    }

    // ---------- Design and Architecture ----------

    /// Open the New session dialog for project `pid` with `task` filled in and `context`
    /// to go with it.
    pub(crate) fn start_with_context(
        &mut self,
        pid: u64,
        task: &str,
        context: StartContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_project = Some(pid);
        self.new_task
            .update(cx, |t, cx| t.set_value(task, window, cx));
        if context.in_place {
            self.new_isolated = false;
        }
        self.new_context = Some(context);
        self.open_new_session();
        cx.notify();
    }

    /// Give a just-created session the context it was started with: sent ahead of its
    /// first prompt, with the images attached when its agent reads them here.
    pub(crate) fn apply_context(&mut self, id: u64, context: &StartContext) {
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        session.handoff = if context.text.is_empty() {
            std::mem::take(&mut session.handoff)
        } else if session.handoff.is_empty() {
            context.text.clone()
        } else {
            format!("{}\n\n{}", session.handoff, context.text)
        };
        let images =
            session.connection == connections::LOCAL && agent::takes_images(&session.provider);
        if images && !context.images.is_empty() {
            self.launch_images.insert(id, context.images.clone());
        }
    }

    /// The context waiting on the New session dialog, with a way to drop it.
    pub(crate) fn context_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let context = self.new_context.as_ref()?;
        let images = context.images.len();
        Some(
            div()
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(divider())
                .bg(tint(c_subtle()))
                .text_size(px(12.5))
                .child(icon(IconName::Paperclip, 13., c_secondary()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(format!("{} goes with this task", context.label)),
                )
                .when(images > 0, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(rgb(c_muted()))
                            .child(icon(IconName::Image, 12., c_muted()))
                            .child(images.to_string()),
                    )
                })
                .child(
                    self.ghost_button_sized("context-drop", IconName::X, "Start without it", 22.)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.new_context = None;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}

impl RunoApp {
    /// The project a Design or Architecture page shows, beside its title: a switcher with
    /// the same list as the top bar's.
    pub(crate) fn mode_project_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        let project = self.scoped_project();
        let open = self.menu.as_deref() == Some("mode-project");
        div()
            .relative()
            .min_w_0()
            .child(
                div()
                    .id("mode-project")
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .cursor_pointer()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .when(open, |el| el.bg(tint(c_active())))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = if this.menu.as_deref() == Some("mode-project") {
                            None
                        } else {
                            Some("mode-project".into())
                        };
                        cx.notify();
                    }))
                    .child(match project {
                        Some(p) => project_mark(p, 18.).into_any_element(),
                        None => icon(IconName::LayoutGrid, 14., c_secondary()).into_any_element(),
                    })
                    .child(div().max_w(px(260.)).truncate().child(match project {
                        Some(p) => p.name.clone(),
                        None => "All projects".to_string(),
                    }))
                    .child(icon(IconName::ChevronDown, 12., c_muted())),
            )
            .when(open, |el| el.child(self.project_menu(cx)))
            .into_any_element()
    }
}

impl RunoApp {
    /// Design and Architecture under All projects: every project with what it has, to
    /// pick one, instead of guessing which project is meant.
    pub(crate) fn project_chooser(&self, mode: Mode, cx: &mut Context<Self>) -> AnyElement {
        let mut grid = div().flex().flex_wrap().gap_3();
        for project in &self.state.projects {
            let pid = project.id;
            let root = std::path::Path::new(&project.path);
            let missing = !self.demo && !root.is_dir();
            let summary = if missing {
                "Folder missing".to_string()
            } else {
                chooser_summary(mode, root, self.arch.maps.get(&pid), self.demo)
            };
            let sessions = self
                .state
                .sessions
                .iter()
                .filter(|s| s.project_id == pid && !s.archived && s.canvas.is_empty())
                .count();
            grid = grid.child(
                div()
                    .id(("chooser-project", pid as usize))
                    .w(px(280.))
                    .p_4()
                    .v_flex()
                    .gap_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(divider())
                    .bg(pane(c_canvas()))
                    .cursor_pointer()
                    .hover(|el| el.border_color(rgb(c_muted())).bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_project(pid, window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(project_mark(project, 22.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(project.name.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(if missing { c_amber() } else { c_secondary() }))
                            .child(summary),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .truncate()
                            .child(match sessions {
                                0 => project.path.clone(),
                                1 => format!("1 session · {}", project.path),
                                n => format!("{n} sessions · {}", project.path),
                            }),
                    ),
            );
        }
        let (start, line) = if mode == Mode::Design {
            (
                "New design project",
                "Start from a brief, sketches or screenshots, with no code yet.",
            )
        } else {
            (
                "New architecture project",
                "Sketch the system before writing it.",
            )
        };
        grid = grid.child(
            div()
                .id("chooser-new")
                .w(px(280.))
                .p_4()
                .v_flex()
                .gap_2()
                .rounded_xl()
                .border_1()
                .border_dashed()
                .border_color(divider())
                .cursor_pointer()
                .text_color(rgb(c_secondary()))
                .hover(|el| el.border_color(rgb(c_muted())).text_color(rgb(c_text())))
                .on_click(cx.listener(move |this, _, _, cx| this.start_project(mode, cx)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(icon(IconName::Plus, 15., c_secondary()))
                        .child(start),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child(line),
                ),
        );
        grid.into_any_element()
    }
}

/// One line on a chooser card: what the project has for this mode.
fn chooser_summary(
    mode: Mode,
    root: &std::path::Path,
    map: Option<&architecture::ProjectMap>,
    demo: bool,
) -> String {
    if demo {
        return "Sample project".into();
    }
    if mode == Mode::Design {
        let refs = std::fs::read_dir(root.join(design::FOLDER))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| turns::is_image(&e.path()))
            .count();
        let notes = design::notes_path(root).is_file();
        match (refs, notes) {
            (0, false) => "No references or notes yet".into(),
            (0, true) => "Design notes written".into(),
            (1, n) => format!("1 reference{}", if n { " · notes written" } else { "" }),
            (r, n) => format!("{r} references{}", if n { " · notes written" } else { "" }),
        }
    } else {
        let notes = architecture::notes_path(root).is_file();
        let shape = map.map(|m| match m.packages.len() {
            0 => format!("{} files", m.files),
            1 => format!("1 package · {} files", m.files),
            n => format!("{n} packages · {} files", m.files),
        });
        match (shape, notes) {
            (Some(shape), true) => format!("{shape} · notes written"),
            (Some(shape), false) => shape,
            (None, true) => "Notes written".into(),
            (None, false) => "No notes yet".into(),
        }
    }
}

/// What a session started from Design or Architecture brings along: named on the New
/// session dialog, sent ahead of the task, with its images attached.
#[derive(Clone, Debug, Default)]
pub(crate) struct StartContext {
    pub label: String,
    pub text: String,
    pub images: Vec<String>,
    /// Run in the project folder rather than a worktree, so the edits show where they
    /// were asked for (the architecture canvas previews them).
    pub in_place: bool,
}

/// The Design and Architecture pages' frame: the mode's mark and title over the body.
pub(crate) fn mode_frame(
    mode: Mode,
    title: &str,
    project: Option<AnyElement>,
    subtitle: &str,
    body: impl IntoElement,
    action: Option<AnyElement>,
) -> AnyElement {
    let color = if mode == Mode::Design {
        c_green()
    } else {
        c_blue()
    };
    div()
        .flex_1()
        .min_w_0()
        .overflow_y_scrollbar()
        .child(
            div()
                .max_w(px(1480.))
                .mx_auto()
                .p_8()
                .v_flex()
                .gap_6()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .size(px(44.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(rgb(settings::mix(color, c_canvas(), 0.3)))
                                .bg(rgb(settings::mix(
                                    color,
                                    c_canvas(),
                                    if is_dark() { 0.2 } else { 0.1 },
                                )))
                                .child(icon(mode.glyph(), 21., color)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .v_flex()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_size(px(22.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(title.to_string()),
                                        )
                                        .children(project),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(rgb(c_secondary()))
                                        .child(subtitle.to_string()),
                                ),
                        )
                        .children(action),
                )
                .child(body),
        )
        .into_any_element()
}

/// A project's square badge: its icon on its colour, or its initial.
/// A piece of the tab strip that carries the strip's bottom edge.
fn strip_edge(el: Div) -> Div {
    el.h_full()
}

/// Where a tab sits in the strip, with the gap after it.
fn tab_slot(_active: bool, tab: Stateful<Div>) -> Div {
    div()
        .h_full()
        .flex_none()
        .flex()
        .items_end()
        .pr(px(4.))
        .child(tab)
}

/// A tab's look: the open tab is cut from the page below the strip, which the strip's
/// fill paints (see `strip_fill`); the others sit on the frame as soft raised chips, so
/// they read as tabs, and lift a step more under the pointer.
fn tab_shape(el: Stateful<Div>, active: bool) -> Stateful<Div> {
    if active {
        el.relative()
            .h(px(31.))
            .child(strip_cut("tabs"))
            .text_color(rgb(c_text()))
    } else {
        let (rest, hover) = tab_chip();
        el.h(px(28.))
            .mb(px(3.))
            .rounded(px(8.))
            .bg(rest)
            .text_color(rgb(c_secondary()))
            .hover(move |el| el.bg(hover).text_color(rgb(c_text())))
    }
}

/// Fill of a tab that is not open, at rest and under the pointer: a wash of white over
/// the frame, stronger on the light theme's grey frame than on the dark one. In glass
/// mode the open tab is as see-through as its island, so a resting wash would read as the
/// open one: there the rest sit clear on the frame and only the pointer lifts them.
pub(crate) fn tab_chip() -> (Rgba, Rgba) {
    if settings::glass_on() {
        if is_dark() {
            (rgba(0), rgba(0xffffff1a))
        } else {
            (rgba(0), rgba(0xffffff59))
        }
    } else if is_dark() {
        (rgba(0xffffff14), rgba(0xffffff26))
    } else {
        (rgba(0xffffff80), rgba(0xffffffd0))
    }
}

pub(crate) fn project_mark(project: &Project, size: f32) -> Div {
    let color = projects::color_of(project).unwrap_or(c_secondary());
    let initial = project
        .name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.26))
        .bg(rgb(settings::mix(color, c_canvas(), 0.78)))
        .text_size(px(size * 0.55))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(color))
        .child(initial)
}

/// The canvas context for an ask in a conversation that already has `prev`: the parts
/// it hasn't seen, such as a new selection or changed screens, without repeating the
/// rules and anything else unchanged. Parts are the paragraphs between blank lines.
pub(crate) fn lean_context(prev: &str, now: &str) -> String {
    let parts = |text: &str| -> Vec<String> {
        text.split("\n\n")
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect()
    };
    let seen = parts(prev);
    let all = parts(now);
    let fresh: Vec<&String> = all.iter().filter(|p| !seen.contains(p)).collect();
    if prev.is_empty() || fresh.len() == all.len() {
        return now.to_string();
    }
    // Nothing changed: the ask goes on its own.
    if fresh.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "## The canvas\nThe rules for this canvas and anything not repeated here are as in your \
         earlier requests in this conversation.",
    );
    for p in fresh {
        out.push_str("\n\n");
        out.push_str(p);
    }
    out.push('\n');
    out
}

/// What a session asked from a canvas stores to say which: see `Session::canvas`.
pub(crate) fn canvas_name(view: View) -> &'static str {
    match view {
        View::Design => "design",
        _ => "architecture",
    }
}

/// The canvas a session was asked from, if it was.
pub(crate) fn canvas_of(s: &Session) -> Option<View> {
    match s.canvas.as_str() {
        "design" => Some(View::Design),
        "architecture" => Some(View::Architecture),
        _ => None,
    }
}

pub(crate) fn canvas_title(view: View) -> &'static str {
    match view {
        View::Design => "Design",
        _ => "Architecture",
    }
}

impl RunoApp {
    /// An ask from a canvas's AI bar. The canvas keeps one chat: a later ask continues it,
    /// with its history and the agent's own conversation, handing it to another agent
    /// with a brief if one was picked. The first ask starts it in the project folder with
    /// the picked agent. Neither asks anything more. Asks that don't edit in place still
    /// open the New session dialog.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ask_from_canvas(
        &mut self,
        pid: u64,
        view: View,
        task: &str,
        context: StartContext,
        agent: usize,
        model: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !context.in_place {
            self.new_provider = agent;
            self.start_with_context(pid, task, context, window, cx);
            return;
        }
        let same = agent::PROVIDERS[agent].id;
        let chat = self.canvas_sessions(pid, view).first().map(|s| {
            let busy = self.is_running(s) && s.provider != same;
            (s.id, busy)
        });
        let full = context.text.clone();
        if let Some((id, false)) = chat {
            // An agent resuming its own conversation already has the rules and the canvas
            // as they were; it gets only what changed. A new agent gets it all.
            let resumes = self
                .state
                .session(id)
                .is_some_and(|s| s.provider == same && !s.agent_session.is_empty());
            self.carry_to(id, agent);
            let mut context = context;
            if let Some(session) = self.state.session_mut(id) {
                if resumes {
                    context.text = lean_context(&session.canvas_context, &full);
                }
                session.canvas_context = full;
                session.unread = false;
                let current = session
                    .selections()
                    .get("model")
                    .cloned()
                    .unwrap_or_default();
                if current != model {
                    session.set_model(model);
                }
            }
            self.apply_context(id, &context);
            let provider = agent::PROVIDERS[agent].id;
            let opts = cx.global::<AppSettings>().agent(provider);
            self.start_agent(id, task.to_string(), &opts);
            self.save();
            return;
        }
        // The first ask, or another agent while this one still works: a new chat.
        self.new_project = Some(pid);
        self.new_provider = agent;
        self.new_isolated = false;
        self.new_context = Some(context);
        self.new_model = Some(model);
        self.new_back = view;
        self.new_task
            .update(cx, |t, cx| t.set_value(task, window, cx));
        self.branch.update(cx, |b, cx| b.set_value("", window, cx));
        self.new_connection = self.project_connection(pid);
        let race = std::mem::replace(&mut self.race_mode, false);
        let before = self.state.next_id;
        self.create_session(window, cx);
        self.new_model = None;
        self.race_mode = race;
        let name = canvas_name(view);
        for s in self.state.sessions.iter_mut() {
            if s.id > before && s.project_id == pid && s.canvas == name {
                s.canvas_context = full.clone();
            }
        }
        if matches!(self.view, View::NewSession) {
            self.view = view;
        }
    }

    /// The agent and model a canvas's last chat used, for its AI bar to start on.
    pub(crate) fn canvas_agent(&self, pid: u64, view: View) -> Option<(usize, String)> {
        let s = self.canvas_sessions(pid, view).into_iter().next()?;
        let agent = agent::PROVIDERS.iter().position(|p| p.id == s.provider)?;
        Some((
            agent,
            s.selections().get("model").cloned().unwrap_or_default(),
        ))
    }

    /// Open a session where it belongs: one asked from the Design or Architecture canvas
    /// opens that canvas, any other its chat.
    pub(crate) fn open_session(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pid, view)) = self
            .state
            .session(id)
            .and_then(|s| Some((s.project_id, canvas_of(s)?)))
        else {
            self.select(id);
            return;
        };
        self.menu = None;
        self.project = Some(pid);
        self.home_all = false;
        if let Some(session) = self.state.session_mut(id) {
            session.unread = false;
        }
        self.view = view;
        self.save();
        window.focus(&self.focus, cx);
    }

    /// The agent chats asked from a canvas of project `pid`, newest first.
    pub(crate) fn canvas_sessions(&self, pid: u64, view: View) -> Vec<&Session> {
        let name = canvas_name(view);
        let mut out: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid && s.canvas == name && !s.archived)
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.id));
        out
    }

    /// On a canvas's header: the chats of the agents asked from it, to read what they did
    /// and said, or carry on talking to one. Hidden until there is one.
    pub(crate) fn canvas_chats(
        &self,
        pid: u64,
        view: View,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let sessions = self.canvas_sessions(pid, view);
        if sessions.is_empty() {
            return None;
        }
        let open = self.menu.as_deref() == Some("canvas-chats");
        let busy = sessions.iter().any(|s| self.is_running(s));
        let mut list = div().v_flex().p_1().child(
            div()
                .px_2()
                .pt_1()
                .pb_2()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .whitespace_normal()
                .child(format!(
                    "Agents asked from the {} canvas. Open one to read its chat or ask it more.",
                    canvas_title(view)
                )),
        );
        for s in &sessions {
            let id = s.id;
            let p = agent::provider(&s.provider);
            let state = if self.is_running(s) {
                "Working".to_string()
            } else if self.need(s).is_some() {
                "Needs you".to_string()
            } else {
                let turns = s.messages.iter().filter(|m| m.role == Role::User).count();
                match turns {
                    1 => "1 ask".to_string(),
                    n => format!("{n} asks"),
                }
            };
            list = list.child(
                div()
                    .id(("canvas-chat", id as usize))
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.select(id);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_none()
                            .text_color(rgb(p.color))
                            .child(p.mark),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(div().truncate().text_size(px(13.)).child(s.title.clone()))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(format!("{} · {state}", p.name)),
                            ),
                    )
                    .when(s.unread, |el| {
                        el.child(div().size(px(7.)).rounded_full().bg(rgb(c_blue())))
                    }),
            );
        }
        Some(
            div()
                .relative()
                .flex_none()
                .child(
                    div()
                        .id("canvas-chats")
                        .h(px(30.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded_lg()
                        .border_1()
                        .border_color(divider())
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .when(open, |el| el.bg(tint(c_active())))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .tooltip(crate::archdraw::view::tip(
                            "The agents' chats behind this canvas",
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu = if this.menu.as_deref() == Some("canvas-chats") {
                                None
                            } else {
                                Some("canvas-chats".into())
                            };
                            cx.notify();
                        }))
                        .child(icon(IconName::MessagesSquare, 14., c_secondary()))
                        .child("Chats")
                        .child(
                            div()
                                .px(px(5.))
                                .rounded_full()
                                .bg(tint(c_active()))
                                .text_size(px(11.))
                                .text_color(rgb(if busy { c_blue() } else { c_muted() }))
                                .child(sessions.len().to_string()),
                        ),
                )
                .when(open, |el| {
                    el.child(
                        div().absolute().top(px(34.)).right_0().child(
                            deferred(
                                anchored().snap_to_window_with_margin(px(8.)).child(
                                    div()
                                        .id("canvas-chats-menu")
                                        .w(px(320.))
                                        .max_h(px(360.))
                                        .overflow_y_scroll()
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(float_edge())
                                        .bg(float(c_raised()))
                                        .shadow_lg()
                                        .occlude()
                                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                            this.menu = None;
                                            cx.notify();
                                        }))
                                        .child(list),
                                ),
                            )
                            .with_priority(2),
                        ),
                    )
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn canvas_context_sends_only_what_changed() {
        let first = "## Rules\nEdit the files.\n\nScreens now:\n- Home\n\nNothing is selected on the canvas.\n";
        let next = "## Rules\nEdit the files.\n\nScreens now:\n- Home\n\nSelected: the button.\n";
        let lean = lean_context(first, next);
        assert!(lean.contains("Selected: the button."));
        assert!(!lean.contains("Edit the files."));
        assert!(!lean.contains("- Home"));
        // Nothing sent before, or nothing in common: all of it.
        assert_eq!(lean_context("", next), next);
        assert_eq!(lean_context("Other.", next), next);
        assert_eq!(lean_context(next, next), "");
    }

    use super::*;
    use std::prelude::v1::test;
    use store::Message;

    #[test]
    fn only_design_and_architecture_show_one_project() {
        assert!(Mode::Design.per_project() && Mode::Architecture.per_project());
        assert!(Mode::MAIN.iter().all(|m| !m.per_project()));
    }

    #[test]
    fn every_rail_place_owns_its_views() {
        // Each place on the rail opens a view that lights that same place up again.
        for (view, mode) in [
            (View::Review, Mode::Git),
            (View::GitHub, Mode::Git),
            (View::Branches, Mode::Git),
            (View::GitHistory, Mode::Git),
            (View::History, Mode::History),
            (View::Notes, Mode::Notes),
            (View::Automations, Mode::Playbooks),
            (View::Session, Mode::Sessions),
        ] {
            assert_eq!(Mode::of(view), mode);
        }
        let rail: Vec<Mode> = Mode::MAIN
            .iter()
            .chain(Mode::PLAN.iter())
            .copied()
            .collect();
        for mode in &rail {
            assert_eq!(rail.iter().filter(|m| *m == mode).count(), 1);
            assert!(
                keymap::SHORTCUTS.iter().any(|s| s.id == mode.shortcut()),
                "{} has no shortcut",
                mode.label()
            );
        }
    }

    #[test]
    fn one_rule_for_needs_you() {
        let hidden = std::collections::HashSet::new();
        let mut s = Session {
            id: 7,
            messages: vec![Message::new(Role::Agent, "done")],
            ..Session::default()
        };
        assert_eq!(need_of(&s, false, &hidden), None);
        // Finished or failed unseen, until opened.
        s.unread = true;
        assert_eq!(need_of(&s, false, &hidden), Some(Need::Finished));
        s.status = Status::Failed;
        assert_eq!(need_of(&s, false, &hidden), Some(Need::Failed));
        s.unread = false;
        assert_eq!(need_of(&s, false, &hidden), None);
        // Blocked on a permission, even after it was opened, until dismissed.
        s.messages[0].denied = vec!["Bash".into()];
        assert_eq!(need_of(&s, false, &hidden), Some(Need::Blocked));
        assert_eq!(need_of(&s, true, &hidden), None);
        let dismissed: std::collections::HashSet<String> = ["s:7:0".to_string()].into();
        assert_eq!(need_of(&s, false, &dismissed), None);
        s.archived = true;
        assert_eq!(need_of(&s, false, &hidden), None);
    }

    #[test]
    fn arrows_step_through_lists() {
        let list = [4, 5, 6];
        assert_eq!(step_in(&list, None, 1, false), Some(4));
        assert_eq!(step_in(&list, None, -1, false), Some(6));
        assert_eq!(step_in(&list, Some(4), 1, false), Some(5));
        assert_eq!(step_in(&list, Some(6), 1, false), Some(6));
        assert_eq!(step_in(&list, Some(6), 1, true), Some(4));
        assert_eq!(step_in(&list, Some(4), -1, true), Some(6));
        assert_eq!(step_in(&list, Some(99), 1, true), Some(4));
        assert_eq!(step_in(&[], None, 1, true), None);
    }

    #[test]
    fn tabs_move_to_where_they_are_dropped() {
        let mut tabs = vec![1, 2, 3, 4];
        move_tab(&mut tabs, 4, 2);
        assert_eq!(tabs, vec![1, 4, 2, 3]);
        move_tab(&mut tabs, 1, 3);
        assert_eq!(tabs, vec![4, 2, 3, 1]);
        move_tab(&mut tabs, 9, 2);
        assert_eq!(tabs, vec![4, 2, 3, 1]);
    }
}
