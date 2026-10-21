//! Git's History tab: the project folder's commits as a graph, newest first, and what to
//! do with the picked one (take it back, copy it to another branch, start a branch or a
//! release from it, go back to it), with an agent's plain-words account of what it did.

use super::*;
use branches::{git_term, Naming};
use git::{GraphRow, LogRow};

/// How many commits History reads at once.
const LIMIT: usize = 300;
/// Height of one commit row, and the room each graph lane takes.
const ROW: f32 = 34.;
const LANE: f32 = 14.;
/// Width of the picked commit's page.
const PAGE_WIDTH: f32 = 400.;

/// The project folder's commits, read off the UI thread.
#[derive(Clone, Default)]
pub(crate) struct History {
    pub dir: String,
    /// The checked-out branch.
    pub branch: String,
    pub rows: Vec<LogRow>,
    pub graph: Vec<GraphRow>,
    /// The most lanes any row uses.
    pub lanes: usize,
}

/// The picked commit, and where it stands against the checked-out branch.
#[derive(Clone, Default)]
pub(crate) struct Picked {
    pub info: git::CommitInfo,
    /// Part of the checked-out branch.
    pub here: bool,
    /// Commits on the checked-out branch after it.
    pub after: usize,
}

/// A lane's colour, the same lane keeping it from row to row.
fn lane_color(lane: usize) -> u32 {
    match lane % 5 {
        0 => c_blue(),
        1 => c_green(),
        2 => c_amber(),
        3 => 0xb48cf2,
        _ => c_red(),
    }
}

fn commits(n: usize) -> String {
    match n {
        1 => "1 commit".into(),
        n => format!("{n} commits"),
    }
}

/// One branch or tag name on a commit, as git decorates it.
enum RefName {
    /// The checked-out branch.
    Head(String),
    Local(String),
    Remote(String),
    Tag(String),
}

fn ref_names(refs: &[String], remote: &str) -> Vec<RefName> {
    refs.iter()
        .map(|r| {
            if let Some(name) = r.strip_prefix("HEAD -> ") {
                RefName::Head(name.to_string())
            } else if let Some(name) = r.strip_prefix("tag: ") {
                RefName::Tag(name.to_string())
            } else if !remote.is_empty() && r.starts_with(&format!("{remote}/")) {
                RefName::Remote(r.clone())
            } else if r == "HEAD" {
                RefName::Head("this folder".into())
            } else {
                RefName::Local(r.clone())
            }
        })
        .collect()
}

fn ref_chip(name: &RefName) -> Div {
    let (glyph, text, color, strong) = match name {
        RefName::Head(n) => (IconName::GitBranch, n, c_blue(), true),
        RefName::Local(n) => (IconName::GitBranch, n, c_green(), false),
        RefName::Remote(n) => (IconName::Cloud, n, c_secondary(), false),
        RefName::Tag(n) => (IconName::Tag, n, c_amber(), false),
    };
    div()
        .flex_none()
        .h(px(20.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(5.))
        .border_1()
        .border_color(rgba((color << 8) | 0x66))
        .when(strong, |el| el.bg(rgba((color << 8) | 0x22)))
        .text_size(px(11.))
        .text_color(rgb(color))
        .child(icon(glyph, 11., color))
        .child(text.clone())
}

/// The graph's slice for one row: lines in from the row above, out to the row below, and
/// the commit's dot.
fn graph_cell(row: GraphRow, lanes: usize, merge: bool) -> impl IntoElement {
    let width = LANE * lanes.max(1) as f32 + 12.;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let x = |lane: usize| bounds.origin.x + px(10. + LANE * lane as f32);
            let top = bounds.origin.y;
            let mid = top + px(ROW / 2.);
            let bottom = top + px(ROW);
            let mut line = |from: usize, to: usize, y0: Pixels, y1: Pixels| {
                let mut b = PathBuilder::stroke(px(1.6));
                b.move_to(point(x(from), y0));
                if from == to {
                    b.line_to(point(x(to), y1));
                } else {
                    let half = (y1 - y0) / 2.;
                    b.cubic_bezier_to(
                        point(x(to), y1),
                        point(x(from), y0 + half),
                        point(x(to), y1 - half),
                    );
                }
                if let Ok(p) = b.build() {
                    // A line keeps the colour of the lane it runs down to.
                    window.paint_path(p, rgb(lane_color(from.max(to))));
                }
            };
            for &(from, to) in &row.top {
                line(from, to, top, mid);
            }
            for &(from, to) in &row.bottom {
                line(from, to, mid, bottom);
            }
            let r = if merge { 4.5 } else { 3.5 };
            let c = point(x(row.lane), mid);
            let mut dot = PathBuilder::fill();
            dot.move_to(point(c.x - px(r), c.y));
            dot.arc_to(
                point(px(r), px(r)),
                px(0.),
                false,
                true,
                point(c.x + px(r), c.y),
            );
            dot.arc_to(
                point(px(r), px(r)),
                px(0.),
                false,
                true,
                point(c.x - px(r), c.y),
            );
            dot.close();
            if let Ok(p) = dot.build() {
                window.paint_path(p, rgb(lane_color(row.lane)));
            }
        },
    )
    .w(px(width))
    .h(px(ROW))
    .flex_none()
}

impl RunoApp {
    /// Show Git's History tab.
    pub(crate) fn open_git_history(&mut self) {
        self.view = View::GitHistory;
        self.git_tab = View::GitHistory;
        self.load_repo();
        self.load_history();
    }

    /// Read the commits again, with the filter as it is.
    pub(crate) fn load_history(&mut self) {
        if self.view != View::GitHistory {
            return;
        }
        let Some(dir) = self.git_place_dir() else {
            self.git_history = None;
            return;
        };
        let filter = self.git_log_filter.clone();
        self.bg(
            move || {
                if !git::is_repo(&dir) {
                    return None;
                }
                let rows = git::log(&dir, &filter, LIMIT);
                // Found by words, the commits aren't joined up: dots only.
                let graph = if filter.text.trim().is_empty() {
                    git::graph(&rows)
                } else {
                    vec![GraphRow::default(); rows.len()]
                };
                let lanes = graph
                    .iter()
                    .flat_map(|g| {
                        g.top
                            .iter()
                            .chain(&g.bottom)
                            .map(|(a, b)| *a.max(b) + 1)
                            .chain([g.lane + 1])
                    })
                    .max()
                    .unwrap_or(1);
                Some(History {
                    branch: git::current_branch(&dir),
                    dir,
                    rows,
                    graph,
                    lanes,
                })
            },
            |this, loaded, cx| {
                match loaded {
                    Some(history) if Some(&history.dir) == this.git_place_dir().as_ref() => {
                        let keep = this
                            .git_commit_sel
                            .as_ref()
                            .filter(|s| history.rows.iter().any(|r| r.sha == **s))
                            .cloned();
                        let pick = keep.or_else(|| history.rows.first().map(|r| r.sha.clone()));
                        this.git_history = Some(history);
                        match pick {
                            Some(sha) => this.pick_commit(sha),
                            None => {
                                this.git_commit_sel = None;
                                this.git_commit = None;
                            }
                        }
                    }
                    Some(_) => {}
                    None => this.git_history = None,
                }
                cx.notify();
            },
        );
    }

    /// Pick a commit, and read its files and where it stands.
    fn pick_commit(&mut self, sha: String) {
        if self.git_commit_sel.as_ref() != Some(&sha) {
            self.git_go_back = None;
            self.git_naming = None;
        }
        self.git_commit_sel = Some(sha.clone());
        let Some(dir) = self.git_history.as_ref().map(|h| h.dir.clone()) else {
            return;
        };
        self.bg(
            move || {
                let here = git::is_ancestor(&dir, &sha, "HEAD");
                Picked {
                    info: git::commit_info(&dir, &sha),
                    after: if here {
                        git::commits_ahead(&dir, "HEAD", &sha)
                    } else {
                        0
                    },
                    here,
                }
            },
            |this, picked, cx| {
                if this.git_commit_sel.as_ref() == Some(&picked.info.sha) {
                    this.git_commit = Some(picked);
                }
                cx.notify();
            },
        );
    }

    /// Show only the commits that touch `path`, or all of them again.
    fn filter_path(&mut self, path: String) {
        self.git_log_filter.path = path;
        self.load_history();
    }

    pub(crate) fn on_git_log_search(&mut self, event: &InputEvent, cx: &mut Context<Self>) {
        if matches!(event, InputEvent::Change) {
            let text = self.git_log_search.read(cx).value().trim().to_string();
            if text != self.git_log_filter.text {
                self.git_log_filter.text = text;
                self.load_history();
            }
        }
    }

    /// Ask the default agent what a commit did, in a few plain sentences.
    fn explain_commit(&mut self, sha: String, cx: &mut Context<Self>) {
        let Some(dir) = self.git_history.as_ref().map(|h| h.dir.clone()) else {
            return;
        };
        let patch = git::commit_patch(&dir, &sha, 16_000);
        let prompt = format!(
            "Explain what this git commit does for someone who doesn't write code. Reply with \
             two or three short, plain sentences about what changes for the people using the \
             project, not how the code does it. No file names unless they help, no code, no \
             lists, no preamble.\n\n{patch}"
        );
        let st = cx.global::<AppSettings>();
        let provider = st.default_provider.clone();
        let configured = st.agent(&provider).binary;
        let mut command = agent::ask_command(&provider, &configured, &prompt);
        // A small, fast model is plenty for a few sentences.
        if provider == "claude" {
            let args: Vec<_> = command.get_args().map(|a| a.to_owned()).collect();
            command = std::process::Command::new(command.get_program());
            command.args(["--model", "haiku"]).args(args);
        }
        agent::oneshot(format!("explain:{sha}"), command, &dir, self.tx.clone());
        self.git_explain.insert(sha, None);
    }

    /// An agent's account of a commit, or why there isn't one.
    pub(crate) fn handle_explain(&mut self, key: &str, result: Result<String, String>) {
        let Some(sha) = key.strip_prefix("explain:") else {
            return;
        };
        match result.map(|t| t.trim().to_string()) {
            Ok(text) if !text.is_empty() => {
                self.git_explain.insert(sha.to_string(), Some(text));
            }
            Ok(_) => {
                self.git_explain.remove(sha);
                self.fail("git", "The agent didn't say what the commit does.".into());
            }
            Err(error) => {
                self.git_explain.remove(sha);
                let first = error.lines().next().unwrap_or("").to_string();
                self.fail("git", format!("Couldn't explain the commit: {first}"));
            }
        }
    }

    /// Copy the picked commit onto another branch.
    fn copy_commit(&mut self, sha: String, subject: String, branch: String) {
        self.git_step("Copying…", move |dir| {
            let undo = git::branches_point(dir, &[&branch]);
            let folder = git::folder_point(dir).filter(|_| git::current_branch(dir) == branch);
            git::copy_to(dir, &sha, &branch)?;
            Ok(Some((
                format!("Copied “{subject}” to {branch}."),
                Some(match folder {
                    Some(folder) => folder,
                    None => undo,
                }),
            )))
        });
    }

    fn commit_row(
        &self,
        i: usize,
        row: &LogRow,
        data: &History,
        remote: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.git_commit_sel.as_deref() == Some(row.sha.as_str());
        let sha = row.sha.clone();
        let graph = data.graph.get(i).cloned().unwrap_or_default();
        div()
            .id(("git-commit-row", i))
            .h(px(ROW))
            .pl_1()
            .pr_3()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.5))
            .when(on, |el| el.bg(tint(c_active())))
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pick_commit(sha.clone());
                cx.notify();
            }))
            .child(graph_cell(graph, data.lanes, row.parents.len() > 1))
            .children(ref_names(&row.refs, remote).iter().map(ref_chip))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.subject.clone()),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_none()
                    .truncate()
                    .text_color(rgb(c_secondary()))
                    .child(row.author.clone()),
            )
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .font_family("JetBrains Mono")
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(row.short.clone()),
            )
            .child(
                div()
                    .w(px(96.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(row.when.clone()),
            )
            .into_any_element()
    }

    /// The History page: the filter, the commit graph, and the picked commit beside it.
    pub(crate) fn git_history_view(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.git_time_on {
            return self.time_machine_view(cx);
        }
        let Some(data) = self.git_history.as_ref() else {
            let text = if self.git_place_dir().is_none() {
                "Pick a project in the top bar to see its history."
            } else {
                "Reading history…"
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
        let remote = self
            .git_repo
            .as_ref()
            .map(|r| r.state.remote.clone())
            .unwrap_or_default();
        let all = self.git_log_filter.all;
        let scope = |id: &'static str, label: String, on: bool, all: bool| {
            div()
                .id(id)
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                .when(!on, |el| el.text_color(rgb(c_muted())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.git_log_filter.all = all;
                    this.load_history();
                    cx.notify();
                }))
                .child(label)
        };
        let bar = div()
            .h(px(48.))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.git_log_search).bordered(true).h(px(30.))),
            )
            .child(
                div()
                    .flex_none()
                    .p(px(2.))
                    .flex()
                    .gap(px(2.))
                    .rounded_md()
                    .border_1()
                    .border_color(divider())
                    .child(scope("git-log-all", "All branches".into(), all, true))
                    .child(scope(
                        "git-log-here",
                        format!("Just {}", data.branch),
                        !all,
                        false,
                    )),
            )
            .child(
                self.button("git-time-on", "Time machine", false)
                    .h(px(28.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_time_machine(true);
                        cx.notify();
                    })),
            )
            .when(!self.git_log_filter.path.is_empty(), |el| {
                el.child(
                    div()
                        .id("git-log-path")
                        .flex_none()
                        .max_w(px(220.))
                        .h(px(28.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded_md()
                        .bg(rgba((c_blue() << 8) | 0x22))
                        .text_size(px(12.))
                        .text_color(rgb(c_blue()))
                        .cursor_pointer()
                        .child(icon(IconName::File, 12., c_blue()))
                        .child(div().truncate().child(self.git_log_filter.path.clone()))
                        .child(icon(IconName::X, 12., c_blue()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.filter_path(String::new());
                            cx.notify();
                        })),
                )
            });
        let rows: Vec<AnyElement> = data
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| self.commit_row(i, row, data, &remote, cx))
            .collect();
        let empty = rows.is_empty();
        let list = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .bg(pane(c_sheet()))
            .child(bar)
            .child(
                div()
                    .id("git-log")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(12.))
                    .children(rows)
                    .when(empty, |el| {
                        el.child(
                            div()
                                .p_4()
                                .text_size(px(12.5))
                                .text_color(rgb(c_muted()))
                                .child(
                                    if self.git_log_filter == git::LogFilter::default()
                                        || self.git_log_filter
                                            == (git::LogFilter {
                                                all: true,
                                                ..Default::default()
                                            })
                                    {
                                        "No commits yet."
                                    } else {
                                        "No commits match. Try other words, or clear the file."
                                    },
                                ),
                        )
                    })
                    .when(data.rows.len() >= LIMIT, |el| {
                        el.child(
                            div()
                                .px_4()
                                .py_3()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(format!(
                                    "Showing the latest {LIMIT}. Search to find older ones."
                                )),
                        )
                    }),
            );
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(list)
            .child(
                div()
                    .w(px(PAGE_WIDTH))
                    .flex_none()
                    .h_full()
                    .border_l_1()
                    .border_color(divider())
                    .bg(glass_fill(c_canvas()))
                    .child(self.commit_page(data, &remote, cx)),
            )
            .into_any_element()
    }

    fn commit_page(&self, data: &History, remote: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self
            .git_commit_sel
            .as_ref()
            .and_then(|s| data.rows.iter().find(|r| r.sha == *s))
            .cloned()
        else {
            return div().into_any_element();
        };
        let picked = self.git_commit.as_ref().filter(|p| p.info.sha == row.sha);
        let sha = row.sha.clone();
        // What the agent said, if it was asked.
        let explain = match self.git_explain.get(&row.sha) {
            Some(Some(text)) => div()
                .text_size(px(13.))
                .line_height(px(20.))
                .child(text.clone())
                .into_any_element(),
            Some(None) => div()
                .text_size(px(12.5))
                .text_color(rgb(c_muted()))
                .child("Reading the change…")
                .into_any_element(),
            None => div()
                .id("git-explain")
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.5))
                .text_color(rgb(c_blue()))
                .cursor_pointer()
                .hover(|el| el.underline())
                .child("Ask the agent what this did")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.explain_commit(sha.clone(), cx);
                    cx.notify();
                }))
                .into_any_element(),
        };
        let mut files = div().v_flex();
        let (mut added, mut removed) = (0, 0);
        if let Some(p) = picked {
            for (i, f) in p.info.files.iter().enumerate() {
                added += f.added;
                removed += f.removed;
                let path = f.path.clone();
                files = files.child(
                    div()
                        .id(("git-commit-file", i))
                        .h(px(28.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .text_size(px(12.5))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.filter_path(path.clone());
                            cx.notify();
                        }))
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
        let n_files = picked.map_or(0, |p| p.info.files.len());
        let body = picked.map(|p| p.info.body.clone()).unwrap_or_default();
        let here = picked.is_some_and(|p| p.here);
        let after = picked.map_or(0, |p| p.after);
        let branch = data.branch.clone();
        let going_back = self.git_go_back.as_ref() == Some(&row.sha);
        // The steps, each with its plain name, what it does, and its git word.
        let mut actions: Vec<(&'static str, IconName, String, String, &'static str)> = Vec::new();
        if here {
            actions.push((
                "revert",
                IconName::Undo2,
                "Undo this commit".into(),
                format!("Adds a commit to {branch} that takes it back."),
                "revert",
            ));
        }
        actions.push((
            "copy",
            IconName::Copy,
            "Copy to another branch…".into(),
            "Bring just this change somewhere else.".into(),
            "cherry-pick",
        ));
        actions.push((
            "branch",
            IconName::GitBranchPlus,
            "Start a branch from here".into(),
            "Try something new from this point.".into(),
            "branch",
        ));
        actions.push((
            "tag",
            IconName::Tag,
            "Mark as a release…".into(),
            "Give this point a version name, like v1.2.".into(),
            "tag",
        ));
        if here && after > 0 {
            actions.push((
                "back",
                IconName::Rewind,
                if going_back {
                    "Click again to go back".into()
                } else {
                    "Go back to this point…".into()
                },
                if going_back {
                    format!(
                        "Drops the {} after it from {branch}. Changes you haven't committed stay. You can undo this for 30 minutes.",
                        commits(after)
                    )
                } else {
                    format!("Drops the {} after it from {branch}. Asks first.", commits(after))
                },
                "reset",
            ));
        }
        let copy_menu = self.menu.as_deref() == Some("git-copy");
        let mut list = div().v_flex().gap(px(2.));
        for (key, glyph, title, sub, term) in actions {
            let (sha, subject, short) = (row.sha.clone(), row.subject.clone(), row.short.clone());
            let warn = key == "back" && going_back;
            list = list.child(
                div()
                    .id(SharedString::from(format!("git-commit-{key}")))
                    .relative()
                    .p(px(8.))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(warn, |el| {
                        el.bg(rgba((c_amber() << 8) | 0x1f))
                            .border_1()
                            .border_color(rgba((c_amber() << 8) | 0x66))
                    })
                    .when(!warn, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(tint(c_subtle()))
                            .child(icon(
                                glyph,
                                13.,
                                if key == "back" {
                                    c_amber()
                                } else {
                                    c_secondary()
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
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
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(title),
                                    )
                                    .child(git_term(term)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .child(sub),
                            ),
                    )
                    .when(key == "copy" && copy_menu, |el| {
                        el.child(self.git_copy_menu(&row, cx))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        match key {
                            "revert" => {
                                let (sha, subject) = (sha.clone(), subject.clone());
                                this.git_step("Undoing the commit…", move |dir| {
                                    let undo = git::folder_point(dir);
                                    git::revert(dir, &sha)?;
                                    Ok(Some((
                                        format!("Took back “{subject}” with a new commit."),
                                        undo,
                                    )))
                                });
                            }
                            "copy" => {
                                this.menu = if this.menu.as_deref() == Some("git-copy") {
                                    None
                                } else {
                                    Some("git-copy".into())
                                };
                            }
                            "branch" => {
                                this.start_naming_at(Naming::BranchAt(sha.clone()), window, cx)
                            }
                            "tag" => this.start_naming_at(Naming::Tag(sha.clone()), window, cx),
                            "back" if this.git_go_back.as_ref() == Some(&sha) => {
                                this.git_go_back = None;
                                let (sha, short) = (sha.clone(), short.clone());
                                this.git_step("Going back…", move |dir| {
                                    let undo = git::folder_point(dir);
                                    let dropped = git::go_back(dir, &sha)?;
                                    Ok(Some((
                                        format!(
                                            "Went back to {short}, dropping {}.",
                                            commits(dropped)
                                        ),
                                        undo,
                                    )))
                                });
                            }
                            "back" => this.git_go_back = Some(sha.clone()),
                            _ => {}
                        }
                        cx.notify();
                    })),
            );
        }
        let naming = matches!(
            &self.git_naming,
            Some(Naming::BranchAt(s) | Naming::Tag(s)) if *s == row.sha
        );
        div()
            .id("git-commit-page")
            .size_full()
            .overflow_y_scroll()
            .px(px(20.))
            .py(px(18.))
            .v_flex()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .children(ref_names(&row.refs, remote).iter().map(ref_chip))
                    .child(
                        div()
                            .px(px(6.))
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .bg(tint(c_subtle()))
                            .font_family("JetBrains Mono")
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(row.short.clone()),
                    ),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(row.subject.clone()),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child(format!("{}  ·  {}", row.author, row.when)),
            )
            .when(!body.is_empty(), |el| {
                el.child(
                    div()
                        .mt_3()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child(body),
                )
            })
            .child(
                div()
                    .mt_4()
                    .p(px(14.))
                    .v_flex()
                    .gap(px(6.))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgba((c_blue() << 8) | 0x55))
                    .bg(rgba((c_blue() << 8) | 0x14))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_blue()))
                            .child(icon(IconName::Sparkles, 11., c_blue()))
                            .child("IN PLAIN WORDS"),
                    )
                    .child(explain),
            )
            .when(n_files > 0, |el| {
                el.child(
                    div()
                        .mt(px(20.))
                        .mb(px(4.))
                        .flex()
                        .gap(px(6.))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_muted()))
                        .child(format!("{n_files} FILE{}", if n_files == 1 { "" } else { "S" }))
                        .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                        .child(div().text_color(rgb(c_del())).child(format!("−{removed}")))
                        .child(div().flex_1())
                        .child(div().font_weight(FontWeight::NORMAL).child("Click one for its history")),
                )
                .child(files)
            })
            .child(
                div()
                    .mt(px(20.))
                    .mb(px(4.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_muted()))
                    .child("DO SOMETHING WITH IT"),
            )
            .when(naming, |el| {
                el.child(
                    div()
                        .mb_2()
                        .v_flex()
                        .gap_1()
                        .child(Input::new(&self.git_name).bordered(true).h(px(28.)))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .child(match &self.git_naming {
                                    Some(Naming::Tag(_)) => format!("A release name for {}, like v1.2, then Enter.", row.short),
                                    _ => format!("A name for the new branch, then Enter. It starts at {} and this folder stays where it is.", row.short),
                                }),
                        ),
                )
            })
            .child(list)
            .into_any_element()
    }

    fn start_naming_at(&mut self, naming: Naming, window: &mut Window, cx: &mut Context<Self>) {
        let hint = match naming {
            Naming::Tag(_) => "Release name",
            _ => "Branch name",
        };
        self.git_naming = Some(naming);
        self.git_name.update(cx, |input, cx| {
            input.set_placeholder(hint, window, cx);
            input.set_value(String::new(), window, cx);
            input.focus(window, cx);
        });
    }

    /// The branches the picked commit can be copied to.
    fn git_copy_menu(&self, row: &LogRow, cx: &mut Context<Self>) -> Div {
        let Some(repo) = self.git_repo.as_ref() else {
            return div();
        };
        let here = self.git_commit.as_ref().is_some_and(|p| p.here);
        let mut list = div().v_flex().child(
            div()
                .px_2()
                .pt_1()
                .pb(px(6.))
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child("Copy this commit onto the end of:"),
        );
        let targets: Vec<&String> = repo
            .state
            .branches
            .iter()
            .filter(|b| !(here && **b == repo.state.branch))
            .collect();
        for (i, name) in targets.iter().enumerate() {
            let (target, sha, subject) = ((*name).clone(), row.sha.clone(), row.subject.clone());
            list = list.child(
                div()
                    .id(("git-copy-pick", i))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .text_size(px(13.))
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.copy_commit(sha.clone(), subject.clone(), target.clone());
                        cx.notify();
                    }))
                    .child(icon(IconName::GitBranch, 13., c_muted()))
                    .child(div().flex_1().min_w_0().truncate().child((*name).clone())),
            );
        }
        if targets.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child("There's no other branch yet."),
            );
        }
        self.git_popover("git-copy-menu", 260., list, cx)
    }
}
