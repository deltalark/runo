//! The pull request watcher. A Crew thread that pushed and opened a pull request ends its
//! turn instead of waiting on CI; Runo reads the pull request every couple of minutes
//! (`load_pr_status`) and wakes the thread only when there is something to do: a check
//! failed, someone else reviewed or commented, or the branch conflicts with its base.
//! Only comments from the repository's owner, members and collaborators wake it: anyone
//! can comment on a public repository, so another person's comment only shows on the
//! card ("New comment from @x") for the person to read.
//! Checks passing only turn the thread's card green, without a wake: a turn just to
//! answer "done" would reload the thread's whole context for nothing.
//!
//! A watched thread isn't running, so it holds no slot. What the watcher decides is in
//! `decide` and `read_failed`, pure functions over the saved `Watch` and one read of the
//! pull request; the rest of this file reads, wakes and draws.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::github::{PrStatus, Voice};
use crate::provider::{self, Host, TrackingPlan, TrackingReason};

/// How often a watched pull request is read.
pub const POLL: Duration = Duration::from_secs(120);
/// Failed reads in a row before the watcher gives up.
pub const MAX_FAILED_READS: u32 = 8;
/// Wakes for review feedback alone before the watcher gives up.
pub const MAX_COMMENT_WAKES: u32 = 10;
/// Wakes to fix CI on one head commit before the person is asked.
pub const MAX_CI_WAKES: u32 = 3;
/// Wakes to fix CI, over all head commits, since checks last passed. Each fix pushes a new
/// head, so the per-head cap alone would let a thread chase CI forever.
pub const MAX_CI_STREAK: u32 = 6;
/// Reads of an open pull request where no CI run has appeared before the watcher
/// decides there is no CI on this push. With `POLL` of two minutes, two reads land
/// just over two minutes after the first one and give external CI (Jenkins, Buildkite,
/// Circle) time to report back to the host.
pub const MAX_PROBE_READS: u32 = 2;

/// Why a thread was last woken, so its card can say what it is doing.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Wake {
    Ci,
    Review,
    Conflict,
}

/// A thread's watched pull request, saved with the thread.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Watch {
    pub pr: u64,
    #[serde(default)]
    pub url: String,
    /// The pull request's head commit at the last read.
    #[serde(default)]
    pub head: String,
    /// The head commit CI wakes were last counted for, and how many it got.
    #[serde(default)]
    pub ci_head: String,
    #[serde(default)]
    pub ci_wakes: u32,
    /// CI wakes since checks last passed.
    #[serde(default)]
    pub ci_streak: u32,
    #[serde(default)]
    pub comment_wakes: u32,
    #[serde(default)]
    pub failed_reads: u32,
    /// The time of the latest review or comment from someone else already sent.
    #[serde(default)]
    pub seen: String,
    /// The head commit the thread was told conflicts with its base.
    #[serde(default)]
    pub conflict_head: String,
    /// The head commit whose checks all passed.
    #[serde(default)]
    pub green_head: String,
    /// The wake the thread is working on, until its turn ends.
    #[serde(default)]
    pub waking: Option<Wake>,
    /// Why the watcher stopped waking the thread and wants the person.
    #[serde(default)]
    pub needs: Option<String>,
    /// Who last commented without being one of the repository's people, shown on the
    /// card instead of waking the thread, until its next turn ends.
    #[serde(default)]
    pub stranger: Option<String>,
    /// The time of that latest comment already shown.
    #[serde(default)]
    pub stranger_seen: String,
    /// Reads of this pull request where no CI run or check had appeared yet. A push
    /// that never triggers CI stops waking the thread after `MAX_PROBE_READS`.
    #[serde(default)]
    pub probe_reads: u32,
    /// The push produced no CI at all; the card says so instead of pretending to wait.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_ci_detected: bool,
}

/// What to do after one read of the pull request.
#[derive(Clone, PartialEq, Debug)]
pub enum Step {
    /// Nothing new.
    Wait,
    /// Checks passed: mark the thread green, without a wake.
    Green,
    WakeCi,
    WakeReview,
    WakeConflict,
    /// Someone outside the repository commented: show it on the card, without a wake.
    Stranger(String),
    /// Stop waking the thread and show it as needing the person, for this reason.
    NeedsYou(String),
    /// Stop watching, for this reason.
    Stop(String),
}

/// A watch for a thread on `branch`, when `pr` is an open pull request from it. A
/// `plan` with no CI tracking (no remote, host unsupported, person turned it off,
/// custom URL) stops the watcher from starting at all.
pub fn start(pr: &PrStatus, branch: &str, plan: Option<&TrackingPlan>) -> Option<Watch> {
    if let Some(plan) = plan {
        if !matches!(plan.reason, TrackingReason::Watching) {
            return None;
        }
    }
    (pr.state == "OPEN" && !branch.is_empty() && pr.head_branch == branch).then(|| Watch {
        pr: pr.number,
        url: pr.url.clone(),
        head: pr.head.clone(),
        ..Default::default()
    })
}

/// Reviews and comments on `pr` from someone other than its author (the account the
/// thread pushes with) posted after `seen`. Bots' conversation comments, such as
/// coverage or preview links, don't count; their reviews do.
fn others_since<'a>(pr: &'a PrStatus, seen: &'a str) -> impl Iterator<Item = &'a Voice> {
    pr.voices
        .iter()
        .filter(|v| !v.author.is_empty() && v.author != pr.author)
        .filter(|v| v.review || !v.author.ends_with("[bot]"))
        .filter(move |v| v.at.as_str() > seen)
}

/// The time of the latest feedback since `seen` from the repository's people, which wakes
/// the thread.
pub fn new_feedback(pr: &PrStatus, seen: &str) -> Option<String> {
    others_since(pr, seen)
        .filter(|v| v.trusted)
        .map(|v| v.at.clone())
        .max()
}

/// The latest comment since `seen` from anyone else, as (who, when): shown, never sent.
pub fn new_stranger(pr: &PrStatus, seen: &str) -> Option<(String, String)> {
    others_since(pr, seen)
        .filter(|v| !v.trusted)
        .max_by(|a, b| a.at.cmp(&b.at))
        .map(|v| (v.author.clone(), v.at.clone()))
}

fn conflicts(pr: &PrStatus) -> bool {
    pr.mergeable == "CONFLICTING" || pr.merge_state == "DIRTY"
}

fn short(head: &str) -> &str {
    head.get(..7).unwrap_or(head)
}

/// Decide what one successful read of the pull request means. `busy` is true while the
/// thread is running, queued or waiting for a slot: nothing wakes it then, and feedback
/// stays unseen until it is free.
pub fn decide(w: &mut Watch, pr: &PrStatus, busy: bool) -> Step {
    w.failed_reads = 0;
    match pr.state.as_str() {
        "MERGED" => return Step::Stop(format!("#{} was merged", pr.number)),
        "CLOSED" => return Step::Stop(format!("#{} was closed", pr.number)),
        _ => {}
    }
    w.pr = pr.number;
    w.url = pr.url.clone();
    w.head = pr.head.clone();
    if busy || w.needs.is_some() {
        return Step::Wait;
    }
    if w.comment_wakes >= MAX_COMMENT_WAKES {
        return Step::Stop(format!(
            "{MAX_COMMENT_WAKES} wakes for comments on #{}",
            pr.number
        ));
    }
    if conflicts(pr) && w.conflict_head != pr.head {
        w.conflict_head = pr.head.clone();
        return Step::WakeConflict;
    }
    // Track whether this push produced any CI at all. External CI reports back a few
    // seconds after the push; two reads is enough to tell silence from "still spinning
    // up".
    if pr.pending == 0 && pr.passed == 0 && pr.failed.is_empty() {
        w.probe_reads = w.probe_reads.saturating_add(1);
        if w.probe_reads >= MAX_PROBE_READS {
            w.no_ci_detected = true;
        }
    } else {
        w.probe_reads = 0;
        w.no_ci_detected = false;
    }
    if !pr.failed.is_empty() {
        if w.ci_head != pr.head {
            w.ci_head = pr.head.clone();
            w.ci_wakes = 0;
        }
        let failed = pr.failed.join(", ");
        if w.ci_wakes >= MAX_CI_WAKES {
            let why = format!(
                "CI still fails on {} after {MAX_CI_WAKES} tries to fix it: {failed}",
                short(&pr.head)
            );
            w.needs = Some(why.clone());
            return Step::NeedsYou(why);
        }
        if w.ci_streak >= MAX_CI_STREAK {
            let why = format!(
                "CI has failed {MAX_CI_STREAK} times in a row on #{}: {failed}",
                pr.number
            );
            w.needs = Some(why.clone());
            return Step::NeedsYou(why);
        }
        w.ci_wakes += 1;
        w.ci_streak += 1;
        return Step::WakeCi;
    }
    if let Some(at) = new_feedback(pr, &w.seen) {
        w.seen = at;
        w.comment_wakes += 1;
        return Step::WakeReview;
    }
    if let Some((who, at)) = new_stranger(pr, &w.stranger_seen) {
        w.stranger_seen = at;
        w.stranger = Some(who.clone());
        return Step::Stranger(who);
    }
    if pr.pending == 0 && pr.passed > 0 && w.green_head != pr.head {
        w.green_head = pr.head.clone();
        w.ci_streak = 0;
        return Step::Green;
    }
    Step::Wait
}

/// Count a read that failed: gh couldn't run or answer, or the pull request is gone.
pub fn read_failed(w: &mut Watch) -> Option<Step> {
    w.failed_reads += 1;
    (w.failed_reads >= MAX_FAILED_READS).then(|| {
        Step::Stop(format!(
            "{MAX_FAILED_READS} reads of #{} failed in a row",
            w.pr
        ))
    })
}

/// How long until the next read: every two minutes, backing off to 16 after failures.
pub fn interval(failed_reads: u32) -> Duration {
    POLL * 2u32.pow(failed_reads.min(3))
}

/// The checks have all passed on the pull request's current head.
pub fn green(w: &Watch) -> bool {
    !w.head.is_empty() && w.green_head == w.head
}

/// What a watched thread's card says, and whether it reads as trouble, while `working`.
/// None when a working thread is on something other than a wake. `plan` tags labels
/// with the provider's own words ("GitLab pipeline green" rather than "CI green") and
/// handles the no-remote and unsupported-host cases.
pub fn label(w: &Watch, working: bool, plan: Option<&TrackingPlan>) -> Option<(String, bool)> {
    if let Some(why) = &w.needs {
        return Some((format!("Needs you: {why}"), true));
    }
    let ci_name = plan
        .and_then(|p| p.ci.map(|ci| ci.label().to_string()))
        .unwrap_or_else(|| "CI".into());
    let text = match (working, w.waking) {
        (true, Some(Wake::Ci)) => format!("{ci_name} failed, fixing"),
        (true, Some(Wake::Review)) => "Answering review".into(),
        (true, Some(Wake::Conflict)) => "Fixing conflicts".into(),
        (true, None) => return None,
        (false, _) if w.stranger.is_some() => {
            let who = w.stranger.as_deref().unwrap_or_default();
            return Some((format!("New comment from @{who}"), false));
        }
        (false, _) if green(w) => format!("{ci_name} green"),
        (false, _) => match plan.map(|p| &p.reason) {
            Some(TrackingReason::NoRemote) => "No remote, no CI tracking".into(),
            Some(TrackingReason::DisabledByProject) => "CI tracking off".into(),
            Some(TrackingReason::Custom) => "Custom CI".into(),
            Some(TrackingReason::HostUnsupported) => {
                let host = plan.map(|p| p.host).unwrap_or(Host::Unknown);
                format!("{} not supported yet", host.name())
            }
            Some(TrackingReason::NoCiConfigured) => "No CI configured".into(),
            Some(TrackingReason::Watching) if w.no_ci_detected => {
                "No CI detected on this push".into()
            }
            _ if w.no_ci_detected => "No CI detected on this push".into(),
            _ => format!("Watching {ci_name}"),
        },
    };
    Some((text, working && w.waking == Some(Wake::Ci)))
}

/// The prompt that wakes a thread whose branch conflicts with its base.
pub fn conflict_prompt(pr: &PrStatus) -> String {
    let base = if pr.base.is_empty() { "main" } else { &pr.base };
    format!(
        "Pull request #{} ({}) has merge conflicts with {base}. Fetch, rebase onto origin/{base}, resolve each conflict so both sides' intent survives, run `cargo check` or the project's build, and push with `--force-with-lease`. Then end your turn: Runo keeps watching the pull request.",
        pr.number, pr.url
    )
}

/// Said after a wake's own prompt, so the thread ends its turn again once it has pushed.
const END_TURN: &str = "\n\nPush when done and end your turn; don't wait for CI. Runo keeps watching the pull request and wakes you if something else needs doing.";

/// What the app keeps for the watcher while it runs; the watches themselves are saved
/// with their threads.
#[derive(Default)]
pub(crate) struct Polls {
    /// When each watched or probed thread's pull request is read next.
    next: HashMap<u64, Instant>,
    /// Threads that just ended a turn on a branch, read once to see if they opened a
    /// pull request.
    probing: std::collections::HashSet<u64>,
    /// The wall clock at the last tick, to notice the computer waking from sleep.
    last_tick: Option<SystemTime>,
}

impl RunoApp {
    /// A thread that is doing something, or about to, and mustn't be woken.
    fn watch_busy(&self, s: &Session) -> bool {
        self.is_running(s)
            || !s.queue.is_empty()
            || self.waiting_for_slot(s.id)
            || self.preparing.contains_key(&s.id)
            || self.git_busy.contains_key(&s.id)
    }

    /// A thread's turn ended: what it was woken for is done, and if it is on a branch of
    /// its own, read once to see whether it opened a pull request to watch.
    pub(crate) fn watch_turn_ended(&mut self, id: u64, ok: bool) {
        let Some(s) = self.state.session_mut(id) else {
            return;
        };
        let on_branch = !s.branch.is_empty()
            && s.branch != "local folder"
            && (s.base.is_empty() || s.branch != s.base);
        let Some(info) = s.crew.as_mut() else {
            return;
        };
        match info.watch.as_mut() {
            Some(w) => {
                w.waking = None;
                w.stranger = None;
            }
            None if ok && on_branch => {
                self.pr_watch.probing.insert(id);
            }
            None => return,
        }
        // Read soon, once the push the turn ended with has reached GitHub.
        self.pr_watch
            .next
            .insert(id, Instant::now() + Duration::from_secs(20));
    }

    /// Read the pull requests that are due. Runs every couple of seconds; reads only
    /// every two minutes a thread, and none while the computer sleeps (timers stop).
    pub(crate) fn pr_watch_tick(&mut self) {
        let now = Instant::now();
        let wall = SystemTime::now();
        // Back from sleep: give the network a moment before the reads that piled up.
        let slept = self
            .pr_watch
            .last_tick
            .and_then(|t| wall.duration_since(t).ok())
            .is_some_and(|gap| gap > Duration::from_secs(60));
        self.pr_watch.last_tick = Some(wall);
        if slept {
            for at in self.pr_watch.next.values_mut() {
                *at = (*at).max(now + Duration::from_secs(30));
            }
            return;
        }
        let due: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| !s.archived)
            .filter(|s| {
                s.crew
                    .as_ref()
                    .is_some_and(|i| i.watch.is_some() && !i.resolved)
                    || self.pr_watch.probing.contains(&s.id)
            })
            .map(|s| s.id)
            .collect();
        self.pr_watch.next.retain(|id, _| due.contains(id));
        for (n, id) in due.into_iter().enumerate() {
            // Watches saved from before a restart start a few seconds apart.
            let at = *self
                .pr_watch
                .next
                .entry(id)
                .or_insert(now + Duration::from_secs(10 + 3 * n as u64));
            if at <= now {
                // Pushed on again when the read comes back; this keeps a slow read from
                // being started twice.
                self.pr_watch.next.insert(id, now + POLL);
                self.load_pr_status(id);
            }
        }
    }

    /// One read of a thread's pull request came back (from `load_pr_status`).
    pub(crate) fn pr_watch_read(
        &mut self,
        id: u64,
        read: Result<Option<PrStatus>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.state.session(id) else {
            return;
        };
        let probing = self.pr_watch.probing.remove(&id);
        let plan = self.tracking_plan(s);
        let Some(mut watch) = s.crew.as_ref().and_then(|i| i.watch.clone()) else {
            // A thread that just ended a turn: watch its pull request if it opened one.
            if let (true, Ok(Some(pr))) = (probing, &read) {
                if let Some(w) = start(pr, &s.branch, plan.as_ref()) {
                    self.state
                        .log(id, format!("Watching CI on pull request #{}", w.pr));
                    self.set_watch(id, Some(w));
                    // Read again shortly: the checks have only just started.
                    self.pr_watch.next.insert(id, Instant::now() + POLL);
                } else {
                    self.pr_watch.next.remove(&id);
                }
            } else {
                self.pr_watch.next.remove(&id);
            }
            return;
        };
        let busy = self.watch_busy(s);
        let before = watch.clone();
        let step = match &read {
            Ok(Some(pr)) => decide(&mut watch, pr, busy),
            Ok(None) | Err(_) => read_failed(&mut watch).unwrap_or(Step::Wait),
        };
        self.pr_watch
            .next
            .insert(id, Instant::now() + interval(watch.failed_reads));
        let pr = read.ok().flatten();
        match step {
            // Most reads change nothing, and then nothing is saved.
            Step::Wait if watch == before => return,
            Step::Wait => self.set_watch(id, Some(watch)),
            Step::Green => {
                let n = watch.pr;
                self.state
                    .log(id, format!("CI passed on pull request #{n}"));
                self.set_watch(id, Some(watch));
                self.watch_news(id, format!("CI is green on #{n}."));
            }
            Step::Stop(why) => {
                self.state.log(id, format!("Stopped watching CI: {why}"));
                self.pr_watch.next.remove(&id);
                self.set_watch(id, None);
            }
            Step::Stranger(who) => {
                let n = watch.pr;
                self.state.log(
                    id,
                    format!("New comment from @{who} on #{n}; not sent to the thread, since they aren't one of the repository's people"),
                );
                self.set_watch(id, Some(watch));
                self.watch_news(
                    id,
                    format!("has a new comment from @{who} on #{n}. It wasn't sent to the thread: read it and pass on what should be."),
                );
            }
            Step::NeedsYou(why) => {
                self.state.log(id, format!("Watching CI needs you: {why}"));
                self.set_watch(id, Some(watch));
                self.watch_news(id, format!("needs you: {why}."));
            }
            Step::WakeCi | Step::WakeReview | Step::WakeConflict => {
                let Some(pr) = pr else { return };
                let kind = match step {
                    Step::WakeCi => Wake::Ci,
                    Step::WakeReview => Wake::Review,
                    _ => Wake::Conflict,
                };
                watch.waking = Some(kind);
                self.set_watch(id, Some(watch));
                self.watch_wake(id, kind, &pr, cx);
            }
        }
        self.save();
    }

    fn set_watch(&mut self, id: u64, watch: Option<Watch>) {
        if let Some(info) = self.state.session_mut(id).and_then(|s| s.crew.as_mut()) {
            info.watch = watch;
        }
    }

    /// A line about thread `id` in its project's Crew chat.
    pub(crate) fn watch_news(&mut self, id: u64, what: String) {
        let Some((pid, title)) = self
            .state
            .session(id)
            .map(|s| (s.project_id, s.title.clone()))
        else {
            return;
        };
        if let Some(chat) = self.crew_chat(pid).map(|c| c.id) {
            self.crew_note(
                chat,
                Role::System,
                format!("{title} {what}"),
                Some(crate::crew::Card::News { thread: id }),
            );
        }
    }

    /// Wake thread `id` for `kind` on `pr`.
    fn watch_wake(&mut self, id: u64, kind: Wake, pr: &PrStatus, cx: &mut Context<Self>) {
        match kind {
            Wake::Ci => {
                self.state.log(
                    id,
                    format!("CI failed on #{}: woke the thread to fix it", pr.number),
                );
                let prompt = format!("{}{END_TURN}", crate::ship::ci_prompt(pr));
                self.send_or_queue(id, prompt, cx);
            }
            Wake::Conflict => {
                self.state.log(
                    id,
                    format!("#{} conflicts with its base: woke the thread", pr.number),
                );
                self.send_or_queue(id, conflict_prompt(pr), cx);
            }
            // Reads the comments, then sends `review_prompt` (`address_review`).
            Wake::Review => self.address_review(id),
        }
    }

    /// The watch line for thread `s`: what it says, whether it reads as trouble, and the
    /// pull request it links to.
    pub(crate) fn watch_line(&self, s: &Session) -> Option<(String, bool, u64, String)> {
        let w = s.crew.as_ref()?.watch.as_ref()?;
        let plan = self.tracking_plan(s);
        let (text, trouble) = label(w, self.is_running(s), plan.as_ref())?;
        Some((text, trouble, w.pr, w.url.clone()))
    }

    /// What the CI watcher should do for a session's project: host, CI configs the
    /// repo has, and whether the person turned the watcher off here. Returns `None`
    /// when the session has no project path to read, so callers fall through to the
    /// plain "Watching CI" label.
    pub(crate) fn tracking_plan(&self, s: &Session) -> Option<TrackingPlan> {
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == s.project_id)?;
        let repo = if !s.dir.is_empty() && std::path::Path::new(&s.dir).is_dir() {
            s.dir.clone()
        } else if !project.path.is_empty() {
            project.path.clone()
        } else {
            return None;
        };
        let remote = git::remote_name(&repo);
        let url = if remote.is_empty() {
            String::new()
        } else {
            std::process::Command::new("git")
                .args(["-C", &repo, "remote", "get-url", &remote])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        };
        let host = provider::detect_host(&url);
        let configs = provider::detect_ci_configs(std::path::Path::new(&repo));
        let override_ = provider::CiTracking::parse(&project.ci_tracking);
        Some(provider::plan(host, &override_, configs))
    }

    /// A small linked chip for a watched thread: "Watching CI #123", "CI failed, fixing
    /// #123", "CI green #123" or "Needs you". Clicking it opens the pull request.
    pub(crate) fn watch_chip(
        &self,
        s: &Session,
        key: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (text, trouble, pr, url) = self.watch_line(s)?;
        let w = s.crew.as_ref()?.watch.as_ref()?;
        let tone = if w.needs.is_some() {
            c_amber()
        } else if trouble {
            c_red()
        } else if green(w) {
            c_green()
        } else {
            c_blue()
        };
        let text = if w.needs.is_some() {
            "Needs you".to_string()
        } else {
            text
        };
        let tip = w.needs.clone();
        Some(
            div()
                .id(SharedString::from(format!("watch-{key}-{}", s.id)))
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .px(px(6.))
                .rounded_full()
                .bg(rgba((tone << 8) | 0x22))
                .text_size(px(10.5))
                .text_color(rgb(tone))
                .whitespace_nowrap()
                .cursor_pointer()
                .hover(|el| el.underline())
                .child(format!("{text} #{pr}"))
                .when_some(tip, |el, tip| {
                    el.tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&url);
                }))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Voice;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    fn pr() -> PrStatus {
        PrStatus {
            number: 7,
            url: "https://github.com/o/r/pull/7".into(),
            state: "OPEN".into(),
            head_branch: "runo/x".into(),
            head: "aaaaaaa1".into(),
            author: "me".into(),
            mergeable: "MERGEABLE".into(),
            ..Default::default()
        }
    }

    fn failing(head: &str) -> PrStatus {
        PrStatus {
            head: head.into(),
            failed: vec!["test (ubuntu)".into()],
            passed: 1,
            ..pr()
        }
    }

    fn voice(author: &str, at: &str, review: bool) -> Voice {
        Voice {
            author: author.into(),
            at: at.into(),
            review,
            trusted: true,
        }
    }

    #[test]
    fn watches_only_an_open_pull_request_from_the_threads_branch() {
        assert_eq!(start(&pr(), "runo/x", None).map(|w| w.pr), Some(7));
        assert!(start(&pr(), "runo/other", None).is_none());
        assert!(start(&pr(), "", None).is_none());
        let merged = PrStatus {
            state: "MERGED".into(),
            ..pr()
        };
        assert!(start(&merged, "runo/x", None).is_none());
        // A plan that says the project isn't being watched (no remote, host unsupported,
        // disabled) stops the watcher from starting even for a valid pull request.
        let off = TrackingPlan {
            host: Host::None,
            ci: None,
            configs: Vec::new(),
            reason: TrackingReason::NoRemote,
            custom_url: None,
        };
        assert!(start(&pr(), "runo/x", Some(&off)).is_none());
        let unsupported = TrackingPlan {
            host: Host::BitbucketServer,
            reason: TrackingReason::HostUnsupported,
            ..off.clone()
        };
        assert!(start(&pr(), "runo/x", Some(&unsupported)).is_none());
    }

    #[test]
    fn a_failed_check_wakes_the_thread() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        assert_eq!(decide(&mut w, &failing("a1"), false), Step::WakeCi);
        assert_eq!((w.ci_wakes, w.ci_streak), (1, 1));
        // Still running or queued: nothing wakes it, nothing is counted.
        assert_eq!(decide(&mut w, &failing("a1"), true), Step::Wait);
        assert_eq!(w.ci_wakes, 1);
        // Running checks don't wake it.
        let running = PrStatus { pending: 2, ..pr() };
        assert_eq!(decide(&mut w, &running, false), Step::Wait);
    }

    #[test]
    fn ci_wakes_stop_at_three_a_head_then_need_the_person() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        for _ in 0..MAX_CI_WAKES {
            assert_eq!(decide(&mut w, &failing("a1"), false), Step::WakeCi);
        }
        let step = decide(&mut w, &failing("a1"), false);
        assert!(matches!(step, Step::NeedsYou(ref why) if why.contains("after 3 tries")));
        assert!(w.needs.is_some());
        // Needing the person, it isn't woken again, even for a new head.
        assert_eq!(decide(&mut w, &failing("b2"), false), Step::Wait);
    }

    #[test]
    fn a_new_head_restarts_the_count_but_the_streak_caps_it() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        for head in ["h1", "h2", "h3", "h4", "h5", "h6"] {
            assert_eq!(decide(&mut w, &failing(head), false), Step::WakeCi);
            assert_eq!(w.ci_wakes, 1);
        }
        assert!(matches!(
            decide(&mut w, &failing("h7"), false),
            Step::NeedsYou(_)
        ));
    }

    #[test]
    fn green_marks_the_card_once_and_resets_the_streak() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        assert_eq!(decide(&mut w, &failing("h1"), false), Step::WakeCi);
        let passed = PrStatus {
            head: "h2".into(),
            passed: 3,
            ..pr()
        };
        assert_eq!(decide(&mut w, &passed, false), Step::Green);
        assert!(green(&w));
        assert_eq!(w.ci_streak, 0);
        assert_eq!(decide(&mut w, &passed, false), Step::Wait);
        assert_eq!(label(&w, false, None).unwrap().0, "CI green");
        // No checks at all isn't green.
        let mut w = start(&pr(), "runo/x", None).unwrap();
        assert_eq!(decide(&mut w, &pr(), false), Step::Wait);
        assert_eq!(label(&w, false, None).unwrap().0, "Watching CI");
    }

    #[test]
    fn feedback_from_someone_else_wakes_but_not_the_threads_own() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        let mut p = pr();
        p.voices = vec![
            voice("me", "2026-10-25T10:00:00Z", false),
            voice("ci-preview[bot]", "2026-10-25T10:01:00Z", false),
        ];
        assert_eq!(decide(&mut w, &p, false), Step::Wait);
        p.voices.push(voice("ann", "2026-10-25T10:02:00Z", true));
        assert_eq!(decide(&mut w, &p, false), Step::WakeReview);
        assert_eq!(
            (w.seen.as_str(), w.comment_wakes),
            ("2026-10-25T10:02:00Z", 1)
        );
        // Already sent.
        assert_eq!(decide(&mut w, &p, false), Step::Wait);
        // A bot's review counts.
        p.voices
            .push(voice("copilot[bot]", "2026-10-25T10:03:00Z", true));
        assert_eq!(decide(&mut w, &p, false), Step::WakeReview);
        // Feedback that came while the thread was busy waits, then wakes.
        p.voices.push(voice("bo", "2026-10-25T10:04:00Z", false));
        assert_eq!(decide(&mut w, &p, true), Step::Wait);
        assert_eq!(decide(&mut w, &p, false), Step::WakeReview);
    }

    #[test]
    fn a_strangers_comment_shows_on_the_card_but_never_wakes() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        let mut p = pr();
        p.voices = vec![Voice {
            trusted: false,
            ..voice("eve", "2026-10-25T10:00:00Z", false)
        }];
        assert_eq!(decide(&mut w, &p, false), Step::Stranger("eve".into()));
        assert_eq!(w.comment_wakes, 0);
        assert_eq!(
            label(&w, false, None),
            Some(("New comment from @eve".into(), false))
        );
        // Shown once.
        assert_eq!(decide(&mut w, &p, false), Step::Wait);
        // A collaborator's comment still wakes it.
        p.voices.push(voice("ann", "2026-10-25T10:01:00Z", false));
        assert_eq!(decide(&mut w, &p, false), Step::WakeReview);
    }

    #[test]
    fn comment_wakes_stop_after_ten() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        let mut p = pr();
        for n in 0..MAX_COMMENT_WAKES {
            p.voices
                .push(voice("ann", &format!("2026-10-25T10:{n:02}:00Z"), false));
            assert_eq!(decide(&mut w, &p, false), Step::WakeReview);
        }
        p.voices.push(voice("ann", "2026-10-25T11:00:00Z", false));
        assert!(matches!(decide(&mut w, &p, false), Step::Stop(_)));
    }

    #[test]
    fn a_conflict_wakes_once_a_head() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        let p = PrStatus {
            mergeable: "CONFLICTING".into(),
            ..pr()
        };
        assert_eq!(decide(&mut w, &p, false), Step::WakeConflict);
        assert_eq!(decide(&mut w, &p, false), Step::Wait);
        let dirty = PrStatus {
            head: "bbbbbbb2".into(),
            merge_state: "DIRTY".into(),
            ..pr()
        };
        assert_eq!(decide(&mut w, &dirty, false), Step::WakeConflict);
        assert!(conflict_prompt(&dirty).contains("--force-with-lease"));
    }

    #[test]
    fn merge_or_close_stops_watching_even_while_busy() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        let merged = PrStatus {
            state: "MERGED".into(),
            ..pr()
        };
        assert!(
            matches!(decide(&mut w, &merged, true), Step::Stop(ref why) if why.contains("merged"))
        );
        let closed = PrStatus {
            state: "CLOSED".into(),
            ..pr()
        };
        assert!(
            matches!(decide(&mut w, &closed, false), Step::Stop(ref why) if why.contains("closed"))
        );
    }

    #[test]
    fn failed_reads_back_off_and_stop_after_eight_in_a_row() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        for n in 1..MAX_FAILED_READS {
            assert_eq!(read_failed(&mut w), None);
            assert_eq!(w.failed_reads, n);
        }
        // A good read clears the run.
        decide(&mut w, &pr(), false);
        assert_eq!(w.failed_reads, 0);
        for _ in 1..MAX_FAILED_READS {
            read_failed(&mut w);
        }
        assert!(matches!(read_failed(&mut w), Some(Step::Stop(_))));
        assert_eq!(interval(0), POLL);
        assert_eq!(interval(1), POLL * 2);
        assert_eq!(interval(7), POLL * 8);
    }

    #[test]
    fn labels_follow_the_wake() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        assert_eq!(label(&w, false, None), Some(("Watching CI".into(), false)));
        w.waking = Some(Wake::Ci);
        assert_eq!(label(&w, true, None), Some(("CI failed, fixing".into(), true)));
        w.waking = None;
        assert_eq!(label(&w, true, None), None);
        w.needs = Some("CI still fails".into());
        assert_eq!(label(&w, false, None).map(|l| l.1), Some(true));
    }

    #[test]
    fn labels_use_the_providers_own_words() {
        let w = start(&pr(), "runo/x", None).unwrap();
        let plan = TrackingPlan {
            host: Host::Gitlab,
            ci: Some(crate::provider::CiKind::GitlabPipelines),
            configs: vec![crate::provider::CiKind::GitlabPipelines],
            reason: TrackingReason::Watching,
            custom_url: None,
        };
        assert_eq!(
            label(&w, false, Some(&plan)).map(|l| l.0),
            Some("Watching GitLab Pipelines".into())
        );
        let unsupported = TrackingPlan {
            host: Host::BitbucketServer,
            ci: None,
            configs: Vec::new(),
            reason: TrackingReason::HostUnsupported,
            custom_url: None,
        };
        assert_eq!(
            label(&w, false, Some(&unsupported)).map(|l| l.0),
            Some("Bitbucket Server not supported yet".into())
        );
    }

    #[test]
    fn two_reads_without_ci_mark_the_push_as_no_ci() {
        let mut w = start(&pr(), "runo/x", None).unwrap();
        // First read with no CI info: still probing.
        assert_eq!(decide(&mut w, &pr(), false), Step::Wait);
        assert_eq!((w.probe_reads, w.no_ci_detected), (1, false));
        // Second read in a row: push produced no CI.
        assert_eq!(decide(&mut w, &pr(), false), Step::Wait);
        assert!(w.no_ci_detected);
        assert_eq!(
            label(&w, false, None).map(|l| l.0),
            Some("No CI detected on this push".into())
        );
        // A later read that finds a check clears the state.
        let running = PrStatus { pending: 1, ..pr() };
        assert_eq!(decide(&mut w, &running, false), Step::Wait);
        assert_eq!((w.probe_reads, w.no_ci_detected), (0, false));
    }
}
