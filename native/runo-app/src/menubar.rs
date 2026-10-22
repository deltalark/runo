//! Runo in the macOS menu bar: a glyph with a count, and a menu of what agents are doing,
//! what needs the person, and Ask Runo. The menu is AppKit's own, so it looks and behaves
//! like the system's. It is rebuilt from the latest snapshot each time it opens; the app
//! refreshes the snapshot every second and picks up clicks a few times a second.
//!
//! gpui has no tray support, so other platforms show nothing for now.

use crate::*;
use shell::Need;

/// Rows shown per section; the rest are one "more" line away.
const ROWS: usize = 8;

/// One session in the menu.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Row {
    pub id: u64,
    pub title: String,
    /// What it is doing and where: "Waiting on you · Claude · api".
    pub line: String,
    pub dot: Dot,
    /// The submenu's lines: agent and branch, status, last reply, changes and cost.
    pub detail: Vec<String>,
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
    /// Under "Runo" at the top: "3 working · 2 need you".
    pub summary: String,
    pub needs: Vec<Row>,
    pub more_needs: usize,
    pub working: Vec<Row>,
    pub more_working: usize,
    /// Idle sessions with changes to review.
    pub review: usize,
    /// No window is open: the menu only offers to open one.
    pub closed: bool,
}

/// A menu item picked by the person. Only the Mac menu makes them.
#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) enum Click {
    Open(u64),
    Ask,
    AskAbout(u64),
    NewSession,
    Inbox,
    Sessions,
    Review,
    OpenRuno,
    Quit,
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
            closed: false,
        }
    }

    fn menubar_row(&self, s: &Session, dot: Dot, status: String) -> Row {
        let agent = agent::provider(&s.provider).name;
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == s.project_id)
            .map(|p| p.name.as_str())
            .unwrap_or("");
        let mut line = vec![status.clone(), agent.to_string()];
        if !project.is_empty() {
            line.push(project.to_string());
        }

        let mut head = vec![agent.to_string()];
        if let Some(model) = s.opts.get("model").filter(|m| !m.is_empty()) {
            head.push(model.clone());
        }
        if !s.branch.is_empty() {
            head.push(format!("branch {}", s.branch));
        }
        let mut detail = vec![head.join(" · ")];
        detail.push(match dot {
            Dot::Working if status == "just now" => "Started just now".into(),
            Dot::Working => format!("Working for {status}"),
            _ => status,
        });
        if let Some(reply) = s
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Agent && !m.text.trim().is_empty())
        {
            detail.push(format!("“{}”", first_line(&reply.text, 60)));
        }
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
        if !facts.is_empty() {
            detail.push(facts.join(" · "));
        }
        Row {
            id: s.id,
            title: s.title.clone(),
            line: line.join(" · "),
            dot,
            detail,
        }
    }

    /// Act on a menu item that needs the window.
    pub(crate) fn menubar_click(
        &mut self,
        click: Click,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match click {
            Click::Open(id) => {
                if self.state.session(id).is_some() {
                    self.select(id);
                }
            }
            Click::Ask => self.open_assistant(window, cx),
            Click::AskAbout(id) => {
                if self.state.session(id).is_some() {
                    self.select(id);
                }
                self.open_assistant(window, cx);
            }
            Click::NewSession => self.open_new_session(),
            Click::Inbox => self.go(Mode::Inbox, window, cx),
            Click::Sessions => {
                self.view = View::AllSessions;
            }
            Click::Review => self.go(Mode::Home, window, cx),
            Click::OpenRuno | Click::Quit => {}
        }
        cx.notify();
    }
}

/// Show, refresh or hide the menu bar item, and act on what was picked. Runs every
/// quarter second on the main thread; the snapshot is rebuilt every fourth time.
fn tick(refresh: bool, cx: &mut App) {
    if !cx.global::<AppSettings>().menu_bar {
        platform::hide();
        return;
    }
    let app = cx.try_global::<MainApp>().and_then(|m| m.0.upgrade());
    if refresh || !platform::shown() {
        let snapshot = match &app {
            Some(app) => app.read(cx).menubar_snapshot(),
            None => Snapshot::closed(),
        };
        platform::show(snapshot);
    }
    for click in platform::take_clicks() {
        match click {
            Click::Quit => crate::quit(&Quit, cx),
            Click::OpenRuno if app.is_none() => {
                crate::open_main_window(cx);
                cx.activate(true);
            }
            _ => {
                let Some(app) = app.clone() else {
                    crate::open_main_window(cx);
                    cx.activate(true);
                    continue;
                };
                cx.activate(true);
                for window in cx.windows() {
                    let _ = window.update(cx, |_, window, cx| {
                        window.activate_window();
                        app.update(cx, |this, cx| this.menubar_click(click, window, cx));
                    });
                }
            }
        }
    }
}

/// Start the menu bar item, once, as the app starts.
pub(crate) fn install(cx: &mut App) {
    if !platform::SUPPORTED {
        return;
    }
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

#[cfg(target_os = "macos")]
use mac as platform;

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{Click, Snapshot};

    pub(super) const SUPPORTED: bool = false;

    pub(super) fn show(_: Snapshot) {}

    pub(super) fn shown() -> bool {
        false
    }

    pub(super) fn hide() {}

    pub(super) fn take_clicks() -> Vec<Click> {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
mod mac {
    //! The status item, its menu and the object its items call. Everything here lives on
    //! the main thread, where AppKit and gpui's foreground tasks both run.

    use super::{Click, Dot, Row, Snapshot};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, NSObject, ProtocolObject};
    use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
        NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar, NSStatusItem,
        NSVariableStatusItemLength,
    };
    use objc2_foundation::{
        NSAttributedString, NSData, NSDictionary, NSMutableAttributedString, NSObjectProtocol,
        NSRect, NSSize, NSString,
    };
    use std::cell::RefCell;

    pub(super) const SUPPORTED: bool = true;

    static GLYPH: &[u8] = include_bytes!("../assets/icon/menubar/runo-menubar.png");
    static GLYPH_DOT: &[u8] = include_bytes!("../assets/icon/menubar/runo-menubar-dot.png");

    struct Bar {
        item: Retained<NSStatusItem>,
        // Kept alive: the menu holds its delegate and the items their target weakly.
        _menu: Retained<NSMenu>,
        _target: Retained<Target>,
        /// What the button shows now: the count and whether it has the dot.
        button: (String, bool),
    }

    thread_local! {
        static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
        /// The latest snapshot, which the menu is built from when it opens.
        static LATEST: RefCell<Snapshot> = RefCell::new(Snapshot::default());
        /// What each item's tag stands for, from the last time the menu was built.
        static TAGS: RefCell<Vec<Click>> = const { RefCell::new(Vec::new()) };
        static CLICKS: RefCell<Vec<Click>> = const { RefCell::new(Vec::new()) };
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Target has no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "RunoMenuBarTarget"]
        struct Target;

        impl Target {
            #[unsafe(method(pick:))]
            fn pick(&self, sender: &NSMenuItem) {
                let tag = sender.tag();
                if let Some(click) = TAGS.with_borrow(|t| usize::try_from(tag).ok().and_then(|i| t.get(i).copied())) {
                    CLICKS.with_borrow_mut(|c| c.push(click));
                }
            }
        }

        unsafe impl NSObjectProtocol for Target {}

        unsafe impl NSMenuDelegate for Target {
            #[unsafe(method(menuNeedsUpdate:))]
            fn menu_needs_update(&self, menu: &NSMenu) {
                let mtm = self.mtm();
                LATEST.with_borrow(|s| build(menu, s, self, mtm));
            }
        }
    );

    impl Target {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
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
                let menu = NSMenu::new(mtm);
                menu.setAutoenablesItems(false);
                menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
                item.setMenu(Some(&menu));
                Bar {
                    item,
                    _menu: menu,
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
        LATEST.with_borrow_mut(|latest| *latest = snapshot);
    }

    pub(super) fn hide() {
        if let Some(bar) = BAR.with_borrow_mut(Option::take) {
            NSStatusBar::systemStatusBar().removeStatusItem(&bar.item);
        }
        CLICKS.with_borrow_mut(Vec::clear);
    }

    pub(super) fn take_clicks() -> Vec<Click> {
        CLICKS.with_borrow_mut(std::mem::take)
    }

    /// The glyph as a template image, so macOS draws it in the menu bar's own color.
    fn glyph(dot: bool) -> Option<Retained<NSImage>> {
        let data = NSData::with_bytes(if dot { GLYPH_DOT } else { GLYPH });
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        image.setSize(NSSize::new(18., 18.));
        image.setTemplate(true);
        Some(image)
    }

    /// A small filled circle in `color`, for a session's row.
    fn dot(dot: Dot) -> Retained<NSImage> {
        let draw = RcBlock::new(move |rect: NSRect| -> Bool {
            let color = match dot {
                Dot::Waiting => NSColor::systemOrangeColor(),
                Dot::Failed => NSColor::systemRedColor(),
                Dot::Finished => NSColor::systemGreenColor(),
                Dot::Working => NSColor::systemBlueColor(),
            };
            color.setFill();
            let inset = rect.size.width * 0.15;
            let oval = NSRect::new(
                objc2_foundation::NSPoint::new(rect.origin.x + inset, rect.origin.y + inset),
                NSSize::new(rect.size.width - inset * 2., rect.size.height - inset * 2.),
            );
            NSBezierPath::bezierPathWithOvalInRect(oval).fill();
            Bool::YES
        });
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(10., 10.), false, &draw)
    }

    /// `text` in the menu's font, `size` points, secondary colour when `dim`.
    fn styled(text: &str, size: f64, bold: bool, dim: bool) -> Retained<NSAttributedString> {
        let font = if bold {
            NSFont::boldSystemFontOfSize(size)
        } else {
            NSFont::menuFontOfSize(size)
        };
        let color = if dim {
            NSColor::secondaryLabelColor()
        } else {
            NSColor::labelColor()
        };
        let font: &AnyObject = (*font).as_ref();
        let color: &AnyObject = (*color).as_ref();
        let attrs: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
            unsafe { &[NSFontAttributeName, NSForegroundColorAttributeName] },
            &[font, color],
        );
        unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(text), &attrs) }
    }

    /// A title over a smaller, dimmer line.
    fn two_lines(title: &str, line: &str) -> Retained<NSAttributedString> {
        let out =
            NSMutableAttributedString::from_attributed_nsstring(&styled(title, 13., false, false));
        out.appendAttributedString(&styled(&format!("\n{line}"), 11., false, true));
        Retained::into_super(out)
    }

    fn item(title: &str, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
        unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        }
    }

    /// A line that only informs.
    fn label(menu: &NSMenu, text: Retained<NSAttributedString>, mtm: MainThreadMarker) {
        let it = item("", mtm);
        it.setAttributedTitle(Some(&text));
        it.setEnabled(false);
        menu.addItem(&it);
    }

    /// Fill `menu` from `s`. Each clickable item's tag indexes `TAGS`.
    fn build(menu: &NSMenu, s: &Snapshot, target: &Target, mtm: MainThreadMarker) {
        menu.removeAllItems();
        let mut tags = Vec::new();
        let mut action = |menu: &NSMenu, it: Retained<NSMenuItem>, click: Click| {
            it.setTag(tags.len() as isize);
            tags.push(click);
            unsafe {
                it.setTarget(Some(target));
                it.setAction(Some(sel!(pick:)));
            }
            menu.addItem(&it);
        };
        let plain = |title: &str| item(title, mtm);

        let head =
            NSMutableAttributedString::from_attributed_nsstring(&styled("Runo", 13., true, false));
        head.appendAttributedString(&styled(&format!("\n{}", s.summary), 11., false, true));
        label(menu, Retained::into_super(head), mtm);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        if s.closed {
            action(menu, plain("Open Runo"), Click::OpenRuno);
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            action(menu, plain("Quit Runo"), Click::Quit);
            TAGS.with_borrow_mut(|t| *t = tags);
            return;
        }

        let section =
            |menu: &NSMenu,
             title: &str,
             rows: &[Row],
             more: usize,
             more_click: Click,
             action: &mut dyn FnMut(&NSMenu, Retained<NSMenuItem>, Click)| {
                if rows.is_empty() {
                    return;
                }
                label(menu, styled(title, 11., true, true), mtm);
                for row in rows {
                    let it = plain(&row.title);
                    it.setAttributedTitle(Some(&two_lines(&row.title, &row.line)));
                    it.setImage(Some(&dot(row.dot)));
                    // An item with a submenu opens it rather than acting, so Open is in there.
                    let sub = NSMenu::new(mtm);
                    sub.setAutoenablesItems(false);
                    let mut lines = row.detail.iter();
                    let top = NSMutableAttributedString::from_attributed_nsstring(&styled(
                        &row.title, 13., true, false,
                    ));
                    if let Some(first) = lines.next() {
                        top.appendAttributedString(&styled(
                            &format!("\n{first}"),
                            11.,
                            false,
                            true,
                        ));
                    }
                    label(&sub, Retained::into_super(top), mtm);
                    sub.addItem(&NSMenuItem::separatorItem(mtm));
                    for line in lines {
                        label(&sub, styled(line, 12., false, true), mtm);
                    }
                    sub.addItem(&NSMenuItem::separatorItem(mtm));
                    action(&sub, plain("Open session"), Click::Open(row.id));
                    action(&sub, plain("Ask Runo about this…"), Click::AskAbout(row.id));
                    it.setSubmenu(Some(&sub));
                    menu.addItem(&it);
                }
                if more > 0 {
                    action(menu, plain(&format!("{more} more…")), more_click);
                }
            };
        section(
            menu,
            "NEEDS YOU",
            &s.needs,
            s.more_needs,
            Click::Inbox,
            &mut action,
        );
        section(
            menu,
            "WORKING",
            &s.working,
            s.more_working,
            Click::Sessions,
            &mut action,
        );
        if s.review > 0 {
            let text = if s.review == 1 {
                "1 session has changes to review".to_string()
            } else {
                format!("{} sessions have changes to review", s.review)
            };
            action(menu, plain(&text), Click::Review);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let ask = plain("Ask Runo…");
        ask.setAttributedTitle(Some(&styled("Ask Runo…", 13., true, false)));
        action(menu, ask, Click::Ask);
        action(menu, plain("New session…"), Click::NewSession);
        action(menu, plain("Open Runo"), Click::OpenRuno);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        action(menu, plain("Quit Runo"), Click::Quit);
        TAGS.with_borrow_mut(|t| *t = tags);
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
    fn closed_offers_only_open() {
        let s = Snapshot::closed();
        assert!(s.closed && s.needs.is_empty() && s.count.is_empty());
    }
}
