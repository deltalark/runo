//! The file viewer and editor: code with syntax highlighting and Ctrl-S save,
//! rendered markdown, images, CSV tables, and PDF / Mermaid fallbacks.

use super::*;
use gpui_kit::component::input::{Editor, EditorState};
use std::path::{Path, PathBuf};

/// Files above this size open read-only, so a log or bundle cannot stall the editor.
const MAX_EDIT_BYTES: u64 = 2_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FileKind {
    Code,
    Markdown,
    Image,
    Csv(char),
    Pdf,
    Mermaid,
    Binary,
}

pub(crate) struct OpenFile {
    pub path: String,
    pub abs: PathBuf,
    pub kind: FileKind,
    /// Text as last read or saved, to tell whether the editor has changes.
    pub saved: String,
    pub size: u64,
    pub read_only: bool,
}

pub(crate) fn file_kind(path: &str, head: &[u8]) -> FileKind {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" => FileKind::Image,
        "md" | "markdown" | "mdx" => FileKind::Markdown,
        "csv" => FileKind::Csv(','),
        "tsv" => FileKind::Csv('\t'),
        "pdf" => FileKind::Pdf,
        "mmd" | "mermaid" => FileKind::Mermaid,
        _ if head.contains(&0) => FileKind::Binary,
        _ => FileKind::Code,
    }
}

/// The highlighter's name for a file, or "text" for plain text.
pub(crate) fn language_for(path: &str) -> &'static str {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name == "Makefile" || name == "makefile" {
        return "make";
    }
    if name == "Dockerfile"
        || name.ends_with(".sh")
        || name.ends_with(".bash")
        || name.ends_with(".zsh")
    {
        return "bash";
    }
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" | "json" | "jsonc" => "javascript",
        "py" | "pyi" => "python",
        "go" => "go",
        "toml" | "lock" => "toml",
        "yml" | "yaml" => "yaml",
        "md" | "markdown" | "mdx" => "markdown",
        "css" | "scss" => "css",
        "html" | "htm" => "html",
        _ => "text",
    }
}

/// Split CSV text into rows, honouring quoted fields with separators and quotes inside.
pub(crate) fn parse_csv(text: &str, sep: char, max_rows: usize) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                if rows.len() >= max_rows {
                    return rows;
                }
            }
            c if c == sep => row.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / 1024.),
        b => format!("{b} bytes"),
    }
}

impl RunoApp {
    pub(crate) fn new_file_editor(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorState> {
        cx.new(|cx| {
            EditorState::new(window, cx)
                .line_number(true)
                .soft_wrap(false)
        })
    }

    /// Open a file of the selected session in the viewer (the next frame applies it).
    pub(crate) fn open_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let abs = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            let Some(dir) = self.current().map(|s| s.dir.clone()) else {
                return;
            };
            Path::new(&dir).join(path)
        };
        let size = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
        let bytes = if self.demo && !Path::new(path).is_absolute() {
            b"(demo data)".to_vec()
        } else {
            std::fs::read(&abs).unwrap_or_default()
        };
        let kind = file_kind(path, &bytes[..bytes.len().min(8000)]);
        let text = match kind {
            FileKind::Image | FileKind::Pdf | FileKind::Binary => String::new(),
            _ => String::from_utf8_lossy(&bytes).into_owned(),
        };
        let read_only = size > MAX_EDIT_BYTES || (self.demo && !Path::new(path).is_absolute());
        let language = match kind {
            FileKind::Code => language_for(path),
            FileKind::Markdown => "markdown",
            _ => "text",
        };
        self.file_editor.update(cx, |e, cx| {
            e.set_highlighter(language, cx);
            e.set_value(text.clone(), window, cx);
        });
        // New or empty files open straight into the editor.
        self.file_preview =
            !text.trim().is_empty() || kind == FileKind::Image || kind == FileKind::Pdf;
        self.file_preview &= matches!(
            kind,
            FileKind::Markdown
                | FileKind::Csv(_)
                | FileKind::Image
                | FileKind::Pdf
                | FileKind::Mermaid
        );
        self.file_dirty = false;
        self.open_file = Some(OpenFile {
            path: path.to_string(),
            abs,
            kind,
            saved: text,
            size,
            read_only,
        });
        cx.notify();
    }

    pub(crate) fn save_file(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.open_file.as_mut() else {
            return;
        };
        if file.read_only || !self.file_dirty {
            return;
        }
        let text = self.file_editor.read(cx).value().to_string();
        if let Some(parent) = file.abs.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&file.abs, &text) {
            Ok(()) => {
                file.saved = text;
                file.size = file.saved.len() as u64;
                self.file_dirty = false;
                let path = file.path.clone();
                if let Some(id) = self.selected {
                    self.state.log(id, format!("Edited {path}"));
                }
                self.info = Some((format!("Saved {path}."), std::time::Instant::now()));
                self.refresh();
            }
            Err(e) => {
                self.info = Some((format!("Could not save: {e}"), std::time::Instant::now()));
            }
        }
        cx.notify();
    }

    /// Open a config file (absolute path) in the main-area editor, from Settings.
    pub(crate) fn edit_path(&mut self, path: &Path) {
        self.pending_file = Some(path.display().to_string());
        self.editor_back = self.view;
        self.view = View::Editor;
    }

    pub(crate) fn file_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(file) = &self.open_file {
            let value = self.file_editor.read(cx).value();
            self.file_dirty = value.as_ref() != file.saved;
        }
    }

    pub(crate) fn file_icon(kind: FileKind) -> IconName {
        match kind {
            FileKind::Image => IconName::Image,
            FileKind::Markdown | FileKind::Pdf => IconName::FileText,
            FileKind::Csv(_) => IconName::Sheet,
            FileKind::Mermaid => IconName::Workflow,
            FileKind::Binary => IconName::File,
            FileKind::Code => IconName::FileCode,
        }
    }

    /// The open file: a header with its actions, then the viewer or editor.
    pub(crate) fn file_view(&self, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(file) = &self.open_file else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(empty_hint(
                    "file-empty",
                    IconName::FileCode,
                    "No file open",
                    "Pick a file above to view it, or edit and save with Ctrl+S.",
                ))
                .into_any_element();
        };
        let kind = file.kind;
        let can_preview = matches!(
            kind,
            FileKind::Markdown | FileKind::Csv(_) | FileKind::Mermaid
        );
        let dirty = self.file_dirty;
        let toggle = |id: &'static str,
                      label: &'static str,
                      on: bool,
                      preview: bool,
                      cx: &mut Context<Self>| {
            div()
                .id(id)
                .h(px(22.))
                .px_2()
                .flex()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.))
                .when(on, |el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                .when(!on, |el| {
                    el.text_color(rgb(c_muted()))
                        .hover(|el| el.text_color(rgb(c_text())))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.file_preview = preview;
                    cx.notify();
                }))
                .child(label)
        };
        let header = div()
            .h(px(36.))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(c_border()))
            .child(icon(Self::file_icon(kind), 14., c_muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(self.mono_font.clone())
                    .text_size(px(12.5))
                    .child(match std::env::var("HOME") {
                        Ok(home) if !home.is_empty() && file.path.starts_with(&home) => {
                            format!("~{}", &file.path[home.len()..])
                        }
                        _ => file.path.clone(),
                    }),
            )
            .when(!file.abs.exists(), |el| {
                el.child(div().text_xs().text_color(rgb(c_muted())).child("New file"))
            })
            .when(dirty, |el| {
                el.child(
                    div()
                        .id("file-dirty")
                        .size(px(7.))
                        .rounded_full()
                        .bg(rgb(c_amber()))
                        .with_animation(
                            "file-dirty-pulse",
                            Animation::new(Duration::from_millis(900))
                                .repeat()
                                .with_easing(pulsating_between(0.45, 1.)),
                            |el, t| el.opacity(t),
                        ),
                )
            })
            .when(file.read_only, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(rgb(c_muted()))
                        .child("Read only"),
                )
            })
            .when(can_preview, |el| {
                el.child(toggle(
                    "file-preview",
                    "Preview",
                    self.file_preview,
                    true,
                    cx,
                ))
                .child(toggle("file-edit", "Edit", !self.file_preview, false, cx))
            })
            .when(dirty, |el| {
                el.child(
                    self.button("file-save", "Save", true)
                        .child(icon(IconName::Save, 12., c_canvas()))
                        .h(px(24.))
                        .px_2()
                        .text_xs()
                        .on_click(cx.listener(|this, _, _, cx| this.save_file(cx))),
                )
            })
            .child(
                self.ghost_button_sized(
                    "file-wide",
                    if wide {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    },
                    if wide {
                        "Back to the session"
                    } else {
                        "Open in the main area"
                    },
                    24.,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if wide {
                        this.view = this.editor_back;
                    } else {
                        this.editor_back = View::Session;
                        this.view = View::Editor;
                    }
                    cx.notify();
                })),
            )
            .child(
                self.ghost_button_sized("file-close", IconName::X, "Close file", 24.)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_file = None;
                        this.file_dirty = false;
                        if wide {
                            this.view = this.editor_back;
                        }
                        cx.notify();
                    })),
            );
        let editor = || {
            div()
                .flex_1()
                .min_h_0()
                .font_family(self.mono_font.clone())
                .text_size(px(12.5))
                .child(
                    Editor::new(&self.file_editor)
                        .appearance(false)
                        .readonly(file.read_only)
                        .h_full(),
                )
                .into_any_element()
        };
        let body = match kind {
            FileKind::Image => div()
                .id("file-image")
                .flex_1()
                .min_h_0()
                .overflow_scroll()
                .p_4()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(
                    img(file.abs.clone())
                        .max_w_full()
                        .max_h(px(if wide { 720. } else { 420. }))
                        .object_fit(ObjectFit::Contain)
                        .rounded_md(),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(c_muted()))
                        .child(human_size(file.size)),
                )
                .into_any_element(),
            FileKind::Pdf | FileKind::Binary => {
                let url = format!("file://{}", file.abs.display());
                div()
                    .flex_1()
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .p_6()
                    .child(icon(Self::file_icon(kind), 28., c_muted()))
                    .child(div().text_sm().text_color(rgb(c_secondary())).child(
                        if kind == FileKind::Pdf {
                            format!("PDF document · {}", human_size(file.size))
                        } else {
                            format!("Binary file · {}", human_size(file.size))
                        },
                    ))
                    .child(
                        self.button("file-open-ext", "Open in default app", false)
                            .h(px(28.))
                            .child(icon(IconName::ExternalLink, 12., c_secondary()))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                    )
                    .into_any_element()
            }
            FileKind::Markdown if self.file_preview => div()
                .id("file-md")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_5()
                .py_4()
                .text_size(px(13.5))
                .child(
                    TextView::markdown(
                        SharedString::from(format!("md-{}", file.path)),
                        self.file_editor.read(cx).value().to_string(),
                    )
                    .selectable(true),
                )
                .into_any_element(),
            FileKind::Csv(sep) if self.file_preview => {
                let text = self.file_editor.read(cx).value().to_string();
                let rows = parse_csv(&text, sep, 500);
                let cols = rows.iter().map(Vec::len).max().unwrap_or(0).min(40);
                let mut table = div().v_flex().min_w_full();
                for (r, row) in rows.iter().enumerate() {
                    let mut line = div()
                        .flex()
                        .border_b_1()
                        .border_color(rgb(c_border()))
                        .when(r == 0, |el| {
                            el.bg(tint(c_subtle())).font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(r > 0 && r % 2 == 0, |el| el.bg(rgb(c_canvas())));
                    for c in 0..cols {
                        line = line.child(
                            div()
                                .w(px(140.))
                                .flex_none()
                                .px_2()
                                .py(px(4.))
                                .truncate()
                                .border_r_1()
                                .border_color(rgb(c_border()))
                                .child(row.get(c).cloned().unwrap_or_default()),
                        );
                    }
                    table = table.child(line);
                }
                div()
                    .id("file-csv")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .text_size(px(12.))
                    .child(table)
                    .child(
                        div()
                            .p_2()
                            .text_xs()
                            .text_color(rgb(c_muted()))
                            .child(format!(
                                "{} rows · {cols} columns{}",
                                rows.len().saturating_sub(1),
                                if rows.len() >= 500 {
                                    " (first 500 shown)"
                                } else {
                                    ""
                                }
                            )),
                    )
                    .into_any_element()
            }
            FileKind::Mermaid if self.file_preview => div()
                .id("file-mmd")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .v_flex()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .p_3()
                        .rounded_lg()
                        .bg(tint(c_subtle()))
                        .text_sm()
                        .text_color(rgb(c_secondary()))
                        .child(icon(IconName::Info, 13., c_secondary()))
                        .child(
                            "Mermaid diagrams are shown as source. Switch to Edit to change them.",
                        ),
                )
                .child(
                    TextView::markdown(
                        SharedString::from(format!("mmd-{}", file.path)),
                        format!(
                            "```mermaid\n{}\n```",
                            self.file_editor.read(cx).value().replace("```", "'''")
                        ),
                    )
                    .selectable(true),
                )
                .into_any_element(),
            _ => editor(),
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .v_flex()
            .child(header)
            .child(fade(
                div().flex_1().min_h_0().v_flex().child(body),
                SharedString::from(format!("file-{}-{}", file.path, self.file_preview)),
                180,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn kinds_languages_and_csv() {
        assert_eq!(file_kind("a/b.png", b""), FileKind::Image);
        assert_eq!(file_kind("README.md", b"# hi"), FileKind::Markdown);
        assert_eq!(file_kind("x.tsv", b""), FileKind::Csv('\t'));
        assert_eq!(file_kind("doc.pdf", b"%PDF"), FileKind::Pdf);
        assert_eq!(file_kind("flow.mmd", b"graph TD"), FileKind::Mermaid);
        assert_eq!(file_kind("blob.bin", b"ab\0cd"), FileKind::Binary);
        assert_eq!(file_kind("main.rs", b"fn main() {}"), FileKind::Code);
        assert_eq!(language_for("src/main.rs"), "rust");
        assert_eq!(language_for("app.tsx"), "tsx");
        assert_eq!(language_for("Makefile"), "make");
        assert_eq!(language_for("notes.txt"), "text");
        let rows = parse_csv("name,note\nann,\"a, b\"\nbo,\"say \"\"hi\"\"\"\n", ',', 10);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], ["ann", "a, b"]);
        assert_eq!(rows[2], ["bo", "say \"hi\""]);
        assert_eq!(parse_csv("a,b\nc,d\ne,f\n", ',', 2).len(), 2);
        assert_eq!(human_size(2048), "2.0 KB");
    }
}
