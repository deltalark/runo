//! The accent corner's optional glow: the bracket on the open tab and the selected row
//! can stay still (the default), breathe like a neon tube in the accent, or cycle the
//! rainbow like an RGB light.
//!
//! It is kept cheap. A moving glow asks for a new frame at most `FPS` times a second,
//! not every display refresh, only while its window is in front, and only while a
//! bracket is on screen to ask. With Animations off in Settings, or Reduce motion on in
//! the system, the glow holds still.

use super::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// How often a moving glow redraws. Plenty for a slow pulse or colour fade, and a
/// quarter of what a 120Hz display would otherwise draw.
const FPS: u64 = 30;

static STYLE: AtomicU8 = AtomicU8::new(0);
static SPEED: AtomicU8 = AtomicU8::new(1);
static INTENSITY: AtomicU8 = AtomicU8::new(1);

const STATIC: u8 = 0;
const NEON: u8 = 1;
const RGB: u8 = 2;

/// Take the style, speed and intensity from Settings.
pub fn configure(style: &str, speed: &str, intensity: &str) {
    let style = match style {
        "neon" => NEON,
        "rgb" => RGB,
        _ => STATIC,
    };
    STYLE.store(style, Ordering::Relaxed);
    SPEED.store(
        match speed {
            "slow" => 0,
            "fast" => 2,
            _ => 1,
        },
        Ordering::Relaxed,
    );
    INTENSITY.store(
        match intensity {
            "soft" => 0,
            "bright" => 2,
            _ => 1,
        },
        Ordering::Relaxed,
    );
}

/// Whether the corner glows at all. When it doesn't, brackets draw exactly as before.
pub fn on() -> bool {
    STYLE.load(Ordering::Relaxed) != STATIC
}

/// What a bracket looks like this frame: its own colour, and the glow round it as a
/// colour and a strength from 0 to 1.
#[derive(Clone, Copy)]
pub struct Look {
    pub core: Hsla,
    pub glow: Option<(Hsla, f32)>,
}

/// The bracket's look now. Asks for the next frame when the glow is moving, so call it
/// while painting a bracket that is on screen.
pub fn look(window: &mut Window, cx: &mut App) -> Look {
    let accent = c_blue();
    let style = STYLE.load(Ordering::Relaxed);
    if style == STATIC {
        return Look {
            core: rgb(accent).into(),
            glow: None,
        };
    }
    let moving = anim_on() && !reduce_motion();
    if moving {
        tick(window, cx);
    }
    let speed = SPEED.load(Ordering::Relaxed) as usize;
    match style {
        NEON => {
            // A slow breath: dim to bright and back, eased at both ends like a bulb
            // warming and cooling. Still, it holds a little above the middle.
            let breath = if moving {
                let t = phase([3.6, 2.4, 1.4][speed]);
                0.5 - 0.5 * (t * std::f32::consts::TAU).cos()
            } else {
                0.6
            };
            // The tube itself warms toward white at the top of each breath.
            let warm = if is_dark() {
                0.1 + 0.3 * breath
            } else {
                0.05 * breath
            };
            Look {
                core: rgb(mix(accent, 0xffffff, warm)).into(),
                glow: Some((rgb(accent).into(), 0.35 + 0.65 * breath)),
            }
        }
        _ => {
            // Round the colour wheel, starting from the accent so switching it on
            // doesn't jump. Held still, it stays on the accent.
            let start = Hsla::from(rgb(accent)).h;
            let h = if moving {
                (start + phase([10., 6., 3.][speed])).fract()
            } else {
                start
            };
            let c = hsla(h, 0.9, if is_dark() { 0.64 } else { 0.5 }, 1.);
            Look {
                core: c,
                glow: Some((c, 0.85)),
            }
        }
    }
}

/// Paint the glow round a bracket: soft strokes along its line, widest and faintest
/// first, before the bracket's own colour goes on top. `line` traces the bracket's
/// centre line into the builder it is given.
pub fn paint_glow(window: &mut Window, look: Look, line: impl Fn(&mut PathBuilder)) {
    let Some((color, strength)) = look.glow else {
        return;
    };
    let (alpha, reach) = match INTENSITY.load(Ordering::Relaxed) {
        0 => (0.6, 0.8),
        2 => (1.5, 1.25),
        _ => (1., 1.),
    };
    // A glow reads fainter on a light window, which it can only darken.
    let alpha = alpha * strength * if is_dark() { 1. } else { 1.35 };
    for (width, a) in [(12., 0.07), (8., 0.11), (5., 0.18), (3., 0.3)] {
        let mut f = PathBuilder::stroke(px(width * reach));
        line(&mut f);
        if let Ok(path) = f.build() {
            window.paint_path(
                path,
                Hsla {
                    a: (a * alpha).min(1.),
                    ..color
                },
            );
        }
    }
}

/// Where the glow is in a cycle of `secs` seconds, from 0 to 1. Every bracket shares one
/// clock, so they all pulse together.
fn phase(secs: f32) -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f32();
    (t / secs).fract()
}

thread_local! {
    /// Views with a glow frame already on its way, so many brackets ask once.
    static TICKING: RefCell<HashSet<EntityId>> = RefCell::new(HashSet::new());
}

/// Ask for the next glow frame for the view being painted, `1 / FPS` from now. A window
/// in the back gets none, and picks up again when it comes forward, which redraws it.
fn tick(window: &mut Window, cx: &mut App) {
    if !window.is_window_active() {
        return;
    }
    let view = window.current_view();
    if !TICKING.with_borrow_mut(|t| t.insert(view)) {
        return;
    }
    window
        .spawn(cx, async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1000 / FPS))
                .await;
            TICKING.with_borrow_mut(|t| t.remove(&view));
            cx.update(|_, cx| cx.notify(view)).ok();
        })
        .detach();
}

/// Whether the system asks for less motion (macOS: Accessibility > Display > Reduce
/// motion). Looked up at most every few seconds.
fn reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::cell::Cell;
        thread_local! {
            static CHECKED: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
        }
        if let Some((at, v)) = CHECKED.get() {
            if at.elapsed() < Duration::from_secs(3) {
                return v;
            }
        }
        let v =
            objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        CHECKED.set(Some((Instant::now(), v)));
        v
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
