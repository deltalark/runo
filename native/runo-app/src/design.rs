//! The Design page: design references (images, screenshots and their notes) collected per
//! project, the app's dev server beside them, and agent sessions that start with the picked
//! references attached.

use super::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use workspace::unix_now;

/// Where a project keeps its references: beside the session attachments, out of git.
pub(crate) const FOLDER: &str = ".runo/design";
const INDEX: &str = "references.json";

/// One design reference: an image in the project's design folder and what it is for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Reference {
    /// The file name inside the design folder.
    pub file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub added: u64,
}

pub(crate) fn folder(root: &Path) -> PathBuf {
    root.join(FOLDER)
}

/// The project's references, in the order they were added. Images put in the folder by
/// hand show up too, and entries whose file is gone are dropped.
pub(crate) fn load(root: &Path) -> Vec<Reference> {
    let dir = folder(root);
    let mut refs: Vec<Reference> = std::fs::read_to_string(dir.join(INDEX))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    refs.retain(|r| dir.join(&r.file).is_file());
    let mut extra: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && turns::is_image(p))
        .filter_map(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
        .filter(|name| !refs.iter().any(|r| &r.file == name))
        .collect();
    extra.sort();
    refs.extend(extra.into_iter().map(|file| Reference {
        file,
        ..Default::default()
    }));
    refs
}

pub(crate) fn save(root: &Path, refs: &[Reference]) -> std::io::Result<()> {
    let dir = folder(root);
    std::fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(refs).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(INDEX), text)
}

/// A file name for a new reference: a timestamp, then the original name made safe.
pub(crate) fn file_name(stamp: &str, name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{stamp}-{}", safe.trim_start_matches('.'))
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S%3f").to_string()
}

/// The design folder, made and kept out of git.
fn ready_folder(root: &Path) -> Option<PathBuf> {
    let dir = folder(root);
    std::fs::create_dir_all(&dir).ok()?;
    turns::exclude_runo_folder(&root.to_string_lossy());
    Some(dir)
}

/// Copy an image into the design folder as a new reference.
pub(crate) fn add_file(root: &Path, image: &Path) -> Option<Reference> {
    let dir = ready_folder(root)?;
    let file = file_name(&stamp(), &image.file_name()?.to_string_lossy());
    std::fs::copy(image, dir.join(&file)).ok()?;
    Some(Reference {
        file,
        note: String::new(),
        added: unix_now(),
    })
}

/// Save image bytes (a paste or a capture) as a new reference.
pub(crate) fn add_bytes(root: &Path, bytes: &[u8], name: &str, note: &str) -> Option<Reference> {
    let dir = ready_folder(root)?;
    let file = file_name(&stamp(), name);
    std::fs::write(dir.join(&file), bytes).ok()?;
    Some(Reference {
        file,
        note: note.to_string(),
        added: unix_now(),
    })
}

/// Where the project keeps its design notes: an existing `DESIGN.md` or
/// `docs/design.md`, else `docs/design.md` once something is written.
pub(crate) fn notes_path(root: &Path) -> PathBuf {
    for name in ["DESIGN.md", "design.md", "docs/DESIGN.md", "docs/design.md"] {
        let path = root.join(name);
        if path.is_file() {
            return path;
        }
    }
    root.join("docs").join("design.md")
}

/// What a session started from the Design page brings along: the design notes, the
/// picked references with their notes, and the page open in the preview.
pub(crate) fn context(
    project: &str,
    notes: &str,
    file: &str,
    refs: &[(String, String)],
    preview: Option<&str>,
) -> String {
    let mut out = format!("## Design for {project}\n");
    if !refs.is_empty() {
        out.push_str("Design references the person picked, attached as images:\n");
        for (path, note) in refs {
            if note.trim().is_empty() {
                out.push_str(&format!("- {path}\n"));
            } else {
                out.push_str(&format!("- {path}: {}\n", note.trim()));
            }
        }
    }
    if let Some(url) = preview {
        out.push_str(&format!(
            "\nThe running app is open at {url}; check your work there.\n"
        ));
    }
    if !notes.trim().is_empty() {
        out.push_str(&format!(
            "\n## Design notes ({file})\nFollow these.\n\n{}\n",
            notes.trim()
        ));
    }
    out
}

/// The browser page key of a project's design preview, apart from every session's.
pub(crate) fn page_key(project: u64) -> u64 {
    u64::MAX - project
}

/// Tasks offered as one-click starts.
const QUICK: [(&str, &str); 4] = [
    (
        "Build to match",
        "Build or change the UI so it matches the attached design references. Keep to the existing components and styles where they fit.",
    ),
    (
        "Compare with the app",
        "Compare the running app with the attached design references. List every difference in layout, spacing, type, colour and states, most visible first, then fix them.",
    ),
    (
        "Write a design brief",
        "Write a short design brief in the design notes file from the attached references: who it is for, the main flows, every screen with its states, and the visual language (type, colour, spacing).",
    ),
    (
        "Extract design tokens",
        "Extract design tokens (colours, type scale, spacing, radii, shadows) from the attached references and set them up the way this project defines styles.",
    ),
];

/// The page's own state.
pub(crate) struct DesignPage {
    pub notes: Entity<TextareaState>,
    pub task: Entity<InputState>,
    /// The note of the reference being looked at.
    pub caption: Entity<InputState>,
    /// The project whose references and notes are loaded.
    pub project: Option<u64>,
    /// The page was on screen last frame; coming back to it reads the files again.
    pub shown: bool,
    pub refs: Vec<Reference>,
    /// Files of the references picked to go with the next session.
    pub picked: Vec<String>,
    /// The reference shown large, with its note.
    pub focus: Option<String>,
    pub file: Option<PathBuf>,
    pub loading: bool,
    pub saved: Option<std::time::Instant>,
    /// The app's dev server is shown beside the references.
    pub preview: bool,
    pub capturing: bool,
}

impl DesignPage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<RunoApp>) -> (Self, Vec<Subscription>) {
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(
                "Who it is for, the look and feel, and rules for spacing, type and colour.",
            )
        });
        let task = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("What should the agent do with these references?")
        });
        let caption = cx.new(|cx| {
            InputState::new(window, cx).placeholder("What this reference is for, then Enter")
        });
        let subs = vec![
            cx.subscribe_in(&notes, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.design_notes_changed(cx);
                    cx.notify();
                }
            }),
            cx.subscribe_in(&task, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let task = input.read(cx).value().trim().to_string();
                    this.start_design_session(task, window, cx);
                }
                cx.notify();
            }),
            cx.subscribe_in(&caption, window, |this, input, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let note = input.read(cx).value().trim().to_string();
                    this.set_caption(note);
                }
                cx.notify();
            }),
        ];
        (
            Self {
                notes,
                task,
                caption,
                project: None,
                shown: false,
                refs: Vec::new(),
                picked: Vec::new(),
                focus: None,
                file: None,
                loading: false,
                saved: None,
                preview: false,
                capturing: false,
            },
            subs,
        )
    }
}

impl RunoApp {
    fn design_root(&self) -> Option<(u64, String, PathBuf)> {
        let pid = self.design.project?;
        let p = self.state.projects.iter().find(|p| p.id == pid)?;
        Some((pid, p.name.clone(), PathBuf::from(&p.path)))
    }

    /// The browser page shown on the Design page, while its preview is open.
    pub(crate) fn design_page_key(&self) -> Option<u64> {
        (self.view == View::Design && self.design.preview && !self.all_scope())
            .then_some(self.design.project)
            .flatten()
            .map(page_key)
    }

    /// Load the current project's references and notes, when the page shows another
    /// project.
    pub(crate) fn prepare_design(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.all_scope() {
            return;
        }
        let Some(project) = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            return;
        };
        let back = !std::mem::replace(&mut self.design.shown, true);
        let same = self.design.project == Some(project.id);
        if same && !back {
            return;
        }
        let root = PathBuf::from(&project.path);
        self.design.project = Some(project.id);
        self.design.refs = if self.demo { Vec::new() } else { load(&root) };
        if same {
            // Picks survive a visit elsewhere, as long as their files do.
            let refs = &self.design.refs;
            self.design
                .picked
                .retain(|f| refs.iter().any(|r| &r.file == f));
            if !self
                .design
                .focus
                .as_ref()
                .is_some_and(|f| refs.iter().any(|r| &r.file == f))
            {
                self.design.focus = None;
            }
        } else {
            self.design.picked.clear();
            self.design.focus = None;
        }
        self.design.saved = None;
        let file = notes_path(&root);
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        self.design.file = Some(file);
        self.design.loading = true;
        self.design
            .notes
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.design.loading = false;
    }

    fn design_notes_changed(&mut self, cx: &mut Context<Self>) {
        if self.design.loading || self.demo {
            return;
        }
        let Some(file) = self.design.file.clone() else {
            return;
        };
        let text = self.design.notes.read(cx).value().to_string();
        match architecture::save_notes(&file, &text) {
            Ok(()) => self.design.saved = Some(std::time::Instant::now()),
            Err(e) => self.fail("design", format!("Couldn't save {}: {e}", file.display())),
        }
    }

    fn save_refs(&mut self) {
        if let Some((_, _, root)) = self.design_root() {
            if let Err(e) = save(&root, &self.design.refs) {
                self.fail(
                    "design",
                    format!("Couldn't save the design references: {e}"),
                );
            }
        }
    }

    fn push_refs(&mut self, added: Vec<Reference>) {
        if added.is_empty() {
            return;
        }
        for r in &added {
            self.design.picked.push(r.file.clone());
        }
        self.design.refs.extend(added);
        self.save_refs();
    }

    /// Add images as references: from the clipboard when it holds some, else picked files.
    pub(crate) fn add_design_refs(&mut self, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some((_, _, root)) = self.design_root() else {
            return;
        };
        if let Some(item) = cx.read_from_clipboard() {
            let mut added = Vec::new();
            for entry in item.entries() {
                match entry {
                    ClipboardEntry::Image(image) => {
                        let ext = match image.format {
                            ImageFormat::Jpeg => "jpg",
                            ImageFormat::Gif => "gif",
                            ImageFormat::Webp => "webp",
                            ImageFormat::Bmp => "bmp",
                            _ => "png",
                        };
                        added.extend(add_bytes(&root, &image.bytes, &format!("paste.{ext}"), ""));
                    }
                    ClipboardEntry::ExternalPaths(paths) => {
                        for path in paths.paths().iter().filter(|p| turns::is_image(p)) {
                            added.extend(add_file(&root, path));
                        }
                    }
                    _ => {}
                }
            }
            if !added.is_empty() {
                self.push_refs(added);
                return;
            }
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add references".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |this, cx| {
                    this.add_design_paths(&paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn add_design_paths(&mut self, paths: &[PathBuf]) {
        let Some((_, _, root)) = self.design_root() else {
            return;
        };
        let mut added = Vec::new();
        for path in paths {
            if !turns::is_image(path) {
                self.fail(
                    "design",
                    format!(
                        "{} is not an image. References are PNG, JPEG, GIF, WebP or BMP.",
                        path.display()
                    ),
                );
                continue;
            }
            match add_file(&root, path) {
                Some(r) => added.push(r),
                None => self.fail("design", format!("Couldn't add {}.", path.display())),
            }
        }
        self.push_refs(added);
    }

    /// Save the preview page as it looks now as a reference.
    fn capture_preview(&mut self) {
        let (Some(key), Some(engine), Some((pid, _, root))) = (
            self.design_page_key(),
            self.browser.clone(),
            self.design_root(),
        ) else {
            return;
        };
        if self.design.capturing {
            return;
        }
        let url = engine.info(key).map(|i| i.url).unwrap_or_default();
        self.design.capturing = true;
        self.bg(
            move || {
                let png = engine.screenshot(key)?;
                add_bytes(&root, &png, "capture.png", &format!("Captured from {url}"))
            },
            move |this, added, cx| {
                this.design.capturing = false;
                match added {
                    Some(r) if this.design.project == Some(pid) => this.push_refs(vec![r]),
                    Some(_) => {}
                    None => this.fail("design", "Couldn't capture the page.".into()),
                }
                cx.notify();
            },
        );
    }

    fn remove_ref(&mut self, file: &str) {
        if let Some((_, _, root)) = self.design_root() {
            let _ = std::fs::remove_file(folder(&root).join(file));
        }
        self.design.refs.retain(|r| r.file != file);
        self.design.picked.retain(|f| f != file);
        if self.design.focus.as_deref() == Some(file) {
            self.design.focus = None;
        }
        self.save_refs();
    }

    fn toggle_pick(&mut self, file: &str) {
        if let Some(i) = self.design.picked.iter().position(|f| f == file) {
            self.design.picked.remove(i);
        } else {
            self.design.picked.push(file.to_string());
        }
    }

    fn focus_ref(&mut self, file: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let note = file
            .as_ref()
            .and_then(|f| self.design.refs.iter().find(|r| &r.file == f))
            .map(|r| r.note.clone())
            .unwrap_or_default();
        self.design.focus = file;
        self.design
            .caption
            .update(cx, |input, cx| input.set_value(note, window, cx));
    }

    fn set_caption(&mut self, note: String) {
        let Some(file) = self.design.focus.clone() else {
            return;
        };
        if let Some(r) = self.design.refs.iter_mut().find(|r| r.file == file) {
            r.note = note;
        }
        self.save_refs();
        self.info = Some(("Note saved.".into(), std::time::Instant::now()));
    }

    /// Open the New session dialog with the picked references and the design notes.
    pub(crate) fn start_design_session(
        &mut self,
        task: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((pid, name, root)) = self.design_root() else {
            return;
        };
        let task = if task.is_empty() {
            QUICK[0].1.to_string()
        } else {
            task
        };
        let dir = folder(&root);
        let picked: Vec<(String, String)> = self
            .design
            .picked
            .iter()
            .filter_map(|f| self.design.refs.iter().find(|r| &r.file == f))
            .map(|r| {
                (
                    dir.join(&r.file).to_string_lossy().into_owned(),
                    r.note.clone(),
                )
            })
            .collect();
        let preview = self
            .design_page_key()
            .and_then(|key| self.browser.as_ref()?.info(key))
            .map(|i| i.url)
            .filter(|u| !u.is_empty() && u != "about:blank");
        let notes = self.design.notes.read(cx).value().to_string();
        let file = self
            .design
            .file
            .as_ref()
            .and_then(|f| f.strip_prefix(&root).ok())
            .map(|f| f.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| "docs/design.md".into());
        let text = context(&name, &notes, &file, &picked, preview.as_deref());
        let n = picked.len();
        self.design
            .task
            .update(cx, |t, cx| t.set_value("", window, cx));
        self.start_with_context(
            pid,
            &task,
            shell::StartContext {
                label: match n {
                    0 => "Design notes".into(),
                    1 => "Design notes and 1 reference".into(),
                    n => format!("Design notes and {n} references"),
                },
                text,
                images: picked.into_iter().map(|(path, _)| path).collect(),
                in_place: false,
            },
            window,
            cx,
        );
    }

    pub(crate) fn design_view(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.all_scope() {
            return shell::mode_frame(
                Mode::Design,
                "Design",
                Some(self.mode_project_chip(cx)),
                "Pick a project to see its references and design notes.",
                self.project_chooser(Mode::Design, cx),
                None,
            );
        }
        let Some((_, _, root)) = self.design_root() else {
            return shell::mode_frame(
                Mode::Design,
                "Design",
                None,
                "Collect design references and start agents from them.",
                div().child(empty_hint(
                    "design-empty",
                    IconName::FolderPlus,
                    "No project yet",
                    "Add a project to collect design references for it.",
                )),
                None,
            );
        };
        let dir = folder(&root);
        let browser_on = cx.global::<AppSettings>().browser_enabled;
        let page_open = self
            .design_page_key()
            .and_then(|key| self.browser.as_ref()?.info(key))
            .is_some();

        // The references, as a grid of thumbnails to pick from.
        let mut grid = div().flex().flex_wrap().gap_3();
        for (i, r) in self.design.refs.iter().enumerate() {
            let file = r.file.clone();
            let picked = self.design.picked.contains(&r.file);
            let focused = self.design.focus.as_deref() == Some(r.file.as_str());
            let label = if r.note.is_empty() {
                r.file.clone()
            } else {
                r.note.clone()
            };
            let (f1, f2, f3) = (file.clone(), file.clone(), file.clone());
            grid = grid.child(
                div()
                    .id(("design-ref", i))
                    .w(px(168.))
                    .v_flex()
                    .rounded_lg()
                    .border_1()
                    .overflow_hidden()
                    .cursor_pointer()
                    .bg(tint(c_raised()))
                    .border_color(rgb(if picked {
                        c_blue()
                    } else if focused {
                        c_secondary()
                    } else {
                        c_border()
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_pick(&f1);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .relative()
                            .h(px(112.))
                            .bg(tint(c_subtle()))
                            .child(
                                img(dir.join(&r.file))
                                    .size_full()
                                    .object_fit(ObjectFit::Cover),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(6.))
                                    .left(px(6.))
                                    .size(px(18.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(if picked { c_blue() } else { c_border() }))
                                    .bg(rgb(if picked { c_blue() } else { c_canvas() }))
                                    .when(picked, |el| {
                                        el.child(icon(IconName::Check, 12., c_canvas()))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(rgb(if r.note.is_empty() {
                                        c_muted()
                                    } else {
                                        c_secondary()
                                    }))
                                    .child(label),
                            )
                            .child(
                                self.ghost_button_sized(
                                    ("design-ref-open", i),
                                    IconName::Pencil,
                                    "Look and add a note",
                                    22.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        let open =
                                            this.design.focus.as_deref() != Some(f2.as_str());
                                        this.focus_ref(open.then(|| f2.clone()), window, cx);
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                self.ghost_button_sized(
                                    ("design-ref-remove", i),
                                    IconName::Trash,
                                    "Remove",
                                    22.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.remove_ref(&f3);
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            );
        }

        let picked = self.design.picked.len();
        let refs_card = div()
            .v_flex()
            .gap_3()
            .p_4()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("References"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .child(if self.design.refs.is_empty() {
                                        "Mockups, screenshots and inspiration. Kept in .runo/design, out of git.".to_string()
                                    } else {
                                        format!(
                                            "{} kept, {picked} picked for the next session. Click to pick.",
                                            self.design.refs.len()
                                        )
                                    }),
                            ),
                    )
                    .when(page_open, |el| {
                        el.child(
                            self.button(
                                "design-capture",
                                if self.design.capturing { "Capturing…" } else { "Capture preview" },
                                false,
                            )
                            .child(icon(IconName::Camera, 13., c_secondary()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.capture_preview();
                                cx.notify();
                            })),
                        )
                    })
                    .child(
                        self.button("design-add", "Add images", false)
                            .child(icon(IconName::ImagePlus, 13., c_secondary()))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Adds the image on the clipboard, or pick image files.",
                                )
                                .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.add_design_refs(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(if self.design.refs.is_empty() {
                div()
                    .py_6()
                    .flex()
                    .justify_center()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child("No references yet. Copy an image and click Add images, or capture the preview.")
                    .into_any_element()
            } else {
                grid.into_any_element()
            })
            .when_some(
                self.design
                    .focus
                    .as_ref()
                    .and_then(|f| self.design.refs.iter().find(|r| &r.file == f)),
                |el, r| {
                    el.child(
                        div()
                            .v_flex()
                            .gap_2()
                            .pt_3()
                            .border_t_1()
                            .border_color(rgb(c_border()))
                            .child(
                                div()
                                    .flex()
                                    .justify_center()
                                    .max_h(px(360.))
                                    .child(
                                        img(dir.join(&r.file))
                                            .max_h(px(360.))
                                            .max_w_full()
                                            .object_fit(ObjectFit::Contain),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1().child(Input::new(&self.design.caption).bordered(true).h(px(32.))))
                                    .child(
                                        self.ghost_button_sized("design-focus-close", IconName::X, "Close", 28.)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.focus_ref(None, window, cx);
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                },
            );

        let saved = match self.design.saved {
            Some(at) if at.elapsed() < Duration::from_secs(3) => "Saved",
            _ => "Saved as you type",
        };
        let notes_card = div()
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .overflow_hidden()
            .child(
                div()
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Design notes"),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_family(self.mono_font.clone())
                            .text_color(rgb(c_muted()))
                            .child(
                                self.design
                                    .file
                                    .as_ref()
                                    .and_then(|f| f.strip_prefix(&root).ok())
                                    .map(|f| f.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(saved),
                    ),
            )
            .child(
                div().px_4().py_3().text_size(px(13.5)).child(
                    Textarea::new(&self.design.notes)
                        .appearance(false)
                        .bordered(false)
                        .h(px(200.)),
                ),
            );

        let mut quick = div().flex().flex_wrap().gap_1();
        for (j, (label, task)) in QUICK.iter().enumerate() {
            quick = quick.child(
                div()
                    .id(("design-quick", j))
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_design_session(task.to_string(), window, cx);
                        cx.notify();
                    }))
                    .child(icon(IconName::Sparkles, 11., c_muted()))
                    .child(*label),
            );
        }
        let start_card = div()
            .v_flex()
            .gap_2()
            .p_4()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Start an agent"))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child(match picked {
                        0 => "The session starts with the design notes. Pick references to attach them as images.".to_string(),
                        1 => "The session starts with the design notes and the picked reference attached.".to_string(),
                        n => format!("The session starts with the design notes and the {n} picked references attached."),
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.design.task).bordered(true).h(px(32.))))
                    .child(
                        self.button("design-start", "Start", true)
                            .child(icon(IconName::ArrowRight, 13., c_canvas()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let task = this.design.task.read(cx).value().trim().to_string();
                                this.start_design_session(task, window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(quick);

        let left = div()
            .flex_1()
            .min_w(px(360.))
            .v_flex()
            .gap_4()
            .child(refs_card)
            .child(start_card)
            .child(notes_card);

        let preview_toggle = self
            .button(
                "design-preview",
                if self.design.preview {
                    "Hide preview"
                } else {
                    "Show preview"
                },
                false,
            )
            .child(icon(IconName::PanelRight, 13., c_secondary()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.design.preview = !this.design.preview;
                if this.design.preview {
                    this.scan_ports();
                }
                cx.notify();
            }));

        let body = if self.design.preview {
            let right = if browser_on {
                div()
                    .flex_1()
                    .min_w(px(360.))
                    .h(px(
                        (f32::from(window.viewport_size().height) - 170.).max(480.)
                    ))
                    .v_flex()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(panel(c_sidebar()))
                    .overflow_hidden()
                    .child(self.browser_tab(cx))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w(px(320.))
                    .p_6()
                    .v_flex()
                    .items_center()
                    .gap_3()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .child(empty_hint(
                        "design-browser-off",
                        IconName::Globe,
                        "The built-in browser is off",
                        "Turn it on to see your app's dev server beside the references.",
                    ))
                    .child(
                        self.button("design-browser-on", "Turn on the browser", true)
                            .on_click(cx.listener(|this, _, _, cx| {
                                settings::update(cx, |s| s.browser_enabled = true);
                                this.scan_ports();
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            };
            div().flex().items_start().gap_4().child(left).child(right)
        } else {
            div().flex().child(left)
        };
        shell::mode_frame(
            Mode::Design,
            "Design",
            Some(self.mode_project_chip(cx)),
            "Collect references, keep design notes, see the running app beside them, and start agents with the references attached.",
            body,
            Some(preview_toggle.into_any_element()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-design-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn references_are_saved_and_found() {
        let root = temp("refs");
        let a = add_bytes(&root, b"png", "home screen.png", "The landing page").unwrap();
        assert!(a.file.ends_with("-home-screen.png"));
        save(&root, std::slice::from_ref(&a)).unwrap();
        // An image dropped into the folder by hand shows up after the saved ones.
        std::fs::write(folder(&root).join("zz.jpg"), b"jpg").unwrap();
        std::fs::write(folder(&root).join("notes.txt"), b"not an image").unwrap();
        let refs = load(&root);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0], a);
        assert_eq!(refs[1].file, "zz.jpg");
        // A reference whose file is gone is dropped.
        std::fs::remove_file(folder(&root).join(&a.file)).unwrap();
        assert_eq!(load(&root).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn names_are_safe() {
        assert_eq!(file_name("1", "My Shot (2).png"), "1-My-Shot--2-.png");
        assert_eq!(file_name("1", ".hidden.png"), "1-hidden.png");
    }

    #[test]
    fn notes_file_prefers_an_existing_one() {
        let root = temp("notes");
        assert_eq!(notes_path(&root), root.join("docs/design.md"));
        std::fs::write(root.join("DESIGN.md"), "x").unwrap();
        assert_eq!(notes_path(&root), root.join("DESIGN.md"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn context_lists_references_and_notes() {
        let refs = vec![
            (
                "/p/.runo/design/a.png".to_string(),
                "Login screen".to_string(),
            ),
            ("/p/.runo/design/b.png".to_string(), String::new()),
        ];
        let text = context(
            "shop",
            "Use 8px spacing.",
            "docs/design.md",
            &refs,
            Some("http://localhost:5173/"),
        );
        assert!(text.starts_with("## Design for shop\n"));
        assert!(text.contains("- /p/.runo/design/a.png: Login screen\n"));
        assert!(text.contains("- /p/.runo/design/b.png\n"));
        assert!(text.contains("open at http://localhost:5173/"));
        assert!(text.contains("## Design notes (docs/design.md)"));
        let bare = context("shop", "  ", "docs/design.md", &[], None);
        assert_eq!(bare, "## Design for shop\n");
    }

    #[test]
    fn preview_pages_do_not_meet_session_pages() {
        assert_ne!(page_key(1), 1);
        assert_ne!(page_key(1), page_key(2));
    }
}
