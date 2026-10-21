//! How long launching takes, step by step. With `RUNO_STARTUP_TRACE=1` set, each step
//! prints the time since `main` started to stderr, along with the first frames and any
//! stall on the main thread in the first few seconds. Without it, nothing is recorded.

use gpui_kit::{App, Window};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();
static ON: OnceLock<bool> = OnceLock::new();

/// Start the clock. The first thing `main` does.
pub fn begin() {
    START.get_or_init(Instant::now);
}

pub fn on() -> bool {
    *ON.get_or_init(|| std::env::var_os("RUNO_STARTUP_TRACE").is_some())
}

fn since_start() -> f64 {
    START
        .get()
        .map_or(0., |s| s.elapsed().as_secs_f64() * 1000.)
}

/// A step reached.
pub fn mark(step: &str) {
    if on() {
        eprintln!("[startup] {:8.1} ms  {step}", since_start());
    }
}

/// A step and how long it took on its own.
pub fn time<T>(step: &str, f: impl FnOnce() -> T) -> T {
    if !on() {
        return f();
    }
    let t = Instant::now();
    let out = f();
    eprintln!(
        "[startup] {:8.1} ms  {step} ({:.1} ms)",
        since_start(),
        t.elapsed().as_secs_f64() * 1000.
    );
    out
}

static FRAMES: AtomicU32 = AtomicU32::new(0);

/// When the main window starts building one of its first frames.
pub fn frame_begin() -> Option<Instant> {
    (on() && FRAMES.load(Ordering::Relaxed) < 6).then(Instant::now)
}

/// The main window's first frames: when each was drawn, and how long building it took.
pub fn frame_end(began: Option<Instant>, window: &Window) {
    let Some(began) = began else {
        return;
    };
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    let build = began.elapsed().as_secs_f64() * 1000.;
    window.on_next_frame(move |_, _| {
        eprintln!(
            "[startup] {:8.1} ms  frame {n} on screen (render {build:.1} ms)",
            since_start()
        );
    });
}

/// Watch the main thread for the first few seconds and report the longest it was busy,
/// so work that blocks it after the first frame shows up too.
pub fn watch_main_thread(cx: &mut App) {
    if !on() {
        return;
    }
    cx.spawn(async move |cx| {
        let tick = Duration::from_millis(8);
        let mut worst = Duration::ZERO;
        let mut worst_at = 0.;
        let end = Instant::now() + Duration::from_secs(6);
        while Instant::now() < end {
            let asked = Instant::now();
            cx.background_executor().timer(tick).await;
            let late = asked.elapsed().saturating_sub(tick);
            if late > worst {
                worst = late;
                worst_at = since_start();
            }
            if late > Duration::from_millis(50) {
                eprintln!(
                    "[startup] {:8.1} ms  main thread busy {:.0} ms",
                    since_start(),
                    late.as_secs_f64() * 1000.
                );
            }
        }
        eprintln!(
            "[startup] longest main-thread stall in the first 6 s: {:.0} ms (at {worst_at:.0} ms)",
            worst.as_secs_f64() * 1000.
        );
    })
    .detach();
}
