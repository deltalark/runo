//! How long Runo takes to start, for `RUNO_TRACE_STARTUP=1`: each step's time since
//! `main` began, on stderr. `RUNO_TRACE_STARTUP=exit` also quits once the window
//! responds, so a script can launch Runo again and again.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();
static MODE: OnceLock<Option<String>> = OnceLock::new();

fn mode() -> Option<&'static str> {
    MODE.get_or_init(|| {
        std::env::var("RUNO_TRACE_STARTUP")
            .ok()
            .filter(|v| !v.is_empty())
    })
    .as_deref()
}

/// The start of `main`; later steps are timed from here.
pub fn begin() {
    START.get_or_init(Instant::now);
    if mode().is_some() {
        // Wall-clock milliseconds, so a launcher can add the time before `main` (loading
        // the binary and its libraries).
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        eprintln!("startup main_epoch_ms={epoch}");
    }
}

/// Note that `step` was reached.
pub fn mark(step: &str) {
    if mode().is_none() {
        return;
    }
    let ms = START
        .get()
        .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
    eprintln!("startup {ms:8.1} ms  {step}");
}

/// Whether to quit once the window responds.
pub fn exit_when_ready() -> bool {
    mode() == Some("exit")
}
