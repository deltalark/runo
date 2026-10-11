mod agent;
mod caps;
mod git;
mod history;
mod settings;
mod settings_ui;
mod store;
mod stream;

use agent::{Event, Handle, Target};
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use settings::*;
use settings_ui::SettingsPage;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;
use store::{Message, Note, Project, Role, Session, State, Status};

const DEMO_DIFF: &str = "@@ -42,13 +42,20 @@ impl NavItem {
     impl NavItem {
-    pub fn new(label: &str, icon: Icon) -> Self {
-        Self {
-            label: label.to_string(),
-            icon,
-        }
-    }
+    pub fn new(label: impl Into<String>, icon: Icon, active: bool) -> Self {
+        Self {
+            label: label.into(),
+            icon,
+            active,
+        }
+    }
 }

 pub struct Sidebar {
-    items: Vec<NavItem>,
+    items: Vec<NavItem>,
 }";

actions!(
    runo,
    [
        NewSession,
        OpenSettings,
        StopAgent,
        TogglePanel,
        ToggleSidebar,
        Dismiss,
        QuickOpen
    ]
);

/// Global shortcuts. `secondary` is ⌘ on macOS and Ctrl elsewhere.
fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewSession, Some("Runo")),
        KeyBinding::new("secondary-,", OpenSettings, Some("Runo")),
        KeyBinding::new("secondary-.", StopAgent, Some("Runo")),
        KeyBinding::new("secondary-b", TogglePanel, Some("Runo")),
        KeyBinding::new("secondary-\\", ToggleSidebar, Some("Runo")),
        KeyBinding::new("escape", Dismiss, Some("Runo")),
        KeyBinding::new("secondary-k", QuickOpen, Some("Runo")),
        KeyBinding::new("secondary-p", QuickOpen, Some("Runo")),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum View {
    Session,
    NewSession,
    Inbox,
    AllSessions,
    History,
    Worktrees,
    Settings,
}

/// What a quick-open entry does when chosen.
#[derive(Clone)]
enum Jump {
    /// Handled by the entry's own Action.
    Action,
    View(View),
    Session(u64),
    File(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum RightTab {
    Changes,
    Browser,
    Files,
}

struct RunoApp {
    state: State,
    demo: bool,
    selected: Option<u64>,
    view: View,
    right_tab: RightTab,
    cli_tab: bool,
    menu: Option<String>,
    settings_page: SettingsPage,
    settings_search: Entity<InputState>,
    settings_inputs: HashMap<String, Entity<InputState>>,
    settings_menu: Option<String>,
    discovered: Option<Discovered>,
    applied_dark: Option<bool>,
    applied_enter: Option<bool>,
    changes: Vec<git::Change>,
    dir_is_git: bool,
    selected_file: Option<String>,
    diff: String,
    files: Vec<String>,
    preview: Option<(String, String)>,
    terminal: HashMap<u64, Vec<String>>,
    handles: HashMap<Target, Handle>,
    tx: Sender<Event>,
    chat_scroll: ScrollHandle,
    stderr: HashMap<u64, Vec<String>>,
    /// Parsers for Claude runs, which print stream-json rather than plain text.
    streams: HashMap<u64, stream::ClaudeStream>,
    search: Entity<InputState>,
    prompt: Entity<TextareaState>,
    new_task: Entity<TextareaState>,
    branch: Entity<InputState>,
    term_input: Entity<InputState>,
    address: Entity<InputState>,
    new_provider: usize,
    new_project: Option<u64>,
    new_isolated: bool,
    /// Race mode in the New session form, and the agents picked for it.
    race_mode: bool,
    /// Past Claude and Codex conversations found on this machine.
    history: Vec<history::Past>,
    /// Change counts (files, added, removed) of each session racing the selected one.
    race_stats: HashMap<u64, (usize, usize, usize)>,
    new_race: Vec<usize>,
    expanded: Vec<u64>,
    /// Sessions open as tabs in the title bar, in tab order.
    tabs: Vec<u64>,
    /// Agent replies whose step list is expanded, by (session, message index).
    open_steps: std::collections::HashSet<(u64, usize)>,
    show_sidebar: bool,
    /// Change counts (files, added, removed) per session, for the sidebar.
    stats: HashMap<u64, (usize, usize, usize)>,
    confirm_delete: Option<u64>,
    /// Diff line a review note is being written for: path, line number, code.
    note_at: Option<(String, usize, String)>,
    note_input: Entity<InputState>,
    /// Tasks waiting for their worktree setup command to finish.
    pending_start: HashMap<u64, String>,
    /// The quick-open palette, while open, and what each of its rows leads to.
    palette: Option<Entity<CommandState>>,
    palette_jumps: Vec<Vec<Jump>>,
    /// The session title is being edited in the header.
    renaming: bool,
    rename: Entity<InputState>,
    /// Changed file waiting for a second click before its changes are thrown away.
    confirm_discard: Option<String>,
    commit_msg: Entity<InputState>,
    sync: git::Sync,
    notice: Option<String>,
    /// A short confirmation (not an error) shown in the status bar.
    info: Option<(String, std::time::Instant)>,
    mono_font: SharedString,
    /// Whether the right-hand Changes / Files / Terminal panel is shown.
    show_panel: bool,
    /// Root focus, so global shortcuts work before any input has been clicked.
    focus: FocusHandle,
    /// Last window-width class seen, so the panel auto-hides once when the window narrows.
    panel_wide: Option<bool>,
    /// When each running agent started, for the elapsed-time label.
    started: HashMap<u64, std::time::Instant>,
    /// Sessions the user asked to stop, so their exit is not reported as an error.
    stopping: std::collections::HashSet<u64>,
    _subscriptions: Vec<Subscription>,
}

fn demo_changes() -> Vec<git::Change> {
    [
        ("src/sidebar/navigation.rs", 48, 12),
        ("src/components/nav_item.rs", 32, 6),
        ("src/components/icons.rs", 18, 4),
        ("src/styles/navigation.css", 22, 8),
    ]
    .into_iter()
    .map(|(path, added, removed)| git::Change {
        path: path.into(),
        status: "M".into(),
        added,
        removed,
    })
    .collect()
}

/// `with_animation` that is skipped entirely when animations are switched off.
trait Animate: IntoElement + Sized + 'static {
    fn animated(
        self,
        id: impl Into<ElementId>,
        animation: Animation,
        f: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnyElement {
        if anim_on() {
            self.with_animation(id, animation, f).into_any_element()
        } else {
            self.into_any_element()
        }
    }
}
impl<T: IntoElement + 'static> Animate for T {}

fn fade<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>, ms: u64) -> AnyElement {
    el.animated(
        id,
        Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d),
    )
}

/// Fade-in whose start is delayed by `i` steps, so lists cascade in.
fn stagger<E: IntoElement + Styled + 'static>(
    el: E,
    id: impl Into<ElementId>,
    i: usize,
) -> AnyElement {
    let i = i.min(8) as f32;
    el.animated(
        id,
        Animation::new(Duration::from_millis(700)).with_easing(linear),
        move |el, d| {
            let t = ((d * 700.0 - i * 70.0) / 260.0).clamp(0.0, 1.0);
            el.opacity(t)
        },
    )
}

/// "Working" row: spinning loader, label and three dots that pulse in sequence.
fn working_indicator() -> impl IntoElement {
    let mut dots = div().flex().items_center().gap(px(3.));
    for i in 0..3usize {
        dots = dots.child(
            div()
                .id(("wd", i))
                .size(px(5.))
                .rounded_full()
                .bg(rgb(c_muted()))
                .animated(
                    ("wd-anim", i),
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    move |el, d| {
                        let p = (d + i as f32 * 0.18) % 1.0;
                        el.opacity(0.2 + 0.8 * (p * std::f32::consts::PI).sin())
                    },
                ),
        );
    }
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_color(rgb(c_secondary()))
        .child(icon(IconName::Loader, 14., c_blue()).animated(
            "working-spin",
            Animation::new(Duration::from_millis(900)).repeat(),
            |i, d| i.rotate(percentage(d)),
        ))
        .child("Working")
        .child(dots)
}

/// File listing for folders that are not git repositories (shallow, capped).
fn plain_files(dir: &str) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if out.len() >= 2000 {
                return;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target") {
                continue;
            }
            if path.is_dir() {
                if depth < 4 {
                    walk(root, &path, depth + 1, out);
                }
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(
        std::path::Path::new(dir),
        std::path::Path::new(dir),
        0,
        &mut out,
    );
    out.sort();
    out
}

/// Colour for a git status letter: added, deleted, renamed, otherwise modified.
fn change_color(status: &str) -> u32 {
    match status.chars().next() {
        Some('A') => c_green(),
        Some('D') => c_red(),
        Some('R') | Some('C') => c_blue(),
        _ => c_amber(),
    }
}

/// The platform's command modifier, as shown in shortcut hints.
pub(crate) const MOD_KEY: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};

/// `42s`, `3:07`, `1:02:09`.
fn elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}:{:02}", secs / 60, secs % 60),
        _ => format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60),
    }
}

fn icon(name: IconName, size: f32, color: u32) -> Icon {
    Icon::new(name).size(px(size)).text_color(rgb(color))
}

impl RunoApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        caps::refresh();
        let (tx, rx) = channel::<Event>();
        let mut ticks = 0u32;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            ticks = ticks.wrapping_add(1);
            let events: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            if events.is_empty() {
                // Repaint about once a second while agents run, for their elapsed time.
                let tick = ticks.is_multiple_of(12);
                let repaint = caps::take_updated();
                // Every ~2s, re-read git status for the session being watched while its
                // agent works, so the Changes panel fills in as files are edited.
                let rescan = ticks.is_multiple_of(25);
                let alive = this.update(cx, |this, cx| {
                    let watching = this.view == View::Session
                        && this
                            .selected
                            .is_some_and(|id| this.started.contains_key(&id));
                    if rescan && watching && !this.demo {
                        this.refresh();
                        cx.notify();
                    } else if repaint || (tick && (!this.started.is_empty() || this.info.is_some()))
                    {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
                continue;
            }
            let alive = this.update(cx, |this, cx| {
                for event in events {
                    this.handle_event(event, cx);
                }
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();

        let mut state = State::load();
        let demo = state.projects.is_empty();
        if demo {
            state = State::demo();
        }
        let selected = state.sessions.first().map(|s| s.id);

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter sessions"));
        let search_sub = cx.subscribe_in(&search, window, |_, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let settings_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search settings…"));
        let settings_search_sub =
            cx.subscribe_in(&settings_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let term_input = cx.new(|cx| InputState::new(window, cx).placeholder("Run a command…"));
        let term_sub = cx.subscribe_in(&term_input, window, |this, input, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let line = input.read(cx).value().to_string();
                input.update(cx, |input, cx| input.set_value("", window, cx));
                this.run_terminal(line, cx);
            }
        });
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Session name"));
        let rename_sub = cx.subscribe_in(&rename, window, |this, input, event, _, cx| {
            match event {
                InputEvent::PressEnter { .. } => {
                    let title = input.read(cx).value().trim().to_string();
                    this.finish_rename(Some(title));
                }
                InputEvent::Blur => this.finish_rename(None),
                _ => {}
            }
            cx.notify();
        });
        let note_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Leave a note for the agent, then Enter")
        });
        let note_sub = cx.subscribe_in(&note_input, window, |this, input, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = input.read(cx).value().trim().to_string();
                input.update(cx, |input, cx| input.set_value("", window, cx));
                this.add_note(text);
                cx.notify();
            }
        });
        let commit_msg = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        let commit_sub = cx.subscribe_in(&commit_msg, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit(window, cx);
                cx.notify();
            }
        });
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What would you like to change?")
                .submit_on_enter(true)
        });
        let prompt_sub = cx.subscribe_in(&prompt, window, |this, _, event, window, cx| {
            let on_enter = cx.global::<AppSettings>().send_on_enter;
            let send = match event {
                InputEvent::PressEnter { shift, secondary } => {
                    if on_enter {
                        !*shift
                    } else {
                        *secondary
                    }
                }
                // Repaint so the send button reflects whether there is anything to send.
                InputEvent::Change => {
                    cx.notify();
                    false
                }
                _ => false,
            };
            if send {
                this.send_message(window, cx);
                cx.notify();
            }
        });

        let mut terminal = HashMap::new();
        if demo {
            terminal.insert(
                11,
                vec![
                    "$ cargo check -p runo-native".to_string(),
                    "    Finished dev profile in 0.97s".to_string(),
                    "$ git status --short".to_string(),
                    " M src/sidebar/navigation.rs".to_string(),
                ],
            );
        }
        let mut app = Self {
            expanded: state.projects.iter().map(|p| p.id).collect(),
            tabs: Vec::new(),
            open_steps: Default::default(),
            show_sidebar: true,
            stats: HashMap::new(),
            new_project: state.projects.first().map(|p| p.id),
            view: View::Session,
            state,
            demo,
            selected,
            right_tab: RightTab::Changes,
            cli_tab: false,
            menu: None,
            settings_page: SettingsPage::General,
            settings_search,
            settings_inputs: HashMap::new(),
            settings_menu: None,
            discovered: None,
            applied_dark: None,
            applied_enter: None,
            changes: Vec::new(),
            dir_is_git: true,
            selected_file: None,
            diff: String::new(),
            files: Vec::new(),
            preview: None,
            terminal,
            handles: HashMap::new(),
            tx,
            chat_scroll: ScrollHandle::new(),
            stderr: HashMap::new(),
            streams: HashMap::new(),
            search,
            prompt,
            new_task: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Describe what you want the agent to work on…")
            }),
            branch: cx.new(|cx| InputState::new(window, cx).placeholder("branch (optional)")),
            term_input,
            address: cx.new(|cx| InputState::new(window, cx).placeholder("https://localhost:3000")),
            new_provider: agent::PROVIDERS
                .iter()
                .position(|p| p.id == cx.global::<AppSettings>().default_provider)
                .unwrap_or(0),
            new_isolated: true,
            race_mode: false,
            history: Vec::new(),
            race_stats: HashMap::new(),
            new_race: Vec::new(),
            confirm_delete: None,
            note_at: None,
            note_input: note_input.clone(),
            pending_start: HashMap::new(),
            palette: None,
            palette_jumps: Vec::new(),
            renaming: false,
            rename: rename.clone(),
            confirm_discard: None,
            commit_msg: commit_msg.clone(),
            sync: git::Sync::default(),
            notice: None,
            info: None,
            mono_font: cx.theme().mono_font_family.clone(),
            show_panel: true,
            focus: cx.focus_handle(),
            panel_wide: None,
            started: HashMap::new(),
            stopping: Default::default(),
            _subscriptions: vec![
                search_sub,
                term_sub,
                prompt_sub,
                settings_search_sub,
                commit_sub,
                rename_sub,
                note_sub,
            ],
        };
        // The New session form previews the branch and enables Start as you type.
        let repaint = |_: &mut Self,
                       _: &Entity<InputState>,
                       event: &InputEvent,
                       _: &mut Window,
                       cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        };
        let branch_sub = cx.subscribe_in(&app.branch, window, repaint);
        let task_sub =
            cx.subscribe_in(
                &app.new_task,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => {
                        this.create_session(window, cx);
                        cx.notify();
                    }
                    _ => {}
                },
            );
        app._subscriptions.extend([branch_sub, task_sub]);
        let ids: Vec<u64> = app.state.sessions.iter().map(|s| s.id).collect();
        for id in ids {
            app.measure(id);
        }
        app.tabs.extend(app.selected);
        app.refresh();
        window.focus(&app.focus, cx);
        app
    }

    // ---------- state & events ----------

    fn save(&self) {
        if !self.demo {
            self.state.save();
        }
    }

    fn current(&self) -> Option<&Session> {
        self.selected.and_then(|id| self.state.session(id))
    }

    fn refresh(&mut self) {
        let Some(dir) = self.current().map(|s| s.dir.clone()) else {
            self.changes.clear();
            self.files.clear();
            return;
        };
        self.dir_is_git = self.demo || git::is_repo(&dir);
        if self.demo {
            self.changes = demo_changes();
            self.files = self.changes.iter().map(|c| c.path.clone()).collect();
            self.selected_file = Some(self.changes[0].path.clone());
            self.diff = DEMO_DIFF.to_string();
            self.sync = git::Sync {
                remote: "origin".into(),
                upstream: true,
                ahead: 2,
            };
            return;
        }
        if !self.dir_is_git {
            self.changes.clear();
            self.selected_file = None;
            self.diff.clear();
            self.files = plain_files(&dir);
            return;
        }
        self.changes = git::changes(&dir);
        if let Some(id) = self.selected {
            let (a, r) = self
                .changes
                .iter()
                .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
            self.stats.insert(id, (self.changes.len(), a, r));
        }
        self.files = git::tracked_files(&dir);
        self.sync = git::sync_state(&dir);
        self.race_stats.clear();
        if let Some(race) = self.current().map(|s| s.race).filter(|r| *r != 0) {
            let others: Vec<(u64, String)> = self
                .state
                .sessions
                .iter()
                .filter(|s| s.race == race)
                .map(|s| (s.id, s.dir.clone()))
                .collect();
            for (id, dir) in others {
                let changes = git::changes(&dir);
                let (a, r) = changes
                    .iter()
                    .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
                self.race_stats.insert(id, (changes.len(), a, r));
            }
        }
        if let Some(file) = self.selected_file.clone() {
            if self.changes.iter().any(|c| c.path == file) {
                self.diff = git::file_diff(&dir, &file);
            } else {
                self.selected_file = None;
                self.diff.clear();
            }
        }
        if self.selected_file.is_none() {
            if let Some(first) = self.changes.first() {
                self.selected_file = Some(first.path.clone());
                self.diff = git::file_diff(&dir, &first.path);
            }
        }
    }

    /// Files changed and lines added/removed in a session's folder, for its sidebar card.
    fn measure(&mut self, id: u64) {
        let Some(dir) = self.state.session(id).map(|s| s.dir.clone()) else {
            return;
        };
        let changes = if self.demo {
            demo_changes()
        } else if git::is_repo(&dir) {
            git::changes(&dir)
        } else {
            return;
        };
        let (a, r) = changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        self.stats.insert(id, (changes.len(), a, r));
    }

    fn open_change(&mut self, path: &str) {
        if !self.demo {
            if let Some(dir) = self.current().map(|s| s.dir.clone()) {
                self.diff = git::file_diff(&dir, path);
            }
        }
        self.selected_file = Some(path.to_string());
    }

    fn close_tab(&mut self, id: u64) {
        let Some(pos) = self.tabs.iter().position(|t| *t == id) else {
            return;
        };
        self.tabs.remove(pos);
        if self.selected == Some(id) {
            match self.tabs.get(pos).or(self.tabs.last()).copied() {
                Some(next) => self.select(next),
                None => {
                    self.selected = None;
                    self.view = View::Inbox;
                }
            }
        }
    }

    fn select(&mut self, id: u64) {
        self.selected = Some(id);
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
        }
        self.renaming = false;
        self.note_at = None;
        if let Some(session) = self.state.session_mut(id) {
            if session.unread {
                session.unread = false;
                self.save();
            }
        }
        self.view = View::Session;
        self.selected_file = None;
        self.diff.clear();
        self.preview = None;
        self.confirm_delete = None;
        self.confirm_discard = None;
        self.notice = None;
        self.info = None;
        self.menu = None;
        self.refresh();
        self.chat_scroll.scroll_to_bottom();
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Line(Target::Session(id), line) => {
                let updates = match self.streams.get_mut(&id) {
                    Some(parser) => parser.feed(&line),
                    None => vec![stream::Update::Text(line)],
                };
                let plain = !self.streams.contains_key(&id);
                if let Some(session) = self.state.session_mut(id) {
                    for update in updates {
                        if !matches!(session.messages.last(), Some(m) if m.role == Role::Agent) {
                            session.messages.push(Message::new(Role::Agent, ""));
                        }
                        let Some(m) = session.messages.last_mut() else {
                            continue;
                        };
                        match update {
                            stream::Update::Text(text) => {
                                if plain && !m.text.is_empty() {
                                    m.text.push('\n');
                                }
                                m.text.push_str(&text);
                            }
                            stream::Update::Step(step) => m.steps.push(step),
                            stream::Update::Summary(summary) => m.meta = summary,
                            stream::Update::Error(error) => {
                                session.messages.push(Message::new(Role::System, error))
                            }
                        }
                    }
                }
                if self.selected == Some(id) && autoscroll() {
                    self.chat_scroll.scroll_to_bottom();
                }
            }
            Event::Err(Target::Session(id), line) => {
                let log = self.stderr.entry(id).or_default();
                log.push(line);
                if log.len() > 200 {
                    log.drain(..100);
                }
            }
            Event::Err(..) => {}
            Event::Line(Target::Terminal(id), line) => {
                let log = self.terminal.entry(id).or_default();
                log.push(line);
                if log.len() > 2000 {
                    log.drain(..500);
                }
            }
            Event::Done(Target::Session(id), ok) => {
                self.handles.remove(&Target::Session(id));
                self.streams.remove(&id);
                self.started.remove(&id);
                self.measure(id);
                let on_screen = self.selected == Some(id) && self.view == View::Session;
                if let Some(session) = self.state.session_mut(id) {
                    session.status = if ok { Status::Done } else { Status::Failed };
                    session.unread = !on_screen;
                    if session.messages.last().is_some_and(|m| {
                        m.role == Role::Agent && m.text.is_empty() && m.steps.is_empty()
                    }) {
                        if ok {
                            if let Some(m) = session.messages.last_mut() {
                                m.text = "(The agent finished without any output.)".into();
                            }
                        } else {
                            session.messages.pop();
                        }
                    }
                    notify(
                        &session.title,
                        if ok {
                            "Agent finished"
                        } else {
                            "Agent stopped or failed"
                        },
                    );
                    if !ok {
                        let all = self.stderr.get(&id).cloned().unwrap_or_default();
                        let mut errors: Vec<String> = Vec::new();
                        for line in all.iter().filter(|l| {
                            l.starts_with("ERROR:")
                                || l.starts_with("Error:")
                                || l.starts_with("error:")
                        }) {
                            if !errors.contains(line) {
                                errors.push(line.clone());
                            }
                        }
                        let tail: Vec<String> = if errors.is_empty() {
                            all.iter().rev().take(12).rev().cloned().collect()
                        } else {
                            errors
                        };
                        let log = tail.join("\n");
                        let stopped = self.stopping.remove(&id);
                        let mut text = String::from(if stopped {
                            "Stopped."
                        } else {
                            "The agent stopped with an error."
                        });
                        if !log.trim().is_empty() && !stopped {
                            text.push_str(&format!(
                                "\n```text\n{}\n```",
                                log.replace("```", "'''")
                            ));
                        }
                        if log.contains("model")
                            && (log.contains("not supported")
                                || log.contains("not found")
                                || log.contains("does not exist"))
                        {
                            text.push_str("\nTip: choose another model from the model menu, or set one under Settings for this agent.");
                        }
                        session.messages.push(Message::new(Role::System, text));
                    }
                }
                self.save();
                if self.selected == Some(id) {
                    self.refresh();
                }
            }
            Event::Done(Target::Terminal(id), ok) => {
                self.handles.remove(&Target::Terminal(id));
                if !ok {
                    self.terminal
                        .entry(id)
                        .or_default()
                        .push("[exited with an error]".into());
                }
                if let Some(task) = self.pending_start.remove(&id) {
                    if !ok {
                        self.terminal.entry(id).or_default().push(
                            "Setup failed; starting the agent anyway so you can fix it together."
                                .into(),
                        );
                    }
                    if let Some(provider) = self.state.session(id).map(|s| s.provider.clone()) {
                        let opts = cx.global::<AppSettings>().agent(&provider);
                        self.start_agent(id, task, &opts);
                    }
                }
                if self.selected == Some(id) {
                    self.refresh();
                }
            }
        }
    }

    fn start_agent(&mut self, id: u64, prompt: String, opts: &AgentOptions) {
        self.stderr.remove(&id);
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        let resume = session.has_conversation();
        if session.provider == "claude" && !resume {
            session.agent_session = agent::new_uuid();
        }
        session
            .messages
            .push(Message::now(Role::User, prompt.clone()));
        session.messages.push(Message::now(Role::Agent, ""));
        let (provider, dir) = (session.provider.clone(), session.dir.clone());
        let sel = session.selections();
        let agent_session = session.agent_session.clone();
        let spec = agent::RunSpec {
            provider: &provider,
            prompt: &prompt,
            resume,
            sel: &sel,
            opts,
            in_git: git::is_repo(&dir),
            session_id: &agent_session,
        };
        match agent::run_agent(Target::Session(id), &spec, &dir, self.tx.clone()) {
            Ok(handle) => {
                session.status = Status::Working;
                if provider == "claude" {
                    self.streams.insert(id, stream::ClaudeStream::new(&dir));
                }
                self.handles.insert(Target::Session(id), handle);
                self.started.insert(id, std::time::Instant::now());
            }
            Err(error) => {
                session.status = Status::Failed;
                session.messages.pop();
                session.messages.push(Message::new(Role::System, error));
            }
        }
        self.save();
        self.chat_scroll.scroll_to_bottom();
    }

    /// Demo projects are not real folders, so ask for a real repository instead.
    fn demo_guard(&mut self, cx: &mut Context<Self>) -> bool {
        if self.demo {
            self.pick_project(cx);
        }
        self.demo
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some(id) = self.selected else { return };
        if self.handles.contains_key(&Target::Session(id)) {
            self.notice = Some("The agent is still working. Stop it or wait.".into());
            return;
        }
        let text = self.prompt.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.prompt.update(cx, |p, cx| p.set_value("", window, cx));
        let opts = self
            .current()
            .map(|s| cx.global::<AppSettings>().agent(&s.provider))
            .unwrap_or_default();
        self.start_agent(id, text, &opts);
    }

    /// Load a file of the current session into the Files preview.
    fn open_preview(&mut self, path: &str) {
        if self.demo {
            self.preview = Some((path.to_string(), "(demo data)".into()));
        } else if let Some(dir) = self.current().map(|s| s.dir.clone()) {
            let text = std::fs::read_to_string(std::path::Path::new(&dir).join(path))
                .unwrap_or_else(|_| "(binary, deleted or unreadable file)".into());
            self.preview = Some((
                path.to_string(),
                text.lines().take(1500).collect::<Vec<_>>().join("\n"),
            ));
        }
    }

    fn open_settings(&mut self) {
        self.view = View::Settings;
        let dir = self.current().filter(|_| !self.demo).map(|s| s.dir.clone());
        self.discovered = Some(settings::discover(dir.as_deref()));
    }

    fn stop(&mut self) {
        if let Some(id) = self.selected {
            if let Some(handle) = self.handles.get(&Target::Session(id)) {
                self.stopping.insert(id);
                handle.kill();
            }
        }
    }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.current().map(|s| s.title.clone()) else {
            return;
        };
        self.renaming = true;
        self.rename.update(cx, |input, cx| {
            input.set_value(title, window, cx);
            input.focus(window, cx);
        });
    }

    /// Save the new title, or with `None` just leave rename mode.
    fn finish_rename(&mut self, title: Option<String>) {
        if !self.renaming {
            return;
        }
        self.renaming = false;
        let Some(title) = title.filter(|t| !t.is_empty()) else {
            return;
        };
        if let Some(id) = self.selected {
            if let Some(session) = self.state.session_mut(id) {
                session.title = title;
            }
            self.save();
        }
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        self.menu = None;
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |s, cx| s.focus(window, cx));
        self.palette = Some(state);
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus, cx);
    }

    fn palette_jump(&mut self, jump: Jump, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        match jump {
            Jump::Action => {}
            Jump::View(view) => self.view = view,
            Jump::Session(id) => self.select(id),
            Jump::File(path) => {
                self.view = View::Session;
                self.show_panel = true;
                self.open_preview(&path);
                self.right_tab = RightTab::Files;
            }
        }
    }

    /// The palette's rows: commands, then sessions, then files of the open session.
    fn palette_view(&mut self, state: Entity<CommandState>, cx: &mut Context<Self>) -> AnyElement {
        let mut jumps: Vec<Vec<Jump>> = Vec::new();
        let mut commands = CommandGroup::new().label("Commands");
        let mut command_jumps = Vec::new();
        let actions: [(&str, IconName, Box<dyn Action>); 5] = [
            ("New session", IconName::Plus, Box::new(NewSession)),
            ("Open settings", IconName::Settings, Box::new(OpenSettings)),
            (
                "Toggle the Changes panel",
                IconName::PanelRightOpen,
                Box::new(TogglePanel),
            ),
            (
                "Toggle the sidebar",
                IconName::PanelLeft,
                Box::new(ToggleSidebar),
            ),
            (
                "Stop the running agent",
                IconName::CircleStop,
                Box::new(StopAgent),
            ),
        ];
        for (label, name, action) in actions {
            commands = commands.item(CommandItem::new().label(label).icon(name).action(action));
            command_jumps.push(Jump::Action);
        }
        for (label, name, view) in [
            ("Go to Inbox", IconName::BellDot, View::Inbox),
            ("Go to All sessions", IconName::Layers, View::AllSessions),
            ("Go to Worktrees", IconName::GitBranch, View::Worktrees),
        ] {
            commands = commands.item(CommandItem::new().label(label).icon(name));
            command_jumps.push(Jump::View(view));
        }
        jumps.push(command_jumps);

        let mut sessions = CommandGroup::new().label("Sessions");
        let mut session_jumps = Vec::new();
        for session in self.state.sessions.iter().filter(|s| !s.archived) {
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == session.project_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            sessions = sessions.item(
                CommandItem::new()
                    .label(format!("{}  ·  {project}", session.title))
                    .icon(IconName::MessageSquare)
                    .keywords([session.branch.clone(), session.provider.clone()]),
            );
            session_jumps.push(Jump::Session(session.id));
        }
        jumps.push(session_jumps);

        let mut files = CommandGroup::new().label("Files in this session");
        let mut file_jumps = Vec::new();
        if self.view == View::Session && self.current().is_some() {
            for path in self.files.iter().take(3000) {
                files = files.item(CommandItem::new().label(path.clone()).icon(IconName::File));
                file_jumps.push(Jump::File(path.clone()));
            }
        }
        jumps.push(file_jumps);
        self.palette_jumps = jumps;

        let this = cx.entity().downgrade();
        let cancel = this.clone();
        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .pt(px(110.))
            .bg(gpui_kit::black().opacity(0.28))
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_palette(window, cx);
                cx.notify();
            }))
            .child(
                div()
                    .id("palette")
                    .w(px(640.))
                    .h_auto()
                    .max_h(px(520.))
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_canvas()))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        Command::new(&state)
                            .group(commands)
                            .group(sessions)
                            .group(files)
                            .bordered(false)
                            .max_h(px(440.))
                            .placeholder("Jump to a session, file or command…")
                            .on_confirm(move |ix, window, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    let jump = this
                                        .palette_jumps
                                        .get(ix.section)
                                        .and_then(|g| g.get(ix.row))
                                        .cloned();
                                    if let Some(jump) = jump {
                                        this.palette_jump(jump, window, cx);
                                    }
                                    cx.notify();
                                });
                            })
                            .on_cancel(move |window, cx| {
                                let _ = cancel.update(cx, |this, cx| {
                                    this.close_palette(window, cx);
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .animated(
                "palette-in",
                Animation::new(Duration::from_millis(160)).with_easing(ease_out_quint()),
                |el, delta| el.opacity(delta),
            )
            .into_any_element()
    }

    fn add_note(&mut self, text: String) {
        let Some((path, line, code)) = self.note_at.take() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some(id) = self.selected {
            if let Some(session) = self.state.session_mut(id) {
                session.notes.push(Note {
                    path,
                    line,
                    code,
                    text,
                });
            }
            self.save();
        }
    }

    fn remove_note(&mut self, index: usize) {
        if let Some(id) = self.selected {
            if let Some(session) = self.state.session_mut(id) {
                if index < session.notes.len() {
                    session.notes.remove(index);
                }
            }
            self.save();
        }
    }

    /// Send every review note to the agent as one follow-up.
    fn send_notes(&mut self, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some(id) = self.selected else { return };
        if self.handles.contains_key(&Target::Session(id)) {
            self.notice =
                Some("The agent is still working. Send the notes when it finishes.".into());
            return;
        }
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        if session.notes.is_empty() {
            return;
        }
        let prompt = store::notes_prompt(&session.notes);
        session.notes.clear();
        let provider = session.provider.clone();
        let opts = cx.global::<AppSettings>().agent(&provider);
        self.start_agent(id, prompt, &opts);
    }

    fn toggle_pin(&mut self, id: u64) {
        if let Some(session) = self.state.session_mut(id) {
            session.pinned = !session.pinned;
            self.save();
        }
    }

    fn toggle_archive(&mut self, id: u64) {
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        session.archived = !session.archived;
        let (archived, title) = (session.archived, session.title.clone());
        self.save();
        self.info = Some((
            if archived {
                format!("Archived “{title}”. Find it under All sessions.")
            } else {
                format!("Restored “{title}”.")
            },
            std::time::Instant::now(),
        ));
    }

    /// Commit every change in the session folder. An empty message falls back to the
    /// session title.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some((dir, title)) = self.current().map(|s| (s.dir.clone(), s.title.clone())) else {
            return;
        };
        let typed = self.commit_msg.read(cx).value().trim().to_string();
        let message = if typed.is_empty() { title } else { typed };
        match git::commit_all(&dir, &message) {
            Ok(hash) => {
                self.commit_msg
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.notice = None;
                self.info = Some((
                    format!("Committed {hash}: {message}"),
                    std::time::Instant::now(),
                ));
                self.refresh();
            }
            Err(error) => self.notice = Some(format!("Commit failed: {error}")),
        }
    }

    fn discard(&mut self, path: &str) {
        self.confirm_discard = None;
        if self.demo {
            return;
        }
        let Some(dir) = self.current().map(|s| s.dir.clone()) else {
            return;
        };
        match git::discard(&dir, path) {
            Ok(()) => {
                self.info = Some((format!("Discarded {path}."), std::time::Instant::now()));
                self.refresh();
            }
            Err(error) => self.notice = Some(format!("Could not discard {path}: {error}")),
        }
    }

    /// Push the branch, then optionally open a pull request with `gh`. Output goes to
    /// the Terminal so progress and links stay visible.
    fn ship(&mut self, open_pr: bool, cx: &mut Context<Self>) {
        let remote = self.sync.remote.clone();
        if remote.is_empty() {
            self.notice = Some("This repository has no remote to push to.".into());
            return;
        }
        let mut line = format!("git push -u {remote} HEAD");
        if open_pr {
            line.push_str(" && gh pr create --fill");
        }
        self.run_terminal(line, cx);
    }

    fn run_terminal(&mut self, line: String, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let line = line.trim().to_string();
        let Some(session) = self.current() else {
            return;
        };
        if line.is_empty() {
            return;
        }
        let (id, dir) = (session.id, session.dir.clone());
        if self.handles.contains_key(&Target::Terminal(id)) {
            self.notice = Some("A command is already running in this terminal.".into());
            return;
        }
        self.terminal
            .entry(id)
            .or_default()
            .push(format!("$ {line}"));
        match agent::run_shell(Target::Terminal(id), &line, &dir, self.tx.clone()) {
            Ok(handle) => {
                self.handles.insert(Target::Terminal(id), handle);
            }
            Err(error) => self.terminal.entry(id).or_default().push(error),
        }
        cx.notify();
    }

    fn add_project(&mut self, path: &std::path::Path) {
        let path_str = path.to_string_lossy().into_owned();
        if !path.is_dir() {
            self.notice = Some(format!("{path_str} is not a folder."));
            return;
        }
        if self.demo {
            // Leave the sample workspace behind and start from the real state on disk.
            self.demo = false;
            self.state = State::load();
            self.expanded.clear();
            self.selected = None;
            self.terminal.clear();
        }
        if self.state.projects.iter().any(|p| p.path == path_str) {
            self.notice = Some("That project is already added.".into());
            return;
        }
        let id = self.state.next_id();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_str.clone());
        self.state.projects.push(Project {
            id,
            name,
            path: path_str,
        });
        self.expanded.push(id);
        self.new_project = Some(id);
        self.view = View::NewSession;
        self.save();
    }

    fn pick_project(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.first() {
                    let path = path.clone();
                    let _ = this.update(cx, |this, cx| {
                        this.add_project(&path);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn create_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let task = self.new_task.read(cx).value().trim().to_string();
        let Some(project) = self
            .new_project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            self.notice = Some("Add a project first.".into());
            return;
        };
        if task.is_empty() {
            self.notice = Some("Describe the task first.".into());
            return;
        }
        let typed = git::sanitize_branch(&self.branch.read(cx).value());
        let is_git = git::is_repo(&project.path);
        let racing = self.racing() && is_git;
        let providers: Vec<usize> = if racing {
            self.new_race.clone()
        } else {
            vec![self.new_provider]
        };
        let mut created = Vec::new();
        let race_base = if typed.is_empty() {
            format!("runo/race-{}", self.state.next_id + 1)
        } else {
            typed.clone()
        };
        for provider in providers {
            let branch = if racing {
                format!("{race_base}-{}", agent::PROVIDERS[provider].id)
            } else {
                typed.clone()
            };
            let isolated = (self.new_isolated || racing) && is_git;
            match self.spawn_session(&project, provider, &branch, isolated, is_git, &task, cx) {
                Ok(id) => created.push(id),
                Err(error) => {
                    self.notice = Some(error);
                    break;
                }
            }
        }
        let Some(&first) = created.first() else {
            return;
        };
        if created.len() > 1 {
            for &id in &created {
                if let Some(session) = self.state.session_mut(id) {
                    session.race = first;
                }
            }
            self.save();
        }
        self.new_task
            .update(cx, |t, cx| t.set_value("", window, cx));
        self.branch.update(cx, |b, cx| b.set_value("", window, cx));
        if !self.expanded.contains(&project.id) {
            self.expanded.push(project.id);
        }
        // Starting each one selects it; land on the first.
        for &id in &created {
            self.select(id);
            self.launch(id, task.clone(), cx);
        }
        self.select(first);
    }

    /// Whether the New session form will race several agents.
    fn racing(&self) -> bool {
        self.race_mode && self.new_race.len() > 1
    }

    /// Create one session (and its worktree) without starting the agent.
    #[allow(clippy::too_many_arguments)]
    fn spawn_session(
        &mut self,
        project: &Project,
        provider: usize,
        typed_branch: &str,
        isolated: bool,
        is_git: bool,
        task: &str,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        let id = self.state.next_id();
        let mut setup_log = Vec::new();
        let (branch, dir) = if !is_git {
            ("local folder".to_string(), project.path.clone())
        } else if isolated {
            let branch = if typed_branch.is_empty() {
                format!("runo/session-{id}")
            } else {
                typed_branch.to_string()
            };
            let dir = git::add_worktree(&project.path, &branch)
                .map_err(|error| format!("Could not create worktree: {error}"))?;
            match git::copy_included(&project.path, &dir) {
                Ok(copied) if !copied.is_empty() => setup_log.push(format!(
                    "Copied {} file{} from .worktreeinclude: {}",
                    copied.len(),
                    if copied.len() == 1 { "" } else { "s" },
                    copied.join(", ")
                )),
                Ok(_) => {}
                Err(error) => {
                    setup_log.push(format!("Could not copy .worktreeinclude files: {error}"))
                }
            }
            (branch, dir)
        } else {
            (git::current_branch(&project.path), project.path.clone())
        };
        let title: String = task
            .lines()
            .next()
            .unwrap_or("Session")
            .chars()
            .take(48)
            .collect();
        self.state.sessions.push(Session {
            id,
            project_id: project.id,
            title,
            provider: agent::PROVIDERS[provider].id.into(),
            branch,
            dir,
            isolated,
            mode: String::new(),
            model: String::new(),
            opts: std::collections::HashMap::from([(
                "mode".to_string(),
                cx.global::<AppSettings>().default_mode.clone(),
            )]),
            status: Status::Idle,
            messages: Vec::new(),
            agent_session: String::new(),
            conversation_start: 0,
            pinned: false,
            archived: false,
            unread: false,
            notes: Vec::new(),
            race: 0,
        });
        if !setup_log.is_empty() {
            self.terminal.entry(id).or_default().extend(setup_log);
        }
        Ok(id)
    }

    /// Start a new session's agent, after the worktree setup command when there is one.
    /// The session must be selected, since setup runs in its Terminal.
    fn launch(&mut self, id: u64, task: String, cx: &mut Context<Self>) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let (isolated, provider) = (session.isolated, session.provider.clone());
        let setup = cx.global::<AppSettings>().worktree_setup.trim().to_string();
        if isolated && !setup.is_empty() {
            self.pending_start.insert(id, task);
            self.run_terminal(setup, cx);
            if self.handles.contains_key(&Target::Terminal(id)) {
                self.info = Some((
                    "Running the worktree setup command first.".into(),
                    std::time::Instant::now(),
                ));
                return;
            }
            // The command could not start; go straight to the agent.
            let Some(task) = self.pending_start.remove(&id) else {
                return;
            };
            let opts = cx.global::<AppSettings>().agent(&provider);
            self.start_agent(id, task, &opts);
            return;
        }
        let opts = cx.global::<AppSettings>().agent(&provider);
        self.start_agent(id, task, &opts);
    }

    fn open_history(&mut self) {
        self.view = View::History;
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            self.history = history::scan(std::path::Path::new(&home));
        }
    }

    /// The Runo project a folder belongs to: the project itself or one of its worktrees.
    fn project_for(&self, dir: &str) -> Option<&Project> {
        self.state.projects.iter().find(|p| {
            dir == p.path
                || std::path::Path::new(dir).starts_with(
                    git::worktree_path(&p.path, "x")
                        .parent()
                        .unwrap_or(std::path::Path::new("/")),
                )
        })
    }

    /// Open a past CLI conversation as a Runo session; the next message resumes it.
    fn resume_past(&mut self, past: history::Past, cx: &mut Context<Self>) {
        if !std::path::Path::new(&past.cwd).is_dir() {
            self.notice = Some(format!("{} no longer exists.", past.cwd));
            return;
        }
        if let Some(existing) = self
            .state
            .sessions
            .iter()
            .find(|s| s.agent_session == past.id && s.provider == past.provider)
        {
            let id = existing.id;
            self.select(id);
            return;
        }
        let project_id = match self.project_for(&past.cwd).map(|p| p.id) {
            Some(id) => id,
            None => {
                let id = self.state.next_id();
                let name = std::path::Path::new(&past.cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| past.cwd.clone());
                self.state.projects.push(Project {
                    id,
                    name,
                    path: past.cwd.clone(),
                });
                id
            }
        };
        let id = self.state.next_id();
        let provider = agent::provider(past.provider);
        let branch = if git::is_repo(&past.cwd) {
            git::current_branch(&past.cwd)
        } else {
            "local folder".into()
        };
        self.state.sessions.push(Session {
            id,
            project_id,
            title: past.title.clone(),
            provider: past.provider.into(),
            branch,
            dir: past.cwd.clone(),
            isolated: false,
            mode: String::new(),
            model: String::new(),
            opts: std::collections::HashMap::from([(
                "mode".to_string(),
                cx.global::<AppSettings>().default_mode.clone(),
            )]),
            status: Status::Idle,
            messages: vec![Message::now(
                Role::Agent,
                format!(
                    "Picked up the {} conversation “{}” from {}. Earlier messages stay in {}; send a message to continue it.",
                    provider.name,
                    past.title,
                    history::ago(past.modified),
                    provider.name
                ),
            )],
            agent_session: past.id.clone(),
            conversation_start: 0,
            pinned: false,
            archived: false,
            unread: false,
            notes: Vec::new(),
            race: 0,
        });
        if !self.expanded.contains(&project_id) {
            self.expanded.push(project_id);
        }
        self.save();
        self.select(id);
    }

    fn history_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().v_flex().gap_2();
        for (i, past) in self.history.iter().enumerate().take(200) {
            let provider = agent::provider(past.provider);
            let place = match self.project_for(&past.cwd) {
                Some(p) if p.path == past.cwd => p.name.clone(),
                Some(p) => format!("{} · worktree", p.name),
                None => past.cwd.clone(),
            };
            let open = self
                .state
                .sessions
                .iter()
                .any(|s| s.agent_session == past.id && s.provider == past.provider);
            let item = past.clone();
            rows = rows.child(stagger(
                div()
                    .id(("past", i))
                    .p_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.resume_past(item.clone(), cx);
                        cx.notify();
                    }))
                    .child(self.provider_badge(provider, 26.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(past.title.clone()),
                            )
                            .child(div().truncate().text_xs().text_color(rgb(c_muted())).child(
                                format!(
                                    "{}  ·  {place}  ·  {}",
                                    provider.name,
                                    history::ago(past.modified)
                                ),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(rgb(c_blue()))
                            .child(if open { "Open" } else { "Resume" })
                            .child(icon(IconName::ArrowRight, 12., c_blue())),
                    ),
                SharedString::from(format!("past-{i}")),
                i.min(12),
            ));
        }
        if self.history.is_empty() {
            rows = rows.child(
                div()
                    .py_10()
                    .v_flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::RotateCcwClock, 28., c_muted()))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("No past conversations found"),
                    )
                    .child(div().text_xs().text_color(rgb(c_muted())).child(
                        "Claude Code and Codex conversations from this machine show up here.",
                    )),
            );
        }
        self.page(
            "History",
            div()
                .v_flex()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(c_secondary()))
                        .child("Conversations Claude Code and Codex ran on this machine, inside or outside Runo. Pick one to continue it as a session."),
                )
                .child(rows),
        )
    }

    /// Keep one session of a race: archive the others.
    fn keep_racer(&mut self, keep: u64) {
        let Some(race) = self.state.session(keep).map(|s| s.race).filter(|r| *r != 0) else {
            return;
        };
        let mut archived = 0;
        for session in self.state.sessions.iter_mut() {
            if session.race == race && session.id != keep && !session.archived {
                session.archived = true;
                archived += 1;
            }
        }
        self.save();
        self.info = Some((
            format!(
                "Kept this one and archived {archived} other{}.",
                if archived == 1 { "" } else { "s" }
            ),
            std::time::Instant::now(),
        ));
    }

    fn delete_session(&mut self, id: u64) {
        if let Some(handle) = self.handles.remove(&Target::Session(id)) {
            handle.kill();
        }
        if let Some(handle) = self.handles.remove(&Target::Terminal(id)) {
            handle.kill();
        }
        let Some(pos) = self.state.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        let session = self.state.sessions.remove(pos);
        self.tabs.retain(|t| *t != id);
        self.stats.remove(&id);
        if session.isolated && !self.demo && remove_worktree() {
            if let Some(project) = self
                .state
                .projects
                .iter()
                .find(|p| p.id == session.project_id)
            {
                if let Err(error) = git::remove_worktree(&project.path, &session.dir) {
                    self.notice =
                        Some(format!("Session removed; worktree cleanup failed: {error}"));
                }
            }
        }
        self.terminal.remove(&id);
        self.confirm_delete = None;
        self.selected = self.state.sessions.first().map(|s| s.id);
        self.save();
        self.refresh();
    }

    // ---------- small UI helpers ----------

    fn status_color(status: Status) -> u32 {
        match status {
            Status::Idle => c_muted(),
            Status::Working => c_blue(),
            Status::Done => c_green(),
            Status::Failed => c_red(),
        }
    }

    fn status_label(status: Status) -> &'static str {
        match status {
            Status::Idle => "Idle",
            Status::Working => "Working",
            Status::Done => "Completed",
            Status::Failed => "Failed",
        }
    }

    fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        primary: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(30.))
            .px(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .when(primary, |el| {
                el.bg(rgb(c_text()))
                    .text_color(rgb(c_canvas()))
                    .hover(|el| el.opacity(0.9))
            })
            .when(!primary, |el| {
                el.border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_raised()))
                    .text_color(rgb(c_text()))
                    .hover(|el| el.bg(rgb(c_subtle())))
            })
            .child(label.into())
    }

    /// Borderless square icon button with a tooltip.
    fn ghost_button(&self, id: &'static str, name: IconName, tip: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .size(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|el| el.bg(rgb(c_subtle())))
            .child(icon(name, 15., c_secondary()))
    }

    fn ghost_button_sized(
        &self,
        id: impl Into<ElementId>,
        name: IconName,
        tip: &'static str,
        size: f32,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .size(px(size))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|el| el.bg(rgb(c_subtle())))
            .child(icon(name, (size * 0.5).round(), c_secondary()))
    }

    fn chip(&self, id: (&'static str, usize), label: String, selected: bool) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.5))
            .border_1()
            .when(selected, |el| {
                el.border_color(rgb(c_blue()))
                    .bg(rgb(c_selected()))
                    .text_color(rgb(c_text()))
            })
            .when(!selected, |el| {
                el.border_color(rgb(c_border()))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(rgb(c_subtle())))
            })
            .child(label)
    }

    /// Status dot; pulses while the agent is working.
    fn status_dot(&self, status: Status, id: impl Into<ElementId>, size: f32) -> AnyElement {
        let dot = div()
            .id(id)
            .size(px(size))
            .flex_none()
            .rounded_full()
            .bg(rgb(Self::status_color(status)));
        if status == Status::Working {
            dot.animated(
                "pulse",
                Animation::new(Duration::from_millis(1200))
                    .repeat()
                    .with_easing(pulsating_between(0.3, 1.0)),
                |el, delta| el.opacity(delta),
            )
            .into_any_element()
        } else {
            dot.into_any_element()
        }
    }

    /// The agent's glyph in its colour, without a background, for dense rows.
    fn provider_mark(&self, provider: &agent::Provider, size: f32) -> Div {
        div()
            .w(px(size + 2.))
            .flex_none()
            .flex()
            .justify_center()
            .text_size(px(size))
            .line_height(px(size + 2.))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(provider.color))
            .child(provider.mark)
    }

    fn provider_badge(&self, provider: &agent::Provider, size: f32) -> Div {
        div()
            .size(px(size))
            .flex_none()
            .rounded(px((size * 0.28).min(6.)))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(size * 0.55))
            .bg(rgb(settings::mix(
                provider.color,
                c_canvas(),
                if is_dark() { 0.2 } else { 0.12 },
            )))
            .text_color(rgb(provider.color))
            .font_weight(FontWeight::BOLD)
            .child(provider.mark)
    }

    // ---------- title bar ----------

    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs = div()
            .flex()
            .items_end()
            .h_full()
            .min_w_0()
            .overflow_hidden();
        for &id in &self.tabs {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let active = self.selected == Some(id) && self.view == View::Session;
            let working = self.handles.contains_key(&Target::Session(id));
            tabs = tabs.child(
                div()
                    .id(("tab", id as usize))
                    .group("tab")
                    .h(px(34.))
                    .min_w(px(120.))
                    .max_w(px(210.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_t_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(active, |el| {
                        el.bg(rgb(c_canvas()))
                            .text_color(rgb(c_text()))
                            .border_1()
                            .border_b_0()
                            .border_color(rgb(c_border()))
                    })
                    .when(!active, |el| {
                        el.text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(rgb(c_subtle())))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(id);
                        cx.notify();
                    }))
                    .child(self.provider_mark(provider, 12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .id(("tab-x", id as usize))
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .hover(|el| el.bg(rgb(c_border())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(id);
                                cx.notify();
                            }))
                            .child(if working || session.unread {
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(rgb(if working { c_blue() } else { c_amber() }))
                                    .group_hover("tab", |el| el.opacity(0.))
                                    .into_any_element()
                            } else {
                                icon(IconName::X, 11., c_muted()).into_any_element()
                            }),
                    ),
            );
        }
        let (added, removed) = self
            .changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        let show_stats = self.view == View::Session && !self.changes.is_empty();
        div()
            .h(px(40.))
            .flex_none()
            .flex()
            .items_end()
            .gap_2()
            .pl(px(if cfg!(target_os = "macos") { 80. } else { 8. }))
            .pr_2()
            .border_b_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_sidebar()))
            .window_control_area(WindowControlArea::Drag)
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        self.ghost_button("toggle-sidebar", IconName::PanelLeft, "Toggle sidebar")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_sidebar = !this.show_sidebar;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.ghost_button("open-palette", IconName::Search, "Quick open")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_palette(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_end()
                    .child(tabs),
            )
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(show_stats, |el| {
                        el.child(
                            div()
                                .flex()
                                .gap_1()
                                .px_2()
                                .text_size(px(12.))
                                .font_family(self.mono_font.clone())
                                .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                                .child(div().text_color(rgb(c_del())).child(format!("−{removed}"))),
                        )
                    })
                    .when(
                        self.view == View::Session && self.current().is_some(),
                        |el| {
                            el.child(
                                self.ghost_button(
                                    "toggle-panel",
                                    IconName::PanelRight,
                                    "Toggle the Changes panel",
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.show_panel = !this.show_panel;
                                        cx.notify();
                                    },
                                )),
                            )
                        },
                    ),
            )
    }

    // ---------- sidebar ----------

    fn nav_item(
        &self,
        id: &'static str,
        name: IconName,
        label: &'static str,
        count: usize,
        view: View,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.view == view;
        div()
            .id(id)
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(rgb(if selected { c_text() } else { c_secondary() }))
            .when(selected, |el| el.bg(rgb(c_active())))
            .hover(|el| el.bg(rgb(c_subtle())).text_color(rgb(c_text())))
            .on_click(cx.listener(move |this, _, _, cx| {
                if view == View::Settings {
                    this.open_settings();
                } else if view == View::History {
                    this.open_history();
                } else {
                    this.view = view;
                }
                cx.notify();
            }))
            .child(icon(name, 15., if selected { c_text() } else { c_muted() }))
            .child(div().flex_1().child(label))
            .when(count > 0 && view != View::Inbox, |el| {
                el.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(count.to_string()),
                )
            })
            .when(count > 0 && view == View::Inbox, |el| {
                el.child(
                    div()
                        .min_w(px(18.))
                        .h(px(16.))
                        .px_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(c_blue()))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(0xffffff))
                        .child(count.to_string()),
                )
            })
    }

    /// Small icon button shown on hover over a row.
    fn row_action(
        &self,
        id: impl Into<ElementId>,
        name: IconName,
        tip: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(rgb(c_raised()))
            .border_1()
            .border_color(rgb(c_border()))
            .cursor_pointer()
            .hover(|el| el.bg(rgb(c_subtle())))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .on_click(on_click)
            .child(icon(name, 12., c_secondary()))
    }

    /// A session in the sidebar: agent and age, title, then branch and change counts.
    fn session_row(&self, session: &Session, cx: &mut Context<Self>) -> impl IntoElement {
        let id = session.id;
        let provider = agent::provider(&session.provider);
        let selected = self.selected == Some(id) && self.view == View::Session;
        let unread = session.unread;
        let pinned = session.pinned;
        let working = session.status == Status::Working;
        let model = session
            .selections()
            .get("model")
            .filter(|m| !m.is_empty())
            .cloned();
        let when = match self.started.get(&id) {
            Some(t) if working => elapsed(t.elapsed()),
            _ => session
                .messages
                .last()
                .map(|m| m.time.clone())
                .unwrap_or_default(),
        };
        let stats = self.stats.get(&id).copied();
        div()
            .id(("session", id as usize))
            .relative()
            .group("session-row")
            .mx_1()
            .px_2()
            .py(px(7.))
            .v_flex()
            .gap(px(2.))
            .rounded_md()
            .cursor_pointer()
            .when(selected, |el| el.bg(rgb(c_active())))
            .when(!selected, |el| el.hover(|el| el.bg(rgb(c_subtle()))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(id);
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(self.provider_mark(provider, 11.))
                    .child(div().flex_1().min_w_0().truncate().child(match model {
                        Some(m) => format!("{} · {m}", provider.name),
                        None => provider.name.to_string(),
                    }))
                    .child(
                        div()
                            .flex_none()
                            .text_color(rgb(if working { c_blue() } else { c_muted() }))
                            .child(when),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(if unread || selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(rgb(c_text()))
                            .child(session.title.clone()),
                    )
                    .when(pinned, |el| el.child(icon(IconName::Pin, 11., c_muted())))
                    .child(if unread {
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(c_blue()))
                            .into_any_element()
                    } else if session.status == Status::Failed {
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(c_red()))
                            .into_any_element()
                    } else if working {
                        self.status_dot(Status::Working, ("dot", id as usize), 7.)
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(icon(IconName::GitBranch, 11., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(session.branch.clone()),
                    )
                    .when_some(stats.filter(|s| s.0 > 0), |el, (_, a, r)| {
                        el.child(
                            div()
                                .flex()
                                .gap_1()
                                .font_family(self.mono_font.clone())
                                .child(div().text_color(rgb(c_add())).child(format!("+{a}")))
                                .child(div().text_color(rgb(c_del())).child(format!("−{r}"))),
                        )
                    }),
            )
            .child(
                div()
                    .absolute()
                    .right(px(6.))
                    .top(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .opacity(0.)
                    .group_hover("session-row", |el| el.opacity(1.))
                    .child(self.row_action(
                        ("pin", id as usize),
                        if pinned {
                            IconName::PinOff
                        } else {
                            IconName::Pin
                        },
                        if pinned { "Unpin" } else { "Pin to top" },
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_pin(id);
                            cx.notify();
                        }),
                    ))
                    .child(self.row_action(
                        ("archive", id as usize),
                        IconName::Archive,
                        "Archive",
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_archive(id);
                            cx.notify();
                        }),
                    )),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.search.read(cx).value().to_lowercase();
        let mut projects = div().v_flex().gap(px(2.)).pb_2();
        for project in &self.state.projects {
            let pid = project.id;
            let open = self.expanded.contains(&pid);
            let mut sessions: Vec<&Session> = self
                .state
                .sessions
                .iter()
                .filter(|s| s.project_id == pid && !s.archived)
                .filter(|s| {
                    filter.is_empty()
                        || s.title.to_lowercase().contains(&filter)
                        || s.branch.to_lowercase().contains(&filter)
                        || project.name.to_lowercase().contains(&filter)
                })
                .collect();
            sessions.sort_by_key(|s| !s.pinned);
            if !filter.is_empty() && sessions.is_empty() {
                continue;
            }
            projects = projects.child(
                div()
                    .id(("project", pid as usize))
                    .group("project")
                    .h(px(28.))
                    .mt_2()
                    .mx_1()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(pos) = this.expanded.iter().position(|p| *p == pid) {
                            this.expanded.remove(pos);
                        } else {
                            this.expanded.push(pid);
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_secondary()))
                            .child(project.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .group_hover("project", |el| el.opacity(0.))
                            .child(if sessions.is_empty() {
                                String::new()
                            } else {
                                sessions.len().to_string()
                            }),
                    )
                    .child(
                        div()
                            .id(("project-new", pid as usize))
                            .absolute()
                            .right(px(10.))
                            .p(px(3.))
                            .rounded_sm()
                            .cursor_pointer()
                            .opacity(0.)
                            .group_hover("project", |el| el.opacity(1.))
                            .hover(|el| el.bg(rgb(c_border())))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "New session in this project",
                                )
                                .build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.new_project = Some(pid);
                                this.view = View::NewSession;
                                cx.notify();
                            }))
                            .child(icon(IconName::Plus, 13., c_secondary())),
                    )
                    .relative(),
            );
            if open || !filter.is_empty() {
                if sessions.is_empty() {
                    projects = projects.child(
                        div()
                            .ml(px(30.))
                            .py_1()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child("No sessions yet"),
                    );
                }
                for session in sessions {
                    projects = projects.child(self.session_row(session, cx));
                }
            }
        }
        let unread = self
            .state
            .sessions
            .iter()
            .filter(|s| s.unread && !s.archived)
            .count();
        div()
            .w(px(264.))
            .flex_none()
            .h_full()
            .v_flex()
            .border_r_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_sidebar()))
            .child(
                div()
                    .p_2()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .id("new-session")
                            .h(px(32.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .bg(rgb(c_raised()))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .hover(|el| el.bg(rgb(c_subtle())))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view = View::NewSession;
                                cx.notify();
                            }))
                            .child(icon(IconName::SquarePen, 14., c_text()))
                            .child(div().flex_1().child("New session"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(c_muted()))
                                    .child(format!("{MOD_KEY}N")),
                            ),
                    )
                    .child(
                        div()
                            .h(px(28.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .bg(rgb(c_subtle()))
                            .child(icon(IconName::Search, 13., c_muted()))
                            .child(
                                Input::new(&self.search)
                                    .appearance(false)
                                    .h(px(26.))
                                    .text_size(px(12.5)),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .v_flex()
                            .child(self.nav_item(
                                "inbox",
                                IconName::Inbox,
                                "Inbox",
                                unread,
                                View::Inbox,
                                cx,
                            ))
                            .child(self.nav_item(
                                "all-sessions",
                                IconName::Layers,
                                "All sessions",
                                self.state.sessions.len(),
                                View::AllSessions,
                                cx,
                            ))
                            .child(self.nav_item(
                                "worktrees",
                                IconName::GitBranch,
                                "Worktrees",
                                self.state.sessions.iter().filter(|s| s.isolated).count(),
                                View::Worktrees,
                                cx,
                            ))
                            .child(self.nav_item(
                                "history",
                                IconName::RotateCcwClock,
                                "History",
                                0,
                                View::History,
                                cx,
                            )),
                    ),
            )
            .child(
                div()
                    .mt_1()
                    .px_3()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
                            .child("PROJECTS"),
                    )
                    .child(
                        div()
                            .id("add-project")
                            .p_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|el| el.bg(rgb(c_subtle())))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("Add a project folder")
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.pick_project(cx)))
                            .child(icon(IconName::FolderPlus, 14., c_muted())),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(projects),
            )
            .child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .child(self.nav_item(
                        "settings",
                        IconName::Settings,
                        "Settings",
                        0,
                        View::Settings,
                        cx,
                    )),
            )
    }

    fn step_row(&self, text: &str) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .h(px(24.))
            .text_size(px(12.5))
            .child(icon(IconName::Check, 12., c_muted()))
            .child(
                div()
                    .truncate()
                    .text_color(rgb(c_secondary()))
                    .child(text.to_string()),
            )
    }

    fn changes_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (added, removed) = self
            .changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        let mut rows = div().v_flex();
        for (i, change) in self.changes.iter().take(4).enumerate() {
            let path = change.path.clone();
            let (dir, name) = match change.path.rsplit_once('/') {
                Some((d, n)) => (format!("{d}/"), n.to_string()),
                None => (String::new(), change.path.clone()),
            };
            rows = rows.child(
                div()
                    .id(("card-file", i))
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.right_tab = RightTab::Changes;
                        this.show_panel = true;
                        this.open_change(&path);
                        cx.notify();
                    }))
                    .child(icon(IconName::File, 12., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .text_size(px(12.5))
                            .child(div().flex_none().child(name))
                            .child(
                                div()
                                    .ml_2()
                                    .truncate()
                                    .text_color(rgb(c_muted()))
                                    .child(dir),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_add()))
                            .child(format!("+{}", change.added)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_del()))
                            .child(format!("−{}", change.removed)),
                    ),
            );
        }
        let more = self.changes.len().saturating_sub(4);
        div()
            .id("changes-card")
            .mt_1()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .overflow_hidden()
            .child(
                div()
                    .id("changes-card-head")
                    .h(px(32.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(rgb(c_subtle()))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.right_tab = RightTab::Changes;
                        this.show_panel = true;
                        cx.notify();
                    }))
                    .child(icon(IconName::GitCompare, 13., c_secondary()))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!(
                                "{} file{} changed",
                                self.changes.len(),
                                if self.changes.len() == 1 { "" } else { "s" }
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_add()))
                            .child(format!("+{added}")),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_del()))
                            .child(format!("−{removed}")),
                    )
                    .child(
                        div()
                            .ml_1()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .hover(|el| el.text_color(rgb(c_text())))
                            .child("Review"),
                    ),
            )
            .child(rows)
            .when(more > 0, |el| {
                el.child(
                    div()
                        .h(px(26.))
                        .px_3()
                        .flex()
                        .items_center()
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .text_size(px(12.))
                        .text_color(rgb(c_muted()))
                        .child(format!("{more} more")),
                )
            })
            .animated(
                SharedString::from(format!(
                    "card-{}-{}",
                    self.selected.unwrap_or(0),
                    self.changes.len()
                )),
                Animation::new(Duration::from_millis(400)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d),
            )
    }

    fn message_view(
        &self,
        index: usize,
        message: &Message,
        provider: &agent::Provider,
        show_changes: bool,
        working: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let anim = SharedString::from(format!("msg-{}-{}", self.selected.unwrap_or(0), index));
        let fade = |el: Stateful<Div>| {
            el.animated(
                anim.clone(),
                Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                |el, delta| el.opacity(delta).mt(px((1.0 - delta) * 6.0)),
            )
        };
        let markdown = |text: String| {
            TextView::markdown(
                SharedString::from(format!("body-{}-{}", self.selected.unwrap_or(0), index)),
                text,
            )
            .selectable(true)
        };
        if message.role == Role::User {
            return fade(
                div()
                    .id(("message", index))
                    .w_full()
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .max_w(px(560.))
                            .px(px(14.))
                            .py(px(10.))
                            .rounded(px(14.))
                            .bg(rgb(c_active()))
                            .text_size(px(13.5))
                            .text_color(rgb(c_text()))
                            .whitespace_normal()
                            .child(markdown(message.text.clone())),
                    ),
            )
            .into_any_element();
        }
        if message.role == Role::System {
            return fade(
                div()
                    .id(("message", index))
                    .px_3()
                    .py(px(10.))
                    .flex()
                    .items_start()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(settings::mix(c_red(), c_canvas(), 0.35)))
                    .bg(rgb(settings::mix(c_red(), c_canvas(), 0.07)))
                    .text_size(px(13.))
                    .child(
                        div()
                            .pt(px(2.))
                            .child(icon(IconName::TriangleAlert, 14., c_red())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(markdown(message.text.clone())),
                    ),
            )
            .into_any_element();
        }
        let key = (self.selected.unwrap_or(0), index);
        let live = working;
        let open = live || self.open_steps.contains(&key);
        let mut steps = div()
            .v_flex()
            .ml(px(5.))
            .pl_3()
            .border_l_1()
            .border_color(rgb(c_border()));
        let shown = if open { message.steps.len() } else { 0 };
        for (i, step) in message.steps.iter().take(shown).enumerate() {
            steps = steps.child(stagger(
                div().child(self.step_row(step)),
                SharedString::from(format!("step-{}-{}-{}", key.0, index, i)),
                i,
            ));
        }
        let n = message.steps.len();
        let copy_text = message.text.clone();
        let summary = div()
            .id(("steps-toggle", index))
            .h(px(26.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(rgb(c_secondary()))
            .hover(|el| el.text_color(rgb(c_text())))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.open_steps.remove(&key) {
                    this.open_steps.insert(key);
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
            .child(format!(
                "{} {} step{}",
                if live { "Working ·" } else { "Worked ·" },
                n,
                if n == 1 { "" } else { "s" }
            ));
        fade(
            div()
                .id(("message", index))
                .group("message")
                .v_flex()
                .gap_2()
                .when(n > 0, |el| {
                    el.child(summary).when(open, |el| el.child(steps))
                })
                .child(
                    div()
                        .w_full()
                        .whitespace_normal()
                        .text_size(px(13.5))
                        .text_color(rgb(c_text()))
                        .child(if message.text.is_empty() {
                            working_indicator().into_any_element()
                        } else {
                            markdown(message.text.clone()).into_any_element()
                        }),
                )
                .when(show_changes && !working, |el| {
                    el.child(self.changes_card(cx))
                })
                .when(!message.text.is_empty(), |el| {
                    el.child(
                        div()
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(self.provider_mark(provider, 12.))
                            .child(provider.name)
                            .when(!message.time.is_empty(), |el| {
                                el.child("·").child(message.time.clone())
                            })
                            .when(!message.meta.is_empty(), |el| {
                                el.child("·").child(message.meta.clone())
                            })
                            .child(
                                self.ghost_button_sized(
                                    ("copy", index),
                                    IconName::Copy,
                                    "Copy",
                                    22.,
                                )
                                .opacity(0.)
                                .group_hover("message", |el| el.opacity(1.))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_text.clone(),
                                        ));
                                        this.info = Some((
                                            "Copied to the clipboard.".into(),
                                            std::time::Instant::now(),
                                        ));
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                }),
        )
        .into_any_element()
    }

    /// The sessions racing on the same task, to compare and keep one.
    fn race_strip(&self, session: &Session, cx: &mut Context<Self>) -> impl IntoElement {
        let racers: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.race == session.race)
            .collect();
        let me = session.id;
        let mut chips = div().flex().flex_wrap().gap_2();
        for racer in &racers {
            let id = racer.id;
            let provider = agent::provider(&racer.provider);
            let current = id == me;
            let changes = if current {
                let (a, r) = self
                    .changes
                    .iter()
                    .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
                Some((self.changes.len(), a, r))
            } else {
                self.race_stats.get(&id).copied()
            };
            chips = chips.child(
                div()
                    .id(("racer", id as usize))
                    .px_3()
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .cursor_pointer()
                    .when(current, |el| {
                        el.border_color(rgb(c_blue())).bg(rgb(c_selected()))
                    })
                    .when(!current, |el| {
                        el.border_color(rgb(c_border()))
                            .hover(|el| el.bg(rgb(c_subtle())))
                    })
                    .when(racer.archived, |el| el.opacity(0.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(id);
                        cx.notify();
                    }))
                    .child(self.provider_badge(provider, 18.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(provider.name),
                    )
                    .child(self.status_dot(racer.status, ("racer-dot", id as usize), 7.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(c_secondary()))
                            .child(Self::status_label(racer.status)),
                    )
                    .when_some(changes, |el, (files, a, r)| {
                        el.child(div().text_xs().text_color(rgb(c_muted())).child(format!(
                            "{files} file{} · +{a} −{r}",
                            if files == 1 { "" } else { "s" }
                        )))
                    }),
            );
        }
        let others = racers.iter().filter(|s| s.id != me && !s.archived).count();
        div()
            .p_3()
            .v_flex()
            .gap_2()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_subtle()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::Swords, 14., c_blue()))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Race between {} agents", racers.len())),
                    )
                    .when(others > 0, |el| {
                        el.child(
                            self.button("keep-racer", "Keep this one", false)
                                .h(px(28.))
                                .text_xs()
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Archive the other agents' sessions. Their worktrees stay until you delete them.",
                                    )
                                    .build(window, cx)
                                })
                                .child(icon(IconName::Trophy, 13., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.keep_racer(me);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(chips)
    }

    fn session_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.current() else {
            return self.empty_state(cx).into_any_element();
        };
        let provider = agent::provider(&session.provider);
        let working = self.handles.contains_key(&Target::Session(session.id));
        let id = session.id;
        let confirming = self.confirm_delete == Some(id);
        let isolated = session.isolated;
        let last_agent = session.messages.iter().rposition(|m| m.role == Role::Agent);
        let mut list = div()
            .w_full()
            .max_w(px(780.))
            .mx_auto()
            .v_flex()
            .gap_6()
            .px_6()
            .pt_6()
            .pb_8();
        if session.race != 0 {
            list = list.child(self.race_strip(session, cx));
        }
        if let Some(first) = session.messages.first().filter(|m| !m.time.is_empty()) {
            list = list.child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_xs()
                    .text_color(rgb(c_muted()))
                    .text_size(px(11.5))
                    .child(format!("Started {}", first.time)),
            );
        }
        for (i, message) in session.messages.iter().enumerate() {
            let show = Some(i) == last_agent && !self.changes.is_empty();
            list = list.child(self.message_view(
                i,
                message,
                provider,
                show,
                working && Some(i) == last_agent,
                cx,
            ));
        }
        if let Some(task) = self.pending_start.get(&session.id) {
            list = list.child(
                div()
                    .p_4()
                    .v_flex()
                    .gap_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_subtle()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(icon(IconName::Loader, 14., c_blue()))
                            .child("Preparing the worktree"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child(format!(
                                "The setup command is running in the Terminal. {} starts on “{}” when it finishes.",
                                provider.name,
                                task.lines().next().unwrap_or_default()
                            )),
                    ),
            );
        }
        let status = match self.started.get(&id).filter(|_| working) {
            Some(start) => format!("Working · {}", elapsed(start.elapsed())),
            None => Self::status_label(session.status).to_string(),
        };
        let header = div()
            .h(px(44.))
            .flex_none()
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(rgb(c_border()))
            .child(if self.renaming {
                div()
                    .w(px(360.))
                    .child(Input::new(&self.rename).bordered(true).h(px(28.)))
                    .into_any_element()
            } else {
                div()
                    .id("session-title")
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .flex_shrink(1.)
                    .cursor_pointer()
                    .group("title")
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Click to rename")
                            .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_rename(window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .opacity(0.)
                            .group_hover("title", |el| el.opacity(1.))
                            .child(icon(IconName::Pencil, 12., c_muted())),
                    )
                    .into_any_element()
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child(icon(IconName::GitBranch, 12., c_muted()))
                    .child(session.branch.clone())
                    .when(isolated, |el| {
                        el.child(
                            div()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded_sm()
                                .bg(rgb(c_subtle()))
                                .text_size(px(11.))
                                .child("worktree"),
                        )
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(self.status_dot(session.status, "hdr-dot", 6.))
                    .child(status),
            )
            .when(working, |el| {
                el.child(
                    self.button("stop", "Stop", false)
                        .h(px(26.))
                        .text_color(rgb(c_red()))
                        .child(icon(IconName::CircleStop, 13., c_red()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop();
                            cx.notify();
                        })),
                )
            })
            .when(confirming, |el| {
                el.child(
                    self.button("cancel-delete", "Cancel", false)
                        .h(px(26.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_delete = None;
                            cx.notify();
                        })),
                )
                .child(
                    self.button(
                        "delete",
                        if isolated && !self.demo && remove_worktree() {
                            "Delete session & worktree"
                        } else {
                            "Delete session"
                        },
                        false,
                    )
                    .h(px(26.))
                    .border_color(rgb(c_red()))
                    .text_color(rgb(c_red()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_session(id);
                        cx.notify();
                    })),
                )
            })
            .when(!confirming, |el| {
                el.child(
                    self.ghost_button("delete", IconName::Trash, "Delete session")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if confirm_delete() {
                                this.confirm_delete = Some(id);
                            } else {
                                this.delete_session(id);
                            }
                            cx.notify();
                        })),
                )
            });
        div()
            .flex_1()
            .min_w_0()
            .v_flex()
            .child(header)
            .child(
                div()
                    .id("chat")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.chat_scroll)
                    .child(list),
            )
            .child(self.composer(provider, working, cx))
            .into_any_element()
    }

    fn menu_button(
        &self,
        id: SharedString,
        key: &str,
        lead: Option<AnyElement>,
        caption: Option<String>,
        label: String,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.menu.as_deref() == Some(key);
        let key = key.to_string();
        div()
            .id(id)
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(rgb(c_secondary()))
            .when(open, |el| el.bg(rgb(c_active())).text_color(rgb(c_text())))
            .hover(|el| el.bg(rgb(c_subtle())).text_color(rgb(c_text())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.menu = if this.menu.as_deref() == Some(key.as_str()) {
                    None
                } else {
                    Some(key.clone())
                };
                cx.notify();
            }))
            .when_some(lead, |el, lead| el.child(lead))
            .when_some(caption, |el, caption| {
                el.child(div().text_color(rgb(c_muted())).child(caption))
            })
            .child(label)
            .child(icon(IconName::ChevronDown, 11., c_muted()))
    }

    /// Popup list anchored above its button; picking an item or clicking elsewhere closes it.
    fn menu_popup(
        &self,
        key: &str,
        items: Vec<(String, String, bool, Option<AnyElement>)>,
        on_pick: impl Fn(&mut Self, usize) + 'static + Clone,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = div().v_flex().p_1();
        for (i, (label, desc, selected, lead)) in items.into_iter().enumerate() {
            let on_pick = on_pick.clone();
            list = list.child(
                div()
                    .id(("menu-item", i))
                    .min_h(px(28.))
                    .px_2()
                    .py(px(5.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .hover(|el| el.bg(rgb(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        on_pick(this, i);
                        this.menu = None;
                        cx.notify();
                    }))
                    .when_some(lead, |el, lead| el.child(lead))
                    .child(div().flex_1().v_flex().child(div().child(label)).when(
                        !desc.is_empty(),
                        |el| {
                            el.child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .max_w(px(280.))
                                    .whitespace_normal()
                                    .child(desc),
                            )
                        },
                    ))
                    .when(selected, |el| {
                        el.child(icon(IconName::Check, 13., c_text()))
                    }),
            );
        }
        fade(
            div()
                .absolute()
                .bottom(px(32.))
                .left_0()
                .min_w(px(220.))
                .max_h(px(340.))
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(rgb(c_raised()))
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id(SharedString::from(format!("popup-scroll-{key}")))
                        .max_h(px(340.))
                        .overflow_y_scroll()
                        .child(list),
                ),
            SharedString::from(format!("menu-{key}")),
            140,
        )
        .into_any_element()
    }

    fn composer(
        &self,
        provider: &agent::Provider,
        working: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(session) = self.current() else {
            return div().into_any_element();
        };
        let sid = session.id;
        let isolated = session.isolated;
        let empty_prompt = self.prompt.read(cx).value().trim().is_empty();
        let sel = session.selections();
        let controls = caps::controls(&session.provider, &sel);
        let mut popup: Option<(String, AnyElement)> = None;

        if self.menu.as_deref() == Some("provider") {
            let items = agent::PROVIDERS
                .iter()
                .map(|p| {
                    (
                        p.name.to_string(),
                        String::new(),
                        p.id == session.provider,
                        Some(self.provider_badge(p, 18.).into_any_element()),
                    )
                })
                .collect();
            popup = Some((
                "provider".into(),
                self.menu_popup(
                    "provider",
                    items,
                    move |this, i| {
                        if let Some(s) = this.state.session_mut(sid) {
                            s.switch_provider(agent::PROVIDERS[i].id);
                        }
                        this.save();
                    },
                    cx,
                ),
            ));
        }
        for control in &controls {
            if self.menu.as_deref() != Some(control.id) {
                continue;
            }
            let current = sel.get(control.id).cloned().unwrap_or_default();
            let items = control
                .choices
                .iter()
                .map(|c| (c.label.clone(), c.desc.clone(), c.value == current, None))
                .collect();
            let (id, values): (&'static str, Vec<String>) = (
                control.id,
                control.choices.iter().map(|c| c.value.clone()).collect(),
            );
            popup = Some((
                id.to_string(),
                self.menu_popup(
                    id,
                    items,
                    move |this, i| {
                        if let Some(s) = this.state.session_mut(sid) {
                            let v = values[i].clone();
                            if id == "model" {
                                // Effort and speed depend on the model, so reset them with it.
                                s.opts.remove("effort");
                                s.opts.remove("tier");
                                s.model.clear();
                            }
                            if id == "mode" {
                                s.mode.clear();
                            }
                            if v.is_empty() {
                                s.opts.remove(id);
                                s.opts.insert(id.to_string(), String::new());
                            } else {
                                s.opts.insert(id.to_string(), v);
                            }
                        }
                        this.save();
                    },
                    cx,
                ),
            ));
        }

        let mut chips = div().flex().flex_wrap().items_center().gap_1().child(
            div()
                .relative()
                .child(self.menu_button(
                    SharedString::from("chip-provider"),
                    "provider",
                    Some(self.provider_mark(provider, 13.).into_any_element()),
                    None,
                    provider.name.to_string(),
                    cx,
                ))
                .when(popup.as_ref().is_some_and(|(k, _)| k == "provider"), |el| {
                    el.children(popup.take().map(|(_, p)| p))
                }),
        );
        for control in &controls {
            let current = sel.get(control.id).cloned().unwrap_or_default();
            let label = control
                .choices
                .iter()
                .find(|c| c.value == current)
                .map(|c| c.label.clone())
                .unwrap_or_else(|| {
                    if current.is_empty() {
                        control.choices[0].label.clone()
                    } else {
                        agent::generic_mode_label(&current)
                            .map(str::to_string)
                            .unwrap_or_else(|| current.clone())
                    }
                });
            chips = chips.child(
                div()
                    .relative()
                    .child(self.menu_button(
                        SharedString::from(format!("chip-{}", control.id)),
                        control.id,
                        None,
                        Some(control.label.clone()),
                        label,
                        cx,
                    ))
                    .when(popup.as_ref().is_some_and(|(k, _)| k == control.id), |el| {
                        el.children(popup.take().map(|(_, p)| p))
                    }),
            );
        }
        if controls.is_empty() && !caps::get(&session.provider).detected {
            chips = chips.child(
                div()
                    .text_xs()
                    .text_color(rgb(c_muted()))
                    .child("Detecting options…"),
            );
        }

        let ready = !working && !empty_prompt;
        div()
            .w_full()
            .max_w(px(780.))
            .mx_auto()
            .px_6()
            .pb_3()
            .v_flex()
            .gap(px(6.))
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_2()
                    .v_flex()
                    .gap_2()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_raised()))
                    .shadow_sm()
                    .child(
                        div().px_1().text_size(px(13.5)).child(
                            Textarea::new(&self.prompt)
                                .appearance(false)
                                .bordered(false)
                                .h(px(48.)),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(div().flex_1().min_w_0().child(chips))
                            .child(
                                div()
                                    .id("send")
                                    .size(px(28.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(rgb(if ready { c_text() } else { c_active() }))
                                    .when(ready, |el| el.hover(|el| el.opacity(0.85)))
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new("Send")
                                            .build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_message(window, cx);
                                        cx.notify();
                                    }))
                                    .child(if working {
                                        icon(IconName::Loader, 14., c_secondary()).animated(
                                            "spin",
                                            Animation::new(Duration::from_millis(900)).repeat(),
                                            |i, d| i.rotate(percentage(d)),
                                        )
                                    } else {
                                        icon(
                                            IconName::ArrowUp,
                                            15.,
                                            if ready { c_canvas() } else { c_muted() },
                                        )
                                        .into_any_element()
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .px_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon(
                                if isolated {
                                    IconName::GitBranch
                                } else {
                                    IconName::Folder
                                },
                                11.,
                                c_muted(),
                            ))
                            .child(if isolated {
                                "Isolated worktree"
                            } else {
                                "Project directory"
                            }),
                    )
                    .child(if cx.global::<AppSettings>().send_on_enter {
                        "↵ send  ·  ⇧↵ new line".to_string()
                    } else {
                        format!("{MOD_KEY}↵ send  ·  ↵ new line")
                    }),
            )
            .into_any_element()
    }

    fn empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tip = |key: String, text: &'static str| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .text_sm()
                .text_color(rgb(c_secondary()))
                .child(
                    div()
                        .min_w(px(64.))
                        .h(px(24.))
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .bg(rgb(c_subtle()))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_text()))
                        .child(key),
                )
                .child(text)
        };
        let has_projects = !self.state.projects.is_empty();
        div()
            .flex_1()
            .v_flex()
            .items_center()
            .justify_center()
            .gap_5()
            .px_8()
            .child(
                div()
                    .size(px(56.))
                    .rounded_2xl()
                    .bg(rgb(c_selected()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::Bot, 28., c_blue())),
            )
            .child(
                div()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child(if has_projects {
                                "Start a session"
                            } else {
                                "Add your first project"
                            }),
                    )
                    .child(
                        div()
                            .max_w(px(420.))
                            .text_center()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child(if has_projects {
                                "Give an agent a task. It works in its own git worktree, so several can run side by side without colliding."
                            } else {
                                "Pick any folder on this computer. Git repositories get an isolated worktree for every session."
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        self.button("empty-new", "New session", true)
                            .child(icon(IconName::Plus, 14., c_canvas()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view = View::NewSession;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("empty-add", "Add project", false)
                            .child(icon(IconName::FolderPlus, 14., c_secondary()))
                            .on_click(cx.listener(|this, _, _, cx| this.pick_project(cx))),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .v_flex()
                    .gap_2()
                    .child(tip(format!("{MOD_KEY}N"), "New session"))
                    .child(tip(format!("{MOD_KEY},"), "Settings"))
                    .child(tip(format!("{MOD_KEY}B"), "Show or hide the changes panel"))
                    .child(tip(format!("{MOD_KEY}."), "Stop the running agent")),
            )
    }

    fn page(&self, title: &'static str, body: impl IntoElement) -> AnyElement {
        div()
            .flex_1()
            .min_w_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .max_w(px(780.))
                    .mx_auto()
                    .p_8()
                    .v_flex()
                    .gap_4()
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    fn new_session_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = |t: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_muted()))
                .child(t)
        };
        let card = |id: (&'static str, usize), selected: bool| {
            div()
                .id(id)
                .flex_1()
                .min_w(px(180.))
                .p_3()
                .flex()
                .items_center()
                .gap_3()
                .rounded_xl()
                .border_1()
                .cursor_pointer()
                .when(selected, |el| {
                    el.border_color(rgb(c_blue())).bg(rgb(c_selected()))
                })
                .when(!selected, |el| {
                    el.border_color(rgb(c_border()))
                        .hover(|el| el.bg(rgb(c_subtle())))
                })
        };

        let mut projects = div().flex().flex_wrap().gap_2();
        for (i, project) in self.state.projects.iter().enumerate() {
            let pid = project.id;
            projects = projects.child(
                self.chip(
                    ("np", i),
                    project.name.clone(),
                    self.new_project == Some(pid),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.new_project = Some(pid);
                    cx.notify();
                })),
            );
        }
        projects = projects.child(
            self.button("add-proj", "Add project…", false)
                .h(px(32.))
                .child(icon(IconName::FolderPlus, 14., c_secondary()))
                .on_click(cx.listener(|this, _, _, cx| this.pick_project(cx))),
        );

        let mut providers = div().flex().flex_wrap().gap_2();
        for (i, p) in agent::PROVIDERS.iter().enumerate() {
            let caps = caps::get(p.id);
            let installed = agent::available(p.bin, &cx.global::<AppSettings>().agent(p.id).binary);
            let detail = if !installed {
                format!("{} not found on PATH", p.bin)
            } else if caps.version.is_empty() {
                "Installed".to_string()
            } else {
                format!("v{}", caps.version.trim_start_matches('v'))
            };
            let picked = if self.race_mode {
                self.new_race.contains(&i)
            } else {
                self.new_provider == i
            };
            providers = providers.child(
                card(("pv", i), picked)
                    .when(!installed, |el| el.opacity(0.6))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.race_mode {
                            if let Some(pos) = this.new_race.iter().position(|p| *p == i) {
                                this.new_race.remove(pos);
                            } else {
                                this.new_race.push(i);
                                this.new_race.sort();
                            }
                        } else {
                            this.new_provider = i;
                        }
                        cx.notify();
                    }))
                    .when(self.race_mode, |el| {
                        el.child(icon(
                            if picked {
                                IconName::SquareCheck
                            } else {
                                IconName::Square
                            },
                            16.,
                            if picked { c_blue() } else { c_muted() },
                        ))
                    })
                    .child(self.provider_badge(p, 30.))
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(p.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(if installed { c_muted() } else { c_red() }))
                                    .child(detail),
                            ),
                    ),
            );
        }

        let project = self
            .new_project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id));
        let is_git = self.demo || project.is_some_and(|p| git::is_repo(&p.path));
        let racing = self.racing() && is_git;
        let isolated = (self.new_isolated || racing) && is_git;
        let typed = git::sanitize_branch(&self.branch.read(cx).value());
        let branch = if racing {
            format!(
                "{}-*",
                if typed.is_empty() {
                    format!("runo/race-{}", self.state.next_id + 1)
                } else {
                    typed
                }
            )
        } else if typed.is_empty() {
            format!("runo/session-{}", self.state.next_id + 1)
        } else {
            typed
        };
        let where_to = match project {
            None => "Add a project to start.".to_string(),
            Some(_) if racing => format!(
                "Each agent gets its own worktree and branch: {}",
                self.new_race
                    .iter()
                    .map(|i| branch.replace('*', agent::PROVIDERS[*i].id))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some(p) if isolated => format!(
                "Creates branch {branch} in {}",
                git::worktree_path(&p.path, &branch).display()
            ),
            Some(p) if is_git => format!("Runs on the current branch, directly in {}", p.path),
            Some(p) => format!("Runs directly in {}", p.path),
        };
        let provider = &agent::PROVIDERS[self.new_provider];
        let task_empty = self.new_task.read(cx).value().trim().is_empty();
        let ready = project.is_some() && !task_empty && (!self.race_mode || racing);

        let mode = |id: usize, title: &'static str, desc: &'static str, on: bool| {
            card(("iso", id), on)
                .child(icon(
                    if id == 0 {
                        IconName::GitBranchPlus
                    } else {
                        IconName::Folder
                    },
                    18.,
                    if on { c_blue() } else { c_secondary() },
                ))
                .child(
                    div()
                        .v_flex()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().text_xs().text_color(rgb(c_muted())).child(desc)),
                )
        };

        div()
            .flex_1()
            .min_w_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .max_w(px(780.))
                    .mx_auto()
                    .p_8()
                    .v_flex()
                    .gap_4()
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child("New session"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(c_secondary()))
                                    .child("Pick a project and an agent, then describe the task."),
                            ),
                    )
                    .child(label("PROJECT"))
                    .child(projects)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(label("AGENT"))
                            .when(is_git, |el| {
                                el.child(
                                    div()
                                        .id("race-toggle")
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(if self.race_mode {
                                            c_blue()
                                        } else {
                                            c_secondary()
                                        }))
                                        .hover(|el| el.bg(rgb(c_subtle())))
                                        .tooltip(|window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                "Run the same task on several agents, each in its own worktree, then keep the best result.",
                                            )
                                            .build(window, cx)
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.race_mode = !this.race_mode;
                                            if this.race_mode && this.new_race.is_empty() {
                                                this.new_race.push(this.new_provider);
                                            }
                                            cx.notify();
                                        }))
                                        .child(icon(
                                            IconName::Swords,
                                            13.,
                                            if self.race_mode { c_blue() } else { c_secondary() },
                                        ))
                                        .child(if self.race_mode {
                                            "Racing: pick two or more"
                                        } else {
                                            "Race several agents"
                                        }),
                                )
                            }),
                    )
                    .child(providers)
                    .child(label("WHERE IT RUNS"))
                    .child(if racing {
                        div()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child("Racing always uses isolated worktrees, one per agent.")
                            .into_any_element()
                    } else if is_git {
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                mode(0, "Isolated worktree", "Its own branch and folder", self.new_isolated)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.new_isolated = true;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                mode(1, "Project directory", "Your checkout, current branch", !self.new_isolated)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.new_isolated = false;
                                        cx.notify();
                                    })),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child("This folder is not a git repository, so the agent runs directly in it and diffs are unavailable.")
                            .into_any_element()
                    })
                    .when(isolated, |el| {
                        el.child(Input::new(&self.branch).bordered(true).h(px(36.)))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(c_secondary()))
                            .child(icon(IconName::Info, 13., c_muted()))
                            .child(div().min_w_0().truncate().child(where_to)),
                    )
                    .child(label("TASK"))
                    .child(Textarea::new(&self.new_task).bordered(true).h(px(140.)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                self.button(
                                    "start",
                                    if racing {
                                        format!("Race {} agents on {branch}", self.new_race.len())
                                    } else if isolated {
                                        format!("Start {} on {branch}", provider.name)
                                    } else {
                                        format!("Start {}", provider.name)
                                    },
                                    true,
                                )
                                .when(!ready, |el| el.opacity(0.5))
                                .child(icon(IconName::ArrowRight, 14., c_canvas()))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.create_session(window, cx);
                                    cx.notify();
                                })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .child(format!("{MOD_KEY}↵ to start")),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn list_view(&self, kind: View, cx: &mut Context<Self>) -> AnyElement {
        let worktrees_only = kind == View::Worktrees;
        let mut sessions: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| match kind {
                View::Inbox => s.unread && !s.archived,
                View::Worktrees => s.isolated,
                _ => true,
            })
            .collect();
        // Live sessions first, archived ones last.
        sessions.sort_by_key(|s| (s.archived, !s.pinned));
        let mut rows = div().v_flex().gap_2();
        let mut archived_header = false;
        for (i, session) in sessions.iter().enumerate() {
            let id = session.id;
            let provider = agent::provider(&session.provider);
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == session.project_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            if session.archived && !archived_header && kind == View::AllSessions {
                archived_header = true;
                rows = rows.child(
                    div()
                        .mt_4()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_secondary()))
                        .child("Archived"),
                );
            }
            let (archived, pinned, unread) = (session.archived, session.pinned, session.unread);
            rows = rows.child(stagger(
                div()
                    .id(("row", id as usize))
                    .group("list-row")
                    .p_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .when(archived, |el| el.opacity(0.7))
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(id);
                        cx.notify();
                    }))
                    .child(self.provider_badge(provider, 26.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .when(pinned, |el| {
                                        el.child(icon(IconName::Pin, 12., c_muted()))
                                    })
                                    .when(unread, |el| {
                                        el.child(
                                            div().size(px(7.)).rounded_full().bg(rgb(c_blue())),
                                        )
                                    })
                                    .child(
                                        div()
                                            .truncate()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(session.title.clone()),
                                    ),
                            )
                            .child(div().truncate().text_xs().text_color(rgb(c_muted())).child(
                                if worktrees_only && !self.demo {
                                    session.dir.clone()
                                } else {
                                    format!("{project}  ·  {}", session.branch)
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .opacity(0.)
                            .group_hover("list-row", |el| el.opacity(1.))
                            .when(!archived, |el| {
                                el.child(self.row_action(
                                    ("list-pin", id as usize),
                                    if pinned {
                                        IconName::PinOff
                                    } else {
                                        IconName::Pin
                                    },
                                    if pinned { "Unpin" } else { "Pin to top" },
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_pin(id);
                                        cx.notify();
                                    }),
                                ))
                            })
                            .child(self.row_action(
                                ("list-archive", id as usize),
                                if archived {
                                    IconName::ArchiveRestore
                                } else {
                                    IconName::Archive
                                },
                                if archived { "Restore" } else { "Archive" },
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_archive(id);
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(c_secondary()))
                            .child(
                                div()
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(rgb(Self::status_color(session.status))),
                            )
                            .child(Self::status_label(session.status)),
                    ),
                SharedString::from(format!("list-{}-{id}", kind as u8)),
                i,
            ));
        }
        if sessions.is_empty() {
            rows = rows.child(
                div()
                    .py_10()
                    .v_flex()
                    .items_center()
                    .gap_2()
                    .child(icon(
                        if kind == View::Inbox {
                            IconName::Inbox
                        } else {
                            IconName::Layers
                        },
                        28.,
                        c_muted(),
                    ))
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                        if kind == View::Inbox {
                            "You're all caught up"
                        } else {
                            "Nothing here yet"
                        },
                    ))
                    .child(div().text_xs().text_color(rgb(c_muted())).child(
                        if kind == View::Inbox {
                            "Sessions that finish while you're elsewhere land here."
                        } else {
                            "Start a session to see it here."
                        },
                    )),
            );
        }
        self.page(
            match kind {
                View::Inbox => "Inbox",
                View::Worktrees => "Worktrees",
                _ => "All sessions",
            },
            rows,
        )
    }

    // ---------- right pane ----------

    fn right_tab_button(
        &self,
        tab: RightTab,
        name: IconName,
        label: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.right_tab == tab;
        div()
            .id(SharedString::from(format!("rt-{label}")))
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .when(selected, |el| {
                el.bg(rgb(c_active())).text_color(rgb(c_text()))
            })
            .when(!selected, |el| {
                el.text_color(rgb(c_secondary()))
                    .hover(|el| el.text_color(rgb(c_text())))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.right_tab = tab;
                cx.notify();
            }))
            .child(icon(name, 13., if selected { c_text() } else { c_muted() }))
            .child(label)
    }

    fn diff_lines(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use git::LineKind;
        let path = self.selected_file.clone().unwrap_or_default();
        let notes: Vec<(usize, Note)> = self
            .current()
            .map(|s| {
                s.notes
                    .iter()
                    .cloned()
                    .enumerate()
                    .filter(|(_, n)| n.path == path)
                    .collect()
            })
            .unwrap_or_default();
        let mut col = div()
            .v_flex()
            .py_1()
            .font_family(self.mono_font.clone())
            .text_xs();
        for (i, row) in git::diff_rows(&self.diff)
            .into_iter()
            .take(1500)
            .enumerate()
        {
            if row.kind == LineKind::Hunk {
                col = col.child(
                    div()
                        .id(("dl", i))
                        .mt_1()
                        .px_3()
                        .py(px(3.))
                        .text_color(rgb(c_blue()))
                        .bg(rgb(c_selected()))
                        .whitespace_nowrap()
                        .child(row.text),
                );
                continue;
            }
            let (bg, mark, color) = match row.kind {
                LineKind::Added => (Some(c_add_bg()), "+", c_add()),
                LineKind::Removed => (Some(c_del_bg()), "−", c_del()),
                LineKind::Note => (None, "", c_muted()),
                _ => (None, " ", c_text()),
            };
            let line = row.number.unwrap_or(0);
            let can_note = row.kind != LineKind::Note && row.number.is_some();
            let code = format!("{mark}{}", row.text);
            let target = (path.clone(), line, code);
            let writing = self.note_at.as_ref() == Some(&target);
            col = col.child(
                div()
                    .id(("dl", i))
                    .group("diff-line")
                    .flex()
                    .min_h(px(19.))
                    .when_some(bg, |el, bg| el.bg(rgb(bg)))
                    .when(can_note, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(rgb(c_selected())))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.note_at = Some(target.clone());
                                this.note_input
                                    .update(cx, |input, cx| input.focus(window, cx));
                                cx.notify();
                            }))
                    })
                    .child(
                        div()
                            .w(px(44.))
                            .flex_none()
                            .px_2()
                            .text_right()
                            .text_color(rgb(c_muted()))
                            .child(row.number.map(|n| n.to_string()).unwrap_or_default()),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .text_color(rgb(color))
                            .child(mark),
                    )
                    .child(
                        div()
                            .flex_1()
                            .whitespace_nowrap()
                            .text_color(rgb(color))
                            .when(row.kind == LineKind::Note, |el| el.italic())
                            .child(row.text),
                    )
                    .when(can_note, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .px_2()
                                .opacity(0.)
                                .group_hover("diff-line", |el| el.opacity(1.))
                                .child(icon(IconName::MessageSquarePlus, 12., c_blue())),
                        )
                    }),
            );
            for (index, note) in notes
                .iter()
                .filter(|(_, n)| n.line == line && can_note && n.code.starts_with(mark))
            {
                let index = *index;
                col = col.child(
                    div()
                        .mx_2()
                        .my_1()
                        .ml(px(58.))
                        .px_3()
                        .py_2()
                        .flex()
                        .items_start()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .bg(rgb(c_canvas()))
                        .font_family(cx.theme().font_family.clone())
                        .child(icon(IconName::MessageSquareText, 13., c_blue()))
                        .child(div().flex_1().text_xs().child(note.text.clone()))
                        .child(
                            div()
                                .id(("note-x", index))
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(|el| el.bg(rgb(c_subtle())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_note(index);
                                    cx.notify();
                                }))
                                .child(icon(IconName::X, 12., c_muted())),
                        ),
                );
            }
            if writing {
                col = col.child(
                    div()
                        .ml(px(58.))
                        .mr_2()
                        .my_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .font_family(cx.theme().font_family.clone())
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.note_input).bordered(true).h(px(30.))),
                        )
                        .child(
                            div()
                                .id("note-cancel")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_xs()
                                .cursor_pointer()
                                .text_color(rgb(c_secondary()))
                                .hover(|el| el.bg(rgb(c_subtle())))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.note_at = None;
                                    cx.notify();
                                }))
                                .child("Cancel"),
                        ),
                );
            }
        }
        col
    }

    fn changes_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.current().is_none() {
            return div()
                .p_4()
                .text_sm()
                .text_color(rgb(c_muted()))
                .child("Select a session to see its changes.")
                .into_any_element();
        }
        if !self.dir_is_git {
            return div().p_4().v_flex().gap_2()
                .child(div().text_base().font_weight(FontWeight::BOLD).child("Changes"))
                .child(div().text_sm().text_color(rgb(c_secondary())).child("This folder is not a git repository, so diffs and isolated worktrees are unavailable. Run `git init` in the Terminal below to enable them."))
                .into_any_element();
        }
        let (added, removed) = self
            .changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        let mut list = div().v_flex();
        for (i, change) in self.changes.iter().enumerate() {
            let path = change.path.clone();
            let selected = self.selected_file.as_deref() == Some(change.path.as_str());
            let confirming = self.confirm_discard.as_deref() == Some(change.path.as_str());
            list = list.child(stagger(
                div()
                    .id(("chg", i))
                    .group("change-row")
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(rgb(c_active())))
                    .when(!selected, |el| el.hover(|el| el.bg(rgb(c_subtle()))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.demo {
                            this.selected_file = Some(path.clone());
                        } else if let Some(dir) = this.current().map(|s| s.dir.clone()) {
                            this.diff = git::file_diff(&dir, &path);
                            this.selected_file = Some(path.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(12.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(change_color(&change.status)))
                            .child(change.status.clone()),
                    )
                    .child({
                        let (dir, name) = match change.path.rsplit_once('/') {
                            Some((d, n)) => (d.to_string(), n.to_string()),
                            None => (String::new(), change.path.clone()),
                        };
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.5))
                            .child(div().flex_none().text_color(rgb(c_text())).child(name))
                            .child(div().truncate().text_color(rgb(c_muted())).child(dir))
                    })
                    .child(if confirming {
                        let (keep, gone) = (change.path.clone(), change.path.clone());
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id(("discard-yes", i))
                                    .px_2()
                                    .py(px(3.))
                                    .rounded_md()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(c_canvas()))
                                    .bg(rgb(c_red()))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.discard(&gone);
                                        cx.notify();
                                    }))
                                    .child("Discard"),
                            )
                            .child(
                                div()
                                    .id(("discard-no", i))
                                    .px_2()
                                    .py(px(3.))
                                    .rounded_md()
                                    .text_xs()
                                    .text_color(rgb(c_secondary()))
                                    .cursor_pointer()
                                    .hover(|el| el.bg(rgb(c_subtle())))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if this.confirm_discard.as_deref() == Some(keep.as_str()) {
                                            this.confirm_discard = None;
                                        }
                                        cx.notify();
                                    }))
                                    .child("Keep"),
                            )
                            .into_any_element()
                    } else {
                        let ask = change.path.clone();
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.5))
                            .child(
                                div()
                                    .text_color(rgb(c_add()))
                                    .child(format!("+{}", change.added)),
                            )
                            .child(
                                div()
                                    .text_color(rgb(c_del()))
                                    .child(format!("−{}", change.removed)),
                            )
                            .child(
                                div()
                                    .id(("discard", i))
                                    .size(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .opacity(0.)
                                    .group_hover("change-row", |el| el.opacity(1.))
                                    .hover(|el| el.bg(rgb(c_subtle())))
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            "Discard changes to this file",
                                        )
                                        .build(window, cx)
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.confirm_discard = Some(ask.clone());
                                        cx.notify();
                                    }))
                                    .child(icon(IconName::Undo2, 13., c_muted())),
                            )
                            .into_any_element()
                    }),
                SharedString::from(format!("chg-{}-{}", self.selected.unwrap_or(0), i)),
                i,
            ));
        }
        let selected = self.selected_file.clone().unwrap_or_default();
        let sel_stats = self
            .changes
            .iter()
            .find(|c| c.path == selected)
            .map(|c| (c.added, c.removed))
            .unwrap_or((0, 0));
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(div().p_3().child(self.ship_bar(cx)))
            .child(
                div()
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_muted()))
                    .child("CHANGES")
                    .child(format!("{}", self.changes.len()))
                    .when(!self.changes.is_empty(), |el| {
                        el.child(
                            div()
                                .text_color(rgb(c_add()))
                                .font_weight(FontWeight::NORMAL)
                                .child(format!("+{added}")),
                        )
                        .child(
                            div()
                                .text_color(rgb(c_del()))
                                .font_weight(FontWeight::NORMAL)
                                .child(format!("−{removed}")),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        self.ghost_button_sized(
                            "refresh-changes",
                            IconName::RefreshCw,
                            "Refresh. Changes are compared with the last commit.",
                            22.,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh();
                            cx.notify();
                        })),
                    ),
            )
            .child(if self.changes.is_empty() {
                div()
                    .flex_1()
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .py_10()
                    .child(icon(IconName::GitCompare, 28., c_muted()))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("No uncommitted changes"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(c_muted()))
                            .child("Files the agent edits show up here as it works."),
                    )
                    .into_any_element()
            } else {
                // Scroll areas take their full max height, so size it to the rows.
                div()
                    .flex_none()
                    .h(px((self.changes.len() as f32 * 28. + 4.).min(176.)))
                    .px_2()
                    .overflow_y_scrollbar()
                    .child(list)
                    .into_any_element()
            })
            .when(!self.changes.is_empty(), |el| {
                el.child(
                    div()
                        .mt_2()
                        .flex_1()
                        .min_h_0()
                        .v_flex()
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .bg(rgb(c_canvas()))
                        .child(
                            div()
                                .h(px(32.))
                                .pl_3()
                                .pr_1()
                                .flex()
                                .items_center()
                                .gap_2()
                                .border_b_1()
                                .border_color(rgb(c_border()))
                                .child(icon(IconName::File, 12., c_muted()))
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_size(px(12.))
                                        .font_family(self.mono_font.clone())
                                        .child(selected.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(rgb(c_add()))
                                        .child(format!("+{}", sel_stats.0)),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(rgb(c_del()))
                                        .child(format!("−{}", sel_stats.1)),
                                )
                                .child(
                                    self.ghost_button_sized(
                                        "open-file",
                                        IconName::ArrowUpRight,
                                        "Open file",
                                        24.,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            if let Some(path) = this.selected_file.clone() {
                                                this.open_preview(&path);
                                                this.right_tab = RightTab::Files;
                                            }
                                            cx.notify();
                                        },
                                    )),
                                ),
                        )
                        .child(fade(
                            div()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scrollbar()
                                .child(self.diff_lines(cx)),
                            SharedString::from(format!(
                                "diff-{}-{}",
                                self.selected.unwrap_or(0),
                                selected
                            )),
                            220,
                        )),
                )
            })
            .into_any_element()
    }

    /// Commit box and push / pull request actions for the session branch.
    fn ship_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_changes = !self.changes.is_empty();
        let sync = &self.sync;
        let busy = self
            .selected
            .is_some_and(|id| self.handles.contains_key(&Target::Terminal(id)));
        let status = if sync.remote.is_empty() {
            "No remote".to_string()
        } else if sync.ahead > 0 {
            format!(
                "{} unpushed commit{}",
                sync.ahead,
                if sync.ahead == 1 { "" } else { "s" }
            )
        } else if sync.upstream {
            format!("Up to date with {}", sync.remote)
        } else {
            "Not pushed yet".to_string()
        };
        let can_push = !sync.remote.is_empty() && !busy && (sync.ahead > 0 || !sync.upstream);
        let can_pr = !sync.remote.is_empty() && !busy;
        let notes = self.current().map(|s| s.notes.len()).unwrap_or(0);
        div()
            .v_flex()
            .gap_2()
            .when(notes > 0, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_lg()
                        .bg(rgb(c_selected()))
                        .child(icon(IconName::MessageSquareText, 14., c_blue()))
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(c_text()))
                                .child(format!(
                                    "{notes} review note{}",
                                    if notes == 1 { "" } else { "s" }
                                )),
                        )
                        .child(
                            self.button("clear-notes", "Clear", false)
                                .h(px(28.))
                                .text_xs()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.selected {
                                        if let Some(s) = this.state.session_mut(id) {
                                            s.notes.clear();
                                        }
                                        this.save();
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button("send-notes", "Send to agent", true)
                                .h(px(28.))
                                .text_xs()
                                .child(icon(IconName::ArrowUp, 13., c_canvas()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.send_notes(cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(has_changes, |el| {
                el.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(
                            div().text_size(px(12.5)).child(
                                Input::new(&self.commit_msg)
                                    .bordered(true)
                                    .bg(rgb(c_canvas()))
                                    .h(px(30.)),
                            ),
                        )
                        .child(
                            self.button("commit", "Commit", true)
                                .w_full()
                                .justify_center()
                                .h(px(28.))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Stage and commit every change. An empty message uses the session title.",
                                    )
                                    .build(window, cx)
                                })
                                .child(icon(IconName::GitCommitHorizontal, 14., c_canvas()))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.commit(window, cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::GitBranch, 12., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .child(status),
                    )
                    .child(
                        self.button("push", "Push", false)
                            .h(px(24.))
                            .px_2()
                            .text_size(px(12.))
                            .when(!can_push, |el| el.opacity(0.45))
                            .child(icon(IconName::CloudUpload, 13., c_secondary()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if can_push {
                                    this.ship(false, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("create-pr", "Create PR", false)
                            .h(px(24.))
                            .px_2()
                            .text_size(px(12.))
                            .when(!can_pr, |el| el.opacity(0.45))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Push the branch and open a pull request with the GitHub CLI (gh).",
                                )
                                .build(window, cx)
                            })
                            .child(icon(IconName::GitPullRequestCreate, 13., c_secondary()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if can_pr {
                                    this.ship(true, cx);
                                }
                                cx.notify();
                            })),
                    ),
            )
    }

    fn browser_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        div().p_4().v_flex().gap_3()
            .child(div().text_base().font_weight(FontWeight::BOLD).child("Browser"))
            .child(div().text_sm().text_color(rgb(c_secondary())).child("Embedded pages are not available in the native build yet. Open a URL, such as a worktree dev server, in your system browser."))
            .child(Input::new(&self.address).bordered(true).h(px(36.)))
            .child(div().flex().child(self.button("open-url", "Open in browser", true).on_click(cx.listener(|this, _, _, cx| {
                let raw = this.address.read(cx).value().trim().to_string();
                if raw.starts_with("http://") || raw.starts_with("https://") {
                    cx.open_url(&raw);
                } else if !raw.is_empty() {
                    this.notice = Some("Enter a full http:// or https:// URL.".into());
                }
                cx.notify();
            }))))
            .into_any_element()
    }

    fn files_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().v_flex();
        for (i, path) in self.files.iter().enumerate() {
            let p = path.clone();
            list = list.child(
                div()
                    .id(("file", i))
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_preview(&p);
                        cx.notify();
                    }))
                    .child(icon(IconName::File, 14., c_muted()))
                    .child(div().truncate().child(path.clone())),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(div().max_h(px(220.)).overflow_y_scrollbar().child(list))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .overflow_y_scrollbar()
                    .p_3()
                    .font_family(self.mono_font.clone())
                    .text_xs()
                    .child(match &self.preview {
                        Some((path, text)) => div()
                            .v_flex()
                            .gap_2()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(path.clone()))
                            .child(
                                TextView::markdown(
                                    SharedString::from(format!("preview-{path}")),
                                    format!("```\n{}\n```", text.replace("```", "'''")),
                                )
                                .selectable(true),
                            )
                            .into_any_element(),
                        None => div()
                            .text_color(rgb(c_muted()))
                            .child("Select a file to preview.")
                            .into_any_element(),
                    }),
            )
            .into_any_element()
    }

    fn terminal_tab(
        &self,
        label: &'static str,
        cli: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.cli_tab == cli;
        div()
            .id(label)
            .h(px(22.))
            .px_2()
            .flex()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .when(selected, |el| {
                el.bg(rgb(c_active())).text_color(rgb(c_text()))
            })
            .when(!selected, |el| {
                el.text_color(rgb(c_muted()))
                    .hover(|el| el.text_color(rgb(c_text())))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.cli_tab = cli;
                cx.notify();
            }))
            .child(label)
    }

    fn terminal_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.selected.unwrap_or(0);
        let running = self.handles.contains_key(&Target::Terminal(id));
        let has_output = self.terminal.get(&id).is_some_and(|l| !l.is_empty());
        let mut log = div()
            .v_flex()
            .font_family(self.mono_font.clone())
            .text_xs()
            .p_3()
            .gap(px(2.));
        if self.cli_tab {
            for p in agent::PROVIDERS.iter() {
                let ok = agent::available(p.bin, &cx.global::<AppSettings>().agent(p.id).binary);
                let version = caps::get(p.id).version;
                log = log.child(
                    div()
                        .flex()
                        .gap_3()
                        .child(div().w(px(90.)).child(p.bin))
                        .child(
                            div()
                                .w(px(90.))
                                .text_color(rgb(if ok { c_green() } else { c_red() }))
                                .child(if ok { "installed" } else { "not found" }),
                        )
                        .child(div().text_color(rgb(c_muted())).child(version)),
                );
            }
        } else if !has_output {
            let dir = self
                .current()
                .map(|s| s.dir.clone())
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "the session folder".into());
            log = log.child(
                div()
                    .text_color(rgb(c_muted()))
                    .child(format!("Commands run in {dir}. Output streams here.")),
            );
        } else if let Some(lines) = self.terminal.get(&id) {
            let text: Vec<&str> = lines
                .iter()
                .rev()
                .take(300)
                .rev()
                .map(String::as_str)
                .collect();
            log = log.child(
                TextView::markdown(
                    SharedString::from(format!("term-{id}")),
                    format!("```text\n{}\n```", text.join("\n").replace("```", "'''")),
                )
                .selectable(true),
            );
        }
        div()
            .h(px(230.))
            .flex_none()
            .v_flex()
            .border_t_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_term_bg()))
            .child(
                div()
                    .h(px(32.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.terminal_tab("Terminal", false, cx))
                            .child(self.terminal_tab("CLI", true, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(c_secondary()))
                            .when(running, |el| {
                                el.child(self.status_dot(Status::Working, "term-running", 6.))
                                    .child("Running")
                                    .child(
                                        self.button("term-stop", "Stop", false)
                                            .h(px(26.))
                                            .px_2()
                                            .text_xs()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(h) =
                                                    this.handles.get(&Target::Terminal(id))
                                                {
                                                    h.kill();
                                                }
                                                cx.notify();
                                            })),
                                    )
                            })
                            .when(!running && !self.cli_tab && has_output, |el| {
                                el.child(
                                    self.ghost_button_sized(
                                        "term-clear",
                                        IconName::Eraser,
                                        "Clear output",
                                        22.,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.terminal.remove(&id);
                                            cx.notify();
                                        },
                                    )),
                                )
                            }),
                    ),
            )
            .child(fade(
                div().flex_1().min_h_0().overflow_y_scrollbar().child(log),
                SharedString::from(format!("termbody-{}-{}", self.cli_tab, id)),
                220,
            ))
            .when(!self.cli_tab, |el| {
                el.child(
                    div()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .child(
                            div()
                                .font_family(self.mono_font.clone())
                                .text_size(px(12.))
                                .text_color(rgb(c_green()))
                                .child("❯"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.5))
                                .child(Input::new(&self.term_input).appearance(false).h(px(30.))),
                        ),
                )
            })
    }

    fn right_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(460.))
            .flex_none()
            .h_full()
            .v_flex()
            .border_l_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_sidebar()))
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
                    .child(self.right_tab_button(
                        RightTab::Changes,
                        IconName::GitCompare,
                        if self.changes.is_empty() {
                            "Changes".into()
                        } else {
                            format!("Changes  {}", self.changes.len())
                        },
                        cx,
                    ))
                    .child(self.right_tab_button(
                        RightTab::Browser,
                        IconName::Globe,
                        "Browser".into(),
                        cx,
                    ))
                    .child(self.right_tab_button(
                        RightTab::Files,
                        IconName::FileText,
                        "Files".into(),
                        cx,
                    )),
            )
            .child(fade(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(match self.right_tab {
                        RightTab::Changes => self.changes_tab(cx),
                        RightTab::Browser => self.browser_tab(cx),
                        RightTab::Files => self.files_tab(cx),
                    }),
                SharedString::from(format!(
                    "rtab-{}-{}",
                    self.right_tab as u8,
                    self.selected.unwrap_or(0)
                )),
                260,
            ))
            .child(self.terminal_panel(cx))
    }
}

impl Render for RunoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let st = cx.global::<AppSettings>().clone();
        let dark = match st.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        set_dark(dark);
        if self.applied_dark != Some(dark) {
            self.applied_dark = Some(dark);
            gpui_kit::component::Theme::change(
                if dark {
                    gpui_kit::component::ThemeMode::Dark
                } else {
                    gpui_kit::component::ThemeMode::Light
                },
                Some(window),
                cx,
            );
            // A theme switch reloads the theme file's fonts; keep the bundled UI font.
            if cx.theme().font_family.as_ref() != UI_FONT {
                gpui_kit::component::Theme::update(cx, |theme| theme.font_family = UI_FONT.into());
            }
        }
        if self.applied_enter != Some(st.send_on_enter) {
            self.applied_enter = Some(st.send_on_enter);
            self.prompt
                .update(cx, |p, cx| p.set_submit_on_enter(st.send_on_enter, cx));
        }
        let center = match self.view {
            View::Session => self.session_view(cx),
            View::NewSession => self.new_session_view(cx),
            View::Inbox => self.list_view(View::Inbox, cx),
            View::AllSessions => self.list_view(View::AllSessions, cx),
            View::History => self.history_view(cx),
            View::Worktrees => self.list_view(View::Worktrees, cx),
            View::Settings => self.settings_view(window, cx),
        };
        // The side panel needs room; on narrow windows it starts hidden (toggle in the header).
        let wide = window.viewport_size().width >= px(1180.);
        if self.panel_wide != Some(wide) {
            self.panel_wide = Some(wide);
            self.show_panel = wide;
        }
        let show_right = self.view == View::Session && self.current().is_some() && self.show_panel;
        if self
            .info
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(3))
        {
            self.info = None;
        }
        let (status, status_color, status_icon) = if let Some(notice) = &self.notice {
            (notice.clone(), c_red(), Some(IconName::CircleAlert))
        } else if let Some((info, _)) = &self.info {
            (info.clone(), c_blue(), Some(IconName::Check))
        } else if self.demo {
            (
                "Sample workspace. Add a project folder to start a real session.".to_string(),
                c_muted(),
                Some(IconName::Info),
            )
        } else {
            (String::new(), c_muted(), None)
        };
        let running = self.started.len();
        let summary = match self.state.sessions.len() {
            0 => String::new(),
            n => format!("{n} session{}", if n == 1 { "" } else { "s" }),
        };
        let palette = self
            .palette
            .clone()
            .map(|state| self.palette_view(state, cx));
        let root = div()
            .size_full()
            .relative()
            .v_flex()
            .key_context("Runo")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &QuickOpen, window, cx| {
                this.open_palette(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewSession, _, cx| {
                this.view = View::NewSession;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.open_settings();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &StopAgent, _, cx| {
                this.stop();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePanel, _, cx| {
                this.show_panel = !this.show_panel;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.show_sidebar = !this.show_sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                if this.palette.is_some() {
                    this.close_palette(window, cx);
                } else if this.menu.is_some() || this.settings_menu.is_some() {
                    this.menu = None;
                    this.settings_menu = None;
                } else if this.renaming {
                    this.renaming = false;
                } else if this.note_at.is_some() {
                    this.note_at = None;
                } else if this.confirm_delete.is_some() {
                    this.confirm_delete = None;
                } else if this.notice.is_some() {
                    this.notice = None;
                } else if this.view != View::Session {
                    this.view = View::Session;
                }
                cx.notify();
            }))
            .bg(rgb(c_canvas()))
            .text_color(rgb(c_text()))
            .text_size(px(13.))
            .font_family(cx.theme().font_family.clone())
            .child(self.title_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(self.view != View::Settings && self.show_sidebar, |el| {
                        el.child(self.sidebar(cx))
                    })
                    .child(div().flex_1().min_w_0().flex().child(center).animated(
                        SharedString::from(format!(
                            "center-{}-{}",
                            self.view as u8,
                            self.selected.unwrap_or(0)
                        )),
                        Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                        |el, delta| el.opacity(0.35 + 0.65 * delta),
                    ))
                    .when(show_right, |el| el.child(self.right_pane(cx))),
            )
            .child(
                div()
                    .id("status")
                    .h(px(24.))
                    .flex_none()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_sidebar()))
                    .text_size(px(11.5))
                    .gap_4()
                    .child(fade(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .text_color(rgb(status_color))
                            .when_some(status_icon, |el, i| el.child(icon(i, 12., status_color)))
                            .child(div().truncate().child(status.clone()))
                            .when(self.notice.is_some(), |el| {
                                el.child(
                                    div()
                                        .id("dismiss-notice")
                                        .px_1()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .hover(|el| el.bg(rgb(c_subtle())))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.notice = None;
                                            cx.notify();
                                        }))
                                        .child(icon(IconName::X, 11., c_muted())),
                                )
                            }),
                        SharedString::from(format!("status-{status}")),
                        300,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_3()
                            .text_color(rgb(c_muted()))
                            .when(running > 0, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .text_color(rgb(c_secondary()))
                                        .child(self.status_dot(Status::Working, "sb-run", 6.))
                                        .child(format!("{running} running")),
                                )
                            })
                            .when_some(
                                self.current()
                                    .filter(|_| self.view == View::Session)
                                    .map(|s| s.branch.clone())
                                    .filter(|b| !b.is_empty()),
                                |el, branch| {
                                    el.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(icon(IconName::GitBranch, 11., c_muted()))
                                            .child(branch),
                                    )
                                },
                            )
                            .when(!summary.is_empty(), |el| el.child(summary))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(div().size(px(6.)).rounded_full().bg(rgb(c_green())))
                                    .child("Local"),
                            ),
                    ),
            )
            .children(palette);
        fade(root, "app-root", 450)
    }
}

/// The UI typeface, bundled so the app looks the same on every platform.
const UI_FONT: &str = "Inter";

fn load_fonts(cx: &mut App) {
    let fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.otf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.otf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.otf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.otf")),
    ];
    if cx.text_system().add_fonts(fonts).is_ok() {
        gpui_kit::component::Theme::update(cx, |theme| theme.font_family = UI_FONT.into());
    }
}

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        init(cx);
        load_fonts(cx);
        bind_keys(cx);
        let settings = AppSettings::load();
        settings.apply();
        cx.set_global(settings);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1560.), px(960.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Runo".into()),
                appears_transparent: cfg!(target_os = "macos"),
                traffic_light_position: Some(point(px(16.), px(16.))),
            }),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            cx.new(|cx| RunoApp::new(window, cx))
        })
        .expect("failed to open Runo window");
    });
}
