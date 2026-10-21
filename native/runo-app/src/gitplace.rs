//! The Git place's own controls for the top bar project's folder: which branch it is on,
//! syncing with the remote, and taking the last step back. Every step says what it did
//! in plain words, and the ones that move the folder can be undone for a while.

use super::*;
use std::time::{Duration, Instant};

/// How long the last step stays undoable.
const UNDO_FOR: Duration = Duration::from_secs(30 * 60);
/// Opening the Git place fetches again once the last fetch is this old.
const FETCH_AFTER: Duration = Duration::from_secs(2 * 60);
/// How long a step's note stays at the foot of the Git place.
pub(crate) const TOAST_FOR: Duration = Duration::from_secs(6);

/// The project folder the Git place works on.
#[derive(Clone)]
pub(crate) struct Repo {
    pub dir: String,
    pub state: git::RepoState,
    /// When the remote was last fetched, if it has been.
    pub fetched: Option<Instant>,
    /// A rebase or merge stopped on clashing lines, waiting to be sorted out.
    pub clash: Option<git::Clash>,
}

/// Which half of a sync to run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncStep {
    /// Bring the remote's commits in, then send this folder's up.
    Both,
    Pull,
    Push,
}

/// `text` with a capital first letter and a full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// `text` with a small first letter, to follow on from another clause.
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_lowercase(), chars.as_str()),
        None => String::new(),
    }
}

fn commits(n: usize) -> String {
    match n {
        1 => "1 commit".into(),
        n => format!("{n} commits"),
    }
}

impl RunoApp {
    /// The top bar project's folder, when it is a git repository.
    pub(crate) fn git_place_dir(&self) -> Option<String> {
        let pid = self.current_project()?;
        let project = self.state.projects.iter().find(|p| p.id == pid)?;
        (!project.path.is_empty()).then(|| project.path.clone())
    }

    /// Read the project folder's branch and remote state again, fetching first when the
    /// last fetch is old. Runs off the UI thread.
    pub(crate) fn load_repo(&mut self) {
        if self.demo {
            return;
        }
        let Some(dir) = self.git_place_dir() else {
            self.git_repo = None;
            return;
        };
        self.load_radar();
        let known = self.git_repo.as_ref().filter(|r| r.dir == dir);
        let fetched = known.and_then(|r| r.fetched);
        let fetch = fetched.is_none_or(|at| at.elapsed() > FETCH_AFTER);
        self.bg(
            move || {
                if !git::is_repo(&dir) {
                    return None;
                }
                let fetched = if fetch {
                    git::fetch(&dir).ok().map(|_| Instant::now())
                } else {
                    fetched
                };
                Some(Repo {
                    state: git::repo_state(&dir),
                    clash: git::clash(&dir),
                    dir,
                    fetched,
                })
            },
            |this, repo, cx| {
                // The person may have switched project while it ran.
                if repo
                    .as_ref()
                    .is_none_or(|r| Some(&r.dir) == this.git_place_dir().as_ref())
                {
                    this.git_repo = repo;
                }
                cx.notify();
            },
        );
    }

    /// Take a git step in the project folder off the UI thread and say what happened.
    /// `work` returns the note to show and how Undo takes the step back, if it can, or
    /// None when nothing changed.
    pub(crate) fn git_step(
        &mut self,
        busy: &str,
        work: impl FnOnce(&str) -> Result<Option<(String, Option<git::Undo>)>, String> + Send + 'static,
    ) {
        if self.demo || self.git_working.is_some() {
            return;
        }
        let Some(dir) = self.git_repo.as_ref().map(|r| r.dir.clone()) else {
            return;
        };
        self.git_working = Some(busy.to_string());
        self.menu = None;
        self.bg(
            move || {
                let result = work(&dir);
                let state = git::repo_state(&dir);
                let clash = git::clash(&dir);
                (result, dir, state, clash)
            },
            |this, (result, dir, state, clash), cx| {
                this.git_working = None;
                if let Some(repo) = this.git_repo.as_mut() {
                    repo.state = state;
                    repo.clash = clash;
                }
                match result {
                    Ok(_) => this.git_folder_msg_kept = None,
                    Err(_) => this.git_folder_msg_set = this.git_folder_msg_kept.take(),
                }
                match result {
                    Ok(Some((note, undo))) => {
                        if let Some(undo) = undo {
                            let point = git::UndoPoint {
                                dir,
                                label: note.clone(),
                                undo,
                            };
                            this.git_undo = Some((point, Instant::now()));
                        }
                        this.git_toast = Some((note, Instant::now()));
                    }
                    Ok(None) => {
                        this.git_toast = Some(("Already up to date.".into(), Instant::now()));
                    }
                    Err(error) => this.fail("git", error),
                }
                // Sessions on this folder, and the branch list, see the change too.
                this.refresh();
                this.load_branches();
                this.load_history();
                this.load_time();
                this.load_folder();
                this.load_radar();
                cx.notify();
            },
        );
    }

    /// Bring the remote's commits in, send this folder's up, or both.
    pub(crate) fn git_sync(&mut self, step: SyncStep) {
        let Some(repo) = self.git_repo.as_ref() else {
            return;
        };
        let state = repo.state.clone();
        if state.remote.is_empty() {
            self.fail(
                "git",
                "This folder is not connected to GitHub or any other remote yet.".into(),
            );
            return;
        }
        if state.branch.is_empty() {
            self.fail(
                "git",
                "This folder is not on a branch. Switch to one first.".into(),
            );
            return;
        }
        let busy = match step {
            SyncStep::Both => "Syncing…",
            SyncStep::Pull => "Bringing in…",
            SyncStep::Push => "Sending…",
        };
        self.git_step(
            busy,
            move |dir| {
                let before = git::folder_point(dir);
                let mut notes: Vec<String> = Vec::new();
                // Once commits are sent, taking the folder back would not take them back.
                let mut sent = false;
                if step != SyncStep::Push && !state.upstream.is_empty() {
                    git::fetch(dir)?;
                    let before = git::repo_state(dir).behind;
                    match git::pull(dir)? {
                        git::Update::Conflicts(files) => {
                            // The clash page takes it from here; calling it off there
                            // puts everything back.
                            return Ok(Some((
                                format!(
                                    "The new commits on {} and yours change the same lines in {}. Pick what to keep.",
                                    state.host,
                                    files.join(", ")
                                ),
                                None,
                            )));
                        }
                        git::Update::Updated => {
                            let behind = before.max(1);
                            notes.push(format!("brought in {}", commits(behind)));
                        }
                        git::Update::UpToDate => {}
                    }
                }
                if step != SyncStep::Pull {
                    let now = git::repo_state(dir);
                    if now.upstream.is_empty() || now.ahead > 0 {
                        git::push(dir, &now.remote)?;
                        sent = true;
                        notes.push(if now.upstream.is_empty() {
                            format!("put {} on {}", now.branch, now.host)
                        } else {
                            format!("sent {}", commits(now.ahead))
                        });
                    }
                }
                Ok((!notes.is_empty())
                    .then(|| (sentence(&notes.join(", then ")), before.filter(|_| !sent))))
            },
        );
        if let Some(repo) = self.git_repo.as_mut() {
            repo.fetched = Some(Instant::now());
        }
    }

    /// Check out another branch in the project folder.
    pub(crate) fn git_switch(&mut self, branch: String) {
        self.git_step("Switching…", move |dir| {
            let before = git::folder_point(dir);
            git::switch(dir, &branch).map_err(|e| format!("Could not switch to {branch}: {e}"))?;
            Ok(Some((format!("Switched to {branch}."), before)))
        });
    }

    /// The last step still in reach of Undo.
    fn git_undo_point(&self) -> Option<&git::UndoPoint> {
        let dir = self.git_repo.as_ref().map(|r| r.dir.as_str())?;
        self.git_undo
            .as_ref()
            .filter(|(p, at)| at.elapsed() < UNDO_FOR && p.dir == dir)
            .map(|(p, _)| p)
    }

    /// Take the last step back.
    pub(crate) fn git_undo_last(&mut self) {
        let Some(point) = self.git_undo_point().cloned() else {
            return;
        };
        self.git_undo = None;
        self.git_step("Undoing…", move |_| {
            git::undo(&point).map_err(|e| format!("Could not undo: {e}"))?;
            Ok(Some((
                format!("Undone: {}", lower_first(point.label.trim_end_matches('.'))),
                None,
            )))
        });
    }

    /// The branch picker, Sync and Undo, for the right end of the Git place's tab strip.
    pub(crate) fn git_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(repo) = self.git_repo.as_ref() else {
            return div().into_any_element();
        };
        let state = &repo.state;
        let working = self.git_working.clone();
        let chip = |id: &'static str| {
            div()
                .id(id)
                .relative()
                .h(px(28.))
                .px_2()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(divider())
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(rgb(c_secondary()))
                .hover(|el| el.bg(tint(c_subtle())))
        };
        let branch = chip("git-branch")
            .child(icon(IconName::GitBranch, 12., c_muted()))
            .child(
                div()
                    .max_w(px(160.))
                    .truncate()
                    .text_color(rgb(c_text()))
                    .font_weight(FontWeight::MEDIUM)
                    .child(if let Some(clash) = repo.clash.as_ref() {
                        // Mid-rebase the folder has no branch checked out; say whose it is.
                        clash.yours.clone()
                    } else if state.branch.is_empty() {
                        "No branch".to_string()
                    } else {
                        state.branch.clone()
                    }),
            )
            .child(icon(IconName::ChevronDown, 11., c_muted()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.menu = match this.menu.as_deref() {
                    Some("git-branch") => None,
                    _ => Some("git-branch".into()),
                };
                cx.notify();
            }))
            .when(self.menu.as_deref() == Some("git-branch"), |el| {
                el.child(self.git_branch_menu(cx))
            });
        let counts = |glyph: IconName, n: usize| {
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(icon(glyph, 12., c_muted()))
                .child(
                    div()
                        .text_color(rgb(if n > 0 { c_text() } else { c_muted() }))
                        .font_weight(FontWeight::MEDIUM)
                        .child(n.to_string()),
                )
        };
        let sync = chip("git-sync")
            .pl(px(10.))
            .pr(px(3.))
            .gap(px(8.))
            .when(!state.remote.is_empty(), |el| {
                el.child(counts(IconName::ArrowDown, state.behind))
                    .child(counts(IconName::ArrowUp, state.ahead))
            })
            .child(
                div()
                    .h(px(22.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .rounded(px(6.))
                    .bg(rgb(c_text()))
                    .text_color(rgb(c_canvas()))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(icon(
                        if working.is_some() {
                            IconName::LoaderCircle
                        } else {
                            IconName::RefreshCw
                        },
                        11.,
                        c_canvas(),
                    ))
                    .child(working.clone().unwrap_or_else(|| "Sync".into())),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.menu = match this.menu.as_deref() {
                    Some("git-sync") => None,
                    _ => Some("git-sync".into()),
                };
                cx.notify();
            }))
            .when(self.menu.as_deref() == Some("git-sync"), |el| {
                el.child(self.git_sync_menu(cx))
            });
        let undo = self.git_undo_point().map(|point| {
            let label: SharedString = format!("Undo: {}", point.label).into();
            chip("git-undo")
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(label.clone()).build(window, cx)
                })
                .child(icon(IconName::Undo, 12., c_secondary()))
                .child("Undo")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.git_undo_last();
                    cx.notify();
                }))
        });
        div()
            .flex_none()
            .mb(px(5.))
            .flex()
            .items_center()
            .gap(px(6.))
            .children(undo)
            .child(branch)
            .child(sync)
            .into_any_element()
    }

    /// A popover hanging under one of the strip's controls.
    pub(crate) fn git_popover(
        &self,
        id: &'static str,
        width: f32,
        body: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        div().absolute().top(px(32.)).right_0().child(
            deferred(
                anchored().snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id(id)
                        .w(px(width))
                        .max_h(px(380.))
                        .overflow_y_scroll()
                        .p(px(6.))
                        .rounded_lg()
                        .border_1()
                        .border_color(float_edge())
                        .bg(float(c_raised()))
                        .shadow_lg()
                        .occlude()
                        .cursor_default()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }))
                        .child(body),
                ),
            )
            .with_priority(2),
        )
    }

    fn git_branch_menu(&self, cx: &mut Context<Self>) -> Div {
        let Some(repo) = self.git_repo.as_ref() else {
            return div();
        };
        let mut list = div().v_flex().child(
            div()
                .px_2()
                .pt_1()
                .pb(px(6.))
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child(
                    "Switch this folder to another branch. Your uncommitted changes come along.",
                ),
        );
        for (i, name) in repo.state.branches.iter().enumerate() {
            let on = *name == repo.state.branch;
            let target = name.clone();
            list = list.child(
                div()
                    .id(("git-branch-pick", i))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .text_size(px(13.))
                    .when(!on, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(tint(c_active())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.git_switch(target.clone());
                                cx.notify();
                            }))
                    })
                    .child(icon(IconName::GitBranch, 13., c_muted()))
                    .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                    .when(on, |el| el.child(icon(IconName::Check, 13., c_text()))),
            );
        }
        self.git_popover("git-branch-menu", 280., list, cx)
    }

    fn git_sync_menu(&self, cx: &mut Context<Self>) -> Div {
        let Some(repo) = self.git_repo.as_ref() else {
            return div();
        };
        let state = &repo.state;
        let checked = match repo.fetched {
            Some(at) if at.elapsed() < Duration::from_secs(60) => "checked just now".to_string(),
            Some(at) => format!("checked {} min ago", at.elapsed().as_secs() / 60),
            None => "not checked yet".into(),
        };
        let tile = |glyph: IconName, n: usize, what: &'static str| {
            div()
                .flex_1()
                .p(px(10.))
                .v_flex()
                .gap(px(2.))
                .rounded_md()
                .bg(tint(c_subtle()))
                .child(icon(glyph, 13., c_muted()))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(n.to_string()),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(what),
                )
        };
        let button = |id: &'static str, label: &'static str, primary: bool, step: SyncStep| {
            div()
                .id(id)
                .h(px(26.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .when(primary, |el| {
                    el.bg(rgb(c_text()))
                        .text_color(rgb(c_canvas()))
                        .hover(|el| el.opacity(0.9))
                })
                .when(!primary, |el| {
                    el.border_1()
                        .border_color(divider())
                        .hover(|el| el.bg(tint(c_subtle())))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.git_sync(step);
                    cx.notify();
                }))
        };
        let body = if state.remote.is_empty() {
            div().p_2().v_flex().gap_1().child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Not connected to GitHub"),
            ).child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child("This folder has no remote to sync with. Ask an agent to put the project on GitHub, then Sync appears here."),
            )
        } else {
            let unpublished = state.upstream.is_empty();
            div()
                .p(px(6.))
                .v_flex()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(IconName::RefreshCw, 13., c_text()))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Sync with {}", state.host)),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .child(checked),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(tile(IconName::ArrowDown, state.behind, "new to bring in"))
                        .child(tile(IconName::ArrowUp, state.ahead, "yours to send up")),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(if unpublished {
                            format!("{} is not on {} yet. Sync puts it there.", state.branch, state.host)
                        } else {
                            "Sync brings theirs in first, then sends yours. Nothing of yours is overwritten, and bringing in can be undone.".into()
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .when(!unpublished, |el| {
                            el.child(button("git-sync-pull", "Bring in only", false, SyncStep::Pull))
                                .child(button("git-sync-push", "Send only", false, SyncStep::Push))
                        })
                        .child(div().flex_1())
                        .child(button("git-sync-both", "Sync now", true, SyncStep::Both)),
                )
        };
        self.git_popover("git-sync-menu", 320., body, cx)
    }

    /// What the last step did, floating at the foot of the Git place, with Undo when it
    /// can be taken back.
    pub(crate) fn git_toast_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (note, _) = self
            .git_toast
            .as_ref()
            .filter(|(_, at)| at.elapsed() < TOAST_FOR)?;
        let undo = self.git_undo_point().is_some_and(|p| p.label == *note);
        // Above the clash page's own bar.
        let clash = self.git_repo.as_ref().is_some_and(|r| r.clash.is_some());
        Some(
            div()
                .absolute()
                .bottom(px(if clash { 70. } else { 18. }))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("git-toast")
                        .flex()
                        .items_center()
                        .gap_3()
                        .py(px(7.))
                        .pl(px(14.))
                        .pr(px(8.))
                        .rounded_lg()
                        .border_1()
                        .border_color(float_edge())
                        .bg(float(c_raised()))
                        .shadow_lg()
                        .text_size(px(12.5))
                        .child(icon(IconName::Check, 13., c_add()))
                        .child(note.clone())
                        .when(undo, |el| {
                            el.child(
                                div()
                                    .id("git-toast-undo")
                                    .h(px(24.))
                                    .px(px(9.))
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(divider())
                                    .cursor_pointer()
                                    .hover(|el| el.bg(tint(c_subtle())))
                                    .child(icon(IconName::Undo, 12., c_secondary()))
                                    .child("Undo")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.git_undo_last();
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}
