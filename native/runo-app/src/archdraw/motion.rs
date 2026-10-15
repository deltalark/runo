//! Where moving things are at a moment: dashes and packets along connectors, glows on
//! shapes, and which step of a story is playing. Pure, so the canvas and the exports agree.

use super::geom::{self, Pt};
use super::model::Speed;

/// A marching dash and the gap after it, in diagram units.
pub(crate) const DASH: f32 = 7.;
pub(crate) const GAP: f32 = 9.;
/// Room between packets on a connector.
pub(crate) const PACKET_SPACING: f32 = 140.;
/// How long one step of a story plays, in seconds.
pub(crate) const STEP_SECS: f32 = 2.2;
/// One breath of a pulsing shape or line, in seconds.
pub(crate) const PULSE_SECS: f32 = 1.8;

/// A connector as a polyline, with its curve (if any) flattened.
pub(crate) fn flatten(points: &[Pt], curved: bool) -> Vec<Pt> {
    if curved && points.len() == 4 {
        let (a, b, c, d) = (points[0], points[1], points[2], points[3]);
        (0..=24)
            .map(|i| {
                let t = i as f32 / 24.;
                let u = 1. - t;
                let w = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
                (
                    w[0] * a.0 + w[1] * b.0 + w[2] * c.0 + w[3] * d.0,
                    w[0] * a.1 + w[1] * b.1 + w[2] * c.1 + w[3] * d.1,
                )
            })
            .collect()
    } else {
        points.to_vec()
    }
}

pub(crate) fn length(line: &[Pt]) -> f32 {
    line.windows(2).map(|w| geom::dist(w[0], w[1])).sum()
}

/// The point `s` along the line, clamped to its ends.
pub(crate) fn at(line: &[Pt], s: f32) -> Pt {
    let mut left = s.max(0.);
    for w in line.windows(2) {
        let d = geom::dist(w[0], w[1]);
        if left <= d && d > 0. {
            let t = left / d;
            return (
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            );
        }
        left -= d;
    }
    line.last().copied().unwrap_or((0., 0.))
}

/// The part of the line from `a` to `b` along it.
pub(crate) fn slice(line: &[Pt], a: f32, b: f32) -> Vec<Pt> {
    let mut out = vec![at(line, a)];
    let mut walked = 0.;
    for w in line.windows(2) {
        walked += geom::dist(w[0], w[1]);
        if walked > a && walked < b {
            out.push(w[1]);
        }
    }
    out.push(at(line, b));
    out
}

/// The dashes of a line marching forward over time `t` (seconds), as short polylines.
pub(crate) fn dashes(line: &[Pt], t: f32, speed: Speed) -> Vec<Vec<Pt>> {
    let len = length(line);
    let period = DASH + GAP;
    let offset = (t * speed.rate()) % period;
    let mut out = Vec::new();
    let mut s = offset - period;
    while s < len {
        let (a, b) = (s.max(0.), (s + DASH).min(len));
        if b > a {
            out.push(slice(line, a, b));
        }
        s += period;
    }
    out
}

/// Where packets are along a line of length `len` at time `t`, travelling forward.
pub(crate) fn packets(len: f32, t: f32, speed: Speed) -> Vec<f32> {
    if len <= 0. {
        return Vec::new();
    }
    let count = (len / PACKET_SPACING).ceil().max(1.);
    let spacing = len / count;
    let head = (t * speed.rate()) % spacing;
    (0..count as usize)
        .map(|i| head + i as f32 * spacing)
        .filter(|s| *s <= len)
        .collect()
}

/// How far into its breath a pulse is: 0 at rest, 1 at its brightest.
pub(crate) fn pulse(t: f32) -> f32 {
    0.5 - 0.5 * (t / PULSE_SECS * std::f32::consts::TAU).cos()
}

/// Whether a blinking ring shows at time `t`.
pub(crate) fn blink(t: f32) -> bool {
    (t * 1.6).fract() < 0.55
}

/// The angle of a spinning icon, in radians.
pub(crate) fn spin(t: f32) -> f32 {
    (t * 1.4 * std::f32::consts::TAU) % std::f32::consts::TAU
}

/// The outline of a box grown by `grow`, as a closed polyline starting at its top left.
pub(crate) fn outline(x: f32, y: f32, w: f32, h: f32, grow: f32) -> Vec<Pt> {
    let (l, t, r, b) = (x - grow, y - grow, x + w + grow, y + h + grow);
    vec![(l, t), (r, t), (r, b), (l, b), (l, t)]
}

/// A spinning shape's comet: the stretch of its outline the head and tail cover at `t`.
pub(crate) fn orbit(outline: &[Pt], t: f32) -> (f32, f32) {
    let len = length(outline);
    let head = spin(t) / std::f32::consts::TAU * len;
    (head, (head - len * 0.18).max(0.))
}

/// Which step of a story of `steps` steps plays `t` seconds in, and how far through it
/// (0 to 1). None once it's over.
pub(crate) fn step_at(t: f32, steps: usize) -> Option<(usize, f32)> {
    if t < 0. || steps == 0 {
        return None;
    }
    let i = (t / STEP_SECS) as usize;
    (i < steps).then(|| (i, (t / STEP_SECS).fract()))
}

/// Ease in and out, so a story's packet starts and lands gently.
pub(crate) fn ease(p: f32) -> f32 {
    let p = p.clamp(0., 1.);
    p * p * (3. - 2. * p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_along_a_line() {
        let line = vec![(0., 0.), (10., 0.), (10., 10.)];
        assert_eq!(length(&line), 20.);
        assert_eq!(at(&line, 5.), (5., 0.));
        assert_eq!(at(&line, 15.), (10., 5.));
        assert_eq!(at(&line, 99.), (10., 10.));
        assert_eq!(slice(&line, 5., 15.), vec![(5., 0.), (10., 0.), (10., 5.)]);
    }

    #[test]
    fn dashes_march_and_stay_on_the_line() {
        let line = vec![(0., 0.), (100., 0.)];
        let a = dashes(&line, 0., Speed::Normal);
        let b = dashes(&line, 0.05, Speed::Normal);
        assert!(!a.is_empty());
        assert_ne!(a[0], b[0]);
        for d in a.iter().chain(&b) {
            for p in d {
                assert!(p.0 >= 0. && p.0 <= 100.);
            }
            assert!(length(d) <= DASH + 0.01);
        }
    }

    #[test]
    fn packets_move_forward_and_wrap() {
        let p0 = packets(300., 0., Speed::Fast);
        let p1 = packets(300., 0.2, Speed::Fast);
        assert_eq!(p0.len(), p1.len());
        assert!(p1[0] > p0[0]);
        assert!(packets(300., 100., Speed::Slow).iter().all(|s| *s <= 300.));
        assert!(packets(0., 1., Speed::Slow).is_empty());
    }

    #[test]
    fn curves_flatten_from_end_to_end() {
        let pts = flatten(&[(0., 0.), (0., 50.), (100., 50.), (100., 100.)], true);
        assert_eq!(pts.first(), Some(&(0., 0.)));
        assert_eq!(pts.last(), Some(&(100., 100.)));
        assert_eq!(pts.len(), 25);
    }

    #[test]
    fn stories_play_step_by_step() {
        assert_eq!(step_at(0., 3).map(|s| s.0), Some(0));
        assert_eq!(step_at(STEP_SECS * 1.5, 3).map(|s| s.0), Some(1));
        assert!(step_at(STEP_SECS * 3.1, 3).is_none());
        assert!((pulse(0.)).abs() < 1e-6);
        assert!((pulse(PULSE_SECS / 2.) - 1.).abs() < 1e-4);
        assert_eq!(ease(0.5), 0.5);
    }

    #[test]
    fn a_spinning_comet_stays_on_the_outline() {
        let o = outline(0., 0., 100., 50., 4.);
        assert_eq!(length(&o), 2. * (108. + 58.));
        for i in 0..20 {
            let (head, tail) = orbit(&o, i as f32 * 0.07);
            assert!(tail <= head && head <= length(&o));
        }
    }
}
