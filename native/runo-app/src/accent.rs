//! The accent's glow strip: a selected row or an open tab has the edge it leads with lit
//! in the accent (the left edge of a row or the rail's tab, the top of a tab), brightest
//! at the middle and fading out before the corners. When it opens the light grows out
//! from the middle, and while the pointer is over it the light brightens, easing in and
//! out. With Animations off it snaps.

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

/// How long the light takes to grow out when something opens, after
/// a short hold that lets the motion it belongs to (a tab sliding out, a row landing)
/// come first.
const OPEN_MS: u64 = 520;
const OPEN_LEAD: f32 = 0.25;

/// The animation of the light growing out when something opens, from 0 to 1, easing
/// out. Every lit rim opens with it, so they all move alike.
pub fn opening() -> Animation {
    Animation::new(Duration::from_millis(OPEN_MS)).with_easing(|t: f32| {
        let t = ((t - OPEN_LEAD) / (1. - OPEN_LEAD)).clamp(0., 1.);
        1. - (1. - t).powi(4)
    })
}

/// A layer over its parent that paints a lit rim with `paint`, given its bounds and how
/// far the light has grown out (see `opening`). It grows out each time the layer is
/// mounted, such as when a row is picked, even during a page switch, so opening a place
/// or a tab always shows the same motion. With Animations off it is there at once.
pub fn drawn_in(
    id: impl Into<ElementId>,
    paint: impl Fn(Bounds<Pixels>, f32, &mut Window) + 'static,
) -> AnyElement {
    let paint = std::rc::Rc::new(paint);
    let layer = move |open: f32| {
        let paint = paint.clone();
        canvas(|_, _, _| (), move |b, _, window, _| paint(b, open, window)).size_full()
    };
    let el = div().absolute().inset_0();
    if anim_on() {
        el.with_animation(id, opening(), move |el, open| el.child(layer(open)))
            .into_any_element()
    } else {
        el.child(layer(1.)).into_any_element()
    }
}

/// An outline traced with straight edges and quarter turns, and the corners that bound
/// the edge it leads with. Start it with `new`, trace it with `line` and `corner`, and
/// paint it with `paint`.
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
        lead: bool,
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

    /// A quarter turn from where the outline is to `to` round the square corner `c`.
    /// `lead` marks a corner of the edge the item leads with: the edge between two such
    /// corners, or on a closed outline, the edge into the last one.
    pub fn corner(&mut self, c: Point<Pixels>, to: Point<Pixels>, lead: bool) {
        self.steps.push(Step::Corner { c, to, lead });
    }

    /// Joins the outline's end back to its start.
    pub fn close(mut self) -> Self {
        self.closed = true;
        self
    }

    /// Paints the glow on the edge the item leads with, `hover` of the way to its hovered
    /// look: a line in the accent, brightest at the edge's middle and fading to nothing
    /// by its corners, over a faint wider glow. `open` is how far it has grown out from
    /// the middle, from 0 to 1 (see `opening`).
    pub fn paint(self, hover: f32, open: f32, window: &mut Window) {
        // The outline as a polyline, each turn cut fine enough to read as a curve, with
        // how far along it each point is, and where each leading corner runs.
        const CUTS: usize = 16;
        let k = 0.5523;
        let mut pts = vec![self.start];
        let mut lead = Vec::new();
        for step in &self.steps {
            let from = *pts.last().unwrap();
            match *step {
                Step::Line(to) => pts.push(to),
                Step::Corner { c, to, lead: on } => {
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
                        lead.push((first, pts.len() - 1));
                    }
                }
            }
        }
        let len = |d: Point<Pixels>| (f32::from(d.x).powi(2) + f32::from(d.y).powi(2)).sqrt();
        let mut at = vec![0.];
        for w in pts.windows(2) {
            at.push(at.last().unwrap() + len(w[1] - w[0]));
        }
        // The leading edge, as how far along the outline it starts and ends.
        let Some((lo, hi)) = (match lead.as_slice() {
            [(_, b), (a, _)] if !self.closed => Some((at[*b], at[*a])),
            [.., (a, _)] if *a > 0 => Some((at[a - 1], at[*a])),
            _ => None,
        }) else {
            return;
        };
        // The point `s` along the outline.
        let point_at = |s: f32| {
            let v = at.partition_point(|&a| a <= s).clamp(1, pts.len() - 1);
            let span = at[v] - at[v - 1];
            let t = if span > 0. {
                (s - at[v - 1]) / span
            } else {
                0.
            };
            pts[v - 1] + (pts[v] - pts[v - 1]) * t
        };
        let color = color(hover);
        // gpui's gradients don't follow a path, so the fade is built from layers, each
        // out from the middle a step further, so the middle is full and each end thins
        // out in even steps.
        const LAYERS: usize = 8;
        let mid = (lo + hi) / 2.;
        let half = ((hi - lo) / 2. + 4.) * open;
        for (width, strength) in [(6., 0.1), (2., 1.)] {
            for j in 1..=LAYERS {
                let t = j as f32 / LAYERS as f32;
                let (a, b) = (mid - half * t, mid + half * t);
                let mut f = PathBuilder::stroke(px(width));
                f.move_to(point_at(a));
                for (p, s) in pts.iter().zip(&at) {
                    if *s > a && *s < b {
                        f.line_to(*p);
                    }
                }
                f.line_to(point_at(b));
                if let Ok(path) = f.build() {
                    let mut lit = color;
                    lit.a *= strength * (0.85 + 0.15 * hover) * 1.6 / LAYERS as f32;
                    window.paint_path(path, lit);
                }
            }
        }
    }
}
