//! The Design page: each project's design canvas, and the pictures to design from
//! (mockups, screenshots and sketches) and design notes its AI asks use.

use super::*;
use crate::designdraw::view::{DesignCanvas, DesignEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// The page's own state.
pub(crate) struct DesignPage {
    /// The project the page shows.
    pub project: Option<u64>,
    /// The page was on screen last frame; coming back to it reads the files again.
    pub shown: bool,
    /// The design canvas of each project.
    pub canvases: HashMap<u64, Entity<DesignCanvas>>,
    pub canvas_subs: Vec<Subscription>,
}

impl DesignPage {
    pub(crate) fn new(_: &mut Window, _: &mut Context<RunoApp>) -> (Self, Vec<Subscription>) {
        (
            Self {
                project: None,
                shown: false,
                canvases: HashMap::new(),
                canvas_subs: Vec::new(),
            },
            Vec::new(),
        )
    }
}

impl RunoApp {
    /// Show the current project's canvas, reading its files again when the page comes
    /// back.
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
        self.ensure_design_canvas(&project, back, window, cx);
        self.design.project = Some(project.id);
        self.watch_design(Some(project.id), cx);
    }

    /// Let only the canvas on screen watch its folder.
    pub(crate) fn watch_design(&mut self, shown: Option<u64>, cx: &mut Context<Self>) {
        for (id, canvas) in &self.design.canvases {
            let on = shown == Some(*id);
            if canvas.read(cx).watching() != on {
                canvas.update(cx, |c, cx| c.set_watching(on, cx));
            }
        }
    }

    /// Make the project's design canvas, or read its files again when the page comes back.
    fn ensure_design_canvas(
        &mut self,
        project: &Project,
        back: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(canvas) = self.design.canvases.get(&project.id) {
            if back {
                canvas.update(cx, |c, cx| {
                    c.reload(cx);
                    c.load_rules(window, cx);
                });
            }
            return;
        }
        let root = PathBuf::from(&project.path);
        let canvas = cx.new(|cx| DesignCanvas::new(root, window, cx));
        let pid = project.id;
        let sub = cx.subscribe_in(&canvas, window, move |this, _, event, window, cx| {
            match event {
                DesignEvent::Ask {
                    task,
                    context,
                    edit,
                    images,
                    agent,
                    model,
                } => this.ask_from_canvas(
                    pid,
                    View::Design,
                    task,
                    shell::StartContext {
                        label: "Design canvas".into(),
                        text: context.clone(),
                        images: images.clone(),
                        in_place: *edit,
                    },
                    *agent,
                    model.clone(),
                    window,
                    cx,
                ),
            }
            cx.notify();
        });
        self.design.canvas_subs.push(sub);
        if let Some((agent, model)) = self.canvas_agent(pid, View::Design) {
            canvas.update(cx, |c, _| c.use_agent(agent, model));
        }
        self.design.canvases.insert(project.id, canvas);
    }

    /// A turn of a chat asked from a Design canvas ended: the canvas looks at what the
    /// agent made, then offers to keep or undo it.
    pub(crate) fn design_turn_done(&mut self, id: u64, stopped: bool, cx: &mut Context<Self>) {
        let Some(pid) = self
            .state
            .session(id)
            .filter(|s| shell::canvas_of(s) == Some(View::Design))
            .map(|s| s.project_id)
        else {
            return;
        };
        if let Some(canvas) = self.design.canvases.get(&pid) {
            canvas.update(cx, |c, cx| c.agent_done(stopped, cx));
        }
    }

    /// The design canvas, edge to edge: the project is in the top bar and the canvas has
    /// its own head, so the agents asked from it float over its top right corner.
    fn design_canvas(
        &self,
        pid: u64,
        canvas: Entity<DesignCanvas>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .child(div().size_full().child(canvas))
            .children(self.canvas_chats(pid, View::Design, cx).map(|chats| {
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(crate::designdraw::view::RIGHT_PANEL_W
                        + crate::shell::GUTTER
                        + 12.))
                    .child(chats)
            }))
            .into_any_element()
    }

    pub(crate) fn design_view(&self, _: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.all_scope() {
            return shell::mode_frame(
                Mode::Design,
                "Design",
                Some(self.mode_project_chip(cx)),
                "Pick a project to design its screens.",
                self.project_chooser(Mode::Design, cx),
                None,
            );
        }
        let Some(pid) = self
            .design
            .project
            .filter(|pid| self.state.projects.iter().any(|p| p.id == *pid))
        else {
            return shell::mode_frame(
                Mode::Design,
                "Design",
                None,
                "Design screens on a canvas and click through them.",
                div().child(empty_hint(
                    "design-empty",
                    IconName::FolderPlus,
                    "No project yet",
                    "Add a project to design its screens.",
                )),
                None,
            );
        };
        match self.design.canvases.get(&pid) {
            Some(canvas) => self.design_canvas(pid, canvas.clone(), cx),
            None => div().into_any_element(),
        }
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
}
