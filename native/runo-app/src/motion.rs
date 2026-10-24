//! One clock for everything in the main window that moves on its own: the working
//! indicator, live dots, pulses. Each frame of a loop redraws the whole window, so rather
//! than every loop asking for the next display refresh (120 a second on a ProMotion
//! screen), they read the time from here and ask for motion, and the window then redraws
//! about 30 times a second, in step with the display: enough for a glow or a spinner, at
//! a quarter of the work. Scrolling and typing still draw at the display's own rate, and
//! loops move on those frames too. While the window isn't the
//! active one, with Animations off, or with Reduce motion on in macOS, loops hold still
//! and the window redraws only twice a second, for elapsed times.

use super::*;
use std::cell::Cell;
use std::time::Instant;

/// The shortest gap between two frames of motion: a little under a 30 Hz refresh, so a
/// 120 Hz display draws every fourth refresh and a 60 Hz one every other.
const MIN_GAP: Duration = Duration::from_millis(30);
/// How often the window redraws while something shows a running time but nothing moves.
const SECONDS_GAP: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Want {
    Nothing,
    Seconds,
    Smooth,
}

thread_local! {
    static EPOCH: Instant = Instant::now();
    /// The time of the frame being drawn, so everything in it moves together.
    static NOW: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Whether the main window is drawing a frame, from its render to its last paint.
    static DRAWING: Cell<bool> = const { Cell::new(false) };
    /// Whether loops may move this frame.
    static MOVING: Cell<bool> = const { Cell::new(false) };
    static WANT: Cell<Want> = const { Cell::new(Want::Nothing) };
    /// A next frame is already asked for.
    static PENDING: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The main window starts drawing a frame. `active`: it is the active window.
pub(crate) fn begin(active: bool, cx: &App) {
    let now = Instant::now();
    NOW.set(Some(now));
    LAST.set(Some(now));
    DRAWING.set(true);
    MOVING.set(active && anim_on() && !calm(cx));
    WANT.set(Want::Nothing);
}

/// Whether motion should be calmed: the system asks for less (Accessibility > Display >
/// Reduce motion).
pub(crate) fn calm(cx: &App) -> bool {
    cx.reduce_motion() || system_reduce_motion()
}

#[cfg(target_os = "macos")]
fn system_reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
fn system_reduce_motion() -> bool {
    false
}

/// Whether the main window is drawing, so a loop can run on this clock. Loops drawn in
/// other windows (the menu bar panel) keep their own animation.
pub(crate) fn drawing() -> bool {
    DRAWING.get()
}

/// The frame the main window is drawing, while it draws: everything worked out from the
/// app's state can be worked out once per frame.
pub(crate) fn frame() -> Option<Instant> {
    DRAWING.get().then(|| NOW.get()).flatten()
}

/// Whether loops move this frame. Asks for the next frame when they do.
pub(crate) fn moving() -> bool {
    let on = MOVING.get();
    if on {
        want(Want::Smooth);
    }
    on
}

/// Something shows a running time: the window redraws at least twice a second.
pub(crate) fn ticking() {
    want(Want::Seconds);
}

fn want(w: Want) {
    if w > WANT.get() {
        WANT.set(w);
    }
}

/// Seconds since the clock started, at the frame being drawn.
pub(crate) fn now() -> f32 {
    let now = NOW.get().unwrap_or_else(Instant::now);
    EPOCH.with(|e| (now - *e).as_secs_f32())
}

/// How far through a loop of `period` the frame being drawn is, from 0 up to 1.
pub(crate) fn phase(period: Duration) -> f32 {
    let p = period.as_secs_f32();
    if p <= 0. {
        return 0.;
    }
    (now() / p).fract()
}

/// The main window's last paint: asks for the next frame if anything wants one.
pub(crate) fn end(window: &mut Window, cx: &mut App) {
    DRAWING.set(false);
    let want = WANT.replace(Want::Nothing);
    if want == Want::Nothing || PENDING.get() {
        return;
    }
    PENDING.set(true);
    let view = window.current_view();
    if want == Want::Smooth {
        window.on_next_frame(move |window, cx| next_frame(view, window, cx));
    } else {
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(SECONDS_GAP).await;
                PENDING.set(false);
                cx.update(move |_, cx| cx.notify(view)).ok();
            })
            .detach();
    }
}

/// A display refresh: draws if the last frame is far enough back, else waits for the
/// next refresh.
fn next_frame(view: EntityId, window: &mut Window, cx: &mut App) {
    if LAST.get().is_some_and(|at| at.elapsed() < MIN_GAP) {
        window.on_next_frame(move |window, cx| next_frame(view, window, cx));
        return;
    }
    PENDING.set(false);
    cx.notify(view);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn phase_wraps_within_a_period() {
        let p = phase(Duration::from_millis(1200));
        assert!((0. ..1.).contains(&p));
        assert_eq!(phase(Duration::ZERO), 0.);
    }

    #[test]
    fn the_strongest_want_wins() {
        WANT.set(Want::Nothing);
        ticking();
        want(Want::Smooth);
        ticking();
        assert!(WANT.get() == Want::Smooth);
        WANT.set(Want::Nothing);
    }
}
