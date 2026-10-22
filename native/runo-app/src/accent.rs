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

/// An outline traced as points, with the corners its rim is lit at. Start it with `new`,
/// trace it with `line` and `corner`, and paint it with `paint`.
pub struct Rim {
    pts: Vec<Point<Pixels>>,
    lit: Vec<usize>,
    closed: bool,
}

impl Rim {
    pub fn new(from: Point<Pixels>) -> Self {
        Self {
            pts: vec![from],
            lit: Vec::new(),
            closed: false,
        }
    }

    pub fn line(&mut self, to: Point<Pixels>) {
        self.pts.push(to);
    }

    /// A quarter turn from where the outline is to `to` round the square corner `c`, lit
    /// at its middle when `lit`.
    pub fn corner(&mut self, c: Point<Pixels>, to: Point<Pixels>, lit: bool) {
        let k = 0.5523;
        let from = *self.pts.last().unwrap();
        let (c1, c2) = (from + (c - from) * k, to + (c - to) * k);
        const STEPS: usize = 8;
        for n in 1..=STEPS {
            let t = n as f32 / STEPS as f32;
            let u = 1. - t;
            let p = from * (u * u * u)
                + c1 * (3. * u * u * t)
                + c2 * (3. * u * t * t)
                + to * (t * t * t);
            self.pts.push(p);
            if lit && n == STEPS / 2 {
                self.lit.push(self.pts.len() - 1);
            }
        }
    }

    /// Joins the outline's end back to its start.
    pub fn close(mut self) -> Self {
        self.closed = true;
        self
    }

    /// Strokes the outline 2px wide in the accent over a faint 6px glow, `hover` of the way
    /// to its hovered look: full strength at each lit corner, fading to nothing `fade`
    /// along the outline from it, in short runs of one strength each so the fade is smooth.
    pub fn paint(mut self, hover: f32, window: &mut Window) {
        if self.closed {
            self.pts.push(self.pts[0]);
        }
        let len = |a: Point<Pixels>, b: Point<Pixels>| {
            let d = b - a;
            (f32::from(d.x).powi(2) + f32::from(d.y).powi(2)).sqrt()
        };
        let mut along = vec![0.];
        for w in self.pts.windows(2) {
            along.push(along.last().unwrap() + len(w[0], w[1]));
        }
        let total = *along.last().unwrap();
        let lit: Vec<f32> = self.lit.iter().map(|&i| along[i]).collect();
        let fade = 34. + 18. * hover;
        let strength = |s: f32| {
            let d = lit
                .iter()
                .map(|&l| {
                    let d = (s - l).abs();
                    if self.closed {
                        d.min(total - d)
                    } else {
                        d
                    }
                })
                .fold(f32::MAX, f32::min);
            (1. - d / fade).max(0.).powf(1.25)
        };
        let color = color(hover);
        // A soft glow first, then the line itself. Each is laid in pieces of at most 2px,
        // each at the strength of its middle, merged into runs that share a step of it.
        const LEVELS: f32 = 24.;
        let flush = |run: &mut Option<(f32, PathBuilder)>, window: &mut Window, glow: bool| {
            if let Some((level, f)) = run.take() {
                if let Ok(path) = f.build() {
                    let mut c = color;
                    c.a *= level / LEVELS * if glow { 0.22 } else { 1. };
                    window.paint_path(path, c);
                }
            }
        };
        for glow in [true, false] {
            let mut run: Option<(f32, PathBuilder)> = None;
            for (i, w) in self.pts.windows(2).enumerate() {
                let (a, b, s0) = (w[0], w[1], along[i]);
                let l = len(a, b);
                let pieces = (l / 2.).ceil().max(1.) as usize;
                for n in 0..pieces {
                    let (t0, t1) = (n as f32 / pieces as f32, (n + 1) as f32 / pieces as f32);
                    let (p0, p1) = (a + (b - a) * t0, a + (b - a) * t1);
                    let level = (strength(s0 + l * (t0 + t1) / 2.) * LEVELS).round();
                    if level == 0. {
                        flush(&mut run, window, glow);
                        continue;
                    }
                    match &mut run {
                        Some((lv, f)) if *lv == level => f.line_to(p1),
                        _ => {
                            flush(&mut run, window, glow);
                            let mut f = PathBuilder::stroke(px(if glow { 6. } else { 2. }));
                            f.move_to(p0);
                            f.line_to(p1);
                            run = Some((level, f));
                        }
                    }
                }
            }
            flush(&mut run, window, glow);
        }
    }
}
