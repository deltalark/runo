//! Waiting in tests for something another thread does, with a deadline, so a regression
//! fails the test instead of hanging the whole run.

use std::time::{Duration, Instant};

/// How long a test waits before it gives up.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// Poll `f` until it gives a value, panicking with `what` after [`PATIENCE`].
pub fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(value) = f() {
            return value;
        }
        assert!(
            start.elapsed() < PATIENCE,
            "gave up after {PATIENCE:?} waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Poll `done` until it holds, panicking with `what` after [`PATIENCE`].
pub fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    wait_for(what, || done().then_some(()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn gives_the_value_once_there() {
        let mut calls = 0;
        assert_eq!(
            wait_for("the third call", || {
                calls += 1;
                (calls == 3).then_some(calls)
            }),
            3
        );
    }
}
