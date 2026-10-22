//! Runo in the macOS menu bar: a glyph with a count, and a panel under it with what
//! agents are doing, what needs the person, and Ask Runo, all usable in place: allow or
//! deny a waiting agent, stop one, or chat with the assistant without opening Runo.
//! With the menu bar on, closing the window or ⌘Q hides Runo to it so agents keep
//! running; Quit in the panel quits.
//!
//! gpui has no tray support, so other platforms show nothing for now.

use crate::*;
use shell::Need;

/// Rows shown per section; the rest are one "more" line away.
pub(crate) const ROWS: usize = 8;

/// One session in the panel.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Row {
    pub id: u64,
    pub title: String,
    pub project: String,
    pub branch: String,
    /// The agent's name, glyph and colour, for its badge.
    pub agent: &'static str,
    pub mark: &'static str,
    pub color: u32,
    pub dot: Dot,
    /// "Waiting on you", "Failed", "Finished", or how long it has run: "12m".
    pub status: String,
    /// The first line of its last reply.
    pub last: Option<String>,
    /// "4 files changed · $0.86".
    pub facts: String,
    /// The permission it waits on: "Claude wants to run" and the command, if any.
    pub approval: Option<(String, Option<String>)>,
    /// Its agent runs on this machine and can be stopped.
    pub stoppable: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Dot {
    Waiting,
    Failed,
    Finished,
    Working,
}

/// What the menu bar shows, built from the app's sessions.
#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Snapshot {
    /// Beside the glyph: what needs the person, else how many are working; empty for none.
    pub count: String,
    /// The glyph carries a dot while something needs the person.
    pub attention: bool,
    /// Beside "Runo" at the top: "3 working · 2 need you".
    pub summary: String,
    pub needs: Vec<Row>,
    pub more_needs: usize,
    pub working: Vec<Row>,
    pub more_working: usize,
    /// Idle sessions with changes to review.
    pub review: usize,
    pub running: usize,
    pub needs_you: usize,
    /// No window is open: the panel only offers to open one.
    pub closed: bool,
}

/// Somewhere in Runo the panel sends the person: the window comes forward on it.
#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) enum Go {
    Open(u64),
    NewSession,
    Inbox,
    Sessions,
    Review,
    Settings,
    OpenRuno,
}

impl Snapshot {
    /// Runo is running with its window closed.
    pub(crate) fn closed() -> Self {
        Self {
            summary: "No window open".into(),
            closed: true,
            ..Default::default()
        }
    }
}

/// "just now", "12m", "1h 5m".
pub(crate) fn age(secs: u64) -> String {
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

/// The first line of `text`, cut to about `max` characters.
fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max).collect::<String>().trim_end())
    } else {
        line.to_string()
    }
}

/// "run `cargo test`" as ("run", Some("cargo test")); text without a code span stays whole.
fn split_code(what: &str) -> (String, Option<String>) {
    match what.split_once('`') {
        Some((verb, rest)) => (
            verb.trim().to_string(),
            Some(rest.trim_end_matches('`').to_string()),
        ),
        None => (what.to_string(), None),
    }
}

impl RunoApp {
    /// What the menu bar shows now.
    pub(crate) fn menubar_snapshot(&self) -> Snapshot {
        let counts = self.counts();
        let mut needs = Vec::new();
        for (s, need) in self.needs_you() {
            let dot = match need {
                Need::Blocked => Dot::Waiting,
                Need::Failed => Dot::Failed,
                Need::Finished => Dot::Finished,
            };
            needs.push(self.menubar_row(s, dot, need.label().to_string()));
        }
        let mut working: Vec<(&Session, u64)> = self
            .state
            .sessions
            .iter()
            .filter(|s| self.bucket(s) == Some(shell::Bucket::Running))
            .map(|s| {
                let secs = self
                    .started
                    .get(&s.id)
                    .map(|at| at.elapsed().as_secs())
                    .unwrap_or_default();
                (s, secs)
            })
            .collect();
        // The longest running first.
        working.sort_by_key(|(s, secs)| (std::cmp::Reverse(*secs), s.id));
        let working: Vec<Row> = working
            .into_iter()
            .map(|(s, secs)| self.menubar_row(s, Dot::Working, age(secs)))
            .collect();

        let mut summary = Vec::new();
        if counts.running > 0 {
            summary.push(format!("{} working", counts.running));
        }
        if counts.needs_you > 0 {
            summary.push(format!(
                "{} need{} you",
                counts.needs_you,
                if counts.needs_you == 1 { "s" } else { "" }
            ));
        }
        if summary.is_empty() {
            summary.push("Nothing running".into());
        }
        let count = match (counts.needs_you, counts.running) {
            (0, 0) => String::new(),
            (0, n) | (n, _) => n.to_string(),
        };
        Snapshot {
            count,
            attention: counts.needs_you > 0,
            summary: summary.join(" · "),
            more_needs: needs.len().saturating_sub(ROWS),
            needs: needs.into_iter().take(ROWS).collect(),
            more_working: working.len().saturating_sub(ROWS),
            working: working.into_iter().take(ROWS).collect(),
            review: counts.review,
            running: counts.running,
            needs_you: counts.needs_you,
            closed: false,
        }
    }

    fn menubar_row(&self, s: &Session, dot: Dot, status: String) -> Row {
        let provider = agent::provider(&s.provider);
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == s.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let last = s
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Agent && !m.text.trim().is_empty())
            .map(|m| first_line(&m.text, 80));
        let mut facts = Vec::new();
        if let Some((files, _, _)) = self.stats.get(&s.id).filter(|st| st.0 > 0) {
            facts.push(format!(
                "{files} file{} changed",
                if *files == 1 { "" } else { "s" }
            ));
        }
        let cents: f64 = s.messages.iter().map(|m| m.usage.cost_cents).sum();
        if cents >= 0.5 {
            facts.push(format!("${:.2}", cents / 100.));
        }
        let approval = self.approval_for(s.id).map(|r| {
            let (verb, code) = split_code(&approvals::describe(&r.tool, &r.input));
            (format!("{} wants to {verb}", provider.name), code)
        });
        Row {
            id: s.id,
            title: s.title.clone(),
            project,
            branch: s.branch.clone(),
            agent: provider.name,
            mark: provider.mark,
            color: provider.color,
            dot,
            status,
            last,
            facts: facts.join(" · "),
            approval,
            stoppable: self.handles.contains_key(&Target::Session(s.id)),
        }
    }

    /// Show the place `go` names in the window.
    pub(crate) fn menubar_go(&mut self, go: Go, window: &mut Window, cx: &mut Context<Self>) {
        match go {
            Go::Open(id) => {
                if self.state.session(id).is_some() {
                    self.select(id);
                }
            }
            Go::NewSession => self.open_new_session(),
            Go::Inbox => self.go(Mode::Inbox, window, cx),
            Go::Sessions => self.view = View::AllSessions,
            Go::Review => self.go(Mode::Home, window, cx),
            Go::Settings => self.open_settings(),
            Go::OpenRuno => {}
        }
        cx.notify();
    }
}

/// Whether ⌘Q and closing the window hide Runo to the menu bar instead of quitting.
pub(crate) fn keeps_running(cx: &App) -> bool {
    let st = cx.global::<AppSettings>();
    platform::SUPPORTED && st.menu_bar && st.run_in_background
}

/// Hide Runo to the menu bar: the window and the Dock icon go, the agents keep running.
/// The first time, a notification says where Runo went.
pub(crate) fn to_background(cx: &mut App) {
    if let Some(app) = cx.try_global::<MainApp>().and_then(|m| m.0.upgrade()) {
        app.update(cx, |this, _| this.save_now());
    }
    panel::close(cx);
    if let Some(window) = cx.try_global::<MainApp>().map(|m| m.1) {
        let _ = window.update(cx, |_, window, _| platform::order_out(window));
    }
    platform::set_dock_icon(false);
    if !cx.global::<AppSettings>().background_told {
        settings::update(cx, |s| s.background_told = true);
        crate::desktop_notify::send(
            cx,
            None,
            "Runo is still running",
            "It's in the menu bar, and your agents keep working. Quit from there to quit.",
        );
    }
}

/// Bring Runo back from the menu bar, with its Dock icon.
pub(crate) fn to_foreground(cx: &mut App) {
    platform::set_dock_icon(true);
    cx.activate(true);
    match cx.try_global::<MainApp>().map(|m| m.1) {
        Some(window) => {
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
        None => crate::open_main_window(cx),
    }
}

/// Bring the window forward on `go`, closing the panel.
pub(crate) fn go(go: Go, cx: &mut App) {
    panel::close(cx);
    let main = cx
        .try_global::<MainApp>()
        .and_then(|m| m.0.upgrade().map(|app| (app, m.1)));
    to_foreground(cx);
    if let Some((app, window)) = main {
        let _ = window.update(cx, |_, window, cx| {
            app.update(cx, |this, cx| this.menubar_go(go, window, cx));
        });
    }
}

/// Quit for real, from the panel: running agents still get the usual warning, in the
/// window.
pub(crate) fn quit(cx: &mut App) {
    panel::close(cx);
    let app = cx.try_global::<MainApp>().and_then(|m| m.0.upgrade());
    let go = app.is_none_or(|app| {
        app.update(cx, |this, cx| {
            this.save();
            this.may_close(cx)
        })
    });
    if go {
        cx.quit();
    } else {
        to_foreground(cx);
    }
}

/// What the menu bar shows now.
pub(crate) fn snapshot(cx: &App) -> Snapshot {
    match cx.try_global::<MainApp>().and_then(|m| m.0.upgrade()) {
        Some(app) => app.read(cx).menubar_snapshot(),
        None => Snapshot::closed(),
    }
}

/// Show, refresh or hide the menu bar item, and open or close the panel when the glyph
/// was clicked.
fn tick(refresh: bool, cx: &mut App) {
    if !cx.global::<AppSettings>().menu_bar {
        panel::close(cx);
        platform::hide();
        return;
    }
    if refresh || !platform::shown() {
        platform::show(snapshot(cx));
    }
    if platform::take_toggle() {
        panel::toggle(platform::anchor(), cx);
    }
}

/// Start the menu bar item, once, as the app starts.
pub(crate) fn install(cx: &mut App) {
    if !platform::SUPPORTED {
        return;
    }
    // A click on the glyph is handled straight away rather than on the next tick.
    let async_cx = cx.to_async();
    platform::on_click(move || {
        let cx = async_cx.clone();
        async_cx
            .foreground_executor()
            .spawn(async move { cx.update(|cx| tick(false, cx)) })
            .detach();
    });
    cx.spawn(async move |cx| {
        let mut n = 0u32;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let refresh = n.is_multiple_of(4);
            n = n.wrapping_add(1);
            cx.update(|cx| tick(refresh, cx));
        }
    })
    .detach();
}

#[path = "menubar_panel.rs"]
mod panel;

/// Where the glyph is on screen, in the display's own coordinates from its top left:
/// the point under its middle where the panel hangs, and the display it is on.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Anchor {
    pub x: f32,
    pub y: f32,
    pub display: Option<u32>,
}

#[cfg(target_os = "macos")]
use mac as platform;

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{Anchor, Snapshot};

    pub(super) const SUPPORTED: bool = false;

    pub(super) fn show(_: Snapshot) {}

    pub(super) fn shown() -> bool {
        false
    }

    pub(super) fn hide() {}

    pub(super) fn take_toggle() -> bool {
        false
    }

    pub(super) fn on_click(_: impl Fn() + 'static) {}

    pub(super) fn anchor() -> Option<Anchor> {
        None
    }

    pub(super) fn set_dock_icon(_: bool) {}

    pub(super) fn place(_: &gpui_kit::Window, _: f32, _: f32) {}

    pub(super) fn order_out(_: &gpui_kit::Window) {}
}

#[cfg(target_os = "macos")]
mod mac {
    //! The status item and the object its button calls. Everything here lives on the main
    //! thread, where AppKit and gpui's foreground tasks both run.

    use super::{Anchor, Snapshot};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSImage, NSScreen, NSStatusBar, NSStatusItem,
        NSVariableStatusItemLength, NSView, NSWindow,
    };
    use objc2_foundation::{NSData, NSObjectProtocol, NSPoint, NSSize, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::{Cell, RefCell};

    pub(super) const SUPPORTED: bool = true;

    static GLYPH: &[u8] = include_bytes!("../assets/icon/menubar/runo-menubar.png");
    static GLYPH_DOT: &[u8] = include_bytes!("../assets/icon/menubar/runo-menubar-dot.png");

    struct Bar {
        item: Retained<NSStatusItem>,
        // Kept alive: the button holds its target weakly.
        _target: Retained<Target>,
        /// What the button shows now: the count and whether it has the dot.
        button: (String, bool),
    }

    thread_local! {
        static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
        static CLICKED: Cell<bool> = const { Cell::new(false) };
        static ON_CLICK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Target has no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "RunoMenuBarTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(toggle:))]
            fn toggle(&self, _sender: &AnyObject) {
                CLICKED.set(true);
                ON_CLICK.with_borrow(|f| {
                    if let Some(f) = f {
                        f();
                    }
                });
            }
        }

        unsafe impl NSObjectProtocol for Target {}
    );

    impl Target {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    pub(super) fn on_click(f: impl Fn() + 'static) {
        ON_CLICK.with_borrow_mut(|slot| *slot = Some(Box::new(f)));
    }

    pub(super) fn take_toggle() -> bool {
        CLICKED.replace(false)
    }

    pub(super) fn shown() -> bool {
        BAR.with_borrow(|b| b.is_some())
    }

    pub(super) fn show(snapshot: Snapshot) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        BAR.with_borrow_mut(|bar| {
            let bar = bar.get_or_insert_with(|| {
                let item =
                    NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
                let target = Target::new(mtm);
                if let Some(button) = item.button(mtm) {
                    unsafe {
                        button.setTarget(Some(&target));
                        button.setAction(Some(sel!(toggle:)));
                    }
                }
                Bar {
                    item,
                    _target: target,
                    button: (String::from("unset"), false),
                }
            });
            let want = (snapshot.count.clone(), snapshot.attention);
            if bar.button != want {
                if let Some(button) = bar.item.button(mtm) {
                    button.setImage(glyph(snapshot.attention).as_deref());
                    // A thin space keeps the count off the glyph.
                    let title = if want.0.is_empty() {
                        String::new()
                    } else {
                        format!("\u{2009}{}", want.0)
                    };
                    button.setTitle(&NSString::from_str(&title));
                    button.setToolTip(Some(&NSString::from_str(&format!(
                        "Runo: {}",
                        snapshot.summary
                    ))));
                }
                bar.button = want;
            }
        });
    }

    pub(super) fn hide() {
        if let Some(bar) = BAR.with_borrow_mut(Option::take) {
            NSStatusBar::systemStatusBar().removeStatusItem(&bar.item);
        }
        CLICKED.set(false);
    }

    /// The glyph as a template image, so macOS draws it in the menu bar's own color.
    fn glyph(dot: bool) -> Option<Retained<NSImage>> {
        let data = NSData::with_bytes(if dot { GLYPH_DOT } else { GLYPH });
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        image.setSize(NSSize::new(18., 18.));
        image.setTemplate(true);
        Some(image)
    }

    /// The glyph's screen: its frame, and its id as gpui names displays.
    fn screen_of(button: &NSView) -> Option<(objc2_foundation::NSRect, Retained<NSScreen>)> {
        let window = button.window()?;
        let frame = window.frame();
        let screen = window.screen()?;
        Some((frame, screen))
    }

    pub(super) fn anchor() -> Option<Anchor> {
        let mtm = MainThreadMarker::new()?;
        BAR.with_borrow(|bar| {
            let button = bar.as_ref()?.item.button(mtm)?;
            let (frame, screen) = screen_of(&button)?;
            let display = screen.frame();
            let number = screen
                .deviceDescription()
                .objectForKey(&NSString::from_str("NSScreenNumber"))
                .and_then(|n| n.downcast::<objc2_foundation::NSNumber>().ok())
                .map(|n| n.unsignedIntValue());
            Some(Anchor {
                x: (frame.origin.x + frame.size.width / 2. - display.origin.x) as f32,
                y: (display.origin.y + display.size.height - frame.origin.y) as f32,
                display: number,
            })
        })
    }

    /// Hang the panel `window` under the glyph, kept on screen, and bring it up.
    pub(super) fn place(window: &gpui_kit::Window, width: f32, height: f32) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(panel) = ns_window(window) else {
            return;
        };
        let spot = BAR.with_borrow(|bar| {
            let button = bar.as_ref()?.item.button(mtm)?;
            screen_of(&button)
        });
        let Some((frame, screen)) = spot else {
            return;
        };
        let visible = screen.visibleFrame();
        let (w, h) = (width as f64, height as f64);
        let x = (frame.origin.x + frame.size.width / 2. - w / 2.)
            .min(visible.origin.x + visible.size.width - w - 8.)
            .max(visible.origin.x + 8.);
        let top = frame.origin.y - 6.;
        panel.setFrameTopLeftPoint(NSPoint::new(x, top.max(visible.origin.y + h)));
        panel.makeKeyAndOrderFront(None);
    }

    /// The AppKit window behind a gpui one.
    fn ns_window(window: &gpui_kit::Window) -> Option<Retained<NSWindow>> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: gpui hands out its own content view, alive while the window is.
        let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    /// Take the window off screen without closing it; showing it again brings it back.
    pub(super) fn order_out(window: &gpui_kit::Window) {
        if let Some(window) = ns_window(window) {
            window.orderOut(None);
        }
    }

    /// Show or hide Runo's Dock icon: hidden while it runs only in the menu bar.
    pub(super) fn set_dock_icon(shown: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(if shown {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        });
        if shown {
            // The icon comes back as the bundle's; set Runo's own again.
            crate::set_dock_icon();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn ages_read_short() {
        assert_eq!(age(5), "just now");
        assert_eq!(age(12 * 60 + 30), "12m");
        assert_eq!(age(3600 + 5 * 60), "1h 5m");
    }

    #[test]
    fn first_line_skips_blanks_and_cuts() {
        assert_eq!(
            first_line("\n\n  Done. All tests pass.\nMore", 60),
            "Done. All tests pass."
        );
        assert_eq!(first_line("abcdef", 3), "abc…");
        assert_eq!(first_line("", 3), "");
    }

    #[test]
    fn commands_come_out_of_their_code_span() {
        assert_eq!(
            split_code("run `cargo test -p runo`"),
            ("run".to_string(), Some("cargo test -p runo".to_string()))
        );
        assert_eq!(
            split_code("edit src/main.rs"),
            ("edit src/main.rs".to_string(), None)
        );
    }

    #[test]
    fn closed_offers_only_open() {
        let s = Snapshot::closed();
        assert!(s.closed && s.needs.is_empty() && s.count.is_empty());
    }
}
