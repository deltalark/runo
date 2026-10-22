//! The accent's lit rim: a selected row or an open tab has its own outline lit in the
//! accent at two of its corners, fading out along the edges between them, like light
//! catching the edge. While the pointer is over it the light reaches further and
//! brightens, easing in and out. With Animations off it snaps.

use super::*;
use std::cell::RefCell;
use std::time::Instant;

thread_local! {
    /// Whether the pointer was over each bracket's tab or row when it last painted, and
    /// since when, keyed by the bracket.
    static HOVERS: RefCell<HashMap<SharedString, (bool, Instant)>> =
        RefCell::new(HashMap::new());
}

/// How long a bracket takes to reach out or settle back when hovered.
const HOVER_MS: f32 = 180.;

/// How far the bracket `key` is into its hovered look, from 0 to 1, with the pointer
/// over `area` (its tab or row) or not. Eases both ways, and redraws the view when the
/// pointer crosses into or out of `area`, so it needs nothing from the element it marks.
/// Call it while painting the bracket.
pub fn hover(key: &SharedString, area: Bounds<Pixels>, window: &mut Window) -> f32 {
    let over = area.contains(&window.mouse_position());
    let view = window.current_view();
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && area.contains(&e.position) != over {
            cx.notify(view);
        }
    });
    let now = Instant::now();
    let since = HOVERS.with_borrow_mut(|h| {
        // First seen: already settled, so nothing moves on arrival.
        let seen = h
            .entry(key.clone())
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
    if over {
        t
    } else {
        1. - t
    }
}

/// The bracket's colour, `hover` of the way to its hovered look: the accent, a step
/// brighter while hovered. `mix(a, b, t)` takes `t` of `a`, so the white goes first: at
/// rest none of it, hovered a quarter (a tenth on light).
pub fn color(hover: f32) -> Hsla {
    rgb(mix(
        0xffffff,
        c_blue(),
        hover * if is_dark() { 0.25 } else { 0.1 },
    ))
    .into()
}

/// An outline traced with straight edges and quarter turns, and the corners its rim is
/// lit at. Start it with `new`, trace it with `line` and `corner`, and paint it with
/// `paint`.
pub struct Rim {
    start: Point<Pixels>,
    steps: Vec<Step>,
    closed: bool,
}

enum Step {
    Line(Point<Pixels>),
    Corner {
        c: Point<Pixels>,
        to: Point<Pixels>,
        lit: bool,
    },
}

impl Rim {
    pub fn new(from: Point<Pixels>) -> Self {
        Self {
            start: from,
            steps: Vec::new(),
            closed: false,
        }
    }

    pub fn line(&mut self, to: Point<Pixels>) {
        self.steps.push(Step::Line(to));
    }

    /// A quarter turn from where the outline is to `to` round the square corner `c`, lit
    /// when `lit`.
    pub fn corner(&mut self, c: Point<Pixels>, to: Point<Pixels>, lit: bool) {
        self.steps.push(Step::Corner { c, to, lit });
    }

    /// Joins the outline's end back to its start.
    pub fn close(mut self) -> Self {
        self.closed = true;
        self
    }

    /// Paints the light at each lit corner, `hover` of the way to its hovered look: one
    /// unbroken stroke round the corner and on along the outline either side, through
    /// any curves it meets, in the accent at the corner and fading to nothing `fade`
    /// along (or halfway to the next lit corner, or at the outline's end), over a faint
    /// wider glow.
    pub fn paint(self, hover: f32, window: &mut Window) {
        // The outline as a dense polyline, each turn cut fine enough to read as a curve,
        // with how far along it each point is, and where each lit corner runs.
        const CUTS: usize = 16;
        let k = 0.5523;
        let mut pts = vec![self.start];
        let mut lit = Vec::new();
        for step in &self.steps {
            let from = *pts.last().unwrap();
            match *step {
                Step::Line(to) => pts.push(to),
                Step::Corner { c, to, lit: on } => {
                    let (c1, c2) = (from + (c - from) * k, to + (c - to) * k);
                    let first = pts.len() - 1;
                    for n in 1..=CUTS {
                        let t = n as f32 / CUTS as f32;
                        let u = 1. - t;
                        pts.push(
                            from * (u * u * u)
                                + c1 * (3. * u * u * t)
                                + c2 * (3. * u * t * t)
                                + to * (t * t * t),
                        );
                    }
                    if on {
                        lit.push((first, pts.len() - 1));
                    }
                }
            }
        }
        if self.closed {
            pts.push(self.start);
        }
        let len = |d: Point<Pixels>| (f32::from(d.x).powi(2) + f32::from(d.y).powi(2)).sqrt();
        let mut at = vec![0.];
        for w in pts.windows(2) {
            at.push(at.last().unwrap() + len(w[1] - w[0]));
        }
        let total = *at.last().unwrap();
        // The point `s` along the outline, wrapping round a closed one.
        let point_at = |s: f32| {
            let s = if self.closed {
                s.rem_euclid(total)
            } else {
                s.clamp(0., total)
            };
            let v = at.partition_point(|&a| a <= s).clamp(1, pts.len() - 1);
            let span = at[v] - at[v - 1];
            let t = if span > 0. {
                (s - at[v - 1]) / span
            } else {
                0.
            };
            pts[v - 1] + (pts[v] - pts[v - 1]) * t
        };
        // The outline from `lo` to `hi` along it.
        let stretch = |lo: f32, hi: f32| {
            let mut out = vec![point_at(lo)];
            let laps: &[f32] = if self.closed { &[-1., 0., 1.] } else { &[0.] };
            for lap in laps {
                for (p, a) in pts.iter().zip(&at) {
                    let a = a + lap * total;
                    if a > lo && a < hi {
                        out.push(*p);
                    }
                }
            }
            out.push(point_at(hi));
            out.dedup_by(|a, b| len(*a - *b) < 0.01);
            out
        };
        let fade = 34. + 18. * hover;
        let color = color(hover);
        let count = lit.len();
        for (n, &(a, b)) in lit.iter().enumerate() {
            let (sa, sb) = (at[a], at[b]);
            // How far the light may run back and on: `fade`, but no further than halfway
            // to the lit corner beside it, or the outline's end.
            let back = match (n, self.closed) {
                (0, false) => sa,
                (0, true) => {
                    let (_, pb) = lit[count - 1];
                    (sa - (at[pb] - total)) / 2.
                }
                _ => (sa - at[lit[n - 1].1]) / 2.,
            };
            let on = match (n + 1 == count, self.closed) {
                (true, false) => total - sb,
                (true, true) => {
                    let (na, _) = lit[0];
                    (at[na] + total - sb) / 2.
                }
                _ => (at[lit[n + 1].0] - sb) / 2.,
            };
            let (back, on) = (fade.min(back), fade.min(on));
            // gpui's gradients don't follow a bent path, so the fade is built from
            // layers: each the whole corner and the outline either side out to a step
            // further, so the corner is full and each side thins out in even steps.
            const LAYERS: usize = 12;
            for (width, strength) in [(5., 0.12), (2., 1.)] {
                for j in 1..=LAYERS {
                    // What the layers from j out must add up to over step j, and so what
                    // this one alone adds over the ones further out.
                    let want = |j: usize| 1. - (j as f32 - 1.) / LAYERS as f32;
                    let alpha = if j == LAYERS {
                        want(j)
                    } else {
                        1. - (1. - want(j)) / (1. - want(j + 1))
                    };
                    let t = j as f32 / LAYERS as f32;
                    let line = stretch(sa - back * t, sb + on * t);
                    let mut f = PathBuilder::stroke(px(width));
                    f.move_to(line[0]);
                    for p in &line[1..] {
                        f.line_to(*p);
                    }
                    if let Ok(path) = f.build() {
                        let mut lit = color;
                        lit.a *= alpha * strength;
                        window.paint_path(path, lit);
                    }
                }
            }
        }
    }
}
