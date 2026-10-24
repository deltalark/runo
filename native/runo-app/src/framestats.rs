//! Frame timing for the main window, on only when `RUNO_FRAME_STATS` names a file: every
//! two seconds a line goes to it with how many frames drew, how long building and drawing
//! them took, how many missed the display's refresh, and how much CPU Runo used meanwhile.

use std::cell::RefCell;
use std::io::Write as _;
use std::time::{Duration, Instant};

/// A frame drawn later than this after the one before it missed a refresh at 120 Hz.
const LATE: Duration = Duration::from_micros(12_500);
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
    /// Frames drawn within 50 ms of the one before: the ones a scroll or stream asked for,
    /// as opposed to one-off redraws, so a hitch is counted only among them.
    continuous: usize,
}

thread_local! {
    static STATS: RefCell<Option<Stats>> = RefCell::new(
        std::env::var_os("RUNO_FRAME_STATS").map(|p| Stats {
            path: Some(p.into()),
            ..Stats::default()
        }),
    );
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
        let Some(start) = s.window_start else { return };
        let span = start.elapsed();
        if span < EVERY {
            return;
        }
        let cpu = cpu_time().saturating_sub(s.cpu_start);
        let line = format!(
            "frames/s {fps:.0}\trender p50 {r50:.2} p95 {r95:.2} max {rmax:.2}\tframe p50 {f50:.2} p95 {f95:.2} max {fmax:.2} ms\tlate {late}/{cont}\tcpu {cpu:.0}%\n",
            fps = s.frame.len() as f64 / span.as_secs_f64(),
            r50 = pct(&mut s.render, 0.5),
            r95 = pct(&mut s.render, 0.95),
            rmax = pct(&mut s.render, 1.),
            f50 = pct(&mut s.frame, 0.5),
            f95 = pct(&mut s.frame, 0.95),
            fmax = pct(&mut s.frame, 1.),
            late = s.late,
            cont = s.continuous,
            cpu = cpu.as_secs_f64() / span.as_secs_f64() * 100.,
        );
        if let Some(path) = &s.path {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = f.write_all(line.as_bytes());
            }
        }
        s.render.clear();
        s.frame.clear();
        s.late = 0;
        s.continuous = 0;
        s.window_start = None;
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
