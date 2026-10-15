//! Geometry for the diagram editor: connector routes, hit testing, snapping guides,
//! align and distribute, and the layered auto-layout.

use super::model::{Rect, Route};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(crate) type Pt = (f32, f32);

/// The side of a box a connector leaves or enters by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub(crate) const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

    pub(crate) fn point(self, r: &Rect) -> Pt {
        let (cx, cy) = r.center();
        match self {
            Side::Top => (cx, r.y),
            Side::Right => (r.right(), cy),
            Side::Bottom => (cx, r.bottom()),
            Side::Left => (r.x, cy),
        }
    }

    fn normal(self) -> Pt {
        match self {
            Side::Top => (0., -1.),
            Side::Right => (1., 0.),
            Side::Bottom => (0., 1.),
            Side::Left => (-1., 0.),
        }
    }
}

/// The sides a connector between two boxes should use.
fn sides(a: &Rect, b: &Rect) -> (Side, Side) {
    let (ax, ay) = a.center();
    let (bx, by) = b.center();
    let dx = bx - ax;
    let dy = by - ay;
    // Compare the gaps between the boxes rather than the centres, so wide boxes stacked
    // above each other still connect top to bottom.
    let gap_x = (b.x - a.right()).max(a.x - b.right());
    let gap_y = (b.y - a.bottom()).max(a.y - b.bottom());
    if gap_x > gap_y {
        if dx >= 0. {
            (Side::Right, Side::Left)
        } else {
            (Side::Left, Side::Right)
        }
    } else if dy >= 0. {
        (Side::Bottom, Side::Top)
    } else {
        (Side::Top, Side::Bottom)
    }
}

/// Where the line from the centre of `r` towards `to` leaves `r`.
fn clip(r: &Rect, to: Pt) -> Pt {
    let (cx, cy) = r.center();
    let dx = to.0 - cx;
    let dy = to.1 - cy;
    if dx.abs() < f32::EPSILON && dy.abs() < f32::EPSILON {
        return (cx, cy);
    }
    let sx = if dx.abs() > f32::EPSILON {
        (r.w / 2.) / dx.abs()
    } else {
        f32::INFINITY
    };
    let sy = if dy.abs() > f32::EPSILON {
        (r.h / 2.) / dy.abs()
    } else {
        f32::INFINITY
    };
    let s = sx.min(sy).min(1.);
    (cx + dx * s, cy + dy * s)
}

/// A connector's shape: a polyline, or a cubic curve as [start, ctrl1, ctrl2, end].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Wire {
    pub points: Vec<Pt>,
    pub curved: bool,
}

impl Wire {
    /// Points along the connector, with curves flattened, for hit tests and labels.
    pub(crate) fn polyline(&self) -> Vec<Pt> {
        if !self.curved || self.points.len() != 4 {
            return self.points.clone();
        }
        let [p0, p1, p2, p3] = [
            self.points[0],
            self.points[1],
            self.points[2],
            self.points[3],
        ];
        (0..=24)
            .map(|i| {
                let t = i as f32 / 24.;
                let u = 1. - t;
                let a = u * u * u;
                let b = 3. * u * u * t;
                let c = 3. * u * t * t;
                let d = t * t * t;
                (
                    a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
                    a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
                )
            })
            .collect()
    }

    /// The point halfway along, where the label goes.
    pub(crate) fn middle(&self) -> Pt {
        let pts = self.polyline();
        let total: f32 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
        let mut left = total / 2.;
        for w in pts.windows(2) {
            let d = dist(w[0], w[1]);
            if d >= left && d > 0. {
                let t = left / d;
                return (
                    w[0].0 + (w[1].0 - w[0].0) * t,
                    w[0].1 + (w[1].1 - w[0].1) * t,
                );
            }
            left -= d;
        }
        pts.first().copied().unwrap_or_default()
    }

    /// The last two points that differ, for drawing the end arrowhead.
    pub(crate) fn end_dir(&self) -> Option<(Pt, Pt)> {
        let pts = &self.points;
        let end = *pts.last()?;
        pts.iter()
            .rev()
            .skip(1)
            .find(|p| dist(**p, end) > 0.5)
            .map(|p| (*p, end))
    }

    pub(crate) fn start_dir(&self) -> Option<(Pt, Pt)> {
        let pts = &self.points;
        let start = *pts.first()?;
        pts.iter()
            .skip(1)
            .find(|p| dist(**p, start) > 0.5)
            .map(|p| (*p, start))
    }
}

pub(crate) fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Distance from `p` to the segment `a`–`b`.
pub(crate) fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    if len < f32::EPSILON {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0., 1.);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// The route of a connector from box `a` to box `b`.
pub(crate) fn route(a: &Rect, b: &Rect, kind: Route) -> Wire {
    if a == b {
        // A box that talks to itself: a loop off its right side.
        let (x, y) = (a.right(), a.y + a.h * 0.3);
        let y2 = a.y + a.h * 0.7;
        return Wire {
            points: vec![(x, y), (x + 40., y), (x + 40., y2), (x, y2)],
            curved: false,
        };
    }
    match kind {
        Route::Straight => {
            let start = clip(a, b.center());
            let end = clip(b, a.center());
            Wire {
                points: vec![start, end],
                curved: false,
            }
        }
        Route::Elbow => {
            let (sa, sb) = sides(a, b);
            let start = sa.point(a);
            let end = sb.point(b);
            let points = match sa {
                Side::Left | Side::Right => {
                    let mx = (start.0 + end.0) / 2.;
                    vec![start, (mx, start.1), (mx, end.1), end]
                }
                Side::Top | Side::Bottom => {
                    let my = (start.1 + end.1) / 2.;
                    vec![start, (start.0, my), (end.0, my), end]
                }
            };
            let mut points: Vec<Pt> = points;
            points.dedup_by(|p, q| dist(*p, *q) < 0.5);
            // Drop bends that don't bend.
            let mut i = 1;
            while i + 1 < points.len() {
                let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
                let straight = ((a.0 - b.0).abs() < 0.5 && (b.0 - c.0).abs() < 0.5)
                    || ((a.1 - b.1).abs() < 0.5 && (b.1 - c.1).abs() < 0.5);
                if straight {
                    points.remove(i);
                } else {
                    i += 1;
                }
            }
            Wire {
                points,
                curved: false,
            }
        }
        Route::Curved => {
            let (sa, sb) = sides(a, b);
            let start = sa.point(a);
            let end = sb.point(b);
            let k = (dist(start, end) / 2.).clamp(30., 160.);
            let (na, nb) = (sa.normal(), sb.normal());
            Wire {
                points: vec![
                    start,
                    (start.0 + na.0 * k, start.1 + na.1 * k),
                    (end.0 + nb.0 * k, end.1 + nb.1 * k),
                    end,
                ],
                curved: true,
            }
        }
    }
}

/// How close `p` is to the connector.
pub(crate) fn wire_dist(w: &Wire, p: Pt) -> f32 {
    w.polyline()
        .windows(2)
        .map(|s| seg_dist(p, s[0], s[1]))
        .fold(f32::INFINITY, f32::min)
}

/// A resize handle on a selected box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handle {
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

impl Handle {
    pub(crate) const ALL: [Handle; 8] = [
        Handle::N,
        Handle::NE,
        Handle::E,
        Handle::SE,
        Handle::S,
        Handle::SW,
        Handle::W,
        Handle::NW,
    ];

    pub(crate) fn point(self, r: &Rect) -> Pt {
        let (cx, cy) = r.center();
        match self {
            Handle::N => (cx, r.y),
            Handle::NE => (r.right(), r.y),
            Handle::E => (r.right(), cy),
            Handle::SE => (r.right(), r.bottom()),
            Handle::S => (cx, r.bottom()),
            Handle::SW => (r.x, r.bottom()),
            Handle::W => (r.x, cy),
            Handle::NW => (r.x, r.y),
        }
    }

    /// The rectangle after dragging this handle by `d`, never smaller than `min`.
    pub(crate) fn resize(self, r: &Rect, d: Pt, min: f32) -> Rect {
        let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.right(), r.bottom());
        let (w, e, n, s) = match self {
            Handle::N => (false, false, true, false),
            Handle::NE => (false, true, true, false),
            Handle::E => (false, true, false, false),
            Handle::SE => (false, true, false, true),
            Handle::S => (false, false, false, true),
            Handle::SW => (true, false, false, true),
            Handle::W => (true, false, false, false),
            Handle::NW => (true, false, true, false),
        };
        if w {
            x0 = (x0 + d.0).min(x1 - min);
        }
        if e {
            x1 = (x1 + d.0).max(x0 + min);
        }
        if n {
            y0 = (y0 + d.1).min(y1 - min);
        }
        if s {
            y1 = (y1 + d.1).max(y0 + min);
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// A guide line shown while dragging, in diagram space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Guide {
    pub from: Pt,
    pub to: Pt,
}

/// Nudge a moving box so its edges or centre line up with another box's, within `tol`.
/// Falls back to the grid when nothing lines up. Returns the correction and the guides.
pub(crate) fn snap(
    moving: &Rect,
    others: &[Rect],
    tol: f32,
    grid: Option<f32>,
) -> (Pt, Vec<Guide>) {
    let xs = |r: &Rect| [r.x, r.x + r.w / 2., r.right()];
    let ys = |r: &Rect| [r.y, r.y + r.h / 2., r.bottom()];
    let mut best_x: Option<(f32, f32)> = None; // (correction, line x)
    let mut best_y: Option<(f32, f32)> = None;
    for o in others {
        for mx in xs(moving) {
            for ox in xs(o) {
                let d = ox - mx;
                if d.abs() <= tol && best_x.is_none_or(|(b, _)| d.abs() < b.abs()) {
                    best_x = Some((d, ox));
                }
            }
        }
        for my in ys(moving) {
            for oy in ys(o) {
                let d = oy - my;
                if d.abs() <= tol && best_y.is_none_or(|(b, _)| d.abs() < b.abs()) {
                    best_y = Some((d, oy));
                }
            }
        }
    }
    let grid_fix = |v: f32| grid.map(|g| (v / g).round() * g - v).unwrap_or(0.);
    let dx = best_x.map(|(d, _)| d).unwrap_or_else(|| grid_fix(moving.x));
    let dy = best_y.map(|(d, _)| d).unwrap_or_else(|| grid_fix(moving.y));
    let snapped = Rect::new(moving.x + dx, moving.y + dy, moving.w, moving.h);
    let mut guides = Vec::new();
    if let Some((_, gx)) = best_x {
        let mut y0 = snapped.y;
        let mut y1 = snapped.bottom();
        for o in others {
            if xs(o).iter().any(|x| (x - gx).abs() < 0.5) {
                y0 = y0.min(o.y);
                y1 = y1.max(o.bottom());
            }
        }
        guides.push(Guide {
            from: (gx, y0),
            to: (gx, y1),
        });
    }
    if let Some((_, gy)) = best_y {
        let mut x0 = snapped.x;
        let mut x1 = snapped.right();
        for o in others {
            if ys(o).iter().any(|y| (y - gy).abs() < 0.5) {
                x0 = x0.min(o.x);
                x1 = x1.max(o.right());
            }
        }
        guides.push(Guide {
            from: (x0, gy),
            to: (x1, gy),
        });
    }
    ((dx, dy), guides)
}

/// Ways to line up several boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
    /// Even horizontal gaps.
    SpreadX,
    /// Even vertical gaps.
    SpreadY,
    /// Same width and height as the first.
    SameSize,
}

/// New rectangles for the boxes after lining them up. The first box leads for SameSize.
pub(crate) fn align(rects: &[Rect], how: Align) -> Vec<Rect> {
    let Some(all) = Rect::bounds(rects.iter().copied()) else {
        return Vec::new();
    };
    let mut out: Vec<Rect> = rects.to_vec();
    match how {
        Align::Left => out.iter_mut().for_each(|r| r.x = all.x),
        Align::Right => out.iter_mut().for_each(|r| r.x = all.right() - r.w),
        Align::Center => out
            .iter_mut()
            .for_each(|r| r.x = all.x + all.w / 2. - r.w / 2.),
        Align::Top => out.iter_mut().for_each(|r| r.y = all.y),
        Align::Bottom => out.iter_mut().for_each(|r| r.y = all.bottom() - r.h),
        Align::Middle => out
            .iter_mut()
            .for_each(|r| r.y = all.y + all.h / 2. - r.h / 2.),
        Align::SameSize => {
            let (w, h) = (rects[0].w, rects[0].h);
            out.iter_mut().for_each(|r| {
                r.w = w;
                r.h = h;
            });
        }
        Align::SpreadX | Align::SpreadY => {
            if rects.len() < 3 {
                return out;
            }
            let horizontal = how == Align::SpreadX;
            let mut order: Vec<usize> = (0..rects.len()).collect();
            order.sort_by(|&a, &b| {
                let (ka, kb) = if horizontal {
                    (rects[a].x, rects[b].x)
                } else {
                    (rects[a].y, rects[b].y)
                };
                ka.total_cmp(&kb)
            });
            let size: f32 = rects
                .iter()
                .map(|r| if horizontal { r.w } else { r.h })
                .sum();
            let span = if horizontal { all.w } else { all.h };
            let gap = (span - size) / (rects.len() - 1) as f32;
            let mut at = if horizontal { all.x } else { all.y };
            for i in order {
                if horizontal {
                    out[i].x = at;
                    at += out[i].w + gap;
                } else {
                    out[i].y = at;
                    at += out[i].h + gap;
                }
            }
        }
    }
    out
}

/// A layered layout: each box goes one layer below everything that points at it, and
/// boxes in a layer are ordered to keep connectors short. Returns top-left corners.
pub(crate) fn layered(
    ids: &[String],
    sizes: &HashMap<String, Pt>,
    links: &[(String, String)],
    origin: Pt,
    horizontal: bool,
) -> HashMap<String, Pt> {
    let index: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let n = ids.len();
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (a, b) in links {
        if let (Some(&i), Some(&j)) = (index.get(a.as_str()), index.get(b.as_str())) {
            if i != j && !out_edges[i].contains(&j) {
                out_edges[i].push(j);
            }
        }
    }
    // Break cycles: drop edges that go back to a node still on the DFS stack.
    let mut state = vec![0u8; n];
    let mut keep: Vec<Vec<usize>> = vec![Vec::new(); n];
    fn dfs(v: usize, out: &[Vec<usize>], state: &mut [u8], keep: &mut [Vec<usize>]) {
        state[v] = 1;
        for &w in &out[v] {
            if state[w] == 1 {
                continue;
            }
            keep[v].push(w);
            if state[w] == 0 {
                dfs(w, out, state, keep);
            }
        }
        state[v] = 2;
    }
    for v in 0..n {
        if state[v] == 0 {
            dfs(v, &out_edges, &mut state, &mut keep);
        }
    }
    // Longest path from the sources.
    let mut indeg = vec![0usize; n];
    for outs in &keep {
        for &w in outs {
            indeg[w] += 1;
        }
    }
    let mut layer = vec![0usize; n];
    let mut queue: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let v = queue[qi];
        qi += 1;
        for &w in &keep[v] {
            layer[w] = layer[w].max(layer[v] + 1);
            indeg[w] -= 1;
            if indeg[w] == 0 {
                queue.push(w);
            }
        }
    }
    let depth = layer.iter().copied().max().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for v in 0..n {
        layers[layer[v]].push(v);
    }
    // Order each layer by the average position of its neighbours, a few sweeps each way.
    let mut ins: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (v, outs) in keep.iter().enumerate() {
        for &w in outs {
            ins[w].push(v);
        }
    }
    let mut pos = vec![0f32; n];
    let set_pos = |layers: &Vec<Vec<usize>>, pos: &mut Vec<f32>| {
        for l in layers {
            for (i, &v) in l.iter().enumerate() {
                pos[v] = i as f32;
            }
        }
    };
    set_pos(&layers, &mut pos);
    for sweep in 0..6 {
        let down = sweep % 2 == 0;
        let range: Vec<usize> = if down {
            (1..depth).collect()
        } else {
            (0..depth.saturating_sub(1)).rev().collect()
        };
        for li in range {
            let mut keyed: Vec<(f32, usize)> = layers[li]
                .iter()
                .map(|&v| {
                    let nb = if down { &ins[v] } else { &keep[v] };
                    let k = if nb.is_empty() {
                        pos[v]
                    } else {
                        nb.iter().map(|&u| pos[u]).sum::<f32>() / nb.len() as f32
                    };
                    (k, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            layers[li] = keyed.into_iter().map(|(_, v)| v).collect();
            set_pos(&layers, &mut pos);
        }
    }
    // Place: layers stacked along the main axis, boxes spread and centred across it.
    let size = |v: usize| sizes.get(&ids[v]).copied().unwrap_or((180., 90.));
    let (gap_across, gap_along) = (60., 90.);
    let across = |v: usize| if horizontal { size(v).1 } else { size(v).0 };
    let along = |v: usize| if horizontal { size(v).0 } else { size(v).1 };
    let widths: Vec<f32> = layers
        .iter()
        .map(|l| {
            l.iter().map(|&v| across(v)).sum::<f32>()
                + gap_across * l.len().saturating_sub(1) as f32
        })
        .collect();
    let widest = widths.iter().copied().fold(0., f32::max);
    let mut result = HashMap::new();
    let mut main = 0.;
    for (li, l) in layers.iter().enumerate() {
        let thick = l.iter().map(|&v| along(v)).fold(0., f32::max);
        let mut cross = (widest - widths[li]) / 2.;
        for &v in l {
            let off_main = main + (thick - along(v)) / 2.;
            let p = if horizontal {
                (origin.0 + off_main, origin.1 + cross)
            } else {
                (origin.0 + cross, origin.1 + off_main)
            };
            result.insert(ids[v].clone(), p);
            cross += across(v) + gap_across;
        }
        main += thick + gap_along;
    }
    result
}

/// Cycles among the links: every link that sits on one.
pub(crate) fn cyclic_links(links: &[(String, String)]) -> HashSet<(String, String)> {
    // Strongly connected components (Tarjan); a link inside a component is on a cycle.
    let mut ids: BTreeMap<&str, usize> = BTreeMap::new();
    for (a, b) in links {
        let n = ids.len();
        ids.entry(a.as_str()).or_insert(n);
        let n = ids.len();
        ids.entry(b.as_str()).or_insert(n);
    }
    let n = ids.len();
    let mut adj = vec![Vec::new(); n];
    for (a, b) in links {
        adj[ids[a.as_str()]].push(ids[b.as_str()]);
    }
    struct T {
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        comp: Vec<usize>,
        comps: usize,
    }
    fn go(v: usize, adj: &[Vec<usize>], t: &mut T) {
        t.index[v] = Some(t.next);
        t.low[v] = t.next;
        t.next += 1;
        t.stack.push(v);
        t.on[v] = true;
        for &w in &adj[v] {
            match t.index[w] {
                None => {
                    go(w, adj, t);
                    t.low[v] = t.low[v].min(t.low[w]);
                }
                Some(iw) if t.on[w] => t.low[v] = t.low[v].min(iw),
                _ => {}
            }
        }
        if Some(t.low[v]) == t.index[v] {
            while let Some(w) = t.stack.pop() {
                t.on[w] = false;
                t.comp[w] = t.comps;
                if w == v {
                    break;
                }
            }
            t.comps += 1;
        }
    }
    let mut t = T {
        index: vec![None; n],
        low: vec![0; n],
        on: vec![false; n],
        stack: Vec::new(),
        next: 0,
        comp: vec![0; n],
        comps: 0,
    };
    for v in 0..n {
        if t.index[v].is_none() {
            go(v, &adj, &mut t);
        }
    }
    links
        .iter()
        .filter(|(a, b)| a != b && t.comp[ids[a.as_str()]] == t.comp[ids[b.as_str()]])
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elbows_leave_by_the_facing_sides() {
        let a = Rect::new(0., 0., 100., 50.);
        let right = Rect::new(300., 100., 100., 50.);
        let w = route(&a, &right, Route::Elbow);
        assert_eq!(w.points.first(), Some(&(100., 25.)));
        assert_eq!(w.points.last(), Some(&(300., 125.)));
        assert_eq!(w.points.len(), 4);
        let below = Rect::new(0., 200., 100., 50.);
        let w = route(&a, &below, Route::Elbow);
        assert_eq!(w.points, vec![(50., 50.), (50., 200.)]);
    }

    #[test]
    fn straight_lines_stop_at_the_box_edges() {
        let a = Rect::new(0., 0., 100., 100.);
        let b = Rect::new(300., 0., 100., 100.);
        let w = route(&a, &b, Route::Straight);
        assert_eq!(w.points, vec![(100., 50.), (300., 50.)]);
        assert_eq!(w.middle(), (200., 50.));
    }

    #[test]
    fn curves_flatten_for_hit_tests() {
        let a = Rect::new(0., 0., 100., 100.);
        let b = Rect::new(400., 0., 100., 100.);
        let w = route(&a, &b, Route::Curved);
        assert!(w.curved);
        assert!(wire_dist(&w, (250., 50.)) < 1.);
        assert!(wire_dist(&w, (250., 150.)) > 50.);
    }

    #[test]
    fn handles_resize_and_keep_a_minimum() {
        let r = Rect::new(0., 0., 100., 100.);
        assert_eq!(
            Handle::SE.resize(&r, (20., 10.), 20.),
            Rect::new(0., 0., 120., 110.)
        );
        assert_eq!(
            Handle::NW.resize(&r, (200., 0.), 20.),
            Rect::new(80., 0., 20., 100.)
        );
    }

    #[test]
    fn snapping_lines_up_edges_and_centres() {
        let other = Rect::new(0., 0., 100., 100.);
        let moving = Rect::new(203., 47., 50., 50.);
        let ((dx, dy), guides) = snap(&moving, &[other], 5., Some(10.));
        // Its centre (228+, 72) is far in x, so x snaps to the grid; its top lines up with
        // the other box's centre line at 50.
        assert_eq!(dx, -3.);
        assert_eq!(dy, 3.);
        assert_eq!(guides.len(), 1);
        assert_eq!(guides[0].from.1, 50.);
    }

    #[test]
    fn align_and_spread() {
        let rects = [
            Rect::new(0., 0., 10., 10.),
            Rect::new(50., 20., 20., 10.),
            Rect::new(100., 5., 10., 30.),
        ];
        let left = align(&rects, Align::Left);
        assert!(left.iter().all(|r| r.x == 0.));
        let bottom = align(&rects, Align::Bottom);
        assert!(bottom.iter().all(|r| r.bottom() == 35.));
        let spread = align(&[rects[0], rects[2], rects[1]], Align::SpreadX);
        // Total 110 wide, 40 of boxes, so two gaps of 35.
        assert_eq!(spread[2].x, 45.);
        let same = align(&rects, Align::SameSize);
        assert!(same.iter().all(|r| r.w == 10. && r.h == 10.));
    }

    #[test]
    fn layered_layout_puts_callers_above_callees() {
        let ids: Vec<String> = ["web", "api", "db", "cache"].map(String::from).to_vec();
        let links = vec![
            ("web".to_string(), "api".to_string()),
            ("api".to_string(), "db".to_string()),
            ("api".to_string(), "cache".to_string()),
            // A cycle back to the top must not loop forever.
            ("db".to_string(), "web".to_string()),
        ];
        let pos = layered(&ids, &HashMap::new(), &links, (0., 0.), false);
        assert!(pos["web"].1 < pos["api"].1);
        assert!(pos["api"].1 < pos["db"].1);
        assert_eq!(pos["db"].1, pos["cache"].1);
        assert_ne!(pos["db"].0, pos["cache"].0);
    }

    #[test]
    fn cycles_are_found() {
        let links: Vec<(String, String)> = [("a", "b"), ("b", "c"), ("c", "a"), ("c", "d")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let on = cyclic_links(&links);
        assert_eq!(on.len(), 3);
        assert!(!on.contains(&("c".to_string(), "d".to_string())));
    }
}
