//! Changes: the project folder's own uncommitted work, shown file by file and part by
//! part, so a commit can take whole files, single parts or single lines and leave the
//! rest in the folder. Parts can also be thrown away, and every step can be undone.

use super::*;
use branches::git_term;
use git::{FileParts, Undo};
use seatbelt::Snag;
use std::collections::HashSet;

/// Marks a whole-only file (binary, deleted) as left out of the next commit.
const WHOLE: (usize, usize) = (usize::MAX, usize::MAX);
/// Width of the folder page's tree of changed files.
const TREE_WIDTH: f32 = 340.;

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

/// Something the seatbelt found in what's ticked: the file, the line (or `WHOLE` for the
/// file itself), its number in the new file, and what it is.
type Found = (String, (usize, usize), usize, Snag);

/// Where a file's path splits into its name and its folder.
fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (name.to_string(), dir.to_string()),
        None => (path.to_string(), String::new()),
    }
}

/// A run of the folder's files shown together under one header.
pub(crate) struct Section {
    /// What folds the section, kept across reloads.
    pub key: String,
    pub title: String,
    pub icon: IconName,
    pub color: u32,
    /// Indexes into the folder's files.
    pub files: Vec<usize>,
}

/// The folder's files in sections by their top folder, in the order they first appear,
/// with files at the top level in a section of their own.
pub(crate) fn sections(folder: &Folder) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for (i, file) in folder.files.iter().enumerate() {
        let top = match file.change.path.split_once('/') {
            Some((top, _)) => top.to_string(),
            None => String::new(),
        };
        match out.iter_mut().find(|s| s.key == top) {
            Some(section) => section.files.push(i),
            None => out.push(Section {
                title: if top.is_empty() {
                    "Top level".into()
                } else {
                    format!("{top}/")
                },
                icon: if top.is_empty() {
                    IconName::File
                } else {
                    IconName::Folder
                },
                color: c_blue(),
                key: top,
                files: vec![i],
            }),
        }
    }
    out
}

/// A file's kind from its name: a short tag and its colour.
fn file_kind(name: &str) -> (&'static str, u32) {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => ("RS", 0xdc7f4a),
        "ts" | "tsx" | "mts" => ("TS", 0x3b82f6),
        "js" | "jsx" | "mjs" | "cjs" => ("JS", 0xd4a017),
        "py" => ("PY", 0x3b82f6),
        "go" => ("GO", 0x06b6d4),
        "swift" => ("SW", 0xf97316),
        "css" | "scss" | "less" => ("CSS", 0x0ea5e9),
        "html" | "htm" => ("HTML", 0xf97316),
        "md" | "mdx" | "txt" => ("MD", 0x8b93b8),
        "toml" | "yaml" | "yml" | "json" | "lock" => ("CFG", 0xa78bfa),
        "sh" | "bash" | "zsh" | "fish" => ("SH", 0x22c55e),
        "svg" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" => ("IMG", 0xec4899),
        _ => ("FILE", 0x8b93b8),
    }
}

/// A small coloured tag naming a file's kind.
fn kind_badge(name: &str) -> Div {
    let (tag, color) = file_kind(name);
    div()
        .w(px(30.))
        .h(px(18.))
        .flex_none()
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba((color << 8) | 0x26))
        .text_color(rgb(color))
        .text_size(px(8.5))
        .font_weight(FontWeight::BOLD)
        .child(tag)
}

/// What happened to a file, as a small outlined chip.
fn status_chip(status: &str) -> Div {
    let (word, color) = match status {
        "A" | "?" | "??" => ("new", c_add()),
        "D" => ("deleted", c_del()),
        s if s.starts_with('R') => ("renamed", c_blue()),
        _ => ("edited", c_amber()),
    };
    div()
        .flex_none()
        .h(px(18.))
        .px(px(7.))
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(rgba((color << 8) | 0x80))
        .text_size(px(10.5))
        .text_color(rgb(color))
        .child(word)
}

/// What happened to a file, as a dot in its colour.
fn status_dot(status: &str) -> Div {
    let color = match status {
        "A" | "?" | "??" => c_add(),
        "D" => c_del(),
        s if s.starts_with('R') => c_blue(),
        _ => c_amber(),
    };
    div().size(px(6.)).flex_none().rounded_full().bg(rgb(color))
}

/// Five blocks showing how big a change is and how it splits between added and removed
/// lines, the way GitHub shows a diff's size.
fn diff_bars(added: usize, removed: usize, binary: bool) -> Div {
    const BLOCKS: usize = 5;
    let total = added + removed;
    let filled = if binary || total == 0 {
        0
    } else {
        (BLOCKS * total.min(200)).div_ceil(200).clamp(1, BLOCKS)
    };
    let green = if total == 0 {
        0
    } else {
        ((filled * added) as f32 / total as f32).round() as usize
    };
    div()
        .flex_none()
        .flex()
        .gap(px(2.))
        .children((0..BLOCKS).map(|b| {
            div().size(px(7.)).rounded(px(2.)).bg(if b < green {
                rgb(c_add())
            } else if b < filled {
                rgb(c_del())
            } else {
                tint(c_border())
            })
        }))
}

/// A thin bar split between added and removed lines.
fn ratio_bar(added: usize, removed: usize, height: f32) -> Div {
    let total = (added + removed).max(1) as f32;
    div()
        .w_full()
        .h(px(height))
        .flex()
        .rounded_full()
        .overflow_hidden()
        .bg(tint(c_border()))
        .when(added > 0, |el| {
            el.child(
                div()
                    .h_full()
                    .w(relative(added as f32 / total))
                    .bg(rgb(c_add())),
            )
        })
        .when(removed > 0, |el| {
            el.child(
                div()
                    .h_full()
                    .w(relative(removed as f32 / total))
                    .bg(rgb(c_del())),
            )
        })
}

/// The project folder's mark: its icon on a tile of the accent.
fn folder_mark(size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .rounded(px(size * 0.3))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba((c_blue() << 8) | 0x26))
        .border_1()
        .border_color(rgba((c_blue() << 8) | 0x40))
        .child(icon(IconName::Folder, size * 0.47, c_blue()))
}

impl RunoApp {
    /// Read the project folder's changes again.
    pub(crate) fn load_folder(&mut self) {
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

    /// What the seatbelt finds in what the next commit would take.
    fn folder_snags(&self, folder: &Folder) -> Vec<Found> {
        let mut out = Vec::new();
        for file in &folder.files {
            let path = &file.change.path;
            if self.file_ticked(file) == Ticked::None || file.change.status == "D" {
                continue;
            }
            if seatbelt::secret_file(path) {
                out.push((path.clone(), WHOLE, 0, Snag::SecretFile));
            }
            let size = std::fs::metadata(std::path::Path::new(&folder.dir).join(path))
                .map_or(0, |m| m.len());
            if size > seatbelt::HUGE {
                out.push((path.clone(), WHOLE, 0, Snag::Huge(size)));
            }
            if file.whole_only() {
                continue;
            }
            for (p, part) in file.parts.iter().enumerate() {
                for (l, line) in part.lines.iter().enumerate() {
                    if line.kind != '+' || !self.line_in(path, p, l) {
                        continue;
                    }
                    if let Some(snag) = seatbelt::scan_line(path, &line.text) {
                        out.push((path.clone(), (p, l), line.new, snag));
                    }
                }
            }
        }
        out
    }

    /// The seatbelt's card over the commit button: each thing found, with a way to keep
    /// it out of the commit.
    fn seatbelt_card(&self, found: &[Found], cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().v_flex().gap(px(4.));
        for (i, (path, at, line_no, snag)) in found.iter().take(12).enumerate() {
            let whole = *at == WHOLE;
            // New files only: one git already tracks needs taking out of history too.
            let untracked = self
                .folder_file(path)
                .is_some_and(|f| matches!(f.change.status.as_str(), "A" | "?" | "??"));
            let (leave, ignore, key) = (path.clone(), path.clone(), *at);
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .child(icon(
                        if snag.serious() {
                            IconName::ShieldAlert
                        } else {
                            IconName::TriangleAlert
                        },
                        12.,
                        c_amber(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(div().truncate().child(snag.words()))
                            .child(
                                div()
                                    .truncate()
                                    .font_family("JetBrains Mono")
                                    .text_size(px(11.))
                                    .text_color(rgb(c_muted()))
                                    .child(match line_no {
                                        0 => path.clone(),
                                        n => format!("{path}:{n}"),
                                    }),
                            ),
                    )
                    .child(
                        self.button(
                            ("belt-leave", i),
                            if whole { "Leave the file out" } else { "Leave the line out" },
                            false,
                        )
                        .h(px(26.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if key == WHOLE {
                                this.flip_file(&leave);
                            } else {
                                this.git_left_out.entry(leave.clone()).or_default().insert(key);
                            }
                            cx.notify();
                        })),
                    )
                    .when(whole && untracked, |el| {
                        el.child(
                            self.button(("belt-ignore", i), "Ignore it", false)
                                .h(px(26.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let path = ignore.clone();
                                    this.git_step("Ignoring…", move |dir| {
                                        git::ignore_path(dir, &path)?;
                                        Ok(Some((
                                            format!("Added {path} to .gitignore. git won't offer it again."),
                                            None,
                                        )))
                                    });
                                    cx.notify();
                                })),
                        )
                    }),
            );
        }
        div()
            .mt_2()
            .p_3()
            .v_flex()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(rgba((c_amber() << 8) | 0x66))
            .bg(rgba((c_amber() << 8) | 0x14))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::ShieldCheck, 13., c_amber()))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(match found.len() {
                                1 => "Seatbelt: 1 thing to check before committing".to_string(),
                                n => format!("Seatbelt: {n} things to check before committing"),
                            }),
                    ),
            )
            .child(rows)
            .into_any_element()
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
        let unpushed = self.git_repo.as_ref().map_or(0, |r| r.state.ahead);
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
                        .pt(px(8.))
                        .pb(px(9.))
                        .v_flex()
                        .gap(px(7.))
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
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(folder_mark(28.))
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
                                        .child(
                                            div()
                                                .text_color(rgb(c_add()))
                                                .child(format!("+{added}")),
                                        )
                                        .child(
                                            div()
                                                .text_color(rgb(c_del()))
                                                .child(format!("−{removed}")),
                                        ),
                                ),
                        )
                        // How the folder's work splits between added and removed lines, and
                        // what is still waiting to go up.
                        .child(
                            div()
                                .pl(px(36.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().child(ratio_bar(added, removed, 4.)))
                                .when(unpushed > 0, |el| {
                                    el.child(
                                        div()
                                            .flex_none()
                                            .text_size(px(10.5))
                                            .text_color(rgb(c_muted()))
                                            .child(format!("{unpushed} to push")),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Tick or untick every file of a section together: all in when any were out.
    fn flip_section(&mut self, paths: &[String]) {
        let files: Vec<FileParts> = paths.iter().filter_map(|p| self.folder_file(p)).collect();
        let all_in = files.iter().all(|f| self.file_ticked(f) == Ticked::All);
        for file in files {
            let lines = if file.whole_only() {
                vec![WHOLE]
            } else {
                Self::changed_lines(&file)
            };
            let out = self
                .git_left_out
                .entry(file.change.path.clone())
                .or_default();
            for key in lines {
                if all_in {
                    out.insert(key);
                } else {
                    out.remove(&key);
                }
            }
        }
    }

    /// One changed file in the tree: its tick box, kind and name, what happened to it and
    /// how much.
    fn folder_file_row(&self, i: usize, file: &FileParts, cx: &mut Context<Self>) -> Stateful<Div> {
        let path = file.change.path.clone();
        let (fname, fdir) = split_path(&path);
        let on = self.git_folder_file.as_ref() == Some(&path);
        let state = self.file_ticked(file);
        let (flip, pick) = (path.clone(), path.clone());
        let accent = c_blue();
        // Files in a deeper folder than their section's say where they sit.
        let sub = fdir.split_once('/').map(|(_, rest)| rest.to_string());
        div()
            .id(("folder-file", i))
            .relative()
            .h(px(30.))
            .pl(px(30.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(12.5))
            .when(on, |el| {
                el.bg(rgba((accent << 8) | 0x24)).child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.))
                        .bg(rgb(accent)),
                )
            })
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.git_folder_file = Some(pick.clone());
                cx.notify();
            }))
            .child(
                div()
                    .id(("folder-file-tick", i))
                    .p(px(2.))
                    .child(tick_box(state))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.flip_file(&flip);
                        cx.notify();
                    })),
            )
            .child(kind_badge(&fname))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_none()
                            .max_w_full()
                            .truncate()
                            .when(on, |el| el.font_weight(FontWeight::MEDIUM))
                            .when(state == Ticked::None, |el| el.text_color(rgb(c_muted())))
                            .when(file.change.status == "D", |el| el.line_through())
                            .child(fname),
                    )
                    .when_some(sub, |el, sub| {
                        el.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .child(sub),
                        )
                    }),
            )
            .child(status_dot(&file.change.status))
            .when(!file.change.binary, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .gap(px(5.))
                        .text_size(px(11.))
                        .when(file.change.added > 0, |el| {
                            el.child(
                                div()
                                    .text_color(rgb(c_add()))
                                    .child(format!("+{}", file.change.added)),
                            )
                        })
                        .when(file.change.removed > 0, |el| {
                            el.child(
                                div()
                                    .text_color(rgb(c_del()))
                                    .child(format!("−{}", file.change.removed)),
                            )
                        }),
                )
            })
    }

    /// A section of the tree: a header that ticks or folds its files, then the files.
    fn folder_section(
        &self,
        s: usize,
        section: &Section,
        folder: &Folder,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files: Vec<&FileParts> = section.files.iter().map(|i| &folder.files[*i]).collect();
        let (added, removed) = files.iter().fold((0, 0), |a, f| {
            (a.0 + f.change.added, a.1 + f.change.removed)
        });
        let ticks: Vec<Ticked> = files.iter().map(|f| self.file_ticked(f)).collect();
        let state = if ticks.iter().all(|t| *t == Ticked::All) {
            Ticked::All
        } else if ticks.iter().all(|t| *t == Ticked::None) {
            Ticked::None
        } else {
            Ticked::Some
        };
        let shut = self.git_folder_shut.contains(&section.key);
        let paths: Vec<String> = files.iter().map(|f| f.change.path.clone()).collect();
        let key = section.key.clone();
        let head = div()
            .id(("folder-section", s))
            .h(px(30.))
            .pl(px(8.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(12.5))
            .hover(|el| el.bg(tint(c_subtle())))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.git_folder_shut.remove(&key) {
                    this.git_folder_shut.insert(key.clone());
                }
                cx.notify();
            }))
            .child(icon(
                if shut {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                },
                12.,
                c_muted(),
            ))
            .child(
                div()
                    .id(("folder-section-tick", s))
                    .p(px(2.))
                    .child(tick_box(state))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.flip_section(&paths);
                        cx.notify();
                    })),
            )
            .child(icon(section.icon, 13., section.color))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_secondary()))
                    .child(section.title.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(files.len().to_string()),
            )
            .when(shut, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .gap(px(5.))
                        .text_size(px(11.))
                        .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                        .child(div().text_color(rgb(c_del())).child(format!("−{removed}"))),
                )
            });
        div()
            .v_flex()
            .child(head)
            .when(!shut, |el| {
                el.children(
                    section
                        .files
                        .iter()
                        .zip(files.iter())
                        .map(|(i, file)| self.folder_file_row(*i, file, cx)),
                )
            })
            .into_any_element()
    }

    /// The picked file's diff: its path and size above its parts.
    fn folder_diff(&self, file: &FileParts, cx: &mut Context<Self>) -> AnyElement {
        let path = file.change.path.clone();
        let (fname, fdir) = split_path(&path);
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(divider())
                    .child(kind_badge(&fname))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .font_family("JetBrains Mono")
                            .text_size(px(12.5))
                            .when(!fdir.is_empty(), |el| {
                                el.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(rgb(c_muted()))
                                        .child(format!("{fdir}/")),
                                )
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(fname),
                            ),
                    )
                    .child(status_chip(&file.change.status))
                    .when(!file.change.binary, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .flex()
                                .gap(px(6.))
                                .text_size(px(12.))
                                .child(
                                    div()
                                        .text_color(rgb(c_add()))
                                        .child(format!("+{}", file.change.added)),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(c_del()))
                                        .child(format!("−{}", file.change.removed)),
                                ),
                        )
                    })
                    .child(diff_bars(
                        file.change.added,
                        file.change.removed,
                        file.change.binary,
                    )),
            )
            .child(
                div()
                    .id("folder-diff")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .pb(px(16.))
                    .child(self.file_parts_view(file, cx)),
            )
            .into_any_element()
    }

    /// The folder's page: the changed files as a tree beside the picked file's diff, with
    /// the commit bar under the diff.
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
        let found = self.folder_snags(folder);
        let belt = !found.is_empty();
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
        let commit_label = if belt && picked > 0 {
            "Commit anyway".to_string()
        } else {
            commit_label
        };
        let (added, removed) = folder.files.iter().fold((0, 0), |a, f| {
            (a.0 + f.change.added, a.1 + f.change.removed)
        });
        let n = folder.files.len();
        let ticked_files = folder
            .files
            .iter()
            .filter(|f| self.file_ticked(f) != Ticked::None)
            .count();
        let amendable = folder.last.clone().filter(|(_, local)| *local);
        let empty = folder.files.is_empty();
        let header = div()
            .h(px(58.))
            .flex_none()
            .px(px(20.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(divider())
            .child(folder_mark(30.))
            .child(
                div()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(if empty {
                                "Nothing changed since the last commit".to_string()
                            } else {
                                format!("{n} changed  ·  {ticked_files} ticked for the next commit")
                            }),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .h(px(22.))
                    .rounded_full()
                    .bg(tint(c_subtle()))
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .child(icon(IconName::GitBranch, 11., c_muted()))
                    .child(folder.branch.clone()),
            )
            .child(div().flex_1())
            .when(!empty, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.5))
                        .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                        .child(div().text_color(rgb(c_del())).child(format!("−{removed}")))
                        .child(div().w(px(90.)).child(ratio_bar(added, removed, 4.))),
                )
            });
        if empty {
            return div()
                .flex_1()
                .min_w_0()
                .h_full()
                .v_flex()
                .child(header)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.))
                        .text_color(rgb(c_muted()))
                        .child("Nothing to commit. The folder matches its last commit."),
                )
                .into_any_element();
        }
        let sections: Vec<AnyElement> = sections(folder)
            .iter()
            .enumerate()
            .map(|(s, section)| self.folder_section(s, section, folder, cx))
            .collect();
        let tree = div()
            .w(px(TREE_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .border_r_1()
            .border_color(divider())
            .child(
                div()
                    .h(px(34.))
                    .flex_none()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_muted()))
                    .child("CHANGES")
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .child(format!("{ticked_files} of {n} ticked")),
                    ),
            )
            .child(
                div()
                    .id("folder-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(8.))
                    .v_flex()
                    .gap(px(4.))
                    .children(sections),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(divider())
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("Tick files, parts or single lines. What you leave unticked stays in your folder."),
            );
        let shown = self
            .git_folder_file
            .as_ref()
            .and_then(|p| folder.files.iter().find(|f| f.change.path == *p));
        let diff = match shown {
            Some(file) => self.folder_diff(file, cx),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(rgb(c_muted()))
                .child("Pick a file to see what changed in it.")
                .into_any_element(),
        };
        let go = if belt { c_amber() } else { c_blue() };
        let on_go = if is_dark() { c_canvas() } else { 0xffffff };
        let commit_bar = div()
            .flex_none()
            .px(px(14.))
            .py(px(10.))
            .flex()
            .items_center()
            .gap_2()
            .border_t_1()
            .border_color(divider())
            .bg(tint(c_subtle()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.git_folder_msg).bordered(true).h(px(34.))),
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
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_active())))
                        .child(icon(IconName::Plus, 12., c_secondary()))
                        .child(
                            div()
                                .max_w(px(180.))
                                .truncate()
                                .child(format!("Add to “{subject}”")),
                        )
                        .child(git_term("amend").self_center())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.commit_folder(true, cx);
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .id("folder-commit")
                    .flex_none()
                    .h(px(34.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(rgb(go))
                    .text_color(rgb(on_go))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(|el| el.opacity(0.9))
                    .when(picked == 0, |el| el.opacity(0.5))
                    .child(icon(IconName::GitCommitHorizontal, 14., on_go))
                    .child(commit_label)
                    .child(
                        div()
                            .px(px(5.))
                            .rounded(px(4.))
                            .bg(rgba((on_go << 8) | 0x24))
                            .font_family("JetBrains Mono")
                            .text_size(px(10.5))
                            .child("commit"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.commit_folder(false, cx);
                        cx.notify();
                    })),
            );
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(header)
            .child(
                div().flex_1().min_h_0().flex().child(tree).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .v_flex()
                        .child(diff)
                        .when(belt, |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .px(px(14.))
                                    .pb(px(10.))
                                    .child(self.seatbelt_card(&found, cx)),
                            )
                        })
                        .child(commit_bar),
                ),
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
