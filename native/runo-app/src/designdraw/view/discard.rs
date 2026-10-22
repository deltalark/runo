//! Throwing away the design's changes since the project's last commit: one screen, or the
//! whole design folder, so git shows no changes there. Nothing is committed, and Undo on
//! the bar puts the files back.

use super::figma::{caption, sec};
use super::*;

impl DesignCanvas {
    /// Look again, off the UI thread, at which design files differ from the last commit.
    pub(super) fn refresh_uncommitted(&mut self, cx: &mut Context<Self>) {
        let dir = self.root.to_string_lossy().into_owned();
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn(async move {
                    if crate::git::is_repo(&dir) {
                        crate::git::changed_under(&dir, FOLDER)
                    } else {
                        Vec::new()
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.uncommitted != files {
                    this.uncommitted = files;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Whether a screen's file differs from the last commit.
    pub(super) fn screen_uncommitted(&self, id: &str) -> bool {
        let file = format!("{FOLDER}/{SCREENS}/{id}.html");
        self.uncommitted.contains(&file)
    }

    /// The design as it was before what's thrown away now, for Undo: before an earlier
    /// change still waiting to be kept, so Undo takes back both. None while AI works on
    /// the design or its change waits to be kept.
    fn before_discard(&mut self, cx: &mut Context<Self>) -> Option<HashMap<PathBuf, Vec<u8>>> {
        match self.ai.take() {
            Some(ai) if ai.by_ai => {
                self.ai = Some(ai);
                self.say("Keep or undo the AI change first.");
                cx.notify();
                None
            }
            Some(ai) => Some(ai.before),
            None => Some(snapshot(&folder(&self.root))),
        }
    }

    /// Put the selected screen back as the last commit has it, with its place on the
    /// canvas; a screen the commit didn't have goes.
    pub(super) fn discard_screen(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        let Some(before) = self.before_discard(cx) else {
            return;
        };
        let dir = self.root.to_string_lossy().into_owned();
        let name = self
            .canvas
            .screen(&id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| id.clone());
        let file = screen_path(&self.root, &id);
        let written =
            match crate::git::committed_text(&dir, &format!("{FOLDER}/{SCREENS}/{id}.html")) {
                Some(text) => std::fs::write(&file, text),
                None => std::fs::remove_file(&file),
            };
        if let Err(e) = written {
            self.ai = Some(AiChange::by_hand(
                "Put the design back as it was".into(),
                before,
            ));
            self.error = Some(format!("Couldn't put {id}.html back: {e}"));
            cx.notify();
            return;
        }
        // Its place on the canvas, as the commit had it.
        let then_text = crate::git::committed_text(&dir, &format!("{FOLDER}/canvas.toml"));
        let then: Canvas = then_text
            .as_deref()
            .and_then(|t| toml::from_str(t).ok())
            .unwrap_or_default();
        if let Some(i) = self.canvas.screens.iter().position(|s| s.id == id) {
            match then.screen(&id) {
                Some(was) => self.canvas.screens[i] = was.clone(),
                None => {
                    self.canvas.screens.remove(i);
                }
            }
        }
        // Written without the autosave a save brings, which would be a change of its own.
        // The commit's own text when the canvas is as it had it, however it was written.
        let canvas_file = folder(&self.root).join("canvas.toml");
        let saved = match then_text.filter(|_| self.canvas == then) {
            Some(text) => std::fs::write(&canvas_file, text),
            // Screens found in the folder are placed without a canvas file; don't start one.
            None if !canvas_file.exists() => Ok(()),
            None => save(&self.root, &self.canvas),
        };
        if let Err(e) = saved {
            self.error = Some(format!("Couldn't save the canvas: {e}"));
        }
        // When only versions saved since the commit are left, the design is as it was then,
        // so they go too.
        let left = crate::git::changed_under(&dir, FOLDER);
        let versions = format!("{FOLDER}/.versions/");
        if !left.is_empty() && left.iter().all(|f| f.starts_with(&versions)) {
            let _ = crate::git::discard_under(&dir, &versions);
        }
        self.reread(cx);
        self.ai = Some(AiChange::by_hand(
            format!("Put {name} back as the last commit has it"),
            before,
        ));
        self.refresh_uncommitted(cx);
        cx.notify();
    }

    /// Put design/ back as the last commit has it, so git shows no changes there; a design
    /// never committed goes.
    pub(super) fn discard_all(&mut self, cx: &mut Context<Self>) {
        let Some(before) = self.before_discard(cx) else {
            return;
        };
        let dir = self.root.to_string_lossy().into_owned();
        if let Err(e) = crate::git::discard_under(&dir, FOLDER) {
            self.ai = Some(AiChange::by_hand(
                "Put the design back as it was".into(),
                before,
            ));
            self.error = Some(format!("Couldn't discard the design changes: {e}"));
            cx.notify();
            return;
        }
        self.reread(cx);
        self.ai = Some(AiChange::by_hand(
            "Discarded the design changes since the last commit".into(),
            before,
        ));
        self.refresh_uncommitted(cx);
        cx.notify();
    }

    /// Start over from the files, writing nothing back.
    pub(super) fn reread(&mut self, cx: &mut Context<Self>) {
        // A version saved now would be a change of its own.
        self.version_due = Some(Instant::now());
        self.canvas = load(&self.root).unwrap_or_default();
        self.seen = self.file_times();
        self.files_changed += 1;
        let gone: Vec<String> = self
            .shots
            .keys()
            .filter(|id| self.canvas.screen(id).is_none())
            .cloned()
            .collect();
        for id in gone {
            drop_shot(self.shots.remove(&id), cx);
        }
        if self
            .screen
            .as_ref()
            .is_some_and(|id| self.canvas.screen(id).is_none())
        {
            self.select_screen(None);
        }
        self.tokens = std::fs::read_to_string(folder(&self.root).join(TOKENS)).unwrap_or_default();
        self.library = crate::designdraw::library::load(&self.root);
        self.versions = crate::designdraw::versions::list(&self.root);
        // The steps on the undo history were taken on files that are gone.
        self.undo.clear();
        self.redo.clear();
        self.element = None;
        self.also.clear();
        self.draw_all(cx);
        self.load_layers(cx);
    }

    /// What's changed since the last commit, and the way to throw it all away.
    pub(super) fn uncommitted_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.uncommitted.is_empty() {
            return None;
        }
        let n = self.uncommitted.len();
        Some(
            sec("Since the last commit", None)
                .child(caption(format!(
                    "{n} file{} in {FOLDER}/ changed. Discarding puts them back as the last commit has them, so git shows no changes; nothing is committed.",
                    if n == 1 { "" } else { "s" }
                )))
                .child(
                    div().flex().child(
                        text_btn(
                            "design-discard-all",
                            "Discard all design changes",
                            Some(IconName::Trash),
                            false,
                        )
                        .tooltip(tip("Put design/ back as the last commit has it"))
                        .on_click(cx.listener(|this, _, _, cx| this.discard_all(cx))),
                    ),
                ),
        )
    }
}
