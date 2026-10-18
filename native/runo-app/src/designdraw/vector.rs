//! Vector outlines: reading and writing svg path data, turning boxes and ellipses into
//! outlines, and joining, subtracting, intersecting or excluding outlines.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use serde::Deserialize;

pub(crate) type P = [f64; 2];

/// One run of points; closed ones join their last point back to the first.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Contour {
    pub pts: Vec<P>,
    pub closed: bool,
}

/// How shapes combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

impl Op {
    pub(crate) const ALL: [Op; 4] = [Op::Union, Op::Subtract, Op::Intersect, Op::Exclude];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Op::Union => "Union",
            Op::Subtract => "Subtract",
            Op::Intersect => "Intersect",
            Op::Exclude => "Exclude",
        }
    }

    fn rule(self) -> OverlayRule {
        match self {
            Op::Union => OverlayRule::Union,
            Op::Subtract => OverlayRule::Difference,
            Op::Intersect => OverlayRule::Intersect,
            Op::Exclude => OverlayRule::Xor,
        }
    }
}

/// A shape on the page as the page describes it (see `outlines` in the page helper).
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Outline {
    pub kind: String,
    #[serde(default)]
    pub d: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default = "one")]
    pub sx: f64,
    #[serde(default = "one")]
    pub sy: f64,
    #[serde(default)]
    pub radius: f64,
    pub rect: super::dom::Box,
    #[serde(default)]
    pub fill: String,
    #[serde(default)]
    pub stroke: String,
}

fn one() -> f64 {
    1.
}

impl Outline {
    /// Its contours in page units.
    pub(crate) fn contours(&self) -> Vec<Contour> {
        let r = &self.rect;
        let (x, y, w, h) = (r.x as f64, r.y as f64, r.w as f64, r.h as f64);
        match self.kind.as_str() {
            "path" => parse(&self.d)
                .into_iter()
                .map(|c| Contour {
                    pts: c
                        .pts
                        .iter()
                        .map(|p| [self.x + p[0] * self.sx, self.y + p[1] * self.sy])
                        .collect(),
                    closed: c.closed,
                })
                .collect(),
            "ellipse" => vec![ellipse(x + w / 2., y + h / 2., w / 2., h / 2.)],
            _ => vec![rounded_rect(x, y, w, h, self.radius)],
        }
    }
}

fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Contour {
    let n = 64;
    Contour {
        pts: (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                [cx + rx * a.cos(), cy + ry * a.sin()]
            })
            .collect(),
        closed: true,
    }
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> Contour {
    let r = r.min(w / 2.).min(h / 2.).max(0.);
    if r < 0.5 {
        return Contour {
            pts: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            closed: true,
        };
    }
    let mut pts = vec![];
    // Corners clockwise from the top right, each a quarter turn in 8 steps.
    for (cx, cy, from) in [
        (x + w - r, y + r, -90f64),
        (x + w - r, y + h - r, 0.),
        (x + r, y + h - r, 90.),
        (x + r, y + r, 180.),
    ] {
        for i in 0..=8 {
            let a = (from + 90. * i as f64 / 8.).to_radians();
            pts.push([cx + r * a.cos(), cy + r * a.sin()]);
        }
    }
    Contour { pts, closed: true }
}

/// Read svg path data. Curves are followed in short straight steps; arcs are taken as
/// straight lines to their ends.
pub(crate) fn parse(d: &str) -> Vec<Contour> {
    let mut toks: Vec<String> = vec![];
    let mut num = String::new();
    let flush = |num: &mut String, toks: &mut Vec<String>| {
        if !num.is_empty() {
            toks.push(std::mem::take(num));
        }
    };
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() && ch != 'e' && ch != 'E' {
            flush(&mut num, &mut toks);
            toks.push(ch.to_string());
        } else if (ch == '-' && !num.is_empty() && !num.ends_with(['e', 'E']))
            || (ch == '.' && num.contains('.'))
        {
            flush(&mut num, &mut toks);
            num.push(ch);
        } else if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == 'e' || ch == 'E' {
            num.push(ch);
        } else {
            flush(&mut num, &mut toks);
        }
    }
    flush(&mut num, &mut toks);

    let mut out: Vec<Contour> = vec![];
    let mut cur = Contour {
        pts: vec![],
        closed: false,
    };
    let mut at: P = [0., 0.];
    let mut start: P = [0., 0.];
    let mut cmd = 'M';
    let mut i = 0;
    let n = |i: &mut usize| -> Option<f64> {
        let v = toks.get(*i)?.parse().ok()?;
        *i += 1;
        Some(v)
    };
    let end = |cur: &mut Contour, out: &mut Vec<Contour>| {
        if cur.pts.len() > 1 {
            out.push(std::mem::replace(
                cur,
                Contour {
                    pts: vec![],
                    closed: false,
                },
            ));
        } else {
            cur.pts.clear();
            cur.closed = false;
        }
    };
    while i < toks.len() {
        if let Some(c) = toks[i].chars().next().filter(|c| c.is_ascii_alphabetic()) {
            cmd = c;
            i += 1;
            if c == 'Z' || c == 'z' {
                cur.closed = true;
                end(&mut cur, &mut out);
                at = start;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { at } else { [0., 0.] };
        match cmd.to_ascii_uppercase() {
            'M' => {
                let (Some(x), Some(y)) = (n(&mut i), n(&mut i)) else {
                    break;
                };
                end(&mut cur, &mut out);
                at = [base[0] + x, base[1] + y];
                start = at;
                cur.pts.push(at);
                // Pairs after a move are lines.
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' | 'T' => {
                let (Some(x), Some(y)) = (n(&mut i), n(&mut i)) else {
                    break;
                };
                at = [base[0] + x, base[1] + y];
                cur.pts.push(at);
            }
            'H' => {
                let Some(x) = n(&mut i) else { break };
                at = [if rel { at[0] + x } else { x }, at[1]];
                cur.pts.push(at);
            }
            'V' => {
                let Some(y) = n(&mut i) else { break };
                at = [at[0], if rel { at[1] + y } else { y }];
                cur.pts.push(at);
            }
            'C' | 'S' | 'Q' => {
                let count = if cmd.eq_ignore_ascii_case(&'C') { 3 } else { 2 };
                let mut ps = vec![at];
                for _ in 0..count {
                    let (Some(x), Some(y)) = (n(&mut i), n(&mut i)) else {
                        break;
                    };
                    ps.push([base[0] + x, base[1] + y]);
                }
                if ps.len() != count + 1 {
                    break;
                }
                for s in 1..=12 {
                    cur.pts.push(bezier(&ps, s as f64 / 12.));
                }
                at = *ps.last().unwrap_or(&at);
            }
            'A' => {
                let mut v = vec![];
                for _ in 0..7 {
                    match n(&mut i) {
                        Some(x) => v.push(x),
                        None => break,
                    }
                }
                if v.len() != 7 {
                    break;
                }
                at = [base[0] + v[5], base[1] + v[6]];
                cur.pts.push(at);
            }
            _ => i += 1,
        }
    }
    end(&mut cur, &mut out);
    out
}

fn bezier(ps: &[P], t: f64) -> P {
    let mut v = ps.to_vec();
    while v.len() > 1 {
        v = v
            .windows(2)
            .map(|w| {
                [
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                ]
            })
            .collect();
    }
    v[0]
}

/// Svg path data for contours, moved by `off`.
pub(crate) fn to_d(contours: &[Contour], off: P) -> String {
    let r = |v: f64| (v * 10.).round() / 10.;
    let mut d = String::new();
    for c in contours {
        for (i, p) in c.pts.iter().enumerate() {
            if !d.is_empty() {
                d.push(' ');
            }
            d.push_str(&format!(
                "{}{} {}",
                if i == 0 { "M" } else { "L" },
                r(p[0] - off[0]),
                r(p[1] - off[1])
            ));
        }
        if c.closed {
            d.push_str(" Z");
        }
    }
    d
}

/// The box around contours: left, top, right, bottom.
pub(crate) fn bounds(contours: &[Contour]) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in contours.iter().flat_map(|c| &c.pts) {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    (b[0] <= b[2]).then_some(b)
}

/// Combine shapes in order: the first with the second, the result with the third, and so
/// on. Subtract takes every later shape out of the first.
pub(crate) fn combine(op: Op, shapes: &[Vec<Contour>]) -> Vec<Contour> {
    let as_paths = |cs: &Vec<Contour>| -> Vec<Vec<P>> {
        cs.iter()
            .filter(|c| c.pts.len() >= 3)
            .map(|c| c.pts.clone())
            .collect()
    };
    let Some((first, rest)) = shapes.split_first() else {
        return vec![];
    };
    let mut acc: Vec<Vec<Vec<P>>> = vec![as_paths(first)];
    if rest.is_empty() {
        // One shape: just its filled outline.
        acc = as_paths(first).overlay(&Vec::<Vec<P>>::new(), OverlayRule::Union, FillRule::NonZero);
    }
    for next in rest {
        acc = acc.overlay(&as_paths(next), op.rule(), FillRule::NonZero);
    }
    acc.into_iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|pts| Contour { pts, closed: true })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Vec<Contour> {
        vec![rounded_rect(x, y, s, s, 0.)]
    }

    fn area(cs: &[Contour]) -> f64 {
        cs.iter()
            .map(|c| {
                let n = c.pts.len();
                (0..n)
                    .map(|i| {
                        let (a, b) = (c.pts[i], c.pts[(i + 1) % n]);
                        a[0] * b[1] - b[0] * a[1]
                    })
                    .sum::<f64>()
                    / 2.
            })
            .sum::<f64>()
            .abs()
    }

    #[test]
    fn path_data_reads_back_as_written() {
        let cs = parse("M2 2 L22 2 L12 22 Z M0 0 l10 0 h5 v5");
        assert_eq!(cs.len(), 2);
        assert!(cs[0].closed && !cs[1].closed);
        assert_eq!(cs[1].pts, vec![[0., 0.], [10., 0.], [15., 0.], [15., 5.]]);
        assert_eq!(to_d(&cs[..1], [2., 2.]), "M0 0 L20 0 L10 20 Z");
        // Numbers run together, as minifiers write them.
        assert_eq!(parse("M1-2L3.5.5")[0].pts, vec![[1., -2.], [3.5, 0.5]]);
        // A curve ends where it says.
        let c = parse("M0 0 C 0 10 10 10 10 0");
        assert_eq!(c[0].pts.last(), Some(&[10., 0.]));
    }

    #[test]
    fn shapes_combine_like_a_design_tool() {
        let (a, b) = (square(0., 0., 10.), square(5., 0., 10.));
        let shapes = [a, b];
        let near = |x: f64, y: f64| (x - y).abs() < 0.5;
        assert!(near(area(&combine(Op::Union, &shapes)), 150.));
        assert!(near(area(&combine(Op::Subtract, &shapes)), 50.));
        assert!(near(area(&combine(Op::Intersect, &shapes)), 50.));
        assert!(near(area(&combine(Op::Exclude, &shapes)), 100.));
        // A ring: a small circle taken out of a big one.
        let ring = combine(
            Op::Subtract,
            &[
                vec![ellipse(50., 50., 40., 40.)],
                vec![ellipse(50., 50., 20., 20.)],
            ],
        );
        assert_eq!(ring.len(), 2);
        let b = bounds(&ring).unwrap();
        assert!(near(b[0], 10.) && near(b[2], 90.));
    }
}
