//! One clock for everything in the main window that moves on its own: the working
//! indicator, live dots, pulses. Each frame of a loop redraws the whole window, so rather
//! than every loop asking for the next display refresh (120 a second on a ProMotion
//! screen), they read the time from here and ask for motion, and the window then redraws
//! about 60 times a second, on every other refresh of a 120 Hz display and every refresh
//! of a 60 Hz one, so frames of motion come evenly (see `Pacer`). Scrolling and typing
//! still draw at the display's own rate, and loops move on those frames too, but they
//! don't hold the next frame of motion back. Everything that moves works out where it is
//! from the time of the frame, never from how many frames it has had, so a frame that
//! comes late only skips ahead. While the window isn't the active one, with Animations
//! off, or with Reduce motion on in macOS, loops hold still and the window redraws only
//! when an elapsed time shown in it next changes.
//!
//! Only the part of the window a loop is in draws again: each pane (see `panes`) notes
//! itself while it renders, and the next frame notifies the panes that asked, or for loops
//! outside any pane the window's ticker, which redraws the window's frame and leaves the
//! panes as they were.

use super::*;
use std::cell::{Cell, RefCell};
use std::time::Instant;

/// The gap between two frames of motion: 60 a second.
const GAP: Duration = Duration::from_micros(16_667);
/// What a display refresh is taken to be until two have been seen in a row.
const REFRESH: Duration = Duration::from_micros(8_333);
/// How often the window redraws while something shows a running time but nothing moves,
/// unless it says when the time next changes (see `tick_in`).
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
    /// When the window next redraws for a running time, if sooner than `SECONDS_GAP`.
    static TICK: Cell<Option<Duration>> = const { Cell::new(None) };
    static PACER: RefCell<Pacer> = RefCell::new(Pacer::default());
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
    NOW.set(Some(Instant::now()));
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

/// Something shows a running time that next changes in `d`: the window redraws then.
pub(crate) fn tick_in(d: Duration) {
    want(Want::Seconds);
    TICK.set(Some(TICK.get().map_or(d, |t| t.min(d))));
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

/// Time since the clock started, at the frame being drawn.
pub(crate) fn now() -> Duration {
    let now = NOW.get().unwrap_or_else(Instant::now);
    EPOCH.with(|e| now.saturating_duration_since(*e))
}

/// How far through a loop of `period` the frame being drawn is, from 0 up to 1.
pub(crate) fn phase(period: Duration) -> f32 {
    phase_at(now(), period)
}

/// How far through a loop of `period` time `t` is, from 0 up to 1. Worked out in whole
/// nanoseconds, so it's as exact days after the clock started as in the first minute:
/// seconds in an `f32` would, after a day, only tell times 8 ms apart, and loops would
/// move in uneven steps.
pub(crate) fn phase_at(t: Duration, period: Duration) -> f32 {
    let p = period.as_nanos();
    if p == 0 {
        return 0.;
    }
    ((t.as_nanos() % p) as f64 / p as f64) as f32
}

/// The main window's last paint: asks for the next frame if anything wants one.
pub(crate) fn end(window: &mut Window, cx: &mut App) {
    DRAWING.set(false);
    let want = WANT.replace(Want::Nothing);
    let tick = TICK.take().unwrap_or(SECONDS_GAP).min(SECONDS_GAP);
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
                cx.background_executor().timer(tick).await;
                PENDING_SECONDS.set(false);
                cx.update(|_, cx| notify_next(cx)).ok();
            })
            .detach();
    }
}

/// A display refresh: draws the next frame of motion if it's due, else waits for the
/// next refresh.
fn next_frame(window: &mut Window, cx: &mut App) {
    let now = Instant::now();
    if !PACER.with(|p| p.borrow_mut().refresh(now)) {
        window.on_next_frame(next_frame);
        return;
    }
    framestats::motion_frame(now);
    PENDING_SMOOTH.set(false);
    notify_next(cx);
}

/// When frames of motion are drawn: on whole display refreshes, as near `GAP` apart as
/// the display allows, and the same number of refreshes apart each time, so they come
/// evenly. Counted from the last frame of motion only: a frame drawn for something else
/// in between, a key typed or a caret blinking, doesn't hold the next one back.
#[derive(Default)]
struct Pacer {
    /// The last display refresh seen, and the last a frame of motion was drawn on.
    seen: Option<Instant>,
    drawn: Option<Instant>,
    /// The display's refresh, as seen so far.
    refresh: Option<Duration>,
}

impl Pacer {
    /// A display refresh at `now`: whether to draw a frame of motion on it.
    fn refresh(&mut self, now: Instant) -> bool {
        if let Some(seen) = self.seen {
            self.refresh = Some(refresh_from(self.refresh.unwrap_or(REFRESH), now - seen));
        }
        self.seen = Some(now);
        let due = self
            .drawn
            .is_none_or(|drawn| now - drawn >= due_after(self.refresh.unwrap_or(REFRESH)));
        if due {
            self.drawn = Some(now);
        }
        due
    }
}

/// The display's refresh, from the last estimate and a gap of `gap` between two
/// refreshes in a row: a running average, so a refresh that comes a little early or late
/// barely moves it. Gaps no display has, from a stalled main thread say, don't count.
fn refresh_from(estimate: Duration, gap: Duration) -> Duration {
    if !(Duration::from_millis(4)..=Duration::from_millis(40)).contains(&gap) {
        return estimate;
    }
    estimate.mul_f32(0.875) + gap.mul_f32(0.125)
}

/// How long after a frame of motion the next is due, on a display refreshing every
/// `refresh`: `GAP` in whole refreshes, less half a refresh, so a refresh that comes a
/// little early or late still lands on the same side.
fn due_after(refresh: Duration) -> Duration {
    let refreshes = (GAP.as_secs_f32() / refresh.as_secs_f32()).round().max(1.);
    refresh.mul_f32(refreshes - 0.5)
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
    fn phase_stays_exact_days_in() {
        // A week after the clock started, frames 1/60 s apart are still 1/60 s apart
        // round a 1.2 s loop.
        let period = Duration::from_millis(1200);
        let week = Duration::from_secs(7 * 24 * 3600) + Duration::from_millis(100);
        let frame = Duration::from_micros(16_667);
        let step = phase_at(week + frame, period) - phase_at(week, period);
        assert!((step - frame.as_secs_f32() / 1.2).abs() < 1e-5, "{step}");
    }

    /// The gaps between frames of motion on a display refreshing every `refresh`, with
    /// each refresh up to `jitter` late, by a fixed pattern.
    fn gaps(refresh: Duration, jitter: Duration) -> Vec<Duration> {
        let start = Instant::now();
        let mut pacer = Pacer::default();
        let mut drawn = vec![];
        for k in 0..240u32 {
            let late = jitter.mul_f32([0., 0.9, 0.3, 0.6, 0.1][k as usize % 5]);
            let at = start + refresh * k + late;
            if pacer.refresh(at) {
                drawn.push(at);
            }
        }
        // After the first few, while it learns the display's refresh.
        drawn.windows(2).skip(10).map(|w| w[1] - w[0]).collect()
    }

    fn ms(d: Duration) -> f32 {
        d.as_secs_f32() * 1000.
    }

    #[test]
    fn motion_comes_evenly_at_about_sixty_a_second() {
        let jitter = Duration::from_millis(3);
        for (hz, every) in [(120., 2), (60., 1), (144., 2), (100., 2)] {
            let refresh = Duration::from_secs_f32(1. / hz);
            let gaps = gaps(refresh, jitter);
            assert!(!gaps.is_empty());
            for gap in gaps {
                // Every gap the same number of refreshes, give or take the jitter.
                let want = ms(refresh) * every as f32;
                assert!(
                    (ms(gap) - want).abs() <= ms(jitter) + 0.01,
                    "{hz} Hz: a gap of {:.2} ms, not {want:.2}",
                    ms(gap)
                );
            }
        }
    }

    #[test]
    fn other_frames_dont_hold_motion_back() {
        // Frames drawn for a key typed don't reach the pacer at all: it only sees the
        // refreshes it waits on, so motion keeps its pace whatever else draws.
        let refresh = Duration::from_micros(8_333);
        let start = Instant::now();
        let mut pacer = Pacer::default();
        let drawn: Vec<u32> = (0..12u32)
            .filter(|&k| pacer.refresh(start + refresh * k))
            .collect();
        assert_eq!(drawn, [0, 2, 4, 6, 8, 10]);
    }

    #[test]
    fn a_stall_only_skips_ahead() {
        let refresh = Duration::from_micros(8_333);
        let start = Instant::now();
        let mut pacer = Pacer::default();
        assert!(pacer.refresh(start));
        assert!(!pacer.refresh(start + refresh));
        // The main thread was busy for 100 ms: the next refresh draws at once, and the
        // one after it doesn't, and the estimate of the refresh holds.
        let back = start + refresh * 14;
        assert!(pacer.refresh(back));
        assert!(!pacer.refresh(back + refresh));
        assert!(pacer.refresh(back + refresh * 2));
        assert!(ms(pacer.refresh.unwrap()) < 8.4);
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
