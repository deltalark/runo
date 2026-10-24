//! How a turn that failed for a reason outside the agent's reply ended: an error that reads
//! like a sign-in problem, which is checked before it is believed, or a CLI that stopped
//! without replying, and why where that can be told.

use std::time::Duration;

/// Under this much free space the disk counts as full: the CLI can't write its session or
/// Runo the turn's output.
const LOW_DISK: u64 = 200 * 1024 * 1024;

/// What the note says when a turn is sent again after a sign-in error.
pub const RETRIED: &str = "Sign-in token was refreshing; retried.";

/// The note shown while Runo checks whether `provider` is still signed in.
pub fn checking(provider: &str) -> String {
    format!(
        "{} reported a sign-in error; checking whether it is still signed in…",
        crate::agent::provider(provider).name
    )
}

/// What to do about a turn whose errors read like a sign-in problem.
#[derive(Debug, PartialEq)]
pub enum SignIn {
    /// The agent is signed in after all, most often a login token renewed by another run
    /// at the same moment: send the turn again once.
    Retry,
    /// Say it isn't signed in, and hold the queue.
    SignedOut,
}

/// `signed_in` is what the check found (None when there is no check for this agent, or
/// it couldn't tell); `retried` whether this turn was already the one retry.
pub fn sign_in(signed_in: Option<bool>, retried: bool) -> SignIn {
    if signed_in == Some(true) && !retried {
        SignIn::Retry
    } else {
        SignIn::SignedOut
    }
}

/// How long to wait before the retry: two to five seconds, varied so several runs that
/// failed together don't all renew the token again at once.
pub fn retry_delay() -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    Duration::from_millis(2000 + u64::from(nanos) % 3000)
}

/// Why `name`'s CLI stopped without replying. `exit` is its exit code where known
/// (128 + the signal when one ended it), `log` its error output and `free` the bytes free
/// on the disk it worked on.
pub fn no_reply(name: &str, exit: Option<i32>, log: &str, free: Option<u64>) -> String {
    let lower = log.to_lowercase();
    if lower.contains("enospc")
        || lower.contains("no space left")
        || free.is_some_and(|f| f < LOW_DISK)
    {
        return format!("{name} stopped without replying: the disk is full. Free some space, then retry.");
    }
    if lower.contains("out of memory")
        || lower.contains("heap out of memory")
        || lower.contains("enomem")
    {
        return format!("{name} stopped without replying: it ran out of memory. Close some apps or agents, then retry.");
    }
    if lower.contains("timed out") || lower.contains("etimedout") {
        return format!("{name} stopped without replying: it timed out. Retry.");
    }
    match exit {
        // SIGKILL, which is also how macOS ends a process when memory runs short.
        Some(137) => format!(
            "{name} stopped without replying: it was killed, most often because memory ran short. Retry."
        ),
        Some(130) | Some(143) | Some(129) => {
            format!("{name} stopped without replying: it was ended by another program. Retry.")
        }
        _ => format!("{name} stopped without replying. Retry."),
    }
}

/// Bytes free on the disk holding `path`.
pub fn free_space(path: &str) -> Option<u64> {
    #[cfg(unix)]
    {
        let c = std::ffi::CString::new(path).ok()?;
        // SAFETY: statvfs only writes into the zeroed struct we pass it.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_in_error_while_signed_in_is_retried_once() {
        assert_eq!(sign_in(Some(true), false), SignIn::Retry);
        // The retry failed the same way: now it is said.
        assert_eq!(sign_in(Some(true), true), SignIn::SignedOut);
    }

    #[test]
    fn a_real_sign_out_still_says_so() {
        assert_eq!(sign_in(Some(false), false), SignIn::SignedOut);
        // No way to check keeps what Runo did before.
        assert_eq!(sign_in(None, false), SignIn::SignedOut);
    }

    #[test]
    fn an_empty_exit_says_why_where_it_can() {
        let full = no_reply("Claude Code", Some(0), "", Some(10 * 1024 * 1024));
        assert!(full.contains("disk is full"), "{full}");
        let enospc = no_reply("Claude Code", Some(1), "Error: ENOSPC: no space left on device, write", Some(u64::MAX));
        assert!(enospc.contains("disk is full"), "{enospc}");
        let killed = no_reply("Claude Code", Some(137), "", Some(u64::MAX));
        assert!(killed.contains("killed"), "{killed}");
        let oom = no_reply("Claude Code", Some(1), "FATAL ERROR: Reached heap limit Allocation failed - JavaScript heap out of memory", None);
        assert!(oom.contains("out of memory"), "{oom}");
        assert_eq!(
            no_reply("Claude Code", Some(0), "", Some(u64::MAX)),
            "Claude Code stopped without replying. Retry."
        );
        assert!(!no_reply("Claude Code", None, "", None).contains("signed in"));
    }

    #[test]
    fn the_delay_is_two_to_five_seconds() {
        let d = retry_delay();
        assert!(d >= Duration::from_secs(2) && d < Duration::from_secs(5));
    }
}
