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
            .mx_1()
            .px_2()
            .py(px(7.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .when(on, |el| el.bg(tint(c_active())))
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::Session(id), cx),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.review(id);
                cx.notify();
            }))
            .child(self.provider_mark(agent::provider(&session.provider), 14.))
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

    /// The Review page: the waiting list on the left, the picked session's changes and
    /// ship controls beside it.
    pub(crate) fn review_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let list = self.review_list();
        let rows: Vec<AnyElement> = list
            .iter()
            .filter_map(|id| self.state.session(*id))
            .map(|s| self.review_row(s, cx))
            .collect();
        let side = div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .border_r_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .child(
                div()
                    .h(px(46.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Review")
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(c_muted()))
                            .child(list.len().to_string()),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .pt_1()
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
                            .border_color(rgb(c_border()))
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
