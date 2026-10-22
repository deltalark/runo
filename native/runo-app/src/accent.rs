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
/// `paint`. Each lit corner must sit between two straight edges.
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

impl Step {
    fn end(&self) -> Point<Pixels> {
        match self {
            Step::Line(to) | Step::Corner { to, .. } => *to,
        }
    }
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
    /// unbroken stroke round the corner and out along both its edges, in the accent at
    /// the corner and fading to nothing `fade` along each edge (or halfway along an edge
    /// lit at both ends), over a faint wider glow. One path per corner, so its curve is
    /// as smooth as the outline's own.
    pub fn paint(self, hover: f32, window: &mut Window) {
        let n = self.steps.len();
        let begin = |i: usize| match i {
            0 => self.start,
            _ => self.steps[i - 1].end(),
        };
        let lit = |i: usize| matches!(self.steps[i], Step::Corner { lit: true, .. });
        let fade = px(34. + 18. * hover);
        let len = |d: Point<Pixels>| (f32::from(d.x).powi(2) + f32::from(d.y).powi(2)).sqrt();
        let color = color(hover);
        for i in 0..n {
            let Step::Corner { c, to, lit: true } = self.steps[i] else {
                continue;
            };
            let from = begin(i);
            // The straight edges either side, and whether the corner beyond each is lit too.
            let prev = if i > 0 {
                Some(i - 1)
            } else {
                self.closed.then_some(n - 1)
            };
            let next = if i + 1 < n {
                Some(i + 1)
            } else {
                self.closed.then_some(0)
            };
            let arm = |edge: Option<usize>, at: Point<Pixels>, far_side: usize| {
                let edge = edge.filter(|&e| matches!(self.steps[e], Step::Line(_)))?;
                let other = match far_side {
                    0 => begin(edge),
                    _ => self.steps[edge].end(),
                };
                let beyond = match far_side {
                    0 => edge.checked_sub(1).or(self.closed.then_some(n - 1)),
                    _ => Some(edge + 1)
                        .filter(|&e| e < n)
                        .or(self.closed.then_some(0)),
                };
                let span = len(other - at);
                let span = if beyond.is_some_and(lit) {
                    span / 2.
                } else {
                    span
                };
                let reach = f32::from(fade).min(span);
                (span > 0.).then(|| at + (other - at) * (reach / span))
            };
            let a = arm(prev, from, 0).unwrap_or(from);
            let b = arm(next, to, 1).unwrap_or(to);
            // gpui's gradients don't follow a bent path, so the fade is built from
            // layers: each the whole corner and its arms out to a step further, so the
            // corner is full and each arm thins out in even steps to nothing.
            const LAYERS: usize = 12;
            let k = 0.5523;
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
                    let mut f = PathBuilder::stroke(px(width));
                    f.move_to(from + (a - from) * t);
                    f.line_to(from);
                    f.cubic_bezier_to(to, from + (c - from) * k, to + (c - to) * k);
                    f.line_to(to + (b - to) * t);
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
