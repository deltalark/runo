//! Geometry for the diagram editor: connector routes, hit testing, snapping guides,
//! align and distribute, and the layered auto-layout.

use super::model::{Rect, Route};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(crate) type Pt = (f32, f32);

/// The side of a box a connector leaves or enters by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub(crate) const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Side::Top => "Top",
            Side::Right => "Right",
            Side::Bottom => "Bottom",
            Side::Left => "Left",
        }
    }

    fn across(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }

    /// Where a connector meets this side: `at` along it, from 0 to 1, else the middle.
    pub(crate) fn port(self, r: &Rect, at: Option<f32>) -> Pt {
        let Some(t) = at else {
            return self.point(r);
        };
        match self {
            Side::Top => (r.x + r.w * t, r.y),
            Side::Right => (r.right(), r.y + r.h * t),
            Side::Bottom => (r.x + r.w * t, r.bottom()),
            Side::Left => (r.x, r.y + r.h * t),
        }
    }

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
pub(crate) fn sides(a: &Rect, b: &Rect) -> (Side, Side) {
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
    route_with(a, b, kind, &Way::default())
}

/// What a connector's route respects beyond its two boxes: sides it is pinned to, bends the
/// user placed, and boxes it should go around.
#[derive(Default)]
pub(crate) struct Way<'a> {
    pub from: Option<Side>,
    pub to: Option<Side>,
    pub waypoints: &'a [Pt],
    pub obstacles: &'a [Rect],
    /// Where along its side each end meets its box (0 to 1), when ends share a side.
    pub from_at: Option<f32>,
    pub to_at: Option<f32>,
}

/// The side of `r` that faces `p`.
pub(crate) fn side_toward(r: &Rect, p: Pt) -> Side {
    let (cx, cy) = r.center();
    let (dx, dy) = ((p.0 - cx) / r.w.max(1.), (p.1 - cy) / r.h.max(1.));
    if dx.abs() >= dy.abs() {
        if dx >= 0. {
            Side::Right
        } else {
            Side::Left
        }
    } else if dy >= 0. {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// Drop repeated points and bends that don't bend.
fn simplify(points: &mut Vec<Pt>) {
    points.dedup_by(|p, q| dist(*p, *q) < 0.5);
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
}

/// Whether a straight axis-aligned segment passes through the inside of `r`.
fn seg_hits(a: Pt, b: Pt, r: &Rect) -> bool {
    let e = 0.5;
    if (a.1 - b.1).abs() < e {
        a.1 > r.y + e
            && a.1 < r.bottom() - e
            && a.0.max(b.0) > r.x + e
            && a.0.min(b.0) < r.right() - e
    } else if (a.0 - b.0).abs() < e {
        a.0 > r.x + e
            && a.0 < r.right() - e
            && a.1.max(b.1) > r.y + e
            && a.1.min(b.1) < r.bottom() - e
    } else {
        // A diagonal: test its box, which is enough to send it round.
        Rect::span(a, b).intersects(r)
    }
}

/// Room kept between a routed connector and the boxes it passes.
const CLEARANCE: f32 = 14.;

/// An orthogonal path from `start` (leaving by `sa`) to `end` (entering by `sb`) that keeps
/// out of `obstacles`, with as few bends as it can. None if boxes wall it in.
fn ortho_path(start: Pt, sa: Side, end: Pt, sb: Side, obstacles: &[Rect]) -> Option<Vec<Pt>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let stub = CLEARANCE + 2.;
    let (na, nb) = (sa.normal(), sb.normal());
    let s1 = (start.0 + na.0 * stub, start.1 + na.1 * stub);
    let e1 = (end.0 + nb.0 * stub, end.1 + nb.1 * stub);
    let walls: Vec<Rect> = obstacles.iter().map(|r| r.inflate(CLEARANCE)).collect();
    let mut xs = vec![s1.0, e1.0, (s1.0 + e1.0) / 2.];
    let mut ys = vec![s1.1, e1.1, (s1.1 + e1.1) / 2.];
    for w in &walls {
        xs.extend([w.x, w.right()]);
        ys.extend([w.y, w.bottom()]);
    }
    let tidy = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    };
    tidy(&mut xs);
    tidy(&mut ys);
    let find = |v: &[f32], x: f32| v.iter().position(|y| (y - x).abs() < 0.5);
    let (si, sj) = (find(&xs, s1.0)?, find(&ys, s1.1)?);
    let (ei, ej) = (find(&xs, e1.0)?, find(&ys, e1.1)?);
    let (nx, ny) = (xs.len(), ys.len());
    let inside = |p: Pt| {
        walls.iter().any(|w| {
            p.0 > w.x + 0.5 && p.0 < w.right() - 0.5 && p.1 > w.y + 0.5 && p.1 < w.bottom() - 0.5
        })
    };
    // State: grid point and the axis it was reached along (0 across, 1 up and down).
    let idx = |i: usize, j: usize, d: usize| (j * nx + i) * 2 + d;
    let mut best = vec![f32::INFINITY; nx * ny * 2];
    let mut prev = vec![usize::MAX; nx * ny * 2];
    let mut heap = BinaryHeap::new();
    let d0 = usize::from(!sa.across());
    best[idx(si, sj, d0)] = 0.;
    heap.push((Reverse(0u64), idx(si, sj, d0)));
    const BEND: f32 = 40.;
    let key = |c: f32| Reverse((c * 16.) as u64);
    let goal_dir = usize::from(!sb.across());
    let mut found = None;
    while let Some((Reverse(_), st)) = heap.pop() {
        let (cell, d) = (st / 2, st % 2);
        let (i, j) = (cell % nx, cell / nx);
        let cost = best[st];
        if i == ei && j == ej {
            // Arriving along the axis we enter by saves a last bend.
            let total = cost + if d == goal_dir { 0. } else { BEND };
            if found.is_none_or(|(c, _)| total < c) {
                found = Some((total, st));
            }
            continue;
        }
        if found.is_some_and(|(c, _)| cost >= c) {
            break;
        }
        let here = (xs[i], ys[j]);
        let steps: [(isize, isize, usize); 4] = [(1, 0, 0), (-1, 0, 0), (0, 1, 1), (0, -1, 1)];
        for (di, dj, nd) in steps {
            let (ni, nj) = (i as isize + di, j as isize + dj);
            if ni < 0 || nj < 0 || ni >= nx as isize || nj >= ny as isize {
                continue;
            }
            let (ni, nj) = (ni as usize, nj as usize);
            let there = (xs[ni], ys[nj]);
            let at_end = ni == ei && nj == ej;
            if !at_end && inside(there) {
                continue;
            }
            if walls.iter().any(|w| seg_hits(here, there, w)) {
                continue;
            }
            let c = cost + dist(here, there) + if nd == d { 0. } else { BEND };
            let ns = idx(ni, nj, nd);
            if c < best[ns] {
                best[ns] = c;
                prev[ns] = st;
                heap.push((key(c), ns));
            }
        }
    }
    let (_, mut st) = found?;
    let mut pts = Vec::new();
    loop {
        let cell = st / 2;
        pts.push((xs[cell % nx], ys[cell / nx]));
        if prev[st] == usize::MAX {
            break;
        }
        st = prev[st];
    }
    pts.reverse();
    let mut out = vec![start];
    out.extend(pts);
    out.push(end);
    simplify(&mut out);
    Some(out)
}

/// Join points with right-angled steps, turning first across the way the last step went.
fn orthogonal_through(points: &[Pt], first_across: bool) -> Vec<Pt> {
    let mut out = vec![points[0]];
    let mut across = first_across;
    for w in points.windows(2) {
        let (p, q) = (w[0], w[1]);
        if (p.0 - q.0).abs() > 0.5 && (p.1 - q.1).abs() > 0.5 {
            out.push(if across { (q.0, p.1) } else { (p.0, q.1) });
        } else {
            across = (p.1 - q.1).abs() < 0.5;
        }
        out.push(q);
        across = !across;
    }
    simplify(&mut out);
    out
}

/// Where a connector's bend handles sit: the middle of each segment.
pub(crate) fn bend_handles(points: &[Pt]) -> Vec<Pt> {
    points
        .windows(2)
        .map(|s| ((s[0].0 + s[1].0) / 2., (s[0].1 + s[1].1) / 2.))
        .collect()
}

/// The bends a connector keeps after segment `seg` of `points` is dragged to `at`. A
/// straight connector gains a bend there; an elbow's segment slides across, with a short
/// step added where it leaves or reaches a box.
pub(crate) fn bend(points: &[Pt], seg: usize, at: Pt, straight: bool) -> Vec<Pt> {
    let n = points.len();
    if n < 2 || seg + 1 >= n {
        return points
            .get(1..n.saturating_sub(1))
            .unwrap_or_default()
            .to_vec();
    }
    let mut inner: Vec<Pt> = points[1..n - 1].to_vec();
    if straight {
        inner.insert(seg, at);
        return inner;
    }
    let (p, q) = (points[seg], points[seg + 1]);
    let across = (p.1 - q.1).abs() < 0.5;
    let shift = |pt: Pt| if across { (pt.0, at.1) } else { (at.0, pt.1) };
    // A short step out of the box, so the end stays on its side.
    const STEP: f32 = 20.;
    let toward = |from: Pt, to: Pt| {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let len = (dx * dx + dy * dy).sqrt().max(1.);
        let k = STEP.min(len / 3.);
        (from.0 + dx / len * k, from.1 + dy / len * k)
    };
    let mut out = Vec::new();
    if seg == 0 {
        let s = toward(p, q);
        out.push(s);
        out.push(shift(s));
    } else {
        out.extend_from_slice(&inner[..seg - 1]);
        out.push(shift(p));
    }
    if seg + 1 == n - 1 {
        let e = toward(q, p);
        out.push(shift(e));
        out.push(e);
    } else {
        out.push(shift(q));
        out.extend_from_slice(&inner[seg + 1..]);
    }
    out
}

/// The route of a connector from box `a` to box `b`, keeping to `way`.
pub(crate) fn route_with(a: &Rect, b: &Rect, kind: Route, way: &Way) -> Wire {
    if a == b {
        // A box that talks to itself: a loop off its right side.
        let (x, y) = (a.right(), a.y + a.h * 0.3);
        let y2 = a.y + a.h * 0.7;
        return Wire {
            points: vec![(x, y), (x + 40., y), (x + 40., y2), (x, y2)],
            curved: false,
        };
    }
    let wps = way.waypoints;
    let (auto_a, auto_b) = match (wps.first(), wps.last()) {
        (Some(f), Some(l)) => (side_toward(a, *f), side_toward(b, *l)),
        _ => sides(a, b),
    };
    let (sa, sb) = (way.from.unwrap_or(auto_a), way.to.unwrap_or(auto_b));
    match kind {
        Route::Straight => {
            let start = match (way.from, wps.first()) {
                (Some(s), _) => s.port(a, way.from_at),
                (None, Some(f)) => clip(a, *f),
                (None, None) => clip(a, b.center()),
            };
            let end = match (way.to, wps.last()) {
                (Some(s), _) => s.port(b, way.to_at),
                (None, Some(l)) => clip(b, *l),
                (None, None) => clip(b, a.center()),
            };
            let mut points = vec![start];
            points.extend_from_slice(wps);
            points.push(end);
            Wire {
                points,
                curved: false,
            }
        }
        Route::Elbow if !wps.is_empty() => {
            let mut through = vec![sa.port(a, way.from_at)];
            through.extend_from_slice(wps);
            through.push(sb.port(b, way.to_at));
            let mut points = orthogonal_through(&through, sa.across());
            // Come into the end box square to its side.
            let n = points.len();
            if n >= 3 {
                let (p, q) = (points[n - 3], points[n - 2]);
                let end = points[n - 1];
                let last_across = (q.1 - end.1).abs() < 0.5;
                if last_across != sb.across() && (p.0 - q.0).abs() + (p.1 - q.1).abs() > 0.5 {
                    points[n - 2] = if sb.across() {
                        (q.0, end.1)
                    } else {
                        (end.0, q.1)
                    };
                    simplify(&mut points);
                }
            }
            Wire {
                points,
                curved: false,
            }
        }
        Route::Elbow => {
            let start = sa.port(a, way.from_at);
            let end = sb.port(b, way.to_at);
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
            simplify(&mut points);
            // Go round boxes in the way, when there are any.
            let blocked = |pts: &[Pt]| {
                pts.windows(2).any(|s| {
                    way.obstacles
                        .iter()
                        .any(|r| seg_hits(s[0], s[1], &r.inflate(4.)))
                })
            };
            if !way.obstacles.is_empty() && blocked(&points) {
                let mut walls: Vec<Rect> = way.obstacles.to_vec();
                walls.extend([*a, *b]);
                if let Some(p) = ortho_path(start, sa, end, sb, &walls) {
                    points = p;
                }
            }
            Wire {
                points,
                curved: false,
            }
        }
        Route::Curved => {
            let start = sa.port(a, way.from_at);
            let end = sb.port(b, way.to_at);
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

/// A layered layout of the boxes; see `tidy::layered`.
pub(crate) fn layered(
    ids: &[String],
    sizes: &HashMap<String, Pt>,
    links: &[(String, String)],
    origin: Pt,
    horizontal: bool,
) -> HashMap<String, Pt> {
    super::tidy::layered(ids, sizes, links, origin, horizontal)
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

    #[test]
    fn elbows_go_round_boxes_in_the_way() {
        let a = Rect::new(0., 0., 100., 60.);
        let b = Rect::new(400., 0., 100., 60.);
        let wall = Rect::new(200., -40., 100., 140.);
        let plain = route(&a, &b, Route::Elbow);
        assert!(plain.points.windows(2).any(|s| seg_hits(s[0], s[1], &wall)));
        let w = route_with(
            &a,
            &b,
            Route::Elbow,
            &Way {
                obstacles: &[wall],
                ..Default::default()
            },
        );
        assert!(
            !w.points.windows(2).any(|s| seg_hits(s[0], s[1], &wall)),
            "{:?}",
            w.points
        );
        assert_eq!(w.points.first(), Some(&(100., 30.)));
        assert_eq!(w.points.last(), Some(&(400., 30.)));
        for s in w.points.windows(2) {
            assert!((s[0].0 - s[1].0).abs() < 0.5 || (s[0].1 - s[1].1).abs() < 0.5);
        }
    }

    #[test]
    fn pinned_sides_and_waypoints_shape_the_route() {
        let a = Rect::new(0., 0., 100., 60.);
        let b = Rect::new(400., 0., 100., 60.);
        let w = route_with(
            &a,
            &b,
            Route::Elbow,
            &Way {
                from: Some(Side::Bottom),
                to: Some(Side::Bottom),
                ..Default::default()
            },
        );
        assert_eq!(w.points.first(), Some(&(50., 60.)));
        assert_eq!(w.points.last(), Some(&(450., 60.)));
        let wps = [(250., 200.)];
        let w = route_with(
            &a,
            &b,
            Route::Elbow,
            &Way {
                waypoints: &wps,
                ..Default::default()
            },
        );
        let on = w
            .points
            .windows(2)
            .any(|s| seg_dist((250., 200.), s[0], s[1]) < 0.5);
        assert!(on, "{:?}", w.points);
        for s in w.points.windows(2) {
            assert!((s[0].0 - s[1].0).abs() < 0.5 || (s[0].1 - s[1].1).abs() < 0.5);
        }
        let s = route_with(
            &a,
            &b,
            Route::Straight,
            &Way {
                waypoints: &wps,
                ..Default::default()
            },
        );
        assert_eq!(s.points.len(), 3);
    }

    #[test]
    fn dragging_a_segment_bends_the_connector() {
        // An elbow: right, down, right.
        let pts = vec![(0., 0.), (50., 0.), (50., 100.), (100., 100.)];
        let w = bend(&pts, 1, (70., 40.), false);
        assert_eq!(w, vec![(70., 0.), (70., 100.)]);
        // Its first segment gets a step out of the box.
        let w = bend(&pts, 0, (20., -30.), false);
        assert_eq!(w[0].1, 0.);
        assert_eq!(w[1], (w[0].0, -30.));
        assert_eq!(w[2], (50., -30.));
        assert_eq!(w[3], (50., 100.));
        // A straight connector gains a bend where it's dragged.
        let w = bend(&[(0., 0.), (100., 0.)], 0, (50., 40.), true);
        assert_eq!(w, vec![(50., 40.)]);
        assert_eq!(bend_handles(&pts)[1], (50., 50.));
    }
}
