//! Git's Branches tab: every branch of the top bar project's folder, where each stands
//! against the main branch, and what to do next with it (bring in updates, merge, switch,
//! rename, delete), plus work set aside. Every step that moves something can be undone.

use super::*;
use git::{BranchRow, MergeHow, Undo};

/// Width of the branch list.
const LIST_WIDTH: f32 = 280.;

/// The project folder's branches, read off the UI thread.
#[derive(Clone, Default)]
pub(crate) struct Branches {
    pub dir: String,
    /// The branch everything lands on, such as `main`.
    pub base: String,
    pub rows: Vec<BranchRow>,
    pub stashes: Vec<git::Stash>,
    /// Files changed in the folder and not committed.
    pub dirty: usize,
}

/// The picked branch's commits and files, against the base.
#[derive(Clone, Default)]
pub(crate) struct BranchDetail {
    pub name: String,
    pub commits: Vec<git::CommitRow>,
    pub files: Vec<git::Change>,
}

/// The Merge dialog.
#[derive(Clone)]
pub(crate) struct MergeForm {
    pub branch: String,
    pub how: MergeHow,
    pub delete_after: bool,
    pub push_after: bool,
}

/// What the name box at the top of the list is for.
#[derive(Clone, PartialEq)]
pub(crate) enum Naming {
    New,
    Rename(String),
    /// A branch starting at this commit, from History.
    BranchAt(String),
    /// A release name for this commit, from History.
    Tag(String),
}

/// "The commit", "Both commits", "All 3 commits".
fn every(n: usize) -> String {
    match n {
        1 => "The commit".into(),
        2 => "Both commits".into(),
        n => format!("All {n} commits"),
    }
}

fn commits(n: usize) -> String {
    match n {
        1 => "1 commit".into(),
        n => format!("{n} commits"),
    }
}

impl RunoApp {
    /// Show Git's Branches tab.
    pub(crate) fn open_branches(&mut self) {
        self.view = View::Branches;
        self.git_tab = View::Branches;
        self.load_repo();
        self.load_branches();
    }

    /// Read the branch list again, and the picked branch's commits and files.
    pub(crate) fn load_branches(&mut self) {
        if self.demo || self.view != View::Branches {
            return;
        }
        let Some(dir) = self.git_place_dir() else {
            self.git_branches = None;
            return;
        };
        let picked = self.git_branch_sel.clone();
        self.bg(
            move || {
                if !git::is_repo(&dir) {
                    return None;
                }
                let base = git::default_branch(&dir);
                let rows = git::branch_rows(&dir, &base);
                let current = git::current_branch(&dir);
                let name = picked
                    .filter(|p| rows.iter().any(|r| r.name == *p))
                    .unwrap_or(current);
                let detail = rows.iter().find(|r| r.name == name).map(|row| {
                    let rev = if row.remote_only {
                        format!("{}/{}", git::remote_name(&dir), row.name)
                    } else {
                        row.name.clone()
                    };
                    let base_ref = git::base_ref(&dir, &base);
                    BranchDetail {
                        name: name.clone(),
                        commits: git::commits_only_on(&dir, &rev, &base_ref, 30),
                        files: git::branch_changes(&dir, &rev, &base_ref),
                    }
                });
                Some((
                    Branches {
                        stashes: git::stashes(&dir),
                        dirty: git::changes(&dir).len(),
                        dir,
                        base,
                        rows,
                    },
                    detail,
                ))
            },
            |this, loaded, cx| {
                match loaded {
                    Some((branches, detail))
                        if Some(&branches.dir) == this.git_place_dir().as_ref() =>
                    {
                        this.git_branch_sel = detail.as_ref().map(|d| d.name.clone());
                        this.git_branch_detail = detail;
                        this.git_branches = Some(branches);
                    }
                    Some(_) => {}
                    None => this.git_branches = None,
                }
                cx.notify();
            },
        );
    }

    /// The session whose worktree works on `branch` in the top bar project.
    fn branch_session(&self, branch: &str) -> Option<&Session> {
        let pid = self.current_project()?;
        self.state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid && !s.archived && s.isolated)
            .find(|s| s.branch == branch)
    }

    fn pick_branch(&mut self, name: String) {
        self.git_radar_sel = None;
        self.git_kit = None;
        self.git_branch_sel = Some(name);
        self.git_naming = None;
        self.load_branches();
    }

    /// Bring the base's new commits into a branch, where it is checked out.
    fn update_branch(&mut self, row: &BranchRow, base: String) {
        let Some(at) = row.checked_out.clone() else {
            return;
        };
        let name = row.name.clone();
        self.git_step("Bringing in…", move |dir| {
            let before = git::branches_point(dir, &[&name]);
            let onto = git::base_ref(dir, &base);
            let behind = git::commits_ahead(dir, &onto, &name);
            match git::update_from(&at, &onto, true)? {
                // In the project folder itself, the clash page sorts it out.
                git::Update::Conflicts(files) if at == dir => Ok(Some((
                    format!(
                        "{name} and {base} change the same lines in {}. Pick what to keep.",
                        files.join(", ")
                    ),
                    None,
                ))),
                git::Update::Conflicts(files) => {
                    let _ = git::abort_update(&at);
                    Err(format!(
                        "{name} and {base} change the same lines in {}, so nothing was changed. Ask the agent working on {name} to bring {base} in and sort out the clash.",
                        files.join(", ")
                    ))
                }
                git::Update::UpToDate => Ok(None),
                git::Update::Updated => Ok(Some((
                    format!("Brought {} from {base} into {name}.", commits(behind)),
                    Some(before),
                ))),
            }
        });
    }

    /// Send a branch to the remote, once the seatbelt has looked at it.
    pub(crate) fn send_branch(&mut self, name: String) {
        if !self.belt_clear(&name, explain::Fix::SendBranchAnyway(name.clone())) {
            return;
        }
        let remote = self
            .git_repo
            .as_ref()
            .map(|r| r.state.remote.clone())
            .unwrap_or_default();
        self.git_step("Sending…", move |dir| {
            git::push_branch(dir, &remote, &name)?;
            Ok(Some((format!("Sent {name}."), None)))
        });
    }

    /// Land a branch on the base the way the Merge dialog says.
    fn merge_branch(&mut self, form: MergeForm) {
        let Some(branches) = self.git_branches.as_ref() else {
            return;
        };
        let base = branches.base.clone();
        let remote = self.git_repo.as_ref().map(|r| r.state.remote.clone());
        let message = self
            .branch_session(&form.branch)
            .map(|s| s.title.clone())
            .unwrap_or_else(|| format!("Merge {}", form.branch));
        self.git_merge = None;
        self.git_step("Merging…", move |dir| {
            let before = git::branches_point(dir, &[&base, &form.branch]);
            let landed = git::commits_ahead(dir, &form.branch, &base);
            git::merge_branch(dir, &form.branch, &base, form.how, &message)?;
            let mut note = format!("Merged {} into {base}", form.branch);
            if form.delete_after && git::checked_out_at(dir, &form.branch).is_none() {
                git::delete_branch(dir, &form.branch, true)?;
                note.push_str(&format!(" and deleted {}", form.branch));
            }
            let mut pushed = false;
            if form.push_after {
                if let Some(remote) = remote.filter(|r| !r.is_empty()) {
                    git::push_branch(dir, &remote, &base)?;
                    pushed = true;
                    note.push_str(", then sent it");
                }
            }
            if landed == 0 {
                return Ok(None);
            }
            note.push('.');
            Ok(Some((note, (!pushed).then_some(before))))
        });
    }

    /// Start a branch from where the folder is and switch to it, or rename one.
    fn finish_naming(&mut self, name: String) {
        let Some(naming) = self.git_naming.take() else {
            return;
        };
        let name = git::sanitize_branch(name.trim());
        if name.is_empty() {
            return;
        }
        match naming {
            Naming::New => {
                let picked = name.clone();
                self.git_step("Starting…", move |dir| {
                    let undo = Undo::Steps(
                        git::folder_point(dir)
                            .into_iter()
                            .chain([git::branches_point(dir, &[&name])])
                            .collect(),
                    );
                    git::new_branch(dir, &name, "HEAD")?;
                    git::switch(dir, &name)?;
                    Ok(Some((
                        format!("Started {name}, and switched to it."),
                        Some(undo),
                    )))
                });
                self.git_branch_sel = Some(picked);
            }
            Naming::Rename(from) => {
                if from == name {
                    return;
                }
                let picked = name.clone();
                self.git_step("Renaming…", move |dir| {
                    git::rename_branch(dir, &from, &name)?;
                    Ok(Some((
                        format!("Renamed {from} to {name}."),
                        Some(Undo::Rename {
                            from: from.clone(),
                            to: name.clone(),
                        }),
                    )))
                });
                self.git_branch_sel = Some(picked);
            }
            Naming::BranchAt(sha) => {
                self.git_step("Starting…", move |dir| {
                    let undo = git::branches_point(dir, &[&name]);
                    git::new_branch(dir, &name, &sha)?;
                    Ok(Some((
                        format!("Started {name} from {}.", &sha[..7.min(sha.len())]),
                        Some(undo),
                    )))
                });
            }
            Naming::Tag(sha) => {
                self.git_step("Marking…", move |dir| {
                    git::tag(dir, &name, &sha)?;
                    Ok(Some((
                        format!("Marked {} as {name}.", &sha[..7.min(sha.len())]),
                        Some(Undo::Untag(name.clone())),
                    )))
                });
            }
        }
    }

    /// Open the name box for a new branch where the folder is.
    pub(crate) fn start_new_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_naming(Naming::New, window, cx);
    }

    fn start_naming(&mut self, naming: Naming, window: &mut Window, cx: &mut Context<Self>) {
        let value = match &naming {
            Naming::New => String::new(),
            Naming::Rename(from) => from.clone(),
            Naming::BranchAt(_) | Naming::Tag(_) => String::new(),
        };
        self.git_naming = Some(naming);
        self.git_name.update(cx, |input, cx| {
            input.set_placeholder("Branch name", window, cx);
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
    }

    pub(crate) fn on_git_name(&mut self, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::PressEnter { .. } => {
                let name = self.git_name.read(cx).value().to_string();
                self.finish_naming(name);
            }
            InputEvent::Blur => self.git_naming = None,
            _ => {}
        }
        cx.notify();
    }

    fn list_label(text: impl Into<SharedString>) -> Div {
        div()
            .px_3()
            .pt(px(14.))
            .pb(px(6.))
            .text_size(px(10.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(c_muted()))
            .child(text.into())
    }

    fn branch_row(
        &self,
        i: usize,
        row: &BranchRow,
        base: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.git_branch_sel.as_deref() == Some(row.name.as_str())
            && self.git_radar_sel.is_none();
        let session = self.branch_session(&row.name);
        let host = self
            .git_repo
            .as_ref()
            .map_or("the remote".to_string(), |r| r.state.host.clone());
        let mut detail = Vec::new();
        if let Some(s) = session {
            detail.push(s.title.clone());
        }
        if row.remote_only {
            detail.push(format!("Not on this computer yet · {}", row.when));
        } else if row.name == base {
            detail.push(if row.upstream.is_empty() {
                format!("Not on {host} yet")
            } else {
                "The main branch".into()
            });
        } else {
            if row.ahead > 0 {
                detail.push(format!("{} ahead", row.ahead));
            }
            if row.behind > 0 {
                detail.push(format!("{} behind", row.behind));
            }
            if row.ahead == 0 && row.behind == 0 {
                detail.push(format!("Same as {base}"));
            }
        }
        let glyph = div()
            .size(px(28.))
            .flex_none()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .bg(tint(c_subtle()))
            .child(icon(
                if row.remote_only {
                    IconName::Cloud
                } else {
                    IconName::GitBranch
                },
                13.,
                c_secondary(),
            ));
        let name = row.name.clone();
        let get = row.name.clone();
        div()
            .id(("git-branch-row", i))
            .relative()
            .ml(px(10.))
            .mr_1()
            .px_2()
            .py(px(7.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .when(on, |el| {
                el.child(shell::row_fuse(format!("git-branch-fuse-{i}"), 4.))
            })
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pick_branch(name.clone());
                cx.notify();
            }))
            .child(match session {
                Some(s) => self
                    .provider_badge(agent::provider(&s.provider), 28.)
                    .into_any_element(),
                None => glyph.into_any_element(),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(detail.join("  ·  ")),
                    ),
            )
            .when(row.remote_only, |el| {
                el.child(
                    small_button(("git-branch-get", i), "Get").on_click(cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            let name = get.clone();
                            this.git_step("Getting…", move |dir| {
                                let undo = git::branches_point(dir, &[&name]);
                                git::get_remote_branch(dir, &name)?;
                                Ok(Some((
                                    format!("Got {name}. It's on this computer now."),
                                    Some(undo),
                                )))
                            });
                            cx.notify();
                        },
                    )),
                )
            })
            .into_any_element()
    }

    fn stash_row(&self, i: usize, stash: &git::Stash, cx: &mut Context<Self>) -> AnyElement {
        let (message, commit) = (stash.message.clone(), stash.commit.clone());
        div()
            .id(("git-stash-row", i))
            .ml(px(10.))
            .mr_1()
            .px_2()
            .py(px(7.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .child(
                div()
                    .size(px(28.))
                    .flex_none()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(tint(c_subtle()))
                    .child(icon(IconName::Archive, 13., c_secondary())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(stash.message.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(stash.when.clone()),
                    ),
            )
            .child(
                small_button(("git-stash-back", i), "Bring back").on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.git_step("Bringing back…", move |dir| {
                            git::stash_apply(dir, i)?;
                            Ok(Some(("Brought your set-aside changes back.".into(), None)))
                        });
                        cx.notify();
                    },
                )),
            )
            .child(
                self.ghost_button_sized(("git-stash-drop", i), IconName::Trash, "Throw away", 24.)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let (message, commit) = (message.clone(), commit.clone());
                        this.git_step("Throwing away…", move |dir| {
                            git::stash_drop(dir, i)?;
                            Ok(Some((
                                "Threw away the set-aside changes.".into(),
                                Some(Undo::Restash { commit, message }),
                            )))
                        });
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// The Branches page: the list beside the picked branch.
    pub(crate) fn branches_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(data) = self.git_branches.as_ref() else {
            let text = if self.git_place_dir().is_none() || self.demo {
                "Pick a project in the top bar to see its branches."
            } else {
                "Reading branches…"
            };
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(rgb(c_muted()))
                .bg(glass_fill(c_canvas()))
                .child(text)
                .into_any_element();
        };
        let current = self
            .git_repo
            .as_ref()
            .map(|r| r.state.branch.clone())
            .unwrap_or_default();
        let host = self
            .git_repo
            .as_ref()
            .map_or("the remote".to_string(), |r| r.state.host.clone());
        let mut list = div().v_flex().gap(px(2.)).children(self.radar_rows(cx));
        let mut i = 0;
        let mut group = |list: Div, label: String, rows: Vec<&BranchRow>, i: &mut usize| {
            if rows.is_empty() {
                return list;
            }
            let mut list = list.child(Self::list_label(label));
            for row in rows {
                list = list.child(self.branch_row(*i, row, &data.base, cx));
                *i += 1;
            }
            list
        };
        let (here, rest): (Vec<_>, Vec<_>) = data
            .rows
            .iter()
            .partition(|r| r.name == current && !r.remote_only);
        let (sessions, rest): (Vec<_>, Vec<_>) = rest
            .into_iter()
            .partition(|r| !r.remote_only && self.branch_session(&r.name).is_some());
        let (remote, yours): (Vec<_>, Vec<_>) = rest.into_iter().partition(|r| r.remote_only);
        list = group(list, "YOU'RE ON".into(), here, &mut i);
        list = group(list, "AGENT SESSIONS".into(), sessions, &mut i);
        list = group(list, "YOURS".into(), yours, &mut i);
        list = group(
            list,
            format!("ONLY ON {}", host.to_uppercase()),
            remote,
            &mut i,
        );
        if !data.stashes.is_empty() || data.dirty > 0 {
            list = list.child(Self::list_label("SET ASIDE"));
            for (i, stash) in data.stashes.iter().enumerate() {
                list = list.child(self.stash_row(i, stash, cx));
            }
            if data.dirty > 0 {
                let label = format!(
                    "Set aside {} changed file{}",
                    data.dirty,
                    if data.dirty == 1 { "" } else { "s" }
                );
                list = list.child(
                    div()
                        .id("git-stash-push")
                        .mx(px(10.))
                        .mt_1()
                        .h(px(30.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .border_1()
                        .border_color(divider())
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .child(icon(IconName::Archive, 13., c_secondary()))
                        .child(label)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let branch = this
                                .git_repo
                                .as_ref()
                                .map(|r| r.state.branch.clone())
                                .unwrap_or_default();
                            this.git_step("Setting aside…", move |dir| {
                                let when = chrono::Local::now().format("%b %-d, %H:%M");
                                git::stash_push(dir, &format!("Work on {branch}, {when}"))?;
                                Ok(Some((
                                    "Set your changes aside. The folder is clean now.".into(),
                                    Some(Undo::Unstash),
                                )))
                            });
                            cx.notify();
                        })),
                );
            }
        }
        let naming = self.git_naming.is_some();
        let side = div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .bg(pane(c_sheet()))
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
                            .child(format!("{} BRANCHES", data.rows.len())),
                    )
                    .child(
                        self.ghost_button_sized("git-branch-new", IconName::GitBranchPlus, "Start a new branch from here", 24.)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_naming(Naming::New, window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .when(naming, |el| {
                el.child(
                    div()
                        .px_3()
                        .pt_1()
                        .v_flex()
                        .gap_1()
                        .child(Input::new(&self.git_name).bordered(true).h(px(28.)))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .child(match &self.git_naming {
                                    Some(Naming::Rename(from)) => {
                                        format!("A new name for {from}, then Enter.")
                                    }
                                    _ => format!(
                                        "A name for the new branch, then Enter. It starts from {current} and this folder switches to it."
                                    ),
                                }),
                        ),
                )
            })
            .child(
                div()
                    .id("git-branch-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(12.))
                    .child(list),
            );
        let main = match self
            .git_radar_sel
            .and_then(|i| self.git_radar.as_ref()?.get(i).cloned())
        {
            Some(hit) => self.radar_page(&hit, cx),
            None => self.branch_page(data, &current, &host, cx),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(side)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .bg(glass_fill(c_canvas()))
                    .child(main),
            )
            .into_any_element()
    }

    fn branch_page(
        &self,
        data: &Branches,
        current: &str,
        host: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self
            .git_branch_sel
            .as_ref()
            .and_then(|n| data.rows.iter().find(|r| r.name == *n))
            .cloned()
        else {
            return div().into_any_element();
        };
        if self.git_kit.as_ref() == Some(&row.name) {
            return self.kit_page(&row, cx);
        }
        let base = data.base.clone();
        let is_base = row.name == base;
        let session = self
            .branch_session(&row.name)
            .map(|s| (s.id, s.title.clone()));
        let (headline, sub) = if row.remote_only {
            (
                format!("{} is only on {host}", row.name),
                "Get it to look through it, switch to it, or merge it here.".to_string(),
            )
        } else if is_base {
            let unsent = self
                .git_repo
                .as_ref()
                .filter(|r| r.state.branch == base)
                .map_or(0, |r| r.state.ahead);
            (
                format!("{base} is the main branch"),
                if unsent > 0 {
                    format!(
                        "Every other branch lands here once it's ready. {} not on {host} yet.",
                        commits(unsent)
                    )
                } else {
                    "Every other branch lands here once it's ready.".to_string()
                },
            )
        } else {
            let headline = match (row.ahead, row.behind) {
                (0, 0) => format!("Same as {base}"),
                (a, 0) => format!("{} only here, and {base} hasn't moved on", commits(a)),
                (0, b) => format!("Nothing new here, and {base} has moved on by {b}"),
                (a, b) => format!("{} only here, and {base} has moved on by {b}", commits(a)),
            };
            let mut sub = Vec::new();
            if row.ahead > 0 && row.behind > 0 {
                sub.push(if row.checked_out.is_some() {
                    format!("Bring {base}'s updates in first, so you merge onto the latest.")
                } else {
                    format!("Merging keeps {base}'s newer commits as they are.")
                });
            }
            if row.upstream.is_empty() {
                sub.push(format!("Not on {host} yet."));
            }
            (headline, sub.join(" "))
        };
        // Two bars: what both share, what only the base has, and what only this branch has.
        let bars = (!is_base && !row.remote_only && (row.ahead > 0 || row.behind > 0)).then(|| {
            let total = (row.ahead + row.behind).max(1) as f32;
            let shared = 1.0f32;
            let bar = |label: String, behind: f32, ahead: f32, color: u32, note: String| {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(div().w(px(110.)).truncate().child(label))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .gap(px(3.))
                            .child(
                                div()
                                    .flex_grow(shared)
                                    .flex_basis(px(0.))
                                    .h(px(8.))
                                    .rounded(px(3.))
                                    .bg(tint(c_border())),
                            )
                            .when(behind > 0., |el| {
                                el.child(
                                    div()
                                        .flex_grow(behind)
                                        .flex_basis(px(0.))
                                        .h(px(8.))
                                        .rounded(px(3.))
                                        .bg(rgb(color)),
                                )
                            })
                            .when(ahead > 0., |el| {
                                el.child(div().flex_grow(ahead).flex_basis(px(0.)).h(px(8.)))
                            }),
                    )
                    .child(div().w(px(120.)).text_color(rgb(color)).child(note))
            };
            let behind = row.behind as f32 / total;
            let ahead = row.ahead as f32 / total;
            div()
                .v_flex()
                .gap_2()
                .mt_4()
                .child(bar(
                    base.clone(),
                    behind,
                    ahead,
                    c_blue(),
                    format!("{} you don't have", row.behind),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(div().w(px(110.)).truncate().child(row.name.clone()))
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .gap(px(3.))
                                .child(
                                    div()
                                        .flex_grow(shared)
                                        .flex_basis(px(0.))
                                        .h(px(8.))
                                        .rounded(px(3.))
                                        .bg(tint(c_border())),
                                )
                                .when(behind > 0., |el| {
                                    el.child(div().flex_grow(behind).flex_basis(px(0.)).h(px(8.)))
                                })
                                .when(ahead > 0., |el| {
                                    el.child(
                                        div()
                                            .flex_grow(ahead)
                                            .flex_basis(px(0.))
                                            .h(px(8.))
                                            .rounded(px(3.))
                                            .bg(rgb(c_add())),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .w(px(120.))
                                .text_color(rgb(c_add()))
                                .child(format!("{} only here", row.ahead)),
                        ),
                )
        });
        // The next steps, the recommended one first and highlighted.
        let mut tiles: Vec<(&'static str, IconName, String, String, &'static str, bool)> =
            Vec::new();
        let can_update = !is_base && row.behind > 0 && row.checked_out.is_some();
        let can_merge = !is_base && !row.remote_only && row.ahead > 0;
        let recommend = if can_update && row.ahead > 0 {
            "update"
        } else if can_merge {
            "merge"
        } else {
            ""
        };
        if can_update {
            tiles.push(("update", IconName::ArrowDown, format!("Bring in {base}'s updates"), "Replays this branch's work after them. Stops and changes nothing if the same lines clash.".into(), "rebase", recommend == "update"));
        }
        if can_merge {
            tiles.push((
                "merge",
                IconName::GitMerge,
                format!("Merge into {base}"),
                format!(
                    "Put {} on {base}. You choose how, and can undo it.",
                    commits(row.ahead)
                ),
                "merge",
                recommend == "merge",
            ));
        }
        if let Some((_, _)) = &session {
            if row.ahead > 0 {
                tiles.push((
                    "pr",
                    IconName::GitPullRequest,
                    "Open pull request".into(),
                    format!("Send it to {host} and ask for a review, from Changes."),
                    "push + PR",
                    false,
                ));
            }
        } else if !row.remote_only && !is_base && row.upstream.is_empty() {
            tiles.push((
                "send",
                IconName::CloudUpload,
                format!("Send to {host}"),
                "Put this branch there so others can see it.".into(),
                "push",
                false,
            ));
        }
        let unsent = self
            .git_repo
            .as_ref()
            .filter(|r| is_base && r.state.branch == row.name)
            .map_or(0, |r| r.state.ahead);
        if unsent > 0 {
            tiles.push((
                "sync",
                IconName::CloudUpload,
                format!("Send to {host}"),
                format!("{} here that {host} doesn't have yet.", commits(unsent)),
                "push",
                true,
            ));
        }
        if row.checked_out.is_none() && row.name != current {
            tiles.push((
                "switch",
                IconName::ArrowLeftRight,
                "Switch to it".into(),
                "Work on it in this folder. Your uncommitted changes come along.".into(),
                "switch",
                false,
            ));
        }
        if can_merge {
            let lines: usize = self
                .git_branch_detail
                .as_ref()
                .filter(|d| d.name == row.name)
                .map_or(0, |d| d.files.iter().map(|f| f.added + f.removed).sum());
            tiles.push((
                "reviewable",
                IconName::ScanSearch,
                "Make it reviewable".into(),
                if lines > 400 {
                    format!("{lines} lines is a lot for one review. Split it and write a guide.")
                } else {
                    "A reviewer's guide, and a split if it's big.".into()
                },
                "",
                false,
            ));
        }
        if session.is_some() {
            tiles.push((
                "session",
                IconName::MessageSquare,
                "Open the session".into(),
                "Ask the agent to keep working on it.".into(),
                "",
                false,
            ));
        }
        tiles.sort_by_key(|t| !t.5);
        let tile_row = row.clone();
        let tile_base = base.clone();
        let mut grid = div().grid().grid_cols(3).gap(px(10.)).mt(px(22.));
        for (key, glyph, title, body, term, primary) in tiles {
            let row = tile_row.clone();
            let base = tile_base.clone();
            let session_id = session.as_ref().map(|s| s.0);
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("git-tile-{key}")))
                    .p(px(14.))
                    .v_flex()
                    .gap(px(3.))
                    .rounded_lg()
                    .cursor_pointer()
                    .when(primary, |el| {
                        el.bg(rgba((c_blue() << 8) | 0x24))
                            .border_1()
                            .border_color(rgba((c_blue() << 8) | 0x66))
                    })
                    .when(!primary, |el| {
                        el.bg(pane(c_sheet())).hover(|el| el.bg(tint(c_subtle())))
                    })
                    .child(
                        div()
                            .size(px(28.))
                            .mb_2()
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(primary, |el| el.bg(rgb(c_blue())))
                            .when(!primary, |el| el.bg(tint(c_subtle())))
                            .child(icon(
                                glyph,
                                14.,
                                if primary { 0xffffff } else { c_secondary() },
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(body),
                    )
                    .when(!term.is_empty(), |el| el.child(git_term(term).mt_1()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        match key {
                            "update" => this.update_branch(&row, base.clone()),
                            "merge" => {
                                this.git_merge = Some(MergeForm {
                                    branch: row.name.clone(),
                                    how: MergeHow::Squash,
                                    delete_after: row.checked_out.is_none(),
                                    push_after: false,
                                });
                            }
                            "send" => this.send_branch(row.name.clone()),
                            "switch" => this.git_switch(row.name.clone()),
                            "reviewable" => this.git_kit = Some(row.name.clone()),
                            "sync" => this.git_sync(gitplace::SyncStep::Push),
                            "pr" | "session" => {
                                if let Some(id) = session_id {
                                    if key == "pr" {
                                        this.review(id);
                                    } else {
                                        this.select(id);
                                    }
                                }
                            }
                            _ => {}
                        }
                        cx.notify();
                    })),
            );
        }
        let detail = self
            .git_branch_detail
            .as_ref()
            .filter(|d| d.name == row.name);
        let mut commits_list = div().v_flex();
        let mut files_list = div().v_flex();
        if let Some(detail) = detail {
            for c in &detail.commits {
                commits_list = commits_list.child(
                    div()
                        .h(px(30.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.5))
                        .child(
                            div()
                                .w(px(64.))
                                .flex_none()
                                .font_family("JetBrains Mono")
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(c.short.clone()),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(c.subject.clone()))
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(format!("{}  ·  {}", c.author, c.when)),
                        ),
                );
            }
            for f in &detail.files {
                files_list = files_list.child(
                    div()
                        .h(px(28.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.5))
                        .child(
                            div()
                                .w(px(12.))
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(match f.status.as_str() {
                                    "A" => c_add(),
                                    "D" => c_del(),
                                    _ => c_amber(),
                                }))
                                .child(f.status.clone()),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(f.path.clone()))
                        .when(!f.binary, |el| {
                            el.child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_add()))
                                    .child(format!("+{}", f.added)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_del()))
                                    .child(format!("−{}", f.removed)),
                            )
                        }),
                );
            }
        }
        let n_commits = detail.map_or(0, |d| d.commits.len());
        let n_files = detail.map_or(0, |d| d.files.len());
        let deletable =
            !is_base && !row.remote_only && row.checked_out.is_none() && session.is_none();
        let renamable = !is_base && !row.remote_only && session.is_none();
        let rename_from = row.name.clone();
        let delete_name = row.name.clone();
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(
                div()
                    .h(px(46.))
                    .flex_none()
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(divider())
                    .child(icon(
                        if row.remote_only {
                            IconName::Cloud
                        } else {
                            IconName::GitBranch
                        },
                        15.,
                        c_muted(),
                    ))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(row.name.clone()),
                    )
                    .when_some(session.as_ref(), |el, (_, title)| {
                        el.child(
                            div()
                                .flex_none()
                                .px_2()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .rounded_md()
                                .bg(tint(c_subtle()))
                                .text_size(px(11.5))
                                .text_color(rgb(c_secondary()))
                                .child(title.clone()),
                        )
                    })
                    .child(div().flex_1())
                    .when(renamable, |el| {
                        el.child(
                            small_button("git-branch-rename", "Rename").on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.start_naming(
                                        Naming::Rename(rename_from.clone()),
                                        window,
                                        cx,
                                    );
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .when(deletable, |el| {
                        el.child(
                            small_button("git-branch-delete", "Delete").on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let name = delete_name.clone();
                                    this.git_branch_sel = None;
                                    this.git_step("Deleting…", move |dir| {
                                        let undo = git::branches_point(dir, &[&name]);
                                        git::delete_branch(dir, &name, true)?;
                                        Ok(Some((format!("Deleted {name}."), Some(undo))))
                                    });
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .child(
                div()
                    .id("git-branch-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .py(px(20.))
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(headline),
                    )
                    .when(!sub.is_empty(), |el| {
                        el.child(
                            div()
                                .mt_1()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .child(sub),
                        )
                    })
                    .children(bars)
                    .child(grid)
                    .when(n_commits > 0, |el| {
                        el.child(section_label(format!(
                            "ONLY ON THIS BRANCH  ·  {n_commits}"
                        )))
                        .child(commits_list)
                    })
                    .when(n_files > 0, |el| {
                        el.child(section_label(format!("FILES IT CHANGES  ·  {n_files}")))
                            .child(files_list)
                    }),
            )
            .into_any_element()
    }

    /// The Merge dialog over the Git place.
    pub(crate) fn merge_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.git_merge.clone()?;
        let data = self.git_branches.as_ref()?;
        let row = data.rows.iter().find(|r| r.name == form.branch)?;
        let base = data.base.clone();
        let files = self
            .git_branch_detail
            .as_ref()
            .filter(|d| d.name == form.branch)
            .map(|d| d.files.clone())
            .unwrap_or_default();
        let (added, removed) = files
            .iter()
            .fold((0, 0), |a, f| (a.0 + f.added, a.1 + f.removed));
        let title = self
            .branch_session(&form.branch)
            .map(|s| s.title.clone())
            .unwrap_or_else(|| format!("Merge {}", form.branch));
        let host = self
            .git_repo
            .as_ref()
            .map_or("the remote".to_string(), |r| r.state.host.clone());
        let option =
            |i: usize, how: MergeHow, label: &'static str, term: &'static str, body: String| {
                let on = form.how == how;
                div()
                    .id(("git-merge-how", i))
                    .p(px(12.))
                    .flex()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .cursor_pointer()
                    .when(on, |el| {
                        el.border_color(rgb(c_blue()))
                            .bg(rgba((c_blue() << 8) | 0x14))
                    })
                    .when(!on, |el| {
                        el.border_color(divider())
                            .hover(|el| el.bg(tint(c_subtle())))
                    })
                    .child(
                        div()
                            .mt(px(1.))
                            .size(px(16.))
                            .flex_none()
                            .rounded_full()
                            .border_1()
                            .when(on, |el| el.border(px(5.)).border_color(rgb(c_blue())))
                            .when(!on, |el| el.border_color(rgb(c_border()))),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(label),
                                    )
                                    .child(git_term(term)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .child(body),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(form) = this.git_merge.as_mut() {
                            form.how = how;
                        }
                        cx.notify();
                    }))
            };
        let check = |id: &'static str, on: bool, label: String, flip: fn(&mut MergeForm)| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(9.))
                .cursor_pointer()
                .text_size(px(12.5))
                .child(
                    div()
                        .size(px(14.))
                        .flex_none()
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(on, |el| {
                            el.bg(rgb(c_blue()))
                                .child(icon(IconName::Check, 10., 0xffffff))
                        })
                        .when(!on, |el| el.border_1().border_color(rgb(c_border()))),
                )
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = this.git_merge.as_mut() {
                        flip(form);
                    }
                    cx.notify();
                }))
        };
        let n = row.ahead;
        let can_delete = row.checked_out.is_none();
        let form_go = form.clone();
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_start()
                .justify_center()
                .pt(px(90.))
                .bg(rgba(0x06060890))
                .occlude()
                .child(
                    div()
                        .id("git-merge-dialog")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .w(px(520.))
                        .p(px(20.))
                        .v_flex()
                        .gap(px(8.))
                        .rounded_xl()
                        .border_1()
                        .border_color(float_edge())
                        .bg(float(c_raised()))
                        .shadow_lg()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .size(px(28.))
                                        .rounded_md()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(rgb(c_blue()))
                                        .child(icon(IconName::GitMerge, 14., 0xffffff)),
                                )
                                .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(format!("Merge {} into {base}?", form.branch))),
                        )
                        .child(
                            div()
                                .my_2()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .child(format!(
                                    "Adds {} to {base}, touching {} file{} (+{added} −{removed}).{}",
                                    commits(n),
                                    files.len(),
                                    if files.len() == 1 { "" } else { "s" },
                                    if row.behind > 0 {
                                        format!(" {base} has {} this branch doesn't; they stay as they are.", commits(row.behind))
                                    } else {
                                        String::new()
                                    }
                                )),
                        )
                        .child(option(0, MergeHow::Squash, "One tidy commit", "squash", format!("{base} gets a single commit named “{title}”. Best for most work.")))
                        .child(option(1, MergeHow::Merge, "Keep every commit", "merge", format!("{} plus a merge commit, so the history shows the branch.", every(n))))
                        .child(option(2, MergeHow::Rebase, "Line them up on top", "rebase", format!("{}, copied after {base}'s. A straight history.", every(n))))
                        .child(
                            div()
                                .mt_2()
                                .v_flex()
                                .gap_2()
                                .when(can_delete, |el| {
                                    el.child(check("git-merge-delete", form.delete_after, format!("Delete {} afterwards", form.branch), |f| f.delete_after = !f.delete_after))
                                })
                                .child(check("git-merge-push", form.push_after, format!("Send {base} to {host} right away"), |f| f.push_after = !f.push_after)),
                        )
                        .child(
                            div()
                                .mt_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(icon(IconName::Undo, 12., c_muted()))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(11.5))
                                        .text_color(rgb(c_muted()))
                                        .child(if form.push_after {
                                            "Once sent, it can't be undone from here."
                                        } else {
                                            "You can undo this for 30 minutes."
                                        }),
                                )
                                .child(small_button("git-merge-cancel", "Cancel").on_click(cx.listener(|this, _, _, cx| {
                                    this.git_merge = None;
                                    cx.notify();
                                })))
                                .child(
                                    div()
                                        .id("git-merge-go")
                                        .h(px(28.))
                                        .px_3()
                                        .flex()
                                        .items_center()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(rgb(c_blue()))
                                        .text_color(rgb(0xffffff))
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .hover(|el| el.opacity(0.9))
                                        .child("Merge")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.merge_branch(form_go.clone());
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.git_merge = None;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }
}

/// A Git word shown beside its plain name, for people who know it.
pub(crate) fn git_term(term: &'static str) -> Div {
    div()
        .flex_none()
        .self_start()
        .px(px(5.))
        .rounded(px(4.))
        .bg(tint(c_subtle()))
        .font_family("JetBrains Mono")
        .text_size(px(10.5))
        .text_color(rgb(c_muted()))
        .child(term)
}

fn section_label(text: String) -> Div {
    div()
        .mt(px(24.))
        .mb(px(6.))
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_muted()))
        .child(text)
}

fn small_button(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(26.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(divider())
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .hover(|el| el.bg(tint(c_subtle())))
        .child(label)
}
