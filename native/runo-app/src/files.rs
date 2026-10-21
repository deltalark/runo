//! The Files tab: the session folder as a tree, a filter on file names, and search
//! in files (`git grep`, or a plain scan outside git).

use super::*;

/// Rows drawn at most; a note says when there are more.
const MAX_ROWS: usize = 3000;
/// Search results kept.
const MAX_HITS: usize = 300;

/// One row of the tree: a folder or a file, at a depth.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TreeRow {
    Folder {
        path: String,
        name: String,
        depth: usize,
        open: bool,
    },
    File {
        path: String,
        name: String,
        depth: usize,
    },
}

/// The visible rows of a tree over sorted `files`: folders always show, their contents
/// only when the folder and every folder above it are open.
pub(crate) fn tree_rows(
    files: &[String],
    open: &std::collections::HashSet<String>,
) -> Vec<TreeRow> {
    let mut sorted: Vec<&String> = files.iter().collect();
    sorted.sort();
    let mut rows = Vec::new();
    let mut last: Vec<&str> = Vec::new();
    for path in sorted {
        let parts: Vec<&str> = path.split('/').collect();
        let (dirs, name) = parts.split_at(parts.len() - 1);
        let common = last
            .iter()
            .zip(dirs.iter())
            .take_while(|(a, b)| a == b)
            .count();
        // Folders below the first closed one are not shown.
        let shown = |depth: usize| (0..depth).all(|d| open.contains(&dirs[..=d].join("/")));
        for depth in common..dirs.len() {
            if shown(depth) {
                let folder = dirs[..=depth].join("/");
                rows.push(TreeRow::Folder {
                    open: open.contains(&folder),
                    path: folder,
                    name: dirs[depth].to_string(),
                    depth,
                });
            }
        }
        if shown(dirs.len()) {
            rows.push(TreeRow::File {
                path: path.clone(),
                name: name[0].to_string(),
                depth: dirs.len(),
            });
        }
        last = dirs.to_vec();
    }
    rows
}

/// Files whose path contains every word of `query`, any case.
pub(crate) fn filter_files<'a>(files: &'a [String], query: &str) -> Vec<&'a String> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    files
        .iter()
        .filter(|f| {
            let f = f.to_lowercase();
            words.iter().all(|w| f.contains(w.as_str()))
        })
        .collect()
}

/// Search text files outside a git repository, for folders `git grep` can't search.
pub(crate) fn search_plain(dir: &str, files: &[String], query: &str) -> (Vec<git::Hit>, bool) {
    let needle = query.to_lowercase();
    let mut hits = Vec::new();
    for path in files {
        let Ok(bytes) = std::fs::read(std::path::Path::new(dir).join(path)) else {
            continue;
        };
        if bytes.len() > 2_000_000 || bytes[..bytes.len().min(8000)].contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (i, line) in text.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                if hits.len() == MAX_HITS {
                    return (hits, true);
                }
                hits.push(git::Hit {
                    path: path.clone(),
                    line: i + 1,
                    text: line.trim().chars().take(200).collect(),
                });
            }
        }
    }
    (hits, false)
}

impl RunoApp {
    /// Run a search in files for what is typed in the Files filter.
    pub(crate) fn search_files(&mut self, cx: &mut Context<Self>) {
        let query = self.file_query.read(cx).value().trim().to_string();
        let Some(dir) = self.current().map(|s| s.dir.clone()) else {
            return;
        };
        if query.is_empty() {
            self.file_hits = None;
            return;
        }
        let (git_dir, files) = (self.dir_is_git, self.files.clone());
        self.file_searching = true;
        self.bg(
            move || {
                if git_dir {
                    git::grep(&dir, &query, MAX_HITS)
                } else {
                    search_plain(&dir, &files, &query)
                }
            },
            |this, found, _| {
                this.file_searching = false;
                this.file_hits = Some(found);
            },
        );
    }

    fn file_row(
        &self,
        i: usize,
        path: &str,
        label: String,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = path.to_string();
        div()
            .id(("file", i))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::File(path.to_string()), cx),
            )
            .h(px(26.))
            .pl(px(12. + depth as f32 * 14.))
            .pr_3()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_sm()
            .text_color(rgb(c_secondary()))
            .when(
                self.open_file.as_ref().is_some_and(|f| f.path == path),
                |el| el.bg(tint(c_active())).text_color(rgb(c_text())),
            )
            .hover(|el| el.bg(tint(c_subtle())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_preview(&p);
                cx.notify();
            }))
            .child(icon(
                Self::file_icon(fileview::file_kind(path, b"")),
                14.,
                c_muted(),
            ))
            .child(div().truncate().child(label))
            .into_any_element()
    }

    fn note_row(text: String) -> AnyElement {
        div()
            .px_3()
            .py_2()
            .text_size(px(12.))
            .text_color(rgb(c_muted()))
            .child(text)
            .into_any_element()
    }

    pub(crate) fn files_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.file_query.read(cx).value().trim().to_string();
        let mut list = div().v_flex().py_1();
        if let Some((hits, more)) = self.file_hits.as_ref().filter(|_| !query.is_empty()) {
            list = list.child(Self::note_row(match (hits.len(), more) {
                (0, _) => format!("No file contains “{query}”."),
                (n, true) => format!("First {n} matches of “{query}”"),
                (n, false) => format!("{n} match{} of “{query}”", if n == 1 { "" } else { "es" }),
            }));
            for (i, hit) in hits.iter().enumerate() {
                let p = hit.path.clone();
                list = list.child(
                    div()
                        .id(("file-hit", i))
                        .px_3()
                        .py(px(4.))
                        .v_flex()
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_preview(&p);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .child(format!("{}:{}", hit.path, hit.line)),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(self.mono_font.clone())
                                .text_size(px(11.5))
                                .text_color(rgb(c_text()))
                                .child(hit.text.clone()),
                        ),
                );
            }
        } else if !query.is_empty() {
            let found = filter_files(&self.files, &query);
            list = list.child(Self::note_row(if self.file_searching {
                "Searching in files…".to_string()
            } else {
                format!(
                    "{} file name{} match. Press Enter to search inside files.",
                    found.len(),
                    if found.len() == 1 { "" } else { "s" }
                )
            }));
            for (i, path) in found.iter().take(MAX_ROWS).enumerate() {
                list = list.child(self.file_row(i, path, path.to_string(), 0, cx));
            }
            if found.len() > MAX_ROWS {
                list = list.child(Self::note_row(format!(
                    "{} more; type more of the name to narrow it down.",
                    found.len() - MAX_ROWS
                )));
            }
        } else {
            let rows = tree_rows(&self.files, &self.files_open);
            for (i, row) in rows.iter().take(MAX_ROWS).enumerate() {
                list = list.child(match row {
                    TreeRow::File { path, name, depth } => {
                        self.file_row(i, path, name.clone(), *depth, cx)
                    }
                    TreeRow::Folder {
                        path,
                        name,
                        depth,
                        open,
                    } => {
                        let (folder, open) = (path.clone(), *open);
                        div()
                            .id(("folder", i))
                            .h(px(26.))
                            .pl(px(12. + *depth as f32 * 14.))
                            .pr_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.files_open.remove(&folder) {
                                    this.files_open.insert(folder.clone());
                                }
                                cx.notify();
                            }))
                            .child(icon(
                                if open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                },
                                12.,
                                c_muted(),
                            ))
                            .child(icon(
                                if open {
                                    IconName::FolderOpen
                                } else {
                                    IconName::Folder
                                },
                                14.,
                                c_muted(),
                            ))
                            .child(div().truncate().child(name.clone()))
                            .into_any_element()
                    }
                });
            }
            if rows.len() > MAX_ROWS {
                list = list.child(Self::note_row(format!(
                    "{} more rows not shown. Close some folders or filter by name.",
                    rows.len() - MAX_ROWS
                )));
            }
            if !self.dir_is_git && self.files.len() >= PLAIN_FILES_MAX {
                list = list.child(Self::note_row(format!(
                    "Showing the first {PLAIN_FILES_MAX} files of this folder. It isn't a git repository, so ignored folders can't be skipped."
                )));
            }
        }
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(icon(IconName::Search, 13., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.5))
                            .child(Input::new(&self.file_query).appearance(false).h(px(28.))),
                    ),
            )
            .child(
                div()
                    .id("file-list")
                    .max_h(px(260.))
                    .overflow_y_scrollbar()
                    .child(list),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .v_flex()
                    .border_t_1()
                    .border_color(divider())
                    .child(self.file_view(false, cx)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn trees_show_open_folders_only() {
        let files: Vec<String> = ["src/ui/view.rs", "README.md", "src/main.rs", "a-b.txt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut open = std::collections::HashSet::new();
        let names = |rows: Vec<TreeRow>| -> Vec<String> {
            rows.into_iter()
                .map(|r| match r {
                    TreeRow::Folder { path, depth, .. } => format!("{depth}d {path}"),
                    TreeRow::File { path, depth, .. } => format!("{depth}f {path}"),
                })
                .collect()
        };
        assert_eq!(
            names(tree_rows(&files, &open)),
            ["0f README.md", "0f a-b.txt", "0d src"]
        );
        open.insert("src".to_string());
        assert_eq!(
            names(tree_rows(&files, &open)),
            [
                "0f README.md",
                "0f a-b.txt",
                "0d src",
                "1f src/main.rs",
                "1d src/ui"
            ]
        );
        open.insert("src/ui".to_string());
        assert!(names(tree_rows(&files, &open)).contains(&"2f src/ui/view.rs".to_string()));
        // A closed parent hides an open child.
        open.remove("src");
        assert_eq!(tree_rows(&files, &open).len(), 3);
        assert_eq!(filter_files(&files, "SRC rs").len(), 2);
    }

    #[test]
    fn plain_search_finds_lines() {
        let dir = std::env::temp_dir().join(format!("runo-plain-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\nTwo words\n").unwrap();
        std::fs::write(dir.join("b.bin"), b"two\0").unwrap();
        let files = vec!["a.txt".to_string(), "b.bin".to_string()];
        let (hits, more) = search_plain(dir.to_str().unwrap(), &files, "two");
        assert!(!more);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].line, hits[0].text.as_str()), (2, "Two words"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
