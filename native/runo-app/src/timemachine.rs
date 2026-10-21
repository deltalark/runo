//! Git's time machine, in the History tab: every step this folder took (commits,
//! switches, syncs, go backs) and every snapshot Runo made before an agent's turn, by
//! day, each with a way back to it that Undo can take back in turn.

use super::*;
use git::{Moment, MomentKind};

/// How many of git's steps the time machine reads.
const LIMIT: usize = 400;
/// Width of the picked moment's page.
const PAGE_WIDTH: f32 = 400.;

/// What the picked moment is, against now: read off the UI thread.
#[derive(Clone, Default)]
pub(crate) struct Against {
    pub commit: String,
    /// Files that differ from the folder now.
    pub files: Vec<String>,
    /// The moment's branch is still there.
    pub kept: bool,
    /// The folder is still at this moment's commit, on its branch.
    pub now: bool,
}

/// "Today", "Yesterday", or the date, for a day's heading.
fn day_name(at: i64) -> String {
    let Some(when) = chrono::DateTime::from_timestamp(at, 0) else {
        return String::new();
    };
    let day = when.with_timezone(&chrono::Local).date_naive();
    let today = chrono::Local::now().date_naive();
    match (today - day).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => day.format("%A, %b %-d").to_string(),
    }
}

/// The time of day, "14:32".
fn clock(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

/// A moment's icon, from what happened.
fn moment_icon(m: &Moment) -> (IconName, u32) {
    if m.kind == MomentKind::Snapshot {
        return (IconName::Bot, c_blue());
    }
    let l = m.label.as_str();
    if l.starts_with("Committed") || l.starts_with("First commit") || l.starts_with("Changed") {
        (IconName::GitCommitHorizontal, c_green())
    } else if l.starts_with("Switched") {
        (IconName::ArrowLeftRight, c_secondary())
    } else if l.starts_with("Moved back") || l.starts_with("Took back") {
        (IconName::Rewind, c_amber())
    } else if l.starts_with("Synced") {
        (IconName::RefreshCw, c_secondary())
    } else if l.starts_with("Brought in")
        || l.starts_with("Finished bringing")
        || l.starts_with("Put your commits")
    {
        (IconName::GitMerge, c_secondary())
    } else {
        (IconName::Dot, c_muted())
    }
}

impl RunoApp {
    /// Turn the time machine on or off.
    pub(crate) fn show_time_machine(&mut self, on: bool) {
        self.git_time_on = on;
        self.git_travel = None;
        if on {
            self.load_time();
        }
    }

    /// The snapshots agents' turns left in `dir`, with what each was taken before.
    fn snapshot_labels(&self, dir: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for s in self.state.sessions.iter().filter(|s| s.dir == dir) {
            for m in &s.messages {
                if m.role == Role::User && !m.checkpoint.is_empty() {
                    let prompt: String = m
                        .text
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(70)
                        .collect();
                    out.push((
                        m.checkpoint.clone(),
                        format!("Before an agent turn: \u{201c}{prompt}\u{201d}"),
                    ));
                }
                if !m.redo.is_empty() {
                    out.push((
                        m.redo.clone(),
                        format!("Before you undid a turn in {}", s.title),
                    ));
                }
            }
        }
        out
    }

    /// Read the folder's moments again, when the time machine is showing.
    pub(crate) fn load_time(&mut self) {
        if self.demo || !self.git_time_on || self.view != View::GitHistory {
            return;
        }
        let Some(dir) = self.git_place_dir() else {
            self.git_time = None;
            return;
        };
        let snapshots = self.snapshot_labels(&dir);
        self.bg(
            move || {
                let list = git::is_repo(&dir).then(|| git::moments(&dir, &snapshots, LIMIT));
                (dir, list)
            },
            |this, (dir, list), cx| {
                if Some(&dir) != this.git_place_dir().as_ref() {
                    return;
                }
                let Some(list) = list else {
                    this.git_time = None;
                    cx.notify();
                    return;
                };
                // Keep the picked moment if it's still there, by what it is.
                let was = this
                    .git_moment_sel
                    .and_then(|i| this.git_time.as_ref()?.1.get(i).cloned());
                let pick = was
                    .and_then(|w| list.iter().position(|m| *m == w))
                    .or((!list.is_empty()).then_some(0));
                this.git_time = Some((dir, list));
                match pick {
                    Some(i) => this.pick_moment(i),
                    None => this.git_moment_sel = None,
                }
                cx.notify();
            },
        );
    }

    /// Pick a moment, and read which files differ from now.
    fn pick_moment(&mut self, i: usize) {
        if self.git_moment_sel != Some(i) {
            self.git_travel = None;
        }
        self.git_moment_sel = Some(i);
        let Some((dir, moment)) = self
            .git_time
            .as_ref()
            .and_then(|(d, l)| Some((d.clone(), l.get(i)?.clone())))
        else {
            return;
        };
        self.bg(
            move || {
                let files = git::differs_from_now(&dir, &moment.commit);
                let kept = moment.branch.is_empty() || git::has_branch(&dir, &moment.branch);
                let now = moment.kind == MomentKind::Step
                    && git::rev(&dir, "HEAD").as_ref() == Some(&moment.commit)
                    && git::current_branch(&dir) == moment.branch;
                Against {
                    commit: moment.commit,
                    files,
                    kept,
                    now,
                }
            },
            |this, files, cx| {
                this.git_moment_files = Some(files);
                cx.notify();
            },
        );
    }

    /// Go back to the picked moment.
    fn travel_to(&mut self, moment: Moment) {
        self.git_travel = None;
        self.git_step("Going back in time…", move |dir| {
            let (note, undo) = git::travel(dir, &moment)?;
            Ok(Some((note, Some(undo))))
        });
    }

    /// The time machine, in place of History's commits.
    pub(crate) fn time_machine_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let bar = div()
            .h(px(48.))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(divider())
            .child(icon(IconName::RotateCcwClock, 15., c_blue()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Time machine"),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .truncate()
                            .child("Every step this folder took, and the files before each agent turn."),
                    ),
            )
            .child(
                self.button("git-time-off", "Back to commits", false)
                    .h(px(28.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_time_machine(false);
                        cx.notify();
                    })),
            );
        let Some((_, list)) = self.git_time.as_ref() else {
            return div()
                .size_full()
                .v_flex()
                .bg(pane(c_sheet()))
                .child(bar)
                .child(
                    div()
                        .p_4()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("Reading this folder's past…"),
                )
                .into_any_element();
        };
        let now = usage::now_secs();
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut day = String::new();
        for (i, m) in list.iter().enumerate() {
            let this_day = day_name(m.at);
            if this_day != day {
                rows.push(
                    div()
                        .px_4()
                        .pt_3()
                        .pb_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_muted()))
                        .child(this_day.to_uppercase())
                        .into_any_element(),
                );
                day = this_day;
            }
            let on = self.git_moment_sel == Some(i);
            let (glyph, color) = moment_icon(m);
            let sub = match (m.kind == MomentKind::Snapshot, m.branch.is_empty()) {
                (true, _) => format!("Snapshot  ·  {}  ·  {}", clock(m.at), usage::ago(m.at, now)),
                (false, true) => format!("No branch  ·  {}", clock(m.at)),
                (false, false) => format!("{}  ·  {}", m.branch, clock(m.at)),
            };
            rows.push(
                div()
                    .id(("git-moment", i))
                    .mx_2()
                    .px_2()
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(on, |el| el.bg(tint(c_active())))
                    .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_moment(i);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .size(px(26.))
                            .flex_none()
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgba((color << 8) | 0x22))
                            .child(icon(glyph, 12., color)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(div().text_size(px(13.)).truncate().child(m.label.clone()))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .truncate()
                                    .child(sub),
                            ),
                    )
                    .into_any_element(),
            );
        }
        let empty = rows.is_empty();
        let left = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .bg(pane(c_sheet()))
            .child(bar)
            .child(
                div()
                    .id("git-moments")
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
                                .child("Nothing has happened in this folder yet."),
                        )
                    }),
            );
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(left)
            .child(
                div()
                    .w(px(PAGE_WIDTH))
                    .flex_none()
                    .h_full()
                    .border_l_1()
                    .border_color(divider())
                    .bg(glass_fill(c_canvas()))
                    .child(self.moment_page(list, cx)),
            )
            .into_any_element()
    }

    /// The picked moment: when it was, what differs from now, and the way back.
    fn moment_page(&self, list: &[Moment], cx: &mut Context<Self>) -> AnyElement {
        let Some((i, m)) = self
            .git_moment_sel
            .and_then(|i| Some((i, list.get(i)?.clone())))
        else {
            return div().into_any_element();
        };
        let here = self
            .git_repo
            .as_ref()
            .map(|r| r.state.branch.clone())
            .unwrap_or_default();
        let read = self
            .git_moment_files
            .as_ref()
            .filter(|a| a.commit == m.commit);
        let files = read.map(|a| a.files.clone());
        let gone = m.kind == MomentKind::Step && read.is_some_and(|a| !a.kept);
        let at_now = read.is_some_and(|a| a.now);
        let (way, what) = match (&m.kind, m.branch.as_str()) {
            (MomentKind::Step, b) if at_now => (
                "",
                format!("This is where {b} is now. Pick an earlier moment to go back to it."),
            ),
            (MomentKind::Snapshot, _) => (
                "Put the files back as they were",
                "Every file goes back to this moment. Runo takes a snapshot of now first, so Undo brings now back.".to_string(),
            ),
            (MomentKind::Step, "") => (
                "",
                "The folder wasn't on a branch then. Find this commit in History to start a branch from it.".to_string(),
            ),
            (MomentKind::Step, b) if b == here => (
                "Go back to here",
                format!("Moves {b} back to this point. Changes you haven't committed stay in the folder."),
            ),
            (MomentKind::Step, b) if gone => (
                "Bring this branch back",
                format!("{b} was deleted after this. It comes back, pointing here."),
            ),
            (MomentKind::Step, b) => (
                "Move that branch back to here",
                format!("Moves {b} to this point. This folder stays where it is."),
            ),
        };
        let confirming = self.git_travel == Some(i);
        let short = m.commit.get(..7).unwrap_or(&m.commit).to_string();
        let when = chrono::DateTime::from_timestamp(m.at, 0)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%A, %b %-d at %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        let mut file_list = div().v_flex();
        match &files {
            None => {
                file_list = file_list.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("Comparing with now…"),
                )
            }
            Some(f) if f.is_empty() => {
                file_list = file_list.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("Every file is the same as now."),
                )
            }
            Some(f) => {
                for path in f.iter().take(40) {
                    file_list = file_list.child(
                        div()
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.5))
                            .child(icon(IconName::File, 12., c_muted()))
                            .child(div().flex_1().min_w_0().truncate().child(path.clone())),
                    );
                }
                if f.len() > 40 {
                    file_list = file_list.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(format!("and {} more", f.len() - 40)),
                    );
                }
            }
        }
        let n = files.as_ref().map_or(0, Vec::len);
        let moment = m.clone();
        div()
            .id("git-moment-page")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .v_flex()
            .gap_4()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(m.label.clone()),
                    )
                    .child(div().text_size(px(12.)).text_color(rgb(c_muted())).child(
                        match m.branch.as_str() {
                            "" => format!("{when}  ·  {short}"),
                            b => format!("{when}  ·  {b}  ·  {short}"),
                        },
                    )),
            )
            .when(!way.is_empty(), |el| {
                el.child(
                    div()
                        .id("git-travel")
                        .p_3()
                        .v_flex()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .when(confirming, |el| {
                            el.bg(rgba((c_amber() << 8) | 0x1f))
                                .border_color(rgba((c_amber() << 8) | 0x66))
                        })
                        .when(!confirming, |el| el.border_color(divider()))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(icon(IconName::Rewind, 13., c_amber()))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(way),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_muted()))
                                .child(what.clone()),
                        )
                        .child(
                            div().flex().child(
                                self.button(
                                    "git-travel-go",
                                    if confirming {
                                        "Click again to go back"
                                    } else {
                                        "Go back"
                                    },
                                    confirming,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if this.git_travel == Some(i) {
                                            this.travel_to(moment.clone());
                                        } else {
                                            this.git_travel = Some(i);
                                        }
                                        cx.notify();
                                    },
                                )),
                            ),
                        ),
                )
            })
            .when(way.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child(what.clone()),
                )
            })
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
                            .child(match n {
                                0 => "DIFFERENT FROM NOW".to_string(),
                                1 => "1 FILE DIFFERENT FROM NOW".to_string(),
                                n => format!("{n} FILES DIFFERENT FROM NOW"),
                            }),
                    )
                    .child(file_list),
            )
            .into_any_element()
    }
}
