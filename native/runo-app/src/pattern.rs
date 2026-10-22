//! The window's background pattern. Aurora is soft light in the theme's accent pooling in
//! the top-left corner, with violet and ember light from the other corners, the same
//! colours Home's headline blends. It lies on the frame round the islands, and in glass
//! mode behind the islands too, between the blurred desktop and the content, so it
//! glows through every surface. It is drawn in window coordinates, so the pieces on the
//! top bar, the rail, the gutters and the status bar line up as one light.

use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pattern {
    Off,
    Aurora,
}

impl Pattern {
    pub const ALL: [Pattern; 2] = [Pattern::Off, Pattern::Aurora];

    pub fn id(self) -> &'static str {
        match self {
            Pattern::Off => "off",
            Pattern::Aurora => "aurora",
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

/// The violet and ember Home's headline blends the accent into.
const VIOLET: u32 = 0x8b5cf6;
const EMBER: u32 = 0xd97757;

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

/// The light behind the islands, in glass mode. Stronger than on the frame, since the
/// islands over it let only a part of it through.
pub fn under() -> Option<AnyElement> {
    (current() != Pattern::Off && settings::glass_on())
        .then(|| layer(|b, window| paint(&[b], UNDER, window)))
}

/// The light on a piece of the frame (a bar, a gutter), over its fill.
pub fn chrome() -> AnyElement {
    layer(|b, window| paint(&[b], FRAME, window))
}

/// A faint veil of the light over the whole window, islands and all, so the light
/// reaches across the content as well as the frame.
pub fn wash() -> Option<AnyElement> {
    (current() != Pattern::Off).then(|| layer(|b, window| paint(&[b], WASH, window)))
}

fn layer(f: impl FnOnce(Bounds<Pixels>, &mut Window) + 'static) -> AnyElement {
    canvas(|_, _, _| (), move |b, _, window, _| f(b, window))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
}

/// How strong the light is on the frame, which is laid straight over it.
pub const FRAME: f32 = 1.9;
/// How strong it is behind the islands in glass mode.
const UNDER: f32 = 2.4;
/// How strong the veil of it over everything is: light falling on the window, faint
/// enough that text keeps its contrast.
const WASH: f32 = 0.24;

/// Paints the pattern clipped to each of `areas`, `gain` times its strength on the frame.
pub fn paint(areas: &[Bounds<Pixels>], gain: f32, window: &mut Window) {
    if current() == Pattern::Off {
        return;
    }
    let view = window.viewport_size();
    let (w, h) = (f32::from(view.width), f32::from(view.height));
    for area in areas {
        if area.size.width <= px(0.) || area.size.height <= px(0.) {
            continue;
        }
        window.with_content_mask(Some(ContentMask { bounds: *area }), |window| {
            aurora(w, h, gain, window)
        });
    }
}

/// Soft light: the accent from the top-left, reaching down the rail and along the top
/// bar, a violet blend of it from the top-right and an ember glow from the bottom-right.
fn aurora(w: f32, h: f32, gain: f32, window: &mut Window) {
    let accent = c_blue();
    let s = gain * if is_dark() { 0.5 } else { 0.3 };
    let blob = |window: &mut Window, x: f32, y: f32, bw: f32, bh: f32, color: u32, a: f32| {
        let mut c = rgb(color);
        c.a = a.clamp(0., 1.);
        window.paint_drop_shadows(
            Bounds::new(
                point(px(x - bw / 2.), px(y - bh / 2.)),
                size(px(bw), px(bh)),
            ),
            Corners::all(px(bw.min(bh) / 2.)),
            &[BoxShadow {
                color: c.into(),
                offset: point(px(0.), px(0.)),
                blur_radius: px(200.),
                spread_radius: px(24.),
                inset: false,
            }],
        );
    };
    blob(window, w * 0.14, 0., w * 0.46, 240., accent, s);
    blob(window, 0., h * 0.42, 220., h * 0.56, accent, s * 0.75);
    blob(
        window,
        w * 0.78,
        0.,
        w * 0.34,
        180.,
        mix(VIOLET, accent, 0.6),
        s * 0.75,
    );
    blob(window, w, h, w * 0.42, 260., EMBER, s * 0.55);
    blob(
        window,
        w * 0.42,
        h,
        w * 0.3,
        160.,
        mix(VIOLET, accent, 0.6),
        s * 0.35,
    );
}
