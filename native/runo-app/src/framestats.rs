//! Frame timing for the main window, on only when `RUNO_FRAME_STATS` names a file: every
//! two seconds a line goes to it with how many frames drew, how long building and drawing
//! them took, how many missed the display's refresh, how evenly frames of motion (the
//! working indicator, live dots; see `motion`) came, and how much CPU Runo used meanwhile.
//! While a scene is named (see `scene`, which `bench` uses), one line covers the whole
//! scene instead.

use std::cell::RefCell;
use std::io::Write as _;
use std::time::{Duration, Instant};

/// A frame drawn later than this after the one before it missed a refresh at 120 Hz.
const LATE: Duration = Duration::from_micros(12_500);
/// A frame that took longer than this to build and draw misses a 60 Hz refresh.
const SLOW: Duration = Duration::from_micros(16_667);
const EVERY: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Stats {
    path: Option<std::path::PathBuf>,
    began: Option<Instant>,
    last_begin: Option<Instant>,
    window_start: Option<Instant>,
    cpu_start: Duration,
    render: Vec<Duration>,
    frame: Vec<Duration>,
    late: usize,
    /// The gaps between frames of motion, and when the last was drawn.
    motion: Vec<Duration>,
    last_motion: Option<Instant>,
    /// Frames drawn within 50 ms of the one before: the ones a scroll or stream asked for,
    /// as opposed to one-off redraws, so a hitch is counted only among them.
    continuous: usize,
    /// The scene being measured, and when it began.
    scene: Option<(String, Instant)>,
}

thread_local! {
    static STATS: RefCell<Option<Stats>> = RefCell::new(
        std::env::var_os("RUNO_FRAME_STATS").map(|p| Stats {
            path: Some(p.into()),
            ..Stats::default()
        }),
    );
}

/// Whether frame stats are being kept.
pub(crate) fn on() -> bool {
    STATS.with(|s| s.borrow().is_some())
}

/// Add `line` to the stats file as it is.
pub(crate) fn note(line: &str) {
    STATS.with(|s| {
        if let Some(s) = s.borrow().as_ref() {
            s.write(&format!("{line}\n"));
        }
    });
}

/// Measure `name` from now on, as one line when the next scene starts; `None` ends the
/// last one.
pub(crate) fn scene(name: Option<&str>) {
    STATS.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return };
        if let Some((last, began)) = s.scene.take() {
            let line = s.summary(&last, began.elapsed());
            s.write(&line);
        }
        s.reset();
        s.window_start = Some(Instant::now());
        s.cpu_start = cpu_time();
        s.scene = name.map(|n| (n.to_string(), Instant::now()));
    });
}

impl Stats {
    /// One line on the frames since the last, over `span`, headed `name`.
    fn summary(&mut self, name: &str, span: Duration) -> String {
        let cpu = cpu_time().saturating_sub(self.cpu_start);
        format!(
            "{name}\tframes/s {fps:.0}\trender p50 {r50:.2} p95 {r95:.2} max {rmax:.2}\tframe p50 {f50:.2} p95 {f95:.2} max {fmax:.2} ms\tslow {slow}/{frames}\tlate {late}/{cont}\tmotion gap p50 {m50:.2} p95 {m95:.2} max {mmax:.2} ms ({motions})\tcpu {cpu:.0}%\n",
            slow = self.frame.iter().filter(|f| **f > SLOW).count(),
            frames = self.frame.len(),
            fps = self.frame.len() as f64 / span.as_secs_f64(),
            r50 = pct(&mut self.render, 0.5),
            r95 = pct(&mut self.render, 0.95),
            rmax = pct(&mut self.render, 1.),
            f50 = pct(&mut self.frame, 0.5),
            f95 = pct(&mut self.frame, 0.95),
            fmax = pct(&mut self.frame, 1.),
            late = self.late,
            motions = self.motion.len(),
            m50 = pct(&mut self.motion, 0.5),
            m95 = pct(&mut self.motion, 0.95),
            mmax = pct(&mut self.motion, 1.),
            cont = self.continuous,
            cpu = cpu.as_secs_f64() / span.as_secs_f64() * 100.,
        )
    }

    fn write(&self, line: &str) {
        if let Some(path) = &self.path {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }

    fn reset(&mut self) {
        self.render.clear();
        self.frame.clear();
        self.late = 0;
        self.motion.clear();
        self.continuous = 0;
        self.window_start = None;
    }
}

/// The window started building a frame.
pub(crate) fn begin() {
    STATS.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return };
        let now = Instant::now();
        if let Some(last) = s.last_begin {
            let gap = now - last;
            if gap < Duration::from_millis(50) {
                s.continuous += 1;
                if gap > LATE {
                    s.late += 1;
                }
            }
        }
        s.last_begin = Some(now);
        s.began = Some(now);
        if s.window_start.is_none() {
            s.window_start = Some(now);
            s.cpu_start = cpu_time();
        }
    });
}

/// A frame of motion is drawn at `now`. Gaps of more than a quarter second are between
/// one run of motion and the next, not within one, and don't count.
pub(crate) fn motion_frame(now: Instant) {
    STATS.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return };
        if let Some(last) = s.last_motion.replace(now) {
            let gap = now - last;
            if gap < Duration::from_millis(250) {
                s.motion.push(gap);
            }
        }
    });
}

/// The view finished building its element tree.
pub(crate) fn rendered() {
    STATS.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if let Some(b) = s.began {
                s.render.push(b.elapsed());
            }
        }
    });
}

/// The window's last element painted: the frame's layout and paint are done.
pub(crate) fn painted() {
    STATS.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return };
        if let Some(b) = s.began.take() {
            s.frame.push(b.elapsed());
        }
        // A scene is written as one line when it ends.
        if s.scene.is_some() {
            return;
        }
        let Some(start) = s.window_start else { return };
        let span = start.elapsed();
        if span < EVERY {
            return;
        }
        let line = s.summary("-", span);
        s.write(&line);
        s.reset();
    });
}

/// The `q` quantile of `v`, in milliseconds.
fn pct(v: &mut [Duration], q: f64) -> f64 {
    if v.is_empty() {
        return 0.;
    }
    v.sort_unstable();
    let i = ((v.len() - 1) as f64 * q).round() as usize;
    v[i].as_secs_f64() * 1000.
}

/// CPU time this process has used, user and system.
#[cfg(unix)]
fn cpu_time() -> Duration {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills the struct it is given.
    let usage = unsafe {
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
        usage.assume_init()
    };
    let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    tv(usage.ru_utime) + tv(usage.ru_stime)
}

/// Not measured off Unix, so the summary's CPU reads 0%.
#[cfg(not(unix))]
fn cpu_time() -> Duration {
    Duration::ZERO
}
