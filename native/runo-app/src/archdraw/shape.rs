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

/// A shape's filled outline, and extra lines drawn on top (a cylinder's rim, a note's fold).
pub(crate) struct Outline {
    pub fill: Vec<Seg>,
    pub detail: Vec<Seg>,
    /// Where the text goes.
    pub text: Rect,
}

pub(crate) fn outline(kind: Kind, r: &Rect, style: &Style) -> Outline {
    let radius = style.radius.unwrap_or_else(|| default_radius(kind));
    let mut fill = Vec::new();
    let mut detail = Vec::new();
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
            // The near rim of the top.
            detail.push(Seg::Move(x0, y0));
            detail.push(Seg::Cubic(x0, y0 + ky, cx - kx, y0 + ry, cx, y0 + ry));
            detail.push(Seg::Cubic(cx + kx, y0 + ry, x1, y0 + ky, x1, y0));
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
            // The near rim of the right end.
            detail.push(Seg::Move(x1, y0));
            detail.push(Seg::Cubic(x1 - kx, y0, x1 - rx, cy - ky, x1 - rx, cy));
            detail.push(Seg::Cubic(x1 - rx, cy + ky, x1 - kx, y1, x1, y1));
            text = Rect::new(r.x + rx + 6., r.y + 6., r.w - rx * 3. - 12., r.h - 12.);
        }
        Kind::Person => {
            let head = (r.h * 0.34).min(r.w * 0.4);
            let body_top = r.y + head * 0.8;
            ellipse(
                &mut fill,
                r.x + r.w / 2.,
                r.y + head / 2.,
                head / 2.,
                head / 2.,
            );
            rounded(
                &mut fill,
                &Rect::new(r.x, body_top, r.w, r.bottom() - body_top),
                radius.max(head * 0.45),
            );
            text = Rect::new(
                r.x + 8.,
                body_top + head * 0.2 + 4.,
                r.w - 16.,
                r.bottom() - body_top - head * 0.2 - 8.,
            );
        }
        Kind::Cloud => {
            rounded(&mut fill, r, r.h / 2.);
            text = Rect::new(r.x + r.h * 0.3, r.y + 6., r.w - r.h * 0.6, r.h - 12.);
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
        Kind::Boundary => {
            rounded(&mut fill, r, radius);
            text = Rect::new(r.x + 12., r.y + 8., r.w - 24., 22.);
        }
        _ => rounded(&mut fill, r, radius),
    }
    Outline { fill, detail, text }
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
}

pub(crate) fn paint(kind: Kind, style: &Style, theme: &Theme) -> Paint {
    let acc = accent(kind);
    let fill_default = match kind {
        Kind::Text | Kind::Icon => None,
        Kind::Note => Some(if theme.dark { 0x4a3f12 } else { 0xfef3c7 }),
        Kind::Boundary => Some(mix(acc, theme.canvas, if theme.dark { 0.08 } else { 0.05 })),
        Kind::Person | Kind::System => {
            Some(mix(acc, theme.canvas, if theme.dark { 0.45 } else { 0.85 }))
        }
        _ => Some(mix(acc, theme.canvas, if theme.dark { 0.22 } else { 0.10 })),
    };
    let strong = matches!(kind, Kind::Person | Kind::System) && style.fill.is_none();
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
        None if kind == Kind::Text || kind == Kind::Icon => None,
        None => Some(acc),
    };
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
    }
}

/// Swatches offered in the style panel.
pub(crate) const SWATCHES: [u32; 12] = [
    0xffffff, 0xf1f5f9, 0xfef3c7, 0xfee2e2, 0xdcfce7, 0xdbeafe, 0x3b82f6, 0x0d9488, 0x7c3aed,
    0xdc2626, 0xf59e0b, 0x111827,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_an_outline_or_is_text() {
        let r = Rect::new(0., 0., 200., 100.);
        for kind in Kind::ALL {
            let o = outline(kind, &r, &Style::default());
            assert_eq!(o.fill.is_empty(), kind == Kind::Text, "{kind:?}");
            // An icon's name may be wider than its picture.
            assert!(
                kind == Kind::Icon || r.inflate(1.).contains_rect(&o.text),
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

/// Where a shape's picture goes: all of an icon above its name, a badge in the corner of
/// anything else.
pub(crate) fn picture_rect(kind: Kind, r: &Rect) -> Rect {
    match kind {
        Kind::Icon => Rect::new(r.x + 10., r.y + 6., r.w - 20., (r.h - 36.).max(8.)),
        Kind::Boundary => Rect::new(r.x + 10., r.y + 8., 18., 18.),
        _ => {
            let s = (r.h * 0.3).clamp(16., 26.);
            Rect::new(r.x + 7., r.y + 7., s, s)
        }
    }
}
