//! The window's background pattern. Dot matrix is a fine grid of dots in the theme's
//! ink, warming into the accent toward the top-left corner, where the lit rim's light
//! comes from. It lies on the frame round the islands (the top bar, the rail, the gutters
//! and the status bar), and in glass mode behind the islands too, between the blurred
//! desktop and the content. Dots sit on one grid in window coordinates, so every piece
//! lines up as one sheet, and a dot is only ever drawn whole.

use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pattern {
    Off,
    Dots,
}

impl Pattern {
    pub const ALL: [Pattern; 2] = [Pattern::Off, Pattern::Dots];

    pub fn id(self) -> &'static str {
        match self {
            Pattern::Off => "off",
            Pattern::Dots => "dots",
        }
    }
}

static PATTERN: AtomicU8 = AtomicU8::new(0);

pub fn set(id: &str) {
    let ix = Pattern::ALL.iter().position(|p| p.id() == id).unwrap_or(0);
    PATTERN.store(ix as u8, Ordering::Relaxed);
}

pub fn current() -> Pattern {
    Pattern::ALL[PATTERN.load(Ordering::Relaxed) as usize % Pattern::ALL.len()]
}

/// A stand-in desktop for screenshots on a machine whose window can't be see-through
/// (Xvfb): `RUNO_BACKDROP` names an image, already blurred, laid under everything.
pub fn backdrop() -> Option<AnyElement> {
    let path = std::env::var_os("RUNO_BACKDROP")?;
    settings::glass_on().then(|| {
        img(std::path::PathBuf::from(path))
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .object_fit(ObjectFit::Cover)
            .into_any_element()
    })
}

/// The dots behind the islands, in glass mode. Stronger than on the frame, since the
/// islands over them let only a part of them through.
pub fn under() -> Option<AnyElement> {
    (current() != Pattern::Off && settings::glass_on())
        .then(|| layer(|b, window| paint(b, None, UNDER, window)))
}

/// The dots on a piece of the frame (a bar, a gutter), over its fill.
pub fn chrome() -> AnyElement {
    layer(|b, window| paint(b, None, FRAME, window))
}

fn layer(f: impl FnOnce(Bounds<Pixels>, &mut Window) + 'static) -> AnyElement {
    canvas(|_, _, _| (), move |b, _, window, _| f(b, window))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
}

/// How strong the dots are on the frame, which is laid straight over them.
pub const FRAME: f32 = 1.;
/// How strong they are behind the islands in glass mode.
const UNDER: f32 = 3.;

/// The grid's pitch.
const STEP: f32 = 16.;

/// Paints the dots whose whole disc lies in `area` and clear of `skip`, `gain` times
/// their strength on the frame.
pub fn paint(area: Bounds<Pixels>, skip: Option<Bounds<Pixels>>, gain: f32, window: &mut Window) {
    if current() == Pattern::Off {
        return;
    }
    let view = window.viewport_size();
    let (w, h) = (f32::from(view.width), f32::from(view.height));
    let diag = (w * w + h * h).sqrt();
    let (ink, accent) = (c_text(), c_blue());
    let strength = gain * if is_dark() { 0.16 } else { 0.12 };
    let (left, top) = (f32::from(area.origin.x), f32::from(area.origin.y));
    let (right, bottom) = (f32::from(area.right()), f32::from(area.bottom()));
    let mut gy = (top / STEP).floor() * STEP;
    while gy < bottom {
        let mut gx = (left / STEP).floor() * STEP;
        while gx < right {
            let (x, y) = (gx + STEP / 2., gy + STEP / 2.);
            gx += STEP;
            // The corner's light: 1 at the top-left, nothing past half the diagonal.
            let light = (1. - (x * x + y * y).sqrt() / (diag * 0.5))
                .clamp(0., 1.)
                .powi(2);
            let r = 0.85 + 0.35 * light;
            if x - r < left || x + r > right || y - r < top || y + r > bottom {
                continue;
            }
            let dot = Bounds::new(point(px(x - r), px(y - r)), size(px(2. * r), px(2. * r)));
            if skip.is_some_and(|s| s.intersects(&dot)) {
                continue;
            }
            // Calm in the middle of the window, a little stronger toward its edges.
            let (cx, cy) = ((x / w - 0.5) * 2., (y / h - 0.5) * 2.);
            let edge = (cx * cx + cy * cy).sqrt().min(1.);
            let mut c: Hsla = rgb(mix(accent, ink, (light * 1.4).min(1.))).into();
            c.a = (strength * (0.6 + 0.4 * edge + 1.4 * light)).min(1.);
            window.paint_quad(fill(dot, c).corner_radii(px(r)));
        }
        gy += STEP;
    }
}
