//! The design canvas: every screen as an artboard drawn by WebKit (Chromium off macOS),
//! elements picked and changed in place, a theme that restyles every screen, and a
//! prototype to click through.

use super::checks;
use super::dom::{self, Element, Layer};
use super::model::Canvas;
use super::model::*;
use super::render::{Renderer, Shot};
use crate::archdraw::view::{chip_btn, col, icon_btn, label_row, section, text_btn, tip};
use crate::*;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use serde_json::json;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

actions!(design_canvas, [DesignEscape]);

pub(crate) fn bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "escape",
        DesignEscape,
        Some("DesignCanvas"),
    )]
}

/// What the canvas asks of the app around it.
pub(crate) enum DesignEvent {
    /// Start an agent session with this task and context.
    Ask {
        task: String,
        context: String,
        edit: bool,
        images: Vec<String>,
        /// The agent to hand it to, by index into `agent::PROVIDERS`.
        agent: usize,
    },
}

type Pt = (f32, f32);
/// From one point to another, for a link arrow.
type Arrow = (Pt, Pt);

const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 3.;
/// The strip above an artboard that holds its name; drag it to move the screen.
const TITLE_H: f32 = 26.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Inspect,
    Layers,
    Theme,
    Checks,
}

enum Drag {
    None,
    Pan {
        from: Pt,
        start: Pt,
    },
    Move {
        id: String,
        from: Pt,
        start: Pt,
        moved: bool,
    },
    /// Linking the selected element to another screen.
    Link {
        from: Pt,
        to: Pt,
    },
    /// Moving the selected element to a new place on its screen.
    Element {
        id: String,
        from: Pt,
        at: Pt,
        moved: bool,
    },
    /// Resizing the selected element from its right edge, bottom edge or corner.
    Resize {
        wide: bool,
        tall: bool,
        from: Pt,
        at: Pt,
    },
}

/// Where a dragged element would land: before or after the element at `target`.
struct Drop {
    target: dom::Path,
    after: bool,
    /// The line showing it, in canvas units.
    line: (Pt, Pt),
}

/// What to select once an edit lands.
enum After {
    Same,
    Path(dom::Path),
    Clear,
}

/// A layer being dragged to a new place in the layers panel.
#[derive(Clone)]
struct DraggedLayer {
    path: dom::Path,
    label: SharedString,
}

impl Render for DraggedLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(24.))
            .px_2()
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(rgb(c_blue()))
            .bg(rgb(c_raised()))
            .shadow_md()
            .text_size(px(12.))
            .text_color(rgb(c_text()))
            .child(self.label.clone())
    }
}

pub(crate) struct DesignCanvas {
    root: PathBuf,
    canvas: Canvas,
    renderer: Arc<Mutex<Renderer>>,
    shots: HashMap<String, Shot>,
    /// Screens being drawn now, and screens to draw again once they are.
    drawing: HashSet<String>,
    stale: HashSet<String>,
    error: Option<String>,
    notice: Option<(String, std::time::Instant)>,
    focus: FocusHandle,
    pan: Pt,
    zoom: f32,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    fitted: bool,
    drag: Drag,
    /// The element under the pointer, on which screen, and where.
    hover: Option<(String, dom::Path, dom::Box)>,
    /// Alt is held: show the spacing between the selection and what's under the pointer.
    measure: bool,
    drop: Option<Drop>,
    /// The selected screen, and the element picked in it.
    screen: Option<String>,
    element: Option<Element>,
    layers: Vec<Layer>,
    /// Each screen's HTML before a change, for undo and redo.
    undo: Vec<(String, String)>,
    redo: Vec<(String, String)>,
    panel: Panel,
    tokens: String,
    dark: bool,
    /// Links between screens drawn on the canvas and outlined in Play.
    hotspots: bool,
    new_open: bool,
    new_device: Device,
    /// Clicking through the prototype: the screen shown and the way back.
    playing: Option<String>,
    play_back: Vec<String>,
    f_text: Entity<InputState>,
    f_name: Entity<InputState>,
    f_inline: Entity<InputState>,
    inline_open: bool,
    /// What the text fields were filled for.
    loaded: Option<(String, dom::Path)>,
    loaded_screen: Option<String>,
    /// When each file in `design/` last changed, to notice edits made outside the canvas.
    seen: HashMap<PathBuf, SystemTime>,
    f_ask: Entity<InputState>,
    /// The change an agent was asked for, while it works and until it's kept or undone.
    ai: Option<AiChange>,
    /// The ask bar is describing a new screen for this device.
    ask_new: Option<Device>,
    /// The agent the ask bar hands its task to.
    pick: crate::agent_pick::AgentPick,
    _subs: Vec<Subscription>,
}

/// An agent's change to the design: the files as they were, to undo it.
struct AiChange {
    task: String,
    before: HashMap<PathBuf, Vec<u8>>,
    /// Files changed since it was asked for.
    changed: bool,
    /// Made by an agent, rather than by picking a variant.
    by_ai: bool,
    /// When it was asked for, to show how long the agent has been working.
    started: Instant,
}

impl EventEmitter<DesignEvent> for DesignCanvas {}

impl Focusable for DesignCanvas {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DesignCanvas {
    pub(crate) fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (canvas, error) = match load(&root) {
            Ok(c) => (c, None),
            Err(e) => (Canvas::default(), Some(e)),
        };
        let tokens = std::fs::read_to_string(folder(&root).join(TOKENS)).unwrap_or_default();
        let mut subs = Vec::new();
        let f_text = cx.new(|cx| InputState::new(window, cx).placeholder("Text"));
        subs.push(
            cx.subscribe_in(&f_text, window, |this, state, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = state.read(cx).value().to_string();
                    this.set_text(v, cx);
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        window.focus(&this.focus, cx);
                    }
                }
            }),
        );
        let f_inline = cx.new(|cx| InputState::new(window, cx));
        subs.push(
            cx.subscribe_in(&f_inline, window, |this, state, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur)
                    && this.inline_open
                {
                    this.inline_open = false;
                    let v = state.read(cx).value().to_string();
                    this.set_text(v, cx);
                    window.focus(&this.focus, cx);
                }
            }),
        );
        let f_name = cx.new(|cx| InputState::new(window, cx).placeholder("Screen name"));
        subs.push(
            cx.subscribe_in(&f_name, window, |this, state, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = state.read(cx).value().trim().to_string();
                    this.rename_screen(v, cx);
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        window.focus(&this.focus, cx);
                    }
                }
            }),
        );
        let f_ask = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Ask AI to design a screen or change this one…")
        });
        subs.push(
            cx.subscribe_in(&f_ask, window, |this, _, ev, window, cx| match ev {
                InputEvent::PressEnter { .. } => this.send_ask(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
        );
        let mut this = Self {
            renderer: Arc::new(Mutex::new(Renderer::new(root.clone()))),
            root,
            canvas,
            shots: HashMap::new(),
            drawing: HashSet::new(),
            stale: HashSet::new(),
            error,
            notice: None,
            focus: cx.focus_handle(),
            pan: (40., 60.),
            zoom: 0.5,
            bounds: Rc::new(Cell::new(Bounds::default())),
            fitted: false,
            drag: Drag::None,
            hover: None,
            measure: false,
            drop: None,
            screen: None,
            element: None,
            layers: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            panel: Panel::Inspect,
            tokens,
            dark: false,
            hotspots: true,
            new_open: false,
            new_device: Device::Phone,
            playing: None,
            play_back: Vec::new(),
            f_text,
            f_name,
            f_inline,
            inline_open: false,
            loaded: None,
            loaded_screen: None,
            seen: HashMap::new(),
            f_ask,
            ai: None,
            ask_new: None,
            pick: crate::agent_pick::AgentPick::new(cx),
            _subs: subs,
        };
        this.seen = this.file_times();
        let ids: Vec<String> = this.canvas.screens.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            this.draw(&id, cx);
        }
        this.watch(cx);
        this
    }

    // ---- Work on the drawing browser, off the UI thread

    /// Run `work` on the renderer in the background, then `done` with what it gave.
    fn run<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut Renderer) -> T + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let r = self.renderer.clone();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_executor()
                .spawn(async move {
                    let mut g = r.lock().unwrap_or_else(|e| e.into_inner());
                    work(&mut g)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                done(this, out, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Draw a screen from its file, again if it's being drawn now.
    fn draw(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(s) = self.canvas.screen(id) else {
            return;
        };
        if !self.drawing.insert(id.to_string()) {
            self.stale.insert(id.to_string());
            return;
        }
        let (id, device) = (id.to_string(), s.device);
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.shoot(&id, device),
            move |this, out, cx| {
                this.drawing.remove(&id2);
                match out {
                    Ok(shot) => {
                        this.shots.insert(id2.clone(), shot);
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                if this.stale.remove(&id2) {
                    this.draw(&id2, cx);
                }
            },
        );
    }

    fn draw_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.canvas.screens.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            self.draw(&id, cx);
        }
    }

    /// When each file in the design folder last changed.
    fn file_times(&self) -> HashMap<PathBuf, SystemTime> {
        let mut out = HashMap::new();
        let dir = folder(&self.root);
        for d in [dir.clone(), dir.join("screens")] {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_file() {
                    if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                        out.insert(p, t);
                    }
                }
            }
        }
        out
    }

    /// Look for screens and styles changed outside the canvas (an agent, an editor, git)
    /// every so often, and draw them again.
    fn watch(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            if this.update(cx, |this, cx| this.check_files(cx)).is_err() {
                break;
            }
        })
        .detach();
    }

    /// Pick up changes made while the page was away.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.check_files(cx);
    }

    fn check_files(&mut self, cx: &mut Context<Self>) {
        let now = self.file_times();
        if now == self.seen {
            return;
        }
        let changed: Vec<PathBuf> = now
            .iter()
            .filter(|(p, t)| self.seen.get(*p) != Some(*t))
            .map(|(p, _)| p.clone())
            .chain(self.seen.keys().filter(|p| !now.contains_key(*p)).cloned())
            .collect();
        self.seen = now;
        if let Some(ai) = &mut self.ai {
            ai.changed = true;
        }
        self.renderer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .forget();
        let shared = changed
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == "css" || e == "js"));
        if let Ok(c) = load(&self.root) {
            let before: HashSet<String> =
                self.canvas.screens.iter().map(|s| s.id.clone()).collect();
            // Keep places from the canvas file unless it was the file that changed.
            if changed.iter().any(|p| p.ends_with("canvas.toml")) {
                self.canvas = c;
            } else {
                for s in c.screens {
                    if self.canvas.screen(&s.id).is_none() {
                        self.canvas.screens.push(s);
                    }
                }
                let ids: HashSet<String> = std::fs::read_dir(folder(&self.root).join("screens"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|e| e.path().file_stem()?.to_str().map(String::from))
                    .collect();
                self.canvas.screens.retain(|s| ids.contains(&s.id));
            }
            self.shots.retain(|id, _| self.canvas.screen(id).is_some());
            let added: Vec<String> = self
                .canvas
                .screens
                .iter()
                .filter(|s| !before.contains(&s.id))
                .map(|s| s.id.clone())
                .collect();
            self.place_variants(&added);
            if !added.is_empty() {
                self.save_canvas();
            }
            for id in added {
                self.draw(&id, cx);
            }
        }
        self.tokens = std::fs::read_to_string(folder(&self.root).join(TOKENS)).unwrap_or_default();
        if shared {
            self.draw_all(cx);
        } else {
            for p in changed {
                if let Some(id) = p.file_stem().and_then(|s| s.to_str()) {
                    if p.extension().is_some_and(|e| e == "html") {
                        self.draw(id, cx);
                    }
                }
            }
        }
        if let Some(id) = self.screen.clone() {
            if self.canvas.screen(&id).is_none() {
                self.screen = None;
                self.element = None;
            } else if let Some(el) = &self.element {
                let path = el.path.clone();
                self.select_path(&id, path, cx);
            }
        }
        cx.notify();
    }

    /// Put new variants of a screen in a row under it.
    fn place_variants(&mut self, added: &[String]) {
        for id in added {
            let Some(base) = variant_of(id) else {
                continue;
            };
            let Some(b) = self.canvas.screen(&base).cloned() else {
                continue;
            };
            let n = id
                .rsplit("-v")
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            let (w, _) = b.device.size();
            let h = self
                .shots
                .get(&base)
                .map(|s| s.size.1)
                .unwrap_or(b.device.size().1);
            if let Some(s) = self.canvas.screen_mut(id) {
                s.x = b.x + (n - 1) as f32 * (w + VARIANT_GAP);
                s.y = b.y + h + VARIANT_GAP + TITLE_H;
                s.device = b.device;
            }
        }
    }

    /// Remember our own writes so they don't read as outside changes.
    fn note_writes(&mut self) {
        self.seen = self.file_times();
    }

    fn save_canvas(&mut self) {
        if let Err(e) = save(&self.root, &self.canvas) {
            self.error = Some(format!("Couldn't save the canvas: {e}"));
        }
        self.note_writes();
    }

    // ---- AI

    /// Start an agent on what's typed in the ask bar.
    fn send_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.f_ask.read(cx).value().trim().to_string();
        if typed.is_empty() {
            return;
        }
        let task = match self.ask_new {
            Some(device) => describe_task(&typed, device),
            None => typed,
        };
        if self.ask(task, window, cx) {
            self.ask_new = None;
            self.f_ask.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }

    /// Start an agent on a change to the design. False when one can't start now.
    fn ask(&mut self, task: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match &self.ai {
            Some(ai) if ai.changed => {
                self.say("Keep or undo the last AI change first.");
                cx.notify();
                return false;
            }
            Some(_) => {
                self.say("AI is already working on the design.");
                cx.notify();
                return false;
            }
            None => {}
        }
        if let Err(e) = seed(&self.root) {
            self.error = Some(format!("Couldn't set up {FOLDER}/: {e}"));
            cx.notify();
            return false;
        }
        self.save_canvas();
        let context = self.ask_context();
        self.ai = Some(AiChange {
            task: summary(&task),
            before: snapshot(&folder(&self.root)),
            changed: false,
            by_ai: true,
            started: Instant::now(),
        });
        // Leave the keys to the New session dialog that opens next.
        window.blur(cx);
        cx.emit(DesignEvent::Ask {
            task,
            context,
            edit: true,
            images: Vec::new(),
            agent: self.pick.agent,
        });
        self.pick.open = false;
        cx.notify();
        true
    }

    /// Describe a new screen in the ask bar, for AI to design.
    fn start_describe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_open = false;
        self.ask_new = Some(self.new_device);
        let input = self.f_ask.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    /// Have AI make three different takes on the selected screen, beside it.
    fn variants_with_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.screen.as_ref().and_then(|id| self.canvas.screen(id)) else {
            return;
        };
        let task = variants_task(&s.id, &s.name);
        self.ask(task, window, cx);
    }

    /// Use a variant in place of the screen it was made from: its file takes the
    /// screen's, under the screen's name, and the other variants go.
    fn use_variant(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(base) = variant_of(id).filter(|b| self.canvas.screen(b).is_some()) else {
            return;
        };
        let Some(name) = self.canvas.screen(&base).map(|s| s.name.clone()) else {
            return;
        };
        if self.ai.as_ref().is_some_and(|ai| !ai.changed) {
            self.say("Wait for AI to finish first.");
            cx.notify();
            return;
        }
        let dir = folder(&self.root).join("screens");
        let before = match self.ai.take() {
            // An AI change not yet kept: undo puts back what was there before it.
            Some(ai) => ai.before,
            None => snapshot(&folder(&self.root)),
        };
        let Ok(html) = std::fs::read_to_string(dir.join(format!("{id}.html"))) else {
            return;
        };
        let html = with_title(&html, &name);
        if let Err(e) = std::fs::write(dir.join(format!("{base}.html")), html) {
            self.error = Some(format!("Couldn't use the variant: {e}"));
            cx.notify();
            return;
        }
        let gone: Vec<String> = self
            .canvas
            .screens
            .iter()
            .filter(|s| variant_of(&s.id).as_deref() == Some(base.as_str()))
            .map(|s| s.id.clone())
            .collect();
        for v in &gone {
            let _ = std::fs::remove_file(dir.join(format!("{v}.html")));
        }
        self.canvas.screens.retain(|s| !gone.contains(&s.id));
        self.save_canvas();
        let n = id.rsplit("-v").next().unwrap_or_default();
        self.ai = Some(AiChange {
            task: format!("Used variant {n} for {name}"),
            before,
            changed: true,
            by_ai: false,
            started: Instant::now(),
        });
        self.screen = Some(base.clone());
        self.element = None;
        self.undo.clear();
        self.redo.clear();
        self.seen.clear();
        self.check_files(cx);
        cx.notify();
    }

    /// Every screen's check findings, in canvas order.
    fn findings(&self) -> Vec<(String, String, Vec<checks::Finding>)> {
        let ids: HashSet<String> = self.canvas.screens.iter().map(|s| s.id.clone()).collect();
        self.canvas
            .screens
            .iter()
            .filter_map(|s| {
                let facts = self.shots.get(&s.id)?.facts.as_ref()?;
                Some((
                    s.id.clone(),
                    s.name.clone(),
                    checks::review(facts, s.device, &ids),
                ))
            })
            .collect()
    }

    /// Have AI fix what the checks found.
    fn fix_checks(&mut self, only: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let all: Vec<_> = self
            .findings()
            .into_iter()
            .filter(|(id, _, f)| !f.is_empty() && only.as_ref().is_none_or(|o| o == id))
            .collect();
        if all.is_empty() {
            return;
        }
        self.ask(checks::fix_task(&all), window, cx);
    }

    /// Select what a finding is about.
    fn show_finding(&mut self, id: &str, path: Option<dom::Path>, cx: &mut Context<Self>) {
        self.select_screen(Some(id.to_string()));
        if let Some(path) = path {
            self.select_path(id, path, cx);
        }
        cx.notify();
    }

    /// What an agent needs to change the design: where it lives, how screens are made,
    /// and what's picked on the canvas.
    fn ask_context(&self) -> String {
        let mut out = format!(
            "## Change the design\n\
             The design is in {FOLDER}/. Each screen is a standalone HTML file, \
             {FOLDER}/screens/<id>.html, that links ../tokens.css, ../components.css and \
             ../prototype.js (copy the <head> of an existing screen). Style with the classes in \
             {FOLDER}/components.css and the CSS variables in {FOLDER}/tokens.css \
             (var(--primary), var(--surface), var(--space), var(--radius) and so on) rather than \
             raw colours and sizes, so the theme keeps working. Name a screen with <title>, set \
             its device with <meta name=\"runo:device\" content=\"phone\"> (or tablet, \
             desktop), and link screens for the prototype with data-goto=\"<other screen id>\" \
             on what's clicked. New screen files show up on the canvas by themselves. Make the \
             change by editing these files directly: Runo shows it live and the user keeps or \
             undoes it, so don't ask before editing. Don't change files outside {FOLDER}/ for \
             this request.\n"
        );
        if self.canvas.screens.is_empty() {
            out.push_str("\nThere are no screens yet.\n");
        } else {
            out.push_str("\nScreens now:\n");
            for s in &self.canvas.screens {
                out.push_str(&format!(
                    "- {} ({FOLDER}/screens/{}.html, {})\n",
                    s.name,
                    s.id,
                    s.device.label().to_lowercase()
                ));
            }
        }
        if let Some(s) = self.screen.as_ref().and_then(|id| self.canvas.screen(id)) {
            out.push_str(&format!(
                "\nSelected on the canvas (\"this\" means it): the screen {} \
                 ({FOLDER}/screens/{}.html)",
                s.name, s.id
            ));
            if let Some(el) = &self.element {
                let path: Vec<String> = el
                    .path
                    .iter()
                    .map(|i| format!(":nth-child({})", i + 1))
                    .collect();
                out.push_str(&format!(
                    ", and on it the element {}{} at `body > {}`",
                    el.label,
                    el.text
                        .as_ref()
                        .filter(|t| !t.is_empty())
                        .map(|t| format!(" \"{t}\""))
                        .unwrap_or_default(),
                    path.join(" > ")
                ));
            }
            out.push_str(".\n");
            if self.element.is_some() {
                out.push_str(
                    "Change only that element unless asked otherwise. If what's asked for isn't \
                     on the screen yet, add it inside that element when it's a container, else \
                     right after it.\n",
                );
            }
        }
        out
    }

    /// Keep what the agent changed.
    fn keep_ai(&mut self, cx: &mut Context<Self>) {
        self.ai = None;
        self.undo.clear();
        self.redo.clear();
        cx.notify();
    }

    /// Put the design back as it was before the agent's change.
    fn undo_ai(&mut self, cx: &mut Context<Self>) {
        let Some(ai) = self.ai.take() else {
            return;
        };
        if let Err(e) = restore(&folder(&self.root), &ai.before) {
            self.error = Some(format!("Couldn't undo the AI change: {e}"));
        } else {
            self.say(if ai.by_ai {
                "Undid the AI change."
            } else {
                "Put the screens back as they were."
            });
        }
        self.undo.clear();
        self.redo.clear();
        self.check_files(cx);
        cx.notify();
    }

    fn stop_ai(&mut self, cx: &mut Context<Self>) {
        self.ai = None;
        cx.notify();
    }

    /// The bar along the bottom: ask AI, then keep or undo what it did.
    fn ask_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.playing.is_some() {
            return None;
        }
        let body = match &self.ai {
            Some(ai) if ai.changed => askbar::composer("design-ask-bar", true).child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_h(px(32.))
                    .child(div().pl_1().child(icon(IconName::Sparkles, 14., c_blue())))
                    .child(
                        div()
                            .flex_1()
                            .px_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(c_text()))
                            .child(if ai.by_ai {
                                format!("AI changed the design: {}", ai.task)
                            } else {
                                ai.task.clone()
                            }),
                    )
                    .child(
                        text_btn("design-ai-undo", "Undo", Some(IconName::Undo2), false)
                            .on_click(cx.listener(|this, _, _, cx| this.undo_ai(cx))),
                    )
                    .child(
                        text_btn("design-ai-keep", "Keep", Some(IconName::Check), true)
                            .on_click(cx.listener(|this, _, _, cx| this.keep_ai(cx))),
                    ),
            ),
            Some(ai) => askbar::composer("design-ask-bar", true).child(
                askbar::working_row(
                    "design-wait-spin",
                    design_line(ai.started.elapsed().as_secs()).to_string(),
                    ai.task.clone(),
                    ai.started.elapsed().as_secs(),
                )
                .child(
                    text_btn("design-ai-stop", "Stop", None, false)
                        .on_click(cx.listener(|this, _, _, cx| this.stop_ai(cx))),
                ),
            ),
            None => {
                let empty = self.f_ask.read(cx).value().trim().is_empty();
                let screen = self.screen.as_ref().and_then(|id| self.canvas.screen(id));
                let mut chips = div().flex().flex_wrap().gap_1();
                let mut any = false;
                if let Some(d) = self.ask_new {
                    any = true;
                    chips = chips.child(
                        askbar::target_chip(
                            "design-ask-new",
                            IconName::Plus,
                            format!("New {} screen", d.label().to_lowercase()),
                        )
                        .tooltip(tip(
                            "AI designs a new screen. Click to change this one instead.",
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ask_new = None;
                            cx.notify();
                        })),
                    );
                } else if let Some(s) = screen {
                    any = true;
                    let (lead, text) = match &self.element {
                        Some(el) => (
                            IconName::Crosshair,
                            format!(
                                "{}{} on {}",
                                kind_of(&el.tag),
                                el.text
                                    .as_ref()
                                    .filter(|t| !t.is_empty())
                                    .map(|t| format!(
                                        " \"{}\"",
                                        t.chars().take(32).collect::<String>()
                                    ))
                                    .unwrap_or_default(),
                                s.name
                            ),
                        ),
                        None => (IconName::Smartphone, format!("{} screen", s.name)),
                    };
                    let on_element = self.element.is_some();
                    chips = chips.child(
                        askbar::target_chip("design-ask-target", lead, text)
                            .tooltip(tip(if on_element {
                                "AI changes this element, or adds what you ask for beside it. Click to clear."
                            } else {
                                "AI changes this screen. Click to clear."
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.element.is_some() {
                                    this.element = None;
                                } else {
                                    this.screen = None;
                                }
                                cx.notify();
                            })),
                    );
                }
                let ideas: &[&'static str] = if self.ask_new.is_some() {
                    &["A sign-in screen", "A profile with stats", "A checkout"]
                } else if self.element.is_some() {
                    &["Make it stand out", "Rewrite the text", "Add an icon to it"]
                } else if screen.is_some() {
                    &[
                        "Make a dark version",
                        "Add a bottom tab bar",
                        "Tighten the spacing",
                    ]
                } else if self.canvas.screens.is_empty() {
                    &[
                        "Design an onboarding flow",
                        "Design a sign-in screen",
                        "Design a dashboard",
                    ]
                } else {
                    &[
                        "Add a settings screen",
                        "Make the theme warmer",
                        "Link the screens up",
                    ]
                };
                let mut row = div().flex_1().min_w_0().flex().gap_1().overflow_hidden();
                if empty {
                    for (i, idea) in ideas.iter().enumerate() {
                        let idea: &'static str = idea;
                        row = row.child(askbar::suggestion(("design-idea", i), idea).on_click(
                            cx.listener(move |this, _, window, cx| this.fill_ask(idea, window, cx)),
                        ));
                    }
                }
                askbar::composer("design-ask-bar", any)
                    .when(any, |el| el.child(chips))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_h(px(32.))
                            .child(div().pl_1().child(icon(IconName::Sparkles, 14., c_blue())))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.f_ask).appearance(false)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(row)
                            .child(self.pick.chip("design-ask", cx, |t: &mut Self| &mut t.pick))
                            .child(askbar::send_button("design-ask-send", !empty).on_click(
                                cx.listener(|this, _, window, cx| this.send_ask(window, cx)),
                            )),
                    )
            }
        };
        Some(
            div()
                .absolute()
                .bottom(px(10.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .v_flex()
                        .items_end()
                        .gap_1()
                        .when(self.ai.is_none(), |el| {
                            el.children(self.pick.menu(
                                "design-ask",
                                "the screens in design/",
                                cx,
                                |t: &mut Self| &mut t.pick,
                            ))
                        })
                        .child(body),
                )
                .into_any_element(),
        )
    }

    /// Put a suggestion in the ask bar, ready to send or change.
    fn fill_ask(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.f_ask.update(cx, |s, cx| s.set_value(text, window, cx));
        self.focus_ask(window, cx);
    }

    fn focus_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.f_ask.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    /// Whether AI is designing and nothing has changed yet.
    fn thinking_now(&self) -> bool {
        self.ai.as_ref().is_some_and(|a| a.by_ai && !a.changed)
    }

    /// Shows the canvas is busy while AI designs: a sketch taking shape on an empty canvas,
    /// else a glow round the canvas and a line saying what's going on.
    fn thinking(&self) -> Option<AnyElement> {
        let ai = self.ai.as_ref().filter(|a| a.by_ai && !a.changed)?;
        let secs = ai.started.elapsed().as_secs();
        let line = design_line(secs).to_string();
        Some(if self.canvas.screens.is_empty() {
            askbar::busy_card("Designing your screens", line, Some(secs))
        } else {
            askbar::busy_edge(line, Some(secs))
        })
    }

    fn say(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), std::time::Instant::now()));
    }

    // ---- Geometry

    fn local(&self, pos: Point<Pixels>) -> Pt {
        let b = self.bounds.get();
        (f32::from(pos.x - b.origin.x), f32::from(pos.y - b.origin.y))
    }

    fn world(&self, local: Pt) -> Pt {
        (
            (local.0 - self.pan.0) / self.zoom,
            (local.1 - self.pan.1) / self.zoom,
        )
    }

    fn screen_pt(&self, p: Pt) -> Pt {
        (p.0 * self.zoom + self.pan.0, p.1 * self.zoom + self.pan.1)
    }

    fn view_size(&self) -> Pt {
        let b = self.bounds.get();
        (f32::from(b.size.width), f32::from(b.size.height))
    }

    /// A screen's size on the canvas: as drawn, else its device's.
    fn size_of(&self, s: &Screen) -> Pt {
        self.shots
            .get(&s.id)
            .map(|shot| shot.size)
            .unwrap_or_else(|| s.device.size())
    }

    /// The screen under a point, and whether the point is on its name strip.
    fn screen_at(&self, w: Pt) -> Option<(String, bool)> {
        let strip = TITLE_H / self.zoom;
        self.canvas.screens.iter().rev().find_map(|s| {
            let (sw, sh) = self.size_of(s);
            let inside_x = w.0 >= s.x && w.0 <= s.x + sw;
            if inside_x && w.1 >= s.y && w.1 <= s.y + sh {
                Some((s.id.clone(), false))
            } else if inside_x && w.1 >= s.y - strip && w.1 < s.y {
                Some((s.id.clone(), true))
            } else {
                None
            }
        })
    }

    fn fit(&mut self) {
        let (vw, vh) = self.view_size();
        if vw < 10. || self.canvas.screens.is_empty() {
            return;
        }
        let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for s in &self.canvas.screens {
            let (w, h) = self.size_of(s);
            l = l.min(s.x);
            t = t.min(s.y - 40.);
            r = r.max(s.x + w);
            b = b.max(s.y + h);
        }
        let zoom = ((vw - 80.) / (r - l))
            .min((vh - 80.) / (b - t))
            .clamp(MIN_ZOOM, 1.);
        self.zoom = zoom;
        self.pan = (
            (vw - (r - l) * zoom) / 2. - l * zoom,
            (vh - (b - t) * zoom) / 2. - t * zoom,
        );
    }

    fn zoom_at(&mut self, local: Pt, factor: f32) {
        let w = self.world(local);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = (local.0 - w.0 * self.zoom, local.1 - w.1 * self.zoom);
    }

    // ---- Selecting

    fn select_screen(&mut self, id: Option<String>) {
        if self.screen != id {
            self.element = None;
            self.layers.clear();
        }
        self.screen = id;
        self.inline_open = false;
    }

    /// Pick the element of a screen under a point given in the screen's CSS pixels.
    fn pick(&mut self, id: &str, at: Pt, edit_text: bool, cx: &mut Context<Self>) {
        let Some(device) = self.canvas.screen(id).map(|s| s.device) else {
            return;
        };
        let (id, id2) = (id.to_string(), id.to_string());
        self.run(
            cx,
            move |r| r.eval(&id, device, &dom::pick(at.0, at.1)),
            move |this, out, cx| {
                if this.screen.as_deref() != Some(id2.as_str()) {
                    return;
                }
                this.element = out.ok().and_then(|v| serde_json::from_value(v).ok());
                if edit_text && this.element.as_ref().is_some_and(|e| e.text.is_some()) {
                    this.inline_open = true;
                }
                this.panel = Panel::Inspect;
                this.load_layers(cx);
            },
        );
    }

    /// Select the element at `path` in a screen.
    fn select_path(&mut self, id: &str, path: dom::Path, cx: &mut Context<Self>) {
        let Some(device) = self.canvas.screen(id).map(|s| s.device) else {
            return;
        };
        let (id, id2) = (id.to_string(), id.to_string());
        self.run(
            cx,
            move |r| r.eval(&id, device, &dom::info(&path)),
            move |this, out, cx| {
                if this.screen.as_deref() != Some(id2.as_str()) {
                    return;
                }
                this.element = out.ok().and_then(|v| serde_json::from_value(v).ok());
                this.load_layers(cx);
            },
        );
    }

    fn load_layers(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.eval(&id, device, &dom::call("tree", &[])),
            move |this, out, _| {
                if this.screen.as_deref() == Some(id2.as_str()) {
                    this.layers = out
                        .ok()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                }
            },
        );
    }

    // ---- Changing screens

    /// Change the selected screen with `js`, save it and draw it again.
    fn edit(&mut self, js: String, after: After, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.edit(&id, device, &js),
            move |this, out, cx| match out {
                Ok(Some((before, shot))) => {
                    this.undo.push((id2.clone(), before));
                    this.redo.clear();
                    this.shots.insert(id2.clone(), shot);
                    this.note_writes();
                    match after {
                        After::Same => {
                            if let Some(p) = this.element.as_ref().map(|e| e.path.clone()) {
                                this.select_path(&id2, p, cx);
                            } else {
                                this.load_layers(cx);
                            }
                        }
                        After::Path(p) => this.select_path(&id2, p, cx),
                        After::Clear => {
                            this.element = None;
                            this.load_layers(cx);
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => this.say(e),
            },
        );
    }

    fn op(&mut self, name: &str, args: &[serde_json::Value], after: After, cx: &mut Context<Self>) {
        let Some(path) = self.element.as_ref().map(|e| e.path.clone()) else {
            return;
        };
        self.edit(dom::op(&path, name, args), after, cx);
    }

    fn set_style(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        self.op("style", &[json!(key), json!(value)], After::Same, cx);
    }

    fn set_text(&mut self, v: String, cx: &mut Context<Self>) {
        let Some(el) = &self.element else {
            return;
        };
        if el.text.is_none() || el.text.as_deref() == Some(v.as_str()) {
            return;
        }
        self.op("text", &[json!(v)], After::Same, cx);
    }

    /// Move the selected element before or after its neighbour.
    fn nudge(&mut self, by: i32, cx: &mut Context<Self>) {
        let Some(mut path) = self.element.as_ref().map(|e| e.path.clone()) else {
            return;
        };
        let Some(last) = path.last_mut() else {
            return;
        };
        if by < 0 && *last == 0 {
            return;
        }
        *last = (*last as i32 + by).max(0) as usize;
        self.op("move", &[json!(by)], After::Path(path), cx);
    }

    fn duplicate_element(&mut self, cx: &mut Context<Self>) {
        let Some(mut path) = self.element.as_ref().map(|e| e.path.clone()) else {
            return;
        };
        if let Some(last) = path.last_mut() {
            *last += 1;
        }
        self.op("duplicate", &[], After::Path(path), cx);
    }

    fn delete_element(&mut self, cx: &mut Context<Self>) {
        let parent = self.element.as_ref().and_then(|e| dom::parent(&e.path));
        let after = match parent {
            Some(p) if !p.is_empty() => After::Path(p),
            _ => After::Clear,
        };
        self.op("remove", &[], after, cx);
    }

    /// Add a building block inside the selected container, or after the selected element,
    /// or at the end of the screen.
    fn insert(&mut self, html: &str, cx: &mut Context<Self>) {
        match &self.element {
            Some(el) => {
                let inside = el.is_container();
                let mut path = el.path.clone();
                if inside {
                    path.push(el.children);
                } else if let Some(last) = path.last_mut() {
                    *last += 1;
                }
                self.op(
                    "insert",
                    &[json!(html), json!(inside)],
                    After::Path(path),
                    cx,
                );
            }
            None => {
                if self.screen.is_some() {
                    self.edit(
                        dom::op(&vec![], "insert", &[json!(html), json!(true)]),
                        After::Clear,
                        cx,
                    );
                }
            }
        }
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        self.history(true, cx);
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        self.history(false, cx);
    }

    /// Put a screen back as it was before its last change (or after, for redo).
    fn history(&mut self, back: bool, cx: &mut Context<Self>) {
        let entry = if back {
            self.undo.pop()
        } else {
            self.redo.pop()
        };
        let Some((id, html)) = entry else {
            return;
        };
        let now = std::fs::read_to_string(screen_path(&self.root, &id)).unwrap_or_default();
        if back {
            self.redo.push((id.clone(), now));
        } else {
            self.undo.push((id.clone(), now));
        }
        let res = self
            .renderer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write(&id, &html);
        if let Err(e) = res {
            self.say(e);
        }
        self.note_writes();
        self.screen = Some(id.clone());
        self.element = None;
        self.draw(&id, cx);
        self.load_layers(cx);
        cx.notify();
    }

    // ---- Screens

    fn add_screen(&mut self, kind: &str, cx: &mut Context<Self>) {
        if let Err(e) = seed(&self.root) {
            self.say(format!("Couldn't start the design: {e}"));
            return;
        }
        let name = if kind == "Blank" { "Screen" } else { kind };
        let id = self.canvas.new_id(name);
        let html = template(kind, name, self.new_device);
        if let Err(e) = std::fs::write(screen_path(&self.root, &id), html) {
            self.say(format!("Couldn't add the screen: {e}"));
            return;
        }
        let (x, y) = self.canvas.next_spot();
        self.canvas.screens.push(Screen {
            id: id.clone(),
            name: name.to_string(),
            device: self.new_device,
            x,
            y,
        });
        self.tokens = std::fs::read_to_string(folder(&self.root).join(TOKENS)).unwrap_or_default();
        self.save_canvas();
        self.new_open = false;
        self.select_screen(Some(id.clone()));
        self.draw(&id, cx);
        // Bring it into view.
        let (vw, vh) = self.view_size();
        let (w, h) = self.new_device.size();
        if self.canvas.screens.len() == 1 {
            self.fitted = false;
        } else {
            let (sx, sy) = self.screen_pt((x, y));
            if sx < 0. || sy < 0. || sx + w * self.zoom > vw || sy + h * self.zoom > vh {
                self.pan = (
                    vw / 2. - (x + w / 2.) * self.zoom,
                    vh / 2. - (y + h / 2.) * self.zoom,
                );
            }
        }
        cx.notify();
    }

    fn duplicate_screen(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self
            .screen
            .as_ref()
            .and_then(|id| self.canvas.screen(id))
            .cloned()
        else {
            return;
        };
        let name = format!("{} copy", s.name);
        let id = self.canvas.new_id(&name);
        let html = std::fs::read_to_string(screen_path(&self.root, &s.id)).unwrap_or_default();
        if std::fs::write(screen_path(&self.root, &id), with_title(&html, &name)).is_err() {
            return;
        }
        let (w, _) = self.size_of(&s);
        self.canvas.screens.push(Screen {
            id: id.clone(),
            name,
            device: s.device,
            x: s.x + w + GAP,
            y: s.y,
        });
        self.save_canvas();
        self.select_screen(Some(id.clone()));
        self.draw(&id, cx);
        cx.notify();
    }

    fn delete_screen(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        let file = screen_path(&self.root, &id);
        let html = std::fs::read_to_string(&file).unwrap_or_default();
        if std::fs::remove_file(&file).is_err() {
            return;
        }
        self.canvas.screens.retain(|s| s.id != id);
        self.shots.remove(&id);
        self.save_canvas();
        self.select_screen(None);
        self.say(format!(
            "Deleted {id}.html. It's in git if you need it back."
        ));
        let _ = html;
        cx.notify();
    }

    fn rename_screen(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        if name.is_empty() || self.canvas.screen(&id).is_some_and(|s| s.name == name) {
            return;
        }
        if let Some(s) = self.canvas.screen_mut(&id) {
            s.name = name.clone();
        }
        let file = screen_path(&self.root, &id);
        if let Ok(html) = std::fs::read_to_string(&file) {
            let _ = std::fs::write(&file, with_title(&html, &name));
        }
        self.renderer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .forget();
        self.save_canvas();
        cx.notify();
    }

    fn set_device(&mut self, device: Device, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        if let Some(s) = self.canvas.screen_mut(&id) {
            s.device = device;
        }
        let file = screen_path(&self.root, &id);
        if let Ok(html) = std::fs::read_to_string(&file) {
            let tag = |d: Device| {
                format!(
                    "name=\"runo:device\" content=\"{}\"",
                    d.label().to_lowercase()
                )
            };
            let mut out = html.clone();
            for d in Device::ALL {
                out = out.replace(&tag(d), &tag(device));
            }
            if out == html && !html.contains("runo:device") {
                out = html.replacen("<head>", &format!("<head>\n<meta {}>", tag(device)), 1);
            }
            let _ = std::fs::write(&file, out);
        }
        self.renderer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .forget();
        self.save_canvas();
        self.element = None;
        self.draw(&id, cx);
        cx.notify();
    }

    /// Lay the screens out along the links between them.
    fn arrange(&mut self, cx: &mut Context<Self>) {
        let links = self.links();
        let sizes: HashMap<String, f32> = self
            .canvas
            .screens
            .iter()
            .map(|s| (s.id.clone(), self.size_of(s).1))
            .collect();
        self.canvas
            .arrange(&links, &|id| sizes.get(id).copied().unwrap_or(844.));
        self.save_canvas();
        self.fit();
        cx.notify();
    }

    /// Every link between screens, from the drawings.
    fn links(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for s in &self.canvas.screens {
            if let Some(shot) = self.shots.get(&s.id) {
                for l in &shot.links {
                    let pair = (s.id.clone(), l.goto.clone());
                    if !out.contains(&pair) {
                        out.push(pair);
                    }
                }
            }
        }
        out
    }

    // ---- Theme

    fn theme(&self) -> Theme {
        Theme::read(&self.tokens)
    }

    fn set_theme(&mut self, change: impl FnOnce(&mut Theme), cx: &mut Context<Self>) {
        if let Err(e) = seed(&self.root) {
            self.say(format!("Couldn't write the theme: {e}"));
            return;
        }
        let file = folder(&self.root).join(TOKENS);
        let css = std::fs::read_to_string(&file).unwrap_or_default();
        let mut t = Theme::read(&css);
        change(&mut t);
        let out = t.write(&css);
        if out == css {
            return;
        }
        if let Err(e) = std::fs::write(&file, &out) {
            self.say(format!("Couldn't write the theme: {e}"));
            return;
        }
        self.tokens = out;
        self.note_writes();
        self.renderer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .forget();
        self.draw_all(cx);
        cx.notify();
    }

    fn set_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.dark = dark;
        self.renderer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_dark(dark);
        self.draw_all(cx);
        cx.notify();
    }

    /// A colour token's value in the light theme.
    fn token_color(&self, name: &str) -> Option<u32> {
        var(&self.tokens, name).and_then(|v| crate::archdraw::model::parse_hex(&v))
    }

    // ---- Prototype

    fn play(&mut self, cx: &mut Context<Self>) {
        let first = self
            .screen
            .clone()
            .or_else(|| {
                let linked: HashSet<String> = self.links().into_iter().map(|l| l.1).collect();
                self.canvas
                    .screens
                    .iter()
                    .find(|s| !linked.contains(&s.id))
                    .map(|s| s.id.clone())
            })
            .or_else(|| self.canvas.screens.first().map(|s| s.id.clone()));
        self.playing = first;
        self.play_back.clear();
        cx.notify();
    }

    fn play_go(&mut self, to: &str, cx: &mut Context<Self>) {
        if self.canvas.screen(to).is_none() {
            self.say(format!("{to}.html doesn't exist yet."));
            cx.notify();
            return;
        }
        if let Some(now) = self.playing.replace(to.to_string()) {
            self.play_back.push(now);
        }
        cx.notify();
    }

    // ---- Input

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.new_open = false;
        let local = self.local(e.position);
        let w = self.world(local);
        if e.button == MouseButton::Middle {
            self.drag = Drag::Pan {
                from: local,
                start: self.pan,
            };
            return;
        }
        // The link handle on the selected element.
        if let Some(h) = self.link_handle() {
            let s = self.screen_pt(h);
            if (local.0 - s.0).abs() < 9. && (local.1 - s.1).abs() < 9. {
                self.drag = Drag::Link { from: h, to: w };
                return;
            }
        }
        if let Some((wide, tall)) = self.resize_edge(local) {
            self.drag = Drag::Resize {
                wide,
                tall,
                from: w,
                at: w,
            };
            return;
        }
        if e.click_count == 1 {
            if let Some((id, r)) = self.picked_rect() {
                if w.0 >= r.x && w.1 >= r.y && w.0 <= r.x + r.w && w.1 <= r.y + r.h {
                    self.drag = Drag::Element {
                        id,
                        from: w,
                        at: w,
                        moved: false,
                    };
                    return;
                }
            }
        }
        match self.screen_at(w) {
            Some((id, true)) => {
                let start = self
                    .canvas
                    .screen(&id)
                    .map(|s| (s.x, s.y))
                    .unwrap_or_default();
                self.select_screen(Some(id.clone()));
                self.drag = Drag::Move {
                    id,
                    from: w,
                    start,
                    moved: false,
                };
            }
            Some((id, false)) => {
                let Some(s) = self.canvas.screen(&id) else {
                    return;
                };
                let at = (w.0 - s.x, w.1 - s.y);
                self.select_screen(Some(id.clone()));
                self.pick(&id, at, e.click_count >= 2, cx);
            }
            None => {
                self.select_screen(None);
                self.drag = Drag::Pan {
                    from: local,
                    start: self.pan,
                };
            }
        }
        cx.notify();
    }

    fn on_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let local = self.local(e.position);
        let w = self.world(local);
        if matches!(self.drag, Drag::None) {
            let hover = self.box_at(w, None);
            let measure = e.modifiers.alt;
            let same = |a: &Option<(String, dom::Path, dom::Box)>,
                        b: &Option<(String, dom::Path, dom::Box)>| {
                a.as_ref().map(|h| (&h.0, &h.1)) == b.as_ref().map(|h| (&h.0, &h.1))
            };
            if !same(&hover, &self.hover) || measure != self.measure {
                self.hover = hover;
                self.measure = measure;
                cx.notify();
            }
            return;
        }
        match &mut self.drag {
            Drag::None => return,
            Drag::Element {
                at, from, moved, ..
            } => {
                *at = w;
                if (w.0 - from.0).abs() + (w.1 - from.1).abs() > 4. / self.zoom {
                    *moved = true;
                }
                if *moved {
                    self.drop = self.drop_at(w);
                }
            }
            Drag::Resize { at, .. } => *at = w,
            Drag::Pan { from, start } => {
                self.pan = (start.0 + local.0 - from.0, start.1 + local.1 - from.1);
            }
            Drag::Move {
                id,
                from,
                start,
                moved,
            } => {
                let (dx, dy) = (w.0 - from.0, w.1 - from.1);
                if dx.abs() + dy.abs() > 2. {
                    *moved = true;
                }
                let (x, y) = (
                    ((start.0 + dx) / 10.).round() * 10.,
                    ((start.1 + dy) / 10.).round() * 10.,
                );
                let id = id.clone();
                if let Some(s) = self.canvas.screen_mut(&id) {
                    s.x = x;
                    s.y = y;
                }
            }
            Drag::Link { to, .. } => *to = w,
        }
        cx.notify();
    }

    fn on_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let drop = self.drop.take();
        self.hover = None;
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::Move { moved: true, .. } => self.save_canvas(),
            Drag::Element {
                id,
                from,
                moved: false,
                ..
            } => {
                // A click on the picked element: pick what's under the pointer, as before.
                if let Some(s) = self.canvas.screen(&id) {
                    let at = (from.0 - s.x, from.1 - s.y);
                    self.pick(&id, at, false, cx);
                }
            }
            Drag::Element { moved: true, .. } => {
                if let (Some(d), Some(path)) = (drop, self.element.as_ref().map(|e| e.path.clone()))
                {
                    let to = dom::moved_path(&path, &d.target, d.after);
                    self.edit(
                        dom::op(&path, "moveTo", &[json!(d.target), json!(d.after)]),
                        After::Path(to),
                        cx,
                    );
                }
            }
            Drag::Resize {
                wide,
                tall,
                from,
                at,
            } => {
                if let Some(el) = &self.element {
                    let mut map = serde_json::Map::new();
                    if wide {
                        let w = (el.rect.w + at.0 - from.0).max(8.).round();
                        map.insert("width".into(), json!(format!("{w}px")));
                    }
                    if tall {
                        let h = (el.rect.h + at.1 - from.1).max(8.).round();
                        map.insert("height".into(), json!(format!("{h}px")));
                    }
                    if (at.0 - from.0).abs() + (at.1 - from.1).abs() > 0.5 {
                        self.op("styles", &[serde_json::Value::Object(map)], After::Same, cx);
                    }
                }
            }
            Drag::Link { to, .. } => {
                if let Some((target, _)) = self.screen_at(to) {
                    if Some(&target) != self.screen.as_ref() {
                        self.op(
                            "attr",
                            &[json!("data-goto"), json!(target)],
                            After::Same,
                            cx,
                        );
                        let name = self
                            .canvas
                            .screen(&target)
                            .map(|s| s.name.clone())
                            .unwrap_or(target);
                        self.say(format!("Linked to {name}. Press Play to click through."));
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn on_scroll(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let local = self.local(e.position);
        let d = e.delta.pixel_delta(px(16.));
        if e.modifiers.secondary() || e.modifiers.control {
            let f = (1. - f32::from(d.y) * 0.004).clamp(0.5, 2.);
            self.zoom_at(local, f);
        } else {
            self.pan.0 += f32::from(d.x);
            self.pan.1 += f32::from(d.y);
        }
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let k = &e.keystroke;
        let m = &k.modifiers;
        let sec = m.secondary();
        let handled = match (sec, m.shift, m.alt, k.key.as_str()) {
            (true, false, _, "z") => {
                self.undo(cx);
                true
            }
            (true, true, _, "z") | (true, _, _, "y") => {
                self.redo(cx);
                true
            }
            (true, _, _, "d") => {
                if self.element.is_some() {
                    self.duplicate_element(cx);
                } else {
                    self.duplicate_screen(cx);
                }
                true
            }
            (false, _, _, "backspace" | "delete") => {
                if self.element.is_some() {
                    self.delete_element(cx);
                }
                true
            }
            (false, _, true, "up") => {
                self.nudge(-1, cx);
                true
            }
            (false, _, true, "down") => {
                self.nudge(1, cx);
                true
            }
            (false, false, false, "enter") => {
                if self.element.as_ref().is_some_and(|e| e.text.is_some()) {
                    self.inline_open = true;
                }
                true
            }
            (false, true, false, "1") => {
                self.fit();
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Escape: leave play, then the text being edited, then step out to the parent.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.playing.is_some() {
            self.playing = None;
        } else if self.inline_open {
            self.inline_open = false;
        } else if self.new_open {
            self.new_open = false;
        } else if !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
        } else if let Some(el) = &self.element {
            match dom::parent(&el.path).filter(|p| !p.is_empty()) {
                Some(p) => {
                    let id = self.screen.clone().unwrap_or_default();
                    self.select_path(&id, p, cx);
                }
                None => self.element = None,
            }
        } else {
            self.select_screen(None);
        }
        cx.notify();
    }

    /// Where the selected element's link handle sits, in canvas units.
    /// The picked element's box in canvas units, and its screen.
    fn picked_rect(&self) -> Option<(String, dom::Box)> {
        let el = self.element.as_ref()?;
        let id = self.screen.clone()?;
        let s = self.canvas.screen(&id)?;
        Some((
            id,
            dom::Box {
                x: s.x + el.rect.x,
                y: s.y + el.rect.y,
                w: el.rect.w,
                h: el.rect.h,
            },
        ))
    }

    /// Which edges of the picked element are under the pointer, for resizing it:
    /// (right edge, bottom edge).
    fn resize_edge(&self, local: Pt) -> Option<(bool, bool)> {
        let (_, r) = self.picked_rect()?;
        let (x0, y0) = self.screen_pt((r.x, r.y));
        let (x1, y1) = self.screen_pt((r.x + r.w, r.y + r.h));
        // The grip reaches a little outside the edge but only a sliver inside it, so a small
        // element can still be grabbed and moved rather than always resized.
        let near = |v: f32, edge: f32, size: f32| v <= edge + 6. && v >= edge - (size / 4.).min(4.);
        let within = |v: f32, lo: f32, hi: f32| v >= lo - 5. && v <= hi + 5.;
        let wide = near(local.0, x1, x1 - x0) && within(local.1, y0, y1);
        let tall = near(local.1, y1, y1 - y0) && within(local.0, x0, x1);
        // The link handle sits at the middle of the right edge.
        let mid = (y0 + y1) / 2.;
        if wide && !tall && (local.1 - mid).abs() < 10. {
            return None;
        }
        (wide || tall).then_some((wide, tall))
    }

    /// The element under a point on the canvas, leaving out `skip` and what's inside it.
    fn box_at(&self, w: Pt, skip: Option<&dom::Path>) -> Option<(String, dom::Path, dom::Box)> {
        let (id, title) = self.screen_at(w)?;
        if title {
            return None;
        }
        let s = self.canvas.screen(&id)?;
        let shot = self.shots.get(&id)?;
        let b = dom::under(&shot.boxes, w.0 - s.x, w.1 - s.y, skip)?;
        Some((id, b.path.clone(), b.rect.clone()))
    }

    /// Where the picked element would land if dropped at `w`: beside the element under the
    /// pointer, before or after it along the way its parent lays out.
    fn drop_at(&self, w: Pt) -> Option<Drop> {
        let el = self.element.as_ref()?;
        let (id, mut path, mut r) = self.box_at(w, Some(&el.path))?;
        if Some(&id) != self.screen.as_ref() || path.is_empty() {
            return None;
        }
        let s = self.canvas.screen(&id)?;
        let shot = self.shots.get(&id)?;
        // Over something inside one of its siblings, it lands beside that sibling, so cards
        // in a grid reorder rather than nest in each other.
        let home = dom::parent(&el.path)?;
        if path.len() > el.path.len() && path.starts_with(&home) {
            path.truncate(el.path.len());
            r = shot.boxes.iter().find(|b| b.path == path)?.rect.clone();
        }
        let parent = dom::parent(&path)?;
        let row = shot
            .boxes
            .iter()
            .find(|b| b.path == parent)
            .is_some_and(|b| b.row);
        let (x, y) = (s.x + r.x, s.y + r.y);
        Some(if row {
            let after = w.0 > x + r.w / 2.;
            let lx = if after { x + r.w } else { x };
            Drop {
                target: path,
                after,
                line: ((lx, y), (lx, y + r.h)),
            }
        } else {
            let after = w.1 > y + r.h / 2.;
            let ly = if after { y + r.h } else { y };
            Drop {
                target: path,
                after,
                line: ((x, ly), (x + r.w, ly)),
            }
        })
    }

    fn link_handle(&self) -> Option<Pt> {
        let el = self.element.as_ref()?;
        let s = self.canvas.screen(self.screen.as_ref()?)?;
        Some((
            s.x + el.rect.x + el.rect.w,
            s.y + el.rect.y + el.rect.h / 2.,
        ))
    }

    // ---- Drawing

    fn load_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(el) = &self.element {
            let key = (self.screen.clone().unwrap_or_default(), el.path.clone());
            if self.loaded.as_ref() != Some(&key) {
                let text = el.text.clone().unwrap_or_default();
                self.f_text
                    .update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                self.f_inline
                    .update(cx, |s, cx| s.set_value(text, window, cx));
                self.loaded = Some(key);
            }
        } else {
            self.loaded = None;
        }
        if self.loaded_screen != self.screen {
            let name = self
                .screen
                .as_ref()
                .and_then(|id| self.canvas.screen(id))
                .map(|s| s.name.clone())
                .unwrap_or_default();
            self.f_name
                .update(cx, |s, cx| s.set_value(name, window, cx));
            self.loaded_screen = self.screen.clone();
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let sep = || div().w(px(1.)).h(px(18.)).mx_1().bg(rgb(c_border()));
        div()
            .h(px(44.))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_canvas()))
            .child(
                text_btn("design-new", "New screen", Some(IconName::Plus), true).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.new_open = !this.new_open;
                        cx.notify();
                    }),
                ),
            )
            .child(div().flex_1())
            .child(
                icon_btn("design-undo", IconName::Undo2, "Undo (⌘Z)", false)
                    .when(self.undo.is_empty(), |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
            )
            .child(
                icon_btn("design-redo", IconName::Redo2, "Redo (⇧⌘Z)", false)
                    .when(self.redo.is_empty(), |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
            )
            .child(sep())
            .child(
                icon_btn(
                    "design-arrange",
                    IconName::Workflow,
                    "Arrange along the flow",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.arrange(cx))),
            )
            .child(
                icon_btn(
                    "design-dark",
                    if self.dark {
                        IconName::Moon
                    } else {
                        IconName::Sun
                    },
                    "Light or dark",
                    self.dark,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let d = !this.dark;
                    this.set_dark(d, cx)
                })),
            )
            .child(
                icon_btn(
                    "design-hotspots",
                    IconName::MousePointerClick,
                    "Show links between screens",
                    self.hotspots,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.hotspots = !this.hotspots;
                    cx.notify();
                })),
            )
            .child(sep())
            .child(
                icon_btn("design-zoom-out", IconName::Minus, "Zoom out", false).on_click(
                    cx.listener(|this, _, _, cx| {
                        let (w, h) = this.view_size();
                        this.zoom_at((w / 2., h / 2.), 1. / 1.25);
                        cx.notify();
                    }),
                ),
            )
            .child(
                div()
                    .id("design-zoom")
                    .w(px(46.))
                    .text_center()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(format!("{}%", (self.zoom * 100.).round()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fit();
                        cx.notify();
                    })),
            )
            .child(
                icon_btn("design-zoom-in", IconName::Plus, "Zoom in", false).on_click(cx.listener(
                    |this, _, _, cx| {
                        let (w, h) = this.view_size();
                        this.zoom_at((w / 2., h / 2.), 1.25);
                        cx.notify();
                    },
                )),
            )
            .child(sep())
            .child(
                text_btn("design-play", "Play", Some(IconName::Play), false)
                    .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
            )
            .into_any_element()
    }

    /// The menu of screens to start from.
    fn new_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().v_flex();
        for (i, (kind, sub)) in TEMPLATES.iter().enumerate() {
            let kind: &'static str = kind;
            list = list.child(
                div()
                    .id(("design-tpl", i))
                    .px_3()
                    .py(px(6.))
                    .v_flex()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_text()))
                            .child(kind),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(*sub),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.add_screen(kind, cx))),
            );
        }
        let mut devices = div().px_2().py_1().flex().gap_1();
        for (i, d) in Device::ALL.iter().enumerate() {
            let d = *d;
            devices = devices.child(
                chip_btn(("design-new-dev", i), d.label(), self.new_device == d).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.new_device = d;
                        cx.notify();
                    }),
                ),
            );
        }
        let describe = div()
            .id("design-describe")
            .mx_1()
            .mb_1()
            .px_2()
            .py(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .border_1()
            .border_color(rgb(c_border()))
            .hover(|el| el.bg(tint(c_subtle())))
            .child(icon(IconName::Sparkles, 14., c_blue()))
            .child(
                div()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_text()))
                            .child("Describe it to AI"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child("Say what the screen is for; AI designs it"),
                    ),
            )
            .on_click(cx.listener(|this, _, window, cx| this.start_describe(window, cx)));
        div()
            .absolute()
            .top(px(6.))
            .left(px(8.))
            .w(px(260.))
            .p_1()
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(float(c_canvas()))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(devices)
            .child(describe)
            .child(list)
            .into_any_element()
    }

    /// The artboards, with their names, the selection and the inline text field.
    fn artboards(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let z = self.zoom;
        let (vw, vh) = self.view_size();
        for (i, s) in self.canvas.screens.iter().enumerate() {
            let (w, h) = self.size_of(s);
            let (x, y) = self.screen_pt((s.x, s.y));
            if x > vw || y > vh || x + w * z < 0. || y + h * z < -TITLE_H {
                continue;
            }
            let on = self.screen.as_deref() == Some(s.id.as_str());
            out.push(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y - TITLE_H + 4.))
                    .w(px((w * z).max(60.)))
                    .h(px(TITLE_H - 6.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .text_size(px(12.))
                    .text_color(rgb(if on { c_blue() } else { c_secondary() }))
                    .child(div().truncate().child(s.name.clone()))
                    .child(
                        div()
                            .text_color(rgb(c_muted()))
                            .text_size(px(11.))
                            .child(s.device.label()),
                    )
                    .into_any_element(),
            );
            let radius = if s.device == Device::Phone { 28. } else { 10. } * z;
            let body = div()
                .id(("design-board", i))
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w * z))
                .h(px(h * z))
                .rounded(px(radius))
                .overflow_hidden()
                .bg(rgb(if self.dark { 0x0b0d12 } else { 0xffffff }))
                .shadow_lg()
                .border_1()
                .border_color(rgb(if on { c_blue() } else { c_border() }));
            let body = match self.shots.get(&s.id) {
                Some(shot) => body.child(
                    img(ImageSource::Render(shot.image.clone()))
                        .size_full()
                        .rounded(px(radius)),
                ),
                None if self.error.is_some() => body.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(rgb(c_muted()))
                        .child("Couldn't draw this screen"),
                ),
                None => body.child(askbar::skeleton(("design-drawing", i), z)),
            };
            out.push(body.into_any_element());
        }
        // What opens another screen, outlined where it sits.
        if self.hotspots {
            for s in &self.canvas.screens {
                let Some(shot) = self.shots.get(&s.id) else {
                    continue;
                };
                for l in &shot.links {
                    let (x, y) = self.screen_pt((s.x + l.rect.x, s.y + l.rect.y));
                    out.push(
                        div()
                            .absolute()
                            .left(px(x - 1.))
                            .top(px(y - 1.))
                            .w(px(l.rect.w * z + 2.))
                            .h(px(l.rect.h * z + 2.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(col(c_blue(), 0.85))
                            .bg(col(c_blue(), 0.1))
                            .into_any_element(),
                    );
                }
            }
        }
        // What the checks found, marked on the screens.
        if self.panel == Panel::Checks {
            for (id, _, found) in self.findings() {
                let Some(s) = self.canvas.screen(&id) else {
                    continue;
                };
                for f in found {
                    let Some(r) = f.rect else {
                        continue;
                    };
                    let (x, y) = self.screen_pt((s.x + r.x, s.y + r.y));
                    // Kept to the screen: what pokes out of it is marked to its edge.
                    let sw = self.size_of(s).0;
                    let r = dom::Box {
                        w: r.w.min(sw - r.x).max(4.),
                        ..r
                    };
                    let color = if f.kind.serious() { c_red() } else { c_amber() };
                    out.push(
                        div()
                            .absolute()
                            .left(px(x - 2.))
                            .top(px(y - 2.))
                            .w(px(r.w * z + 4.))
                            .h(px(r.h * z + 4.))
                            .rounded_sm()
                            .border_1()
                            .border_dashed()
                            .border_color(rgb(color))
                            .into_any_element(),
                    );
                }
            }
        }
        // What's under the pointer, outlined as it would be picked.
        if let (Some((id, path, r)), Drag::None) = (&self.hover, &self.drag) {
            let picked = self.element.as_ref().map(|e| &e.path) == Some(path)
                && self.screen.as_ref() == Some(id);
            if let (false, Some(s)) = (picked, self.canvas.screen(id)) {
                let (x, y) = self.screen_pt((s.x + r.x, s.y + r.y));
                out.push(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(r.w * z))
                        .h(px(r.h * z))
                        .border_1()
                        .border_color(col(c_blue(), 0.8))
                        .into_any_element(),
                );
            }
        }
        // The picked element.
        if let (Some(el), Some(s)) = (
            &self.element,
            self.screen.as_ref().and_then(|id| self.canvas.screen(id)),
        ) {
            let (x, y) = self.screen_pt((s.x + el.rect.x, s.y + el.rect.y));
            let (w, h) = (el.rect.w * z, el.rect.h * z);
            out.push(
                div()
                    .absolute()
                    .left(px(x - 1.))
                    .top(px(y - 1.))
                    .w(px(w + 2.))
                    .h(px(h + 2.))
                    .border_2()
                    .border_color(rgb(c_blue()))
                    .into_any_element(),
            );
            out.push(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y - 20.))
                    .px_1()
                    .rounded_sm()
                    .bg(rgb(c_blue()))
                    .text_size(px(10.5))
                    .text_color(rgb(0xffffff))
                    .child(format!(
                        "{}  {}×{}",
                        el.label,
                        el.rect.w.round(),
                        el.rect.h.round()
                    ))
                    .into_any_element(),
            );
            // The link handle: drag it onto another screen.
            out.push(
                div()
                    .absolute()
                    .left(px(x + w - 7.))
                    .top(px(y + h / 2. - 7.))
                    .size(px(14.))
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(0xffffff))
                    .bg(rgb(c_blue()))
                    .cursor_pointer()
                    .into_any_element(),
            );
            if !self.inline_open && self.ai.is_none() {
                out.push(
                    div()
                        .id("design-ask-here")
                        .absolute()
                        .left(px(x))
                        .top(px(y + h + 6.))
                        .h(px(22.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded_full()
                        .bg(rgb(c_blue()))
                        .shadow_md()
                        .text_size(px(11.))
                        .text_color(rgb(0xffffff))
                        .cursor_pointer()
                        .child(icon(IconName::Sparkles, 11., 0xffffff))
                        .child("Ask AI")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, window, cx| this.focus_ask(window, cx)))
                        .into_any_element(),
                );
            }
            if self.inline_open {
                out.push(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w.max(160.)))
                        .bg(float(c_canvas()))
                        .rounded_md()
                        .shadow_md()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(Input::new(&self.f_inline).h(px(h.clamp(28., 60.))))
                        .into_any_element(),
                );
            }
        }
        out.extend(self.editing_marks(cx));
        out
    }

    /// What editing on the canvas draws over the picked element: resize handles, the
    /// layout bar, where a dragged element would land, and spacing while Alt is held.
    fn editing_marks(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let (Some(el), Some((sid, r))) = (&self.element, self.picked_rect()) else {
            return out;
        };
        let z = self.zoom;
        let (x, y) = self.screen_pt((r.x, r.y));
        let (w, h) = (r.w * z, r.h * z);
        let red = 0xf24e6b;
        // Spacing to what's under the pointer, while Alt is held.
        if let (true, Some((hid, hpath, hr))) = (self.measure, &self.hover) {
            if *hid == sid && *hpath != el.path {
                let s = self.canvas.screen(&sid);
                let (ox, oy) = s.map(|s| (s.x, s.y)).unwrap_or_default();
                let (hx, hy) = self.screen_pt((ox + hr.x, oy + hr.y));
                out.push(
                    div()
                        .absolute()
                        .left(px(hx))
                        .top(px(hy))
                        .w(px(hr.w * z))
                        .h(px(hr.h * z))
                        .border_1()
                        .border_color(rgb(red))
                        .into_any_element(),
                );
                for (i, (a, b, d)) in dom::spacing(&el.rect, hr).into_iter().enumerate() {
                    let (ax, ay) = self.screen_pt((ox + a.0, oy + a.1));
                    let (bx, by) = self.screen_pt((ox + b.0, oy + b.1));
                    let across = (ay - by).abs() < 0.5;
                    out.push(
                        div()
                            .absolute()
                            .left(px(ax.min(bx)))
                            .top(px(ay.min(by)))
                            .w(px(if across { (bx - ax).abs() } else { 1. }))
                            .h(px(if across { 1. } else { (by - ay).abs() }))
                            .bg(rgb(red))
                            .into_any_element(),
                    );
                    out.push(
                        div()
                            .id(("design-measure", i))
                            .absolute()
                            .left(px((ax + bx) / 2. + if across { -12. } else { 4. }))
                            .top(px((ay + by) / 2. + if across { 3. } else { -8. }))
                            .px_1()
                            .rounded_sm()
                            .bg(rgb(red))
                            .text_size(px(10.5))
                            .text_color(rgb(0xffffff))
                            .child(format!("{}", d.round()))
                            .into_any_element(),
                    );
                }
            }
        }
        match &self.drag {
            // The element follows the pointer, and a line shows where it lands.
            Drag::Element {
                from,
                at,
                moved: true,
                ..
            } => {
                let (dx, dy) = ((at.0 - from.0) * z, (at.1 - from.1) * z);
                out.push(
                    div()
                        .absolute()
                        .left(px(x + dx))
                        .top(px(y + dy))
                        .w(px(w))
                        .h(px(h))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(c_blue()))
                        .bg(col(c_blue(), 0.08))
                        .into_any_element(),
                );
                if let Some(d) = &self.drop {
                    let (ax, ay) = self.screen_pt(d.line.0);
                    let (bx, by) = self.screen_pt(d.line.1);
                    let across = (ay - by).abs() < 0.5;
                    out.push(
                        div()
                            .absolute()
                            .left(px(ax.min(bx) - if across { 0. } else { 1. }))
                            .top(px(ay.min(by) - if across { 1. } else { 0. }))
                            .w(px(if across { (bx - ax).abs() } else { 3. }))
                            .h(px(if across { 3. } else { (by - ay).abs() }))
                            .rounded_full()
                            .bg(rgb(c_blue()))
                            .into_any_element(),
                    );
                }
            }
            // The new size, as it will be.
            Drag::Resize {
                wide,
                tall,
                from,
                at,
            } => {
                let nw = if *wide {
                    (r.w + at.0 - from.0).max(8.)
                } else {
                    r.w
                };
                let nh = if *tall {
                    (r.h + at.1 - from.1).max(8.)
                } else {
                    r.h
                };
                out.push(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(nw * z))
                        .h(px(nh * z))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(c_blue()))
                        .into_any_element(),
                );
                out.push(
                    div()
                        .absolute()
                        .left(px(x + nw * z + 6.))
                        .top(px(y + nh * z + 6.))
                        .px_1()
                        .rounded_sm()
                        .bg(rgb(c_blue()))
                        .text_size(px(10.5))
                        .text_color(rgb(0xffffff))
                        .child(format!("{} × {}", nw.round(), nh.round()))
                        .into_any_element(),
                );
            }
            _ => {}
        }
        if !matches!(self.drag, Drag::None) || self.inline_open {
            return out;
        }
        // Resize: drag the right or bottom edge, or the corner.
        let edge = |id: &'static str| div().id(id).absolute();
        out.push(
            edge("design-resize-right")
                .left(px(x + w - 1.))
                .top(px(y))
                .w(px(6.))
                .h(px(h * 0.4))
                .cursor_ew_resize()
                .into_any_element(),
        );
        out.push(
            edge("design-resize-bottom")
                .left(px(x))
                .top(px(y + h - 1.))
                .w(px(w))
                .h(px(6.))
                .cursor_ns_resize()
                .into_any_element(),
        );
        out.push(
            edge("design-resize-corner")
                .left(px(x + w - 4.))
                .top(px(y + h - 4.))
                .size(px(8.))
                .border_1()
                .border_color(rgb(c_blue()))
                .bg(rgb(0xffffff))
                .cursor_nwse_resize()
                .into_any_element(),
        );
        // The layout bar for a flex container: direction, gap and alignment in a click.
        if el.is_flex() && self.ai.is_none() {
            let row = !el.style("flex-direction").starts_with("column");
            let gap = dom_px(&el.style("gap")).unwrap_or(0.);
            let align = el.style("align-items");
            let next_align = match align.as_str() {
                "center" => "flex-end",
                "flex-end" | "end" => "stretch",
                "stretch" | "normal" | "" => "flex-start",
                _ => "center",
            };
            let align_word = match align.as_str() {
                "center" => "Center",
                "flex-end" | "end" => "End",
                "flex-start" | "start" => "Start",
                _ => "Stretch",
            };
            let chip = |id: &'static str| {
                div()
                    .id(id)
                    .h(px(22.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_active())))
            };
            out.push(
                div()
                    .id("design-layout-bar")
                    .absolute()
                    .left(px(x + 86.))
                    .top(px(y + h + 6.))
                    .h(px(26.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_raised()))
                    .shadow_md()
                    .text_size(px(11.))
                    .text_color(rgb(c_text()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        chip("design-layout-dir")
                            .child(icon(
                                if row {
                                    IconName::ArrowRight
                                } else {
                                    IconName::ArrowDown
                                },
                                12.,
                                c_text(),
                            ))
                            .child(if row { "Row" } else { "Column" })
                            .tooltip(tip("Lay out across or down"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_style(
                                    "flex-direction",
                                    if row { "column" } else { "row" },
                                    cx,
                                )
                            })),
                    )
                    .child(div().w(px(1.)).h(px(14.)).bg(rgb(c_border())))
                    .child(
                        chip("design-layout-gap-less")
                            .child(icon(IconName::Minus, 11., c_secondary()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_style("gap", &format!("{}px", (gap - 4.).max(0.)), cx)
                            })),
                    )
                    .child(
                        div()
                            .min_w(px(44.))
                            .text_center()
                            .text_color(rgb(c_secondary()))
                            .child(format!("Gap {}", gap.round())),
                    )
                    .child(
                        chip("design-layout-gap-more")
                            .child(icon(IconName::Plus, 11., c_secondary()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_style("gap", &format!("{}px", gap + 4.), cx)
                            })),
                    )
                    .child(div().w(px(1.)).h(px(14.)).bg(rgb(c_border())))
                    .child(
                        chip("design-layout-align")
                            .child(format!("Align {align_word}"))
                            .tooltip(tip(
                                "Line children up at the start, centre, end, or stretch them",
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_style("align-items", next_align, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    /// Arrows for the links between screens, and the one being dragged.
    fn paint_links(&self) -> (Vec<Arrow>, Option<Arrow>) {
        let mut out = Vec::new();
        for s in self.canvas.screens.iter().filter(|_| self.hotspots) {
            let Some(shot) = self.shots.get(&s.id) else {
                continue;
            };
            for l in &shot.links {
                let Some(t) = self.canvas.screen(&l.goto) else {
                    continue;
                };
                let from = (s.x + l.rect.x + l.rect.w, s.y + l.rect.y + l.rect.h / 2.);
                let to = if t.x >= s.x {
                    (t.x, t.y + 40.)
                } else {
                    (t.x + self.size_of(t).0, t.y + 40.)
                };
                out.push((self.screen_pt(from), self.screen_pt(to)));
            }
        }
        let dragging = match &self.drag {
            Drag::Link { from, to } => Some((self.screen_pt(*from), self.screen_pt(*to))),
            _ => None,
        };
        (out, dragging)
    }

    fn player(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.playing.clone()?;
        let s = self.canvas.screen(&id)?;
        let shot = self.shots.get(&id);
        let (vw, vh) = self.view_size();
        let (w, h) = s.device.size();
        let scale = ((vh - 80.) / h).min((vw - 80.) / w).min(1.);
        let (dw, dh) = (w * scale, h * scale);
        let mut frame = div()
            .relative()
            .w(px(dw))
            .h(px(dh))
            .rounded(px(if s.device == Device::Phone { 28. } else { 10. } * scale))
            .overflow_hidden()
            .bg(rgb(0xffffff))
            .shadow_lg();
        if let Some(shot) = shot {
            // Only the first screenful plays; the rest of a tall screen is cut off.
            let full_h = shot.size.1 * scale;
            frame = frame.child(
                img(ImageSource::Render(shot.image.clone()))
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(dw))
                    .h(px(full_h)),
            );
            for (i, l) in shot.links.iter().enumerate() {
                let to = l.goto.clone();
                frame = frame.child(
                    div()
                        .id(("design-hot", i))
                        .absolute()
                        .left(px(l.rect.x * scale))
                        .top(px(l.rect.y * scale))
                        .w(px(l.rect.w * scale))
                        .h(px(l.rect.h * scale))
                        .cursor_pointer()
                        .rounded_md()
                        .when(self.hotspots, |el| {
                            el.border_1()
                                .border_color(col(c_blue(), 0.7))
                                .bg(col(c_blue(), 0.1))
                        })
                        .hover(|el| el.bg(col(c_blue(), 0.2)))
                        .on_click(cx.listener(move |this, _, _, cx| this.play_go(&to, cx))),
                );
            }
        }
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .v_flex()
                .items_center()
                .bg(rgb(c_canvas()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(40.))
                        .w_full()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            icon_btn("design-play-back", IconName::ArrowLeft, "Back", false)
                                .when(self.play_back.is_empty(), |el| el.opacity(0.4))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(prev) = this.play_back.pop() {
                                        this.playing = Some(prev);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .text_color(rgb(c_text()))
                                .child(format!(
                                    "{}  ·  click the linked parts to move through the design",
                                    s.name
                                )),
                        )
                        .child(
                            text_btn(
                                "design-play-hotspots",
                                if self.hotspots {
                                    "Hide hotspots"
                                } else {
                                    "Show hotspots"
                                },
                                Some(IconName::MousePointerClick),
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.hotspots = !this.hotspots;
                                cx.notify();
                            })),
                        )
                        .child(
                            text_btn("design-play-stop", "Stop", Some(IconName::X), false)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.playing = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().flex_1().flex().items_center().child(frame))
                .into_any_element(),
        )
    }

    fn empty_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.canvas.screens.is_empty() || self.thinking_now() {
            return None;
        }
        let mut row = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap_2()
            .max_w(px(560.));
        for (i, (kind, _)) in TEMPLATES.iter().enumerate() {
            let kind: &'static str = kind;
            row = row.child(
                text_btn(("design-start", i), kind, None, i == 0)
                    .on_click(cx.listener(move |this, _, _, cx| this.add_screen(kind, cx))),
            );
        }
        let mut devices = div().flex().gap_1();
        for (i, d) in Device::ALL.iter().enumerate() {
            let d = *d;
            devices = devices.child(
                chip_btn(("design-start-dev", i), d.label(), self.new_device == d).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.new_device = d;
                        cx.notify();
                    }),
                ),
            );
        }
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .v_flex()
                        .items_center()
                        .gap_3()
                        .p_6()
                        .rounded_xl()
                        .bg(float(c_canvas()))
                        .border_1()
                        .border_color(rgb(c_border()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(c_text()))
                                .child("Design your first screen"),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .text_center()
                                .max_w(px(440.))
                                .child(format!(
                                    "Screens are HTML files in {FOLDER}/ in the repository, styled by one theme, so what you design here is what gets built."
                                )),
                        )
                        .child(devices)
                        .child(row)
                        .child(
                            text_btn(
                                "design-start-describe",
                                "Or describe it to AI",
                                Some(IconName::Sparkles),
                                false,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_describe(window, cx)
                            })),
                        ),
                )
                .into_any_element(),
        )
    }

    fn canvas_area(&self, cx: &mut Context<Self>) -> AnyElement {
        let bounds = self.bounds.clone();
        let (links, dragging) = self.paint_links();
        let notice = self
            .notice
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(6))
            .map(|(t, _)| t.clone());
        let status = format!(
            "{} screen{} · {FOLDER}/{}",
            self.canvas.screens.len(),
            if self.canvas.screens.len() == 1 {
                ""
            } else {
                "s"
            },
            if self.drawing.is_empty() {
                String::new()
            } else {
                " · drawing…".into()
            }
        );
        let mut area = div()
            .id("design-canvas")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(c_subtle()))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(|this, e: &PinchEvent, _, cx| {
                let local = this.local(e.position);
                this.zoom_at(local, 1. + e.delta);
                cx.notify();
            }))
            .child(
                canvas(
                    move |b, window, _| {
                        let old = bounds.get();
                        bounds.set(b);
                        if old.size != b.size {
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children(self.artboards(cx))
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        let o = (f32::from(b.origin.x), f32::from(b.origin.y));
                        let tf = |p: Pt| point(px(o.0 + p.0), px(o.1 + p.1));
                        for (a, z) in &links {
                            paint_arrow(*a, *z, &tf, col(c_blue(), 0.55), window);
                        }
                        if let Some((a, z)) = dragging {
                            paint_arrow(a, z, &tf, col(c_blue(), 1.), window);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children(self.empty_state(cx))
            .children(self.thinking());
        if let Some(e) = &self.error {
            area = area.child(
                div()
                    .absolute()
                    .top(px(10.))
                    .left(px(12.))
                    .right(px(12.))
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_red()))
                    .bg(float(c_canvas()))
                    .text_size(px(12.5))
                    .text_color(rgb(c_red()))
                    .child(e.clone()),
            );
        }
        if let Some(bar) = self.ask_bar(cx) {
            area = area.child(bar);
        }
        if let Some(text) = notice {
            area = area.child(
                div()
                    .absolute()
                    .bottom(px(56.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .bg(float(c_canvas()))
                            .shadow_md()
                            .text_size(px(12.5))
                            .text_color(rgb(c_text()))
                            .child(text),
                    ),
            );
        }
        if self.new_open {
            area = area.child(self.new_menu(cx));
        }
        area = area.children(self.player(cx));
        // The status sits in a strip under the canvas, so nothing drawn over the canvas covers it.
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(area)
            .child(
                div()
                    .flex_none()
                    .h(px(26.))
                    .px_3()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .bg(tint(c_canvas()))
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(div().min_w_0().truncate().child(status)),
            )
            .into_any_element()
    }

    // ---- The side panel

    fn side_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = |id: &'static str, label: SharedString, p: Panel, cx: &mut Context<Self>| {
            let on = self.panel == p;
            div()
                .id(id)
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |el| {
                    el.bg(tint(c_selected())).font_weight(FontWeight::MEDIUM)
                })
                .text_color(rgb(if on { c_text() } else { c_secondary() }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.panel = p;
                    cx.notify();
                }))
                .child(label)
        };
        let body = match self.panel {
            Panel::Inspect => self.inspector(cx),
            Panel::Layers => self.layers_panel(cx),
            Panel::Theme => self.theme_panel(cx),
            Panel::Checks => self.checks_panel(cx),
        };
        let issues: usize = self.findings().iter().map(|f| f.2.len()).sum();
        div()
            .w(px(290.))
            .flex_none()
            .h_full()
            .v_flex()
            .border_l_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_canvas()))
            .child(
                div()
                    .h(px(44.))
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .child(tab(
                        "design-tab-inspect",
                        "Inspect".into(),
                        Panel::Inspect,
                        cx,
                    ))
                    .child(tab("design-tab-layers", "Layers".into(), Panel::Layers, cx))
                    .child(tab("design-tab-theme", "Theme".into(), Panel::Theme, cx))
                    .child(tab(
                        "design-tab-checks",
                        if issues == 0 {
                            "Checks".into()
                        } else {
                            format!("Checks {issues}").into()
                        },
                        Panel::Checks,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("design-side")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .pb_4()
                    .child(body),
            )
            .into_any_element()
    }

    /// Swatches for a colour property: the theme's tokens, then none.
    fn color_row(&self, key: &'static str, el: &Element, cx: &mut Context<Self>) -> Div {
        let now = el.style(key);
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, t) in COLOR_TOKENS.iter().enumerate() {
            let t: &'static str = t;
            let value = format!("var({t})");
            let on = now.trim() == value;
            let c = self.token_color(t).unwrap_or(0x888888);
            row = row.child(
                div()
                    .id((key, i))
                    .size(px(22.))
                    .rounded_md()
                    .border_2()
                    .border_color(rgb(if on { c_blue() } else { c_border() }))
                    .bg(rgb(c))
                    .cursor_pointer()
                    .tooltip(crate::archdraw::view::tip(token_name(t)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_style(key, &value, cx))),
            );
        }
        row.child(
            chip_btn(
                SharedString::from(format!("{key}-none")),
                "None",
                !el.inline.contains_key(key),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_style(key, "", cx))),
        )
    }

    /// Chips that set a style to one of a few values.
    fn choice_row(
        &self,
        key: &'static str,
        el: &Element,
        options: &[(&'static str, &'static str)],
        cx: &mut Context<Self>,
    ) -> Div {
        let now = el.style(key);
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, value)) in options.iter().enumerate() {
            let value: &'static str = value;
            let on = now.trim() == value || (value.is_empty() && !el.inline.contains_key(key));
            row = row.child(
                chip_btn((key, i), *label, on)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_style(key, value, cx))),
            );
        }
        row
    }

    fn inspector(&self, cx: &mut Context<Self>) -> Div {
        let out = div().v_flex().gap_1();
        let Some(s) = self.screen.as_ref().and_then(|id| self.canvas.screen(id)) else {
            return out
                .child(section("Canvas"))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child("Click a screen to pick what's on it: text, buttons, cards. Drag a screen by its name. Drag the blue dot on a picked element onto another screen to link them, then press Play."),
                )
                .child(section("Add a screen"))
                .child({
                    let mut col_ = div().v_flex().gap_1();
                    for (i, (kind, _)) in TEMPLATES.iter().enumerate() {
                        let kind: &'static str = kind;
                        col_ = col_.child(
                            text_btn(("design-side-tpl", i), kind, None, false)
                                .on_click(cx.listener(move |this, _, _, cx| this.add_screen(kind, cx))),
                        );
                    }
                    col_
                });
        };
        let mut out = out;
        match &self.element {
            None => {
                let mut devices = div().flex().gap_1();
                for (i, d) in Device::ALL.iter().enumerate() {
                    let d = *d;
                    devices = devices.child(
                        chip_btn(("design-dev", i), d.label(), s.device == d)
                            .on_click(cx.listener(move |this, _, _, cx| this.set_device(d, cx))),
                    );
                }
                out = out
                    .child(section("Screen"))
                    .child(label_row("Name", Input::new(&self.f_name)))
                    .child(label_row("Device", devices))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("{FOLDER}/screens/{}.html", s.id)),
                    )
                    .child(
                        div()
                            .pt_2()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(
                                text_btn(
                                    "design-dup-screen",
                                    "Duplicate",
                                    Some(IconName::Copy),
                                    false,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.duplicate_screen(cx))),
                            )
                            .child(
                                text_btn(
                                    "design-variants",
                                    "Variants with AI",
                                    Some(IconName::Sparkles),
                                    false,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.variants_with_ai(window, cx),
                                )),
                            )
                            .when(
                                variant_of(&s.id).is_some_and(|b| self.canvas.screen(&b).is_some()),
                                |el| {
                                    let id = s.id.clone();
                                    el.child(
                                        text_btn(
                                            "design-use-variant",
                                            "Use this variant",
                                            Some(IconName::Check),
                                            true,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.use_variant(&id, cx)
                                            }),
                                        ),
                                    )
                                },
                            )
                            .child(
                                text_btn(
                                    "design-play-here",
                                    "Play from here",
                                    Some(IconName::Play),
                                    false,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                            )
                            .child(
                                text_btn(
                                    "design-del-screen",
                                    "Delete",
                                    Some(IconName::Trash),
                                    false,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.delete_screen(cx))),
                            ),
                    )
                    .child(section("Insert"))
                    .child(self.blocks(cx));
            }
            Some(el) => {
                out = out.child(section("Element")).child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(rgb(c_text()))
                                .child(el.label.clone()),
                        )
                        .child(
                            icon_btn(
                                "design-parent",
                                IconName::ArrowUpLeft,
                                "Select its parent (Esc)",
                                false,
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.escape(window, cx))),
                        ),
                );
                if el.text.is_some() {
                    out = out.child(label_row("Text", Input::new(&self.f_text)));
                }
                out = out
                    .child(label_row("Text colour", self.color_row("color", el, cx)))
                    .child(label_row(
                        "Fill",
                        self.color_row("background-color", el, cx),
                    ))
                    .child(label_row(
                        "Size",
                        self.choice_row(
                            "font-size",
                            el,
                            &[
                                ("12", "12px"),
                                ("14", "14px"),
                                ("16", "16px"),
                                ("20", "20px"),
                                ("24", "24px"),
                                ("30", "30px"),
                                ("Auto", ""),
                            ],
                            cx,
                        ),
                    ))
                    .child(label_row(
                        "Weight",
                        self.choice_row(
                            "font-weight",
                            el,
                            &[
                                ("Regular", "400"),
                                ("Medium", "500"),
                                ("Semibold", "600"),
                                ("Bold", "700"),
                                ("Auto", ""),
                            ],
                            cx,
                        ),
                    ))
                    .child(label_row(
                        "Padding",
                        self.choice_row(
                            "padding",
                            el,
                            &[
                                ("0", "0"),
                                ("1×", "var(--space)"),
                                ("2×", "calc(var(--space) * 2)"),
                                ("3×", "calc(var(--space) * 3)"),
                                ("Auto", ""),
                            ],
                            cx,
                        ),
                    ))
                    .child(label_row(
                        "Corners",
                        self.choice_row(
                            "border-radius",
                            el,
                            &[
                                ("Square", "0"),
                                ("Theme", "var(--radius)"),
                                ("Round", "999px"),
                                ("Auto", ""),
                            ],
                            cx,
                        ),
                    ))
                    .child(label_row(
                        "Width",
                        self.choice_row(
                            "width",
                            el,
                            &[("Hug", "fit-content"), ("Fill", "100%"), ("Auto", "")],
                            cx,
                        ),
                    ))
                    .child(label_row(
                        "Align text",
                        self.choice_row(
                            "text-align",
                            el,
                            &[
                                ("Left", "left"),
                                ("Center", "center"),
                                ("Right", "right"),
                                ("Auto", ""),
                            ],
                            cx,
                        ),
                    ));
                if el.is_flex() {
                    out = out
                        .child(section("Layout"))
                        .child(label_row(
                            "Direction",
                            self.choice_row(
                                "flex-direction",
                                el,
                                &[("Down", "column"), ("Across", "row"), ("Auto", "")],
                                cx,
                            ),
                        ))
                        .child(label_row(
                            "Gap",
                            self.choice_row(
                                "gap",
                                el,
                                &[
                                    ("0", "0"),
                                    ("1×", "var(--space)"),
                                    ("2×", "calc(var(--space) * 2)"),
                                    ("3×", "calc(var(--space) * 3)"),
                                    ("Auto", ""),
                                ],
                                cx,
                            ),
                        ))
                        .child(label_row(
                            "Align",
                            self.choice_row(
                                "align-items",
                                el,
                                &[
                                    ("Start", "flex-start"),
                                    ("Center", "center"),
                                    ("End", "flex-end"),
                                    ("Stretch", "stretch"),
                                    ("Auto", ""),
                                ],
                                cx,
                            ),
                        ))
                        .child(label_row(
                            "Spread",
                            self.choice_row(
                                "justify-content",
                                el,
                                &[
                                    ("Start", "flex-start"),
                                    ("Center", "center"),
                                    ("End", "flex-end"),
                                    ("Between", "space-between"),
                                    ("Auto", ""),
                                ],
                                cx,
                            ),
                        ));
                }
                // Where clicking it goes.
                let mut opens = div().flex().flex_wrap().gap(px(4.));
                opens = opens.child(
                    chip_btn(("design-goto", 0usize), "Nowhere", el.goto.is_none()).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.op("attr", &[json!("data-goto"), json!("")], After::Same, cx)
                        }),
                    ),
                );
                for (i, t) in self.canvas.screens.iter().enumerate() {
                    if Some(&t.id) == self.screen.as_ref() {
                        continue;
                    }
                    let id = t.id.clone();
                    opens = opens.child(
                        chip_btn(
                            ("design-goto", i + 1),
                            t.name.clone(),
                            el.goto.as_deref() == Some(t.id.as_str()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.op("attr", &[json!("data-goto"), json!(id)], After::Same, cx)
                        })),
                    );
                }
                out = out
                    .child(section("Prototype"))
                    .child(label_row("Opens", opens))
                    .child(section("Arrange"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(
                                icon_btn("design-up", IconName::ArrowUp, "Move up (⌥↑)", false)
                                    .on_click(cx.listener(|this, _, _, cx| this.nudge(-1, cx))),
                            )
                            .child(
                                icon_btn(
                                    "design-down",
                                    IconName::ArrowDown,
                                    "Move down (⌥↓)",
                                    false,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.nudge(1, cx))),
                            )
                            .child(
                                icon_btn("design-dup", IconName::Copy, "Duplicate (⌘D)", false)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.duplicate_element(cx)),
                                    ),
                            )
                            .child(
                                icon_btn(
                                    "design-hide",
                                    IconName::EyeOff,
                                    "Hide or show",
                                    el.hidden,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.op("hide", &[], After::Same, cx),
                                )),
                            )
                            .child(
                                icon_btn("design-del", IconName::Trash, "Delete (⌫)", false)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.delete_element(cx)),
                                    ),
                            ),
                    )
                    .child(section(if el.is_container() {
                        "Insert inside"
                    } else {
                        "Insert after"
                    }))
                    .child(self.blocks(cx));
                if !el.inline.is_empty() {
                    let mut css: Vec<String> = el
                        .inline
                        .iter()
                        .map(|(k, v)| format!("{k}: {v};"))
                        .collect();
                    css.sort();
                    out = out.child(section("Its own CSS")).child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(tint(c_subtle()))
                            .font_family("monospace")
                            .text_size(px(11.5))
                            .text_color(rgb(c_secondary()))
                            .child(css.join("\n")),
                    );
                }
            }
        }
        out
    }

    fn blocks(&self, cx: &mut Context<Self>) -> Div {
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, html)) in BLOCKS.iter().enumerate() {
            let html: &'static str = html;
            row = row.child(
                chip_btn(("design-block", i), *label, false)
                    .on_click(cx.listener(move |this, _, _, cx| this.insert(html, cx))),
            );
        }
        row
    }

    fn layers_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut out = div().v_flex().pt_2();
        if self.screen.is_none() {
            return out.child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child("Pick a screen to see what it's made of."),
            );
        }
        if self.layers.is_empty() {
            return out.child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child("Reading the screen…"),
            );
        }
        let picked = self.element.as_ref().map(|e| e.path.clone());
        for (i, l) in self.layers.iter().enumerate() {
            let on = picked.as_ref() == Some(&l.path);
            let (p1, p2, p3) = (l.path.clone(), l.path.clone(), l.path.clone());
            let dragged = DraggedLayer {
                path: l.path.clone(),
                label: l.label.clone().into(),
            };
            out = out.child(
                div()
                    .id(("design-layer", i))
                    .h(px(24.))
                    .pl(px(6. + l.depth as f32 * 12.))
                    .pr_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .when(on, |el| el.bg(tint(c_selected())))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .text_color(rgb(if l.hidden { c_muted() } else { c_text() }))
                    .on_drag(dragged, |d, _, _, cx| cx.new(|_| d.clone()))
                    .drag_over::<DraggedLayer>(|el, _, _, _| {
                        el.border_t_2().border_color(rgb(c_blue()))
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedLayer, _, cx| {
                        let target = p3.clone();
                        if d.path != target {
                            let js = dom::op(&d.path, "moveTo", &[json!(target), json!(false)]);
                            this.edit(js, After::Clear, cx);
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let id = this.screen.clone().unwrap_or_default();
                        this.select_path(&id, p1.clone(), cx);
                    }))
                    .child(div().flex_1().truncate().child(l.label.clone()))
                    .when_some(l.text.clone(), |el, t| {
                        el.child(
                            div()
                                .max_w(px(90.))
                                .truncate()
                                .text_color(rgb(c_muted()))
                                .child(t),
                        )
                    })
                    .child(
                        icon_btn(
                            ("design-layer-eye", i),
                            if l.hidden {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            },
                            "Hide or show",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.edit(dom::op(&p2, "hide", &[]), After::Same, cx);
                        })),
                    ),
            );
        }
        out.child(
            div()
                .pt_2()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child("Drag a layer onto another to move it there."),
        )
    }

    /// What the design checks found on each screen.
    fn checks_panel(&self, cx: &mut Context<Self>) -> Div {
        let all = self.findings();
        let total: usize = all.iter().map(|f| f.2.len()).sum();
        let mut out = div().v_flex().gap_1().pt_2();
        out = out.child(
            div()
                .text_size(px(12.5))
                .text_color(rgb(c_secondary()))
                .child(if self.canvas.screens.is_empty() {
                    "Add a screen, and Runo checks it for text that's hard to read, targets too small to tap, colours outside the theme and more.".to_string()
                } else if total == 0 {
                    "Nothing to fix: text is readable, targets are big enough, colours come from the theme and every link has a screen.".to_string()
                } else {
                    format!(
                        "{total} thing{} to look at. Click one to pick it on its screen.",
                        if total == 1 { "" } else { "s" }
                    )
                }),
        );
        if total > 0 {
            out = out.child(
                div().pt_1().child(
                    text_btn(
                        "design-fix-all",
                        "Fix all with AI",
                        Some(IconName::Sparkles),
                        true,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.fix_checks(None, window, cx))),
                ),
            );
        }
        let mut n: usize = 0;
        for (id, name, found) in all {
            if found.is_empty() {
                continue;
            }
            let sid = id.clone();
            out = out.child(
                div()
                    .pt_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
                            .child(format!("{} · {}", name.to_uppercase(), found.len())),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("design-fix-{id}")))
                            .text_size(px(11.5))
                            .text_color(rgb(c_blue()))
                            .cursor_pointer()
                            .child("Fix with AI")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.fix_checks(Some(sid.clone()), window, cx)
                            })),
                    ),
            );
            for f in found {
                n += 1;
                let (id, path) = (id.clone(), f.path.clone());
                let color = if f.kind.serious() { c_red() } else { c_amber() };
                out = out.child(
                    div()
                        .id(("design-finding", n))
                        .px_2()
                        .py(px(5.))
                        .flex()
                        .items_start()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .child(
                            div()
                                .pt(px(5.))
                                .child(div().size(px(6.)).rounded_full().bg(rgb(color))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .text_color(rgb(c_text()))
                                .child(f.text.clone()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_finding(&id, path.clone(), cx)
                        })),
                );
            }
        }
        out
    }

    fn theme_panel(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme();
        let mut swatches = div().flex().flex_wrap().gap(px(5.));
        for (i, c) in PRIMARIES.iter().enumerate() {
            let c = *c;
            swatches = swatches.child(
                div()
                    .id(("design-primary", i))
                    .size(px(26.))
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(if t.primary == c { c_text() } else { c_border() }))
                    .bg(rgb(c))
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_theme(|t| t.primary = c, cx)),
                    ),
            );
        }
        let mut fonts = div().flex().flex_wrap().gap(px(4.));
        for (i, (name, _)) in FONTS.iter().enumerate() {
            let name: &'static str = name;
            fonts = fonts.child(chip_btn(("design-font", i), name, t.font == name).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.set_theme(|t| t.font = name.to_string(), cx)
                }),
            ));
        }
        let mut radius = div().flex().flex_wrap().gap(px(4.));
        for (i, r) in [0., 4., 8., 12., 16., 24.].iter().enumerate() {
            let r = *r;
            radius = radius.child(
                chip_btn(("design-radius", i), format!("{r}"), t.radius == r).on_click(
                    cx.listener(move |this, _, _, cx| this.set_theme(|t| t.radius = r, cx)),
                ),
            );
        }
        let mut density = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, s)) in [
            ("Compact", 6.),
            ("Regular", 8.),
            ("Roomy", 10.),
            ("Spacious", 12.),
        ]
        .iter()
        .enumerate()
        {
            let s = *s;
            density = density.child(
                chip_btn(("design-space", i), *label, t.space == s).on_click(
                    cx.listener(move |this, _, _, cx| this.set_theme(|t| t.space = s, cx)),
                ),
            );
        }
        let mode = div()
            .flex()
            .gap(px(4.))
            .child(
                chip_btn("design-light", "Light", !self.dark)
                    .on_click(cx.listener(|this, _, _, cx| this.set_dark(false, cx))),
            )
            .child(
                chip_btn("design-darkmode", "Dark", self.dark)
                    .on_click(cx.listener(|this, _, _, cx| this.set_dark(true, cx))),
            );
        div()
            .v_flex()
            .gap_1()
            .child(section("Theme"))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(format!(
                        "Changes {FOLDER}/{TOKENS}, so every screen restyles at once."
                    )),
            )
            .child(label_row("Primary colour", swatches))
            .child(label_row("Font", fonts))
            .child(label_row("Corner radius", radius))
            .child(label_row("Density", density))
            .child(label_row("Preview", mode))
    }

    fn render_inner(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.load_fields(window, cx);
        if !self.fitted && self.view_size().0 > 10. {
            if !self.canvas.screens.is_empty() {
                self.fit();
            }
            self.fitted = true;
        }
        if self.inline_open {
            let input = self.f_inline.clone();
            if !input.read(cx).focus_handle(cx).is_focused(window) {
                window.defer(cx, move |window, cx| {
                    input.update(cx, |s, cx| s.focus(window, cx))
                });
            }
        }
        div()
            .size_full()
            .flex()
            .key_context("DesignCanvas")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &DesignEscape, window, cx| this.escape(window, cx)))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_up))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .v_flex()
                    .child(self.toolbar(cx))
                    .child(self.canvas_area(cx)),
            )
            .child(self.side_panel(cx))
            .into_any_element()
    }
}

impl Render for DesignCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_inner(window, cx)
    }
}

/// A curved arrow from `a` to `b`, in canvas pixels.
fn paint_arrow(a: Pt, b: Pt, tf: &dyn Fn(Pt) -> Point<Pixels>, color: Rgba, window: &mut Window) {
    let dx = ((b.0 - a.0).abs() * 0.5).max(30.);
    let dir = if b.0 >= a.0 { 1. } else { -1. };
    let c1 = (a.0 + dx * dir, a.1);
    let c2 = (b.0 - dx * dir, b.1);
    let mut p = PathBuilder::stroke(px(2.));
    p.move_to(tf(a));
    p.cubic_bezier_to(tf(b), tf(c1), tf(c2));
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
    // The head, pointing along the last stretch of the curve.
    let (ux, uy) = {
        let (x, y) = (b.0 - c2.0, b.1 - c2.1);
        let l = (x * x + y * y).sqrt().max(0.001);
        (x / l, y / l)
    };
    let size = 9.;
    let left = (
        b.0 - ux * size - uy * size * 0.5,
        b.1 - uy * size + ux * size * 0.5,
    );
    let right = (
        b.0 - ux * size + uy * size * 0.5,
        b.1 - uy * size - ux * size * 0.5,
    );
    let mut h = PathBuilder::fill();
    h.move_to(tf(b));
    h.line_to(tf(left));
    h.line_to(tf(right));
    h.close();
    if let Ok(path) = h.build() {
        window.paint_path(path, color);
    }
}

/// A colour token as people say it.
fn token_name(t: &str) -> &'static str {
    match t {
        "--primary" => "Primary",
        "--on-primary" => "On primary",
        "--bg" => "Background",
        "--surface" => "Surface",
        "--text" => "Text",
        "--muted" => "Muted",
        "--border" => "Border",
        _ => "Colour",
    }
}

/// Every file under `dir`, to put back later.
/// Space between a screen and the row of its variants.
const VARIANT_GAP: f32 = 80.;

/// The screen a variant was made from: `home-v2` is a variant of `home`.
fn variant_of(id: &str) -> Option<String> {
    let (base, n) = id.rsplit_once("-v")?;
    (!base.is_empty() && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .then(|| base.to_string())
}

/// The task for AI to design a new screen from a description.
fn describe_task(what: &str, device: Device) -> String {
    format!(
        "Design a new {} screen: {what}\n\nAdd it as a new file in {FOLDER}/screens/ with a \
         short id, a <title> naming it and <meta name=\"runo:device\" content=\"{}\">. Make \
         it complete and realistic, with real-sounding content rather than placeholders, built \
         from the classes in {FOLDER}/components.css and the variables in {FOLDER}/tokens.css. \
         Link it to the screens it naturally leads to and from with data-goto where that fits.",
        device.label().to_lowercase(),
        device.label().to_lowercase()
    )
}

/// The task for AI to make variants of a screen.
fn variants_task(id: &str, name: &str) -> String {
    format!(
        "Make three alternative designs of the screen {name} ({FOLDER}/screens/{id}.html) as \
         new screens {FOLDER}/screens/{id}-v1.html, {id}-v2.html and {id}-v3.html. Keep its \
         purpose, content and links, but make each a clearly different take: a different \
         layout, hierarchy or density, not just other colours. Title each \"{name} · variant \
         N\". Don't change {id}.html itself; Runo shows the variants under it to pick from."
    )
}

/// A task cut to a line, for the ask bar.
fn summary(task: &str) -> String {
    let line = task.lines().next().unwrap_or_default();
    if line.chars().count() > 90 {
        format!("{}…", line.chars().take(89).collect::<String>())
    } else {
        line.to_string()
    }
}

fn snapshot(dir: &std::path::Path) -> HashMap<PathBuf, Vec<u8>> {
    fn go(dir: &std::path::Path, out: &mut HashMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                go(&p, out);
            } else if let Ok(bytes) = std::fs::read(&p) {
                out.insert(p, bytes);
            }
        }
    }
    let mut out = HashMap::new();
    go(dir, &mut out);
    out
}

/// Make `dir` hold exactly the files of a snapshot again.
fn restore(dir: &std::path::Path, before: &HashMap<PathBuf, Vec<u8>>) -> std::io::Result<()> {
    for p in snapshot(dir).into_keys() {
        if !before.contains_key(&p) {
            std::fs::remove_file(&p)?;
        }
    }
    for (p, bytes) in before {
        if std::fs::read(p).ok().as_ref() != Some(bytes) {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(p, bytes)?;
        }
    }
    Ok(())
}

/// What the canvas says while AI designs, by how long it has been working. The agent doesn't
/// report its steps, so this only names what it is usually doing by then.
fn design_line(secs: u64) -> &'static str {
    match secs {
        0..=3 => "Starting the agent",
        4..=12 => "Reading the request and the screens",
        13..=40 => "Laying out the screen",
        41..=90 => "Writing the HTML and styles",
        _ => "Still working. Bigger designs take a little longer",
    }
}

/// A plain name for an element by its tag, for the ask bar.
fn kind_of(tag: &str) -> &'static str {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "Heading",
        "button" => "Button",
        "a" => "Link",
        "img" | "svg" | "picture" => "Image",
        "input" | "textarea" | "select" => "Field",
        "p" | "span" | "label" | "strong" | "em" | "small" => "Text",
        "ul" | "ol" => "List",
        "li" => "List item",
        "nav" => "Navigation",
        "header" => "Header",
        "footer" => "Footer",
        "form" => "Form",
        "section" | "main" | "article" | "aside" => "Section",
        _ => "Box",
    }
}

/// A pixel length as a number: `12px` gives 12.
fn dom_px(css: &str) -> Option<f32> {
    css.trim().strip_suffix("px")?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn variants_are_named_after_their_screen() {
        assert_eq!(variant_of("home-v2").as_deref(), Some("home"));
        assert_eq!(variant_of("sign-up-v10").as_deref(), Some("sign-up"));
        assert_eq!(variant_of("home"), None);
        assert_eq!(variant_of("-v1"), None);
        assert_eq!(variant_of("review-vote"), None);
        let task = variants_task("home", "Home");
        assert!(task.contains("design/screens/home-v1.html"));
        assert!(task.contains("Don't change home.html"));
        let task = describe_task("a checkout with Apple Pay", Device::Phone);
        assert!(task.starts_with("Design a new phone screen: a checkout with Apple Pay"));
        assert!(task.contains("content=\"phone\""));
        assert_eq!(
            summary(&task),
            "Design a new phone screen: a checkout with Apple Pay"
        );
    }

    #[test]
    fn undoing_an_ai_change_puts_the_folder_back() {
        let dir = std::env::temp_dir().join(format!("runo-design-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("screens")).unwrap();
        std::fs::write(dir.join("tokens.css"), "a").unwrap();
        std::fs::write(dir.join("screens/home.html"), "<p>home</p>").unwrap();
        let before = snapshot(&dir);
        std::fs::write(dir.join("tokens.css"), "b").unwrap();
        std::fs::write(dir.join("screens/new.html"), "<p>new</p>").unwrap();
        std::fs::remove_file(dir.join("screens/home.html")).unwrap();
        restore(&dir, &before).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("tokens.css")).unwrap(),
            "a"
        );
        assert!(dir.join("screens/home.html").exists());
        assert!(!dir.join("screens/new.html").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
