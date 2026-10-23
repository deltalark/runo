//! Moving and resizing on the canvas as design tools do it: a handle on every side and
//! corner, Shift to keep the shape and Alt to resize about the middle, smart guides that
//! line a layer up with the ones beside it and show the gaps to them, and arrow keys.

use super::*;

/// The smallest a layer can be resized to, in CSS pixels.
const MIN_SIZE: f32 = 8.;
/// The colour design tools draw alignment and spacing marks in.
const RED: u32 = 0xf24e6b;

/// Which sides of the picked layer a resize drags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Edges {
    pub l: bool,
    pub r: bool,
    pub t: bool,
    pub b: bool,
}

impl Edges {
    fn corner(self) -> bool {
        (self.l || self.r) && (self.t || self.b)
    }
}

/// A line a layer can line up with, in page units: where it lies, and how far it runs the
/// other way, so the mark drawn for it reaches what it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Line {
    pub at: f32,
    pub lo: f32,
    pub hi: f32,
}

/// The box `r` resized by dragging `edges` by `d`. With `ratio` it keeps its proportions,
/// and with `centre` it grows on both sides at once.
pub(super) fn resized(r: &dom::Box, edges: Edges, d: Pt, ratio: bool, centre: bool) -> dom::Box {
    let mut gw = if edges.r {
        d.0
    } else if edges.l {
        -d.0
    } else {
        0.
    };
    let mut gh = if edges.b {
        d.1
    } else if edges.t {
        -d.1
    } else {
        0.
    };
    if centre {
        gw *= 2.;
        gh *= 2.;
    }
    let mut w = (r.w + gw).max(MIN_SIZE);
    let mut h = (r.h + gh).max(MIN_SIZE);
    if ratio && r.w > 0. && r.h > 0. {
        let k = r.w / r.h;
        let wide = edges.l || edges.r;
        let tall = edges.t || edges.b;
        // A corner follows whichever way it was pulled further.
        if wide && (!tall || (w / r.w - 1.).abs() >= (h / r.h - 1.).abs()) {
            h = w / k;
        } else {
            w = h * k;
        }
        (w, h) = (w.max(MIN_SIZE), h.max(MIN_SIZE));
    }
    let (w, h) = (w.round(), h.round());
    // The sides not dragged stay put; with `centre`, or a side grown to keep the ratio, it
    // grows about the middle.
    let x = if centre || (ratio && !edges.l && !edges.r) {
        r.x + (r.w - w) / 2.
    } else if edges.l {
        r.x + r.w - w
    } else {
        r.x
    };
    let y = if centre || (ratio && !edges.t && !edges.b) {
        r.y + (r.h - h) / 2.
    } else if edges.t {
        r.y + r.h - h
    } else {
        r.y
    };
    dom::Box { x, y, w, h }
}

/// The styles that put a layer at `new` from `old`: its size when that changed, and, when
/// it floats, its position, keeping the sides it's anchored to.
pub(super) fn placed(
    inline: &HashMap<String, String>,
    old: &dom::Box,
    new: &dom::Box,
    floating: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let px_of = |k: &str| {
        inline
            .get(k)
            .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
    };
    let mut map = serde_json::Map::new();
    let mut put = |k: &str, v: f32| {
        map.insert(k.into(), json!(format!("{}px", v.round())));
    };
    if (new.w - old.w).abs() > 0.01 {
        put("width", new.w);
    }
    if (new.h - old.h).abs() > 0.01 {
        put("height", new.h);
    }
    if floating {
        let (dl, dr) = (new.x - old.x, new.x + new.w - old.x - old.w);
        if dl.abs() + dr.abs() > 0.01 {
            match (px_of("left"), px_of("right")) {
                (None, Some(r)) => put("right", r - dr),
                (l, _) => put("left", l.unwrap_or(0.) + dl),
            }
        }
        let (dt, db) = (new.y - old.y, new.y + new.h - old.y - old.h);
        if dt.abs() + db.abs() > 0.01 {
            match (px_of("top"), px_of("bottom")) {
                (None, Some(b)) => put("bottom", b - db),
                (t, _) => put("top", t.unwrap_or(0.) + dt),
            }
        }
    }
    map
}

/// Which sides of the box from `(x0, y0)` to `(x1, y1)` on screen the pointer at `p` is on,
/// for resizing. The grip reaches a little outside each side but only a sliver inside it,
/// so a small layer can still be grabbed and moved; on a tiny one the right and bottom win.
pub(super) fn edges_at(p: Pt, (x0, y0): Pt, (x1, y1): Pt) -> Option<Edges> {
    let (w, h) = (x1 - x0, y1 - y0);
    let inset = |size: f32| (size / 4.).min(4.);
    let within = |v: f32, lo: f32, hi: f32| v >= lo - 6. && v <= hi + 6.;
    let across = within(p.1, y0, y1);
    let down = within(p.0, x0, x1);
    let r = across && p.0 <= x1 + 6. && p.0 >= x1 - inset(w);
    let l = across && !r && p.0 >= x0 - 6. && p.0 <= x0 + inset(w);
    let b = down && p.1 <= y1 + 6. && p.1 >= y1 - inset(h);
    let t = down && !b && p.1 >= y0 - 6. && p.1 <= y0 + inset(h);
    let e = Edges { l, r, t, b };
    (l || r || t || b).then_some(e)
}

/// The gaps from `g` to the nearest of `others` on each side that it faces: lines to
/// draw and how long they are, as design tools show while moving a layer.
pub(super) fn neighbour_gaps(g: &dom::Box, others: &[dom::Box]) -> Vec<dom::Gap> {
    let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| {
        let (lo, hi) = (a0.max(b0), a1.min(b1));
        (hi > lo).then_some((lo + hi) / 2.)
    };
    let mut out = Vec::new();
    let nearest = |dist: &dyn Fn(&dom::Box) -> Option<(f32, dom::Gap)>| {
        others
            .iter()
            .filter_map(dist)
            .filter(|(d, _)| *d > -0.5)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            // Touching the nearest one: no gap to show, and nothing further is seen.
            .filter(|(d, _)| *d >= 0.5)
            .map(|(_, gap)| gap)
    };
    let right = nearest(&|b: &dom::Box| {
        let y = overlap(g.y, g.y + g.h, b.y, b.y + b.h)?;
        let d = b.x - (g.x + g.w);
        Some((d, ((g.x + g.w, y), (b.x, y), d)))
    });
    let left = nearest(&|b: &dom::Box| {
        let y = overlap(g.y, g.y + g.h, b.y, b.y + b.h)?;
        let d = g.x - (b.x + b.w);
        Some((d, ((b.x + b.w, y), (g.x, y), d)))
    });
    let below = nearest(&|b: &dom::Box| {
        let x = overlap(g.x, g.x + g.w, b.x, b.x + b.w)?;
        let d = b.y - (g.y + g.h);
        Some((d, ((x, g.y + g.h), (x, b.y), d)))
    });
    let above = nearest(&|b: &dom::Box| {
        let x = overlap(g.x, g.x + g.w, b.x, b.x + b.w)?;
        let d = g.y - (b.y + b.h);
        Some((d, ((x, b.y + b.h), (x, g.y), d)))
    });
    out.extend([left, right, above, below].into_iter().flatten());
    out
}

/// The lines among `lines` that `g`'s sides or middle lie on, down (`vertical`) or across,
/// each reaching from `g` to what it came from.
pub(super) fn touching(g: &dom::Box, lines: &[Line], vertical: bool) -> Vec<Line> {
    let (edges, lo, hi) = if vertical {
        ([g.x, g.x + g.w / 2., g.x + g.w], g.y, g.y + g.h)
    } else {
        ([g.y, g.y + g.h / 2., g.y + g.h], g.x, g.x + g.w)
    };
    let mut out: Vec<Line> = Vec::new();
    for e in edges {
        for l in lines.iter().filter(|l| (l.at - e).abs() < 0.5) {
            match out.iter_mut().find(|o| (o.at - e).abs() < 0.5) {
                Some(o) => (o.lo, o.hi) = (o.lo.min(l.lo), o.hi.max(l.hi)),
                None => out.push(Line {
                    at: e,
                    lo: lo.min(l.lo),
                    hi: hi.max(l.hi),
                }),
            }
        }
    }
    out
}

impl DesignCanvas {
    /// The layers the picked one lines up with on screen `id`: the others in the same box,
    /// that box, and the screen itself.
    fn neighbours(&self, id: &str) -> Vec<dom::Box> {
        let (Some(el), Some(shot)) = (&self.element, self.shots.get(id)) else {
            return vec![];
        };
        if self.screen.as_deref() != Some(id) {
            return vec![];
        }
        let Some(home) = dom::parent(&el.path) else {
            return vec![];
        };
        shot.boxes
            .iter()
            .filter(|b| {
                b.path != el.path
                    && (b.path == home || dom::parent(&b.path).as_ref() == Some(&home))
                    && !b.path.starts_with(&el.path)
            })
            .map(|b| b.rect.clone())
            .collect()
    }

    /// The lines on screen `id` the picked layer can line up with, down (`vertical`) or
    /// across: the sides and middles of its neighbours and of the screen.
    pub(super) fn smart_lines(&self, id: &str, vertical: bool) -> Vec<Line> {
        let mut out = Vec::new();
        if let Some(s) = self.canvas.screen(id) {
            let (w, h) = self.board_size(s);
            let (span, other) = if vertical { (w, h) } else { (h, w) };
            for at in [0., span / 2., span] {
                out.push(Line {
                    at,
                    lo: 0.,
                    hi: other,
                });
            }
        }
        for b in self.neighbours(id) {
            let (a, len, lo, hi) = if vertical {
                (b.x, b.w, b.y, b.y + b.h)
            } else {
                (b.y, b.h, b.x, b.x + b.w)
            };
            for at in [a, a + len / 2., a + len] {
                out.push(Line { at, lo, hi });
            }
        }
        out
    }

    /// Where the picked layer is drawn while it's dragged or resized, in page units.
    pub(super) fn ghost(&self) -> Option<dom::Box> {
        let r = &self.element.as_ref()?.rect;
        match &self.drag {
            Drag::Element {
                from,
                at,
                moved: true,
                ..
            } if self.floating() => Some(dom::Box {
                x: r.x + at.0 - from.0,
                y: r.y + at.1 - from.1,
                ..r.clone()
            }),
            Drag::Resize {
                edges,
                from,
                at,
                ratio,
                centre,
            } => Some(resized(
                r,
                *edges,
                (at.0 - from.0, at.1 - from.1),
                *ratio,
                *centre,
            )),
            _ => None,
        }
    }

    /// Red marks while a layer is moved or resized: the lines it has snapped to, and,
    /// while moving, the gaps to the layers beside it.
    pub(super) fn smart_guides(&self) -> Vec<AnyElement> {
        let (Some(g), Some(id)) = (self.ghost(), self.screen.clone()) else {
            return vec![];
        };
        let Some((ox, oy)) = self.canvas.screen(&id).map(|s| (s.x, s.y)) else {
            return vec![];
        };
        let mut out = Vec::new();
        for vertical in [true, false] {
            for l in touching(&g, &self.smart_lines(&id, vertical), vertical) {
                let (a, b) = if vertical {
                    ((l.at, l.lo), (l.at, l.hi))
                } else {
                    ((l.lo, l.at), (l.hi, l.at))
                };
                let (ax, ay) = self.screen_pt((ox + a.0, oy + a.1));
                let (bx, by) = self.screen_pt((ox + b.0, oy + b.1));
                out.push(
                    div()
                        .absolute()
                        .left(px(ax.min(bx)))
                        .top(px(ay.min(by)))
                        .w(px(if vertical { 1. } else { (bx - ax).abs() }))
                        .h(px(if vertical { (by - ay).abs() } else { 1. }))
                        .bg(rgb(RED))
                        .into_any_element(),
                );
            }
        }
        // The gaps show while moving; while resizing they'd sit on the size.
        let gaps = match self.drag {
            Drag::Element { .. } => neighbour_gaps(&g, &self.neighbours(&id)),
            _ => vec![],
        };
        for (i, (a, b, d)) in gaps.into_iter().enumerate() {
            let (ax, ay) = self.screen_pt((ox + a.0, oy + a.1));
            let (bx, by) = self.screen_pt((ox + b.0, oy + b.1));
            let across = (ay - by).abs() < 0.5;
            if (if across { bx - ax } else { by - ay }).abs() < 4. {
                continue;
            }
            out.push(
                div()
                    .absolute()
                    .left(px(ax.min(bx)))
                    .top(px(ay.min(by)))
                    .w(px(if across { (bx - ax).abs() } else { 1. }))
                    .h(px(if across { 1. } else { (by - ay).abs() }))
                    .bg(rgb(RED))
                    .into_any_element(),
            );
            out.push(
                div()
                    .id(("design-gap", i))
                    .absolute()
                    .left(px((ax + bx) / 2. + if across { -10. } else { 4. }))
                    .top(px((ay + by) / 2. + if across { 3. } else { -8. }))
                    .px_1()
                    .rounded_sm()
                    .bg(rgb(RED))
                    .text_size(px(10.5))
                    .text_color(rgb(0xffffff))
                    .child(format!("{}", d.round()))
                    .into_any_element(),
            );
        }
        out
    }

    /// Which sides of the picked layer are under the pointer, for resizing it.
    pub(super) fn resize_edges(&self, local: Pt) -> Option<Edges> {
        let (_, r) = self.picked_rect()?;
        let a = self.screen_pt((r.x, r.y));
        let b = self.screen_pt((r.x + r.w, r.y + r.h));
        let e = edges_at(local, a, b)?;
        // The link handle sits at the middle of the right edge.
        let mid = (a.1 + b.1) / 2.;
        if self.links_shown() && e.r && !e.corner() && (local.1 - mid).abs() < 10. {
            return None;
        }
        Some(e)
    }

    /// Where the pointer resizes the picked layer from, with the cursor each shows.
    pub(super) fn resize_grips(&self, (x, y): Pt, (w, h): Pt) -> Vec<AnyElement> {
        let grip = |id: &'static str| div().id(id).absolute();
        let side = 6.;
        vec![
            grip("design-resize-l")
                .left(px(x - side / 2.))
                .top(px(y + side))
                .w(px(side))
                .h(px((h - 2. * side).max(0.)))
                .cursor_ew_resize()
                .into_any_element(),
            grip("design-resize-r")
                .left(px(x + w - side / 2.))
                .top(px(y + side))
                .w(px(side))
                .h(px((h - 2. * side).max(0.)))
                .cursor_ew_resize()
                .into_any_element(),
            grip("design-resize-t")
                .left(px(x + side))
                .top(px(y - side / 2.))
                .w(px((w - 2. * side).max(0.)))
                .h(px(side))
                .cursor_ns_resize()
                .into_any_element(),
            grip("design-resize-b")
                .left(px(x + side))
                .top(px(y + h - side / 2.))
                .w(px((w - 2. * side).max(0.)))
                .h(px(side))
                .cursor_ns_resize()
                .into_any_element(),
            grip("design-resize-tl")
                .left(px(x - side))
                .top(px(y - side))
                .size(px(2. * side))
                .cursor(CursorStyle::ResizeUpLeftDownRight)
                .into_any_element(),
            grip("design-resize-br")
                .left(px(x + w - side))
                .top(px(y + h - side))
                .size(px(2. * side))
                .cursor(CursorStyle::ResizeUpLeftDownRight)
                .into_any_element(),
            grip("design-resize-tr")
                .left(px(x + w - side))
                .top(px(y - side))
                .size(px(2. * side))
                .cursor(CursorStyle::ResizeUpRightDownLeft)
                .into_any_element(),
            grip("design-resize-bl")
                .left(px(x - side))
                .top(px(y + h - side))
                .size(px(2. * side))
                .cursor(CursorStyle::ResizeUpRightDownLeft)
                .into_any_element(),
        ]
    }

    /// Put the picked layer where a resize left it.
    pub(super) fn finish_resize(&mut self, new: dom::Box, cx: &mut Context<Self>) {
        let Some(el) = &self.element else {
            return;
        };
        let map = placed(&el.inline, &el.rect, &new, self.floating());
        if !map.is_empty() {
            self.op("styles", &[serde_json::Value::Object(map)], After::Same, cx);
        }
    }

    /// The arrow keys: a floating layer moves a pixel (ten with Shift), one in a layout
    /// moves before or after its neighbour, and a screen with nothing in it picked moves
    /// on the canvas.
    pub(super) fn arrow(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) {
        let step = if shift { 10. } else { 1. };
        let (dx, dy) = match key {
            "left" => (-step, 0.),
            "right" => (step, 0.),
            "up" => (0., -step),
            _ => (0., step),
        };
        if self.element.is_some() {
            if !self.also.is_empty() {
                return;
            }
            if self.floating() {
                self.move_floating(dx, dy, cx);
            } else {
                self.nudge(if dx + dy < 0. { -1 } else { 1 }, cx);
            }
        } else if let Some(id) = self.screen.clone() {
            if let Some(s) = self.canvas.screen_mut(&id) {
                s.x += dx;
                s.y += dy;
                self.save_canvas();
            }
        }
    }

    /// Tab and Shift+Tab: the next or previous layer while the canvas has the keyboard,
    /// else the next or previous field as usual.
    pub(super) fn tab(&mut self, to: Walk, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.is_focused(window) {
            self.walk(to, cx);
        } else {
            cx.propagate();
        }
    }

    /// Walk the layer tree from the keyboard: into the first child, out to the parent, or
    /// along to the next or previous sibling, wrapping round.
    pub(super) fn walk(&mut self, to: Walk, cx: &mut Context<Self>) -> bool {
        let (Some(id), Some(el)) = (self.screen.clone(), &self.element) else {
            return false;
        };
        let Some(shot) = self.shots.get(&id) else {
            return false;
        };
        let here = el.path.clone();
        let target = match to {
            Walk::In => shot
                .boxes
                .iter()
                .filter(|b| dom::parent(&b.path).as_ref() == Some(&here))
                .map(|b| b.path.clone())
                .min(),
            Walk::Out => dom::parent(&here).filter(|p| !p.is_empty()),
            Walk::Next | Walk::Prev => {
                let home = dom::parent(&here);
                let mut sibs: Vec<dom::Path> = shot
                    .boxes
                    .iter()
                    .filter(|b| dom::parent(&b.path) == home)
                    .map(|b| b.path.clone())
                    .collect();
                sibs.sort();
                sibs.dedup();
                let i = sibs.iter().position(|p| *p == here);
                match (i, sibs.len()) {
                    (Some(i), n) if n > 1 => Some(if to == Walk::Next {
                        sibs[(i + 1) % n].clone()
                    } else {
                        sibs[(i + n - 1) % n].clone()
                    }),
                    _ => None,
                }
            }
        };
        match target {
            Some(p) => {
                self.also.clear();
                self.select_path(&id, p, cx);
                true
            }
            None => false,
        }
    }
}

/// A step through the layer tree from the keyboard.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Walk {
    In,
    Out,
    Next,
    Prev,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn bx(x: f32, y: f32, w: f32, h: f32) -> dom::Box {
        dom::Box { x, y, w, h }
    }

    const R: Edges = Edges {
        l: false,
        r: true,
        t: false,
        b: false,
    };
    const TL: Edges = Edges {
        l: true,
        r: false,
        t: true,
        b: false,
    };

    #[test]
    fn resizing_from_the_left_or_top_keeps_the_far_side_still() {
        let r = bx(100., 50., 80., 40.);
        let n = resized(&r, TL, (-20., 10.), false, false);
        assert_eq!(n, bx(80., 60., 100., 30.));
        // The right and bottom sides didn't move.
        assert_eq!(n.x + n.w, 180.);
        assert_eq!(n.y + n.h, 90.);
    }

    #[test]
    fn alt_resizes_about_the_middle_and_shift_keeps_the_shape() {
        let r = bx(0., 0., 100., 50.);
        assert_eq!(
            resized(&r, R, (10., 0.), false, true),
            bx(-10., 0., 120., 50.)
        );
        // Pulling the right side wider makes it taller too, about its middle.
        assert_eq!(
            resized(&r, R, (100., 0.), true, false),
            bx(0., -25., 200., 100.)
        );
        // A corner keeps the shape, following the way it was pulled further.
        let c = Edges {
            r: true,
            b: true,
            ..Edges::default()
        };
        assert_eq!(
            resized(&r, c, (50., 5.), true, false),
            bx(0., 0., 150., 75.)
        );
    }

    #[test]
    fn a_layer_cannot_be_resized_to_nothing() {
        let r = bx(0., 0., 30., 30.);
        let n = resized(&r, TL, (100., 100.), false, false);
        assert_eq!((n.w, n.h), (MIN_SIZE, MIN_SIZE));
        assert_eq!((n.x + n.w, n.y + n.h), (30., 30.));
    }

    #[test]
    fn a_resize_keeps_the_sides_a_floating_layer_is_anchored_to() {
        let old = bx(10., 10., 50., 50.);
        let new = bx(0., 10., 60., 50.);
        let mut inline = HashMap::new();
        inline.insert("left".to_string(), "10px".to_string());
        let m = placed(&inline, &old, &new, true);
        assert_eq!(m.get("left"), Some(&json!("0px")));
        assert_eq!(m.get("width"), Some(&json!("60px")));
        assert!(!m.contains_key("top"), "it didn't move down");
        // Anchored to the right: growing to the left leaves `right` alone.
        let mut inline = HashMap::new();
        inline.insert("right".to_string(), "20px".to_string());
        let m = placed(&inline, &old, &new, true);
        assert_eq!(m.get("right"), Some(&json!("20px")));
        // In a layout only the size changes.
        let m = placed(&HashMap::new(), &old, &new, false);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn every_side_and_corner_can_be_grabbed() {
        let (a, b) = ((100., 100.), (200., 160.));
        let at = |p: Pt| edges_at(p, a, b);
        assert_eq!(at((150., 130.)), None, "the middle moves it");
        assert_eq!(at((99., 130.)).map(|e| (e.l, e.r)), Some((true, false)));
        assert_eq!(at((150., 98.)).map(|e| (e.t, e.b)), Some((true, false)));
        let tl = at((98., 98.)).unwrap();
        assert!(tl.l && tl.t && tl.corner());
        let br = at((203., 163.)).unwrap();
        assert!(br.r && br.b);
        assert_eq!(at((300., 130.)), None, "too far away");
        // On a sliver of a layer the right side wins over the left.
        let thin = edges_at((102., 130.), (100., 100.), (102., 160.)).unwrap();
        assert!(thin.r && !thin.l);
    }

    #[test]
    fn gaps_go_to_the_nearest_layer_on_each_side() {
        let g = bx(100., 100., 50., 50.);
        let others = [
            bx(170., 110., 20., 20.),
            bx(300., 110., 20., 20.),
            bx(100., 10., 50., 60.),
            bx(0., 400., 10., 10.),
        ];
        let gaps = neighbour_gaps(&g, &others);
        let lens: Vec<f32> = gaps.iter().map(|g| g.2).collect();
        assert_eq!(lens, vec![20., 30.], "right to the near one, and up");
        assert_eq!(gaps[0].0, (150., 120.));
    }

    #[test]
    fn a_layer_touching_its_neighbour_shows_no_gap_past_it() {
        let g = bx(0., 0., 50., 50.);
        let others = [bx(0., 50., 50., 10.), bx(0., 200., 50., 10.)];
        assert!(neighbour_gaps(&g, &others).is_empty());
    }

    #[test]
    fn lines_touch_the_sides_and_middle_they_lie_on() {
        let g = bx(100., 100., 50., 20.);
        let lines = [
            Line {
                at: 125.,
                lo: 0.,
                hi: 40.,
            },
            Line {
                at: 101.,
                lo: 0.,
                hi: 40.,
            },
        ];
        let hit = touching(&g, &lines, true);
        assert_eq!(
            hit,
            vec![Line {
                at: 125.,
                lo: 0.,
                hi: 120.
            }]
        );
    }
}
