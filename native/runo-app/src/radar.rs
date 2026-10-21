//! Clash radar: every agent session's work, the project folder's own and the main
//! branch, merged pair by pair in memory to find where two lines of work change the same
//! lines, long before anyone merges. Nothing in any folder is touched; uncommitted work
//! counts too.

use super::*;

/// One line of work the radar compares.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Side {
    /// What people call it: a session's title, "Your folder", or the main branch.
    pub label: String,
    pub branch: String,
    pub session: Option<u64>,
    pub base: bool,
    /// It has commits another branch could bring in, not only uncommitted work.
    pub committed: bool,
}

/// Two lines of work that change the same lines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub a: Side,
    pub b: Side,
    pub files: Vec<git::Overlap>,
}

/// "around line 40", "around lines 40 and 88", "around lines 4, 40 and 88".
pub(crate) fn around(lines: &[usize]) -> String {
    let list: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match list.as_slice() {
        [] => String::new(),
        [one] => format!("around line {one}"),
        [rest @ .., last] => format!("around lines {} and {last}", rest.join(", ")),
    }
}

/// "webhook.rs", "webhook.rs and retry.rs", "webhook.rs and 2 more files".
pub(crate) fn file_names(files: &[git::Overlap]) -> String {
    let name = |f: &git::Overlap| f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
    match files {
        [] => String::new(),
        [one] => name(one),
        [a, b] => format!("{} and {}", name(a), name(b)),
        [a, rest @ ..] => format!("{} and {} more files", name(a), rest.len()),
    }
}

impl RunoApp {
    /// Test-merge every pair of lines of work in the top bar project, off the UI thread.
    pub(crate) fn load_radar(&mut self) {
        if self.demo {
            return;
        }
        let Some(dir) = self.git_place_dir() else {
            self.git_radar = None;
            return;
        };
        let pid = self.current_project();
        // Agent sessions with a branch of their own, and the folder in which they work.
        let sessions: Vec<(Side, String)> = self
            .state
            .sessions
            .iter()
            .filter(|s| {
                Some(s.project_id) == pid && !s.archived && s.isolated && !s.branch.is_empty()
            })
            .map(|s| {
                (
                    Side {
                        label: s.title.clone(),
                        branch: s.branch.clone(),
                        session: Some(s.id),
                        base: false,
                        committed: false,
                    },
                    s.dir.clone(),
                )
            })
            .collect();
        self.bg(
            move || {
                if !git::is_repo(&dir) {
                    return None;
                }
                let base = git::default_branch(&dir);
                let mut sides: Vec<(Side, String)> = Vec::new();
                if let Some(tip) = git::rev(&dir, &git::base_ref(&dir, &base)) {
                    sides.push((
                        Side {
                            label: base.clone(),
                            branch: base.clone(),
                            session: None,
                            base: true,
                            committed: true,
                        },
                        tip,
                    ));
                }
                let here = git::current_branch(&dir);
                if let Some(tip) = git::work_tip(&dir) {
                    sides.push((
                        Side {
                            label: "Your folder".into(),
                            branch: here.clone(),
                            session: None,
                            base: false,
                            committed: false,
                        },
                        tip,
                    ));
                }
                let base_ref = git::base_ref(&dir, &base);
                for (mut side, folder) in sessions {
                    if side.branch == here || !std::path::Path::new(&folder).exists() {
                        continue;
                    }
                    side.committed = git::commits_ahead(&dir, &side.branch, &base_ref) > 0;
                    if let Some(tip) = git::work_tip(&folder) {
                        sides.push((side, tip));
                    }
                }
                let mut hits = Vec::new();
                for i in 0..sides.len() {
                    for j in i + 1..sides.len() {
                        let ((a, ta), (b, tb)) = (&sides[i], &sides[j]);
                        if ta == tb {
                            continue;
                        }
                        if let Ok(files) = git::test_merge(&dir, ta, tb) {
                            if !files.is_empty() {
                                hits.push(Hit {
                                    a: a.clone(),
                                    b: b.clone(),
                                    files,
                                });
                            }
                        }
                    }
                }
                Some((dir, hits))
            },
            |this, found, cx| {
                match found {
                    Some((dir, hits)) if Some(&dir) == this.git_place_dir().as_ref() => {
                        if this.git_radar_sel.is_some_and(|i| i >= hits.len()) {
                            this.git_radar_sel = None;
                        }
                        this.git_radar = Some(hits);
                    }
                    Some(_) => {}
                    None => this.git_radar = None,
                }
                cx.notify();
            },
        );
    }

    /// The radar's rows at the top of the Branches list.
    pub(crate) fn radar_rows(&self, cx: &mut Context<Self>) -> Option<Div> {
        let hits = self.git_radar.as_ref().filter(|h| !h.is_empty())?;
        let mut list = div().v_flex().gap(px(2.)).child(
            div()
                .px_3()
                .pt(px(14.))
                .pb(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_amber()))
                .child(icon(IconName::Radar, 11., c_amber()))
                .child(format!(
                    "HEADS UP  ·  {} CLASH{} COMING",
                    hits.len(),
                    if hits.len() == 1 { "" } else { "ES" }
                )),
        );
        for (i, hit) in hits.iter().enumerate() {
            let on = self.git_radar_sel == Some(i);
            list = list.child(
                div()
                    .id(("radar-row", i))
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
                        el.child(shell::row_fuse(format!("radar-fuse-{i}"), 4.))
                    })
                    .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.git_radar_sel = Some(i);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgba((c_amber() << 8) | 0x22))
                            .child(icon(IconName::TriangleAlert, 13., c_amber())),
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
                                    .child(format!("{} and {}", hit.a.label, hit.b.label)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(format!("Both change {}", file_names(&hit.files))),
                            ),
                    ),
            );
        }
        Some(list)
    }

    /// Ask a session's agent to bring the other line of work in now, while it's small.
    pub(crate) fn radar_ask(
        &mut self,
        id: u64,
        other: &Side,
        files: &[git::Overlap],
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let what = if other.base {
            format!("the latest {}", other.branch)
        } else {
            format!("the branch {}", other.branch)
        };
        let prompt = format!(
            "Heads up: other work changes the same lines as yours in {}. Bring {what} into your \
             branch now (rebase onto it, or merge it in), sort out the clashing lines so both \
             changes still do what they meant to, run the checks, then carry on with your task.",
            paths.join(", ")
        );
        self.send_or_queue(id, prompt, cx);
        self.git_toast = Some((
            "Asked the agent to bring the other work in now.".into(),
            std::time::Instant::now(),
        ));
    }

    /// The picked clash: who, where, and what to do about it.
    pub(crate) fn radar_page(&self, hit: &Hit, cx: &mut Context<Self>) -> AnyElement {
        let mut files = div().v_flex();
        for f in &hit.files {
            files = files.child(
                div()
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.5))
                    .child(icon(IconName::File, 12., c_muted()))
                    .child(div().flex_1().min_w_0().truncate().child(f.path.clone()))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(around(&f.lines)),
                    ),
            );
        }
        // The next steps: whoever can, brings the other's work in now while it's small.
        let mut tiles = div().grid().grid_cols(2).gap(px(10.)).mt(px(22.));
        let mut first = true;
        for (me, other) in [(&hit.a, &hit.b), (&hit.b, &hit.a)] {
            // Only committed work can be brought in.
            let Some(id) = me.session.filter(|_| other.committed) else {
                continue;
            };
            let (other_c, files_c) = (other.clone(), hit.files.clone());
            let primary = first;
            first = false;
            tiles = tiles.child(
                div()
                    .id(SharedString::from(format!("radar-ask-{id}")))
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
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Ask “{}” to bring it in now", me.label)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(format!(
                                "Its agent brings {} in and sorts out the clash while it's small.",
                                other.label
                            )),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.radar_ask(id, &other_c, &files_c, cx);
                        cx.notify();
                    })),
            );
        }
        for side in [&hit.a, &hit.b] {
            if side.base || side.session.is_some() && side.branch.is_empty() {
                continue;
            }
            let branch = side.branch.clone();
            tiles = tiles.child(
                div()
                    .id(SharedString::from(format!("radar-open-{branch}")))
                    .p(px(14.))
                    .v_flex()
                    .gap(px(3.))
                    .rounded_lg()
                    .cursor_pointer()
                    .bg(pane(c_sheet()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Look at {branch}")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child("Its commits, files, and how to merge it."),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.git_radar_sel = None;
                        this.git_branch_sel = Some(branch.clone());
                        this.load_branches();
                        cx.notify();
                    })),
            );
        }
        let advice = match (hit.a.base || hit.b.base, hit.a.session.or(hit.b.session).is_some()) {
            (true, _) => format!(
                "{} has moved on since this work started. Bringing it in now keeps the clash small.",
                if hit.a.base { &hit.a.branch } else { &hit.b.branch }
            ),
            (false, true) => "Whichever lands second will clash. Have one bring the other in now, or merge one first and update the other right after.".to_string(),
            (false, false) => "Whichever lands second will clash. Merge one first and update the other right after.".to_string(),
        };
        div()
            .id("radar-page")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(24.))
            .py(px(20.))
            .v_flex()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_amber()))
                    .child(icon(IconName::Radar, 12., c_amber()))
                    .child("CLASH RADAR"),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "{} and {} change the same lines",
                        hit.a.label, hit.b.label
                    )),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child(format!(
                        "{advice} Found by merging them in memory; nothing was changed."
                    )),
            )
            .child(tiles)
            .child(
                div()
                    .mt(px(24.))
                    .mb(px(6.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_muted()))
                    .child(format!("WHERE THEY CLASH  ·  {}", hit.files.len())),
            )
            .child(files)
            .into_any_element()
    }
}
