//! The architecture editor on screen: a shape palette, the canvas, and a properties panel.
//! You draw; nothing changes the diagram behind your back.

use super::editor::{
    changes, search as search_presets, Beautify, Changes, Editor, Finding, Hit, Preset, Sel,
    GROUPS, PRESETS,
};
use super::geom::{self, Align, Guide, Handle, Pt, Side};
use super::icons::{self, Library};
use super::io;
use super::model::Style as DStyle;
use super::model::*;
use super::motion;
use super::shape::{self, Seg, Theme};
use crate::*;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

actions!(
    arch_editor,
    [CanvasEscape, CanvasTab, CanvasCopy, CanvasFind]
);

/// Keys the canvas takes before the app's own bindings for them.
pub(crate) fn bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("escape", CanvasEscape, Some("ArchEditor")),
        KeyBinding::new("tab", CanvasTab, Some("ArchEditor")),
        KeyBinding::new("secondary-c", CanvasCopy, Some("ArchEditor")),
        KeyBinding::new("secondary-f", CanvasFind, Some("ArchEditor")),
    ]
}

/// What the editor asks of the app around it.
pub(crate) enum ArchEvent {
    /// Start an agent session with this task and context.
    /// `edit`: the agent changes the diagram's files where the editor reads them.
    Ask {
        task: String,
        context: String,
        edit: bool,
        /// Pictures to attach, by path.
        images: Vec<String>,
        /// The agent to hand it to, by index into `agent::PROVIDERS`.
        agent: usize,
    },
    /// Show a file of the project.
    Open(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Select,
    Hand,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Properties,
    Text,
}

enum Drag {
    None,
    Pan {
        from: Pt,
        pan: Pt,
    },
    Move {
        from: Pt,
        start: Vec<(String, Rect)>,
        moved: bool,
    },
    Resize {
        id: String,
        handle: Handle,
        from: Pt,
        start: Rect,
        moved: bool,
    },
    Marquee {
        from: Pt,
        to: Pt,
        add: bool,
    },
    Connect {
        from: String,
        to: Pt,
    },
    Place {
        preset: &'static Preset,
        at: Option<Point<Pixels>>,
    },
    /// A logo or picture from the palette, by its file and name.
    Picture {
        file: PathBuf,
        name: String,
        at: Option<Point<Pixels>>,
    },
    /// Bending a connector by one of its segments.
    Bend {
        id: String,
        seg: usize,
        from: Pt,
        start: Vec<Pt>,
        straight: bool,
        moved: bool,
    },
}

/// The library of logos and tech icons, loaded the first time someone searches.
enum IconLib {
    Idle,
    Loading,
    Ready(Arc<Library>),
    Failed(String),
}

/// One icon's picture in the palette.
#[derive(Clone)]
enum Thumb {
    Loading,
    Ready(PathBuf),
    Failed,
}

/// How many logos a search shows.
const ICON_HITS: usize = 24;

fn thumb_key(i: &icons::Icon) -> String {
    format!("{}/{}", i.set.label(), i.slug)
}

enum Menu {
    Node(String),
    Edge,
    Canvas(Pt),
}

/// A field in the properties panel and what typing in it changes.
type Apply = fn(&mut ArchEditor, String);

const GRID: f32 = 10.;
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 4.;

pub(crate) struct ArchEditor {
    pub ed: Editor,
    root: PathBuf,
    project: String,
    focus: FocusHandle,
    pan: Pt,
    zoom: f32,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    drag: Drag,
    hover: Option<String>,
    /// Where the pointer is on the canvas, in diagram units; letter keys add shapes there.
    pointer: Option<Pt>,
    guides: Vec<Guide>,
    tool: Tool,
    space: bool,
    grid: bool,
    snap: bool,
    horizontal: bool,
    fitted: bool,
    side: SideTab,
    search: Entity<InputState>,
    f_name: Entity<InputState>,
    f_tech: Entity<InputState>,
    f_owner: Entity<InputState>,
    f_tags: Entity<InputState>,
    f_desc: Entity<TextareaState>,
    f_code: Entity<TextareaState>,
    f_view: Entity<InputState>,
    /// The diagram and selection the fields were filled for.
    loaded: Option<(String, Sel, u64)>,
    /// Bumped when the model changes from outside the fields (undo, text view).
    generation: u64,
    text_open: bool,
    inline: Entity<InputState>,
    editing: Option<Hit>,
    quick: Option<Pt>,
    quick_from: Option<String>,
    /// A connector let go on empty canvas: where, and from which shape.
    pending_quick: Option<(Pt, String)>,
    quick_q: Entity<InputState>,
    menu: Option<(Pt, Menu)>,
    t_model: Entity<TextareaState>,
    t_view: Entity<TextareaState>,
    text_loaded: Option<u64>,
    text_error: Option<String>,
    notice: Option<(String, Instant, bool)>,
    save_gen: u64,
    saved_at: Option<Instant>,
    export_open: bool,
    /// The Beautify menu is open.
    beautify_open: bool,
    scanning: bool,
    load_error: Option<String>,
    /// When the editor opened: the clock every animation runs on.
    born: Instant,
    /// Renaming a story (no step) or changing a step's caption.
    f_story: Entity<InputState>,
    story_edit: Option<(usize, Option<usize>)>,
    /// The story whose steps the panel lists.
    open_story: Option<usize>,
    /// A story playing: which, and when it started (or how far in it was paused).
    playing: Option<Playing>,
    /// A repaint is scheduled for the next animation frame.
    ticking: bool,
    /// Find on the canvas: open, what matched, and which match is shown.
    find_open: bool,
    f_find: Entity<InputState>,
    found: Vec<String>,
    found_at: usize,
    /// Renaming a layer.
    f_layer: Entity<InputState>,
    layer_edit: Option<usize>,
    /// Diagrams opened by drilling into a shape, for the way back.
    trail: Vec<String>,
    /// Asking an agent to change the diagram, and its change while it's being reviewed.
    f_ask: Entity<InputState>,
    waiting: Option<Waiting>,
    /// The agent the ask bar hands its task to.
    pick: crate::agent_pick::AgentPick,
    review: Option<Review>,
    /// Bumped to stop watching the files for an agent's edit.
    poll_gen: u64,
    /// The list of checks on the diagram is open.
    check_open: bool,
    /// A picture picked to turn into the diagram, waiting for the next frame.
    pending_picture: Option<PathBuf>,
    icons: IconLib,
    thumbs: std::collections::HashMap<String, Thumb>,
    _subs: Vec<Subscription>,
}

/// An agent asked to change the diagram that hasn't yet.
struct Waiting {
    /// The diagram's files and model as they were, to compare with and go back to.
    snap: Snapshot,
    before: Doc,
    stamp: u64,
    /// The agent's files don't read (yet): why.
    broken: Option<String>,
    /// When the agent was asked, to show how long it has been working.
    started: Instant,
    /// What it was asked, shortened.
    task: String,
}

/// An agent's change on the canvas, waiting to be accepted or rejected.
struct Review {
    snap: Snapshot,
    before: Doc,
    stamp: u64,
    broken: Option<String>,
    /// What changed on the diagram open now, and for which diagram and edit it was worked out.
    changes: Changes,
    seen: (String, u64),
}

#[derive(Clone, Copy)]
struct Playing {
    story: usize,
    start: Instant,
    paused: Option<f32>,
}

impl Playing {
    fn elapsed(&self) -> f32 {
        self.paused
            .unwrap_or_else(|| self.start.elapsed().as_secs_f32())
    }
}

impl EventEmitter<ArchEvent> for ArchEditor {}

impl Focusable for ArchEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn field(
    window: &mut Window,
    cx: &mut Context<ArchEditor>,
    placeholder: &'static str,
    apply: Apply,
    subs: &mut Vec<Subscription>,
) -> Entity<InputState> {
    let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    subs.push(cx.subscribe_in(
        &state,
        window,
        move |this, state, ev, window, cx| match ev {
            InputEvent::Focus => this.text_open = false,
            InputEvent::Change => {
                let v = state.read(cx).value().to_string();
                this.apply_field(apply, v, cx);
            }
            InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
            InputEvent::Blur => {}
        },
    ));
    state
}

fn area(
    window: &mut Window,
    cx: &mut Context<ArchEditor>,
    placeholder: &'static str,
    apply: Option<Apply>,
    subs: &mut Vec<Subscription>,
) -> Entity<TextareaState> {
    let state = cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder));
    if let Some(apply) = apply {
        subs.push(
            cx.subscribe_in(&state, window, move |this, state, ev, _, cx| match ev {
                InputEvent::Focus => this.text_open = false,
                InputEvent::Change => {
                    let v = state.read(cx).value().to_string();
                    this.apply_field(apply, v, cx);
                }
                _ => {}
            }),
        );
    }
    state
}

/// Whether a picked file is a picture rather than a diagram file.
fn is_picture(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif"
        )
    })
}

fn split_list(text: &str, by: char) -> Vec<String> {
    text.split(by)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

impl ArchEditor {
    pub(crate) fn new(
        root: PathBuf,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (doc, load_error) = match load(&root) {
            Ok(doc) => (doc, None),
            Err(e) => (Doc::default(), Some(e)),
        };
        let mut subs = Vec::new();
        let f_name = field(
            window,
            cx,
            "Name",
            |this, v| {
                if let Some(id) = this.one_node() {
                    if let Some(e) = this.ed.doc.element_mut(&id) {
                        e.name = v;
                    }
                } else if let Some(id) = this.one_edge() {
                    if let Some(r) = this.ed.doc.relation_mut(&id) {
                        r.label = v;
                    }
                }
            },
            &mut subs,
        );
        let f_tech = field(
            window,
            cx,
            "e.g. Go, Postgres 16, HTTPS / JSON",
            |this, v| {
                if let Some(id) = this.one_node() {
                    if let Some(e) = this.ed.doc.element_mut(&id) {
                        e.tech = v;
                    }
                } else if let Some(id) = this.one_edge() {
                    if let Some(r) = this.ed.doc.relation_mut(&id) {
                        r.tech = v;
                    }
                }
            },
            &mut subs,
        );
        let f_owner = field(
            window,
            cx,
            "Team or person",
            |this, v| {
                if let Some(e) = this.one_node().and_then(|id| this.ed.doc.element_mut(&id)) {
                    e.owner = v;
                }
            },
            &mut subs,
        );
        let f_tags = field(
            window,
            cx,
            "Comma separated",
            |this, v| {
                if let Some(e) = this.one_node().and_then(|id| this.ed.doc.element_mut(&id)) {
                    e.tags = split_list(&v, ',');
                }
            },
            &mut subs,
        );
        let f_desc = area(
            window,
            cx,
            "What it does",
            Some(|this, v| {
                if let Some(e) = this.one_node().and_then(|id| this.ed.doc.element_mut(&id)) {
                    e.description = v;
                }
            }),
            &mut subs,
        );
        let f_code = area(
            window,
            cx,
            "One path or glob per line, e.g. services/billing/**",
            Some(|this, v| {
                if let Some(e) = this.one_node().and_then(|id| this.ed.doc.element_mut(&id)) {
                    e.code = split_list(&v, '\n');
                }
            }),
            &mut subs,
        );
        let f_view = field(
            window,
            cx,
            "Diagram name",
            |this, v| {
                if !v.trim().is_empty() {
                    this.ed.cur_mut().name = v.trim().to_string();
                }
            },
            &mut subs,
        );
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search shapes and logos"));
        subs.push(cx.subscribe_in(&search, window, |this, _, ev, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.icons_for_search(cx);
                cx.notify();
            }
        }));
        let inline = cx.new(|cx| InputState::new(window, cx));
        subs.push(
            cx.subscribe_in(&inline, window, |this, state, ev, window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    let v = state.read(cx).value().to_string();
                    this.commit_inline(v, window, cx);
                }
                _ => {}
            }),
        );
        let quick_q =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add a shape: type to search"));
        subs.push(
            cx.subscribe_in(&quick_q, window, |this, state, ev, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    let q = state.read(cx).value().to_string();
                    if let Some(p) = search_presets(&q).first() {
                        this.quick_add(p, window, cx);
                    }
                }
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
        );
        let t_model = area(window, cx, "", None, &mut subs);
        let t_view = area(window, cx, "", None, &mut subs);
        let f_story = cx.new(|cx| InputState::new(window, cx));
        subs.push(
            cx.subscribe_in(&f_story, window, |this, state, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = state.read(cx).value().to_string();
                    if let Some((i, step)) = this.story_edit.take() {
                        this.act(cx, |ed| match step {
                            Some(j) => ed.set_caption(i, j, v.trim()),
                            None => ed.rename_story(i, &v),
                        });
                    }
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        window.focus(&this.focus, cx);
                    }
                }
            }),
        );
        let f_find =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find by name, tech or #tag"));
        subs.push(
            cx.subscribe_in(&f_find, window, |this, state, ev, window, cx| match ev {
                InputEvent::Change => {
                    let q = state.read(cx).value().to_string();
                    this.found = this.ed.find(&q);
                    this.found_at = 0;
                    this.show_found(cx);
                }
                InputEvent::PressEnter { .. } => {
                    this.step_found(1, cx);
                    let _ = window;
                }
                _ => {}
            }),
        );
        let f_ask =
            cx.new(|cx| InputState::new(window, cx).placeholder("Ask AI to change the diagram…"));
        subs.push(
            cx.subscribe_in(&f_ask, window, |this, _, ev, window, cx| match ev {
                InputEvent::PressEnter { .. } => this.send_ask(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
        );
        let f_layer = cx.new(|cx| InputState::new(window, cx).placeholder("Layer name"));
        subs.push(
            cx.subscribe_in(&f_layer, window, |this, state, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = state.read(cx).value().to_string();
                    if let Some(i) = this.layer_edit.take() {
                        this.act(cx, |ed| ed.rename_layer(i, &v));
                    }
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        window.focus(&this.focus, cx);
                    }
                }
            }),
        );
        Self {
            ed: Editor::new(doc),
            root,
            project,
            focus: cx.focus_handle(),
            pan: (0., 0.),
            zoom: 1.,
            bounds: Rc::new(Cell::new(Bounds::default())),
            drag: Drag::None,
            hover: None,
            pointer: None,
            guides: Vec::new(),
            tool: Tool::Select,
            space: false,
            grid: true,
            snap: true,
            horizontal: false,
            fitted: false,
            side: SideTab::Properties,
            search,
            f_name,
            f_tech,
            f_owner,
            f_tags,
            f_desc,
            f_code,
            f_view,
            loaded: None,
            generation: 0,
            text_open: false,
            inline,
            editing: None,
            quick: None,
            quick_from: None,
            pending_quick: None,
            quick_q,
            menu: None,
            t_model,
            t_view,
            text_loaded: None,
            text_error: None,
            notice: None,
            save_gen: 0,
            saved_at: None,
            export_open: false,
            beautify_open: false,
            scanning: false,
            load_error,
            born: Instant::now(),
            f_story,
            story_edit: None,
            open_story: None,
            playing: None,
            ticking: false,
            find_open: false,
            f_find,
            found: Vec::new(),
            found_at: 0,
            f_layer,
            layer_edit: None,
            trail: Vec::new(),
            f_ask,
            waiting: None,
            pick: crate::agent_pick::AgentPick::new(cx),
            review: None,
            poll_gen: 0,
            check_open: false,
            pending_picture: None,
            icons: IconLib::Idle,
            thumbs: Default::default(),
            _subs: subs,
        }
    }

    /// Read the files again if nothing is waiting to be saved (an agent or git may have
    /// changed them).
    pub(crate) fn reload_if_clean(&mut self, cx: &mut Context<Self>) {
        if self.ed.dirty || !exists(&self.root) || self.waiting.is_some() || self.review.is_some() {
            return;
        }
        match load(&self.root) {
            Ok(doc) if doc != self.ed.doc => {
                let view = self.ed.view.clone();
                self.ed = Editor::new(doc);
                self.ed.open_view(&view);
                self.generation += 1;
                cx.notify();
            }
            Ok(_) => {}
            Err(e) => self.load_error = Some(e),
        }
    }

    fn one_node(&self) -> Option<String> {
        (self.ed.sel.nodes.len() == 1 && self.ed.sel.edges.is_empty())
            .then(|| self.ed.sel.nodes[0].clone())
    }

    /// The one selected connector, when it can be bent by hand: its route, and whether
    /// it's straight (curves can't be).
    fn bendable(&self) -> Option<(String, geom::Wire, bool)> {
        let id = self.one_edge()?;
        let e = self.ed.cur().edge(&id)?;
        if e.route == Route::Curved {
            return None;
        }
        let (_, w) = self.ed.wires().into_iter().find(|(x, _)| *x == id)?;
        Some((id, w, e.route == Route::Straight))
    }

    fn one_edge(&self) -> Option<String> {
        (self.ed.sel.edges.len() == 1 && self.ed.sel.nodes.is_empty())
            .then(|| self.ed.sel.edges[0].clone())
    }

    fn apply_field(&mut self, apply: Apply, value: String, cx: &mut Context<Self>) {
        if !self.text_open {
            self.ed.checkpoint();
            self.text_open = true;
        }
        apply(self, value);
        self.ed.dirty = true;
        self.text_loaded = None;
        self.changed(cx);
    }

    /// Something changed: save shortly, once the edits settle.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.save_gen += 1;
        let gen = self.save_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.save_gen == gen {
                    this.save_now(cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn save_now(&mut self, cx: &mut Context<Self>) {
        if !self.ed.dirty {
            return;
        }
        match save(&self.root, &self.ed.doc) {
            Ok(()) => {
                self.ed.dirty = false;
                self.saved_at = Some(Instant::now());
            }
            Err(e) => self.note(format!("Couldn't save the diagram: {e}"), true),
        }
        cx.notify();
    }

    fn note(&mut self, text: String, error: bool) {
        if error {
            crate::applog::error("architecture", &text);
        }
        self.notice = Some((text, Instant::now(), error));
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.text_loaded = None;
        self.changed(cx);
    }

    // ---- Camera

    fn local(&self, pos: Point<Pixels>) -> Pt {
        let b = self.bounds.get();
        (f32::from(pos.x - b.origin.x), f32::from(pos.y - b.origin.y))
    }

    fn inside(&self, pos: Point<Pixels>) -> bool {
        self.bounds.get().contains(&pos)
    }

    fn world(&self, local: Pt) -> Pt {
        (
            (local.0 - self.pan.0) / self.zoom,
            (local.1 - self.pan.1) / self.zoom,
        )
    }

    fn screen(&self, p: Pt) -> Pt {
        (p.0 * self.zoom + self.pan.0, p.1 * self.zoom + self.pan.1)
    }

    fn screen_rect(&self, r: &Rect) -> Rect {
        let (x, y) = self.screen((r.x, r.y));
        Rect::new(x, y, r.w * self.zoom, r.h * self.zoom)
    }

    /// The part of the diagram on screen, with a margin for strokes and glows.
    fn visible_world(&self) -> Rect {
        let (w, h) = self.view_size();
        Rect::new(
            -self.pan.0 / self.zoom,
            -self.pan.1 / self.zoom,
            w / self.zoom,
            h / self.zoom,
        )
        .inflate(40. / self.zoom)
    }

    fn view_size(&self) -> Pt {
        let b = self.bounds.get();
        (f32::from(b.size.width), f32::from(b.size.height))
    }

    fn center_world(&self) -> Pt {
        let (w, h) = self.view_size();
        self.world((w / 2., h / 2.))
    }

    fn zoom_at(&mut self, local: Pt, factor: f32) {
        let w = self.world(local);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = (local.0 - w.0 * self.zoom, local.1 - w.1 * self.zoom);
    }

    fn zoom_step(&mut self, factor: f32) {
        let (w, h) = self.view_size();
        self.zoom_at((w / 2., h / 2.), factor);
    }

    /// Whether every shape on the diagram is inside the canvas as it's zoomed now.
    fn shows_all(&self) -> bool {
        let (vw, vh) = self.view_size();
        let Some(area) = Rect::bounds(
            self.ed
                .cur()
                .nodes
                .iter()
                .map(|n| n.rect())
                .collect::<Vec<_>>(),
        ) else {
            return true;
        };
        let (x0, y0) = self.screen((area.x, area.y));
        let (x1, y1) = self.screen((area.x + area.w, area.y + area.h));
        x0 >= 0. && y0 >= 0. && x1 <= vw && y1 <= vh
    }

    /// Show the whole diagram, or the selection.
    fn fit(&mut self, selection: bool) {
        let (vw, vh) = self.view_size();
        if vw < 10. {
            return;
        }
        let rects: Vec<Rect> = if selection && !self.ed.sel.nodes.is_empty() {
            self.ed
                .sel
                .nodes
                .iter()
                .filter_map(|id| self.ed.rect(id))
                .collect()
        } else {
            self.ed.cur().nodes.iter().map(|n| n.rect()).collect()
        };
        let Some(area) = Rect::bounds(rects) else {
            self.zoom = 1.;
            self.pan = (vw / 2., vh / 2.);
            return;
        };
        let area = area.inflate(60.);
        self.zoom = (vw / area.w).min(vh / area.h).clamp(MIN_ZOOM, 1.5);
        self.pan = (
            vw / 2. - (area.x + area.w / 2.) * self.zoom,
            vh / 2. - (area.y + area.h / 2.) * self.zoom,
        );
    }

    // ---- Commands

    fn open_view(&mut self, id: &str, cx: &mut Context<Self>) {
        self.trail.clear();
        self.go_to_view(id, cx);
    }

    /// Open the diagram a shape links to, remembering the way back.
    fn drill(&mut self, id: &str, cx: &mut Context<Self>) {
        let here = self.ed.view.clone();
        if here != id {
            self.trail.push(here);
        }
        self.go_to_view(id, cx);
    }

    /// Go back up the trail to the diagram at `i`.
    fn back_to(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(id) = self.trail.get(i).cloned() {
            self.trail.truncate(i);
            self.go_to_view(&id, cx);
        }
    }

    fn go_to_view(&mut self, id: &str, cx: &mut Context<Self>) {
        self.find_open = false;
        self.found.clear();
        self.ed.open_view(id);
        self.fitted = false;
        self.menu = None;
        self.editing = None;
        self.text_loaded = None;
        cx.notify();
    }

    fn add_preset(&mut self, preset: &Preset, at: Pt, cx: &mut Context<Self>) -> String {
        let id = self.ed.add(preset, at);
        self.after_edit(cx);
        id
    }

    /// Search the logo library for what's typed in the palette: load the library the
    /// first time, then download the pictures of the icons it finds.
    fn icons_for_search(&mut self, cx: &mut Context<Self>) {
        let q = self.search.read(cx).value().trim().to_string();
        if q.chars().count() < 2 {
            return;
        }
        match &self.icons {
            IconLib::Idle | IconLib::Failed(_) => {
                self.icons = IconLib::Loading;
                cx.spawn(async move |this, cx| {
                    let lib = cx.background_spawn(async move { icons::load() }).await;
                    let _ = this.update(cx, |this, cx| {
                        this.icons = match lib {
                            Ok(l) => IconLib::Ready(Arc::new(l)),
                            Err(e) => IconLib::Failed(e),
                        };
                        this.icons_for_search(cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            IconLib::Ready(lib) => {
                let want: Vec<icons::Icon> = lib
                    .search(&q, ICON_HITS)
                    .into_iter()
                    .filter(|i| {
                        !matches!(
                            self.thumbs.get(&thumb_key(i)),
                            Some(Thumb::Loading | Thumb::Ready(_))
                        )
                    })
                    .cloned()
                    .collect();
                for i in want {
                    let key = thumb_key(&i);
                    self.thumbs.insert(key.clone(), Thumb::Loading);
                    cx.spawn(async move |this, cx| {
                        let got = cx.background_spawn(async move { icons::fetch(&i) }).await;
                        let _ = this.update(cx, |this, cx| {
                            this.thumbs
                                .insert(key, got.map(Thumb::Ready).unwrap_or(Thumb::Failed));
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
            IconLib::Loading => {}
        }
    }

    /// Put a picture on the diagram: copied into the diagram's `icons/` folder, then on the
    /// shape it was dropped on, or as a part of its own.
    fn place_picture(
        &mut self,
        file: &std::path::Path,
        name: &str,
        at: Option<Pt>,
        cx: &mut Context<Self>,
    ) {
        let image = match icons::add_to_project(&self.root, file, name) {
            Ok(i) => i,
            Err(e) => {
                self.note(e, true);
                cx.notify();
                return;
            }
        };
        if let Some(at) = at {
            if let Hit::Node(id) = self.ed.hit(at, 2. / self.zoom) {
                let kind = self.ed.kind_of(&id);
                if kind.is_part() {
                    self.ed.set_image(&id, &image);
                    self.ed.sel = Sel {
                        nodes: vec![id],
                        edges: Vec::new(),
                    };
                    self.after_edit(cx);
                    return;
                }
            }
        }
        let at = at.unwrap_or_else(|| self.ed.free_spot(self.center_world(), Kind::Icon.size()));
        self.ed.add_icon(name, &image, at);
        self.after_edit(cx);
    }

    /// Pick a picture of your own to put on the diagram.
    fn pick_picture(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Add a picture: SVG, PNG, JPG, WebP or GIF".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let _ = this.update(cx, |this, cx| {
                    if !icons::is_picture(&path) {
                        this.note("That isn't a picture Runo can show.".into(), true);
                        cx.notify();
                        return;
                    }
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Picture")
                        .to_string();
                    this.place_picture(&path, &name, None, cx);
                });
            }
        })
        .detach();
    }

    fn quick_add(&mut self, preset: &Preset, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.quick.take().unwrap_or_else(|| self.center_world());
        let at = self.ed.free_spot(at, preset.kind.size());
        let id = self.add_preset(preset, at, cx);
        if let Some(from) = self.quick_from.take() {
            self.ed.connect(&from, &id);
            self.after_edit(cx);
            self.ed.sel = Sel {
                nodes: vec![id],
                edges: Vec::new(),
            };
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn open_quick(
        &mut self,
        at: Pt,
        from: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick = Some(at);
        self.quick_from = from;
        self.quick_q.update(cx, |q, cx| q.set_value("", window, cx));
        // After the click that opened it, which focuses the canvas.
        let q = self.quick_q.clone();
        window.defer(cx, move |window, cx| {
            q.update(cx, |q, cx| q.focus(window, cx))
        });
        cx.notify();
    }

    fn start_inline(&mut self, hit: Hit, window: &mut Window, cx: &mut Context<Self>) {
        let text = match &hit {
            Hit::Node(id) => self.ed.doc.element(id).map(|e| e.name.clone()),
            Hit::Edge(id) => self.ed.doc.relation(id).map(|r| r.label.clone()),
            Hit::None => None,
        };
        let Some(text) = text else { return };
        if let Hit::Node(id) = &hit {
            if self.ed.cur().node(id).is_some_and(|n| n.locked) {
                return;
            }
        }
        self.editing = Some(hit);
        self.inline
            .update(cx, |s, cx| s.set_value(text, window, cx));
        let input = self.inline.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    fn commit_inline(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.editing.take() else {
            return;
        };
        let old = match &hit {
            Hit::Node(id) => self.ed.doc.element(id).map(|e| e.name.clone()),
            Hit::Edge(id) => self.ed.doc.relation(id).map(|r| r.label.clone()),
            Hit::None => None,
        };
        if old.as_deref() != Some(value.as_str()) {
            self.ed.checkpoint();
            match &hit {
                Hit::Node(id) => {
                    if let Some(e) = self.ed.doc.element_mut(id) {
                        e.name = value;
                    }
                }
                Hit::Edge(id) => {
                    if let Some(r) = self.ed.doc.relation_mut(id) {
                        r.label = value;
                    }
                }
                Hit::None => {}
            }
            self.after_edit(cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if self.ed.undo() {
            self.after_edit(cx);
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if self.ed.redo() {
            self.after_edit(cx);
        }
    }

    /// Run an editor command that changes the model.
    fn act(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Editor)) {
        self.commit_layer_name(cx);
        f(&mut self.ed);
        self.menu = None;
        self.after_edit(cx);
    }

    fn start_from_code(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        let root = self.root.clone();
        let project = self.project.clone();
        cx.spawn(async move |this, cx| {
            let doc = cx
                .background_spawn(async move {
                    let map = crate::architecture::scan(&root);
                    io::from_scan(&map, &project)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.scanning = false;
                if doc.elements.is_empty() {
                    this.note("No packages found to draw. Start from the palette instead.".into(), false);
                } else {
                    let empty = this.ed.doc.elements.is_empty();
                    io::merge(&mut this.ed, doc);
                    if empty {
                        // The blank starter diagram isn't needed any more.
                        let keep = this.ed.view.clone();
                        this.ed.doc.views.retain(|v| v.id == keep || !v.nodes.is_empty());
                    }
                    this.fitted = false;
                    this.note(
                        "Drew the packages and the services they use. Rearrange and rename freely; it's your diagram now.".into(),
                        false,
                    );
                    this.after_edit(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import a draw.io diagram, or a picture of one".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                if is_picture(&path) {
                    let _ = this.update(cx, |this, cx| {
                        this.pending_picture = Some(path);
                        cx.notify();
                    });
                    return;
                }
                let result = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| io::import_drawio(&t));
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(doc) => {
                            let n = doc.views.len();
                            let empty = this.ed.doc.elements.is_empty();
                            io::merge(&mut this.ed, doc);
                            if empty {
                                let keep = this.ed.view.clone();
                                this.ed
                                    .doc
                                    .views
                                    .retain(|v| v.id == keep || !v.nodes.is_empty());
                            }
                            this.fitted = false;
                            this.note(
                                format!(
                                    "Imported {} as {n} diagram{}.",
                                    path.file_name()
                                        .map(|f| f.to_string_lossy().into_owned())
                                        .unwrap_or_default(),
                                    if n == 1 { "" } else { "s" }
                                ),
                                false,
                            );
                            this.after_edit(cx);
                        }
                        Err(e) => this.note(format!("Couldn't import it: {e}"), true),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Save an export through the save dialog, falling back to `architecture/export/`.
    fn export(&mut self, ext: &'static str, cx: &mut Context<Self>) {
        self.export_open = false;
        let mut doc = self.ed.doc.clone();
        icons::inline_pictures(&self.root, &mut doc);
        let view = self.ed.cur().clone();
        let name = format!(
            "{}.{ext}",
            if ext == "drawio" {
                slug(&self.project, "architecture")
            } else {
                view.id.clone()
            }
        );
        let fallback = folder(&self.root).join("export").join(&name);
        let rx = cx.prompt_for_new_path(&folder(&self.root), Some(&name));
        cx.spawn(async move |this, cx| {
            let path = match rx.await {
                Ok(Ok(Some(p))) => p,
                Ok(Ok(None)) => return,
                _ => fallback,
            };
            let out = path.clone();
            let result = cx
                .background_spawn(async move {
                    let bytes = match ext {
                        "svg" => io::svg(&doc, &view).into_bytes(),
                        "motion.svg" => io::animated_svg(&doc, &view).into_bytes(),
                        "html" => io::story_html(&doc, &view).into_bytes(),
                        "png" => io::png(&doc, &view)?,
                        "mmd" => io::mermaid(&doc, &view).into_bytes(),
                        _ => io::drawio(&doc).into_bytes(),
                    };
                    if let Some(dir) = out.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&out, bytes).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.note(format!("Exported to {}.", path.display()), false),
                    Err(e) => this.note(format!("Couldn't export: {e}"), true),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn copy_mermaid(&mut self, cx: &mut Context<Self>) {
        self.export_open = false;
        let text = io::mermaid(&self.ed.doc, self.ed.cur());
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.note(
            "Copied the diagram as Mermaid. Paste it into a README or a doc.".into(),
            false,
        );
        cx.notify();
    }

    /// The first file of an element's code, to open.
    fn code_file(&self, id: &str) -> Option<PathBuf> {
        let e = self.ed.doc.element(id)?;
        for pattern in &e.code {
            let base = pattern
                .split(['*', '?', '['])
                .next()
                .unwrap_or("")
                .trim_end_matches('/');
            let path = self.root.join(base);
            if path.is_file() {
                return Some(path);
            }
            if path.is_dir() {
                let (files, _) = crate::architecture::walk(&path);
                let mut files: Vec<PathBuf> = files;
                files.sort_by_key(|f| {
                    let name = f
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    let main = ["main.", "lib.", "index.", "app.", "mod.", "readme"]
                        .iter()
                        .any(|m| name.starts_with(m));
                    (!main, f.components().count())
                });
                if let Some(f) = files.into_iter().next() {
                    return Some(path.join(f));
                }
            }
        }
        None
    }

    fn ask_about(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(e) = self.ed.doc.element(id).cloned() else {
            return;
        };
        let mut context = format!("## Architecture element: {} ({})\n", e.name, e.kind.label());
        if !e.tech.is_empty() {
            context.push_str(&format!("Technology: {}\n", e.tech));
        }
        if !e.description.is_empty() {
            context.push_str(&format!("Description: {}\n", e.description));
        }
        if !e.code.is_empty() {
            context.push_str(&format!("Code: {}\n", e.code.join(", ")));
        }
        let rels: Vec<String> = self
            .ed
            .doc
            .relations
            .iter()
            .filter(|r| r.from == e.id || r.to == e.id)
            .filter_map(|r| {
                let a = self.ed.doc.element(&r.from)?;
                let b = self.ed.doc.element(&r.to)?;
                let how = [r.label.as_str(), r.tech.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(if how.is_empty() {
                    format!("- {} → {}", a.name, b.name)
                } else {
                    format!("- {} → {} ({how})", a.name, b.name)
                })
            })
            .collect();
        if !rels.is_empty() {
            context.push_str("Connections on the diagram:\n");
            context.push_str(&rels.join("\n"));
            context.push('\n');
        }
        context.push_str(&format!(
            "\nThe diagram lives in {FOLDER}/ (model.toml and views/*.toml). Don't change those files unless asked.\n"
        ));
        cx.emit(ArchEvent::Ask {
            task: format!(
                "Explain how {} works in the code and what depends on it.",
                e.name
            ),
            context,
            edit: false,
            images: Vec::new(),
            agent: self.pick.agent,
        });
    }

    // ---- Asking an agent to change the diagram

    /// Hand the request in the ask bar to an agent that edits the diagram's files, and
    /// watch them for its change.
    fn send_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.f_ask.read(cx).value().trim().to_string();
        if task.is_empty() {
            return;
        }
        if self.ask_edit(task, Vec::new(), window, cx) {
            self.f_ask.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }

    /// Start an agent on a change to the diagram and watch for it. Returns whether it
    /// started.
    fn ask_edit(
        &mut self,
        task: String,
        images: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.waiting.is_some() {
            self.note("An agent is already working on the diagram.".into(), false);
            cx.notify();
            return false;
        }
        if self.review.is_some() {
            self.note("Accept or reject the last change first.".into(), false);
            cx.notify();
            return false;
        }
        // The agent works from the files, so they must hold what's on screen, with the
        // guide to them alongside.
        if let Err(e) = save(&self.root, &self.ed.doc) {
            self.note(format!("Couldn't save the diagram: {e}"), true);
            cx.notify();
            return false;
        }
        self.ed.dirty = false;
        self.saved_at = Some(Instant::now());
        let context = self.ask_context();
        self.waiting = Some(Waiting {
            snap: snapshot(&self.root),
            before: self.ed.doc.clone(),
            stamp: stamp(&self.root),
            broken: None,
            started: Instant::now(),
            task: task.chars().take(120).collect(),
        });
        window.focus(&self.focus, cx);
        cx.emit(ArchEvent::Ask {
            task,
            context,
            edit: true,
            images,
            agent: self.pick.agent,
        });
        self.pick.open = false;
        self.watch(cx);
        cx.notify();
        true
    }

    /// Put the cursor in the ask bar, for a change to what's selected.
    fn focus_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let input = self.f_ask.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    /// Have an agent break a part down into a diagram of its own, linked from it.
    fn expand_with_ai(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.ed.doc.element(id) else {
            return;
        };
        let name = e.name.replace('\n', " ");
        let task = format!(
            "Break {name} (id {id}) down into its main parts, from the code where you can find \
             it. Add them as elements, add a new diagram {FOLDER}/views/{id}-inside.toml that \
             shows them and how they connect to each other and to what {name} talks to, and set \
             link = \"{id}-inside\" on {name} so it opens that diagram."
        );
        self.ask_edit(task, Vec::new(), window, cx);
    }

    /// Have an agent add what the diagram leaves out.
    fn suggest_missing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = "Compare this diagram with the project's code and add what it leaves out: \
                    services, data stores, queues, external systems and the connections between \
                    them, each placed near what it connects to and labelled with what flows. \
                    Don't remove anything."
            .to_string();
        self.ask_edit(task, Vec::new(), window, cx);
    }

    /// Have an agent redraw a picture of a diagram (a whiteboard photo, a sketch, a
    /// screenshot) as this one.
    fn draw_picture(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let empty = self.ed.cur().nodes.is_empty();
        let task = format!(
            "Turn the attached picture of an architecture diagram into {}. Keep its parts, \
             groupings, connections and labels, use the closest kinds (person, container, \
             database, queue, external, boundary and so on), and lay the shapes out like the \
             picture, tidied onto a grid.",
            if empty {
                "this diagram"
            } else {
                "a new diagram next to this one"
            }
        );
        self.ask_edit(task, vec![path.to_string_lossy().into_owned()], window, cx);
    }

    /// Select what a finding is about and bring it into view.
    fn show_finding(&mut self, f: &Finding, cx: &mut Context<Self>) {
        for id in &f.nodes {
            if !self.ed.sight().shows(id) {
                self.ed.reveal(id);
            }
        }
        self.ed.sel = Sel {
            nodes: f.nodes.clone(),
            edges: f.edge.iter().cloned().collect(),
        };
        let first = f.nodes.first().cloned().or_else(|| {
            let r = self.ed.doc.relation(f.edge.as_deref()?)?;
            Some(r.from.clone())
        });
        if let Some(r) = first.and_then(|id| self.ed.rect(&id)) {
            let vis = self.visible_world().inflate(-40. / self.zoom);
            if !vis.contains_rect(&r) {
                let (vw, vh) = self.view_size();
                let (cx_, cy_) = r.center();
                self.pan = (vw / 2. - cx_ * self.zoom, vh / 2. - cy_ * self.zoom);
            }
        }
        cx.notify();
    }

    /// The checks on the diagram open now, each a click away from what it's about.
    fn check_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.check_open {
            return None;
        }
        let found = self.ed.findings();
        let mut list = div().v_flex().max_h(px(360.)).overflow_y_hidden();
        if found.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child("Nothing to fix: every part is connected and labelled."),
            );
        }
        for (i, f) in found.iter().take(40).enumerate() {
            let f2 = f.clone();
            list = list.child(
                div()
                    .id(("arch-finding", i))
                    .px_3()
                    .py(px(6.))
                    .flex()
                    .gap_2()
                    .items_start()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .text_size(px(12.5))
                    .text_color(rgb(c_text()))
                    .child(
                        div()
                            .pt(px(2.))
                            .child(icon(IconName::CircleAlert, 12., 0xf59e0b)),
                    )
                    .child(div().flex_1().min_w_0().child(f.text.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| this.show_finding(&f2, cx))),
            );
        }
        let score = self.ed.layout_score();
        if !score.is_clean() {
            list = list.child(
                div()
                    .id("arch-finding-layout")
                    .px_3()
                    .py(px(6.))
                    .flex()
                    .gap_2()
                    .items_center()
                    .rounded_md()
                    .text_size(px(12.5))
                    .text_color(rgb(c_text()))
                    .child(icon(IconName::CircleAlert, 12., 0xf59e0b))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(format!("Hard to follow: {}", score.describe())),
                    )
                    .child(
                        text_btn(
                            "arch-finding-beautify",
                            "Beautify",
                            Some(IconName::WandSparkles),
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.check_open = false;
                            this.run_beautify(Beautify::Best, cx);
                        })),
                    ),
            );
        }
        let more = found.len().saturating_sub(40);
        Some(
            div()
                .absolute()
                .top(px(6.))
                .right(px(8.))
                .w(px(340.))
                .p_1()
                .v_flex()
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(float(c_canvas()))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(c_text()))
                                .child(if found.is_empty() {
                                    "Checks".to_string()
                                } else {
                                    format!("{} to look at", found.len())
                                }),
                        )
                        .child(
                            icon_btn("arch-check-close", IconName::X, "Close", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.check_open = false;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .child(list)
                .when(more > 0, |el| {
                    el.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("And {more} more")),
                    )
                })
                .when(!found.is_empty(), |el| {
                    el.child(
                        div().p_2().flex().justify_end().child(
                            text_btn(
                                "arch-check-fix",
                                "Fix with AI",
                                Some(IconName::Sparkles),
                                true,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.fix_findings(window, cx)),
                            ),
                        ),
                    )
                })
                .into_any_element(),
        )
    }

    /// Have an agent fix what the checks found.
    fn fix_findings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.ed.findings();
        if found.is_empty() {
            return;
        }
        let mut task = String::from(
            "Fix these problems on the diagram, using the project's code for missing labels and \
             technologies, and moving shapes apart where they overlap:\n",
        );
        for f in found.iter().take(60) {
            task.push_str(&format!("- {}\n", f.text));
        }
        self.check_open = false;
        self.ask_edit(task, Vec::new(), window, cx);
    }

    /// What an agent needs to change the diagram: where the files are, which diagram is
    /// open and what's selected on it.
    fn ask_context(&self) -> String {
        let v = self.ed.cur();
        let mut out = format!(
            "## Change the architecture diagram\n\
             The diagram is in {FOLDER}/. Read {FOLDER}/README.md for the file format, then make \
             the change by editing {FOLDER}/model.toml and {FOLDER}/views/*.toml directly. Runo \
             shows your edit on the canvas as a preview the user accepts or rejects, so don't ask \
             before editing. Change no other files for this request.\n\n\
             The diagram open now is \"{}\" ({FOLDER}/views/{}.toml); change that one unless the \
             request says otherwise.\n",
            v.name, v.id
        );
        let name = |id: &str| {
            self.ed
                .doc
                .element(id)
                .map(|e| {
                    format!(
                        "{} (id {id}, {})",
                        e.name.replace('\n', " "),
                        e.kind.label()
                    )
                })
                .unwrap_or_else(|| id.to_string())
        };
        if !self.ed.sel.nodes.is_empty() || !self.ed.sel.edges.is_empty() {
            out.push_str("\nSelected on the canvas (\"this\" or \"these\" means them):\n");
            for id in &self.ed.sel.nodes {
                out.push_str(&format!("- {}\n", name(id)));
            }
            for id in &self.ed.sel.edges {
                if let Some(r) = self.ed.doc.relation(id) {
                    out.push_str(&format!(
                        "- the connector {} → {} (id {id})\n",
                        name(&r.from),
                        name(&r.to)
                    ));
                }
            }
        }
        out.push_str(&format!(
            "\nThe diagram has {} shapes and {} connectors.\n",
            v.nodes.len(),
            v.edges.len()
        ));
        out
    }

    /// Check the diagram's files for an agent's edit every so often, while one is awaited
    /// or under review.
    fn watch(&mut self, cx: &mut Context<Self>) {
        self.poll_gen += 1;
        let gen = self.poll_gen;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            let more = this
                .update(cx, |this, cx| this.check_files(gen, cx))
                .unwrap_or(false);
            if !more {
                break;
            }
        })
        .detach();
    }

    /// Show an agent's edit if the files changed. Returns whether to keep watching.
    fn check_files(&mut self, gen: u64, cx: &mut Context<Self>) -> bool {
        if gen != self.poll_gen || (self.waiting.is_none() && self.review.is_none()) {
            return false;
        }
        // Unsaved edits on the canvas win; look again once they're saved.
        if self.ed.dirty {
            return true;
        }
        let now = stamp(&self.root);
        let (seen, broken) = match (&mut self.waiting, &mut self.review) {
            (Some(w), _) => (&mut w.stamp, &mut w.broken),
            (None, Some(r)) => (&mut r.stamp, &mut r.broken),
            _ => return false,
        };
        if *seen == now {
            return true;
        }
        *seen = now;
        match load(&self.root) {
            Ok(doc) => {
                *broken = None;
                if doc != self.ed.doc {
                    let view = self.ed.view.clone();
                    self.ed.replace_doc(doc);
                    self.ed.open_view(&view);
                    // A fresh drawing, or one that grew past the edges, is brought into view.
                    if !self.shows_all() {
                        self.fitted = false;
                    }
                    if let Some(w) = self.waiting.take() {
                        self.review = Some(Review {
                            snap: w.snap,
                            before: w.before,
                            stamp: w.stamp,
                            broken: None,
                            changes: Changes::default(),
                            seen: (String::new(), u64::MAX),
                        });
                    }
                    self.generation += 1;
                    self.text_loaded = None;
                    self.load_error = None;
                }
            }
            Err(e) => *broken = Some(e),
        }
        cx.notify();
        true
    }

    /// Keep what the agent did.
    fn accept(&mut self, cx: &mut Context<Self>) {
        self.review = None;
        self.poll_gen += 1;
        cx.notify();
    }

    /// Put the diagram's files back exactly as they were before the agent's edit.
    fn reject(&mut self, cx: &mut Context<Self>) {
        let (snap, before) = match (self.review.take(), self.waiting.take()) {
            (Some(r), _) => (r.snap, r.before),
            (None, Some(w)) => (w.snap, w.before),
            _ => return,
        };
        self.poll_gen += 1;
        if let Err(e) = restore(&self.root, &snap) {
            self.note(format!("Couldn't put the diagram back: {e}"), true);
        }
        let view = self.ed.view.clone();
        self.ed.replace_doc(load(&self.root).unwrap_or(before));
        self.ed.open_view(&view);
        self.generation += 1;
        self.text_loaded = None;
        cx.notify();
    }

    fn stop_waiting(&mut self, cx: &mut Context<Self>) {
        self.waiting = None;
        self.poll_gen += 1;
        cx.notify();
    }

    /// Work out again what the agent changed when the diagram or the model moved on.
    fn refresh_review(&mut self) {
        let key = (self.ed.view.clone(), self.generation);
        let Some(r) = &mut self.review else {
            return;
        };
        if r.seen != key {
            r.changes = changes(&r.before, &self.ed.doc, &self.ed.view);
            r.seen = key;
        }
    }

    /// The agent's change drawn over the diagram.
    fn review_draw(&self) -> Option<ReviewDraw> {
        let r = self.review.as_ref()?;
        let c = &r.changes;
        let rects = |ids: &mut dyn Iterator<Item = &String>| -> Vec<Rect> {
            ids.filter_map(|id| self.ed.rect(id)).collect()
        };
        let wires: std::collections::HashMap<String, geom::Wire> = if c.edges_added.is_empty() {
            Default::default()
        } else {
            self.ed.wires().into_iter().collect()
        };
        Some(ReviewDraw {
            added: rects(&mut c.added.iter()),
            edited: rects(&mut c.edited.iter().filter(|id| !c.added.contains(id))),
            removed: c.removed.iter().map(|(r, _)| *r).collect(),
            moved: c.moved.iter().map(|(_, r)| *r).collect(),
            wires_added: c
                .edges_added
                .iter()
                .filter_map(|id| wires.get(id))
                .map(|w| motion::flatten(&w.points, w.curved))
                .collect(),
            wires_removed: c
                .edges_removed
                .iter()
                .map(|w| motion::flatten(&w.points, w.curved))
                .collect(),
        })
    }

    /// The bar at the foot of the canvas: ask an agent for a change, wait for it, or
    /// accept or reject it.
    fn ask_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let bar = div()
            .id("arch-ask-bar")
            .max_w(px(640.))
            .p_1()
            .flex()
            .items_center()
            .gap_1()
            .rounded_lg()
            .border_1()
            .bg(float(c_canvas()))
            .shadow_lg()
            .text_size(px(12.5))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let broken_note = |b: &Option<String>| {
            b.as_ref().map(|e| {
                div()
                    .max_w(px(520.))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(float(c_canvas()))
                    .text_size(px(11.5))
                    .text_color(rgb(c_red()))
                    .child(format!(
                        "The files don't read yet, so this is the last good diagram. {}",
                        e.lines().next().unwrap_or_default()
                    ))
            })
        };
        let body = if let Some(r) = &self.review {
            let text = if r.changes.is_empty() {
                "The agent's change leaves this diagram as it was".to_string()
            } else {
                format!("AI change: {}", r.changes.summary())
            };
            div()
                .v_flex()
                .items_center()
                .gap_1()
                .child(
                    bar.border_color(rgb(c_blue()))
                        .child(icon(IconName::Sparkles, 14., c_blue()))
                        .child(
                            div()
                                .px_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(c_text()))
                                .child(text),
                        )
                        .child(
                            text_btn("arch-reject", "Reject", Some(IconName::Undo2), false)
                                .on_click(cx.listener(|this, _, _, cx| this.reject(cx))),
                        )
                        .child(
                            text_btn(
                                "arch-accept-tidy",
                                "Accept and beautify",
                                Some(IconName::WandSparkles),
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.accept(cx);
                                this.run_beautify(Beautify::Best, cx);
                            })),
                        )
                        .child(
                            text_btn("arch-accept", "Accept", Some(IconName::Check), true)
                                .on_click(cx.listener(|this, _, _, cx| this.accept(cx))),
                        ),
                )
                .children(broken_note(&r.broken))
        } else if let Some(w) = &self.waiting {
            div()
                .v_flex()
                .items_center()
                .gap_1()
                .child(
                    askbar::composer("arch-ask-bar", true).child(
                        askbar::working_row(
                            "arch-wait-spin",
                            thinking_line(self.working_for(), false).to_string(),
                            w.task.clone(),
                            w.started.elapsed().as_secs(),
                        )
                        .when(w.broken.is_some(), |el| {
                            el.child(
                                text_btn("arch-restore", "Restore", Some(IconName::Undo2), false)
                                    .on_click(cx.listener(|this, _, _, cx| this.reject(cx))),
                            )
                        })
                        .child(
                            text_btn("arch-stop-wait", "Stop", None, false)
                                .on_click(cx.listener(|this, _, _, cx| this.stop_waiting(cx))),
                        ),
                    ),
                )
                .children(broken_note(&w.broken))
        } else {
            let empty = self.f_ask.read(cx).value().trim().is_empty();
            let target = self.ask_target();
            let ideas: &[&'static str] = if target.is_some() {
                &[
                    "Add what this talks to",
                    "Split this into services",
                    "Put these in a boundary",
                ]
            } else if self.ed.cur().nodes.is_empty() {
                &[
                    "Draw a web app with an API and a database",
                    "Draw an event-driven system",
                ]
            } else {
                &[
                    "Add a cache and a queue",
                    "Group the parts by layer",
                    "Add monitoring",
                ]
            };
            let mut footer = div().flex().items_center().gap_1();
            let mut row = div().flex_1().min_w_0().flex().gap_1().overflow_hidden();
            if empty {
                for (i, idea) in ideas.iter().enumerate() {
                    let idea: &'static str = idea;
                    row = row.child(askbar::suggestion(("arch-idea", i), idea).on_click(
                        cx.listener(move |this, _, window, cx| this.fill_ask(idea, window, cx)),
                    ));
                }
            }
            footer = footer
                .child(row)
                .child(self.pick.chip("arch-ask", cx, |t: &mut Self| &mut t.pick))
                .child(
                    askbar::send_button("arch-ask-send", !empty)
                        .on_click(cx.listener(|this, _, window, cx| this.send_ask(window, cx))),
                );
            div()
                .v_flex()
                .items_end()
                .gap_1()
                .children(self.pick.menu(
                    "arch-ask",
                    "the diagram's files in architecture/",
                    cx,
                    |t: &mut Self| &mut t.pick,
                ))
                .child(
                    askbar::composer("arch-ask-bar", target.is_some())
                        .when_some(target, |el, text| {
                            el.child(
                                div().flex().child(
                                    askbar::target_chip(
                                        "arch-ask-target",
                                        IconName::Crosshair,
                                        text,
                                    )
                                    .tooltip(tip("AI changes what's selected. Click to clear."))
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.ed.sel = Sel::default();
                                            cx.notify();
                                        },
                                    )),
                                ),
                            )
                        })
                        .child(
                            Input::new(&self.f_ask)
                                .bordered(false)
                                .h(px(32.))
                                .prefix(icon(IconName::Sparkles, 14., c_blue())),
                        )
                        .child(footer),
                )
        };
        // A change under review sits at the top, clear of the status line and the map.
        div()
            .absolute()
            .when(self.review.is_some(), |el| el.top(px(10.)))
            .when(self.review.is_none(), |el| el.bottom(px(10.)))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(body)
            .into_any_element()
    }

    /// Names on the shapes the agent's change took away.
    fn removed_labels(&self) -> Vec<AnyElement> {
        let Some(r) = &self.review else {
            return Vec::new();
        };
        r.changes
            .removed
            .iter()
            .map(|(rect, name)| {
                let s = self.screen_rect(rect);
                div()
                    .absolute()
                    .left(px(s.x))
                    .top(px(s.y))
                    .w(px(s.w))
                    .h(px(s.h))
                    .p(px(6.))
                    .text_size(px(11.))
                    .text_color(col(c_red(), 0.9))
                    .child(format!("Removed: {}", name.replace('\n', " ")))
                    .into_any_element()
            })
            .collect()
    }

    // ---- Input

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if self.layer_edit.is_some() {
            self.commit_layer_name(cx);
            self.after_edit(cx);
        }
        self.menu = None;
        self.export_open = false;
        if self.editing.is_some() {
            let v = self.inline.read(cx).value().to_string();
            self.commit_inline(v, window, cx);
        }
        self.quick = None;
        self.quick_from = None;
        let local = self.local(e.position);
        let w = self.world(local);
        if e.button == MouseButton::Middle || self.tool == Tool::Hand || self.space {
            self.drag = Drag::Pan {
                from: local,
                pan: self.pan,
            };
            cx.notify();
            return;
        }
        let tol = 6. / self.zoom;
        // Bend handles of a single selected connector.
        if let Some((id, w0, straight)) = self.bendable() {
            for (seg, h) in geom::bend_handles(&w0.points).into_iter().enumerate() {
                if geom::dist(h, w) <= tol * 1.4 {
                    self.drag = Drag::Bend {
                        id,
                        seg,
                        from: w,
                        start: w0.points.clone(),
                        straight,
                        moved: false,
                    };
                    cx.notify();
                    return;
                }
            }
        }
        // Resize handles of a single selected shape.
        if let Some(id) = self.one_node() {
            if let Some(n) = self
                .ed
                .cur()
                .node(&id)
                .filter(|n| !self.ed.is_locked(n) && !n.collapsed)
            {
                let r = n.rect();
                for h in Handle::ALL {
                    if geom::dist(h.point(&r), w) <= tol * 1.4 {
                        self.drag = Drag::Resize {
                            id,
                            handle: h,
                            from: w,
                            start: r,
                            moved: false,
                        };
                        cx.notify();
                        return;
                    }
                }
            }
        }
        // Connection ports around the shape under the mouse.
        for id in self.hover.iter().chain(self.ed.sel.nodes.iter()) {
            if let Some(r) = self.ed.rect(id) {
                for s in Side::ALL {
                    if geom::dist(port(s, &r, self.zoom), w) <= tol * 1.6 {
                        self.drag = Drag::Connect {
                            from: id.clone(),
                            to: w,
                        };
                        cx.notify();
                        return;
                    }
                }
            }
        }
        let hit = self.ed.hit(w, tol);
        let add = e.modifiers.shift || e.modifiers.secondary();
        if e.click_count >= 2 {
            match hit {
                Hit::None => self.open_quick(w, None, window, cx),
                h => self.start_inline(h, window, cx),
            }
            return;
        }
        match &hit {
            Hit::Node(id) => {
                if add {
                    self.ed.select(&hit, true);
                } else if !self.ed.sel.has_node(id) {
                    self.ed.select(&hit, false);
                }
                // Alt-drag copies, as in draw.io and Figma.
                if e.modifiers.alt && self.ed.sel.has_node(id) {
                    self.ed.duplicate();
                    self.after_edit(cx);
                }
                let start = self
                    .ed
                    .movable()
                    .into_iter()
                    .filter_map(|id| self.ed.rect(&id).map(|r| (id, r)))
                    .collect();
                self.drag = Drag::Move {
                    from: w,
                    start,
                    moved: false,
                };
            }
            Hit::Edge(_) => self.ed.select(&hit, add),
            Hit::None => {
                if !add {
                    self.ed.sel = Sel::default();
                }
                self.drag = Drag::Marquee {
                    from: w,
                    to: w,
                    add,
                };
            }
        }
        cx.notify();
    }

    fn on_right(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let local = self.local(e.position);
        let w = self.world(local);
        let hit = self.ed.hit(w, 6. / self.zoom);
        let menu = match &hit {
            Hit::Node(id) => {
                if !self.ed.sel.has_node(id) {
                    self.ed.select(&hit, false);
                }
                Menu::Node(id.clone())
            }
            Hit::Edge(_) => {
                self.ed.select(&hit, false);
                Menu::Edge
            }
            Hit::None => Menu::Canvas(w),
        };
        self.menu = Some((local, menu));
        cx.notify();
    }

    fn on_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.drag, Drag::None) && e.pressed_button.is_none() {
            // The button came up outside the window.
            self.finish(e.position, cx);
            return;
        }
        let local = self.local(e.position);
        let w = self.world(local);
        self.pointer = self.inside(e.position).then_some(w);
        let zoom = self.zoom;
        match &mut self.drag {
            Drag::None => {
                let hover = if self.inside(e.position) {
                    match self.ed.hit(w, 6. / zoom) {
                        Hit::Node(id) => Some(id),
                        _ => self.hover.clone().filter(|id| {
                            self.ed
                                .rect(id)
                                .is_some_and(|r| r.inflate(14. / zoom).contains(w))
                        }),
                    }
                } else {
                    None
                };
                if hover != self.hover {
                    self.hover = hover;
                    cx.notify();
                }
                return;
            }
            Drag::Pan { from, pan } => {
                self.pan = (pan.0 + local.0 - from.0, pan.1 + local.1 - from.1);
            }
            Drag::Move { from, start, moved } => {
                let mut d = (w.0 - from.0, w.1 - from.1);
                if !*moved && (d.0.abs() * zoom < 3. && d.1.abs() * zoom < 3.) {
                    return;
                }
                let start = start.clone();
                let first = !*moved;
                *moved = true;
                if first {
                    self.ed.checkpoint();
                }
                self.guides.clear();
                let moving: Vec<&String> = start.iter().map(|(id, _)| id).collect();
                if self.snap && !e.modifiers.alt {
                    if let Some(bb) = Rect::bounds(start.iter().map(|(_, r)| *r)) {
                        let shifted = Rect::new(bb.x + d.0, bb.y + d.1, bb.w, bb.h);
                        let others: Vec<Rect> = self
                            .ed
                            .cur()
                            .nodes
                            .iter()
                            .filter(|n| !moving.contains(&&n.id))
                            .map(|n| n.rect())
                            .collect();
                        let ((sx, sy), guides) =
                            geom::snap(&shifted, &others, 6. / zoom, self.grid.then_some(GRID));
                        d = (d.0 + sx, d.1 + sy);
                        self.guides = guides;
                    }
                }
                let rects: Vec<(String, Rect)> = start
                    .iter()
                    .map(|(id, r)| (id.clone(), Rect::new(r.x + d.0, r.y + d.1, r.w, r.h)))
                    .collect();
                self.ed.set_rects(&rects);
            }
            Drag::Resize {
                id,
                handle,
                from,
                start,
                moved,
            } => {
                let d = (w.0 - from.0, w.1 - from.1);
                let mut r = handle.resize(start, d, 20.);
                if self.snap && self.grid && !e.modifiers.alt {
                    let snap = |v: f32| (v / GRID).round() * GRID;
                    let (x1, y1) = (snap(r.right()), snap(r.bottom()));
                    r.x = snap(r.x);
                    r.y = snap(r.y);
                    r.w = (x1 - r.x).max(20.);
                    r.h = (y1 - r.y).max(20.);
                }
                if e.modifiers.shift {
                    // Keep the shape's proportions.
                    let ratio = start.w / start.h.max(1.);
                    r.h = r.w / ratio;
                }
                let id = id.clone();
                if !*moved {
                    *moved = true;
                    self.ed.checkpoint();
                }
                self.ed.set_rects(&[(id, r)]);
            }
            Drag::Bend {
                id,
                seg,
                from,
                start,
                straight,
                moved,
            } => {
                if !*moved && geom::dist(*from, w) * zoom < 3. {
                    return;
                }
                if !*moved {
                    *moved = true;
                    self.ed.checkpoint();
                }
                let mut at = w;
                if self.snap && self.grid && !e.modifiers.alt {
                    at = ((at.0 / GRID).round() * GRID, (at.1 / GRID).round() * GRID);
                }
                let pts = geom::bend(start, *seg, at, *straight);
                let id = id.clone();
                self.ed.set_waypoints(&id, pts);
            }
            Drag::Marquee { to, .. } => *to = w,
            Drag::Connect { to, .. } => *to = w,
            Drag::Place { at, .. } | Drag::Picture { at, .. } => *at = Some(e.position),
        }
        cx.notify();
    }

    fn finish(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let w = self.world(self.local(pos));
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::Move { start, moved, .. } => {
                if moved {
                    let ids: Vec<String> = self.ed.sel.nodes.clone();
                    self.ed.adopt(&ids);
                    let _ = start;
                    self.after_edit(cx);
                }
            }
            Drag::Resize { id, moved, .. } => {
                if moved {
                    if self.ed.kind_of(&id) == Kind::Boundary {
                        let all: Vec<String> =
                            self.ed.cur().nodes.iter().map(|n| n.id.clone()).collect();
                        self.ed.adopt(&all);
                    } else {
                        self.ed.adopt(&[id]);
                    }
                    self.after_edit(cx);
                }
            }
            Drag::Marquee { from, to, add } => {
                let r = Rect::span(from, to);
                if r.w * self.zoom > 3. || r.h * self.zoom > 3. {
                    self.ed.select_area(r, add);
                }
            }
            Drag::Connect { from, to } => {
                match self.ed.hit(to, 6. / self.zoom) {
                    Hit::Node(target) => {
                        if self.ed.connect(&from, &target).is_some() {
                            self.after_edit(cx);
                        }
                    }
                    _ => {
                        // Let go on empty canvas: pick a shape to add there, already connected.
                        let fr = self.ed.rect(&from);
                        if fr.is_some_and(|r| !r.inflate(20.).contains(to)) {
                            self.pending_quick = Some((to, from));
                        }
                    }
                }
            }
            Drag::Place { preset, at } => {
                let at = at
                    .filter(|p| self.inside(*p))
                    .map(|p| self.world(self.local(p)))
                    .unwrap_or_else(|| self.ed.free_spot(self.center_world(), preset.kind.size()));
                self.add_preset(preset, at, cx);
            }
            Drag::Picture { file, name, at } => {
                let at = at
                    .filter(|p| self.inside(*p))
                    .map(|p| self.world(self.local(p)));
                self.place_picture(&file, &name, at, cx);
            }
            Drag::Bend { moved, .. } => {
                if moved {
                    self.after_edit(cx);
                }
            }
            Drag::Pan { .. } | Drag::None => {}
        }
        let _ = w;
        self.guides.clear();
        cx.notify();
    }

    fn on_scroll(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let d = e.delta.pixel_delta(px(20.));
        let (dx, dy) = (f32::from(d.x), f32::from(d.y));
        if e.modifiers.secondary() || e.modifiers.control {
            let local = self.local(e.position);
            self.zoom_at(local, (dy * 0.004).exp());
        } else if e.modifiers.shift && dx.abs() < 0.1 {
            self.pan.0 += dy;
        } else {
            self.pan = (self.pan.0 + dx, self.pan.1 + dy);
        }
        self.menu = None;
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let k = &e.keystroke;
        if self.key(k.key.as_str(), &k.modifiers, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Keys the app or the window root also bind (Escape, Tab, copy) arrive as actions.
    fn on_bound_key(
        &mut self,
        key: &str,
        m: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focus.is_focused(window) {
            self.key(key, &m, window, cx);
        } else if key == "escape" {
            // Leave a field in the side panel for the canvas.
            window.focus(&self.focus, cx);
        } else {
            cx.propagate();
        }
    }

    /// Run a key on the canvas; false when it isn't one.
    fn key(
        &mut self,
        key: &str,
        m: &Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let sec = m.secondary();
        let mut handled = true;
        match (sec, m.shift, m.alt, key) {
            (_, _, _, "escape") => {
                if self.find_open {
                    self.close_find(window, cx);
                } else if !matches!(self.drag, Drag::None) {
                    self.drag = Drag::None;
                } else if self.menu.is_some()
                    || self.quick.is_some()
                    || self.export_open
                    || self.beautify_open
                {
                    self.menu = None;
                    self.quick = None;
                    self.export_open = false;
                    self.beautify_open = false;
                } else {
                    self.ed.sel = Sel::default();
                }
            }
            (false, _, _, "backspace") | (false, _, _, "delete") => self.act(cx, |ed| ed.delete()),
            (false, shift, _, "left" | "right" | "up" | "down") => {
                let step = if shift { 10. } else { 1. };
                let d = match key {
                    "left" => (-step, 0.),
                    "right" => (step, 0.),
                    "up" => (0., -step),
                    _ => (0., step),
                };
                if self.ed.sel.nodes.is_empty() {
                    self.pan = (self.pan.0 - d.0 * 20., self.pan.1 - d.1 * 20.);
                } else {
                    self.act(cx, |ed| ed.nudge(d));
                }
            }
            (true, false, false, "z") => self.undo(cx),
            (true, true, false, "z") | (true, false, false, "y") => self.redo(cx),
            (true, false, false, "c") => {
                self.ed.copy();
            }
            (true, false, false, "x") => {
                if self.ed.copy() {
                    self.act(cx, |ed| ed.delete());
                }
            }
            (true, false, false, "v") => {
                if self.ed.can_paste() {
                    self.act(cx, |ed| ed.paste());
                }
            }
            (true, false, false, "d") => self.act(cx, |ed| ed.duplicate()),
            (true, false, false, "a") => self.ed.select_all(),
            (true, false, false, "g") => self.act(cx, |ed| {
                ed.group();
            }),
            (true, true, false, "g") => self.act(cx, |ed| ed.ungroup()),
            (true, true, false, "l") => self.act(cx, |ed| ed.toggle_lock()),
            (true, false, false, "]") => self.act(cx, |ed| ed.order(true)),
            (true, false, false, "[") => self.act(cx, |ed| ed.order(false)),
            (true, false, true, "c") => self.ed.copy_style(),
            (true, false, true, "v") => self.act(cx, |ed| ed.paste_style()),
            (true, false, false, "=" | "+") => self.zoom_step(1.2),
            (true, false, false, "-") => self.zoom_step(1. / 1.2),
            (false, true, false, "1" | "!") => self.fit(false),
            (false, true, false, "2" | "@") => self.fit(true),
            (false, true, false, "0" | ")") => {
                let (w, h) = self.view_size();
                self.zoom_at((w / 2., h / 2.), 1. / self.zoom);
            }
            (false, true, false, "a") => self.run_beautify(Beautify::Best, cx),
            (false, false, false, "v") => self.tool = Tool::Select,
            (false, false, false, "h") => self.tool = Tool::Hand,
            (false, false, false, "space") => self.space = true,
            (false, false, false, "enter") => {
                if let Some(id) = self.one_node() {
                    self.start_inline(Hit::Node(id), window, cx);
                } else if let Some(id) = self.one_edge() {
                    self.start_inline(Hit::Edge(id), window, cx);
                }
            }
            (false, false, false, "tab" | "/") => {
                let at = self
                    .one_node()
                    .and_then(|id| self.ed.rect(&id))
                    .map(|r| {
                        if self.horizontal {
                            (r.right() + 160., r.y + r.h / 2.)
                        } else {
                            (r.x + r.w / 2., r.bottom() + 130.)
                        }
                    })
                    .unwrap_or_else(|| self.center_world());
                let from = if key == "tab" { self.one_node() } else { None };
                self.open_quick(at, from, window, cx);
            }
            (false, false, false, letter) if shortcut(letter).is_some() => {
                if let Some(p) = shortcut(letter) {
                    let at = self.pointer.unwrap_or_else(|| self.center_world());
                    self.add_preset(p, at, cx);
                }
            }
            _ => handled = false,
        }
        if handled {
            cx.notify();
        }
        handled
    }
}

/// Single-key shortcuts that add a shape, as in Figma.
fn shortcut(key: &str) -> Option<&'static Preset> {
    let label = match key {
        "r" => "Container",
        "o" => "Ellipse",
        "t" => "Text",
        "n" => "Sticky note",
        "b" => "Boundary",
        "p" => "Person",
        "d" => "Database",
        "q" => "Queue",
        _ => return None,
    };
    PRESETS.iter().find(|p| p.label == label)
}

/// Where a connection port sits: just outside the middle of each side.
fn port(side: Side, r: &Rect, zoom: f32) -> Pt {
    let (x, y) = side.point(r);
    let off = 12. / zoom;
    match side {
        Side::Top => (x, y - off),
        Side::Right => (x + off, y),
        Side::Bottom => (x, y + off),
        Side::Left => (x - off, y),
    }
}

// ---- Drawing

/// A shape ready to paint, in diagram space.
#[derive(Clone)]
struct ShapeDraw {
    fill: Vec<Seg>,
    detail: Vec<Seg>,
    /// Filled marks over the outline, with their colours.
    marks: Vec<(Vec<Seg>, u32)>,
    shadow: bool,
    fill_c: Option<u32>,
    stroke: Option<u32>,
    opacity: f32,
    dashed: bool,
    width: f32,
    rect: Rect,
    effect: Effect,
    radius: f32,
}

#[derive(Clone)]
struct WireDraw {
    pts: Vec<Pt>,
    curved: bool,
    color: u32,
    dashed: bool,
    width: f32,
    start: bool,
    end: bool,
    motion: Motion,
    speed: Speed,
    /// Drawn faintly: another part of the diagram is playing.
    dim: bool,
}

/// The step of a story on screen: the connector a packet travels and how far it got.
#[derive(Clone)]
struct StoryDraw {
    line: Vec<Pt>,
    progress: f32,
    lit: Vec<Rect>,
}

/// Everything the canvas paints in one frame.
#[derive(Clone)]
struct Scene {
    pan: Pt,
    zoom: f32,
    bg: u32,
    grid: Option<(u32, u32)>,
    under: Vec<ShapeDraw>,
    wires: Vec<WireDraw>,
    over: Vec<ShapeDraw>,
    accent: u32,
    selected: Vec<Rect>,
    locked: Vec<Rect>,
    handles: Vec<Pt>,
    ports: Vec<Pt>,
    guides: Vec<Guide>,
    marquee: Option<Rect>,
    connect: Option<WireDraw>,
    target: Option<Rect>,
    /// Seconds on the animation clock, and whether anything moves.
    t: f32,
    moving: bool,
    story: Option<StoryDraw>,
    /// Connectors on a dependency cycle, for the status line.
    cycles: usize,
    /// Handles in the middle of each segment of the selected connector.
    bends: Vec<Pt>,
    /// Shapes matching the find bar, and the one shown now.
    found: Vec<Rect>,
    found_now: Option<Rect>,
    review: Option<ReviewDraw>,
}

/// An agent's change marked on the canvas: new shapes green, changed ones amber, the
/// ones it took away and where moved ones were as dashed ghosts.
#[derive(Clone)]
struct ReviewDraw {
    added: Vec<Rect>,
    edited: Vec<Rect>,
    removed: Vec<Rect>,
    moved: Vec<Rect>,
    wires_added: Vec<Vec<Pt>>,
    wires_removed: Vec<Vec<Pt>>,
}

/// Below this zoom connectors stop moving: the motion is too small to see.
const MOTION_MIN_ZOOM: f32 = 0.25;
/// Below this zoom a box shows only its name, not its kind, technology or description.
const DETAIL_MIN_ZOOM: f32 = 0.4;

/// The area a connector covers, for skipping the ones off screen.
fn wire_box(w: &geom::Wire) -> Rect {
    Rect::bounds(w.points.iter().map(|p| Rect::new(p.0, p.1, 0., 0.)))
        .unwrap_or_default()
        .inflate(1.)
}

pub(crate) fn col(c: u32, a: f32) -> Rgba {
    rgba((c << 8) | ((a.clamp(0., 1.) * 255.) as u32))
}

fn wire_segs(w: &WireDraw) -> Vec<Seg> {
    let mut out = Vec::new();
    let Some(&(x, y)) = w.pts.first() else {
        return out;
    };
    out.push(Seg::Move(x, y));
    if w.curved && w.pts.len() == 4 {
        let p = &w.pts;
        out.push(Seg::Cubic(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1));
    } else {
        for &(x, y) in &w.pts[1..] {
            out.push(Seg::Line(x, y));
        }
    }
    out
}

fn build_path(
    segs: &[Seg],
    tf: &dyn Fn(Pt) -> Point<Pixels>,
    mut b: PathBuilder,
) -> Option<Path<Pixels>> {
    if segs.is_empty() {
        return None;
    }
    for s in segs {
        match *s {
            Seg::Move(x, y) => b.move_to(tf((x, y))),
            Seg::Line(x, y) => b.line_to(tf((x, y))),
            Seg::Cubic(a, c, d, e, x, y) => b.cubic_bezier_to(tf((x, y)), tf((a, c)), tf((d, e))),
            Seg::Close => b.close(),
        }
    }
    b.build().ok()
}

fn stroke_builder(width: f32, dashed: bool, zoom: f32) -> PathBuilder {
    let b = PathBuilder::stroke(px(width));
    if dashed {
        b.dash_array(&[px(6. * zoom.max(0.4)), px(4. * zoom.max(0.4))])
    } else {
        b
    }
}

fn paint_shape(s: &ShapeDraw, tf: &dyn Fn(Pt) -> Point<Pixels>, zoom: f32, window: &mut Window) {
    if s.shadow && s.fill_c.is_some() {
        let drop = |p: Pt| tf((p.0, p.1 + shape::SHADOW_DROP));
        if let Some(p) = build_path(&s.fill, &drop, PathBuilder::fill()) {
            window.paint_path(p, col(0x0f172a, shape::SHADOW_ALPHA * s.opacity));
        }
    }
    if let Some(c) = s.fill_c {
        if let Some(p) = build_path(&s.fill, tf, PathBuilder::fill()) {
            window.paint_path(p, col(c, s.opacity));
        }
    }
    if let Some(c) = s.stroke {
        let w = (s.width * zoom).max(1.);
        if let Some(p) = build_path(&s.fill, tf, stroke_builder(w, s.dashed, zoom)) {
            window.paint_path(p, col(c, s.opacity));
        }
        if let Some(p) = build_path(&s.detail, tf, stroke_builder(w, false, zoom)) {
            window.paint_path(p, col(c, s.opacity));
        }
    }
    for (segs, c) in &s.marks {
        if let Some(p) = build_path(segs, tf, PathBuilder::fill()) {
            window.paint_path(p, col(*c, s.opacity));
        }
    }
}

fn arrowhead(from: Pt, tip: Pt, size: f32) -> Vec<Seg> {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / len, dy / len);
    let base = (tip.0 - ux * size, tip.1 - uy * size);
    let half = size * 0.45;
    vec![
        Seg::Move(tip.0, tip.1),
        Seg::Line(base.0 - uy * half, base.1 + ux * half),
        Seg::Line(base.0 + uy * half, base.1 - ux * half),
        Seg::Close,
    ]
}

fn paint_wire(
    w: &WireDraw,
    tf: &dyn Fn(Pt) -> Point<Pixels>,
    zoom: f32,
    alpha: f32,
    window: &mut Window,
) {
    let width = (w.width * zoom).max(1.);
    let size = (10. * zoom).max(6.) / zoom;
    // Stop the line short of the arrow's tip so the tip stays sharp.
    let mut pts = w.pts.clone();
    let shorten = |pts: &mut Vec<Pt>, at_end: bool| {
        let n = pts.len();
        if n < 2 {
            return;
        }
        let (i, j) = if at_end { (n - 1, n - 2) } else { (0, 1) };
        let (a, b) = (pts[i], pts[j]);
        let d = geom::dist(a, b).max(0.001);
        let t = (size * 0.8 / d).min(0.5);
        pts[i] = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    };
    let wire = geom::Wire {
        points: w.pts.clone(),
        curved: w.curved,
    };
    if !w.curved {
        if w.end {
            shorten(&mut pts, true);
        }
        if w.start {
            shorten(&mut pts, false);
        }
    }
    let draw = WireDraw { pts, ..w.clone() };
    if let Some(p) = build_path(&wire_segs(&draw), tf, stroke_builder(width, w.dashed, zoom)) {
        window.paint_path(p, col(w.color, alpha));
    }
    if w.end {
        if let Some((from, tip)) = wire.end_dir() {
            if let Some(p) = build_path(&arrowhead(from, tip, size), tf, PathBuilder::fill()) {
                window.paint_path(p, col(w.color, alpha));
            }
        }
    }
    if w.start {
        if let Some((from, tip)) = wire.start_dir() {
            if let Some(p) = build_path(&arrowhead(from, tip, size), tf, PathBuilder::fill()) {
                window.paint_path(p, col(w.color, alpha));
            }
        }
    }
}

/// A line through `pts`, stroked.
fn paint_line(
    pts: &[Pt],
    tf: &dyn Fn(Pt) -> Point<Pixels>,
    width: f32,
    color: Rgba,
    window: &mut Window,
) {
    if pts.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(tf(pts[0]));
    for p in &pts[1..] {
        b.line_to(tf(*p));
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// A filled circle around `c` (in diagram units) with radius `r` (in pixels).
fn paint_dot(c: Pt, r: f32, tf: &dyn Fn(Pt) -> Point<Pixels>, color: Rgba, window: &mut Window) {
    let p = tf(c);
    window.paint_quad(quad(
        Bounds::new(
            point(p.x - px(r), p.y - px(r)),
            size(px(2. * r), px(2. * r)),
        ),
        px(r),
        color,
        px(0.),
        color,
        BorderStyle::Solid,
    ));
}

/// What moves on a connector at time `t`, over its still line.
fn paint_motion(
    w: &WireDraw,
    t: f32,
    tf: &dyn Fn(Pt) -> Point<Pixels>,
    zoom: f32,
    window: &mut Window,
) {
    let line = motion::flatten(&w.pts, w.curved);
    let bright = shape::mix(w.color, 0xffffff, 0.15);
    match w.motion {
        Motion::Flow => {
            let width = ((w.width + 0.8) * zoom).max(1.5);
            for dash in motion::dashes(&line, t, w.speed) {
                paint_line(&dash, tf, width, col(bright, 1.), window);
            }
        }
        Motion::Packets => {
            let len = motion::length(&line);
            let r = (4.2 * zoom).clamp(2.6, 8.);
            for s in motion::packets(len, t, w.speed) {
                paint_dot(motion::at(&line, s), r + 2.5, tf, col(bright, 0.25), window);
                paint_dot(motion::at(&line, s), r, tf, col(bright, 1.), window);
            }
            if w.start && w.end {
                let spacing = len / (len / motion::PACKET_SPACING).ceil().max(1.);
                for s in motion::packets(len, t + spacing / 2. / w.speed.rate(), w.speed) {
                    paint_dot(motion::at(&line, len - s), r, tf, col(bright, 0.8), window);
                }
            }
        }
        Motion::Pulse => {
            let p = motion::pulse(t * w.speed.rate() / Speed::Normal.rate());
            let width = ((w.width + 5. * p) * zoom).max(1.);
            paint_line(&line, tf, width, col(w.color, 0.35 * p), window);
        }
        Motion::None => {}
    }
}

/// A shape's effect at time `t`: a breathing glow, a blinking ring.
fn paint_effect(
    sh: &ShapeDraw,
    t: f32,
    accent: u32,
    tf: &dyn Fn(Pt) -> Point<Pixels>,
    zoom: f32,
    window: &mut Window,
) {
    let ring = |grow: f32, width: f32, color: Rgba, window: &mut Window| {
        let p = tf((sh.rect.x, sh.rect.y));
        let g = grow * zoom;
        window.paint_quad(quad(
            Bounds::new(
                point(p.x - px(g), p.y - px(g)),
                size(px(sh.rect.w * zoom + 2. * g), px(sh.rect.h * zoom + 2. * g)),
            ),
            px((sh.radius + grow) * zoom),
            col(0, 0.),
            px(width),
            color,
            BorderStyle::Solid,
        ));
    };
    let color = sh.stroke.unwrap_or(accent);
    match sh.effect {
        Effect::Pulse => {
            let p = motion::pulse(t);
            ring(
                3. + 9. * p,
                (2.5 * zoom).max(1.),
                col(color, 0.55 * (1. - p) + 0.1),
                window,
            );
        }
        Effect::Blink => {
            if motion::blink(t) {
                ring(5., (2.5 * zoom).max(1.5), col(0xef4444, 0.95), window);
            }
        }
        Effect::Spin => {
            let o = motion::outline(sh.rect.x, sh.rect.y, sh.rect.w, sh.rect.h, 5.);
            let (head, tail) = motion::orbit(&o, t);
            let width = (2.5 * zoom).max(1.5);
            paint_line(
                &motion::slice(&o, tail, head),
                tf,
                width,
                col(color, 0.9),
                window,
            );
            paint_dot(
                motion::at(&o, head),
                width * 1.6,
                tf,
                col(color, 1.),
                window,
            );
        }
        Effect::None => {}
    }
}

fn paint_scene(b: Bounds<Pixels>, s: &Scene, window: &mut Window) {
    let (ox, oy) = (f32::from(b.origin.x), f32::from(b.origin.y));
    let (pan, zoom) = (s.pan, s.zoom);
    let tf = move |p: Pt| point(px(ox + p.0 * zoom + pan.0), px(oy + p.1 * zoom + pan.1));
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    if let Some((minor, major)) = s.grid {
        let mut step = 20. * zoom;
        while step < 10. {
            step *= 5.;
        }
        let world_step = step / zoom;
        let first = |o: f32| (-o / step).floor() as i64;
        let (fx, fy) = (first(pan.0), first(pan.1));
        let mut i = fx;
        loop {
            let x = pan.0 + i as f32 * step;
            if x > w {
                break;
            }
            let is_major = ((i as f32 * world_step) / 100.).fract().abs() < 0.001;
            window.paint_quad(fill(
                Bounds::new(point(px(ox + x), px(oy)), size(px(1.), px(h))),
                rgb(if is_major { major } else { minor }),
            ));
            i += 1;
        }
        let mut j = fy;
        loop {
            let y = pan.1 + j as f32 * step;
            if y > h {
                break;
            }
            let is_major = ((j as f32 * world_step) / 100.).fract().abs() < 0.001;
            window.paint_quad(fill(
                Bounds::new(point(px(ox), px(oy + y)), size(px(w), px(1.))),
                rgb(if is_major { major } else { minor }),
            ));
            j += 1;
        }
    }
    for sh in &s.under {
        paint_shape(sh, &tf, zoom, window);
    }
    for wd in &s.wires {
        let moves = s.moving && !wd.dim && wd.motion != Motion::None;
        let alpha = if wd.dim {
            0.2
        } else if moves && wd.motion == Motion::Flow {
            0.4
        } else {
            1.
        };
        paint_wire(wd, &tf, zoom, alpha, window);
        if moves {
            paint_motion(wd, s.t, &tf, zoom, window);
        }
    }
    for sh in &s.over {
        paint_shape(sh, &tf, zoom, window);
    }
    if s.moving {
        for sh in s.under.iter().chain(&s.over) {
            if sh.effect != Effect::None && sh.opacity > 0.5 {
                paint_effect(sh, s.t, s.accent, &tf, zoom, window);
            }
        }
    }
    if let Some(st) = &s.story {
        for r in &st.lit {
            let p = tf((r.x, r.y));
            window.paint_quad(quad(
                Bounds::new(
                    point(p.x - px(5.), p.y - px(5.)),
                    size(px(r.w * zoom + 10.), px(r.h * zoom + 10.)),
                ),
                px(10.),
                col(0, 0.),
                px(2.5),
                col(0xf59e0b, 0.9),
                BorderStyle::Solid,
            ));
        }
        let len = motion::length(&st.line);
        let head = len * motion::ease(st.progress);
        paint_line(
            &motion::slice(&st.line, 0., head),
            &tf,
            (3. * zoom).max(2.),
            col(0xf59e0b, 0.9),
            window,
        );
        let r = (7. * zoom).clamp(4., 12.);
        paint_dot(
            motion::at(&st.line, head),
            r + 3.,
            &tf,
            col(0xf59e0b, 0.3),
            window,
        );
        paint_dot(
            motion::at(&st.line, head),
            r,
            &tf,
            col(0xf59e0b, 1.),
            window,
        );
    }
    let rect_bounds = |r: &Rect, grow: f32| {
        let p = tf((r.x, r.y));
        Bounds::new(
            point(p.x - px(grow), p.y - px(grow)),
            size(px(r.w * zoom + 2. * grow), px(r.h * zoom + 2. * grow)),
        )
    };
    if let Some(t) = &s.target {
        window.paint_quad(quad(
            rect_bounds(t, 4.),
            px(8.),
            col(s.accent, 0.08),
            px(2.),
            col(s.accent, 1.),
            BorderStyle::Solid,
        ));
    }
    for r in &s.selected {
        window.paint_quad(quad(
            rect_bounds(r, 3.),
            px(4.),
            col(s.accent, 0.),
            px(1.5),
            col(s.accent, 1.),
            BorderStyle::Solid,
        ));
    }
    for r in &s.locked {
        window.paint_quad(quad(
            rect_bounds(r, 3.),
            px(4.),
            col(s.accent, 0.),
            px(1.5),
            col(0x94a3b8, 1.),
            BorderStyle::Dashed,
        ));
    }
    if let Some(rv) = &s.review {
        paint_review(rv, &tf, zoom, window);
    }
    for (i, r) in s.found.iter().enumerate() {
        let now = s.found_now.as_ref() == Some(r);
        let p = tf((r.x, r.y));
        let grow = if now { 6. } else { 4. };
        window.paint_quad(quad(
            Bounds::new(
                point(p.x - px(grow), p.y - px(grow)),
                size(px(r.w * zoom + 2. * grow), px(r.h * zoom + 2. * grow)),
            ),
            px(8.),
            col(0xf59e0b, if now { 0.12 } else { 0.05 }),
            px(if now { 2.5 } else { 1.5 }),
            col(0xf59e0b, if now { 1. } else { 0.7 }),
            BorderStyle::Solid,
        ));
        let _ = i;
    }
    for p in &s.bends {
        let c = tf(*p);
        window.paint_quad(quad(
            Bounds::new(point(c.x - px(4.5), c.y - px(4.5)), size(px(9.), px(9.))),
            px(1.5),
            col(s.accent, 1.),
            px(1.5),
            rgb(0xffffff),
            BorderStyle::Solid,
        ));
    }
    for p in &s.handles {
        let c = tf(*p);
        window.paint_quad(quad(
            Bounds::new(point(c.x - px(4.), c.y - px(4.)), size(px(8.), px(8.))),
            px(2.),
            rgb(0xffffff),
            px(1.5),
            col(s.accent, 1.),
            BorderStyle::Solid,
        ));
    }
    for p in &s.ports {
        let c = tf(*p);
        window.paint_quad(quad(
            Bounds::new(point(c.x - px(5.), c.y - px(5.)), size(px(10.), px(10.))),
            px(5.),
            col(s.bg, 1.),
            px(1.5),
            col(s.accent, 1.),
            BorderStyle::Solid,
        ));
    }
    for g in &s.guides {
        let (a, c) = (tf(g.from), tf(g.to));
        let bounds = if (g.from.0 - g.to.0).abs() < 0.5 {
            Bounds::new(
                point(a.x, a.y.min(c.y)),
                size(px(1.), (c.y - a.y).abs().max(px(1.))),
            )
        } else {
            Bounds::new(
                point(a.x.min(c.x), a.y),
                size((c.x - a.x).abs().max(px(1.)), px(1.)),
            )
        };
        window.paint_quad(fill(bounds, rgb(0xec4899)));
    }
    if let Some(wd) = &s.connect {
        paint_wire(wd, &tf, zoom, 1., window);
    }
    if let Some(m) = &s.marquee {
        window.paint_quad(quad(
            rect_bounds(m, 0.),
            px(2.),
            col(s.accent, 0.08),
            px(1.),
            col(s.accent, 0.7),
            BorderStyle::Solid,
        ));
    }
}

fn paint_review(rv: &ReviewDraw, tf: &dyn Fn(Pt) -> Point<Pixels>, zoom: f32, window: &mut Window) {
    const GREEN: u32 = 0x16a34a;
    const AMBER: u32 = 0xf59e0b;
    const RED: u32 = 0xdc2626;
    for line in &rv.wires_removed {
        let mut segs = Vec::with_capacity(line.len());
        for (i, p) in line.iter().enumerate() {
            segs.push(if i == 0 {
                Seg::Move(p.0, p.1)
            } else {
                Seg::Line(p.0, p.1)
            });
        }
        if let Some(p) = build_path(&segs, tf, stroke_builder(2., true, zoom)) {
            window.paint_path(p, col(RED, 0.75));
        }
    }
    for line in &rv.wires_added {
        paint_line(line, tf, (6. * zoom).max(3.), col(GREEN, 0.3), window);
    }
    let ring =
        |r: &Rect, grow: f32, fill_: Rgba, line: Rgba, style: BorderStyle, window: &mut Window| {
            let p = tf((r.x, r.y));
            window.paint_quad(quad(
                Bounds::new(
                    point(p.x - px(grow), p.y - px(grow)),
                    size(px(r.w * zoom + 2. * grow), px(r.h * zoom + 2. * grow)),
                ),
                px(8.),
                fill_,
                px(2.),
                line,
                style,
            ));
        };
    for r in &rv.moved {
        ring(
            r,
            0.,
            col(0x94a3b8, 0.04),
            col(0x94a3b8, 0.9),
            BorderStyle::Dashed,
            window,
        );
    }
    for r in &rv.removed {
        ring(
            r,
            0.,
            col(RED, 0.06),
            col(RED, 0.8),
            BorderStyle::Dashed,
            window,
        );
    }
    for r in &rv.edited {
        ring(
            r,
            5.,
            col(AMBER, 0.),
            col(AMBER, 1.),
            BorderStyle::Solid,
            window,
        );
    }
    for r in &rv.added {
        ring(
            r,
            5.,
            col(GREEN, 0.06),
            col(GREEN, 1.),
            BorderStyle::Solid,
            window,
        );
    }
}

/// A Lucide icon by its file name, e.g. "database".
pub(crate) fn icon_named(name: &str) -> Option<IconName> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static MAP: OnceLock<HashMap<String, IconName>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        IconName::ALL
            .iter()
            .map(|i| {
                let path = i.path().to_string();
                let stem = path
                    .trim_start_matches("icons/")
                    .trim_end_matches(".svg")
                    .to_string();
                (stem, *i)
            })
            .collect()
    });
    map.get(name).copied()
}

fn kind_icon(kind: Kind) -> IconName {
    let name = match kind {
        Kind::Person => "user",
        Kind::System => "boxes",
        Kind::Container => "box",
        Kind::Component => "component",
        Kind::Database => "database",
        Kind::Queue => "list-ordered",
        Kind::External => "globe",
        Kind::Boundary => "frame",
        Kind::Box | Kind::Rounded => "square",
        Kind::Ellipse => "circle",
        Kind::Diamond => "diamond",
        Kind::Cloud => "cloud",
        Kind::Note => "sticky-note",
        Kind::Text => "type",
        Kind::Icon => "image",
    };
    icon_named(name).unwrap_or(IconName::Square)
}

pub(crate) fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |window, cx| gpui_kit::component::tooltip::Tooltip::new(text).build(window, cx)
}

pub(crate) fn icon_btn(
    id: impl Into<ElementId>,
    name: IconName,
    tooltip: &'static str,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .when(on, |el| el.bg(tint(c_selected())))
        .hover(|el| el.bg(tint(c_subtle())))
        .tooltip(tip(tooltip))
        .child(icon(name, 15., if on { c_text() } else { c_secondary() }))
}

pub(crate) fn text_btn(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    name: Option<IconName>,
    primary: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .rounded_md()
        .cursor_pointer()
        .text_size(px(12.5))
        .when(primary, |el| {
            el.bg(rgb(c_blue()))
                .text_color(rgb(0xffffff))
                .hover(|el| el.opacity(0.9))
        })
        .when(!primary, |el| {
            el.border_1()
                .border_color(rgb(c_border()))
                .text_color(rgb(c_text()))
                .hover(|el| el.bg(tint(c_subtle())))
        })
        .when_some(name, |el, n| {
            el.child(icon(n, 13., if primary { 0xffffff } else { c_secondary() }))
        })
        .child(label.into())
}

pub(crate) fn chip_btn(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.))
        .px_2()
        .flex()
        .items_center()
        .rounded_md()
        .cursor_pointer()
        .text_size(px(11.5))
        .border_1()
        .border_color(rgb(if on { c_blue() } else { c_border() }))
        .bg(if on {
            tint(c_selected())
        } else {
            tint(c_canvas())
        })
        .text_color(rgb(if on { c_text() } else { c_secondary() }))
        .hover(|el| el.bg(tint(c_subtle())))
        .child(label.into())
}

pub(crate) fn section(title: &'static str) -> Div {
    div()
        .pt_3()
        .pb_1()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_muted()))
        .child(title.to_uppercase())
}

pub(crate) fn label_row(label: &'static str, body: impl IntoElement) -> Div {
    div()
        .v_flex()
        .gap(px(3.))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(c_secondary()))
                .child(label),
        )
        .child(body)
}

impl ArchEditor {
    fn theme(&self) -> Theme {
        Theme {
            canvas: c_canvas(),
            text: c_text(),
            muted: c_muted(),
            dark: is_dark(),
        }
    }

    fn scene(&self, theme: &Theme) -> Scene {
        let v = self.ed.cur();
        let accent = c_blue();
        let t = self.born.elapsed().as_secs_f32();
        // A story playing: the edges and shapes it involves, and the step on now.
        let story = self.playing.and_then(|p| {
            let st = v.stories.get(p.story)?;
            let mut edges = std::collections::HashSet::new();
            let mut nodes = std::collections::HashSet::new();
            for step in &st.steps {
                if let Some(r) = self.ed.doc.relation(&step.edge) {
                    edges.insert(step.edge.clone());
                    nodes.insert(r.from.clone());
                    nodes.insert(r.to.clone());
                }
            }
            let now = motion::step_at(p.elapsed(), st.steps.len());
            Some((edges, nodes, now))
        });
        let mut under = Vec::new();
        let mut over = Vec::new();
        let els = self.ed.elements();
        let vis = self.visible_world();
        let sight = self.ed.sight();
        for n in &v.nodes {
            if !n.rect().intersects(&vis) || !sight.shows(&n.id) {
                continue;
            }
            let Some(e) = els.get(n.id.as_str()) else {
                continue;
            };
            let o = shape::outline(e.kind, &n.rect(), &n.style);
            let p = shape::paint(e.kind, &n.style, theme);
            let dim = story.as_ref().is_some_and(|s| !s.1.contains(&n.id));
            // A collapsed boundary is a solid box that stands for what it holds.
            let p = if n.collapsed {
                shape::Paint {
                    fill: Some(shape::mix(
                        p.stroke.unwrap_or(theme.muted),
                        theme.canvas,
                        0.18,
                    )),
                    dashed: false,
                    ..p
                }
            } else {
                p
            };
            let sd = ShapeDraw {
                fill: o.fill,
                detail: o.detail,
                marks: o
                    .marks
                    .into_iter()
                    .map(|m| (m.segs, p.ink(m.ink)))
                    .collect(),
                shadow: p.shadow && !dim,
                fill_c: p.fill,
                stroke: p.stroke,
                opacity: if dim { p.opacity * 0.3 } else { p.opacity },
                dashed: p.dashed,
                width: if e.kind == Kind::Boundary { 1.2 } else { 1.5 },
                rect: n.rect(),
                effect: n.effect,
                radius: n
                    .style
                    .radius
                    .unwrap_or_else(|| shape::default_radius(e.kind)),
            };
            if e.kind == Kind::Boundary && !n.collapsed {
                under.push(sd);
            } else {
                over.push(sd);
            }
        }
        let cycles = self.ed.cycles();
        let line = shape::mix(theme.text, theme.canvas, 0.5);
        let edges: std::collections::HashMap<&str, &Edge> =
            v.edges.iter().map(|e| (e.id.as_str(), e)).collect();
        let wires = self
            .ed
            .wires()
            .into_iter()
            .filter(|(_, w)| wire_box(w).intersects(&vis))
            .filter_map(|(id, w)| {
                let e = *edges.get(id.as_str())?;
                let selected = self.ed.sel.has_edge(&id);
                let color = if selected {
                    accent
                } else if cycles.contains(&id) {
                    0xdc2626
                } else {
                    e.style
                        .stroke
                        .as_deref()
                        .and_then(parse_hex)
                        .unwrap_or(line)
                };
                Some(WireDraw {
                    pts: w.points,
                    curved: w.curved,
                    color,
                    dashed: e.style.dashed,
                    width: if selected { 2.5 } else { 1.5 },
                    start: e.arrow == Arrow::Both,
                    end: e.arrow != Arrow::None,
                    motion: e.motion,
                    speed: e.speed,
                    dim: story.as_ref().is_some_and(|s| !s.0.contains(&id)),
                })
            })
            .collect();
        let mut selected = Vec::new();
        let mut locked = Vec::new();
        for id in &self.ed.sel.nodes {
            if let Some(n) = v.node(id) {
                if self.ed.is_locked(n) {
                    locked.push(n.rect());
                } else {
                    selected.push(n.rect());
                }
            }
        }
        let dragging = !matches!(self.drag, Drag::None);
        let handles = match (&self.drag, self.one_node()) {
            (Drag::None | Drag::Resize { .. }, Some(id)) => v
                .node(&id)
                .filter(|n| !self.ed.is_locked(n) && !n.collapsed)
                .map(|n| Handle::ALL.iter().map(|h| h.point(&n.rect())).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut ports = Vec::new();
        if !dragging && self.tool == Tool::Select {
            for id in self.hover.iter() {
                if let Some(r) = self.ed.rect(id) {
                    if self.ed.kind_of(id) != Kind::Text {
                        ports.extend(Side::ALL.iter().map(|s| port(*s, &r, self.zoom)));
                    }
                }
            }
        }
        let (marquee, connect, target) = match &self.drag {
            Drag::Marquee { from, to, .. } => (Some(Rect::span(*from, *to)), None, None),
            Drag::Connect { from, to } => {
                let target = match self.ed.hit(*to, 6. / self.zoom) {
                    Hit::Node(t) => self.ed.rect(&t),
                    _ => None,
                };
                let wire = self.ed.rect(from).map(|r| {
                    let end = target.unwrap_or(Rect::new(to.0 - 1., to.1 - 1., 2., 2.));
                    let w = geom::route(
                        &r,
                        &end,
                        if target.is_some() {
                            Route::Elbow
                        } else {
                            Route::Straight
                        },
                    );
                    WireDraw {
                        pts: w.points,
                        curved: w.curved,
                        color: accent,
                        dashed: target.is_none(),
                        width: 2.,
                        start: false,
                        end: true,
                        motion: Motion::None,
                        speed: Speed::Normal,
                        dim: false,
                    }
                });
                (None, wire, target)
            }
            _ => (None, None, None),
        };
        Scene {
            pan: self.pan,
            zoom: self.zoom,
            bg: theme.canvas,
            grid: self.grid.then(|| {
                (
                    shape::mix(
                        theme.text,
                        theme.canvas,
                        if theme.dark { 0.06 } else { 0.04 },
                    ),
                    shape::mix(
                        theme.text,
                        theme.canvas,
                        if theme.dark { 0.11 } else { 0.08 },
                    ),
                )
            }),
            under,
            wires,
            over,
            accent,
            selected,
            locked,
            handles,
            ports,
            guides: self.guides.clone(),
            marquee,
            connect,
            target,
            t,
            moving: self.animating(),
            story: self.story_draw(),
            cycles: cycles.len(),
            bends: match &self.drag {
                Drag::None | Drag::Bend { .. } => self
                    .bendable()
                    .map(|(_, w, _)| geom::bend_handles(&w.points))
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            found: if self.find_open {
                self.found
                    .iter()
                    .filter_map(|id| self.ed.rect(id))
                    .collect()
            } else {
                Vec::new()
            },
            found_now: self
                .find_open
                .then(|| self.found.get(self.found_at))
                .flatten()
                .and_then(|id| self.ed.rect(id)),
            review: self.review_draw(),
        }
    }

    /// The step of the playing story, if one is on screen now.
    fn story_draw(&self) -> Option<StoryDraw> {
        let p = self.playing?;
        let st = self.ed.cur().stories.get(p.story)?;
        let (i, progress) = motion::step_at(p.elapsed(), st.steps.len())?;
        let edge = &st.steps[i].edge;
        let (_, wire) = self.ed.wires().into_iter().find(|(id, _)| id == edge)?;
        let r = self.ed.doc.relation(edge)?;
        let line = motion::flatten(&wire.points, wire.curved);
        let lit = [&r.from, &r.to]
            .iter()
            .filter_map(|id| self.ed.rect(id))
            .collect();
        Some(StoryDraw {
            line,
            progress,
            lit,
        })
    }

    /// Fill the property fields for what's selected, when that changes.
    fn load_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = (self.ed.view.clone(), self.ed.sel.clone(), self.generation);
        if self.loaded.as_ref() == Some(&key) {
            return;
        }
        self.loaded = Some(key);
        let set =
            |f: &Entity<InputState>, v: String, window: &mut Window, cx: &mut Context<Self>| {
                f.update(cx, |s, cx| s.set_value(v, window, cx));
            };
        let set_area =
            |f: &Entity<TextareaState>, v: String, window: &mut Window, cx: &mut Context<Self>| {
                f.update(cx, |s, cx| s.set_value(v, window, cx));
            };
        if let Some(e) = self
            .one_node()
            .and_then(|id| self.ed.doc.element(&id).cloned())
        {
            set(&self.f_name, e.name, window, cx);
            set(&self.f_tech, e.tech, window, cx);
            set(&self.f_owner, e.owner, window, cx);
            set(&self.f_tags, e.tags.join(", "), window, cx);
            set_area(&self.f_desc, e.description, window, cx);
            set_area(&self.f_code, e.code.join("\n"), window, cx);
        } else if let Some(r) = self
            .one_edge()
            .and_then(|id| self.ed.doc.relation(&id).cloned())
        {
            set(&self.f_name, r.label, window, cx);
            set(&self.f_tech, r.tech, window, cx);
        }
        let name = self.ed.cur().name.clone();
        set(&self.f_view, name, window, cx);
        self.text_open = false;
    }

    fn load_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.side != SideTab::Text || self.text_loaded == Some(self.generation) {
            return;
        }
        self.text_loaded = Some(self.generation);
        let model = self.ed.doc.model_text();
        let view = Doc::view_text(self.ed.cur());
        self.t_model
            .update(cx, |s, cx| s.set_value(model, window, cx));
        self.t_view
            .update(cx, |s, cx| s.set_value(view, window, cx));
        self.text_error = None;
    }

    fn palette(&self, cx: &mut Context<Self>) -> AnyElement {
        let q = self.search.read(cx).value().trim().to_string();
        let row = |i: usize, p: &'static Preset, cx: &mut Context<Self>| {
            let glyph = icon_named(p.icon).unwrap_or_else(|| kind_icon(p.kind));
            div()
                .id(("arch-preset", i))
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_grab()
                .text_size(px(12.5))
                .text_color(rgb(c_text()))
                .hover(|el| el.bg(tint(c_subtle())))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.drag = Drag::Place {
                            preset: p,
                            at: None,
                        };
                        window.focus(&this.focus, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .size(px(22.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .bg(rgb(shape::mix(
                            shape::accent(p.kind),
                            c_canvas(),
                            if is_dark() { 0.25 } else { 0.12 },
                        )))
                        .child(icon(glyph, 13., shape::accent(p.kind))),
                )
                .child(div().flex_1().min_w_0().truncate().child(p.label))
        };
        let mut list = div().v_flex().gap(px(1.));
        if q.is_empty() {
            let mut i = 0;
            for g in GROUPS {
                list = list.child(section(g));
                for p in PRESETS.iter().filter(|p| p.group == g) {
                    let idx = PRESETS.iter().position(|x| std::ptr::eq(x, p)).unwrap_or(i);
                    list = list.child(row(idx, p, cx));
                    i += 1;
                }
            }
        } else {
            let hits = search_presets(&q);
            if !hits.is_empty() {
                list = list.child(section("Shapes"));
            }
            for p in hits {
                let idx = PRESETS.iter().position(|x| std::ptr::eq(x, p)).unwrap_or(0);
                list = list.child(row(idx, p, cx));
            }
            list = list
                .child(section("Logos and icons"))
                .child(self.icon_results(&q, cx));
        }
        list = list.child(
            div()
                .id("arch-own-picture")
                .mt_1()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(rgb(c_secondary()))
                .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                .tooltip(tip("Add a PNG, JPG or SVG of your own"))
                .on_click(cx.listener(|this, _, _, cx| this.pick_picture(cx)))
                .child(icon(IconName::Image, 13., c_muted()))
                .child("Your own picture…"),
        );
        let unplaced: Vec<(String, String, Kind)> = self
            .ed
            .unplaced()
            .into_iter()
            .take(40)
            .map(|e| (e.id.clone(), e.name.clone(), e.kind))
            .collect();
        if !unplaced.is_empty() && q.is_empty() {
            list = list.child(section("In the model, not here"));
            for (i, (id, name, kind)) in unplaced.into_iter().enumerate() {
                list = list.child(
                    div()
                        .id(("arch-unplaced", i))
                        .h(px(26.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                        .tooltip(tip("Place it on this diagram, with its connections"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let at = this.center_world();
                            this.ed.place(&id, at);
                            this.after_edit(cx);
                        }))
                        .child(icon(kind_icon(kind), 12., c_muted()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(if name.is_empty() {
                                    kind.label().to_string()
                                } else {
                                    name
                                }),
                        ),
                );
            }
        }
        div()
            .w(px(220.))
            .flex_none()
            .h_full()
            .v_flex()
            .border_r_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_sidebar()))
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .child(
                        Input::new(&self.search)
                            .bordered(true)
                            .h(px(30.))
                            .prefix(icon(IconName::Search, 13., c_muted())),
                    ),
            )
            .child(
                div()
                    .id("arch-palette")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pb_3()
                    .child(list),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child("Drag a shape onto the canvas, or click it to add it in the middle."),
            )
            .into_any_element()
    }

    /// The logos that match a search, as a grid to drag from.
    fn icon_results(&self, q: &str, cx: &mut Context<Self>) -> AnyElement {
        let message = |text: String| {
            div()
                .p_2()
                .text_size(px(12.))
                .text_color(rgb(c_muted()))
                .child(text)
                .into_any_element()
        };
        if q.chars().count() < 2 {
            return message("Type two letters or more to search logos.".into());
        }
        let lib = match &self.icons {
            IconLib::Ready(lib) => lib.clone(),
            IconLib::Failed(e) => return message(e.clone()),
            IconLib::Idle | IconLib::Loading => {
                return message("Getting the logo library…".into());
            }
        };
        let hits = lib.search(q, ICON_HITS);
        if hits.is_empty() {
            return message("No logo matches.".into());
        }
        let mut grid = div().flex().flex_wrap().gap(px(2.));
        for (i, hit) in hits.into_iter().enumerate() {
            let thumb = self.thumbs.get(&thumb_key(hit)).cloned();
            let title = hit.title.clone();
            let file = match &thumb {
                Some(Thumb::Ready(f)) => Some(f.clone()),
                _ => None,
            };
            let picture = match thumb {
                Some(Thumb::Ready(f)) => img(f)
                    .size(px(26.))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                Some(Thumb::Failed) => icon(IconName::CircleX, 14., c_muted()).into_any_element(),
                _ => div()
                    .size(px(22.))
                    .rounded(px(5.))
                    .bg(tint(c_subtle()))
                    .into_any_element(),
            };
            grid = grid.child(
                div()
                    .id(("arch-logo", i))
                    .w(px(48.))
                    .h(px(54.))
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap(px(3.))
                    .rounded_md()
                    .when(file.is_some(), |el| el.cursor_grab())
                    .hover(|el| el.bg(tint(c_subtle())))
                    .tooltip({
                        let text: SharedString =
                            format!("{} · {}", hit.title, hit.set.label()).into();
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                .build(window, cx)
                        }
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if let Some(file) = &file {
                                this.drag = Drag::Picture {
                                    file: file.clone(),
                                    name: title.clone(),
                                    at: None,
                                };
                                window.focus(&this.focus, cx);
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(picture)
                    .child(
                        div()
                            .w_full()
                            .px(px(2.))
                            .truncate()
                            .text_center()
                            .text_size(px(9.5))
                            .text_color(rgb(c_secondary()))
                            .child(hit.title.clone()),
                    ),
            );
        }
        grid.into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = div()
            .id("arch-tabs")
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .overflow_x_scroll();
        for (i, v) in self.ed.doc.views.iter().enumerate() {
            let on = v.id == self.ed.view;
            let id = v.id.clone();
            tabs = tabs.child(
                div()
                    .id(("arch-view", i))
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |el| {
                        el.bg(tint(c_selected())).font_weight(FontWeight::MEDIUM)
                    })
                    .text_color(rgb(if on { c_text() } else { c_secondary() }))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_view(&id, cx)))
                    .child(v.name.clone()),
            );
        }
        tabs = tabs.child(
            icon_btn("arch-add-view", IconName::Plus, "New diagram", false).on_click(cx.listener(
                |this, _, _, cx| {
                    let n = this.ed.doc.views.len() + 1;
                    this.act(cx, |ed| {
                        ed.add_view(&format!("Diagram {n}"));
                    });
                    this.fitted = false;
                },
            )),
        );
        let sep = || div().w(px(1.)).h(px(18.)).mx_1().bg(rgb(c_border()));
        let zoom = format!("{}%", (self.zoom * 100.).round());
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
            .child(div().flex_1().min_w_0().child(tabs))
            .child(
                icon_btn(
                    "arch-select",
                    IconName::MousePointer2,
                    "Select (V)",
                    self.tool == Tool::Select,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.tool = Tool::Select;
                    cx.notify();
                })),
            )
            .child(
                icon_btn(
                    "arch-hand",
                    IconName::Hand,
                    "Pan (H, or hold Space)",
                    self.tool == Tool::Hand,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.tool = Tool::Hand;
                    cx.notify();
                })),
            )
            .child(sep())
            .child(
                icon_btn("arch-undo", IconName::Undo2, "Undo", false)
                    .when(!self.ed.can_undo(), |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
            )
            .child(
                icon_btn("arch-redo", IconName::Redo2, "Redo", false)
                    .when(!self.ed.can_redo(), |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
            )
            .child(sep())
            .child(
                text_btn(
                    "arch-beautify",
                    "Beautify",
                    Some(IconName::WandSparkles),
                    self.beautify_open,
                )
                .tooltip(tip("Untangle connectors and tidy the layout (Shift+A)"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.beautify_open = !this.beautify_open;
                    this.export_open = false;
                    this.check_open = false;
                    cx.notify();
                })),
            )
            .child(
                icon_btn("arch-grid", IconName::Grid3x3, "Grid", self.grid).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.grid = !this.grid;
                        cx.notify();
                    },
                )),
            )
            .child(
                icon_btn(
                    "arch-snap",
                    IconName::Magnet,
                    "Snap to grid and guides (hold Alt to move freely)",
                    self.snap,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.snap = !this.snap;
                    cx.notify();
                })),
            )
            .child(
                icon_btn(
                    "arch-find",
                    IconName::Search,
                    "Find on the canvas (⌘F)",
                    self.find_open,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.find_open {
                        this.close_find(window, cx);
                    } else {
                        this.open_find(window, cx);
                    }
                })),
            )
            .child(
                icon_btn(
                    "arch-check",
                    IconName::ListChecks,
                    "Check the diagram",
                    self.check_open,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.check_open = !this.check_open;
                    this.export_open = false;
                    this.beautify_open = false;
                    cx.notify();
                })),
            )
            .child(
                icon_btn(
                    "arch-motion-toggle",
                    IconName::Activity,
                    "Motion: animate connectors and effects (saved with the diagram)",
                    !self.ed.cur().still,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let still = !this.ed.cur().still;
                    this.act(cx, |ed| ed.set_still(still));
                })),
            )
            .child(sep())
            .child(
                icon_btn("arch-zoom-out", IconName::Minus, "Zoom out", false).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.zoom_step(1. / 1.2);
                        cx.notify();
                    }),
                ),
            )
            .child(
                div()
                    .id("arch-zoom")
                    .w(px(48.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .tooltip(tip("Fit the diagram (Shift+1)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fit(false);
                        cx.notify();
                    }))
                    .child(zoom),
            )
            .child(
                icon_btn("arch-zoom-in", IconName::Plus, "Zoom in", false).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.zoom_step(1.2);
                        cx.notify();
                    },
                )),
            )
            .child(sep())
            .child(
                icon_btn(
                    "arch-from-code",
                    if self.scanning {
                        IconName::LoaderCircle
                    } else {
                        IconName::ScanSearch
                    },
                    "From code: draw the project's packages and the services they use",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.start_from_code(cx))),
            )
            .child(
                icon_btn(
                    "arch-import",
                    IconName::Upload,
                    "Import a draw.io file, or a picture of a diagram for AI to redraw",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.import(cx))),
            )
            .child(
                text_btn("arch-export", "Export", Some(IconName::Download), false).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.export_open = !this.export_open;
                        this.beautify_open = false;
                        cx.notify();
                    }),
                ),
            )
            .into_any_element()
    }

    /// Make the diagram easier to read, and say what changed.
    fn run_beautify(&mut self, how: Beautify, cx: &mut Context<Self>) {
        self.commit_layer_name(cx);
        self.menu = None;
        self.beautify_open = false;
        let (before, after) = self.ed.beautify(how);
        self.after_edit(cx);
        if before != after {
            self.fit(false);
        }
        let text = if before == after {
            format!(
                "This is already the cleanest layout Runo found: {}.",
                after.describe()
            )
        } else if after.is_clean() {
            if before.is_clean() {
                "Beautified: lined up and spaced out. ⌘Z undoes it.".to_string()
            } else {
                format!(
                    "Beautified: {} before, none now. ⌘Z undoes it.",
                    before.describe()
                )
            }
        } else {
            format!(
                "Beautified: {} before, {} now. ⌘Z undoes it.",
                before.describe(),
                after.describe()
            )
        };
        self.note(text, false);
        cx.notify();
    }

    /// Have an agent regroup the diagram by meaning, then lay it out.
    fn beautify_with_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.beautify_open = false;
        let task = "Make this diagram easy to read without changing what it says. Group parts \
                    into boundaries by responsibility or tier (for example clients, edge, \
                    services, data, external systems), order them along the way a request \
                    flows, give connectors short labels of what flows, and lay shapes on a \
                    grid in rows or columns so connectors run straight and don't cross or \
                    pass behind shapes. Keep every part and connection; don't add new ones."
            .to_string();
        self.ask_edit(task, Vec::new(), window, cx);
    }

    fn beautify_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let score = self.ed.layout_score();
        let item = |id: &'static str, label: &'static str, sub: &'static str, main: bool| {
            div()
                .id(id)
                .px_3()
                .py_2()
                .v_flex()
                .rounded_md()
                .cursor_pointer()
                .when(main, |el| el.bg(col(c_blue(), 0.12)))
                .hover(|el| el.bg(tint(c_subtle())))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(if main {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(rgb(c_text()))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(sub),
                )
        };
        let routes = [
            (Route::Elbow, "Elbow"),
            (Route::Curved, "Curved"),
            (Route::Straight, "Straight"),
        ];
        let mut route_row = div().flex().gap_1().px_3().pb_2();
        for (i, (r, label)) in routes.into_iter().enumerate() {
            route_row =
                route_row.child(chip_btn(("arch-beautify-route", i), label, false).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.beautify_open = false;
                        this.act(cx, |ed| ed.set_all_routes(r));
                    }),
                ));
        }
        div()
            .absolute()
            .top(px(6.))
            .right(px(8.))
            .w(px(300.))
            .p_1()
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(float(c_canvas()))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(
                        if score.is_clean() {
                            IconName::CircleCheck
                        } else {
                            IconName::CircleAlert
                        },
                        13.,
                        if score.is_clean() { 0x22c55e } else { 0xf59e0b },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .child(format!("Now: {}", score.describe())),
                    ),
            )
            .child(
                item(
                    "arch-beautify-best",
                    "Beautify",
                    "Tries tidying and each layout, and keeps the cleanest",
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_beautify(Beautify::Best, cx))),
            )
            .child(
                item(
                    "arch-beautify-tidy",
                    "Tidy, keep my layout",
                    "Lines up, spaces out and uncrosses without moving things far",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_beautify(Beautify::Tidy, cx))),
            )
            .child(
                item(
                    "arch-beautify-down",
                    "Rearrange top to bottom",
                    "Callers above what they call, in layers",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_beautify(Beautify::Down, cx))),
            )
            .child(
                item(
                    "arch-beautify-across",
                    "Rearrange left to right",
                    "Requests flow from left to right",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.run_beautify(Beautify::Across, cx))),
            )
            .child(
                item(
                    "arch-beautify-ai",
                    "Beautify with AI",
                    "Regroups parts by what they do and labels what flows, for you to accept",
                    false,
                )
                .on_click(cx.listener(|this, _, window, cx| this.beautify_with_ai(window, cx))),
            )
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child("Every connector as"),
            )
            .child(route_row)
            .into_any_element()
    }

    fn export_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: &'static str, label: &'static str, sub: &'static str| {
            div()
                .id(id)
                .px_3()
                .py_2()
                .v_flex()
                .rounded_md()
                .cursor_pointer()
                .hover(|el| el.bg(tint(c_subtle())))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_text()))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(sub),
                )
        };
        div()
            .absolute()
            .top(px(6.))
            .right(px(8.))
            .w(px(250.))
            .p_1()
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(float(c_canvas()))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                item(
                    "arch-exp-drawio",
                    "draw.io file",
                    "Every diagram, editable in draw.io",
                )
                .on_click(cx.listener(|this, _, _, cx| this.export("drawio", cx))),
            )
            .child(
                item("arch-exp-svg", "SVG", "This diagram, as a vector image")
                    .on_click(cx.listener(|this, _, _, cx| this.export("svg", cx))),
            )
            .child(
                item(
                    "arch-exp-motion",
                    "Animated SVG",
                    "Moving connectors and effects, for docs and slides",
                )
                .on_click(cx.listener(|this, _, _, cx| this.export("motion.svg", cx))),
            )
            .child(
                item(
                    "arch-exp-html",
                    "Story player",
                    "A web page that plays this diagram's stories",
                )
                .on_click(cx.listener(|this, _, _, cx| this.export("html", cx))),
            )
            .child(
                item("arch-exp-png", "PNG", "This diagram, as an image at 2x")
                    .on_click(cx.listener(|this, _, _, cx| this.export("png", cx))),
            )
            .child(
                item(
                    "arch-exp-mmd",
                    "Mermaid file",
                    "This diagram as a .mmd flowchart",
                )
                .on_click(cx.listener(|this, _, _, cx| this.export("mmd", cx))),
            )
            .child(
                item(
                    "arch-exp-copy",
                    "Copy as Mermaid",
                    "For a README, an issue or a doc",
                )
                .on_click(cx.listener(|this, _, _, cx| this.copy_mermaid(cx))),
            )
            .into_any_element()
    }

    /// Names and notes drawn over the shapes, and connector labels.
    fn labels(&self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let v = self.ed.cur();
        let z = self.zoom;
        let mut out = Vec::new();
        let editing_node = match &self.editing {
            Some(Hit::Node(id)) => Some(id.clone()),
            _ => None,
        };
        let els = self.ed.elements();
        let vis = self.visible_world();
        let sight = self.ed.sight();
        for n in &v.nodes {
            if !n.rect().intersects(&vis) || !sight.shows(&n.id) {
                continue;
            }
            let Some(e) = els.get(n.id.as_str()) else {
                continue;
            };
            let p = shape::paint(e.kind, &n.style, theme);
            // Fold or open a boundary from its corner.
            if e.kind == Kind::Boundary && z > 0.4 && self.playing.is_none() {
                let nr = self.screen_rect(&n.rect());
                let id = n.id.clone();
                out.push(
                    div()
                        .id(SharedString::from(format!("arch-fold-{}", n.id)))
                        .absolute()
                        .left(px(nr.x + nr.w - 26.))
                        .top(px(nr.y + 6.))
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .tooltip(tip(if n.collapsed {
                            "Expand"
                        } else {
                            "Collapse to one box"
                        }))
                        .child(icon(
                            if n.collapsed {
                                IconName::Maximize
                            } else {
                                IconName::Minimize
                            },
                            12.,
                            p.sub,
                        ))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.act(cx, |ed| ed.toggle_collapse(&id));
                        }))
                        .into_any_element(),
                );
            }
            let o = shape::outline(
                if n.collapsed { Kind::Container } else { e.kind },
                &n.rect(),
                &n.style,
            );
            let fs = p.font * z;
            let nr = self.screen_rect(&n.rect());
            if !e.image.is_empty() && z > 0.15 {
                let file = folder(&self.root).join(&e.image);
                let sr = self.screen_rect(&shape::picture_rect(e.kind, &n.rect()));
                out.push(
                    div()
                        .absolute()
                        .left(px(sr.x))
                        .top(px(sr.y))
                        .w(px(sr.w))
                        .h(px(sr.h))
                        .opacity(p.opacity)
                        .child(img(file).size_full().object_fit(ObjectFit::Contain))
                        .into_any_element(),
                );
            }
            if e.image.is_empty()
                && !e.icon.is_empty()
                && e.kind.is_part()
                // A person's avatar already says what it is.
                && e.kind != Kind::Person
                && nr.h > 34.
                && nr.w > 60.
                && z > 0.45
            {
                if let Some(glyph) = icon_named(&e.icon) {
                    let (ix, iy) = match e.kind {
                        Kind::Database => (nr.x + 8. * z, nr.y + nr.h * 0.25),
                        Kind::Container | Kind::External => {
                            (nr.x + (shape::SPINE + 8.) * z, nr.y + 8. * z)
                        }
                        Kind::Component => (nr.x + 14. * z, nr.y + 8. * z),
                        _ => (nr.x + 8. * z, nr.y + 8. * z),
                    };
                    out.push(
                        div()
                            .absolute()
                            .left(px(ix))
                            .top(px(iy))
                            .opacity(0.8 * p.opacity)
                            .child({
                                let i = icon(glyph, 14. * z, p.stroke.unwrap_or(p.text));
                                if n.effect == Effect::Spin && !v.still {
                                    i.rotate(radians(motion::spin(
                                        self.born.elapsed().as_secs_f32(),
                                    )))
                                } else {
                                    i
                                }
                            })
                            .into_any_element(),
                    );
                }
            }
            if editing_node.as_deref() == Some(n.id.as_str()) || fs < 4. {
                continue;
            }
            let tr = self.screen_rect(&o.text);
            let top = e.kind == Kind::Boundary && !n.collapsed;
            let mut block = div()
                .absolute()
                .left(px(tr.x))
                .top(px(tr.y))
                .w(px(tr.w.max(1.)))
                .h(px(tr.h.max(1.)))
                .overflow_hidden()
                .v_flex()
                .opacity(p.opacity)
                .when(!top, |el| el.items_center().justify_center().text_center())
                .when(top, |el| el.flex_row().items_center().gap(px(6. * z)));
            if top && !e.image.is_empty() {
                block = block.child(div().w(px(18. * z)).flex_none());
            } else if top {
                if let Some(g) = icon_named(&e.icon) {
                    block = block.child(icon(g, 13. * z, p.stroke.unwrap_or(p.text)));
                }
            }
            let name = if e.name.is_empty() && e.kind != Kind::Text {
                String::new()
            } else {
                e.name.clone()
            };
            block = block.child(
                div()
                    .text_size(px(fs))
                    .line_height(px(fs * 1.3))
                    .text_color(rgb(p.text))
                    .when(
                        e.kind != Kind::Text && e.kind != Kind::Note || p.bold,
                        |el| el.font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(name),
            );
            if top && !e.tech.is_empty() {
                block = block.child(
                    div()
                        .text_size(px(fs * 0.85))
                        .text_color(rgb(p.sub))
                        .child(format!("[{}]", e.tech)),
                );
            }
            // Far out, a box shows only its name.
            let detail = z >= DETAIL_MIN_ZOOM;
            if let Some(c4) = e.kind.c4().filter(|_| detail) {
                let line = if e.tech.is_empty() {
                    format!("[{c4}]")
                } else {
                    format!("[{c4}: {}]", e.tech)
                };
                block = block.child(
                    div()
                        .text_size(px(fs * 0.78))
                        .line_height(px(fs * 1.05))
                        .text_color(rgb(p.sub))
                        .child(line),
                );
            }
            if n.collapsed {
                let inside = v.descendants(&n.id).len();
                block = block.child(div().text_size(px(fs * 0.78)).text_color(rgb(p.sub)).child(
                    format!("{inside} part{} inside", if inside == 1 { "" } else { "s" }),
                ));
            }
            if detail && !top && !e.description.is_empty() && tr.h > fs * 4.2 {
                block = block.child(
                    div()
                        .mt(px(3. * z))
                        .text_size(px(fs * 0.8))
                        .line_height(px(fs * 1.1))
                        .text_color(rgb(p.sub))
                        .child(e.description.clone()),
                );
            }
            out.push(block.into_any_element());
            if n.locked && self.ed.sel.has_node(&n.id) {
                out.push(
                    div()
                        .absolute()
                        .left(px(nr.x + nr.w - 14.))
                        .top(px(nr.y - 16.))
                        .child(icon(IconName::Lock, 12., c_muted()))
                        .into_any_element(),
                );
            }
        }
        let editing_edge = match &self.editing {
            Some(Hit::Edge(id)) => Some(id.clone()),
            _ => None,
        };
        let rels = self.ed.relations();
        for (id, w) in self.ed.wires() {
            if editing_edge.as_deref() == Some(id.as_str()) || z < 0.35 {
                continue;
            }
            if !vis.contains(w.middle()) {
                continue;
            }
            let Some(r) = rels.get(id.as_str()) else {
                continue;
            };
            let label = [r.label.as_str(), r.tech.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            if label.is_empty() || z < 0.35 {
                continue;
            }
            let (mx, my) = self.screen(w.middle());
            let fs = 11.5 * z.min(1.6);
            out.push(
                div()
                    .absolute()
                    .left(px(mx - 150.))
                    .top(px(my - fs))
                    .w(px(300.))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px(px(5.))
                            .rounded(px(4.))
                            .bg(rgb(theme.canvas))
                            .text_size(px(fs))
                            .line_height(px(fs * 1.5))
                            .text_color(rgb(if self.ed.sel.has_edge(&id) {
                                c_blue()
                            } else {
                                theme.muted
                            }))
                            .max_w(px(300.))
                            .truncate()
                            .child(label),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    fn menu_view(&self, at: Pt, menu: &Menu, cx: &mut Context<Self>) -> AnyElement {
        type Act = Box<dyn Fn(&mut ArchEditor, &mut Window, &mut Context<ArchEditor>)>;
        let mut items: Vec<(&'static str, &'static str, Act)> = Vec::new();
        let edit: Act = Box::new(|this, window, cx| {
            if let Some(id) = this.one_node() {
                this.start_inline(Hit::Node(id), window, cx);
            } else if let Some(id) = this.one_edge() {
                this.start_inline(Hit::Edge(id), window, cx);
            }
        });
        match menu {
            Menu::Node(id) => {
                let multi = self.ed.sel.nodes.len() > 1;
                if !multi {
                    items.push(("Rename", "Enter", edit));
                }
                if !multi && self.code_file(id).is_some() {
                    let id = id.clone();
                    items.push((
                        "Open its code",
                        "",
                        Box::new(move |this, _, cx| {
                            if let Some(f) = this.code_file(&id) {
                                cx.emit(ArchEvent::Open(f));
                            }
                        }),
                    ));
                }
                items.push((
                    "Change with AI…",
                    "",
                    Box::new(|this, window, cx| this.focus_ask(window, cx)),
                ));
                if !multi && self.ed.kind_of(id).is_part() {
                    let (a, b) = (id.clone(), id.clone());
                    items.push((
                        "Explain with AI",
                        "",
                        Box::new(move |this, _, cx| this.ask_about(&a, cx)),
                    ));
                    items.push((
                        "Expand into parts with AI",
                        "",
                        Box::new(move |this, window, cx| this.expand_with_ai(&b, window, cx)),
                    ));
                }
                let has_picture = self
                    .ed
                    .doc
                    .element(id)
                    .is_some_and(|e| !e.image.is_empty() && e.kind != Kind::Icon);
                if !multi && has_picture {
                    let id = id.clone();
                    items.push((
                        "Remove picture",
                        "",
                        Box::new(move |this, _, cx| {
                            let id = id.clone();
                            this.act(cx, |ed| ed.set_image(&id, ""));
                        }),
                    ));
                }
                if let Some(link) = self
                    .ed
                    .doc
                    .element(id)
                    .map(|e| e.link.clone())
                    .filter(|l| !l.is_empty())
                {
                    items.push((
                        "Open linked diagram",
                        "",
                        Box::new(move |this, _, cx| this.drill(&link, cx)),
                    ));
                }
                items.push((
                    "Duplicate",
                    "⌘D",
                    Box::new(|this, _, cx| this.act(cx, |ed| ed.duplicate())),
                ));
                items.push((
                    "Copy",
                    "⌘C",
                    Box::new(|this, _, _| {
                        this.ed.copy();
                    }),
                ));
                items.push((
                    "Bring to front",
                    "⌘]",
                    Box::new(|this, _, cx| this.act(cx, |ed| ed.order(true))),
                ));
                items.push((
                    "Send to back",
                    "⌘[",
                    Box::new(|this, _, cx| this.act(cx, |ed| ed.order(false))),
                ));
                items.push((
                    "Group in a boundary",
                    "⌘G",
                    Box::new(|this, _, cx| {
                        this.act(cx, |ed| {
                            ed.group();
                        })
                    }),
                ));
                if self.ed.kind_of(id) == Kind::Boundary {
                    let folded = self.ed.cur().node(id).is_some_and(|n| n.collapsed);
                    let fid = id.clone();
                    items.push((
                        if folded {
                            "Expand"
                        } else {
                            "Collapse to one box"
                        },
                        "",
                        Box::new(move |this, _, cx| {
                            let fid = fid.clone();
                            this.act(cx, |ed| ed.toggle_collapse(&fid))
                        }),
                    ));
                    items.push((
                        "Ungroup",
                        "⇧⌘G",
                        Box::new(|this, _, cx| this.act(cx, |ed| ed.ungroup())),
                    ));
                }
                items.push((
                    "Lock or unlock",
                    "⇧⌘L",
                    Box::new(|this, _, cx| this.act(cx, |ed| ed.toggle_lock())),
                ));
                items.push((
                    "Copy style",
                    "⌥⌘C",
                    Box::new(|this, _, _| this.ed.copy_style()),
                ));
                if self.ed.style_clip.is_some() {
                    items.push((
                        "Paste style",
                        "⌥⌘V",
                        Box::new(|this, _, cx| this.act(cx, |ed| ed.paste_style())),
                    ));
                }
                items.push((
                    "Delete",
                    "⌫",
                    Box::new(|this, _, cx| this.act(cx, |ed| ed.delete())),
                ));
            }
            Menu::Edge => {
                items.push(("Edit label", "Enter", edit));
                items.push((
                    "Reset path",
                    "",
                    Box::new(|this, _, cx| this.act(cx, |ed| ed.reset_paths())),
                ));
                items.push((
                    "Reverse direction",
                    "",
                    Box::new(|this, _, cx| {
                        let ids = this.ed.sel.edges.clone();
                        this.act(cx, |ed| {
                            for id in &ids {
                                if let Some(r) = ed.doc.relation_mut(id) {
                                    std::mem::swap(&mut r.from, &mut r.to);
                                }
                            }
                        });
                    }),
                ));
                items.push((
                    "Delete",
                    "⌫",
                    Box::new(|this, _, cx| this.act(cx, |ed| ed.delete())),
                ));
            }
            Menu::Canvas(w) => {
                let w = *w;
                items.push((
                    "Add a shape here",
                    "Double-click",
                    Box::new(move |this, window, cx| this.open_quick(w, None, window, cx)),
                ));
                if self.ed.can_paste() {
                    items.push((
                        "Paste",
                        "⌘V",
                        Box::new(|this, _, cx| this.act(cx, |ed| ed.paste())),
                    ));
                }
                items.push((
                    "Suggest what's missing with AI",
                    "",
                    Box::new(|this, window, cx| this.suggest_missing(window, cx)),
                ));
                items.push((
                    "Select all",
                    "⌘A",
                    Box::new(|this, _, _| this.ed.select_all()),
                ));
                items.push((
                    "Beautify",
                    "⇧A",
                    Box::new(|this, _, cx| this.run_beautify(Beautify::Best, cx)),
                ));
                items.push((
                    "Fit to screen",
                    "⇧1",
                    Box::new(|this, _, _| this.fit(false)),
                ));
            }
        }
        // Keep the whole menu on the canvas.
        let (vw, vh) = self.view_size();
        let tall = items.len() as f32 * 28. + 10.;
        let left = at.0.min(vw - 238.).max(4.);
        let top = at.1.min(vh - tall - 4.).max(4.);
        let mut list = div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(230.))
            .p_1()
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(float(c_canvas()))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }));
        for (i, (label, keys, act)) in items.into_iter().enumerate() {
            let act = Rc::new(act);
            list = list.child(
                div()
                    .id(("arch-menu", i))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .text_color(rgb(if label == "Delete" { c_red() } else { c_text() }))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        (act)(this, window, cx);
                        cx.notify();
                    }))
                    .child(div().flex_1().child(label))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(keys),
                    ),
            );
        }
        list.into_any_element()
    }

    fn quick_view(&self, at: Pt, cx: &mut Context<Self>) -> AnyElement {
        let (x, y) = self.screen(at);
        let q = self.quick_q.read(cx).value().to_string();
        let hits: Vec<&'static Preset> = search_presets(&q).into_iter().take(8).collect();
        let mut list = div().v_flex().mt_1();
        for (i, p) in hits.into_iter().enumerate() {
            let glyph = icon_named(p.icon).unwrap_or_else(|| kind_icon(p.kind));
            list = list.child(
                div()
                    .id(("arch-quick", i))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(i == 0, |el| el.bg(tint(c_subtle())))
                    .hover(|el| el.bg(tint(c_selected())))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| this.quick_add(p, window, cx)))
                    .child(icon(glyph, 13., shape::accent(p.kind)))
                    .child(div().flex_1().child(p.label))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(p.group),
                    ),
            );
        }
        div()
            .absolute()
            .left(px(x - 130.))
            .top(px(y - 20.))
            .w(px(260.))
            .p_2()
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(float(c_canvas()))
            .shadow_xl()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(Input::new(&self.quick_q).bordered(true).h(px(30.)))
            .child(list)
            .when(self.quick_from.is_some(), |el| {
                el.child(
                    div()
                        .pt_1()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child("It will be connected to the shape you dragged from."),
                )
            })
            .into_any_element()
    }

    fn inline_view(&self) -> Option<AnyElement> {
        let hit = self.editing.as_ref()?;
        let (x, y, w) = match hit {
            Hit::Node(id) => {
                let r = self.screen_rect(&self.ed.rect(id)?);
                (r.x + r.w / 2., r.y + r.h / 2., r.w.max(180.))
            }
            Hit::Edge(id) => {
                let (_, wire) = self.ed.wires().into_iter().find(|(e, _)| e == id)?;
                let (mx, my) = self.screen(wire.middle());
                (mx, my, 200.)
            }
            Hit::None => return None,
        };
        Some(
            div()
                .absolute()
                .left(px(x - w / 2.))
                .top(px(y - 16.))
                .w(px(w))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Input::new(&self.inline)
                        .bordered(true)
                        .h(px(32.))
                        .bg(rgb(c_canvas())),
                )
                .into_any_element(),
        )
    }

    fn minimap(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let v = self.ed.cur();
        if v.nodes.len() < 2 {
            return None;
        }
        let (vw, vh) = self.view_size();
        let visible = Rect::new(
            -self.pan.0 / self.zoom,
            -self.pan.1 / self.zoom,
            vw / self.zoom,
            vh / self.zoom,
        );
        let content = self.ed.extent()?;
        let all = content.union(&visible).inflate(40.);
        let (mw, mh) = (180., 120.);
        let s = (mw / all.w).min(mh / all.h);
        let map = |r: &Rect| {
            Rect::new(
                (r.x - all.x) * s,
                (r.y - all.y) * s,
                (r.w * s).max(2.),
                (r.h * s).max(2.),
            )
        };
        let mut el = div()
            .id("arch-minimap")
            .absolute()
            .right(px(12.))
            .bottom(px(12.))
            .w(px(mw))
            .h(px(mh))
            .rounded_md()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(float(c_canvas()))
            .overflow_hidden()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    // Centre the view where the map was clicked.
                    let b = this.bounds.get();
                    let local = (
                        f32::from(e.position.x - b.origin.x) - (f32::from(b.size.width) - 12. - mw),
                        f32::from(e.position.y - b.origin.y)
                            - (f32::from(b.size.height) - 12. - mh),
                    );
                    let world = (local.0 / s + all.x, local.1 / s + all.y);
                    let (vw, vh) = this.view_size();
                    this.pan = (vw / 2. - world.0 * this.zoom, vh / 2. - world.1 * this.zoom);
                    cx.notify();
                }),
            );
        // Painted in one pass, so a diagram of thousands of shapes costs one element here.
        let els = self.ed.elements();
        let (muted, canvas_c) = (c_muted(), c_canvas());
        let sight = self.ed.sight();
        let boxes: Vec<(Rect, Kind)> = v
            .nodes
            .iter()
            .filter(|n| sight.shows(&n.id))
            .map(|n| {
                let kind = els.get(n.id.as_str()).map(|e| e.kind).unwrap_or_default();
                (map(&n.rect()), kind)
            })
            .collect();
        el = el.child(
            canvas(
                |_, _, _| {},
                move |b, _, window, _| {
                    for (r, kind) in &boxes {
                        let bounds = Bounds::new(
                            point(b.origin.x + px(r.x), b.origin.y + px(r.y)),
                            size(px(r.w), px(r.h)),
                        );
                        let boundary = *kind == Kind::Boundary;
                        let fill = if boundary {
                            col(0, 0.)
                        } else {
                            col(shape::mix(shape::accent(*kind), canvas_c, 0.6), 1.)
                        };
                        window.paint_quad(quad(
                            bounds,
                            px(1.5),
                            fill,
                            px(if boundary { 1. } else { 0. }),
                            col(muted, 1.),
                            BorderStyle::Solid,
                        ));
                    }
                },
            )
            .absolute()
            .size_full(),
        );
        let r = map(&visible);
        el = el.child(
            div()
                .absolute()
                .left(px(r.x))
                .top(px(r.y))
                .w(px(r.w))
                .h(px(r.h))
                .border_1()
                .border_color(rgb(c_blue()))
                .bg(col(c_blue(), 0.06)),
        );
        Some(el.into_any_element())
    }

    fn empty_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.ed.cur().nodes.is_empty() || !matches!(self.drag, Drag::None) || self.working() {
            return None;
        }
        let fresh = self.ed.doc.elements.is_empty();
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
                        .w(px(440.))
                        .p_5()
                        .v_flex()
                        .gap_3()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .bg(float(c_canvas()))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(if fresh { "Draw your architecture" } else { "This diagram is empty" }))
                        .child(div().text_size(px(12.5)).text_color(rgb(c_secondary())).child(if fresh {
                            "Drag shapes from the left, or double-click the canvas to add one. Connect two shapes by dragging from the dots on a shape's sides. Everything is saved to architecture/ in the repo."
                        } else {
                            "Add shapes from the left, or place parts of the model you already drew on other diagrams."
                        }))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(text_btn("arch-empty-code", if self.scanning { "Reading the code…" } else { "Start from code" }, Some(IconName::ScanSearch), true).on_click(cx.listener(|this, _, _, cx| this.start_from_code(cx))))
                                .child(text_btn("arch-empty-import", "Import draw.io or a picture", Some(IconName::Upload), false).on_click(cx.listener(|this, _, _, cx| this.import(cx)))),
                        )
                        .child(div().text_size(px(11.5)).text_color(rgb(c_muted())).child("Shortcuts: R container, D database, P person, Q queue, B boundary, T text, N note. Tab adds a connected shape.")),
                )
                .into_any_element(),
        )
    }

    /// Whether an agent or a code scan is drawing right now.
    fn working(&self) -> bool {
        self.waiting.is_some() || self.scanning
    }

    /// Seconds the agent has been at it, or None while nothing is waiting on it.
    fn working_for(&self) -> Option<u64> {
        self.waiting.as_ref().map(|w| w.started.elapsed().as_secs())
    }

    /// Shows the canvas is busy while the agent draws: a sketch of a diagram taking shape on
    /// an empty canvas, else a soft glow round the canvas and a line saying what's going on.
    fn thinking(&self) -> Option<AnyElement> {
        if !self.working() {
            return None;
        }
        let line = thinking_line(self.working_for(), self.scanning).to_string();
        Some(if self.ed.cur().nodes.is_empty() {
            askbar::busy_card(
                if self.scanning {
                    "Reading your code"
                } else {
                    "Drawing your architecture"
                },
                line,
                self.working_for(),
            )
        } else {
            askbar::busy_edge(line, self.working_for())
        })
    }

    /// What an ask is about, for the chip on the ask bar: the selection by name.
    fn ask_target(&self) -> Option<String> {
        let sel = &self.ed.sel;
        let mut names: Vec<String> = sel
            .nodes
            .iter()
            .filter_map(|id| self.ed.doc.element(id).map(|e| e.name.clone()))
            .chain(sel.edges.iter().filter_map(|id| {
                let r = self.ed.doc.relation(id)?;
                let name = |e: &str| {
                    self.ed
                        .doc
                        .element(e)
                        .map(|e| e.name.clone())
                        .unwrap_or_default()
                };
                Some(format!("{} to {}", name(&r.from), name(&r.to)))
            }))
            .filter(|n| !n.is_empty())
            .collect();
        let total = sel.nodes.len() + sel.edges.len();
        if total == 0 {
            return None;
        }
        Some(match names.len() {
            0 => format!("{total} selected"),
            _ if total == 1 => names.remove(0),
            _ => format!("{} and {} more", names.remove(0), total - 1),
        })
    }

    /// Put a suggestion in the ask bar, ready to send or change.
    fn fill_ask(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.f_ask.update(cx, |s, cx| s.set_value(text, window, cx));
        let input = self.f_ask.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    fn canvas_area(&self, scene: Scene, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let bounds = self.bounds.clone();
        let hand = self.tool == Tool::Hand || self.space;
        let status = {
            let v = self.ed.cur();
            let cycles = scene.cycles;
            let mut parts = vec![
                format!("{} shapes", v.nodes.len()),
                format!("{} connectors", v.edges.len()),
            ];
            if cycles > 0 {
                parts.push(format!("{cycles} on a dependency cycle (red)"));
            }
            parts.push(if self.ed.dirty {
                "Saving…".to_string()
            } else if self.saved_at.is_some() {
                format!("Saved to {FOLDER}/")
            } else if exists(&self.root) {
                format!("{FOLDER}/")
            } else {
                "Not saved yet".to_string()
            });
            parts.join(" · ")
        };
        let notice = self
            .notice
            .as_ref()
            .filter(|(_, at, err)| *err || at.elapsed() < Duration::from_secs(8))
            .map(|(t, _, err)| (t.clone(), *err));
        let mut area = div()
            .id("arch-canvas")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(theme.canvas))
            .when(hand, |el| el.cursor_grab())
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right))
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
                    move |b, _, window, _| paint_scene(b, &scene, window),
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children(self.labels(theme, cx))
            .children(self.inline_view())
            .children(self.empty_state(cx))
            .children(self.thinking())
            .children(self.minimap(cx))
            .children(self.player(cx))
            .children(self.breadcrumbs(cx))
            .children(self.find_bar(cx))
            .children(self.removed_labels())
            .child(self.ask_bar(cx));
        if let Some((text, err)) = notice {
            area = area.child(
                div()
                    .absolute()
                    .top(px(10.))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("arch-notice")
                            .max_w(px(560.))
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(if err { c_red() } else { c_border() }))
                            .bg(float(c_canvas()))
                            .shadow_md()
                            .text_size(px(12.5))
                            .text_color(rgb(if err { c_red() } else { c_text() }))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notice = None;
                                cx.notify();
                            }))
                            .child(text),
                    ),
            );
        }
        if let Some(e) = &self.load_error {
            area = area.child(
                div()
                    .absolute()
                    .top(px(52.))
                    .left(px(12.))
                    .right(px(12.))
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_red()))
                    .bg(float(c_canvas()))
                    .text_size(px(12.5))
                    .text_color(rgb(c_red()))
                    .child(format!("Couldn't read the saved diagram, so this is a blank one. Nothing is saved until you change it. {e}")),
            );
        }
        if let Some(at) = self.quick {
            area = area.child(self.quick_view(at, cx));
        }
        if let Some((at, menu)) = &self.menu {
            area = area.child(self.menu_view(*at, menu, cx));
        }
        if self.export_open {
            area = area.child(self.export_menu(cx));
        }
        if self.beautify_open {
            area = area.child(self.beautify_menu(cx));
        }
        if let Some(panel) = self.check_panel(cx) {
            area = area.child(panel);
        }
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

    fn swatches(
        &self,
        id: &'static str,
        current: Option<u32>,
        set: fn(&mut DStyle, Option<String>),
        cx: &mut Context<Self>,
    ) -> Div {
        let mut row = div().flex().flex_wrap().gap(px(5.));
        row =
            row.child(
                div()
                    .id((id, 99usize))
                    .size(px(20.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(tip("Default"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.act(cx, |ed| ed.style(|s| set(s, None)))
                    }))
                    .child(icon(IconName::X, 10., c_muted())),
            );
        for (i, c) in shape::SWATCHES.iter().enumerate() {
            let c = *c;
            row = row.child(
                div()
                    .id((id, i))
                    .size(px(20.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(if current == Some(c) {
                        c_blue()
                    } else {
                        c_border()
                    }))
                    .when(current == Some(c), |el| el.border_2())
                    .bg(rgb(c))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.act(cx, |ed| ed.style(|s| set(s, Some(hex(c)))))
                    })),
            );
        }
        row
    }

    fn style_section(&self, style: &DStyle, edge: bool, cx: &mut Context<Self>) -> Div {
        let fill = style.fill.as_deref().and_then(parse_hex);
        let stroke = style.stroke.as_deref().and_then(parse_hex);
        let text = style.text.as_deref().and_then(parse_hex);
        let mut out = div().v_flex().gap_2().child(section("Style"));
        if !edge {
            out = out.child(label_row(
                "Fill",
                self.swatches("arch-fill", fill, |s, v| s.fill = v, cx),
            ));
        }
        out = out.child(label_row(
            if edge { "Colour" } else { "Line" },
            self.swatches("arch-stroke", stroke, |s, v| s.stroke = v, cx),
        ));
        if !edge {
            out = out.child(label_row(
                "Text",
                self.swatches("arch-text", text, |s, v| s.text = v, cx),
            ));
            let font = style.font.unwrap_or(14.);
            let radius = style.radius;
            out = out.child(
                div()
                    .flex()
                    .gap_3()
                    .child(label_row(
                        "Text size",
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                icon_btn("arch-font-down", IconName::Minus, "Smaller", false)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.act(cx, |ed| {
                                            ed.style(|s| {
                                                s.font = Some((s.font.unwrap_or(14.) - 1.).max(8.))
                                            })
                                        })
                                    })),
                            )
                            .child(
                                div()
                                    .w(px(28.))
                                    .text_center()
                                    .text_size(px(12.))
                                    .child(format!("{font:.0}")),
                            )
                            .child(
                                icon_btn("arch-font-up", IconName::Plus, "Larger", false).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.act(cx, |ed| {
                                            ed.style(|s| {
                                                s.font = Some((s.font.unwrap_or(14.) + 1.).min(72.))
                                            })
                                        })
                                    }),
                                ),
                            ),
                    ))
                    .child(label_row(
                        "Corners",
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                chip_btn("arch-rad-0", "Square", radius == Some(0.)).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.act(cx, |ed| ed.style(|s| s.radius = Some(0.)))
                                    }),
                                ),
                            )
                            .child(chip_btn("arch-rad-d", "Auto", radius.is_none()).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.act(cx, |ed| ed.style(|s| s.radius = None))
                                }),
                            ))
                            .child(
                                chip_btn("arch-rad-r", "Round", radius == Some(24.)).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.act(cx, |ed| ed.style(|s| s.radius = Some(24.)))
                                    }),
                                ),
                            ),
                    )),
            );
            let opacity = style.opacity.unwrap_or(1.);
            let mut ops = div().flex().gap_1();
            for (i, o) in [1., 0.75, 0.5, 0.25].into_iter().enumerate() {
                ops = ops.child(
                    chip_btn(
                        ("arch-op", i),
                        format!("{}%", (o * 100.) as u32),
                        (opacity - o).abs() < 0.01,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.act(cx, |ed| {
                            ed.style(|s| s.opacity = if o >= 1. { None } else { Some(o) })
                        })
                    })),
                );
            }
            out = out.child(label_row("Opacity", ops));
        }
        out.child(
            div()
                .flex()
                .gap_1()
                .child(
                    chip_btn("arch-dashed", "Dashed", style.dashed).on_click(cx.listener(
                        |this, _, _, cx| this.act(cx, |ed| ed.style(|s| s.dashed = !s.dashed)),
                    )),
                )
                .when(!edge, |el| {
                    el.child(
                        chip_btn("arch-bold", "Bold", style.bold).on_click(cx.listener(
                            |this, _, _, cx| this.act(cx, |ed| ed.style(|s| s.bold = !s.bold)),
                        )),
                    )
                })
                .child(
                    chip_btn("arch-copy-style", "Copy style", false).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.ed.copy_style();
                            this.note(
                                "Style copied. Select shapes and press Paste style.".into(),
                                false,
                            );
                            cx.notify();
                        },
                    )),
                )
                .when(self.ed.style_clip.is_some(), |el| {
                    el.child(chip_btn("arch-paste-style", "Paste style", false).on_click(
                        cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.paste_style())),
                    ))
                }),
        )
    }

    fn properties(&self, cx: &mut Context<Self>) -> Div {
        let mut body = div().v_flex().gap_2();
        let v = self.ed.cur();
        if let Some(id) = self.one_node() {
            let (Some(e), Some(n)) = (self.ed.doc.element(&id), v.node(&id)) else {
                return body;
            };
            body = body
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(kind_icon(e.kind), 15., shape::accent(e.kind)))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(e.kind.label()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .font_family("monospace")
                                .child(e.id.clone()),
                        ),
                )
                .child(label_row(
                    "Name",
                    Input::new(&self.f_name).bordered(true).h(px(28.)),
                ));
            let mut kinds = div().flex().flex_wrap().gap(px(4.));
            for (i, k) in Kind::ALL.iter().enumerate() {
                let k = *k;
                kinds = kinds.child(
                    div()
                        .id(("arch-kind", i))
                        .size(px(26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if e.kind == k { c_blue() } else { c_border() }))
                        .when(e.kind == k, |el| el.bg(tint(c_selected())))
                        .cursor_pointer()
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(k.label()).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let ids = this.ed.sel.nodes.clone();
                            this.act(cx, |ed| {
                                for id in &ids {
                                    if let Some(e) = ed.doc.element_mut(id) {
                                        e.kind = k;
                                    }
                                }
                            });
                        }))
                        .child(icon(
                            kind_icon(k),
                            13.,
                            if e.kind == k { c_text() } else { c_secondary() },
                        )),
                );
            }
            body = body.child(label_row("Kind", kinds));
            if e.kind != Kind::Text && e.kind != Kind::Note {
                body = body.child(label_row(
                    "Technology",
                    Input::new(&self.f_tech).bordered(true).h(px(28.)),
                ));
            }
            body = body.child(label_row(
                "Description",
                Textarea::new(&self.f_desc).h(px(64.)),
            ));
            if e.kind.is_part() || e.kind == Kind::Boundary {
                body = body
                    .child(label_row(
                        "Owner",
                        Input::new(&self.f_owner).bordered(true).h(px(28.)),
                    ))
                    .child(label_row(
                        "Tags",
                        Input::new(&self.f_tags).bordered(true).h(px(28.)),
                    ))
                    .child(label_row("Code", Textarea::new(&self.f_code).h(px(56.))));
                let has_code = self.code_file(&id).is_some();
                let id2 = id.clone();
                let id3 = id.clone();
                body = body.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .when(has_code, |el| {
                            el.child(
                                text_btn(
                                    "arch-open-code",
                                    "Open code",
                                    Some(IconName::CodeXml),
                                    false,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if let Some(f) = this.code_file(&id2) {
                                            cx.emit(ArchEvent::Open(f));
                                        }
                                    },
                                )),
                            )
                        })
                        .when(e.kind.is_part(), |el| {
                            el.child(
                                text_btn(
                                    "arch-ask",
                                    "Ask an agent",
                                    Some(IconName::Sparkles),
                                    false,
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.ask_about(&id3, cx)),
                                ),
                            )
                        }),
                );
                // Zoom into another diagram.
                let mut links = div().flex().flex_wrap().gap(px(4.));
                links = links.child(
                    chip_btn("arch-link-none", "None", e.link.is_empty()).on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(id) = this.one_node() {
                                this.act(cx, |ed| {
                                    if let Some(e) = ed.doc.element_mut(&id) {
                                        e.link.clear();
                                    }
                                });
                            }
                        },
                    )),
                );
                for (i, other) in self
                    .ed
                    .doc
                    .views
                    .iter()
                    .filter(|o| o.id != self.ed.view)
                    .enumerate()
                {
                    let vid = other.id.clone();
                    links = links.child(
                        chip_btn(("arch-link", i), other.name.clone(), e.link == other.id)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(id) = this.one_node() {
                                    let vid = vid.clone();
                                    this.act(cx, |ed| {
                                        if let Some(e) = ed.doc.element_mut(&id) {
                                            e.link = vid;
                                        }
                                    });
                                }
                            })),
                    );
                }
                body = body.child(label_row("Opens diagram (right-click it to open)", links));
            }
            if e.kind == Kind::Boundary {
                let collapsed = n.collapsed;
                let id = n.id.clone();
                body = body.child(label_row(
                    "Contents",
                    text_btn(
                        "arch-fold",
                        if collapsed {
                            "Expand"
                        } else {
                            "Collapse to one box"
                        },
                        Some(if collapsed {
                            IconName::Maximize
                        } else {
                            IconName::Minimize
                        }),
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let id = id.clone();
                        this.act(cx, |ed| ed.toggle_collapse(&id));
                    })),
                ));
            }
            body = body.child(self.layer_row(Some(n.layer.clone()), cx));
            body = body.child(self.effect_section(Some(n.effect), cx));
            body = body.child(self.style_section(&n.style.clone(), false, cx));
            body = body.child(section("Arrange")).child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        icon_btn(
                            "arch-front",
                            IconName::BringToFront,
                            "Bring to front",
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.order(true)))),
                    )
                    .child(
                        icon_btn("arch-back", IconName::SendToBack, "Send to back", false)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.order(false))),
                            ),
                    )
                    .child(
                        icon_btn(
                            "arch-lock",
                            if n.locked {
                                IconName::Lock
                            } else {
                                IconName::LockOpen
                            },
                            "Lock in place",
                            n.locked,
                        )
                        .on_click(
                            cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.toggle_lock())),
                        ),
                    )
                    .child(
                        icon_btn("arch-dup", IconName::Copy, "Duplicate", false).on_click(
                            cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.duplicate())),
                        ),
                    )
                    .child(
                        icon_btn("arch-del", IconName::Trash, "Delete", false)
                            .on_click(cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.delete()))),
                    ),
            );
        } else if let Some(id) = self.one_edge() {
            let (Some(r), Some(e)) = (self.ed.doc.relation(&id), v.edge(&id)) else {
                return body;
            };
            let name = |x: &str| {
                self.ed
                    .doc
                    .element(x)
                    .map(|e| e.name.clone())
                    .unwrap_or_default()
            };
            body = body
                .child(
                    div()
                        .v_flex()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Connector"),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .child(format!("{} → {}", name(&r.from), name(&r.to))),
                        ),
                )
                .child(label_row(
                    "Label",
                    Input::new(&self.f_name).bordered(true).h(px(28.)),
                ))
                .child(label_row(
                    "Technology",
                    Input::new(&self.f_tech).bordered(true).h(px(28.)),
                ));
            let mut routes = div().flex().gap_1();
            for (i, (label, route)) in [
                ("Straight", Route::Straight),
                ("Elbow", Route::Elbow),
                ("Curved", Route::Curved),
            ]
            .into_iter()
            .enumerate()
            {
                routes = routes.child(
                    chip_btn(("arch-route", i), label, e.route == route).on_click(cx.listener(
                        move |this, _, _, cx| this.act(cx, |ed| ed.edges(|e| e.route = route)),
                    )),
                );
            }
            let mut arrows = div().flex().gap_1();
            for (i, (label, arrow)) in [
                ("One way", Arrow::End),
                ("Both ways", Arrow::Both),
                ("No arrow", Arrow::None),
            ]
            .into_iter()
            .enumerate()
            {
                arrows = arrows.child(
                    chip_btn(("arch-arrow", i), label, e.arrow == arrow).on_click(cx.listener(
                        move |this, _, _, cx| this.act(cx, |ed| ed.edges(|e| e.arrow = arrow)),
                    )),
                );
            }
            let ends = |from: bool, current: Option<Side>, cx: &mut Context<Self>| {
                let mut row = div().flex().flex_wrap().gap_1();
                let opts = std::iter::once(None).chain(Side::ALL.into_iter().map(Some));
                for (i, side) in opts.enumerate() {
                    let label = side.map(|s| s.label()).unwrap_or("Auto");
                    row = row.child(
                        chip_btn(
                            (
                                if from {
                                    "arch-from-side"
                                } else {
                                    "arch-to-side"
                                },
                                i,
                            ),
                            label,
                            current == side,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.act(cx, |ed| {
                                if from {
                                    ed.pin_sides(Some(side), None)
                                } else {
                                    ed.pin_sides(None, Some(side))
                                }
                            })
                        })),
                    );
                }
                row
            };
            let custom = !e.waypoints.is_empty() || e.from_side.is_some() || e.to_side.is_some();
            body = body
                .child(label_row("Path", routes))
                .child(label_row("Leaves from", ends(true, e.from_side, cx)))
                .child(label_row("Enters at", ends(false, e.to_side, cx)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.5))
                                .text_color(rgb(c_secondary()))
                                .child(if e.route == Route::Curved {
                                    "Elbow and straight connectors can be bent by hand."
                                } else {
                                    "Drag a square on the line to bend it. Elbows go round boxes on their own."
                                }),
                        )
                        .when(custom, |el| {
                            el.child(
                                text_btn("arch-reset-path", "Reset path", Some(IconName::Undo2), false)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.act(cx, |ed| ed.reset_paths())
                                    })),
                            )
                        }),
                )
                .child(label_row("Arrows", arrows))
                .child(self.motion_section(Some(e.motion), Some(e.speed), cx))
                .child(self.style_section(&e.style.clone(), true, cx))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .pt_2()
                        .child(
                            text_btn(
                                "arch-reverse",
                                "Reverse",
                                Some(IconName::ArrowLeftRight),
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let ids = this.ed.sel.edges.clone();
                                this.act(cx, |ed| {
                                    for id in &ids {
                                        if let Some(r) = ed.doc.relation_mut(id) {
                                            std::mem::swap(&mut r.from, &mut r.to);
                                        }
                                    }
                                });
                            })),
                        )
                        .child(
                            text_btn("arch-edge-del", "Delete", Some(IconName::Trash), false)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.delete())),
                                ),
                        ),
                );
        } else if !self.ed.sel.is_empty() {
            let n = self.ed.sel.nodes.len();
            let m = self.ed.sel.edges.len();
            body = body.child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(match (n, m) {
                        (_, 0) => format!("{n} shapes"),
                        (0, _) => format!("{m} connectors"),
                        _ => format!("{n} shapes, {m} connectors"),
                    }),
            );
            if n >= 2 {
                let al = |id: &'static str,
                          glyph: IconName,
                          t: &'static str,
                          how: Align,
                          cx: &mut Context<Self>| {
                    icon_btn(id, glyph, t, false).on_click(
                        cx.listener(move |this, _, _, cx| this.act(cx, |ed| ed.align(how))),
                    )
                };
                body = body
                    .child(section("Align"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(al(
                                "arch-al-l",
                                IconName::AlignStartVertical,
                                "Align left",
                                Align::Left,
                                cx,
                            ))
                            .child(al(
                                "arch-al-c",
                                IconName::AlignCenterVertical,
                                "Align centres",
                                Align::Center,
                                cx,
                            ))
                            .child(al(
                                "arch-al-r",
                                IconName::AlignEndVertical,
                                "Align right",
                                Align::Right,
                                cx,
                            ))
                            .child(al(
                                "arch-al-t",
                                IconName::AlignStartHorizontal,
                                "Align top",
                                Align::Top,
                                cx,
                            ))
                            .child(al(
                                "arch-al-m",
                                IconName::AlignCenterHorizontal,
                                "Align middles",
                                Align::Middle,
                                cx,
                            ))
                            .child(al(
                                "arch-al-b",
                                IconName::AlignEndHorizontal,
                                "Align bottom",
                                Align::Bottom,
                                cx,
                            ))
                            .child(al(
                                "arch-sp-x",
                                IconName::AlignHorizontalDistributeCenter,
                                "Spread evenly across",
                                Align::SpreadX,
                                cx,
                            ))
                            .child(al(
                                "arch-sp-y",
                                IconName::AlignVerticalDistributeCenter,
                                "Spread evenly down",
                                Align::SpreadY,
                                cx,
                            ))
                            .child(al(
                                "arch-same",
                                IconName::Scaling,
                                "Same size as the first",
                                Align::SameSize,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .pt_1()
                            .child(
                                text_btn("arch-group", "Group", Some(IconName::Group), false)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.act(cx, |ed| {
                                            ed.group();
                                        })
                                    })),
                            )
                            .child(
                                text_btn(
                                    "arch-tidy-sel",
                                    "Tidy these",
                                    Some(IconName::Network),
                                    false,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        let h = this.horizontal;
                                        this.act(cx, |ed| ed.auto_layout(h));
                                    },
                                )),
                            ),
                    );
            }
            let first = self
                .ed
                .sel
                .nodes
                .first()
                .and_then(|id| v.node(id))
                .map(|n| n.style.clone())
                .unwrap_or_default();
            if n > 0 {
                let effect = self
                    .ed
                    .sel
                    .nodes
                    .first()
                    .and_then(|id| v.node(id))
                    .map(|n| n.effect);
                body = body.child(self.effect_section(effect, cx));
                let layer = self
                    .ed
                    .sel
                    .nodes
                    .first()
                    .and_then(|id| v.node(id))
                    .map(|n| n.layer.clone());
                body = body.child(self.layer_row(layer, cx));
            }
            if m > 0 {
                let e = self.ed.sel.edges.first().and_then(|id| v.edge(id));
                body = body.child(self.motion_section(e.map(|e| e.motion), e.map(|e| e.speed), cx));
            }
            body = body.child(self.style_section(&first, n == 0, cx)).child(
                div()
                    .flex()
                    .gap_1()
                    .pt_2()
                    .child(
                        icon_btn("arch-mlock", IconName::Lock, "Lock or unlock", false).on_click(
                            cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.toggle_lock())),
                        ),
                    )
                    .child(
                        icon_btn("arch-mdup", IconName::Copy, "Duplicate", false).on_click(
                            cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.duplicate())),
                        ),
                    )
                    .child(
                        icon_btn("arch-mdel", IconName::Trash, "Delete", false)
                            .on_click(cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.delete()))),
                    ),
            );
        } else {
            let parts = self
                .ed
                .doc
                .elements
                .iter()
                .filter(|e| e.kind.is_part())
                .count();
            body = body
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Diagram"))
                .child(label_row("Name", Input::new(&self.f_view).bordered(true).h(px(28.))))
                .child(div().text_size(px(12.)).text_color(rgb(c_secondary())).child(format!(
                    "{} shapes and {} connectors here. The model has {parts} parts across {} diagram{}.",
                    v.nodes.len(),
                    v.edges.len(),
                    self.ed.doc.views.len(),
                    if self.ed.doc.views.len() == 1 { "" } else { "s" }
                )))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(text_btn("arch-dup-view", "Duplicate diagram", Some(IconName::Copy), false).on_click(cx.listener(|this, _, _, cx| this.act(cx, |ed| {
                            ed.duplicate_view();
                        }))))
                        .when(self.ed.doc.views.len() > 1, |el| {
                            el.child(text_btn("arch-del-view", "Delete diagram", Some(IconName::Trash), false).on_click(cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.delete_view()))))
                        }),
                )
                .child(self.layers_section(cx))
                .child(self.stories_section(cx))
                .child(section("Shortcuts"))
                .child(
                    div()
                        .v_flex()
                        .gap(px(3.))
                        .text_size(px(11.5))
                        .text_color(rgb(c_secondary()))
                        .children(
                            [
                                ("Add a shape", "Double-click, or / "),
                                ("Add a connected shape", "Tab"),
                                ("Rename", "Enter or double-click"),
                                ("Connect", "Drag from a side dot"),
                                ("Copy while dragging", "Alt + drag"),
                                ("Move freely", "Hold Alt"),
                                ("Pan", "Space + drag, or scroll"),
                                ("Zoom", "⌘ + scroll, pinch"),
                                ("Fit all / selection", "⇧1 / ⇧2"),
                                ("Group / ungroup", "⌘G / ⇧⌘G"),
                                ("Tidy layout", "⇧A"),
                            ]
                            .into_iter()
                            .map(|(a, b)| {
                                div()
                                    .flex()
                                    .child(div().flex_1().child(a))
                                    .child(div().text_color(rgb(c_muted())).child(b))
                            }),
                        ),
                );
        }
        body
    }

    /// How the selected connectors move.
    fn motion_section(
        &self,
        motion: Option<Motion>,
        speed: Option<Speed>,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut kinds = div().flex().flex_wrap().gap_1();
        for (i, m) in Motion::ALL.into_iter().enumerate() {
            kinds = kinds.child(
                chip_btn(("arch-motion", i), m.label(), motion == Some(m)).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.act(cx, |ed| ed.edges(|e| e.motion = m))
                    }),
                ),
            );
        }
        let mut speeds = div().flex().gap_1();
        for (i, sp) in Speed::ALL.into_iter().enumerate() {
            speeds = speeds.child(
                chip_btn(("arch-speed", i), sp.label(), speed == Some(sp)).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.act(cx, |ed| ed.edges(|e| e.speed = sp))
                    }),
                ),
            );
        }
        let mut out = div()
            .v_flex()
            .gap_2()
            .child(section("Motion"))
            .child(kinds)
            .when(motion != Some(Motion::None), |el| {
                el.child(label_row("Speed", speeds))
            });
        if self.ed.cur().still {
            out = out.child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("Motion is off for this diagram. Turn it on in the toolbar."),
            );
        }
        let stories = &self.ed.cur().stories;
        let mut story_row = div().flex().flex_wrap().gap_1().child(
            text_btn(
                "arch-story-new",
                "New story from these",
                Some(IconName::Play),
                false,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                let n = this.ed.cur().stories.len() + 1;
                let mut made = None;
                this.act(cx, |ed| {
                    made = ed.story_from_selection(&format!("Story {n}"))
                });
                if let Some(i) = made {
                    this.note(format!("Made Story {n}. Press Play to watch it."), false);
                    this.play(i, cx);
                }
            })),
        );
        for (i, st) in stories.iter().enumerate() {
            story_row = story_row.child(
                chip_btn(("arch-story-add", i), format!("Add to {}", st.name), false).on_click(
                    cx.listener(move |this, _, _, cx| this.act(cx, |ed| ed.extend_story(i))),
                ),
            );
        }
        out.child(label_row(
            "Story (an animated walk through the diagram)",
            story_row,
        ))
    }

    /// How the selected shapes draw the eye.
    /// Which layer the selected shapes are on, with a new one a click away.
    fn layer_row(&self, current: Option<Option<String>>, cx: &mut Context<Self>) -> Div {
        let v = self.ed.cur();
        let mut row = div().flex().flex_wrap().gap_1().child(
            chip_btn("arch-layer-base", "Base", current == Some(None))
                .on_click(cx.listener(|this, _, _, cx| this.act(cx, |ed| ed.set_layer(None)))),
        );
        for (i, l) in v.layers.iter().enumerate() {
            if self.layer_edit == Some(i) {
                row = row.child(
                    div()
                        .w(px(140.))
                        .child(Input::new(&self.f_layer).bordered(true).h(px(26.))),
                );
                continue;
            }
            let name = l.name.clone();
            let on = current
                .as_ref()
                .is_some_and(|c| c.as_deref() == Some(name.as_str()));
            row = row.child(
                chip_btn(("arch-layer-pick", i), l.name.clone(), on).on_click(cx.listener(
                    move |this, _, _, cx| {
                        let name = name.clone();
                        this.act(cx, |ed| ed.set_layer(Some(&name)))
                    },
                )),
            );
        }
        row = row.child(
            chip_btn("arch-layer-new", "+ New layer", false).on_click(cx.listener(
                |this, _, window, cx| {
                    let mut name = String::new();
                    this.act(cx, |ed| name = ed.add_layer("Layer"));
                    let i = this.ed.cur().layers.iter().position(|l| l.name == name);
                    if let Some(i) = i {
                        this.edit_layer(i, String::new(), window, cx);
                    }
                },
            )),
        );
        div().v_flex().gap_2().child(section("Layer")).child(row)
    }

    /// The diagram's layers: show or hide, lock, rename, remove.
    fn layers_section(&self, cx: &mut Context<Self>) -> Div {
        let v = self.ed.cur();
        let mut out = div().v_flex().gap_2().child(section("Layers"));
        if v.layers.is_empty() {
            return out.child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .child("Put shapes on layers such as infra or data flows to hide or lock them together. Select shapes, then pick New layer."),
            );
        }
        for (i, l) in v.layers.iter().enumerate() {
            let count = v
                .nodes
                .iter()
                .filter(|n| n.layer.as_deref() == Some(l.name.as_str()))
                .count();
            let (hidden, locked) = (l.hidden, l.locked);
            let renaming = self.layer_edit == Some(i);
            out = out.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        icon_btn(
                            ("arch-layer-eye", i),
                            if hidden {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            },
                            if hidden { "Show" } else { "Hide" },
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.act(cx, |ed| ed.layer_flags(i, Some(!hidden), None))
                        })),
                    )
                    .child(
                        icon_btn(
                            ("arch-layer-lock", i),
                            if locked {
                                IconName::Lock
                            } else {
                                IconName::LockOpen
                            },
                            if locked { "Unlock" } else { "Lock" },
                            locked,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.act(cx, |ed| ed.layer_flags(i, None, Some(!locked)))
                        })),
                    )
                    .child(if renaming {
                        div()
                            .flex_1()
                            .child(Input::new(&self.f_layer).bordered(true).h(px(26.)))
                            .into_any_element()
                    } else {
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .text_color(rgb(if hidden { c_muted() } else { c_text() }))
                            .child(format!("{} · {count}", l.name))
                            .into_any_element()
                    })
                    .child(
                        icon_btn(("arch-layer-ren", i), IconName::Pencil, "Rename", false)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name = this
                                    .ed
                                    .cur()
                                    .layers
                                    .get(i)
                                    .map(|l| l.name.clone())
                                    .unwrap_or_default();
                                this.edit_layer(i, name, window, cx);
                            })),
                    )
                    .child(
                        icon_btn(
                            ("arch-layer-del", i),
                            IconName::Trash,
                            "Remove layer (its shapes stay)",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layer_edit = None;
                            this.act(cx, |ed| ed.delete_layer(i))
                        })),
                    ),
            );
        }
        out
    }

    /// Keep a layer name being typed when something else is clicked.
    fn commit_layer_name(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.layer_edit.take() {
            let v = self.f_layer.read(cx).value().to_string();
            self.ed.rename_layer(i, &v);
        }
    }

    /// Rename a layer in place, starting from `name` (empty keeps the name it has).
    fn edit_layer(&mut self, i: usize, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layer_edit = Some(i);
        self.f_layer
            .update(cx, |s, cx| s.set_value(name, window, cx));
        let input = self.f_layer.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    fn effect_section(&self, effect: Option<Effect>, cx: &mut Context<Self>) -> Div {
        let mut row = div().flex().flex_wrap().gap_1();
        for (i, fx) in Effect::ALL.into_iter().enumerate() {
            row = row.child(
                chip_btn(("arch-effect", i), fx.label(), effect == Some(fx))
                    .on_click(cx.listener(move |this, _, _, cx| this.act(cx, |ed| ed.effect(fx)))),
            );
        }
        div().v_flex().gap_2().child(section("Effect")).child(row)
    }

    /// The diagram's stories, to play or remove.
    fn stories_section(&self, cx: &mut Context<Self>) -> Div {
        let v = self.ed.cur();
        let mut out = div().v_flex().gap_2().child(section("Stories"));
        if v.stories.is_empty() {
            return out.child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .child("Select the connectors a request travels, then choose New story from these. Play shows it step by step."),
            );
        }
        for (i, st) in v.stories.iter().enumerate() {
            let on = self.playing.is_some_and(|p| p.story == i);
            let open = self.open_story == Some(i) || on;
            let renaming = self.story_edit == Some((i, None));
            out = out.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        icon_btn(
                            ("arch-story-play", i),
                            if on { IconName::Square } else { IconName::Play },
                            if on { "Stop" } else { "Play" },
                            on,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.playing.is_some_and(|p| p.story == i) {
                                this.playing = None;
                            } else {
                                this.play(i, cx);
                            }
                            cx.notify();
                        })),
                    )
                    .child(if renaming {
                        div()
                            .flex_1()
                            .child(Input::new(&self.f_story).bordered(true).h(px(26.)))
                            .into_any_element()
                    } else {
                        div()
                            .id(("arch-story-name", i))
                            .flex_1()
                            .min_w_0()
                            .cursor_pointer()
                            .text_size(px(12.5))
                            .child(format!("{} · {} steps", st.name, st.steps.len()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_story = if this.open_story == Some(i) {
                                    None
                                } else {
                                    Some(i)
                                };
                                cx.notify();
                            }))
                            .into_any_element()
                    })
                    .child(
                        icon_btn(("arch-story-ren", i), IconName::Pencil, "Rename", false)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let name = this
                                    .ed
                                    .cur()
                                    .stories
                                    .get(i)
                                    .map(|s| s.name.clone())
                                    .unwrap_or_default();
                                this.edit_story(i, None, name, window, cx);
                            })),
                    )
                    .child(
                        icon_btn(
                            ("arch-story-del", i),
                            IconName::Trash,
                            "Delete story",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.playing.is_some_and(|p| p.story == i) {
                                this.playing = None;
                            }
                            this.open_story = None;
                            this.act(cx, |ed| ed.delete_story(i));
                        })),
                    ),
            );
            if !open {
                continue;
            }
            let current = self
                .playing
                .filter(|p| p.story == i)
                .and_then(|p| motion::step_at(p.elapsed(), st.steps.len()))
                .map(|s| s.0);
            for (j, step) in st.steps.iter().enumerate() {
                let editing = self.story_edit == Some((i, Some(j)));
                let caption = if step.caption.is_empty() {
                    self.ed
                        .doc
                        .relation(&step.edge)
                        .map(|r| {
                            let n = |id: &str| {
                                self.ed
                                    .doc
                                    .element(id)
                                    .map(|e| e.name.clone())
                                    .unwrap_or_default()
                            };
                            format!("{} → {}", n(&r.from), n(&r.to))
                        })
                        .unwrap_or_default()
                } else {
                    step.caption.clone()
                };
                out = out.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .pl(px(30.))
                        .child(
                            div()
                                .w(px(16.))
                                .text_size(px(11.))
                                .text_color(rgb(if current == Some(j) {
                                    0xf59e0b
                                } else {
                                    c_muted()
                                }))
                                .child(format!("{}", j + 1)),
                        )
                        .child(if editing {
                            div()
                                .flex_1()
                                .child(Input::new(&self.f_story).bordered(true).h(px(26.)))
                                .into_any_element()
                        } else {
                            div()
                                .id(("arch-step", i * 1000 + j))
                                .flex_1()
                                .min_w_0()
                                .cursor_pointer()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .hover(|el| el.text_color(rgb(c_text())))
                                .child(caption.clone())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_story(i, Some(j), caption.clone(), window, cx);
                                }))
                                .into_any_element()
                        })
                        .child(
                            icon_btn(
                                ("arch-step-del", i * 1000 + j),
                                IconName::X,
                                "Remove step",
                                false,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.act(cx, |ed| ed.remove_step(i, j)),
                            )),
                        ),
                );
            }
        }
        out
    }

    /// Edit a story's name (no step) or a step's caption in place.
    fn edit_story(
        &mut self,
        i: usize,
        step: Option<usize>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.story_edit = Some((i, step));
        self.f_story
            .update(cx, |s, cx| s.set_value(text, window, cx));
        let input = self.f_story.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    /// Open the find bar with what's selected as the starting text.
    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = true;
        let input = self.f_find.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        cx.notify();
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.found.clear();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Show the current match: open what folds it away, select it and bring it into view.
    fn show_found(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.found.get(self.found_at).cloned() else {
            cx.notify();
            return;
        };
        if !self.ed.sight().shows(&id) {
            self.ed.reveal(&id);
            self.changed(cx);
        }
        self.ed.sel = Sel {
            nodes: vec![id.clone()],
            edges: Vec::new(),
        };
        if let Some(r) = self.ed.rect(&id) {
            let vis = self.visible_world().inflate(-40. / self.zoom);
            if !vis.contains_rect(&r) {
                if self.zoom < 0.6 {
                    self.zoom = 0.8;
                }
                let (vw, vh) = self.view_size();
                let (cx_, cy_) = r.center();
                self.pan = (vw / 2. - cx_ * self.zoom, vh / 2. - cy_ * self.zoom);
            }
        }
        cx.notify();
    }

    /// Move to the next (or previous) match.
    fn step_found(&mut self, by: i32, cx: &mut Context<Self>) {
        let n = self.found.len() as i32;
        if n == 0 {
            return;
        }
        self.found_at = (self.found_at as i32 + by).rem_euclid(n) as usize;
        self.show_found(cx);
    }

    /// The find bar over the canvas.
    fn find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find_open {
            return None;
        }
        let q = self.f_find.read(cx).value().to_string();
        let count = if q.trim().is_empty() {
            String::new()
        } else if self.found.is_empty() {
            "No matches".into()
        } else {
            format!("{} of {}", self.found_at + 1, self.found.len())
        };
        Some(
            div()
                .absolute()
                .top(px(10.))
                .right(px(12.))
                .w(px(360.))
                .p_1()
                .flex()
                .items_center()
                .gap_1()
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(float(c_canvas()))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div().flex_1().child(
                        Input::new(&self.f_find)
                            .bordered(false)
                            .h(px(28.))
                            .prefix(icon(IconName::Search, 13., c_muted())),
                    ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(count),
                )
                .child(
                    icon_btn(
                        "arch-find-prev",
                        IconName::ChevronUp,
                        "Previous match",
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.step_found(-1, cx))),
                )
                .child(
                    icon_btn(
                        "arch-find-next",
                        IconName::ChevronDown,
                        "Next match (Enter)",
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.step_found(1, cx))),
                )
                .child(
                    icon_btn("arch-find-close", IconName::X, "Close (Esc)", false)
                        .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
                )
                .into_any_element(),
        )
    }

    /// The way back up from a diagram opened by drilling into a shape.
    fn breadcrumbs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.trail.is_empty() {
            return None;
        }
        let name = |id: &str| {
            self.ed
                .doc
                .views
                .iter()
                .find(|v| v.id == id)
                .map(|v| v.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let mut row = div()
            .absolute()
            .top(px(10.))
            .left(px(12.))
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .gap_1()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(float(c_canvas()))
            .text_size(px(12.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        for (i, id) in self.trail.iter().enumerate() {
            row = row
                .child(
                    div()
                        .id(("arch-crumb", i))
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                        .child(name(id))
                        .on_click(cx.listener(move |this, _, _, cx| this.back_to(i, cx))),
                )
                .child(icon(IconName::ChevronRight, 12., c_muted()));
        }
        Some(
            row.child(
                div()
                    .px_1()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_text()))
                    .child(name(&self.ed.view)),
            )
            .into_any_element(),
        )
    }

    /// Start playing a story from its first step.
    fn play(&mut self, story: usize, cx: &mut Context<Self>) {
        self.playing = Some(Playing {
            story,
            start: Instant::now(),
            paused: None,
        });
        cx.notify();
    }

    /// Jump `by` steps in the playing story.
    fn step_story(&mut self, by: i32) {
        let Some(p) = self.playing.as_mut() else {
            return;
        };
        let steps = self
            .ed
            .cur()
            .stories
            .get(p.story)
            .map_or(0, |s| s.steps.len());
        let now = (p.elapsed() / motion::STEP_SECS) as i32;
        let to = (now + by).clamp(0, steps.saturating_sub(1) as i32) as f32 * motion::STEP_SECS;
        if p.paused.is_some() {
            p.paused = Some(to);
        } else {
            p.start = Instant::now() - Duration::from_secs_f32(to);
        }
    }

    fn pause_story(&mut self) {
        let Some(p) = self.playing.as_mut() else {
            return;
        };
        match p.paused.take() {
            Some(at) => p.start = Instant::now() - Duration::from_secs_f32(at),
            None => p.paused = Some(p.start.elapsed().as_secs_f32()),
        }
    }

    /// The bar under a playing story: where it is and its controls.
    fn player(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.playing?;
        let st = self.ed.cur().stories.get(p.story)?;
        let (i, _) = motion::step_at(p.elapsed(), st.steps.len())
            .unwrap_or((st.steps.len().saturating_sub(1), 1.));
        let step = st.steps.get(i)?;
        let caption = if step.caption.is_empty() {
            self.ed
                .doc
                .relation(&step.edge)
                .map(|r| {
                    let n = |id: &str| {
                        self.ed
                            .doc
                            .element(id)
                            .map(|e| e.name.clone())
                            .unwrap_or_default()
                    };
                    format!("{} → {}", n(&r.from), n(&r.to))
                })
                .unwrap_or_default()
        } else {
            step.caption.clone()
        };
        Some(
            div()
                .absolute()
                .bottom(px(56.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("arch-player")
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .bg(float(c_canvas()))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .v_flex()
                                .min_w(px(220.))
                                .max_w(px(420.))
                                .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child(
                                    format!("{} · step {} of {}", st.name, i + 1, st.steps.len()),
                                ))
                                .child(
                                    div()
                                        .text_size(px(13.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(caption),
                                ),
                        )
                        .child(
                            icon_btn(
                                "arch-play-prev",
                                IconName::ChevronLeft,
                                "Previous step",
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.step_story(-1);
                                cx.notify();
                            })),
                        )
                        .child(
                            icon_btn(
                                "arch-play-pause",
                                if p.paused.is_some() {
                                    IconName::Play
                                } else {
                                    IconName::Pause
                                },
                                "Pause or play",
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pause_story();
                                cx.notify();
                            })),
                        )
                        .child(
                            icon_btn("arch-play-next", IconName::ChevronRight, "Next step", false)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.step_story(1);
                                    cx.notify();
                                })),
                        )
                        .child(
                            icon_btn("arch-play-stop", IconName::X, "Stop", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.playing = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Repaint soon while something on the canvas moves.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(33))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.ticking = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Whether anything on the diagram moves now.
    fn animating(&self) -> bool {
        if self.playing.is_some() {
            return true;
        }
        let v = self.ed.cur();
        // Far out, motion is too small to see and only costs frames.
        !v.still
            && self.zoom >= MOTION_MIN_ZOOM
            && (v.edges.iter().any(|e| e.motion != Motion::None)
                || v.nodes.iter().any(|n| n.effect != Effect::None))
    }

    fn text_panel(&self, cx: &mut Context<Self>) -> Div {
        div()
            .v_flex()
            .gap_2()
            .child(div().text_size(px(12.)).text_color(rgb(c_secondary())).child(
                "The model and this diagram as the text saved in the repo. Edit and apply, or copy it into a review.",
            ))
            .child(label_row("architecture/model.toml", Textarea::new(&self.t_model).h(px(260.)).font_family("monospace").text_size(px(11.5))))
            .child(label_row(
                "This diagram",
                Textarea::new(&self.t_view).h(px(200.)).font_family("monospace").text_size(px(11.5)),
            ))
            .when_some(self.text_error.clone(), |el, e| {
                el.child(div().text_size(px(12.)).text_color(rgb(c_red())).child(e))
            })
            .child(
                div().flex().gap_1().child(text_btn("arch-text-apply", "Apply", Some(IconName::Check), true).on_click(cx.listener(|this, _, _, cx| {
                    let model = this.t_model.read(cx).value().to_string();
                    let view = this.t_view.read(cx).value().to_string();
                    match this.ed.apply_text(&model, &view) {
                        Ok(()) => {
                            this.text_error = None;
                            this.after_edit(cx);
                        }
                        Err(e) => {
                            this.text_error = Some(e);
                            cx.notify();
                        }
                    }
                }))).child(text_btn("arch-text-reset", "Revert", None, false).on_click(cx.listener(|this, _, _, cx| {
                    this.text_loaded = None;
                    cx.notify();
                }))),
            )
    }

    fn side_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab =
            |id: &'static str, label: &'static str, which: SideTab, cx: &mut Context<Self>| {
                let on = self.side == which;
                div()
                    .id(id)
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |el| {
                        el.bg(tint(c_selected())).font_weight(FontWeight::MEDIUM)
                    })
                    .text_color(rgb(if on { c_text() } else { c_secondary() }))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.side = which;
                        cx.notify();
                    }))
                    .child(label)
            };
        div()
            .w(px(290.))
            .flex_none()
            .h_full()
            .v_flex()
            .border_l_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_sidebar()))
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
                    .child(tab("arch-tab-props", "Properties", SideTab::Properties, cx))
                    .child(tab("arch-tab-text", "Text", SideTab::Text, cx)),
            )
            .child(
                div()
                    .id("arch-side")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .child(match self.side {
                        SideTab::Properties => self.properties(cx),
                        SideTab::Text => self.text_panel(cx),
                    }),
            )
            .into_any_element()
    }
}

impl Render for ArchEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((at, from)) = self.pending_quick.take() {
            self.open_quick(at, Some(from), window, cx);
        }
        if let Some(path) = self.pending_picture.take() {
            self.draw_picture(path, window, cx);
        }
        self.load_fields(window, cx);
        self.load_text(window, cx);
        if !self.fitted && self.view_size().0 > 10. {
            self.fit(false);
            self.fitted = true;
        }
        if let Some(p) = self.playing {
            let steps = self
                .ed
                .cur()
                .stories
                .get(p.story)
                .map_or(0, |s| s.steps.len());
            if p.paused.is_none() && motion::step_at(p.elapsed(), steps).is_none() {
                // Rest on the last step for a moment, then end.
                if p.elapsed() > steps as f32 * motion::STEP_SECS + 1.2 {
                    self.playing = None;
                }
            }
        }
        if self.animating() {
            self.tick(cx);
        }
        self.refresh_review();
        let theme = self.theme();
        let scene = self.scene(&theme);
        let ghost = match &self.drag {
            Drag::Place {
                preset,
                at: Some(at),
            } => Some((preset.kind.size(), preset.label.to_string(), *at)),
            Drag::Picture {
                name, at: Some(at), ..
            } => Some((Kind::Icon.size(), name.clone(), *at)),
            _ => None,
        };
        div()
            .size_full()
            .flex()
            .key_context("ArchEditor")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &CanvasEscape, window, cx| {
                this.on_bound_key("escape", Modifiers::default(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &CanvasTab, window, cx| {
                this.on_bound_key("tab", Modifiers::default(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &CanvasCopy, window, cx| {
                this.on_bound_key("c", Modifiers::secondary_key(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &CanvasFind, window, cx| this.open_find(window, cx)))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, cx| {
                if e.keystroke.key == "space" {
                    this.space = false;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &MouseUpEvent, _, cx| this.finish(e.position, cx)),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, e: &MouseUpEvent, _, cx| this.finish(e.position, cx)),
            )
            .child(self.palette(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .v_flex()
                    .child(self.toolbar(cx))
                    .child(self.canvas_area(scene, &theme, cx)),
            )
            .child(self.side_panel(cx))
            .when_some(ghost, |el, ((w, h), label, at)| {
                let z = self.zoom;
                el.child(deferred(
                    anchored()
                        .position(point(at.x - px(w * z / 2.), at.y - px(h * z / 2.)))
                        .child(
                            div()
                                .w(px(w * z))
                                .h(px(h * z))
                                .rounded_lg()
                                .border_2()
                                .border_dashed()
                                .border_color(rgb(c_blue()))
                                .bg(col(c_blue(), 0.08))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(12.))
                                .text_color(rgb(c_blue()))
                                .child(label),
                        ),
                ))
            })
    }
}

/// What the canvas says while it waits, by how long the agent has been working. The agent
/// doesn't report its steps, so this only names what it is usually doing by then.
fn thinking_line(secs: Option<u64>, scanning: bool) -> &'static str {
    match secs {
        _ if scanning => "Finding the packages and the services they use",
        None | Some(0..=3) => "Starting the agent",
        Some(4..=12) => "Reading the request and the diagram",
        Some(13..=30) => "Working out the parts and how they connect",
        Some(31..=60) => "Laying out the shapes",
        Some(_) => "Still working. Bigger diagrams take a little longer",
    }
}
