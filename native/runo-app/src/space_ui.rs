//! Worktree disk space in the workspace: the size on a session's worktree chip, a
//! project's Worktrees view, and deleting build output, by hand or once a thread is done.
//! What counts as build output and how it is deleted is `buildclean`'s.

use super::*;
use crate::buildclean::{self, Activity, Measure};
use crate::cleanup;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a worktree's size is trusted before the open session's is measured again.
const FRESH: Duration = Duration::from_secs(300);

/// Sizes, what is being measured or freed, and threads waiting for their build output
/// to go, while the app runs.
#[derive(Default)]
pub(crate) struct Space {
    /// By worktree folder as sessions hold it: the size, or why it couldn't be measured.
    sizes: HashMap<String, (Result<Measure, String>, Instant)>,
    measuring: HashSet<String>,
    freeing: HashSet<String>,
    /// Done sessions whose build output is still to go: something ran there, or it
    /// hasn't been tried yet.
    pending: HashSet<u64>,
    auto_running: bool,
    /// The project the Worktrees view shows, and its rows once listed.
    project: u64,
    rows: Option<Vec<SpaceRow>>,
    listing: bool,
}

/// One worktree in a project's Worktrees view.
#[derive(Clone)]
struct SpaceRow {
    /// The size cache's key: the session's folder if one uses it, else the path.
    key: String,
    branch: String,
    /// The session or thread in it, with its title.
    session: Option<(u64, String)>,
}

impl Space {
    /// The worktree's size, once measured.
    fn measure_of(&self, dir: &str) -> Option<Measure> {
        self.sizes
            .get(dir)
            .and_then(|(m, _)| m.as_ref().ok().copied())
    }

    /// Bytes of build output in the worktree, once measured.
    pub(crate) fn build_of(&self, dir: &str) -> Option<u64> {
        self.measure_of(dir).map(|m| m.build)
    }

    /// What the worktree chip says after "worktree": its size, or nothing yet.
    pub(crate) fn chip_label(&self, dir: &str) -> Option<String> {
        self.measure_of(dir).map(|m| cleanup::human(m.total))
    }
}

/// Whether a session is done: archived, or a resolved Crew thread.
fn done(s: &Session) -> bool {
    s.archived || s.crew.as_ref().is_some_and(|c| c.resolved)
}

impl RunoApp {
    /// What Runo itself has running in a worktree, across every session that uses it.
    fn activity_in(&self, dir: &str) -> Activity {
        let mut a = Activity::default();
        for s in self.state.sessions.iter().filter(|s| s.dir == dir) {
            a.turn |= self.turn_running(s.id);
            a.checks |= self.proving.contains(&s.id);
            a.terminal |= self
                .shells
                .iter()
                .any(|sh| sh.session == s.id && !sh.exited());
        }
        a
    }

    /// Why build output in `dir` can't be freed now, as far as Runo can tell without
    /// looking at other programs (the background step does that).
    pub(crate) fn worktree_busy(&self, dir: &str) -> Option<String> {
        if self.space.freeing.contains(dir) {
            return Some("freeing…".into());
        }
        buildclean::busy(self.activity_in(dir), &[], Path::new(dir))
    }

    /// Measure a worktree in the background, unless it already is being measured.
    pub(crate) fn measure_worktree(&mut self, repo: String, dir: String) {
        if dir.is_empty() || !self.space.measuring.insert(dir.clone()) {
            return;
        }
        let key = dir.clone();
        self.bg(
            move || buildclean::measure(&repo, Path::new(&dir)),
            move |this, result, cx| {
                this.space.measuring.remove(&key);
                this.space.sizes.insert(key, (result, Instant::now()));
                cx.notify();
            },
        );
    }

    /// Every ~5s: measure the open session's worktree when it hasn't been lately, and
    /// once a minute try again the done threads whose build output is still there.
    pub(crate) fn space_tick(&mut self, minute: bool) {
        if let Some(s) = self.current().filter(|s| s.isolated && !s.dir.is_empty()) {
            let stale = self
                .space
                .sizes
                .get(&s.dir)
                .is_none_or(|(_, at)| at.elapsed() > FRESH);
            if stale && !self.turn_running(s.id) {
                if let Some(p) = self.project_of(s) {
                    let dir = s.dir.clone();
                    self.measure_worktree(p.path, dir);
                }
            }
        }
        if minute {
            self.run_build_clean();
        }
    }

    /// A thread or session is done (resolved, archived, or merged with its worktree
    /// kept): delete its build output soon, if the setting is on.
    pub(crate) fn queue_build_clean(&mut self, id: u64) {
        if settings::remove_build_output() {
            self.space.pending.insert(id);
            self.run_build_clean();
        }
    }

    /// At startup: every done session's worktree, for the build output left from before.
    pub(crate) fn queue_done_build_clean(&mut self) {
        if !settings::remove_build_output() {
            return;
        }
        let ids: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| done(s) && s.isolated && Path::new(&s.dir).is_dir())
            .map(|s| s.id)
            .collect();
        self.space.pending.extend(ids);
        self.run_build_clean();
    }

    /// Free the build output of the pending done sessions that nothing runs in, off the
    /// UI thread. Busy ones stay pending for the next try.
    fn run_build_clean(&mut self) {
        if self.space.auto_running || self.space.pending.is_empty() {
            return;
        }
        if !settings::remove_build_output() {
            self.space.pending.clear();
            return;
        }
        let mut jobs = Vec::new();
        let mut gone = Vec::new();
        for &id in &self.space.pending {
            let Some(s) = self.state.session(id) else {
                gone.push(id);
                continue;
            };
            // Reopened, not a worktree, or already removed: nothing to do.
            if !done(s) || !s.isolated || !Path::new(&s.dir).is_dir() {
                gone.push(id);
                continue;
            }
            if self.worktree_busy(&s.dir).is_some() {
                continue;
            }
            let Some(project) = self.project_of(s) else {
                gone.push(id);
                continue;
            };
            let label = if s.branch.is_empty() {
                s.title.clone()
            } else {
                s.branch.clone()
            };
            jobs.push((id, project.path, s.dir.clone(), label));
        }
        for id in gone {
            self.space.pending.remove(&id);
        }
        if jobs.is_empty() {
            return;
        }
        self.space.auto_running = true;
        for (_, _, dir, _) in &jobs {
            self.space.freeing.insert(dir.clone());
        }
        self.bg(
            move || {
                jobs.into_iter()
                    .map(|(id, repo, dir, label)| {
                        let result = buildclean::free(&repo, Path::new(&dir), Activity::default());
                        let measured = result
                            .is_ok()
                            .then(|| buildclean::measure(&repo, Path::new(&dir)));
                        (id, dir, label, result, measured)
                    })
                    .collect::<Vec<_>>()
            },
            |this, results, cx| {
                this.space.auto_running = false;
                for (id, dir, label, result, measured) in results {
                    this.space.freeing.remove(&dir);
                    match result {
                        Ok(freed) => {
                            this.space.pending.remove(&id);
                            if freed.bytes > 0 || freed.in_use > 0 {
                                applog::info(
                                    "worktree",
                                    format!(
                                        "Freed {} of build output in {label}",
                                        freed.describe()
                                    ),
                                );
                            }
                        }
                        // Something runs there: try again on the next pass.
                        Err(why) if why.starts_with("left alone") => {
                            applog::info("worktree", format!("Build output in {label} {why}"));
                        }
                        Err(why) => {
                            this.space.pending.remove(&id);
                            applog::warn(
                                "worktree",
                                format!("Could not free build output in {label}: {why}"),
                            );
                        }
                    }
                    if let Some(m) = measured {
                        this.space.sizes.insert(dir, (m, Instant::now()));
                    }
                }
                cx.notify();
            },
        );
    }

    /// The worktree chip's Free build output.
    pub(crate) fn free_session_build(&mut self, id: u64) {
        let Some(s) = self.state.session(id) else {
            return;
        };
        let Some(project) = self.project_of(s) else {
            return;
        };
        let label = if s.branch.is_empty() {
            s.title.clone()
        } else {
            s.branch.clone()
        };
        self.free_worktree(project.path, s.dir.clone(), label);
    }

    /// Delete one worktree's build output in the background, then measure it again.
    fn free_worktree(&mut self, repo: String, dir: String, label: String) {
        if let Some(why) = self.worktree_busy(&dir) {
            self.fail(
                "worktree",
                format!("Left the build output in {label} alone: {why}."),
            );
            return;
        }
        let activity = self.activity_in(&dir);
        self.space.freeing.insert(dir.clone());
        let key = dir.clone();
        self.bg(
            move || {
                let result = buildclean::free(&repo, Path::new(&dir), activity);
                let measured = buildclean::measure(&repo, Path::new(&dir));
                (result, measured)
            },
            move |this, (result, measured), cx| {
                this.space.freeing.remove(&key);
                this.space.sizes.insert(key, (measured, Instant::now()));
                match result {
                    Ok(freed) => {
                        let text = format!("Freed {} of build output in {label}", freed.describe());
                        applog::info("worktree", &text);
                        this.info = Some((format!("{text}."), Instant::now()));
                    }
                    Err(why) => this.fail(
                        "worktree",
                        format!("Could not free build output in {label}: {why}"),
                    ),
                }
                cx.notify();
            },
        );
    }

    /// Open a project's Worktrees view and measure what it lists.
    pub(crate) fn open_worktree_space(&mut self, pid: u64) {
        if self.space.project != pid {
            self.space.rows = None;
        }
        self.space.project = pid;
        self.view = View::WorktreeSpace;
        self.list_worktree_space();
    }

    /// List the project's worktrees in the background, then measure each.
    fn list_worktree_space(&mut self) {
        if self.space.listing {
            return;
        }
        let pid = self.space.project;
        let Some(project) = self.state.projects.iter().find(|p| p.id == pid).cloned() else {
            return;
        };
        let sessions: Vec<(u64, String, String)> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid && !s.dir.is_empty())
            .map(|s| (s.id, s.dir.clone(), s.title.clone()))
            .collect();
        self.space.listing = true;
        let repo = project.path.clone();
        self.bg(
            move || {
                let sessions: Vec<_> = sessions
                    .into_iter()
                    .map(|(id, dir, title)| (cleanup::norm(Path::new(&dir)), id, dir, title))
                    .collect();
                buildclean::worktrees(&repo)
                    .into_iter()
                    .map(|w| {
                        let found = sessions.iter().find(|(p, ..)| *p == w.path);
                        SpaceRow {
                            key: found
                                .map(|(_, _, dir, _)| dir.clone())
                                .unwrap_or_else(|| w.path.to_string_lossy().into_owned()),
                            branch: w.branch,
                            session: found.map(|(_, id, _, title)| (*id, title.clone())),
                        }
                    })
                    .collect::<Vec<_>>()
            },
            move |this, rows, cx| {
                this.space.listing = false;
                if this.space.project != pid {
                    return;
                }
                for row in &rows {
                    this.space.sizes.remove(&row.key);
                    this.measure_worktree(project.path.clone(), row.key.clone());
                }
                this.space.rows = Some(rows);
                cx.notify();
            },
        );
    }

    /// The project's Worktrees view: every Runo worktree, biggest first.
    pub(crate) fn worktree_space_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let pid = self.space.project;
        let project = self.state.projects.iter().find(|p| p.id == pid);
        let repo = project.map(|p| p.path.clone()).unwrap_or_default();
        let name = project.map(|p| p.name.clone()).unwrap_or_default();
        let muted = |text: String| {
            div()
                .text_size(px(12.))
                .text_color(rgb(c_muted()))
                .child(text)
        };
        let Some(rows) = self.space.rows.as_ref() else {
            return self.page(
                "Worktrees",
                muted(format!("Listing the worktrees of {name}…")),
            );
        };
        let mut rows = rows.clone();
        let size = |r: &SpaceRow| self.space.measure_of(&r.key);
        rows.sort_by_key(|r| std::cmp::Reverse(size(r).map(|m| m.total)));
        let measuring = rows.iter().any(|r| size(r).is_none());
        let total: u64 = rows.iter().filter_map(size).map(|m| m.total).sum();
        let build: u64 = rows.iter().filter_map(size).map(|m| m.build).sum();
        let summary = if rows.is_empty() {
            format!("{name} has no Runo worktrees under .runo-worktrees.")
        } else {
            format!(
                "{name} · {} · {} in all, {} of it build output{}",
                plural(rows.len(), "worktree"),
                cleanup::human(total),
                cleanup::human(build),
                if measuring { " · measuring…" } else { "" }
            )
        };
        let mut list = shell::card().v_flex().py_1();
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                list = list.child(shell::card_rule());
            }
            let title = row
                .session
                .as_ref()
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| {
                    if row.branch.is_empty() {
                        row.key.clone()
                    } else {
                        row.branch.clone()
                    }
                });
            let detail = match &row.session {
                Some(_) if !row.branch.is_empty() => row.branch.clone(),
                Some(_) => "detached".into(),
                None => "no session uses it".into(),
            };
            let sizes = match self.space.sizes.get(&row.key) {
                Some((Ok(m), _)) => format!(
                    "{} · {} build output",
                    cleanup::human(m.total),
                    cleanup::human(m.build)
                ),
                Some((Err(e), _)) => e.clone(),
                None => "Measuring…".into(),
            };
            let busy = self
                .worktree_busy(&row.key)
                .map(|why| format!("Not now: {why}"));
            let build = self.space.build_of(&row.key);
            let why = busy.or_else(|| match build {
                None => Some("Measuring…".to_string()),
                Some(0) => Some("No build output to free".to_string()),
                Some(_) => None,
            });
            let enabled = why.is_none();
            let free = {
                let (repo, dir, label) = (repo.clone(), row.key.clone(), title.clone());
                self.button(("space-free", i), "Free space", false)
                    .when_some(why, |el, why| {
                        el.opacity(0.5).tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(why.clone())
                                .build(window, cx)
                        })
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // Not the row's click, which opens the session.
                        cx.stop_propagation();
                        if enabled {
                            this.free_worktree(repo.clone(), dir.clone(), label.clone());
                        }
                        cx.notify();
                    }))
            };
            let open = row.session.as_ref().map(|(id, _)| *id);
            list = list.child(
                div()
                    .id(("space-row", i))
                    .px_4()
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .when_some(open, |el, id| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_session(id, window, cx);
                                cx.notify();
                            }))
                    })
                    .child(icon(IconName::GitBranch, 13., c_muted()).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(muted(detail).truncate()),
                    )
                    .child(muted(sizes).flex_none())
                    .child(free),
            );
        }
        self.page(
            "Worktrees",
            div()
                .v_flex()
                .gap_4()
                .child(muted(summary))
                .when(!rows.is_empty(), |el| el.child(list)),
        )
    }
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}
