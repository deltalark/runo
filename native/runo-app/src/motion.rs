//! One clock for everything in the main window that moves on its own: the working
//! indicator, live dots, pulses. Each frame of a loop redraws the whole window, so rather
//! than every loop asking for the next display refresh (120 a second on a ProMotion
//! screen), they read the time from here and ask for motion, and the window then redraws
//! about 30 times a second, in step with the display: enough for a glow or a spinner, at
//! a quarter of the work. Scrolling and typing still draw at the display's own rate, and
//! loops move on those frames too. While the window isn't the
//! active one, with Animations off, or with Reduce motion on in macOS, loops hold still
//! and the window redraws only twice a second, for elapsed times.
//!
//! Only the part of the window a loop is in draws again: each pane (see `panes`) notes
//! itself while it renders, and the next frame notifies the panes that asked, or for loops
//! outside any pane the window's ticker, which redraws the window's frame and leaves the
//! panes as they were.

use super::*;
use std::cell::{Cell, RefCell};
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
    /// A smooth frame, or a tick for running times, is already asked for.
    static PENDING_SMOOTH: Cell<bool> = const { Cell::new(false) };
    static PENDING_SECONDS: Cell<bool> = const { Cell::new(false) };
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The pane rendering now; `None` outside the panes, in the window's own frame.
    static VIEW: Cell<Option<EntityId>> = const { Cell::new(None) };
    /// What draws the window's frame again without the panes: its ticker.
    static ROOT: Cell<Option<EntityId>> = const { Cell::new(None) };
    /// The views that asked for motion this frame, and those the next frame notifies.
    static ASKED: RefCell<Vec<EntityId>> = const { RefCell::new(Vec::new()) };
    static NEXT: RefCell<Vec<EntityId>> = const { RefCell::new(Vec::new()) };
}

/// The main window starts drawing a frame. `active`: it is the active window. `root`
/// redraws the window's frame on its own (see the module docs).
pub(crate) fn begin(active: bool, root: EntityId, cx: &App) {
    let now = Instant::now();
    NOW.set(Some(now));
    LAST.set(Some(now));
    DRAWING.set(true);
    MOVING.set(active && anim_on() && !calm(cx));
    WANT.set(Want::Nothing);
    ROOT.set(Some(root));
    VIEW.set(None);
}

/// Run `f`, which renders pane `view`: motion asked for meanwhile redraws that pane.
pub(crate) fn within<R>(view: EntityId, f: impl FnOnce() -> R) -> R {
    let outer = VIEW.replace(Some(view));
    let out = f();
    VIEW.set(outer);
    out
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
    if let Some(view) = VIEW.get().or(ROOT.get()) {
        ASKED.with(|a| {
            let mut a = a.borrow_mut();
            if !a.contains(&view) {
                a.push(view);
            }
        });
    }
}

/// Notify every view the next frame is for.
fn notify_next(cx: &mut App) {
    for view in NEXT.with(|n| std::mem::take(&mut *n.borrow_mut())) {
        cx.notify(view);
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
    let asked = ASKED.with(|a| std::mem::take(&mut *a.borrow_mut()));
    if want == Want::Nothing {
        return;
    }
    NEXT.with(|n| {
        let mut n = n.borrow_mut();
        for view in asked {
            if !n.contains(&view) {
                n.push(view);
            }
        }
    });
    if want == Want::Smooth {
        if !PENDING_SMOOTH.replace(true) {
            window.on_next_frame(next_frame);
        }
    } else if !PENDING_SECONDS.replace(true) {
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(SECONDS_GAP).await;
                PENDING_SECONDS.set(false);
                cx.update(|_, cx| notify_next(cx)).ok();
            })
            .detach();
    }
}

/// A display refresh: draws if the last frame is far enough back, else waits for the
/// next refresh.
fn next_frame(window: &mut Window, cx: &mut App) {
    if LAST.get().is_some_and(|at| at.elapsed() < MIN_GAP) {
        window.on_next_frame(next_frame);
        return;
    }
    PENDING_SMOOTH.set(false);
    notify_next(cx);
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
