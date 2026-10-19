//! Keeping an eye on what the person isn't watching: local turns picked up after a
//! restart, agents gone quiet, spending, turn and token limits, the cap on agents running at once, crash
//! reports and newer Runo releases. Also saves the state a moment after it changes.

use super::*;
use crate::applog::{Crash, Release};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// How often Runo looks for a newer release of itself.
const RELEASE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// A Runo download: running, saved to a file, or failed.
#[derive(Clone)]
pub(crate) enum Download {
    Running,
    Saved(PathBuf),
    Failed(String),
}

#[derive(Default)]
pub(crate) struct Health {
    /// The state changed and is saved on the next tick.
    pub(crate) save_due: bool,
    /// When each running session last printed anything.
    last_output: HashMap<u64, Instant>,
    /// Sessions whose quiet-agent card was hidden, until they print again.
    quiet_hidden: HashSet<u64>,
    /// Sessions whose turns wait on a limit, with which and why.
    held: HashMap<u64, Hold>,
    /// Sessions the person let past their limit for one turn.
    allow_once: HashSet<u64>,
    /// Sessions waiting for a free slot, first come first served.
    waiting_slot: Vec<u64>,
    crash: Option<Crash>,
    crash_looked: bool,
    release: Arc<Mutex<Option<Release>>>,
    release_checked: Option<Instant>,
    download: Arc<Mutex<Option<Download>>>,
}

impl Health {
    /// Note output from a session's agent, or that its run ended.
    pub(crate) fn saw(&mut self, event: &Event) {
        match event {
            Event::Line(Target::Session(id), _) | Event::Err(Target::Session(id), _) => {
                self.last_output.insert(*id, Instant::now());
                self.quiet_hidden.remove(id);
            }
            Event::Done(Target::Session(id), _) => {
                self.last_output.remove(id);
                self.quiet_hidden.remove(id);
            }
            _ => {}
        }
    }
}

/// What a session's agents have cost so far, in US cents.
pub(crate) fn session_cost(session: &Session) -> f64 {
    session.messages.iter().map(|m| m.usage.cost_cents).sum()
}

/// What counts toward the limits for a session's next turn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Used {
    /// What the session's agents cost, in US cents.
    pub(crate) session_cents: f64,
    /// What agents cost today, in US cents.
    pub(crate) day_cents: f64,
    /// Turns the session has had.
    pub(crate) session_turns: u32,
    /// Turns started today, across every session.
    pub(crate) day_turns: u32,
    /// Tokens agents reported today.
    pub(crate) day_tokens: u64,
}

/// A limit holding new turns: which kind, for the card's title, and why.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hold {
    pub(crate) title: &'static str,
    pub(crate) why: String,
}

/// Turns a session has had: one for each prompt sent to its agent.
pub(crate) fn session_turns(session: &Session) -> u32 {
    let turns = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .count();
    u32::try_from(turns).unwrap_or(u32::MAX)
}

/// The tokens a finished turn counts toward the daily token limit: input and output,
/// without the cached prompt, which would count the same context again on every turn.
pub(crate) fn turn_tokens(usage: &store::Usage) -> u64 {
    usage.input.saturating_add(usage.output)
}

/// A token count the way people read it: 950, 12k, 1.5M.
pub(crate) fn token_count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{}k", n / 1_000),
        _ => {
            let m = format!("{:.1}", n as f64 / 1e6);
            format!("{}M", m.strip_suffix(".0").unwrap_or(&m))
        }
    }
}

/// Why a new turn must wait for a limit, if it must. 0 means no limit.
pub(crate) fn over_limit(used: &Used, limits: &settings::Limits) -> Option<Hold> {
    let dollars = |cents: f64| format!("${:.2}", cents / 100.);
    let plural = |n: u32| if n == 1 { "" } else { "s" };
    let hold = |title, why| Some(Hold { title, why });
    if limits.session_cents > 0. && used.session_cents >= limits.session_cents {
        hold(
            "Spending limit reached",
            format!(
                "This session has cost {} of its {} limit.",
                dollars(used.session_cents),
                dollars(limits.session_cents)
            ),
        )
    } else if limits.day_cents > 0. && used.day_cents >= limits.day_cents {
        hold(
            "Spending limit reached",
            format!(
                "Agents have cost {} today, reaching the daily limit of {}.",
                dollars(used.day_cents),
                dollars(limits.day_cents)
            ),
        )
    } else if limits.session_turns > 0 && used.session_turns >= limits.session_turns {
        hold(
            "Turn limit reached",
            format!(
                "Held: {} turn{} in this session (limit {}).",
                used.session_turns,
                plural(used.session_turns),
                limits.session_turns
            ),
        )
    } else if limits.day_turns > 0 && used.day_turns >= limits.day_turns {
        hold(
            "Turn limit reached",
            format!(
                "Held: {} turn{} today (limit {}).",
                used.day_turns,
                plural(used.day_turns),
                limits.day_turns
            ),
        )
    } else if limits.day_tokens > 0 && used.day_tokens >= limits.day_tokens {
        hold(
            "Token limit reached",
            format!(
                "Held: {} tokens today (limit {}).",
                token_count(used.day_tokens),
                token_count(limits.day_tokens)
            ),
        )
    } else {
        None
    }
}

impl RunoApp {
    /// Save now rather than on the next tick: the window is closing or Runo quitting.
    pub(crate) fn save_now(&mut self) {
        self.health.save_due = false;
        if !self.demo {
            self.store_layout();
            self.state.save();
        }
    }

    /// Agents running on this machine or elsewhere, for the cap.
    fn running(&self) -> usize {
        self.handles
            .keys()
            .filter(|t| matches!(t, Target::Session(_)))
            .count()
    }

    /// What counts toward the limits for the next turn of `session`.
    fn used(&self, session: &Session) -> Used {
        let today = store::today();
        let day = self.state.used_on(&today);
        Used {
            session_cents: session_cost(session),
            day_cents: self.state.spend.get(&today).copied().unwrap_or_default(),
            session_turns: session_turns(session),
            day_turns: day.turns,
            day_tokens: day.tokens,
        }
    }

    /// Why a new turn of `id` has to wait (a spending, turn or token limit, or no free
    /// slot), noting it so the turn starts once it can. None lets it start.
    pub(crate) fn hold_turn(&mut self, id: u64) -> Option<String> {
        if self.demo {
            return None;
        }
        let session = self.state.session(id)?;
        if !self.health.allow_once.contains(&id) {
            if let Some(hold) = over_limit(&self.used(session), &settings::limits()) {
                let text = format!("Not sent. {}", hold.why);
                self.health.held.insert(id, hold);
                return Some(text);
            }
        }
        let cap = settings::max_running();
        if cap > 0 && self.running() >= cap {
            if !self.health.waiting_slot.contains(&id) {
                self.health.waiting_slot.push(id);
            }
            return Some(format!(
                "Waiting for a slot: {cap} agent{} already running.",
                if cap == 1 { " is" } else { "s are" }
            ));
        }
        self.health.allow_once.remove(&id);
        self.health.held.remove(&id);
        self.health.waiting_slot.retain(|w| *w != id);
        None
    }

    /// What holds a session's queued prompts, for the composer.
    pub(crate) fn held_reason(&self, id: u64) -> Option<(bool, String)> {
        if let Some(hold) = self.health.held.get(&id) {
            return Some((true, hold.why.clone()));
        }
        let place = self.health.waiting_slot.iter().position(|w| *w == id)?;
        Some((
            false,
            match place {
                0 => "Waiting for a slot. It starts when a running agent finishes.".into(),
                n => format!("Waiting for a slot, {} in line.", n + 1),
            },
        ))
    }

    /// Send one held turn past its limit.
    pub(crate) fn send_anyway(&mut self, id: u64, cx: &mut Context<Self>) {
        self.health.allow_once.insert(id);
        self.health.held.remove(&id);
        self.send_queued(id, 0, cx);
    }

    /// Local turns that were running when Runo last closed kept going. Follow them again:
    /// their output replays from the start and they end as usual.
    pub(crate) fn resume_local_turns(&mut self) {
        // The sample workspace's running session is a picture; there is nothing to follow.
        if self.demo {
            return;
        }
        let Some(root) = crate::detach::root() else {
            return;
        };
        let ids: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.status == Status::Working && !s.remote_turn)
            .filter(|s| !self.handles.contains_key(&Target::Session(s.id)))
            .map(|s| s.id)
            .collect();
        let mut picked = 0;
        for id in ids {
            let Some(session) = self.state.session_mut(id) else {
                continue;
            };
            match crate::detach::attach(root, id, self.tx.clone()) {
                Ok(handle) => {
                    // The replay rebuilds everything after the prompt.
                    let start = session
                        .messages
                        .iter()
                        .rposition(|m| m.role == Role::User)
                        .map_or(session.messages.len(), |i| i + 1);
                    session.messages.truncate(start);
                    session.messages.push(Message::now(Role::Agent, ""));
                    if let Some(parser) =
                        stream::Parser::for_provider(&session.provider, &session.dir)
                    {
                        self.streams.insert(id, parser);
                    }
                    self.handles.insert(Target::Session(id), handle);
                    self.started.insert(id, Instant::now());
                    picked += 1;
                }
                Err(e) => {
                    crate::applog::warn(
                        "turn",
                        format!(
                            "session #{id} ({}): couldn't pick up the turn left running: {e}",
                            session.provider
                        ),
                    );
                    session.status = Status::Failed;
                    session.messages.push(Message::new(
                        Role::System,
                        format!("Interrupted: Runo couldn't pick up the running turn ({e})."),
                    ));
                }
            }
        }
        if picked > 0 {
            self.info = Some((
                format!(
                    "Picked up {picked} turn{} that kept running while Runo was closed.",
                    if picked == 1 { "" } else { "s" }
                ),
                Instant::now(),
            ));
            self.keep_awake_update();
        }
        self.save();
    }

    /// Every half second: save if needed, start turns that waited for a slot or a raised
    /// limit, and look for crash reports and Runo updates.
    pub(crate) fn health_tick(&mut self, cx: &mut Context<Self>) {
        if self.health.save_due {
            self.save_now();
        }
        if !self.demo {
            self.start_waiting(cx);
        }
        if !self.health.crash_looked {
            self.health.crash_looked = true;
            self.health.crash = crate::applog::unseen_crash(&crate::applog::logs_dir());
        }
        if cx.global::<AppSettings>().check_runo_updates
            && self
                .health
                .release_checked
                .is_none_or(|at| at.elapsed() > RELEASE_EVERY)
        {
            self.health.release_checked = Some(Instant::now());
            let slot = self.health.release.clone();
            std::thread::spawn(move || {
                if let Some(release) = crate::applog::check_release() {
                    if let Ok(mut slot) = slot.lock() {
                        *slot = Some(release);
                    }
                }
            });
        }
    }

    /// Start turns that waited for a free slot, in order, and those held by a limit that
    /// has since been raised.
    fn start_waiting(&mut self, cx: &mut Context<Self>) {
        let held: Vec<u64> = self.health.held.keys().copied().collect();
        for id in held {
            let free = self.state.session(id).is_none_or(|s| {
                s.queue.is_empty() || over_limit(&self.used(s), &settings::limits()).is_none()
            });
            if free {
                self.health.held.remove(&id);
                self.send_queued(id, 0, cx);
                cx.notify();
            }
        }
        let cap = settings::max_running();
        while let Some(&id) = self.health.waiting_slot.first() {
            if cap > 0 && self.running() >= cap {
                break;
            }
            self.health.waiting_slot.remove(0);
            let ready = self.state.session(id).is_some_and(|s| !s.queue.is_empty())
                && !self.handles.contains_key(&Target::Session(id));
            if ready {
                self.send_queued(id, 0, cx);
                cx.notify();
            }
        }
    }

    /// Sessions whose agent has printed nothing for longer than the setting allows.
    fn quiet_sessions(&self) -> Vec<(u64, u64)> {
        let mins = settings::stuck_after_mins();
        if mins == 0 {
            return Vec::new();
        }
        let mut quiet: Vec<(u64, u64)> = self
            .handles
            .keys()
            .filter_map(|t| match t {
                Target::Session(id) => Some(*id),
                _ => None,
            })
            .filter(|id| !self.health.quiet_hidden.contains(id))
            // An agent waiting on a permission answer is waiting on the person.
            .filter(|id| self.approval_for(*id).is_none())
            .filter_map(|id| {
                let since = self
                    .health
                    .last_output
                    .get(&id)
                    .or_else(|| self.started.get(&id))?;
                let secs = since.elapsed().as_secs();
                (secs >= u64::from(mins) * 60).then_some((id, secs / 60))
            })
            .collect();
        quiet.sort_unstable();
        quiet
    }

    /// Cards for the corner of the window: quiet agents, limits reached, a crash last
    /// time, and a newer Runo.
    pub(crate) fn health_cards(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut cards = Vec::new();
        for (n, (id, mins)) in self.quiet_sessions().into_iter().enumerate() {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let actions = div()
                .flex()
                .gap_2()
                .child(
                    self.button(("quiet-stop", n), "Stop", true)
                        .h(px(26.))
                        .child(icon(IconName::Square, 12., c_canvas()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.stop_session(id);
                            cx.notify();
                        })),
                )
                .child(
                    self.button(("quiet-wait", n), "Keep waiting", false)
                        .h(px(26.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.health.quiet_hidden.insert(id);
                            cx.notify();
                        })),
                );
            cards.push(self.alert_card(
                SharedString::from(format!("alert-quiet-{id}")),
                c_amber(),
                IconName::Clock,
                "Agent may be stuck",
                Some(session.title.clone()),
                Some(self.provider_badge(provider, 18.)),
                format!(
                    "No output for {mins} minute{}. It may be waiting on something, or busy with a long command.",
                    if mins == 1 { "" } else { "s" }
                ),
                actions,
                format!("quiet:{id}:{mins}"),
                Some(id),
                cx,
            ));
        }
        let mut held: Vec<(&u64, &Hold)> = self.health.held.iter().collect();
        held.sort_by_key(|(id, _)| **id);
        for (n, (&id, hold)) in held.into_iter().enumerate() {
            let why = &hold.why;
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let key = format!("limit:{id}:{why}");
            if self.alerts_hidden.contains(&key) {
                continue;
            }
            let provider = agent::provider(&session.provider);
            let actions = div()
                .flex()
                .gap_2()
                .child(
                    self.button(("limit-send", n), "Send anyway", false)
                        .h(px(26.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.send_anyway(id, cx);
                            cx.notify();
                        })),
                )
                .child(
                    self.button(("limit-raise", n), "Change limits", true)
                        .h(px(26.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_settings();
                            this.settings_page = SettingsPage::General;
                            cx.notify();
                        })),
                );
            cards.push(self.alert_card(
                SharedString::from(format!("alert-limit-{id}")),
                c_amber(),
                IconName::TriangleAlert,
                hold.title,
                Some(session.title.clone()),
                Some(self.provider_badge(provider, 18.)),
                format!("{why} Its next prompt is queued."),
                actions,
                key,
                Some(id),
                cx,
            ));
        }
        if let Some(crash) = self.health.crash.clone() {
            let key = format!("crash:{}", crash.path.display());
            if !self.alerts_hidden.contains(&key) {
                let (view, folder, done) = (crash.path.clone(), crash.path.clone(), crash.clone());
                let actions = div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        self.button("crash-view", "View report", true)
                            .h(px(26.))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&view))),
                    )
                    .child(
                        self.button("crash-folder", "Show in folder", false)
                            .h(px(26.))
                            .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&folder))),
                    )
                    .child(
                        self.button("crash-issue", "Report", false)
                            .h(px(26.))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.open_url(&format!(
                                    "https://github.com/{}/issues/new",
                                    crate::applog::REPO
                                ))
                            })),
                    )
                    .child(
                        self.button("crash-done", "Dismiss", false)
                            .h(px(26.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                crate::applog::dismiss_crash(&done);
                                this.health.crash = None;
                                cx.notify();
                            })),
                    );
                cards.push(self.alert_card(
                    SharedString::from("alert-crash"),
                    c_red(),
                    IconName::TriangleAlert,
                    if crash.quit {
                        "Runo quit unexpectedly"
                    } else {
                        "Runo hit an error"
                    },
                    None,
                    None,
                    if crash.summary.is_empty() {
                        "A crash report was saved last time.".to_string()
                    } else {
                        format!("A crash report was saved last time: {}", crash.summary)
                    },
                    actions,
                    key,
                    None,
                    cx,
                ));
            }
        }
        if let Some(card) = self.release_card(cx) {
            cards.push(card);
        }
        cards
    }

    fn release_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let release = self.health.release.lock().ok()?.clone()?;
        let key = format!("runo:{}", release.version);
        let download = self.health.download.lock().ok()?.clone();
        if self.alerts_hidden.contains(&key) && !matches!(download, Some(Download::Running)) {
            return None;
        }
        let page = release.page.clone();
        let notes = self
            .button("runo-notes", "What's new", false)
            .h(px(26.))
            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&page)));
        let (title, body, actions) = match download {
            Some(Download::Running) => (
                "Downloading Runo",
                format!("Downloading version {}.", release.version),
                div().flex(),
            ),
            Some(Download::Saved(path)) => {
                let (open, folder) = (path.clone(), path.clone());
                (
                    "Runo update downloaded",
                    format!(
                        "Saved {} to {}. Open it and replace Runo to finish; your sessions stay.",
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        path.parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    ),
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            self.button("runo-open", "Open", true).h(px(26.)).on_click(
                                cx.listener(move |_, _, _, cx| cx.open_with_system(&open)),
                            ),
                        )
                        .child(
                            self.button("runo-folder", "Show in folder", false)
                                .h(px(26.))
                                .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&folder))),
                        ),
                )
            }
            other => {
                let failed = match other {
                    Some(Download::Failed(why)) => Some(why),
                    _ => None,
                };
                let mut actions = div().flex().gap_2();
                if let Some((name, url)) = release.asset.clone() {
                    let slot = self.health.download.clone();
                    actions = actions.child(
                        self.button(
                            "runo-download",
                            if failed.is_some() {
                                "Try again"
                            } else {
                                "Download"
                            },
                            true,
                        )
                        .h(px(26.))
                        .child(icon(IconName::Download, 12., c_canvas()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if let Ok(mut s) = slot.lock() {
                                *s = Some(Download::Running);
                            }
                            let (slot, name, url) = (slot.clone(), name.clone(), url.clone());
                            std::thread::spawn(move || {
                                let done = crate::applog::download(
                                    &name,
                                    &url,
                                    &crate::applog::downloads_dir(),
                                );
                                if let Ok(mut s) = slot.lock() {
                                    *s = Some(match done {
                                        Ok(path) => Download::Saved(path),
                                        Err(why) => Download::Failed(why),
                                    });
                                }
                            });
                            cx.notify();
                        })),
                    );
                }
                actions = actions.child(notes);
                (
                    if failed.is_some() {
                        "Runo download failed"
                    } else {
                        "Runo update available"
                    },
                    failed.unwrap_or_else(|| {
                        format!(
                            "Version {} is out. You have {}.",
                            release.version,
                            env!("CARGO_PKG_VERSION")
                        )
                    }),
                    actions,
                )
            }
        };
        Some(self.alert_card(
            SharedString::from("alert-runo-update"),
            c_green(),
            IconName::Download,
            title,
            None,
            None,
            body,
            actions,
            key,
            None,
            cx,
        ))
    }

    /// The composer line saying why queued prompts are waiting, with Send anyway for a
    /// limit.
    pub(crate) fn held_line(&self, id: u64, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (limit, text) = self.held_reason(id)?;
        Some(
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .bg(tint(c_subtle()))
                .text_size(px(12.5))
                .child(icon(
                    if limit {
                        IconName::TriangleAlert
                    } else {
                        IconName::Clock
                    },
                    12.,
                    c_amber(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(c_secondary()))
                        .child(text),
                )
                .when(limit, |el| {
                    el.child(
                        self.button("held-send", "Send anyway", false)
                            .h(px(22.))
                            .px_2()
                            .text_size(px(12.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.send_anyway(id, cx);
                                cx.notify();
                            })),
                    )
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn why(used: Used, limits: settings::Limits) -> Option<String> {
        over_limit(&used, &limits).map(|h| h.why)
    }

    #[test]
    fn limits_hold_turns_once_reached() {
        let spent = |session_cents, day_cents| Used {
            session_cents,
            day_cents,
            ..Used::default()
        };
        let cost = |session_cents, day_cents| settings::Limits {
            session_cents,
            day_cents,
            ..settings::Limits::default()
        };
        assert_eq!(over_limit(&spent(500., 900.), &cost(0., 0.)), None);
        let hold = over_limit(&spent(500., 900.), &cost(500., 0.)).unwrap();
        assert_eq!(hold.title, "Spending limit reached");
        assert!(hold.why.contains("$5.00 of its $5.00"));
        assert!(why(spent(499., 900.), cost(500., 0.)).is_none());
        let why = why(spent(10., 1000.), cost(500., 1000.)).unwrap();
        assert!(why.contains("daily limit of $10.00"));
    }

    #[test]
    fn turn_and_token_limits_hold_every_agent() {
        let none = settings::Limits::default();
        let busy = Used {
            session_turns: 50,
            day_turns: 500,
            day_tokens: 90_000_000,
            ..Used::default()
        };
        assert_eq!(over_limit(&busy, &none), None, "all off by default");

        let per_session = settings::Limits {
            session_turns: 20,
            ..none
        };
        let mut used = Used {
            session_turns: 19,
            ..Used::default()
        };
        assert_eq!(over_limit(&used, &per_session), None);
        used.session_turns = 20;
        let hold = over_limit(&used, &per_session).unwrap();
        assert_eq!(hold.title, "Turn limit reached");
        assert_eq!(hold.why, "Held: 20 turns in this session (limit 20).");

        let per_day = settings::Limits {
            day_turns: 20,
            ..none
        };
        let mut used = Used {
            day_turns: 19,
            session_turns: 3,
            ..Used::default()
        };
        assert_eq!(over_limit(&used, &per_day), None);
        used.day_turns = 20;
        assert_eq!(
            why(used, per_day).as_deref(),
            Some("Held: 20 turns today (limit 20).")
        );
        // A limit of one reads as one turn.
        let one = settings::Limits {
            day_turns: 1,
            ..none
        };
        let used = Used {
            day_turns: 1,
            ..Used::default()
        };
        assert_eq!(
            why(used, one).as_deref(),
            Some("Held: 1 turn today (limit 1).")
        );

        let tokens = settings::Limits {
            day_tokens: 1_000_000,
            ..none
        };
        let mut used = Used {
            day_tokens: 999_999,
            ..Used::default()
        };
        assert_eq!(over_limit(&used, &tokens), None);
        used.day_tokens = 1_240_000;
        let hold = over_limit(&used, &tokens).unwrap();
        assert_eq!(hold.title, "Token limit reached");
        assert_eq!(hold.why, "Held: 1.2M tokens today (limit 1M).");

        // Spending is named first when several limits are reached.
        let all = settings::Limits {
            session_cents: 100.,
            session_turns: 1,
            day_turns: 1,
            day_tokens: 1,
            ..none
        };
        let used = Used {
            session_cents: 100.,
            ..busy
        };
        assert_eq!(
            over_limit(&used, &all).unwrap().title,
            "Spending limit reached"
        );
    }

    #[test]
    fn a_session_counts_its_turns_and_a_turn_its_tokens() {
        let mut session = State::demo().sessions[0].clone();
        session.messages.clear();
        assert_eq!(session_turns(&session), 0);
        session.messages.push(Message::new(Role::User, "one"));
        session.messages.push(Message::new(Role::Agent, "a"));
        session.messages.push(Message::new(Role::System, "note"));
        session.messages.push(Message::new(Role::User, "two"));
        assert_eq!(session_turns(&session), 2);
        let usage = store::Usage {
            input: 1_000,
            output: 250,
            cached: 90_000,
            ..store::Usage::default()
        };
        assert_eq!(turn_tokens(&usage), 1_250);
        assert_eq!(token_count(950), "950");
        assert_eq!(token_count(12_400), "12k");
        assert_eq!(token_count(5_000_000), "5M");
        assert_eq!(token_count(1_500_000), "1.5M");
    }

    #[test]
    fn a_session_costs_what_its_turns_reported() {
        let mut session = State::demo().sessions[0].clone();
        session.messages.clear();
        let mut m = Message::new(Role::Agent, "a");
        m.usage.cost_cents = 12.5;
        session.messages.push(m.clone());
        m.usage.cost_cents = 7.5;
        session.messages.push(m);
        assert_eq!(session_cost(&session), 20.);
    }

    #[test]
    fn output_resets_the_quiet_timer() {
        let mut health = Health::default();
        health.quiet_hidden.insert(3);
        health.saw(&Event::Line(Target::Session(3), "x".into()));
        assert!(health.last_output.contains_key(&3) && !health.quiet_hidden.contains(&3));
        health.saw(&Event::Done(Target::Session(3), true));
        assert!(!health.last_output.contains_key(&3));
    }
}
