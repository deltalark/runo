mod agent;
mod caps;
mod git;
mod settings;
mod settings_ui;
mod store;
mod stream;

use agent::{Event, Handle, Target};
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
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
use store::{Message, Project, Role, Session, State, Status};

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
    [NewSession, OpenSettings, StopAgent, TogglePanel, Dismiss]
);

/// Global shortcuts. `secondary` is ⌘ on macOS and Ctrl elsewhere.
fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewSession, Some("Runo")),
        KeyBinding::new("secondary-,", OpenSettings, Some("Runo")),
        KeyBinding::new("secondary-.", StopAgent, Some("Runo")),
        KeyBinding::new("secondary-b", TogglePanel, Some("Runo")),
        KeyBinding::new("escape", Dismiss, Some("Runo")),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum View {
    Session,
    NewSession,
    AllSessions,
    Worktrees,
    Settings,
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
    expanded: Vec<u64>,
    confirm_delete: Option<u64>,
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
                    this.handle_event(event);
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

        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions…"));
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
            confirm_delete: None,
            notice: None,
            info: None,
            mono_font: cx.theme().mono_font_family.clone(),
            show_panel: true,
            focus: cx.focus_handle(),
            panel_wide: None,
            started: HashMap::new(),
            stopping: Default::default(),
            _subscriptions: vec![search_sub, term_sub, prompt_sub, settings_search_sub],
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
        self.files = git::tracked_files(&dir);
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

    fn select(&mut self, id: u64) {
        self.selected = Some(id);
        self.view = View::Session;
        self.selected_file = None;
        self.diff.clear();
        self.preview = None;
        self.confirm_delete = None;
        self.notice = None;
        self.info = None;
        self.menu = None;
        self.refresh();
        self.chat_scroll.scroll_to_bottom();
    }

    fn handle_event(&mut self, event: Event) {
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
                if let Some(session) = self.state.session_mut(id) {
                    session.status = if ok { Status::Done } else { Status::Failed };
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
        let id = self.state.next_id();
        let typed = git::sanitize_branch(&self.branch.read(cx).value());
        let is_git = git::is_repo(&project.path);
        let isolated = self.new_isolated && is_git;
        let (branch, dir) = if !is_git {
            ("local folder".to_string(), project.path.clone())
        } else if isolated {
            let branch = if typed.is_empty() {
                format!("runo/session-{id}")
            } else {
                typed
            };
            match git::add_worktree(&project.path, &branch) {
                Ok(dir) => (branch, dir),
                Err(error) => {
                    self.notice = Some(format!("Could not create worktree: {error}"));
                    return;
                }
            }
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
            provider: agent::PROVIDERS[self.new_provider].id.into(),
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
        });
        self.new_task
            .update(cx, |t, cx| t.set_value("", window, cx));
        self.branch.update(cx, |b, cx| b.set_value("", window, cx));
        if !self.expanded.contains(&project.id) {
            self.expanded.push(project.id);
        }
        self.select(id);
        let opts = cx
            .global::<AppSettings>()
            .agent(agent::PROVIDERS[self.new_provider].id);
        self.start_agent(id, task, &opts);
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
            .h(px(34.))
            .px_3()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded_lg()
            .cursor_pointer()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .when(primary, |el| {
                el.bg(rgb(c_text())).text_color(rgb(c_canvas()))
            })
            .when(!primary, |el| {
                el.border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_canvas()))
                    .text_color(rgb(c_text()))
                    .hover(|el| el.bg(rgb(c_subtle())))
            })
            .child(label.into())
    }

    fn icon_button(&self, id: &'static str, name: IconName, tip: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_canvas()))
            .cursor_pointer()
            .hover(|el| el.bg(rgb(c_subtle())))
            .child(icon(name, 16., c_text()))
    }

    fn chip(&self, id: (&'static str, usize), label: String, selected: bool) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .rounded_lg()
            .cursor_pointer()
            .text_sm()
            .border_1()
            .when(selected, |el| {
                el.border_color(rgb(c_blue()))
                    .bg(rgb(c_selected()))
                    .text_color(rgb(c_blue()))
            })
            .when(!selected, |el| {
                el.border_color(rgb(c_border()))
                    .text_color(rgb(c_secondary()))
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

    fn provider_badge(&self, provider: &agent::Provider, size: f32) -> Div {
        div()
            .size(px(size))
            .flex_none()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(settings::mix(
                provider.color,
                c_canvas(),
                if is_dark() { 0.22 } else { 0.12 },
            )))
            .text_color(rgb(provider.color))
            .font_weight(FontWeight::BOLD)
            .child(provider.mark)
    }

    // ---------- title bar ----------

    fn title_bar(&self) -> impl IntoElement {
        let project = self
            .current()
            .and_then(|s| self.state.projects.iter().find(|p| p.id == s.project_id))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "No project".into());
        let branch = self.current().map(|s| s.branch.clone()).unwrap_or_default();
        let running = self.started.len();
        div()
            .h(px(46.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(if cfg!(target_os = "macos") { 84. } else { 16. }))
            .pr_4()
            .border_b_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_canvas()))
            .window_control_area(WindowControlArea::Drag)
            .child(div().w(px(280.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_sm()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Runo"))
                    .child(div().h(px(14.)).w(px(1.)).bg(rgb(c_border())))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(rgb(c_secondary()))
                            .child(project)
                            .when(!branch.is_empty(), |el| {
                                el.child(div().text_color(rgb(c_muted())).child("/"))
                                    .child(icon(IconName::GitBranch, 13., c_muted()))
                                    .child(branch)
                            }),
                    )
                    .when(running > 0, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .py(px(2.))
                                .rounded_full()
                                .bg(rgb(c_selected()))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(c_blue()))
                                .child(self.status_dot(Status::Working, "title-running", 6.))
                                .child(if running == 1 {
                                    "1 agent running".to_string()
                                } else {
                                    format!("{running} agents running")
                                }),
                        )
                    }),
            )
            .child(if self.view == View::Settings {
                div().w(px(280.)).into_any_element()
            } else {
                div()
                    .w(px(280.))
                    .h(px(32.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_sidebar()))
                    .child(icon(IconName::Search, 14., c_muted()))
                    .child(Input::new(&self.search).appearance(false).h(px(28.)))
                    .into_any_element()
            })
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
            .h(px(36.))
            .px_3()
            .flex()
            .items_center()
            .gap_3()
            .rounded_lg()
            .cursor_pointer()
            .text_sm()
            .text_color(rgb(if selected { c_blue() } else { c_text() }))
            .when(selected, |el| el.bg(rgb(c_selected())))
            .hover(|el| el.bg(rgb(c_subtle())))
            .on_click(cx.listener(move |this, _, _, cx| {
                if view == View::Settings {
                    this.open_settings();
                } else {
                    this.view = view;
                }
                cx.notify();
            }))
            .child(icon(
                name,
                16.,
                if selected { c_blue() } else { c_secondary() },
            ))
            .child(div().flex_1().child(label))
            .when(count > 0, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(rgb(c_muted()))
                        .child(count.to_string()),
                )
            })
    }

    fn session_row(&self, session: &Session, cx: &mut Context<Self>) -> impl IntoElement {
        let id = session.id;
        let provider = agent::provider(&session.provider);
        let selected = self.selected == Some(id) && self.view == View::Session;
        div()
            .id(("session", id as usize))
            .ml_3()
            .mr_1()
            .px_2()
            .py(px(7.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .cursor_pointer()
            .when(selected, |el| el.bg(rgb(c_selected())))
            .hover(|el| el.bg(rgb(c_subtle())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(id);
                cx.notify();
            }))
            .child(self.provider_badge(provider, 24.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_text()))
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(rgb(c_muted()))
                            .child(div().min_w_0().truncate().child(session.branch.clone()))
                            .when(session.status == Status::Working, |el| {
                                el.child(div().flex_none().text_color(rgb(c_blue())).child(
                                    match self.started.get(&id) {
                                        Some(t) => format!("· {}", elapsed(t.elapsed())),
                                        None => "· Working".into(),
                                    },
                                ))
                            })
                            .when(session.status == Status::Failed, |el| {
                                el.child(
                                    div().flex_none().text_color(rgb(c_red())).child("· Failed"),
                                )
                            }),
                    ),
            )
            .child(self.status_dot(session.status, ("dot", id as usize), 8.))
            .animated(
                SharedString::from(format!("row-in-{id}")),
                Animation::new(Duration::from_millis(300)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.search.read(cx).value().to_lowercase();
        let mut projects = div().v_flex().gap_1();
        for project in &self.state.projects {
            let pid = project.id;
            let open = self.expanded.contains(&pid);
            let sessions: Vec<&Session> = self
                .state
                .sessions
                .iter()
                .filter(|s| s.project_id == pid)
                .filter(|s| {
                    filter.is_empty()
                        || s.title.to_lowercase().contains(&filter)
                        || s.branch.to_lowercase().contains(&filter)
                        || project.name.to_lowercase().contains(&filter)
                })
                .collect();
            if !filter.is_empty() && sessions.is_empty() {
                continue;
            }
            projects = projects.child(
                div()
                    .id(("project", pid as usize))
                    .h(px(32.))
                    .px_2()
                    .mt_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
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
                            IconName::FolderOpen
                        } else {
                            IconName::Folder
                        },
                        16.,
                        c_secondary(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child(project.name.clone()),
                    )
                    .when(!open && !sessions.is_empty(), |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(c_muted()))
                                .child(sessions.len().to_string()),
                        )
                    })
                    .child(
                        div()
                            .id(("project-new", pid as usize))
                            .p(px(3.))
                            .rounded_md()
                            .cursor_pointer()
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
                            .child(icon(IconName::Plus, 14., c_muted())),
                    )
                    .child(icon(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        14.,
                        c_muted(),
                    )),
            );
            if open || !filter.is_empty() {
                if sessions.is_empty() {
                    projects = projects.child(
                        div()
                            .ml(px(38.))
                            .py_1()
                            .text_xs()
                            .text_color(rgb(c_muted()))
                            .child("No sessions yet"),
                    );
                }
                for session in sessions {
                    projects = projects.child(self.session_row(session, cx));
                }
            }
        }
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "You".into());
        div()
            .w(px(280.))
            .flex_none()
            .h_full()
            .v_flex()
            .p_3()
            .gap_1()
            .border_r_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_sidebar()))
            .child(
                div()
                    .h(px(52.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(36.))
                            .rounded_lg()
                            .bg(rgb(c_text()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(c_canvas()))
                            .font_weight(FontWeight::BOLD)
                            .child("R"),
                    )
                    .child(
                        div()
                            .v_flex()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Runo"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .child("Personal workspace"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("new-session")
                    .h(px(40.))
                    .px_3()
                    .my_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_canvas()))
                    .cursor_pointer()
                    .text_sm()
                    .hover(|el| el.bg(rgb(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.view = View::NewSession;
                        cx.notify();
                    }))
                    .child(icon(IconName::Plus, 16., c_text()))
                    .child(div().flex_1().child("New session")),
            )
            .child(self.nav_item(
                "all-sessions",
                IconName::Inbox,
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
            .child(
                div()
                    .mt_3()
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_secondary()))
                            .child("Projects"),
                    )
                    .child(
                        div()
                            .id("add-project")
                            .p_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|el| el.bg(rgb(c_subtle())))
                            .on_click(cx.listener(|this, _, _, cx| this.pick_project(cx)))
                            .child(icon(IconName::FolderPlus, 16., c_secondary())),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(projects),
            )
            .child(self.nav_item(
                "settings",
                IconName::Settings,
                "Settings",
                0,
                View::Settings,
                cx,
            ))
            .child(
                div()
                    .mt_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(32.))
                            .rounded_full()
                            .bg(rgb(c_blue()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(c_canvas()))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                user.chars()
                                    .next()
                                    .unwrap_or('U')
                                    .to_uppercase()
                                    .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(user),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .child("Local workspace"),
                            ),
                    )
                    .child(div().size(px(8.)).rounded_full().bg(rgb(c_green()))),
            )
    }

    // ---------- center ----------

    fn step_row(&self, text: &str) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_3()
            .h(px(30.))
            .text_sm()
            .child(icon(IconName::ChevronRight, 14., c_muted()))
            .child(icon(IconName::CircleCheck, 18., c_green()))
            .child(div().text_color(rgb(c_text())).child(text.to_string()))
    }

    fn changes_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (added, removed) = self
            .changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        let names: Vec<String> = self
            .changes
            .iter()
            .take(3)
            .map(|c| c.path.rsplit('/').next().unwrap_or(&c.path).to_string())
            .collect();
        let more = self.changes.len().saturating_sub(3);
        div()
            .id("changes-card")
            .mt_2()
            .max_w(px(640.))
            .px_4()
            .py_3()
            .flex()
            .items_center()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .cursor_pointer()
            .hover(|el| el.bg(rgb(c_subtle())))
            .on_click(cx.listener(|this, _, _, cx| {
                this.right_tab = RightTab::Changes;
                cx.notify();
            }))
            .child(icon(IconName::FileText, 20., c_secondary()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!(
                                "{} file{} changed",
                                self.changes.len(),
                                if self.changes.len() == 1 { "" } else { "s" }
                            )),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(c_muted()))
                            .child(format!(
                                "{}{}",
                                names.join(", "),
                                if more > 0 {
                                    format!("  +{more} more")
                                } else {
                                    String::new()
                                }
                            )),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(c_add()))
                    .child(format!("+{added}")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(c_del()))
                    .child(format!("−{removed}")),
            )
            .child(icon(IconName::ChevronRight, 16., c_muted()))
            .animated(
                SharedString::from(format!(
                    "card-{}-{}",
                    self.selected.unwrap_or(0),
                    self.changes.len()
                )),
                Animation::new(Duration::from_millis(400)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d).mt(px(8.0 + (1.0 - d) * 10.0)),
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
        let is_user = message.role == Role::User;
        let (name, color) = match message.role {
            Role::User => ("You", c_muted()),
            Role::Agent => (provider.name, provider.color),
            Role::System => ("Runo", c_red()),
        };
        let body = if message.text.is_empty() {
            "Working…".to_string()
        } else {
            message.text.clone()
        };
        let mut steps = div().v_flex().my_1();
        for (i, step) in message.steps.iter().enumerate() {
            steps = steps.child(stagger(
                div().child(self.step_row(step)),
                SharedString::from(format!(
                    "step-{}-{}-{}",
                    self.selected.unwrap_or(0),
                    index,
                    i
                )),
                i,
            ));
        }
        let is_system = message.role == Role::System;
        let copy_text = message.text.clone();
        let avatar = match message.role {
            Role::Agent => self
                .provider_badge(provider, 32.)
                .rounded_full()
                .into_any_element(),
            Role::System => div()
                .size(px(32.))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(settings::mix(c_red(), c_canvas(), 0.14)))
                .child(icon(IconName::TriangleAlert, 15., c_red()))
                .into_any_element(),
            Role::User => div()
                .size(px(32.))
                .flex_none()
                .rounded_full()
                .bg(rgb(c_border()))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(color))
                .child("You")
                .into_any_element(),
        };
        div()
            .id(("message", index))
            .group("message")
            .flex()
            .gap_3()
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .when(!message.time.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(c_muted()))
                                        .child(message.time.clone()),
                                )
                            })
                            .when(!message.text.is_empty(), |el| {
                                el.child(
                                    div()
                                        .id(("copy", index))
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_2()
                                        .py(px(2.))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(c_muted()))
                                        .opacity(0.)
                                        .group_hover("message", |el| el.opacity(1.))
                                        .hover(|el| {
                                            el.bg(rgb(c_subtle())).text_color(rgb(c_text()))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy_text.clone(),
                                            ));
                                            this.info = Some((
                                                "Copied to the clipboard.".into(),
                                                std::time::Instant::now(),
                                            ));
                                            cx.notify();
                                        }))
                                        .child(icon(IconName::Copy, 12., c_muted()))
                                        .child("Copy"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(760.))
                            .whitespace_normal()
                            .text_sm()
                            .when(is_user, |el| {
                                el.px_4().py_3().rounded_xl().bg(rgb(c_subtle()))
                            })
                            .when(is_system, |el| {
                                el.px_4()
                                    .py_3()
                                    .rounded_xl()
                                    .border_1()
                                    .border_color(rgb(settings::mix(c_red(), c_canvas(), 0.35)))
                                    .bg(rgb(settings::mix(c_red(), c_canvas(), 0.06)))
                            })
                            .text_color(rgb(c_text()))
                            .child(if message.text.is_empty() {
                                working_indicator().into_any_element()
                            } else {
                                TextView::markdown(
                                    SharedString::from(format!(
                                        "body-{}-{}",
                                        self.selected.unwrap_or(0),
                                        index
                                    )),
                                    body,
                                )
                                .selectable(true)
                                .into_any_element()
                            }),
                    )
                    .when(!message.steps.is_empty(), |el| el.child(steps))
                    .when(!message.meta.is_empty(), |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(c_muted()))
                                .child(message.meta.clone()),
                        )
                    })
                    .when(show_changes && !working, |el| {
                        el.child(self.changes_card(cx))
                    }),
            )
            .animated(
                SharedString::from(format!("msg-{}-{}", self.selected.unwrap_or(0), index)),
                Animation::new(Duration::from_millis(280)).with_easing(ease_out_quint()),
                |el, delta| el.opacity(delta).mt(px((1.0 - delta) * 8.0)),
            )
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
        let mut list = div().v_flex().gap_5().px_8().py_5();
        if let Some(first) = session.messages.first().filter(|m| !m.time.is_empty()) {
            list = list.child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_xs()
                    .text_color(rgb(c_muted()))
                    .child(format!("Started at {}", first.time)),
            );
        }
        for (i, message) in session.messages.iter().enumerate() {
            let show = Some(i) == last_agent && !self.changes.is_empty();
            list = list.child(self.message_view(i, message, provider, show, working, cx));
        }
        div()
            .flex_1()
            .min_w_0()
            .v_flex()
            .child(
                div()
                    .px_8()
                    .pt_5()
                    .pb_3()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child(session.title.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_sm()
                                    .text_color(rgb(c_secondary()))
                                    .child(if working {
                                        self.provider_badge(provider, 18.)
                                            .animated(
                                                "hdr-badge",
                                                Animation::new(Duration::from_millis(1100))
                                                    .repeat()
                                                    .with_easing(pulsating_between(0.35, 1.0)),
                                                |el, d| el.opacity(d),
                                            )
                                            .into_any_element()
                                    } else {
                                        self.provider_badge(provider, 18.).into_any_element()
                                    })
                                    .child(provider.name)
                                    .child(div().h(px(14.)).w(px(1.)).bg(rgb(c_border())))
                                    .child(icon(IconName::GitBranch, 14., c_muted()))
                                    .child(session.branch.clone())
                                    .child(self.status_dot(session.status, "hdr-dot", 7.))
                                    .child(match self.started.get(&id).filter(|_| working) {
                                        Some(start) => {
                                            format!("Working · {}", elapsed(start.elapsed()))
                                        }
                                        None => Self::status_label(session.status).to_string(),
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .when(working, |el| {
                                el.child(
                                    self.button("stop", "Stop", false)
                                        .h(px(36.))
                                        .text_color(rgb(c_red()))
                                        .child(icon(IconName::CircleStop, 15., c_red()))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.stop();
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                self.icon_button(
                                    "toggle-panel",
                                    if self.show_panel {
                                        IconName::PanelRightClose
                                    } else {
                                        IconName::PanelRightOpen
                                    },
                                    if self.show_panel {
                                        "Hide changes panel"
                                    } else {
                                        "Show changes panel"
                                    },
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.show_panel = !this.show_panel;
                                        cx.notify();
                                    },
                                )),
                            )
                            .when(confirming, |el| {
                                el.child(
                                    self.button("cancel-delete", "Cancel", false)
                                        .h(px(36.))
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
                                    .h(px(36.))
                                    .border_color(rgb(c_red()))
                                    .text_color(rgb(c_red()))
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.delete_session(id);
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .when(!confirming, |el| {
                                el.child(
                                    self.icon_button("delete", IconName::Trash, "Delete session")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if confirm_delete() {
                                                this.confirm_delete = Some(id);
                                            } else {
                                                this.delete_session(id);
                                            }
                                            cx.notify();
                                        })),
                                )
                            }),
                    ),
            )
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
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .cursor_pointer()
            .text_sm()
            .text_color(rgb(c_text()))
            .border_color(rgb(if open { c_blue() } else { c_border() }))
            .when(open, |el| el.bg(rgb(c_selected())))
            .hover(|el| el.bg(rgb(c_subtle())))
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
                el.child(div().text_xs().text_color(rgb(c_muted())).child(caption))
            })
            .child(label)
            .child(icon(
                if open {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                },
                12.,
                c_muted(),
            ))
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
                    .min_h(px(32.))
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .when(selected, |el| {
                        el.bg(rgb(c_selected())).text_color(rgb(c_blue()))
                    })
                    .hover(|el| el.bg(rgb(c_subtle())))
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
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .max_w(px(280.))
                                    .whitespace_normal()
                                    .child(desc),
                            )
                        },
                    ))
                    .when(selected, |el| {
                        el.child(icon(IconName::Check, 14., c_blue()))
                    }),
            );
        }
        fade(
            div()
                .absolute()
                .bottom(px(40.))
                .left_0()
                .min_w(px(210.))
                .max_h(px(340.))
                .rounded_xl()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(rgb(c_canvas()))
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

        let mut chips = div().flex().flex_wrap().items_center().gap_2().child(
            div()
                .relative()
                .child(self.menu_button(
                    SharedString::from("chip-provider"),
                    "provider",
                    Some(self.provider_badge(provider, 16.).into_any_element()),
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

        div()
            .px_8()
            .pb_2()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .px_4()
                    .py_3()
                    .v_flex()
                    .gap_3()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_canvas()))
                    .child(
                        Textarea::new(&self.prompt)
                            .appearance(false)
                            .bordered(false)
                            .h(px(56.)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .gap_3()
                            .child(div().flex_1().min_w_0().child(chips))
                            .child(
                                div()
                                    .id("send")
                                    .size(px(38.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_lg()
                                    .cursor_pointer()
                                    .bg(rgb(if working || empty_prompt {
                                        c_muted()
                                    } else {
                                        c_text()
                                    }))
                                    .when(!working && !empty_prompt, |el| {
                                        el.hover(|el| el.bg(rgb(c_blue())))
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_message(window, cx);
                                        cx.notify();
                                    }))
                                    .child(if working {
                                        icon(IconName::Loader, 18., c_canvas()).animated(
                                            "spin",
                                            Animation::new(Duration::from_millis(900)).repeat(),
                                            |i, d| i.rotate(percentage(d)),
                                        )
                                    } else {
                                        icon(IconName::ArrowUp, 18., c_canvas()).into_any_element()
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
                    .text_xs()
                    .text_color(rgb(c_muted()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon(IconName::GitBranch, 12., c_muted()))
                            .child(if isolated {
                                "Isolated worktree"
                            } else {
                                "Project directory"
                            }),
                    )
                    .child(if cx.global::<AppSettings>().send_on_enter {
                        "↵ to send  ·  ⇧↵ new line".to_string()
                    } else {
                        format!("{MOD_KEY}↵ to send  ·  ↵ new line")
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
                    .child(div().text_2xl().font_weight(FontWeight::BOLD).child(title))
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
            let installed = agent::installed(p.bin);
            let detail = if !installed {
                format!("{} not found on PATH", p.bin)
            } else if caps.version.is_empty() {
                "Installed".to_string()
            } else {
                format!("v{}", caps.version.trim_start_matches('v'))
            };
            providers = providers.child(
                card(("pv", i), self.new_provider == i)
                    .when(!installed, |el| el.opacity(0.6))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.new_provider = i;
                        cx.notify();
                    }))
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
        let isolated = self.new_isolated && is_git;
        let typed = git::sanitize_branch(&self.branch.read(cx).value());
        let branch = if typed.is_empty() {
            format!("runo/session-{}", self.state.next_id + 1)
        } else {
            typed
        };
        let where_to = match project {
            None => "Add a project to start.".to_string(),
            Some(p) if isolated => format!(
                "Creates branch {branch} in {}",
                git::worktree_path(&p.path, &branch).display()
            ),
            Some(p) if is_git => format!("Runs on the current branch, directly in {}", p.path),
            Some(p) => format!("Runs directly in {}", p.path),
        };
        let provider = &agent::PROVIDERS[self.new_provider];
        let task_empty = self.new_task.read(cx).value().trim().is_empty();
        let ready = project.is_some() && !task_empty;

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
                    .child(label("AGENT"))
                    .child(providers)
                    .child(label("WHERE IT RUNS"))
                    .child(if is_git {
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
                                    if isolated {
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

    fn list_view(&self, worktrees_only: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().v_flex().gap_2();
        for session in self
            .state
            .sessions
            .iter()
            .filter(|s| !worktrees_only || s.isolated)
        {
            let id = session.id;
            let provider = agent::provider(&session.provider);
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == session.project_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            rows = rows.child(
                div()
                    .id(("row", id as usize))
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
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(session.title.clone()),
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
            );
        }
        if self.state.sessions.is_empty() {
            rows = rows.child(
                div()
                    .text_sm()
                    .text_color(rgb(c_muted()))
                    .child("Nothing here yet."),
            );
        }
        self.page(
            if worktrees_only {
                "Worktrees"
            } else {
                "All sessions"
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
            .h_full()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_sm()
            .when(selected, |el| {
                el.font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .border_b_2()
                    .border_color(rgb(c_blue()))
            })
            .when(!selected, |el| el.text_color(rgb(c_secondary())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.right_tab = tab;
                cx.notify();
            }))
            .child(icon(
                name,
                16.,
                if selected { c_text() } else { c_secondary() },
            ))
            .child(label)
    }

    fn diff_lines(&self) -> impl IntoElement {
        use git::LineKind;
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
            col = col.child(
                div()
                    .id(("dl", i))
                    .flex()
                    .min_h(px(19.))
                    .when_some(bg, |el, bg| el.bg(rgb(bg)))
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
                            .whitespace_nowrap()
                            .text_color(rgb(color))
                            .when(row.kind == LineKind::Note, |el| el.italic())
                            .child(row.text),
                    ),
            );
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
        let mut list = div()
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .overflow_hidden();
        for (i, change) in self.changes.iter().enumerate() {
            let path = change.path.clone();
            let selected = self.selected_file.as_deref() == Some(change.path.as_str());
            list = list.child(stagger(
                div()
                    .id(("chg", i))
                    .h(px(38.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(rgb(c_selected())))
                    .hover(|el| el.bg(rgb(c_subtle())))
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
                            .w(px(16.))
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(change_color(&change.status)))
                            .child(change.status.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(rgb(c_text()))
                            .child(change.path.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(c_add()))
                            .child(format!("+{}", change.added)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(c_del()))
                            .child(format!("−{}", change.removed)),
                    ),
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
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Changes"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(c_add()))
                                    .child(format!("+{added}")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(c_del()))
                                    .child(format!("−{removed}")),
                            ),
                    )
                    .child(
                        self.button("refresh-changes", "Refresh", false)
                            .h(px(30.))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Re-read git status. Changes are compared with the last commit.",
                                )
                                .build(window, cx)
                            })
                            .child(icon(IconName::RefreshCw, 13., c_secondary()))
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
                            .child("No changes yet"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(c_muted()))
                            .child("Files the agent edits show up here as it works."),
                    )
                    .into_any_element()
            } else {
                div()
                    .max_h(px(170.))
                    .overflow_y_scrollbar()
                    .child(list)
                    .into_any_element()
            })
            .when(!self.changes.is_empty(), |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .v_flex()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .overflow_hidden()
                        .child(
                            div()
                                .h(px(40.))
                                .px_3()
                                .flex()
                                .items_center()
                                .gap_3()
                                .border_b_1()
                                .border_color(rgb(c_border()))
                                .child(icon(IconName::File, 16., c_secondary()))
                                .child(div().flex_1().truncate().text_sm().child(selected.clone()))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(c_add()))
                                        .child(format!("+{}", sel_stats.0)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(c_del()))
                                        .child(format!("−{}", sel_stats.1)),
                                ),
                        )
                        .child(fade(
                            div()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scrollbar()
                                .child(self.diff_lines()),
                            SharedString::from(format!(
                                "diff-{}-{}",
                                self.selected.unwrap_or(0),
                                selected
                            )),
                            220,
                        ))
                        .child(
                            div()
                                .h(px(46.))
                                .px_3()
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_t_1()
                                .border_color(rgb(c_border()))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_xs()
                                        .text_color(rgb(c_secondary()))
                                        .child(icon(IconName::GitCompare, 14., c_muted()))
                                        .child(format!(
                                            "{} file{} changed",
                                            self.changes.len(),
                                            if self.changes.len() == 1 { "" } else { "s" }
                                        )),
                                )
                                .child(
                                    self.button("open-file", "Open file", false)
                                        .h(px(30.))
                                        .child(icon(IconName::ArrowUpRight, 13., c_secondary()))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(path) = this.selected_file.clone() {
                                                this.open_preview(&path);
                                                this.right_tab = RightTab::Files;
                                            }
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            })
            .into_any_element()
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
            .h_full()
            .px_3()
            .flex()
            .items_center()
            .cursor_pointer()
            .text_sm()
            .when(selected, |el| {
                el.font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .border_b_2()
                    .border_color(rgb(c_text()))
            })
            .when(!selected, |el| el.text_color(rgb(c_secondary())))
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
                let ok = agent::installed(p.bin);
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
            .h(px(250.))
            .flex_none()
            .v_flex()
            .border_t_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_term_bg()))
            .child(
                div()
                    .h(px(40.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_canvas()))
                    .child(
                        div()
                            .h_full()
                            .flex()
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
                                    self.button("term-clear", "Clear", false)
                                        .h(px(26.))
                                        .px_2()
                                        .text_xs()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.terminal.remove(&id);
                                            cx.notify();
                                        })),
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
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .bg(rgb(c_canvas()))
                        .child(
                            div()
                                .font_family(self.mono_font.clone())
                                .text_sm()
                                .text_color(rgb(c_secondary()))
                                .child("$"),
                        )
                        .child(Input::new(&self.term_input).appearance(false).h(px(30.))),
                )
            })
    }

    fn right_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(500.))
            .flex_none()
            .h_full()
            .v_flex()
            .border_l_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_canvas()))
            .child(
                div()
                    .h(px(46.))
                    .px_2()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .child(self.right_tab_button(
                        RightTab::Changes,
                        IconName::GitCompare,
                        format!("Changes {}", self.changes.len()),
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
        }
        if self.applied_enter != Some(st.send_on_enter) {
            self.applied_enter = Some(st.send_on_enter);
            self.prompt
                .update(cx, |p, cx| p.set_submit_on_enter(st.send_on_enter, cx));
        }
        let center = match self.view {
            View::Session => self.session_view(cx),
            View::NewSession => self.new_session_view(cx),
            View::AllSessions => self.list_view(false, cx),
            View::Worktrees => self.list_view(true, cx),
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
        let summary = match (self.state.sessions.len(), running) {
            (0, _) => String::new(),
            (n, 0) => format!("{n} session{}", if n == 1 { "" } else { "s" }),
            (n, r) => format!(
                "{n} session{}  ·  {r} running",
                if n == 1 { "" } else { "s" }
            ),
        };
        let root = div()
            .size_full()
            .v_flex()
            .key_context("Runo")
            .track_focus(&self.focus)
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
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                if this.menu.is_some() || this.settings_menu.is_some() {
                    this.menu = None;
                    this.settings_menu = None;
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
            .font_family(cx.theme().font_family.clone())
            .child(self.title_bar())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(self.view != View::Settings, |el| el.child(self.sidebar(cx)))
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
                    .h(px(30.))
                    .flex_none()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_canvas()))
                    .text_xs()
                    .gap_4()
                    .child(fade(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .text_color(rgb(status_color))
                            .when_some(status_icon, |el, i| el.child(icon(i, 13., status_color)))
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
                                        .child(icon(IconName::X, 12., c_muted())),
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
                            .gap_2()
                            .text_color(rgb(c_secondary()))
                            .when(!summary.is_empty(), |el| {
                                el.child(summary)
                                    .child(div().text_color(rgb(c_border())).child("|"))
                            })
                            .child(div().size(px(7.)).rounded_full().bg(rgb(c_green())))
                            .child("Local environment"),
                    ),
            );
        fade(root, "app-root", 450)
    }
}

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        init(cx);
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
