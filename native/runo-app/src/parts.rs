//! Changes: the project folder's own uncommitted work, shown file by file and part by
//! part, so a commit can take whole files, single parts or single lines and leave the
//! rest in the folder. Parts can also be thrown away, and every step can be undone.

use super::*;
use branches::git_term;
use git::{FileParts, Undo};
use std::collections::HashSet;

/// Marks a whole-only file (binary, deleted) as left out of the next commit.
const WHOLE: (usize, usize) = (usize::MAX, usize::MAX);

/// The project folder's changed files, read off the UI thread.
#[derive(Clone, Default)]
pub(crate) struct Folder {
    pub dir: String,
    pub branch: String,
    pub files: Vec<FileParts>,
    /// The last commit's subject, and whether it is only on this computer.
    pub last: Option<(String, bool)>,
}

/// How much of a file, or a part, goes into the next commit.
#[derive(Clone, Copy, PartialEq)]
enum Ticked {
    All,
    Some,
    None,
}

fn ticked(included: usize, total: usize) -> Ticked {
    match included {
        0 => Ticked::None,
        n if n >= total => Ticked::All,
        _ => Ticked::Some,
    }
}

/// A tick box that can be full, partly full or empty.
fn tick_box(state: Ticked) -> Div {
    div()
        .size(px(15.))
        .flex_none()
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .when(state != Ticked::None, |el| el.bg(rgb(c_blue())))
        .when(state == Ticked::None, |el| {
            el.border_1().border_color(rgb(c_border()))
        })
        .when(state == Ticked::All, |el| {
            el.child(icon(IconName::Check, 10., 0xffffff))
        })
        .when(state == Ticked::Some, |el| {
            el.child(div().w(px(7.)).h(px(2.)).rounded_full().bg(rgb(0xffffff)))
        })
}

fn parts(n: usize) -> String {
    match n {
        1 => "1 part".into(),
        n => format!("{n} parts"),
    }
}

/// Where a file's path splits into its name and its folder.
fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (name.to_string(), dir.to_string()),
        None => (path.to_string(), String::new()),
    }
}

impl RunoApp {
    /// Read the project folder's changes again.
    pub(crate) fn load_folder(&mut self) {
        if self.demo {
            return;
        }
        let Some(dir) = self.git_place_dir() else {
            self.git_folder = None;
            return;
        };
        self.bg(
            move || {
                if !git::is_repo(&dir) {
                    return None;
                }
                Some(Folder {
                    branch: git::current_branch(&dir),
                    files: git::file_parts(&dir),
                    last: git::last_commit(&dir),
                    dir,
                })
            },
            |this, loaded, cx| {
                match loaded {
                    Some(folder) if Some(&folder.dir) == this.git_place_dir().as_ref() => {
                        // Lines left out stay out when the file changes around them,
                        // found again by what they say.
                        let old = this.git_folder.take();
                        let left_out = std::mem::take(&mut this.git_left_out);
                        for (path, out) in left_out {
                            let now = folder.files.iter().find(|f| f.change.path == path);
                            let then = old
                                .as_ref()
                                .and_then(|o| o.files.iter().find(|f| f.change.path == path));
                            let (Some(now), Some(then)) = (now, then) else {
                                continue;
                            };
                            let kept: HashSet<(usize, usize)> = if now.diff == then.diff {
                                out
                            } else {
                                let gone: Vec<(char, &str)> = out
                                    .iter()
                                    .filter_map(|(p, l)| then.parts.get(*p)?.lines.get(*l))
                                    .map(|line| (line.kind, line.text.as_str()))
                                    .collect();
                                Self::changed_lines(now)
                                    .into_iter()
                                    .filter(|(p, l)| {
                                        let line = &now.parts[*p].lines[*l];
                                        gone.contains(&(line.kind, line.text.as_str()))
                                    })
                                    .collect()
                            };
                            if !kept.is_empty() {
                                this.git_left_out.insert(path, kept);
                            }
                        }
                        let picked = this.git_folder_file.clone();
                        if !folder
                            .files
                            .iter()
                            .any(|f| Some(&f.change.path) == picked.as_ref())
                        {
                            this.git_folder_file =
                                folder.files.first().map(|f| f.change.path.clone());
                        }
                        // With nothing else to review, open on the folder.
                        if !folder.files.is_empty() && this.review_list().is_empty() {
                            this.git_folder_open = true;
                        }
                        if folder.files.is_empty() && this.review_list().is_empty() {
                            this.git_folder_open = false;
                        }
                        this.git_folder = Some(folder);
                    }
                    Some(_) => {}
                    None => this.git_folder = None,
                }
                cx.notify();
            },
        );
    }

    /// Open the project folder's changes in Changes.
    fn open_folder(&mut self) {
        self.git_folder_open = true;
        self.load_folder();
    }

    fn line_in(&self, path: &str, part: usize, line: usize) -> bool {
        !self
            .git_left_out
            .get(path)
            .is_some_and(|out| out.contains(&(part, line)))
    }

    /// The lines of a file that can be ticked: (part, line).
    fn changed_lines(file: &FileParts) -> Vec<(usize, usize)> {
        file.parts
            .iter()
            .enumerate()
            .flat_map(|(p, part)| {
                part.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| matches!(l.kind, '+' | '-'))
                    .map(move |(l, _)| (p, l))
            })
            .collect()
    }

    fn file_ticked(&self, file: &FileParts) -> Ticked {
        let path = &file.change.path;
        if file.whole_only() {
            return if self.line_in(path, WHOLE.0, WHOLE.1) {
                Ticked::All
            } else {
                Ticked::None
            };
        }
        let lines = Self::changed_lines(file);
        let included = lines
            .iter()
            .filter(|(p, l)| self.line_in(path, *p, *l))
            .count();
        ticked(included, lines.len())
    }

    fn part_ticked(&self, file: &FileParts, part: usize) -> Ticked {
        let lines: Vec<_> = Self::changed_lines(file)
            .into_iter()
            .filter(|(p, _)| *p == part)
            .collect();
        let included = lines
            .iter()
            .filter(|(p, l)| self.line_in(&file.change.path, *p, *l))
            .count();
        ticked(included, lines.len())
    }

    /// Tick or untick these lines of a file together: all in when any were out.
    fn flip_lines(&mut self, path: &str, lines: Vec<(usize, usize)>) {
        let all_in = lines.iter().all(|(p, l)| self.line_in(path, *p, *l));
        let out = self.git_left_out.entry(path.to_string()).or_default();
        for key in lines {
            if all_in {
                out.insert(key);
            } else {
                out.remove(&key);
            }
        }
    }

    fn flip_file(&mut self, path: &str) {
        let Some(file) = self.folder_file(path) else {
            return;
        };
        let lines = if file.whole_only() {
            vec![WHOLE]
        } else {
            Self::changed_lines(&file)
        };
        self.flip_lines(path, lines);
    }

    fn folder_file(&self, path: &str) -> Option<FileParts> {
        self.git_folder
            .as_ref()?
            .files
            .iter()
            .find(|f| f.change.path == path)
            .cloned()
    }

    /// Parts picked for the next commit, out of all of them.
    fn picked_parts(&self) -> (usize, usize) {
        let Some(folder) = self.git_folder.as_ref() else {
            return (0, 0);
        };
        folder.files.iter().fold((0, 0), |(picked, total), file| {
            if file.whole_only() {
                let on = self.file_ticked(file) == Ticked::All;
                (picked + on as usize, total + 1)
            } else {
                let on = (0..file.parts.len())
                    .filter(|p| self.part_ticked(file, *p) != Ticked::None)
                    .count();
                (picked + on, total + file.parts.len())
            }
        })
    }

    /// Commit what is ticked, or add it to the last commit.
    fn commit_folder(&mut self, amend: bool, cx: &mut Context<Self>) {
        let Some(folder) = self.git_folder.clone() else {
            return;
        };
        let message = self.git_folder_msg.read(cx).value().trim().to_string();
        if !amend && message.is_empty() {
            self.fail(
                "git",
                "Write a line about what this commit does first.".into(),
            );
            return;
        }
        let mut whole = Vec::new();
        let mut patch = String::new();
        for file in &folder.files {
            match self.file_ticked(file) {
                Ticked::None => {}
                Ticked::All => whole.push(file.change.path.clone()),
                Ticked::Some => {
                    let path = &file.change.path;
                    if let Some(p) = git::picked_patch(file, |p, l| self.line_in(path, p, l)) {
                        patch.push_str(&p);
                    }
                }
            }
        }
        if whole.is_empty() && patch.is_empty() {
            self.fail(
                "git",
                "Tick at least one file, part or line to commit.".into(),
            );
            return;
        }
        let (picked, total) = self.picked_parts();
        // The box empties now, and gets the message back if the commit fails.
        self.git_folder_msg_set = Some(String::new());
        self.git_folder_msg_kept = Some(message.clone());
        self.git_step(
            if amend { "Adding…" } else { "Committing…" },
            move |dir| {
                let head = git::head(dir);
                git::commit_parts(dir, &whole, &patch, &message, amend)?;
                let what = if picked < total {
                    format!("{} of {}", picked, parts(total))
                } else {
                    "everything".to_string()
                };
                let note = if amend {
                    format!("Added {what} to the last commit.")
                } else {
                    format!("Committed {what}: “{message}”.")
                };
                Ok(Some((note, head.map(|head| Undo::Uncommit { head }))))
            },
        );
    }

    /// Throw one part of a file out of the folder.
    fn throw_away_part(&mut self, path: &str, part: usize) {
        let Some(file) = self.folder_file(path) else {
            return;
        };
        let Some(patch) = git::picked_patch(&file, |p, _| p == part) else {
            return;
        };
        let name = split_path(path).0;
        self.git_step("Throwing away…", move |dir| {
            git::throw_away(dir, &patch)?;
            Ok(Some((
                format!("Threw away a part of {name}."),
                Some(Undo::Reapply(patch)),
            )))
        });
    }

    /// The This folder row at the top of the Changes list.
    pub(crate) fn folder_row(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let folder = self.git_folder.as_ref().filter(|f| !f.files.is_empty())?;
        let on = self.git_folder_open;
        let (added, removed) = folder.files.iter().fold((0, 0), |a, f| {
            (a.0 + f.change.added, a.1 + f.change.removed)
        });
        let name = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let n = folder.files.len();
        Some(
            div()
                .v_flex()
                .child(
                    div()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_muted()))
                        .child("THIS FOLDER"),
                )
                .child(
                    div()
                        .id("review-folder-row")
                        .relative()
                        .mt(px(8.))
                        .ml(px(10.))
                        .mr_1()
                        .px_2()
                        .py(px(7.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .when(on, |el| el.child(shell::row_fuse("review-folder-fuse", 4.)))
                        .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_folder();
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
                                .bg(tint(c_subtle()))
                                .child(icon(IconName::Folder, 13., c_secondary())),
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
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(11.5))
                                        .text_color(rgb(c_muted()))
                                        .child(format!(
                                            "{}  ·  {n} file{} you changed",
                                            folder.branch,
                                            if n == 1 { "" } else { "s" }
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .gap_1()
                                .text_size(px(11.5))
                                .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                                .child(div().text_color(rgb(c_del())).child(format!("−{removed}"))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The folder's page: the commit box, the files, and the picked file's parts.
    pub(crate) fn folder_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(folder) = self.git_folder.as_ref() else {
            return div().into_any_element();
        };
        let name = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let (picked, total) = self.picked_parts();
        let everything = picked == total;
        let commit_label = if picked == 0 {
            "Tick what to commit".to_string()
        } else if everything {
            match folder.files.len() {
                1 => "Commit the file".to_string(),
                n => format!("Commit all {n} files"),
            }
        } else {
            format!("Commit {picked} of {}", parts(total))
        };
        let (added, removed) = folder.files.iter().fold((0, 0), |a, f| {
            (a.0 + f.change.added, a.1 + f.change.removed)
        });
        let amendable = folder.last.clone().filter(|(_, local)| *local);
        let mut files = div().v_flex().gap(px(2.));
        for (i, file) in folder.files.iter().enumerate() {
            let path = file.change.path.clone();
            let (fname, fdir) = split_path(&path);
            let on = self.git_folder_file.as_ref() == Some(&path);
            let state = self.file_ticked(file);
            let n_parts = file.parts.len();
            let picked_here = (0..n_parts)
                .filter(|p| self.part_ticked(file, *p) != Ticked::None)
                .count();
            let (flip, pick) = (path.clone(), path.clone());
            files = files.child(
                div()
                    .id(("folder-file", i))
                    .h(px(34.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(on, |el| el.bg(tint(c_active())))
                    .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.git_folder_file = Some(pick.clone());
                        cx.notify();
                    }))
                    .child(
                        div()
                            .id(("folder-file-tick", i))
                            .p(px(3.))
                            .child(tick_box(state))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.flip_file(&flip);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(px(12.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(match file.change.status.as_str() {
                                "A" | "?" | "??" => c_add(),
                                "D" => c_del(),
                                _ => c_amber(),
                            }))
                            .child(match file.change.status.as_str() {
                                "??" => "A".to_string(),
                                s => s.chars().next().unwrap_or('M').to_string(),
                            }),
                    )
                    .child(div().flex_none().child(fname))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(fdir),
                    )
                    .when(state == Ticked::Some && n_parts > 1, |el| {
                        el.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(format!("{picked_here} of {}", parts(n_parts))),
                        )
                    })
                    .when(!file.change.binary, |el| {
                        el.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_add()))
                                .child(format!("+{}", file.change.added)),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_del()))
                                .child(format!("−{}", file.change.removed)),
                        )
                    }),
            );
        }
        let shown = self
            .git_folder_file
            .as_ref()
            .and_then(|p| folder.files.iter().find(|f| f.change.path == *p));
        let parts_view = shown.map(|file| self.file_parts_view(file, cx));
        let empty = folder.files.is_empty();
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
                    .child(icon(IconName::Folder, 15., c_muted()))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
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
                            .child(folder.branch.clone()),
                    ),
            )
            .child(
                div()
                    .id("folder-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.))
                    .py(px(16.))
                    .v_flex()
                    .when(empty, |el| {
                        el.child(
                            div()
                                .py_4()
                                .text_size(px(13.))
                                .text_color(rgb(c_muted()))
                                .child("Nothing to commit. The folder matches its last commit."),
                        )
                    })
                    .when(!empty, |el| {
                        el.child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            Input::new(&self.git_folder_msg)
                                                .bordered(true)
                                                .h(px(34.)),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .mt_2()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .id("folder-commit")
                                        .flex_1()
                                        .h(px(34.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(rgb(c_blue()))
                                        .text_color(rgb(0xffffff))
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .hover(|el| el.opacity(0.9))
                                        .when(picked == 0, |el| el.opacity(0.5))
                                        .child(commit_label)
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .rounded(px(4.))
                                                .bg(rgba(0xffffff26))
                                                .font_family("JetBrains Mono")
                                                .text_size(px(10.5))
                                                .child("commit"),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.commit_folder(false, cx);
                                            cx.notify();
                                        })),
                                )
                                .when_some(amendable, |el, (subject, _)| {
                                    el.child(
                                        div()
                                            .id("folder-amend")
                                            .flex_none()
                                            .h(px(34.))
                                            .px_3()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .rounded_md()
                                            .border_1()
                                            .border_color(divider())
                                            .cursor_pointer()
                                            .text_size(px(12.5))
                                            .hover(|el| el.bg(tint(c_subtle())))
                                            .child(icon(IconName::Plus, 12., c_secondary()))
                                            .child(
                                                div()
                                                    .max_w(px(220.))
                                                    .truncate()
                                                    .child(format!("Add to “{subject}”")),
                                            )
                                            .child(git_term("amend").self_center())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.commit_folder(true, cx);
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .mt(px(20.))
                                .mb(px(6.))
                                .flex()
                                .gap(px(6.))
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(c_muted()))
                                .child(format!("CHANGES {}", folder.files.len()))
                                .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                                .child(div().text_color(rgb(c_del())).child(format!("−{removed}"))),
                        )
                        .child(files)
                        .children(parts_view)
                        .child(
                            div()
                                .mt_3()
                                .text_size(px(12.))
                                .text_color(rgb(c_muted()))
                                .child("Tick a file, a part or single lines to put them in this commit. What you leave unticked stays in your folder."),
                        )
                    }),
            )
            .into_any_element()
    }

    /// A file's parts, each with its tick box, Throw away, and its lines.
    fn file_parts_view(&self, file: &FileParts, cx: &mut Context<Self>) -> AnyElement {
        let path = file.change.path.clone();
        if file.whole_only() {
            let text = if file.change.binary {
                "This file isn't text, so it goes in whole or not at all."
            } else if file.change.status == "D" {
                "This file is deleted. The deletion goes in whole or not at all."
            } else {
                "This file goes in whole or not at all."
            };
            return div()
                .mt_3()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(divider())
                .text_size(px(12.5))
                .text_color(rgb(c_muted()))
                .child(text)
                .into_any_element();
        }
        let mut out = div().mt_3().v_flex().gap(px(10.));
        for (p, part) in file.parts.iter().enumerate() {
            let state = self.part_ticked(file, p);
            let (flip_path, throw_path) = (path.clone(), path.clone());
            let lines_of_part: Vec<(usize, usize)> = part
                .lines
                .iter()
                .enumerate()
                .filter(|(_, l)| matches!(l.kind, '+' | '-'))
                .map(|(l, _)| (p, l))
                .collect();
            let mut body = div().v_flex().py(px(4.));
            for (l, line) in part.lines.iter().enumerate() {
                if line.kind == '\\' {
                    continue;
                }
                let change = matches!(line.kind, '+' | '-');
                let included = change && self.line_in(&path, p, l);
                let color = match line.kind {
                    '+' => c_add(),
                    '-' => c_del(),
                    _ => c_muted(),
                };
                let line_path = path.clone();
                body = body.child(
                    div()
                        .id(SharedString::from(format!("folder-line-{p}-{l}")))
                        .h(px(22.))
                        .pr_3()
                        .flex()
                        .items_center()
                        .font_family("JetBrains Mono")
                        .text_size(px(12.))
                        .when(change && included, |el| el.bg(rgba((color << 8) | 0x1c)))
                        .when(change && !included, |el| el.opacity(0.45))
                        .child(div().w(px(34.)).flex_none().flex().justify_center().when(
                            change,
                            |el| {
                                el.child(
                                    div()
                                        .id(SharedString::from(format!("folder-tick-{p}-{l}")))
                                        .cursor_pointer()
                                        .child(tick_box(if included {
                                            Ticked::All
                                        } else {
                                            Ticked::None
                                        }))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.flip_lines(&line_path, vec![(p, l)]);
                                            cx.notify();
                                        })),
                                )
                            },
                        ))
                        .child(
                            div()
                                .w(px(40.))
                                .flex_none()
                                .text_color(rgb(c_muted()))
                                .child(match (line.old, line.new) {
                                    (_, n) if n > 0 => n.to_string(),
                                    (o, _) if o > 0 => o.to_string(),
                                    _ => String::new(),
                                }),
                        )
                        .child(div().w(px(16.)).flex_none().text_color(rgb(color)).child(
                            if change {
                                line.kind.to_string()
                            } else {
                                String::new()
                            },
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .when(change, |el| el.text_color(rgb(c_text())))
                                .when(!change, |el| el.text_color(rgb(c_secondary())))
                                .child(line.text.replace('\t', "    ")),
                        ),
                );
            }
            out = out.child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .overflow_hidden()
                    .child(
                        div()
                            .h(px(38.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .bg(tint(c_subtle()))
                            .child(
                                div()
                                    .id(("folder-part-tick", p))
                                    .cursor_pointer()
                                    .child(tick_box(state))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.flip_lines(&flip_path, lines_of_part.clone());
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Part {}", p + 1)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_blue()))
                                    .child(part.title.clone()),
                            )
                            .child(div().text_size(px(12.)).text_color(rgb(c_muted())).child(
                                match state {
                                    Ticked::All => "Goes in",
                                    Ticked::Some => "Some lines go in",
                                    Ticked::None => "Left out, stays in your folder",
                                },
                            ))
                            .child(
                                div()
                                    .id(("folder-part-throw", p))
                                    .h(px(26.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .hover(|el| el.bg(tint(c_subtle())))
                                    .child(icon(IconName::Trash, 12., c_secondary()))
                                    .child("Throw away")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.throw_away_part(&throw_path, p);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(body),
            );
        }
        out.into_any_element()
    }

    /// Set the commit box, which needs the window.
    pub(crate) fn apply_folder_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.git_folder_msg_set.take() {
            self.git_folder_msg
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }
}

/// Lines left out of the next commit, per file.
pub(crate) type LeftOut = HashMap<String, HashSet<(usize, usize)>>;
