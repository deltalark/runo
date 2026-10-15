//! What each kind of element looks like: its outline, extra strokes and default colours.
//! Used both by the canvas and by the SVG and PNG export, so they match.

use super::model::{parse_hex, Kind, Rect, Style};

/// A step of an outline, in diagram space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Cubic curve: two control points, then the end.
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// Paths as SVG `d` text.
pub(crate) fn svg_d(segs: &[Seg]) -> String {
    let mut d = String::new();
    for s in segs {
        match *s {
            Seg::Move(x, y) => d.push_str(&format!("M{x:.1} {y:.1} ")),
            Seg::Line(x, y) => d.push_str(&format!("L{x:.1} {y:.1} ")),
            Seg::Cubic(a, b, c, e, x, y) => {
                d.push_str(&format!("C{a:.1} {b:.1} {c:.1} {e:.1} {x:.1} {y:.1} "))
            }
            Seg::Close => d.push_str("Z "),
        }
    }
    d.trim_end().to_string()
}

const K: f32 = 0.552_284_8; // circle approximation by cubic curves

fn ellipse(out: &mut Vec<Seg>, cx: f32, cy: f32, rx: f32, ry: f32) {
    let (kx, ky) = (rx * K, ry * K);
    out.push(Seg::Move(cx + rx, cy));
    out.push(Seg::Cubic(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry));
    out.push(Seg::Cubic(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy));
    out.push(Seg::Cubic(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry));
    out.push(Seg::Cubic(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy));
    out.push(Seg::Close);
}

fn rounded(out: &mut Vec<Seg>, r: &Rect, radius: f32) {
    let rad = radius.min(r.w / 2.).min(r.h / 2.).max(0.);
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    if rad < 0.5 {
        out.extend([
            Seg::Move(x0, y0),
            Seg::Line(x1, y0),
            Seg::Line(x1, y1),
            Seg::Line(x0, y1),
            Seg::Close,
        ]);
        return;
    }
    let k = rad * (1. - K);
    out.push(Seg::Move(x0 + rad, y0));
    out.push(Seg::Line(x1 - rad, y0));
    out.push(Seg::Cubic(x1 - k, y0, x1, y0 + k, x1, y0 + rad));
    out.push(Seg::Line(x1, y1 - rad));
    out.push(Seg::Cubic(x1, y1 - k, x1 - k, y1, x1 - rad, y1));
    out.push(Seg::Line(x0 + rad, y1));
    out.push(Seg::Cubic(x0 + k, y1, x0, y1 - k, x0, y1 - rad));
    out.push(Seg::Line(x0, y0 + rad));
    out.push(Seg::Cubic(x0, y0 + k, x0 + k, y0, x0 + rad, y0));
    out.push(Seg::Close);
}

/// The default corner radius of a kind.
pub(crate) fn default_radius(kind: Kind) -> f32 {
    match kind {
        Kind::Box | Kind::Note | Kind::Text => 0.,
        Kind::Component => 6.,
        Kind::Boundary => 12.,
        _ => 10.,
    }
}

/// The colour a mark is filled with, taken from the shape's paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ink {
    /// The kind's accent: a container's spine, a person's avatar.
    Accent,
    /// A light wash of the accent: a cylinder's top.
    Tint,
    /// The page colour, for a glyph drawn on an accent mark.
    Paper,
}

/// A filled area drawn over a shape's outline: what tells the kinds apart at a glance.
pub(crate) struct Mark {
    pub segs: Vec<Seg>,
    pub ink: Ink,
}

/// A shape's filled outline, extra lines drawn on top (a cylinder's rim, a note's fold),
/// and filled marks over both.
pub(crate) struct Outline {
    pub fill: Vec<Seg>,
    pub detail: Vec<Seg>,
    pub marks: Vec<Mark>,
    /// Where the text goes.
    pub text: Rect,
}

/// How wide a container's accent spine is.
pub(crate) const SPINE: f32 = 5.;

/// The left `w` of a rounded rectangle, following its corners.
fn left_band(out: &mut Vec<Seg>, r: &Rect, radius: f32, w: f32) {
    let rad = radius.min(r.w / 2.).min(r.h / 2.).max(0.);
    let (x0, y0, y1) = (r.x, r.y, r.bottom());
    // How far a corner pulls the edge in at `dx` from the left.
    let dip = |dx: f32| {
        let off = (rad - dx).max(0.);
        rad - (rad * rad - off * off).max(0.).sqrt()
    };
    const STEPS: usize = 6;
    out.push(Seg::Move(x0, y1 - dip(0.)));
    out.push(Seg::Line(x0, y0 + dip(0.)));
    for i in 1..=STEPS {
        let dx = w * i as f32 / STEPS as f32;
        out.push(Seg::Line(x0 + dx, y0 + dip(dx)));
    }
    for i in (1..=STEPS).rev() {
        let dx = w * i as f32 / STEPS as f32;
        out.push(Seg::Line(x0 + dx, y1 - dip(dx)));
    }
    out.push(Seg::Close);
}

/// A person's avatar: a filled disc with a head-and-shoulders glyph cut out of it.
fn avatar(marks: &mut Vec<Mark>, cx: f32, cy: f32, rad: f32) {
    let mut disc = Vec::new();
    ellipse(&mut disc, cx, cy, rad, rad);
    marks.push(Mark {
        segs: disc,
        ink: Ink::Accent,
    });
    let mut glyph = Vec::new();
    let head = rad * 0.27;
    ellipse(&mut glyph, cx, cy - rad * 0.2, head, head);
    // Shoulders: a dome whose foot follows the disc's lower edge.
    let (sx, sy) = (rad * 0.6, rad * 0.72);
    glyph.push(Seg::Move(cx - sx, cy + sy));
    glyph.push(Seg::Cubic(
        cx - sx,
        cy + rad * 0.2,
        cx - rad * 0.3,
        cy + rad * 0.14,
        cx,
        cy + rad * 0.14,
    ));
    glyph.push(Seg::Cubic(
        cx + rad * 0.3,
        cy + rad * 0.14,
        cx + sx,
        cy + rad * 0.2,
        cx + sx,
        cy + sy,
    ));
    glyph.push(Seg::Cubic(
        cx + sx * 0.55,
        cy + rad * 0.88,
        cx - sx * 0.55,
        cy + rad * 0.88,
        cx - sx,
        cy + sy,
    ));
    glyph.push(Seg::Close);
    marks.push(Mark {
        segs: glyph,
        ink: Ink::Paper,
    });
}

/// The size of a person's avatar in a shape of this size.
pub(crate) fn avatar_radius(r: &Rect) -> f32 {
    (r.h * 0.17).min(r.w * 0.16).clamp(8., 22.)
}

/// A closed polygon through `pts`.
fn poly(out: &mut Vec<Seg>, pts: &[(f32, f32)]) {
    for (i, &(x, y)) in pts.iter().enumerate() {
        out.push(if i == 0 {
            Seg::Move(x, y)
        } else {
            Seg::Line(x, y)
        });
    }
    out.push(Seg::Close);
}

/// A rectangle with its own radius for the top corners and for the bottom ones.
fn rounded2(out: &mut Vec<Seg>, r: &Rect, top: f32, bottom: f32) {
    let cap = (r.w / 2.).min(r.h / 2.);
    let (t, b) = (top.clamp(0., cap), bottom.clamp(0., cap));
    let (kt, kb) = (t * (1. - K), b * (1. - K));
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    out.push(Seg::Move(x0 + t, y0));
    out.push(Seg::Line(x1 - t, y0));
    out.push(Seg::Cubic(x1 - kt, y0, x1, y0 + kt, x1, y0 + t));
    out.push(Seg::Line(x1, y1 - b));
    out.push(Seg::Cubic(x1, y1 - kb, x1 - kb, y1, x1 - b, y1));
    out.push(Seg::Line(x0 + b, y1));
    out.push(Seg::Cubic(x0 + kb, y1, x0, y1 - kb, x0, y1 - b));
    out.push(Seg::Line(x0, y0 + t));
    out.push(Seg::Cubic(x0, y0 + kt, x0 + kt, y0, x0 + t, y0));
    out.push(Seg::Close);
}

/// A filled mark of one ink.
fn mark(marks: &mut Vec<Mark>, ink: Ink, draw: impl FnOnce(&mut Vec<Seg>)) {
    let mut segs = Vec::new();
    draw(&mut segs);
    marks.push(Mark { segs, ink });
}

/// How tall the name under a pictured shape is.
const NAME_BELOW: f32 = 26.;

/// A standing figure, `u` tall, its feet at `base` and centred on `cx`: head, body, arms
/// and legs, each its own mark so they never cut holes in each other.
fn figure(marks: &mut Vec<Mark>, ink: Ink, cx: f32, base: f32, u: f32) {
    let (ox, oy) = (cx - 0.275 * u, base - u);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Rect::new(ox + x0 * u, oy + y0 * u, (x1 - x0) * u, (y1 - y0) * u)
    };
    mark(marks, ink, |o| {
        ellipse(o, ox + 0.275 * u, oy + 0.105 * u, 0.095 * u, 0.095 * u)
    });
    mark(marks, ink, |o| {
        rounded2(o, &rect(0.12, 0.225, 0.43, 0.62), 0.1 * u, 0.03 * u)
    });
    for (x0, x1) in [(0.025, 0.098), (0.452, 0.525)] {
        mark(marks, ink, |o| {
            rounded2(o, &rect(x0, 0.25, x1, 0.58), 0.036 * u, 0.036 * u)
        });
    }
    for (x0, x1) in [(0.128, 0.263), (0.287, 0.422)] {
        mark(marks, ink, |o| {
            rounded2(o, &rect(x0, 0.58, x1, 1.), 0., 0.045 * u)
        });
    }
}

/// Head and shoulders, `size` tall with its foot at `base`.
fn bust(marks: &mut Vec<Mark>, ink: Ink, cx: f32, base: f32, size: f32) {
    let head = size * 0.22;
    let top = base - size;
    mark(marks, ink, |o| ellipse(o, cx, top + head, head, head));
    let (w, h) = (size * 0.9, size * 0.5);
    mark(marks, ink, |o| {
        rounded2(
            o,
            &Rect::new(cx - w / 2., base - h, w, h),
            w * 0.42,
            size * 0.06,
        )
    });
}

/// A cloud filling `r`, flat along the bottom.
fn cloud(out: &mut Vec<Seg>, r: &Rect) {
    // Drawn on a 100 by 64 grid, then stretched to the rectangle.
    let p = |x: f32, y: f32| (r.x + r.w * x / 100., r.y + r.h * y / 64.);
    let c = |a: (f32, f32), b: (f32, f32), e: (f32, f32)| {
        let (a, b, e) = (p(a.0, a.1), p(b.0, b.1), p(e.0, e.1));
        Seg::Cubic(a.0, a.1, b.0, b.1, e.0, e.1)
    };
    let (sx, sy) = p(22., 64.);
    out.push(Seg::Move(sx, sy));
    out.push(c((9., 64.), (0., 56.), (0., 46.)));
    out.push(c((0., 36.), (8., 28.), (19., 28.)));
    out.push(c((19., 14.), (30., 6.), (43., 6.)));
    out.push(c((51., 6.), (57., 9.), (61., 15.)));
    out.push(c((65., 11.), (70., 10.), (74., 10.)));
    out.push(c((84., 10.), (91., 18.), (91., 28.)));
    out.push(c((97., 31.), (100., 38.), (100., 45.)));
    out.push(c((100., 56.), (92., 64.), (81., 64.)));
    out.push(Seg::Close);
}

pub(crate) fn outline(kind: Kind, r: &Rect, style: &Style) -> Outline {
    let radius = style.radius.unwrap_or_else(|| default_radius(kind));
    let mut fill = Vec::new();
    let mut detail = Vec::new();
    let mut marks = Vec::new();
    let mut text = r.inflate(-8.);
    match kind {
        Kind::Ellipse => ellipse(
            &mut fill,
            r.x + r.w / 2.,
            r.y + r.h / 2.,
            r.w / 2.,
            r.h / 2.,
        ),
        Kind::Diamond => {
            let (cx, cy) = r.center();
            fill.extend([
                Seg::Move(cx, r.y),
                Seg::Line(r.right(), cy),
                Seg::Line(cx, r.bottom()),
                Seg::Line(r.x, cy),
                Seg::Close,
            ]);
            text = Rect::new(r.x + r.w * 0.2, r.y + r.h * 0.2, r.w * 0.6, r.h * 0.6);
        }
        Kind::Database => {
            let ry = (r.h * 0.12).clamp(6., 18.);
            let (x0, x1, y0, y1) = (r.x, r.right(), r.y + ry, r.bottom() - ry);
            let (cx, rx) = (r.x + r.w / 2., r.w / 2.);
            let (kx, ky) = (rx * K, ry * K);
            fill.push(Seg::Move(x0, y0));
            fill.push(Seg::Cubic(x0, y0 - ky, cx - kx, y0 - ry, cx, y0 - ry));
            fill.push(Seg::Cubic(cx + kx, y0 - ry, x1, y0 - ky, x1, y0));
            fill.push(Seg::Line(x1, y1));
            fill.push(Seg::Cubic(x1, y1 + ky, cx + kx, y1 + ry, cx, y1 + ry));
            fill.push(Seg::Cubic(cx - kx, y1 + ry, x0, y1 + ky, x0, y1));
            fill.push(Seg::Close);
            // The top, washed with the accent, and its near rim.
            let mut top = Vec::new();
            ellipse(&mut top, cx, y0, rx, ry);
            marks.push(Mark {
                segs: top,
                ink: Ink::Tint,
            });
            detail.push(Seg::Move(x0, y0));
            detail.push(Seg::Cubic(x0, y0 + ky, cx - kx, y0 + ry, cx, y0 + ry));
            detail.push(Seg::Cubic(cx + kx, y0 + ry, x1, y0 + ky, x1, y0));
            detail.push(Seg::Cubic(x1, y0 - ky, cx + kx, y0 - ry, cx, y0 - ry));
            detail.push(Seg::Cubic(cx - kx, y0 - ry, x0, y0 - ky, x0, y0));
            text = Rect::new(r.x + 8., r.y + ry * 2. + 2., r.w - 16., r.h - ry * 3. - 4.);
        }
        Kind::Queue => {
            let rx = (r.w * 0.08).clamp(6., 18.);
            let (x0, x1, y0, y1) = (r.x + rx, r.right() - rx, r.y, r.bottom());
            let (cy, ry) = (r.y + r.h / 2., r.h / 2.);
            let (kx, ky) = (rx * K, ry * K);
            fill.push(Seg::Move(x0, y0));
            fill.push(Seg::Line(x1, y0));
            fill.push(Seg::Cubic(x1 + kx, y0, x1 + rx, cy - ky, x1 + rx, cy));
            fill.push(Seg::Cubic(x1 + rx, cy + ky, x1 + kx, y1, x1, y1));
            fill.push(Seg::Line(x0, y1));
            fill.push(Seg::Cubic(x0 - kx, y1, x0 - rx, cy + ky, x0 - rx, cy));
            fill.push(Seg::Cubic(x0 - rx, cy - ky, x0 - kx, y0, x0, y0));
            fill.push(Seg::Close);
            // The open right end, washed with the accent, and its near rim.
            let mut end = Vec::new();
            ellipse(&mut end, x1, cy, rx, ry);
            marks.push(Mark {
                segs: end,
                ink: Ink::Tint,
            });
            detail.push(Seg::Move(x1, y0));
            detail.push(Seg::Cubic(x1 - kx, y0, x1 - rx, cy - ky, x1 - rx, cy));
            detail.push(Seg::Cubic(x1 - rx, cy + ky, x1 - kx, y1, x1, y1));
            text = Rect::new(r.x + rx + 6., r.y + 6., r.w - rx * 3. - 12., r.h - 12.);
        }
        Kind::Person => {
            // A card with an avatar at the top and the name under it.
            rounded(&mut fill, r, radius.max(14.));
            let rad = avatar_radius(r);
            let cy = r.y + 14. + rad;
            avatar(&mut marks, r.x + r.w / 2., cy, rad);
            let top = cy + rad + 4.;
            text = Rect::new(r.x + 8., top, r.w - 16., (r.bottom() - 6. - top).max(1.));
        }
        Kind::Container | Kind::External => {
            rounded(&mut fill, r, radius);
            let mut spine = Vec::new();
            left_band(&mut spine, r, radius, SPINE);
            marks.push(Mark {
                segs: spine,
                ink: Ink::Accent,
            });
            text = Rect::new(r.x + SPINE + 8., r.y + 8., r.w - SPINE - 16., r.h - 16.);
        }
        Kind::Component => {
            // The UML component's two tabs across the left edge.
            rounded(&mut fill, r, radius);
            let (tw, th) = (14_f32.min(r.w * 0.12), 7_f32.min(r.h * 0.14));
            for f in [0.28, 0.56] {
                let y = r.y + r.h * f;
                let mut tab = Vec::new();
                rounded(&mut tab, &Rect::new(r.x - tw / 2., y, tw, th), 2.);
                marks.push(Mark {
                    segs: tab,
                    ink: Ink::Accent,
                });
            }
            text = Rect::new(r.x + tw / 2. + 8., r.y + 8., r.w - tw / 2. - 16., r.h - 16.);
        }
        Kind::Cloud => {
            cloud(&mut fill, r);
            text = Rect::new(r.x + r.w * 0.16, r.y + r.h * 0.3, r.w * 0.7, r.h * 0.62);
        }
        Kind::Note => {
            let f = (r.w.min(r.h) * 0.16).clamp(8., 20.);
            fill.extend([
                Seg::Move(r.x, r.y),
                Seg::Line(r.right() - f, r.y),
                Seg::Line(r.right(), r.y + f),
                Seg::Line(r.right(), r.bottom()),
                Seg::Line(r.x, r.bottom()),
                Seg::Close,
            ]);
            detail.extend([
                Seg::Move(r.right() - f, r.y),
                Seg::Line(r.right() - f, r.y + f),
                Seg::Line(r.right(), r.y + f),
            ]);
            text = r.inflate(-10.);
        }
        Kind::Text => {
            text = *r;
        }
        Kind::Icon => {
            // The picture fills the top; the name sits under it. The outline isn't painted
            // but is what a click lands on.
            rounded(&mut fill, r, 6.);
            text = Rect::new(r.x - 30., r.bottom() - 24., r.w + 60., 24.);
        }
        Kind::Actor => {
            // The figure fills the top; the name sits under it. The outline isn't painted
            // but is what a click lands on.
            rounded(&mut fill, r, 6.);
            let room = (r.h - NAME_BELOW).max(10.);
            let u = room.min(r.w / 0.55);
            figure(&mut marks, Ink::Accent, r.x + r.w / 2., r.y + room, u);
            text = Rect::new(r.x - 30., r.bottom() - NAME_BELOW, r.w + 60., NAME_BELOW);
        }
        Kind::Users => {
            rounded(&mut fill, r, 6.);
            let room = (r.h - NAME_BELOW).max(10.);
            let size = room.min(r.w / 1.75);
            let (cx, base) = (r.x + r.w / 2., r.y + room);
            let back = size * 0.8;
            for dx in [-0.48, 0.48] {
                bust(
                    &mut marks,
                    Ink::Tint,
                    cx + dx * size,
                    base - size * 0.1,
                    back,
                );
            }
            // A ring of page colour sets the front one off from the two behind.
            let ring = size * 0.06;
            let head = size * 0.22;
            mark(&mut marks, Ink::Paper, |o| {
                ellipse(o, cx, base - size + head, head + ring, head + ring)
            });
            let (w, h) = (size * 0.9 + ring * 2., size * 0.5 + ring);
            mark(&mut marks, Ink::Paper, |o| {
                rounded2(
                    o,
                    &Rect::new(cx - w / 2., base - h, w, h),
                    w * 0.42,
                    size * 0.06,
                )
            });
            bust(&mut marks, Ink::Accent, cx, base, size);
            text = Rect::new(r.x - 30., r.bottom() - NAME_BELOW, r.w + 60., NAME_BELOW);
        }
        Kind::Browser => {
            let rad = radius.max(8.);
            rounded(&mut fill, r, rad);
            let bar = (r.h * 0.16).clamp(14., 22.);
            mark(&mut marks, Ink::Tint, |o| {
                rounded2(o, &Rect::new(r.x, r.y, r.w, bar), rad, 0.)
            });
            detail.extend([Seg::Move(r.x, r.y + bar), Seg::Line(r.right(), r.y + bar)]);
            let dot = (bar * 0.13).max(2.);
            for i in 0..3 {
                let x = r.x + 10. + i as f32 * dot * 3.4;
                mark(&mut marks, Ink::Accent, |o| {
                    ellipse(o, x, r.y + bar / 2., dot, dot)
                });
            }
            let from = r.x + 16. + dot * 9.;
            if r.right() - 12. - from > 20. {
                mark(&mut marks, Ink::Paper, |o| {
                    rounded(
                        o,
                        &Rect::new(from, r.y + bar * 0.25, r.right() - 12. - from, bar * 0.5),
                        bar * 0.25,
                    )
                });
            }
            text = Rect::new(r.x + 10., r.y + bar + 6., r.w - 20., r.h - bar - 12.);
        }
        Kind::Phone => {
            let rad = (r.w.min(r.h) * 0.16).clamp(10., 28.);
            rounded(&mut fill, r, rad);
            let notch = (r.w * 0.3).min(40.);
            mark(&mut marks, Ink::Accent, |o| {
                rounded(
                    o,
                    &Rect::new(r.x + (r.w - notch) / 2., r.y + 8., notch, 6.),
                    3.,
                )
            });
            let bar = (r.w * 0.34).min(46.);
            mark(&mut marks, Ink::Tint, |o| {
                rounded(
                    o,
                    &Rect::new(r.x + (r.w - bar) / 2., r.bottom() - 10., bar, 4.),
                    2.,
                )
            });
            text = Rect::new(r.x + 8., r.y + 20., r.w - 16., r.h - 38.);
        }
        Kind::Desktop => {
            let screen = Rect::new(r.x, r.y, r.w, r.h * 0.78);
            rounded(&mut fill, &screen, radius.max(8.));
            let cx = r.x + r.w / 2.;
            let neck = (r.w * 0.1).max(8.);
            let foot = (r.h * 0.06).max(4.);
            mark(&mut marks, Ink::Accent, |o| {
                poly(
                    o,
                    &[
                        (cx - neck / 2., screen.bottom()),
                        (cx + neck / 2., screen.bottom()),
                        (cx + neck * 0.7, r.bottom() - foot),
                        (cx - neck * 0.7, r.bottom() - foot),
                    ],
                )
            });
            let base = (r.w * 0.42).max(30.);
            mark(&mut marks, Ink::Accent, |o| {
                rounded(
                    o,
                    &Rect::new(cx - base / 2., r.bottom() - foot, base, foot),
                    foot / 2.,
                )
            });
            text = screen.inflate(-10.);
        }
        Kind::Server => {
            rounded(&mut fill, r, radius.max(8.));
            let unit = (r.h * 0.13).clamp(10., 18.);
            let mut y = r.y + 10.;
            for _ in 0..3 {
                if y + unit > r.bottom() - 30. {
                    break;
                }
                let slot = Rect::new(r.x + 10., y, r.w - 20., unit);
                mark(&mut marks, Ink::Tint, |o| rounded(o, &slot, 3.));
                let led = (unit * 0.16).max(1.8);
                mark(&mut marks, Ink::Accent, |o| {
                    ellipse(o, slot.right() - 8., y + unit / 2., led, led)
                });
                mark(&mut marks, Ink::Accent, |o| {
                    ellipse(o, slot.right() - 8. - led * 3.4, y + unit / 2., led, led)
                });
                y += unit + 5.;
            }
            text = Rect::new(r.x + 8., y + 2., r.w - 16., (r.bottom() - y - 8.).max(1.));
        }
        Kind::Document => {
            let a = (r.h * 0.08).clamp(4., 10.);
            let (x0, y0, x1) = (r.x, r.y, r.right());
            let yb = r.bottom() - a;
            fill.extend([
                Seg::Move(x0, y0),
                Seg::Line(x1, y0),
                Seg::Line(x1, yb),
                Seg::Cubic(
                    x0 + r.w * 0.66,
                    yb - a * 2.,
                    x0 + r.w * 0.33,
                    yb + a * 2.,
                    x0,
                    yb,
                ),
                Seg::Close,
            ]);
            text = Rect::new(r.x + 8., r.y + 8., r.w - 16., r.h - 16. - a);
        }
        Kind::Folder => {
            let tab = (r.h * 0.14).clamp(8., 16.);
            let tw = r.w * 0.38;
            let rad = 6_f32.min(tab);
            let k = rad * (1. - K);
            let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
            fill.extend([
                Seg::Move(x0 + rad, y0),
                Seg::Line(x0 + tw, y0),
                Seg::Line(x0 + tw + tab, y0 + tab),
                Seg::Line(x1 - rad, y0 + tab),
                Seg::Cubic(x1 - k, y0 + tab, x1, y0 + tab + k, x1, y0 + tab + rad),
                Seg::Line(x1, y1 - rad),
                Seg::Cubic(x1, y1 - k, x1 - k, y1, x1 - rad, y1),
                Seg::Line(x0 + rad, y1),
                Seg::Cubic(x0 + k, y1, x0, y1 - k, x0, y1 - rad),
                Seg::Line(x0, y0 + rad),
                Seg::Cubic(x0, y0 + k, x0 + k, y0, x0 + rad, y0),
                Seg::Close,
            ]);
            detail.extend([Seg::Move(x0, y0 + tab), Seg::Line(x0 + tw + tab, y0 + tab)]);
            text = Rect::new(r.x + 8., r.y + tab + 6., r.w - 16., r.h - tab - 12.);
        }
        Kind::Hexagon => {
            let i = (r.h * 0.3).min(r.w * 0.2);
            let cy = r.y + r.h / 2.;
            poly(
                &mut fill,
                &[
                    (r.x + i, r.y),
                    (r.right() - i, r.y),
                    (r.right(), cy),
                    (r.right() - i, r.bottom()),
                    (r.x + i, r.bottom()),
                    (r.x, cy),
                ],
            );
            text = Rect::new(r.x + i, r.y + 6., r.w - i * 2., r.h - 12.);
        }
        Kind::Triangle => {
            poly(
                &mut fill,
                &[
                    (r.x + r.w / 2., r.y),
                    (r.right(), r.bottom()),
                    (r.x, r.bottom()),
                ],
            );
            text = Rect::new(r.x + r.w * 0.22, r.y + r.h * 0.45, r.w * 0.56, r.h * 0.5);
        }
        Kind::Parallelogram => {
            let s = (r.h * 0.35).min(r.w * 0.2);
            poly(
                &mut fill,
                &[
                    (r.x + s, r.y),
                    (r.right(), r.y),
                    (r.right() - s, r.bottom()),
                    (r.x, r.bottom()),
                ],
            );
            text = Rect::new(r.x + s, r.y + 6., r.w - s * 2., r.h - 12.);
        }
        Kind::Terminator => {
            rounded(&mut fill, r, r.h / 2.);
            text = Rect::new(r.x + r.h / 3., r.y + 6., r.w - r.h * 2. / 3., r.h - 12.);
        }
        Kind::Shield => {
            let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
            let (w, h, cx) = (r.w, r.h, r.x + r.w / 2.);
            fill.extend([
                Seg::Move(x0, y0 + h * 0.13),
                Seg::Cubic(
                    x0 + w * 0.22,
                    y0 + h * 0.12,
                    cx - w * 0.14,
                    y0 + h * 0.06,
                    cx,
                    y0,
                ),
                Seg::Cubic(
                    cx + w * 0.14,
                    y0 + h * 0.06,
                    x1 - w * 0.22,
                    y0 + h * 0.12,
                    x1,
                    y0 + h * 0.13,
                ),
                Seg::Line(x1, y0 + h * 0.46),
                Seg::Cubic(x1, y0 + h * 0.74, cx + w * 0.22, y0 + h * 0.9, cx, y1),
                Seg::Cubic(
                    cx - w * 0.22,
                    y0 + h * 0.9,
                    x0,
                    y0 + h * 0.74,
                    x0,
                    y0 + h * 0.46,
                ),
                Seg::Close,
            ]);
            text = Rect::new(x0 + w * 0.12, y0 + h * 0.18, w * 0.76, h * 0.46);
        }
        Kind::Callout => {
            let body = r.h * 0.8;
            let rad = radius.max(10.).min(body / 2.).min(r.w / 2.);
            let k = rad * (1. - K);
            let (x0, y0, x1, yb) = (r.x, r.y, r.right(), r.y + body);
            fill.extend([
                Seg::Move(x0 + rad, y0),
                Seg::Line(x1 - rad, y0),
                Seg::Cubic(x1 - k, y0, x1, y0 + k, x1, y0 + rad),
                Seg::Line(x1, yb - rad),
                Seg::Cubic(x1, yb - k, x1 - k, yb, x1 - rad, yb),
                Seg::Line(x0 + r.w * 0.34, yb),
                Seg::Line(x0 + r.w * 0.16, r.bottom()),
                Seg::Line(x0 + r.w * 0.2, yb),
                Seg::Line(x0 + rad, yb),
                Seg::Cubic(x0 + k, yb, x0, yb - k, x0, yb - rad),
                Seg::Line(x0, y0 + rad),
                Seg::Cubic(x0, y0 + k, x0 + k, y0, x0 + rad, y0),
                Seg::Close,
            ]);
            text = Rect::new(r.x + 10., r.y + 8., r.w - 20., body - 16.);
        }
        Kind::Arrow => {
            let head = (r.h * 0.6).min(r.w * 0.35);
            let (cy, shaft) = (r.y + r.h / 2., r.h * 0.56);
            let neck = r.right() - head;
            poly(
                &mut fill,
                &[
                    (r.x, cy - shaft / 2.),
                    (neck, cy - shaft / 2.),
                    (neck, r.y),
                    (r.right(), cy),
                    (neck, r.bottom()),
                    (neck, cy + shaft / 2.),
                    (r.x, cy + shaft / 2.),
                ],
            );
            text = Rect::new(r.x + 6., cy - shaft / 2., neck - r.x - 6., shaft);
        }
        Kind::Boundary => {
            rounded(&mut fill, r, radius);
            text = Rect::new(r.x + 12., r.y + 8., r.w - 24., 22.);
        }
        _ => rounded(&mut fill, r, radius),
    }
    Outline {
        fill,
        detail,
        marks,
        text,
    }
}

/// The accent colour of a kind: its stroke, and the hue of its fill.
pub(crate) fn accent(kind: Kind) -> u32 {
    match kind {
        Kind::Person => 0x2563eb,
        Kind::System => 0x1d4ed8,
        Kind::Container => 0x3b82f6,
        Kind::Component => 0x0ea5e9,
        Kind::Database => 0x0d9488,
        Kind::Queue => 0x7c3aed,
        Kind::External => 0x6b7280,
        Kind::Boundary => 0x64748b,
        Kind::Note => 0xca8a04,
        Kind::Text => 0x64748b,
        Kind::Cloud => 0x0284c7,
        Kind::Actor | Kind::Users => 0x2563eb,
        Kind::Browser | Kind::Phone | Kind::Desktop => 0x4f46e5,
        Kind::Server => 0x0891b2,
        Kind::Folder => 0xd97706,
        Kind::Shield => 0x16a34a,
        Kind::Callout => 0x0ea5e9,
        _ => 0x64748b,
    }
}

/// Mix two colours: `t` of `a`, the rest `b`.
pub(crate) fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as f32;
    let m = |s: u32| ((ch(a, s) * t + ch(b, s) * (1. - t)).round() as u32).min(255) << s;
    m(16) | m(8) | m(0)
}

/// The theme colours a diagram is drawn against.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Theme {
    pub canvas: u32,
    pub text: u32,
    pub muted: u32,
    pub dark: bool,
}

impl Theme {
    pub(crate) const LIGHT: Theme = Theme {
        canvas: 0xffffff,
        text: 0x111827,
        muted: 0x6b7280,
        dark: false,
    };
}

/// Final colours of a shape: fill (None for no fill), stroke, text, and opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Paint {
    pub fill: Option<u32>,
    pub stroke: Option<u32>,
    pub text: u32,
    pub sub: u32,
    pub opacity: f32,
    pub dashed: bool,
    pub font: f32,
    pub bold: bool,
    /// What marks are filled with: see [`Ink`].
    pub accent: u32,
    pub tint: u32,
    pub paper: u32,
    /// A soft shadow under the shape lifts it off the page.
    pub shadow: bool,
}

impl Paint {
    pub(crate) fn ink(&self, ink: Ink) -> u32 {
        match ink {
            Ink::Accent => self.accent,
            Ink::Tint => self.tint,
            Ink::Paper => self.paper,
        }
    }
}

/// How far a shape's shadow drops, and how dark it is.
pub(crate) const SHADOW_DROP: f32 = 2.;
pub(crate) const SHADOW_ALPHA: f32 = 0.08;

pub(crate) fn paint(kind: Kind, style: &Style, theme: &Theme) -> Paint {
    let acc = accent(kind);
    let fill_default = match kind {
        Kind::Text | Kind::Icon | Kind::Actor | Kind::Users => None,
        Kind::Note => Some(if theme.dark { 0x4a3f12 } else { 0xfef3c7 }),
        Kind::Boundary => Some(mix(acc, theme.canvas, if theme.dark { 0.08 } else { 0.05 })),
        // The system in focus stands out solid, as C4 draws it.
        Kind::System => Some(mix(acc, theme.canvas, if theme.dark { 0.55 } else { 0.9 })),
        // Cards: the page, barely washed with the kind's colour.
        Kind::Person
        | Kind::Container
        | Kind::Component
        | Kind::External
        | Kind::Browser
        | Kind::Phone
        | Kind::Desktop
        | Kind::Server => Some(mix(acc, theme.canvas, if theme.dark { 0.14 } else { 0.04 })),
        _ => Some(mix(acc, theme.canvas, if theme.dark { 0.2 } else { 0.08 })),
    };
    let strong = kind == Kind::System && style.fill.is_none();
    let text_default = if strong && !theme.dark {
        0xffffff
    } else if kind == Kind::Note && !theme.dark {
        0x422006
    } else {
        theme.text
    };
    let fill = match style.fill.as_deref() {
        Some("none") => None,
        Some(f) => parse_hex(f).or(fill_default),
        None => fill_default,
    };
    let stroke = match style.stroke.as_deref() {
        Some("none") => None,
        Some(s) => parse_hex(s).or(Some(acc)),
        None if matches!(kind, Kind::Text | Kind::Icon | Kind::Actor | Kind::Users) => None,
        // A card's edge is quiet; its spine, tabs or avatar carry the colour.
        None if matches!(
            kind,
            Kind::Person
                | Kind::Container
                | Kind::Component
                | Kind::External
                | Kind::Browser
                | Kind::Phone
                | Kind::Desktop
                | Kind::Server
        ) =>
        {
            Some(mix(acc, theme.canvas, if theme.dark { 0.6 } else { 0.5 }))
        }
        None => Some(acc),
    };
    // Marks follow a stroke the user picked, else the kind.
    let mark = style.stroke.as_deref().and_then(parse_hex).unwrap_or(acc);
    let text = style
        .text
        .as_deref()
        .and_then(parse_hex)
        .unwrap_or(text_default);
    Paint {
        fill,
        stroke,
        text,
        sub: mix(text, fill.unwrap_or(theme.canvas), 0.7),
        opacity: style.opacity.unwrap_or(1.).clamp(0.05, 1.),
        dashed: style.dashed || kind == Kind::Boundary,
        font: style.font.unwrap_or(match kind {
            Kind::Text => 16.,
            Kind::Boundary => 13.,
            _ => 14.,
        }),
        bold: style.bold || kind == Kind::Text && style.font.is_some_and(|f| f >= 20.),
        accent: mark,
        tint: mix(
            mark,
            fill.unwrap_or(theme.canvas),
            if theme.dark { 0.4 } else { 0.22 },
        ),
        paper: if theme.dark { theme.canvas } else { 0xffffff },
        shadow: fill.is_some()
            && !matches!(kind, Kind::Text | Kind::Icon | Kind::Boundary)
            && style.opacity.is_none_or(|o| o > 0.6),
    }
}

/// Swatches offered in the style panel.
pub(crate) const SWATCHES: [u32; 12] = [
    0xffffff, 0xf1f5f9, 0xfef3c7, 0xfee2e2, 0xdcfce7, 0xdbeafe, 0x3b82f6, 0x0d9488, 0x7c3aed,
    0xdc2626, 0xf59e0b, 0x111827,
];

/// Where a shape's picture goes: all of an icon above its name, a badge in the corner of
/// anything else.
pub(crate) fn picture_rect(kind: Kind, r: &Rect) -> Rect {
    match kind {
        Kind::Icon => Rect::new(r.x + 10., r.y + 6., r.w - 20., (r.h - 36.).max(8.)),
        Kind::Boundary => Rect::new(r.x + 10., r.y + 8., 18., 18.),
        _ => {
            let s = (r.h * 0.3).clamp(16., 26.);
            let x = match kind {
                Kind::Container | Kind::External => r.x + SPINE + 7.,
                Kind::Component => r.x + 13.,
                _ => r.x + 7.,
            };
            Rect::new(x, r.y + 7., s, s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_an_outline_or_is_text() {
        let r = Rect::new(0., 0., 200., 100.);
        for kind in Kind::ALL {
            let o = outline(kind, &r, &Style::default());
            assert_eq!(o.fill.is_empty(), kind == Kind::Text, "{kind:?}");
            // A name under a picture may be wider than it.
            assert!(
                kind.name_below() || r.inflate(1.).contains_rect(&o.text),
                "{kind:?} text stays inside"
            );
        }
    }

    #[test]
    fn styles_override_kind_defaults() {
        let base = paint(Kind::Database, &Style::default(), &Theme::LIGHT);
        assert_eq!(base.stroke, Some(accent(Kind::Database)));
        let custom = paint(
            Kind::Database,
            &Style {
                fill: Some("#ff0000".into()),
                stroke: Some("none".into()),
                dashed: true,
                ..Default::default()
            },
            &Theme::LIGHT,
        );
        assert_eq!(custom.fill, Some(0xff0000));
        assert_eq!(custom.stroke, None);
        assert!(custom.dashed);
        assert!(paint(Kind::Boundary, &Style::default(), &Theme::LIGHT).dashed);
    }

    #[test]
    fn svg_paths() {
        let mut segs = Vec::new();
        rounded(&mut segs, &Rect::new(0., 0., 10., 10.), 0.);
        assert_eq!(svg_d(&segs), "M0.0 0.0 L10.0 0.0 L10.0 10.0 L0.0 10.0 Z");
        assert_eq!(mix(0xffffff, 0x000000, 0.5), 0x808080);
    }
}
