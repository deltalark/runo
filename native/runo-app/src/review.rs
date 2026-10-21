//! Review: the one place to check and ship what agents changed. The list holds every
//! session with changes it is done making; picking one shows all its files with the
//! commit, pull request, update-from-base and merge controls. A session's own Changes
//! panel only points here, so shipping happens in one place.

use super::*;

/// Width of the list of sessions waiting for review.
const LIST_WIDTH: f32 = 280.;

impl RunoApp {
    /// Sessions with changed files whose agent has stopped, newest first.
    pub(crate) fn review_list(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| {
                // A mission's parts ship with the mission, from its own session.
                !s.archived
                    && s.canvas.is_empty()
                    && !self.is_running(s)
                    && (s.mission == 0 || self.mission_ready(s.id))
                    && (self.stats.get(&s.id).is_some_and(|st| st.0 > 0)
                        || self.mission_ready(s.id))
            })
            .map(|s| s.id)
            .collect();
        ids.sort_by_key(|id| std::cmp::Reverse(*id));
        ids
    }

    /// Open Review on the selected session when it has something to review, else on the
    /// newest session that does.
    pub(crate) fn open_review(&mut self) {
        let list = self.review_list();
        let keep = self.selected.is_some_and(|id| list.contains(&id));
        if !keep {
            if let Some(&first) = list.first() {
                self.select(first);
            }
        }
        // A mission's work is all in commits: show everything since its base.
        if !self.diff_base && self.selected.is_some_and(|id| self.mission_ready(id)) {
            self.diff_base = true;
            self.refresh();
        }
        self.view = View::Review;
        self.git_tab = View::Review;
        self.load_repo();
    }

    /// Show one session in Review.
    pub(crate) fn review(&mut self, id: u64) {
        // A mission's work is all in commits: show everything since its base.
        if self.mission_ready(id) {
            self.diff_base = true;
            self.selected_file = None;
        }
        self.select(id);
        self.view = View::Review;
        self.git_tab = View::Review;
        self.load_repo();
    }

    fn review_row(&self, session: &Session, cx: &mut Context<Self>) -> AnyElement {
        let id = session.id;
        let on = self.selected == Some(id);
        let (files, added, removed) = self.stats.get(&id).copied().unwrap_or_default();
        let mission = self.mission_ready(id);
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        div()
            .id(("review-row", id as usize))
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
                el.child(shell::row_fuse(format!("review-fuse-{id}"), 4.))
            })
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::Session(id), cx),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.review(id);
                cx.notify();
            }))
            .child(self.provider_badge(agent::provider(&session.provider), 28.))
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
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(match files {
                                _ if mission => format!("Mission branch  ·  {project}"),
                                1 => format!("1 file  ·  {project}"),
                                n => format!("{n} files  ·  {project}"),
                            }),
                    ),
            )
            .when(!mission, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .gap_1()
                        .text_size(px(11.5))
                        .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                        .child(div().text_color(rgb(c_del())).child(format!("−{removed}"))),
                )
            })
            .into_any_element()
    }

    /// Git: what waits to ship, the project folder's branches and its GitHub, as tabs on
    /// the frame, the open one cut from the page below it, with the folder's branch, Sync
    /// and Undo at the strip's right end.
    pub(crate) fn git_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let github = self.view == View::GitHub;
        let branches = self.view == View::Branches;
        let history = self.view == View::GitHistory;
        let tab = |id: &'static str, label: &'static str, count: usize, alert: bool, on: bool| {
            shell::strip_tab(id, "git", on)
                .child(label)
                .when(count > 0, |el| {
                    el.child(
                        div()
                            .px(px(5.))
                            .rounded_full()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(alert, |el| {
                                el.bg(rgba((c_amber() << 8) | 0x33))
                                    .text_color(rgb(c_amber()))
                            })
                            .when(!alert, |el| {
                                el.bg(tint(c_subtle())).text_color(rgb(c_secondary()))
                            })
                            .child(count.to_string()),
                    )
                })
        };
        // The open tab is the colour of what sits under it: the waiting list for
        // Changes, the page for GitHub.
        let under = if github {
            glass_fill(c_canvas())
        } else {
            pane(c_sheet())
        };
        let body = if history {
            shell::island(
                div().size_full().flex().child(self.git_history_view(cx)),
                rgba(0),
                [true, true, false, false],
            )
        } else if branches {
            shell::island(
                div().size_full().flex().child(self.branches_view(cx)),
                rgba(0),
                [true, true, false, false],
            )
        } else if github {
            shell::island(
                div().size_full().flex().child(self.github_view(cx)),
                glass_fill(c_canvas()),
                [true, true, false, false],
            )
        } else {
            shell::island(
                div().size_full().flex().child(self.review_view(cx)),
                rgba(0),
                [true, true, false, false],
            )
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .relative()
                    .px(px(10.))
                    .flex()
                    .items_end()
                    .gap_1()
                    .child(shell::strip_fill("git", under))
                    .child(
                        div()
                            .h(px(28.))
                            .mb(px(3.))
                            .px_2()
                            .mr_1()
                            .flex()
                            .items_center()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Git"),
                    )
                    .child(
                        tab(
                            "git-ship",
                            "Changes",
                            self.review_list().len(),
                            false,
                            !github && !branches && !history,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_review();
                            this.save();
                            cx.notify();
                        })),
                    )
                    .child(
                        tab("git-history", "History", 0, false, history).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.open_git_history();
                                this.save();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        tab(
                            "git-branches",
                            "Branches",
                            self.git_branches
                                .as_ref()
                                .map_or(0, |b| b.rows.iter().filter(|r| !r.remote_only).count()),
                            false,
                            branches,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_branches();
                            this.save();
                            cx.notify();
                        })),
                    )
                    .child(
                        tab("git-github", "GitHub", self.gh.needs_you(), true, github).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.load_github();
                                this.save();
                                cx.notify();
                            }),
                        ),
                    )
                    .child(div().flex_1())
                    .child(self.git_controls(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .child(body.size_full())
                    .children(self.merge_dialog(cx))
                    .children(self.git_toast_view(cx)),
            )
            .into_any_element()
    }

    /// The Review page: the waiting list on the left, the picked session's changes and
    /// ship controls beside it.
    pub(crate) fn review_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let list = self.review_list();
        let rows: Vec<AnyElement> = list
            .iter()
            .filter_map(|id| self.state.session(*id))
            .enumerate()
            .map(|(i, s)| shell::cascade(self.review_row(s, cx), format!("review-in-{}", s.id), i))
            .collect();
        let side = div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .relative()
            .v_flex()
            .bg(pane(c_sheet()))
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_muted()))
                    .child("READY TO REVIEW"),
            )
            .child(
                div()
                    .id("review-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .v_flex()
                    .gap(px(2.))
                    // Room above the first row and below the last for the selected row's
                    // curves, which the scrolling list would otherwise clip.
                    .py(px(12.))
                    .children(rows)
                    .when(list.is_empty(), |el| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(12.5))
                                .text_color(rgb(c_muted()))
                                .child("Nothing waiting. Sessions show up here when their agent has changed files and stopped."),
                        )
                    }),
            );
        let main = match self.current().filter(|s| list.contains(&s.id)) {
            Some(session) => {
                let id = session.id;
                let project = self
                    .project_of(session)
                    .map(|p| p.name)
                    .unwrap_or_default();
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
                            .child(self.provider_mark(agent::provider(&session.provider), 16.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(session.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .h(px(22.))
                                    .rounded_md()
                                    .bg(tint(c_subtle()))
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_secondary()))
                                    .child(icon(IconName::GitBranch, 11., c_muted()))
                                    .child(format!("{project}  ·  {}", session.branch)),
                            )
                            .child(div().flex_1())
                            .child(
                                self.home_button(
                                    "review-open-session",
                                    IconName::MessageSquare,
                                    "Open session",
                                    String::new(),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select(id);
                                    cx.notify();
                                })),
                            ),
                    )
                    .child(self.changes_tab(cx))
                    .into_any_element()
            }
            None => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .v_flex()
                .items_center()
                .justify_center()
                .gap_3()
                .child(icon(IconName::GitCompare, 28., c_muted()))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if list.is_empty() {
                            "Nothing to review yet"
                        } else {
                            "Pick a session to review"
                        }),
                )
                .child(
                    div()
                        .max_w(px(380.))
                        .text_center()
                        .text_size(px(13.))
                        .text_color(rgb(c_muted()))
                        .child("Check every changed file here, leave notes for the agent, then commit, open a pull request or merge."),
                )
                .when(list.is_empty(), |el| {
                    el.child(
                        self.home_button(
                            "review-new-session",
                            IconName::Plus,
                            "New session",
                            keymap::label(cx, "new-session"),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_new_session();
                            cx.notify();
                        })),
                    )
                })
                .into_any_element(),
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

    /// In a session's Changes panel: where its work stands, and the way to Review, which
    /// holds commit, pull request and merge.
    pub(crate) fn review_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let (added, removed) = self
            .changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        let files = self.changes.len();
        let running = self.current().is_some_and(|s| self.is_running(s));
        div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(match files {
                        0 => "No changes yet".to_string(),
                        1 => "1 file changed".to_string(),
                        n => format!("{n} files changed"),
                    })
                    .when(files > 0, |el| {
                        el.child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                            .child(div().text_color(rgb(c_del())).child(format!("−{removed}")))
                    })
                    .when(self.sync.ahead > 0, |el| {
                        el.child(
                            div()
                                .text_color(rgb(c_muted()))
                                .child(format!("·  {} unpushed", self.sync.ahead)),
                        )
                    }),
            )
            .child(
                div()
                    .id("review-and-ship")
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded_md()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .when(files > 0 && !running, |el| {
                        el.bg(rgb(c_text()))
                            .text_color(rgb(c_canvas()))
                            .cursor_pointer()
                            .hover(|el| el.opacity(0.9))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(id) = this.selected {
                                    this.review(id);
                                }
                                cx.notify();
                            }))
                    })
                    .when(files == 0 || running, |el| {
                        el.border_1()
                            .border_color(divider())
                            .text_color(rgb(c_muted()))
                    })
                    .child(icon(
                        IconName::GitCompare,
                        14.,
                        if files > 0 && !running {
                            c_canvas()
                        } else {
                            c_muted()
                        },
                    ))
                    .child(if running {
                        "Review and ship when the agent stops"
                    } else {
                        "Review and ship"
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .opacity(0.7)
                            .child(keymap::label(cx, "go-review")),
                    ),
            )
            .into_any_element()
    }
}
