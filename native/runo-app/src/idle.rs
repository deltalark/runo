//! What Runo does while nobody is typing: the periodic look at the open session's git
//! state, dev-server ports, its pull request and the rest, paced by whether the window
//! is on screen (`wake::shown`). Work that only changes what the window shows stops
//! while it is hidden and picks up when it comes back; work that keeps agents going
//! (saves, queued turns, Crew) only slows down.

use super::*;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

/// The git rescan of the open session while its agent works.
const RESCAN: Duration = Duration::from_secs(2);
/// How often the open session's git marks (index, HEAD, ...) are looked at; a scan
/// follows when one changed.
const MARKS: Duration = Duration::from_secs(2);
/// A scan of an idle session, so outside edits show up, that doubles while nothing
/// changes, up to `IDLE_SCAN_MAX`.
const IDLE_SCAN: Duration = Duration::from_secs(10);
const IDLE_SCAN_MAX: Duration = Duration::from_secs(120);
/// The open session's pull request, looked up again after this, doubling each time the
/// answer is the same, up to `PR_MAX`.
const PR_EVERY: Duration = Duration::from_secs(60);
const PR_MAX: Duration = Duration::from_secs(16 * 60);
/// Dev-server ports, while a session is open with the Browser on.
const PORTS: Duration = Duration::from_secs(5);
/// The open worktree's size.
const SPACE: Duration = Duration::from_secs(5);
/// Plan usage for the status bar: first after `USAGE_FIRST`, then every `USAGE_EVERY`.
const USAGE_FIRST: Duration = Duration::from_secs(10);
const USAGE_EVERY: Duration = Duration::from_secs(300);

/// When each periodic job last ran.
pub(crate) struct Clock {
    started: Instant,
    done_clean: bool,
    second: Every,
    rescan: Every,
    marks: Every,
    ports: Every,
    space: Every,
    minute: Every,
    usage: Option<Instant>,
}

impl Clock {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            done_clean: false,
            second: Every::default(),
            rescan: Every::default(),
            marks: Every::default(),
            ports: Every::default(),
            space: Every::default(),
            // The first look comes right away, as the update check always has.
            minute: Every::default(),
            usage: None,
        }
    }
}

/// A job that runs at most once per period. Timers wake a little late, so a run that
/// comes almost a period after the last counts.
#[derive(Default)]
struct Every(Option<Instant>);

impl Every {
    fn due(&mut self, period: Duration) -> bool {
        let now = Instant::now();
        let slack = period / 10;
        if self
            .0
            .is_none_or(|at| now.duration_since(at) + slack >= period)
        {
            self.0 = Some(now);
            return true;
        }
        false
    }
}

/// What the idle loop keeps between runs.
#[derive(Default)]
pub(crate) struct Idle {
    /// The open session's git marks at its last scan, the time of that scan, and the
    /// wait before the next one if nothing changes.
    scan_marks: Option<(u64, Marks)>,
    scan_at: Option<Instant>,
    scan_every: Option<Duration>,
    /// By session: when its pull request is looked up next, and how many times in a row
    /// the answer was the same.
    pr_next: HashMap<u64, (Instant, u32)>,
    /// Output from sessions not on screen waits until this flushes it, a few times a
    /// second at most.
    pub(crate) unshown_since: Option<Instant>,
    unshown_flush: Option<Task<()>>,
    /// The Browser tab's frame pump while a browser runs, and whether it runs at display
    /// rate (a page is on screen) or slowly (only notes to pick up).
    browser_pump: Option<(Task<()>, bool)>,
}

/// Modification times of what changes when git state does: the index, HEAD and its log,
/// the last fetch, and the folder itself (files added or removed at its top).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Marks(Vec<Option<SystemTime>>);

impl Marks {
    pub(crate) fn of(dir: &str) -> Self {
        let dir = Path::new(dir);
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut out = vec![mtime(dir)];
        if let Some(git) = git_dir(dir) {
            let common = std::fs::read_to_string(git.join("commondir"))
                .ok()
                .map(|c| git.join(c.trim()))
                .unwrap_or_else(|| git.clone());
            for p in [
                git.join("index"),
                git.join("HEAD"),
                git.join("logs").join("HEAD"),
                common.join("FETCH_HEAD"),
            ] {
                out.push(mtime(&p));
            }
        }
        Marks(out)
    }
}

/// The git folder of a repository or worktree whose top is `dir`: `.git`, or where a
/// `.git` file points.
fn git_dir(dir: &Path) -> Option<PathBuf> {
    let dot = dir.join(".git");
    let meta = std::fs::metadata(&dot).ok()?;
    if meta.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let path = text.trim().strip_prefix("gitdir:")?.trim();
    Some(dir.join(path))
}

/// The wait before the next lookup after `same` identical answers in a row.
fn pr_wait(same: u32) -> Duration {
    (PR_EVERY * 2u32.saturating_pow(same.min(8))).min(PR_MAX)
}

impl RunoApp {
    /// How long the idle loop sleeps: a quarter second while agent updates run (their
    /// cards count), a second otherwise, and five seconds while the window is hidden.
    pub(crate) fn housekeeping_pace(&self) -> Duration {
        if !wake::shown() {
            return Duration::from_secs(5);
        }
        if self.update_runs.values().any(|r| r.failed.is_none()) {
            return Duration::from_millis(250);
        }
        Duration::from_secs(1)
    }

    pub(crate) fn housekeeping(&mut self, clock: &mut Clock, cx: &mut Context<Self>) {
        let shown = wake::shown();
        let since = clock.started.elapsed();
        // Runo's own work, hidden or not: agent update checks and the build output of
        // done threads.
        if clock.minute.due(Duration::from_secs(60)) {
            if shown {
                self.maybe_check_updates();
            }
            self.run_build_clean();
        }
        if !clock.done_clean && since >= Duration::from_secs(30) {
            clock.done_clean = true;
            self.queue_done_build_clean();
        }
        let repaint = caps::take_updated();
        let pruned = self.prune_update_runs();
        if !shown {
            return;
        }
        // The rest only changes what the window shows.
        if clock.usage.is_none_or(|at| at <= Instant::now()) && since >= USAGE_FIRST {
            clock.usage = Some(Instant::now() + USAGE_EVERY);
            usage::refresh(true);
        }
        let on_session = self.view == View::Session && self.selected.is_some();
        let watching = on_session && self.selected.is_some_and(|id| self.turns.contains_key(&id));
        if on_session && clock.space.due(SPACE) {
            self.space_tick(false);
        }
        // Dev servers, so the header's Browser button appears once the app is served and
        // goes when it stops.
        if on_session && cx.global::<AppSettings>().browser_enabled && clock.ports.due(PORTS) {
            self.scan_ports();
        }
        if on_session {
            if let Some(id) = self.selected.filter(|id| self.pr_due(*id)) {
                self.load_pr_status(id);
            }
        }
        let second = clock.second.due(Duration::from_secs(1));
        if second {
            let before = (self.file_conflict, self.pending_file.is_some());
            self.watch_open_file();
            if before != (self.file_conflict, self.pending_file.is_some()) {
                cx.notify();
            }
        }
        // The open session's git state: every 2s while its agent works, so the Changes
        // panel fills in as files are edited; otherwise when git's files change, or
        // now and then for edits made outside.
        if watching {
            if clock.rescan.due(RESCAN) {
                self.refresh();
            }
        } else if on_session && clock.marks.due(MARKS) {
            self.idle_scan();
        }
        if repaint
            || pruned
            || (second && (self.info.is_some() || self.git_toast.is_some()))
            || self.update_runs.values().any(|r| r.failed.is_none())
        {
            cx.notify();
        } else if second && !self.turns.is_empty() {
            // Once a second while agents run, for their elapsed time: they show where
            // background output does.
            self.notify_background(cx);
        }
    }

    /// Scan the open, idle session when its git marks changed, or when it hasn't been
    /// scanned for a while; that while doubles each time nothing had changed.
    fn idle_scan(&mut self) {
        let Some(s) = self.current() else {
            return;
        };
        let marks = Marks::of(&s.dir);
        let changed = self
            .idle
            .scan_marks
            .as_ref()
            .is_none_or(|(id, m)| *id != s.id || *m != marks);
        let every = self.idle.scan_every.unwrap_or(IDLE_SCAN);
        if changed {
            self.idle.scan_every = Some(IDLE_SCAN);
        } else if self.idle.scan_at.is_some_and(|at| at.elapsed() < every) {
            return;
        } else {
            self.idle.scan_every = Some((every * 2).min(IDLE_SCAN_MAX));
        }
        self.refresh();
    }

    /// Note a scan starting now, for `idle_scan`.
    pub(crate) fn scan_started(&mut self) {
        self.idle.scan_at = Some(Instant::now());
    }

    /// A scan of `id` finished: its git marks as the scan left them, for `idle_scan`.
    pub(crate) fn scan_done(&mut self, id: u64, dir: &str) {
        self.idle.scan_marks = Some((id, Marks::of(dir)));
    }

    fn pr_due(&self, id: u64) -> bool {
        self.idle
            .pr_next
            .get(&id)
            .is_none_or(|(at, _)| *at <= Instant::now())
    }

    /// A pull request lookup for `id` answered; `same` when nothing had changed. While
    /// its agent works the next one is a minute away, as before.
    pub(crate) fn pr_checked(&mut self, id: u64, same: bool) {
        let streak = match self.idle.pr_next.get(&id) {
            Some((_, n)) if same => n + 1,
            _ => 0,
        };
        let wait = if self.turn_running(id) {
            PR_EVERY
        } else {
            pr_wait(streak)
        };
        self.idle
            .pr_next
            .insert(id, (Instant::now() + wait, streak));
    }

    /// After a batch of events: draw what they changed now, or within `BACKGROUND_WAIT`
    /// for output from sessions not on screen, and bring the Dock's count up to date
    /// unless only output streamed (it doesn't change what needs the person). Output
    /// streaming into the open chat (and into the side panel when `peeked`) draws that
    /// chat at once, and Runo's own `output` the terminal panel.
    pub(crate) fn after_events(
        &mut self,
        at_once: bool,
        streamed: bool,
        peeked: bool,
        output: bool,
        cx: &mut Context<Self>,
    ) {
        if at_once {
            self.idle.unshown_since = None;
            self.idle.unshown_flush = None;
            cx.notify();
            self.show_dock(cx);
            return;
        }
        let waited = *self.idle.unshown_since.get_or_insert_with(Instant::now);
        if waited.elapsed() >= BACKGROUND_WAIT {
            self.idle.unshown_since = None;
            self.idle.unshown_flush = None;
            self.notify_background(cx);
        } else if self.idle.unshown_flush.is_none() {
            let wait = BACKGROUND_WAIT.saturating_sub(waited.elapsed());
            self.idle.unshown_flush = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |this, cx| {
                    this.idle.unshown_since = None;
                    this.idle.unshown_flush = None;
                    this.notify_background(cx);
                });
            }));
        }
        if streamed {
            self.notify_chat(peeked, cx);
        }
        if output {
            self.notify_parts(&[panes::Part::Terminal], cx);
        }
    }

    /// The Dock icon's count, the same number as the corner mark's badge.
    pub(crate) fn show_dock(&self, cx: &App) {
        let on = cx.global::<AppSettings>().dock_badge;
        dock::show(if on { self.attention() } else { 0 });
    }

    /// The window came on screen or went off it.
    pub(crate) fn window_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        wake::set_shown(shown);
        if shown {
            // What waited while it was hidden, and the browser's frames again.
            cx.notify();
        }
        self.ensure_browser_pump(cx);
    }

    /// Run the Browser tab's frame pump at display rate while a page is on screen, slowly
    /// while a browser runs but no page shows (for its notes), and not at all without one.
    pub(crate) fn ensure_browser_pump(&mut self, cx: &mut Context<Self>) {
        if self.browser.is_none() {
            self.idle.browser_pump = None;
            return;
        }
        let fast = self.browser_page_shown(cx);
        if self
            .idle
            .browser_pump
            .as_ref()
            .is_some_and(|(_, running_fast)| *running_fast == fast)
        {
            return;
        }
        let task = cx.spawn(async move |this, cx| loop {
            let wait = if fast {
                Duration::from_millis(16)
            } else {
                Duration::from_secs(1)
            };
            cx.background_executor().timer(wait).await;
            let Ok(keep) = this.update(cx, |this, cx| {
                this.pump_browser(cx);
                // A page came on screen or went: the next pump runs at the other pace.
                let stay = this.browser.is_some() && this.browser_page_shown(cx) == fast;
                if !stay {
                    this.idle.browser_pump = None;
                    this.ensure_browser_pump(cx);
                }
                stay
            }) else {
                break;
            };
            if !keep {
                break;
            }
        });
        self.idle.browser_pump = Some((task, fast));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn every_runs_once_per_period() {
        let mut e = Every::default();
        assert!(e.due(Duration::from_secs(60)));
        assert!(!e.due(Duration::from_secs(60)));
        e.0 = Some(Instant::now() - Duration::from_secs(55));
        assert!(e.due(Duration::from_secs(60)), "within the slack");
        e.0 = Some(Instant::now() - Duration::from_secs(30));
        assert!(!e.due(Duration::from_secs(60)));
    }

    #[test]
    fn pr_lookups_back_off_while_nothing_changes() {
        assert_eq!(pr_wait(0), PR_EVERY);
        assert_eq!(pr_wait(1), PR_EVERY * 2);
        assert_eq!(pr_wait(3), PR_EVERY * 8);
        assert_eq!(pr_wait(4), PR_MAX);
        assert_eq!(pr_wait(40), PR_MAX);
    }

    #[test]
    fn marks_follow_git_and_worktree_files() {
        let root = std::env::temp_dir().join(format!("runo-idle-marks-{}", std::process::id()));
        let repo = root.join("repo");
        let tree = root.join("tree");
        let wt_git = repo.join(".git").join("worktrees").join("tree");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(wt_git.join("HEAD"), "ref: refs/heads/side\n").unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        std::fs::write(tree.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        assert_eq!(git_dir(&repo), Some(repo.join(".git")));
        assert_eq!(git_dir(&tree), Some(wt_git.clone()));
        let before = Marks::of(tree.to_str().unwrap());
        // The worktree's own HEAD exists; its index and the fetch don't yet.
        assert_eq!(before.0.len(), 5);
        assert!(before.0[2].is_some() && before.0[1].is_none() && before.0[4].is_none());
        std::fs::write(repo.join(".git").join("FETCH_HEAD"), "x").unwrap();
        assert_ne!(Marks::of(tree.to_str().unwrap()), before);
        // Not a repository: only the folder itself.
        assert_eq!(Marks::of(root.to_str().unwrap()).0.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
