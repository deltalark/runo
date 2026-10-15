//! The Workbench frame around every screen: the mode rail on the left, the top bar
//! with the project switcher, search and live counters, the tab strip over a session,
//! and the Home page. Design and Architecture have their places on the rail; their pages
//! live in `design` and `architecture`.

use super::*;
use gpui_kit::component::tooltip::Tooltip;

/// Width of the mode rail.
pub(crate) const RAIL_WIDTH: f32 = 52.;

/// The places on the mode rail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Home,
    Sessions,
    Design,
    Architecture,
    Review,
    GitHub,
    Playbooks,
    History,
    Inbox,
    Notes,
    Settings,
}

impl Mode {
    /// The modes at the top of the rail, in order; ⌘1 to ⌘6 jump to them.
    pub(crate) const MAIN: [Mode; 6] = [
        Mode::Home,
        Mode::Sessions,
        Mode::Review,
        Mode::GitHub,
        Mode::Playbooks,
        Mode::History,
    ];

    /// The current project's own modes, grouped under its badge below the main ones; ⌘7
    /// and ⌘8.
    pub(crate) const PLAN: [Mode; 2] = [Mode::Design, Mode::Architecture];

    /// Whether the mode shows one project at a time, or a chooser under All projects.
    pub(crate) fn per_project(self) -> bool {
        Self::PLAN.contains(&self)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Home => "Home",
            Mode::Sessions => "Sessions",
            Mode::Design => "Design",
            Mode::Architecture => "Architecture",
            Mode::Review => "Review",
            Mode::GitHub => "GitHub",
            Mode::Playbooks => "Playbooks",
            Mode::History => "History",
            Mode::Inbox => "Inbox",
            Mode::Notes => "Notes",
            Mode::Settings => "Settings",
        }
    }

    /// The shortcut that opens this mode, as Settings > Keybindings names it.
    fn shortcut(self) -> &'static str {
        match self {
            Mode::Home => "go-home",
            Mode::Sessions => "go-sessions",
            Mode::Design => "go-design",
            Mode::Architecture => "go-architecture",
            Mode::Review => "go-review",
            // Kept under its old name so shortcuts people changed still apply.
            Mode::GitHub => "go-reviews",
            Mode::Playbooks => "go-playbooks",
            Mode::History => "go-history",
            Mode::Inbox => "go-inbox",
            Mode::Notes => "go-notes",
            Mode::Settings => "settings",
        }
    }

    pub(crate) fn glyph(self) -> IconName {
        match self {
            Mode::Home => IconName::House,
            Mode::Sessions => IconName::CodeXml,
            Mode::Design => IconName::PanelsTopLeft,
            Mode::Architecture => IconName::Network,
            Mode::Review => IconName::GitCompare,
            Mode::GitHub => IconName::GitPullRequest,
            Mode::Playbooks => IconName::BookOpen,
            Mode::History => IconName::RotateCcwClock,
            Mode::Inbox => IconName::Inbox,
            Mode::Notes => IconName::NotebookPen,
            Mode::Settings => IconName::Settings,
        }
    }

    /// The mode a view belongs to.
    pub(crate) fn of(view: View) -> Mode {
        match view {
            View::Home => Mode::Home,
            View::Design => Mode::Design,
            View::Architecture => Mode::Architecture,
            View::Automations => Mode::Playbooks,
            View::Review => Mode::Review,
            View::GitHub => Mode::GitHub,
            View::History => Mode::History,
            View::Inbox => Mode::Inbox,
            View::Notes => Mode::Notes,
            View::Settings => Mode::Settings,
            View::Session
            | View::NewSession
            | View::AllSessions
            | View::Worktrees
            | View::Activity
            | View::Editor => Mode::Sessions,
        }
    }
}

/// Where a session sits on Home and in the sidebar's status groups. The same four words
/// are used everywhere: Needs you, Running, Ready to review and Done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Bucket {
    NeedsYou,
    Running,
    Review,
    Done,
}

impl Bucket {
    pub(crate) const ALL: [Bucket; 4] = [
        Bucket::NeedsYou,
        Bucket::Running,
        Bucket::Review,
        Bucket::Done,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Bucket::NeedsYou => "Needs you",
            Bucket::Running => "Running",
            Bucket::Review => "Ready to review",
            Bucket::Done => "Done",
        }
    }
}

/// Why a session needs the person.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Need {
    /// Paused on a permission, a question or a plan to approve.
    Blocked,
    /// Failed while the person was looking elsewhere.
    Failed,
    /// Finished while the person was looking elsewhere.
    Finished,
}

impl Need {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Need::Blocked => "Waiting on you",
            Need::Failed => "Failed",
            Need::Finished => "Finished",
        }
    }
}

/// The one rule for "needs you", shared by the bell, the rail, Home, the sidebar and the
/// Inbox. A blocked agent needs the person until it is answered or its card dismissed; a
/// finished or failed one until its session is opened, which clears `unread`.
pub(crate) fn need_of(
    s: &Session,
    running: bool,
    hidden: &std::collections::HashSet<String>,
) -> Option<Need> {
    if s.archived || running {
        return None;
    }
    if let Some((index, _)) = alerts::waiting_on(s) {
        if !hidden.contains(&format!("s:{}:{index}", s.id)) {
            return Some(Need::Blocked);
        }
    }
    match (s.unread, s.status) {
        (true, Status::Failed) => Some(Need::Failed),
        (true, _) => Some(Need::Finished),
        _ => None,
    }
}

/// The id `step` places after `at` in `list`; with `wrap`, past either end comes round.
/// With nothing picked yet, moving down picks the first and moving up the last.
pub(crate) fn step_in(list: &[u64], at: Option<u64>, step: isize, wrap: bool) -> Option<u64> {
    if list.is_empty() {
        return None;
    }
    let n = list.len() as isize;
    let next = match at.and_then(|id| list.iter().position(|x| *x == id)) {
        None if step < 0 => n - 1,
        None => 0,
        Some(i) if wrap => (i as isize + step).rem_euclid(n),
        Some(i) => (i as isize + step).clamp(0, n - 1),
    };
    list.get(next as usize).copied()
}

/// Move tab `id` to where `target` is, shifting the others along.
pub(crate) fn move_tab(tabs: &mut Vec<u64>, id: u64, target: u64) {
    let (Some(from), Some(to)) = (
        tabs.iter().position(|t| *t == id),
        tabs.iter().position(|t| *t == target),
    ) else {
        return;
    };
    let tab = tabs.remove(from);
    tabs.insert(to, tab);
}

/// Totals shown in the top bar and on Home.
#[derive(Default)]
pub(crate) struct Counts {
    pub running: usize,
    /// Sessions blocked on the person, or finished or failed unseen.
    pub needs_you: usize,
    /// Idle sessions with uncommitted changes.
    pub review: usize,
    /// Spend reported by the agents today.
    pub spend_today: f64,
    /// Spend over every session ever, for the tooltip.
    pub spend_cents: f64,
}

/// A tab being dragged to a new place in the strip.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub id: u64,
    pub title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()))
            .shadow_md()
            .text_size(px(12.5))
            .text_color(rgb(c_text()))
            .child(self.title.clone())
    }
}

impl RunoApp {
    /// The mode of what is on screen; the New session dialog keeps the mode behind it.
    pub(crate) fn mode(&self) -> Mode {
        match self.view {
            View::NewSession => Mode::of(self.new_back),
            view => Mode::of(view),
        }
    }

    pub(crate) fn go(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.cursor = None;
        match mode {
            Mode::Home => self.view = View::Home,
            Mode::Sessions => {
                self.view = if self.current().is_some() {
                    View::Session
                } else {
                    View::AllSessions
                }
            }
            Mode::Design => self.view = View::Design,
            Mode::Architecture => self.view = View::Architecture,
            Mode::Review => self.open_review(),
            Mode::GitHub => self.load_github(),
            Mode::Playbooks => self.view = View::Automations,
            Mode::History => self.open_history(cx),
            Mode::Inbox => self.view = View::Inbox,
            Mode::Notes => self.open_notes(window, cx),
            Mode::Settings => self.open_settings(),
        }
        self.save();
        window.focus(&self.focus, cx);
    }

    /// Whether the session's agent is running now.
    pub(crate) fn is_running(&self, s: &Session) -> bool {
        self.started.contains_key(&s.id)
            || s.status == Status::Working
            || self.handles.contains_key(&Target::Session(s.id))
    }

    pub(crate) fn need(&self, s: &Session) -> Option<Need> {
        // A running agent paused on a permission request needs the person now.
        if !s.archived
            && self
                .approval_for(s.id)
                .is_some_and(|r| !self.alerts_hidden.contains(&format!("a:{}", r.id)))
        {
            return Some(Need::Blocked);
        }
        need_of(s, self.is_running(s), &self.alerts_hidden)
    }

    /// Sessions that need the person, blocked ones first, then the newest.
    pub(crate) fn needs_you(&self) -> Vec<(&Session, Need)> {
        let mut out: Vec<(&Session, Need)> = self
            .state
            .sessions
            .iter()
            .filter_map(|s| self.need(s).map(|n| (s, n)))
            .collect();
        out.sort_by_key(|(s, n)| (*n != Need::Blocked, std::cmp::Reverse(s.id)));
        out
    }

    pub(crate) fn bucket(&self, s: &Session) -> Option<Bucket> {
        if s.archived {
            None
        } else if self.need(s).is_some() {
            Some(Bucket::NeedsYou)
        } else if self.is_running(s) {
            Some(Bucket::Running)
        } else if self.stats.get(&s.id).is_some_and(|st| st.0 > 0) {
            Some(Bucket::Review)
        } else {
            Some(Bucket::Done)
        }
    }

    /// Whether Home shows this session: every project's with "All projects", else only the
    /// top bar's project.
    pub(crate) fn on_home(&self, s: &Session) -> bool {
        self.home_all || Some(s.project_id) == self.current_project()
    }

    pub(crate) fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for s in &self.state.sessions {
            match self.bucket(s) {
                Some(Bucket::NeedsYou) => counts.needs_you += 1,
                Some(Bucket::Running) => counts.running += 1,
                Some(Bucket::Review) => counts.review += 1,
                _ => {}
            }
        }
        counts.spend_cents = self
            .state
            .sessions
            .iter()
            .flat_map(|s| s.messages.iter())
            .map(|m| m.usage.cost_cents)
            .sum();
        counts.spend_today = self
            .state
            .spend
            .get(&store::today())
            .copied()
            .unwrap_or_default();
        counts
    }

    /// Note a finished turn's cost under today, for the top bar's spend.
    pub(crate) fn record_spend(&mut self, id: u64) {
        let cents = self
            .state
            .session(id)
            .and_then(|s| s.messages.iter().rev().find(|m| m.role == Role::Agent))
            .map(|m| m.usage.cost_cents)
            .unwrap_or_default();
        self.state.add_spend(&store::today(), cents);
    }

    // ---------- keyboard ----------

    /// Ctrl+Tab and friends: the next or previous tab, coming round at the ends.
    pub(crate) fn step_tab(&mut self, step: isize) {
        let at = self.selected.filter(|_| self.view == View::Session);
        if let Some(id) = step_in(&self.tabs, at.or(self.selected), step, at.is_some()) {
            self.select(id);
        }
    }

    pub(crate) fn toggle_panel(&mut self) {
        self.show_panel = !self.show_panel;
        self.panel_auto_hidden = false;
        self.save();
    }

    /// Put the cursor in the open session's message box, opening Sessions if needed.
    pub(crate) fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current().is_none() {
            return;
        }
        if self.view != View::Session {
            self.view = View::Session;
        }
        self.prompt.update(cx, |p, cx| p.focus(window, cx));
    }

    /// Shift+F10: the right-click menu of the highlighted or open session, from the keyboard.
    pub(crate) fn keyboard_context_menu(&mut self, window: &Window) {
        let target = match (self.cursor, self.selected) {
            (Some(id), _) => ctxmenu::CtxTarget::Session(id),
            (None, Some(id)) if self.view == View::Session => ctxmenu::CtxTarget::Tab(id),
            (None, Some(id)) => ctxmenu::CtxTarget::Session(id),
            _ => return,
        };
        let size = window.viewport_size();
        let at = point(size.width / 2. - px(110.), px(120.));
        self.ctx_menu = Some((at, target));
        self.ctx_index = Some(0);
    }

    /// Esc twice in a terminal or agent CLI hands the keyboard back to Runo; the first Esc
    /// still reaches the program. Returns whether the key was used.
    pub(crate) fn leave_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !(self.term_focus.is_focused(window) || self.cli_focus.is_focused(window)) {
            self.term_escape = None;
            return false;
        }
        let now = std::time::Instant::now();
        match self.term_escape.take() {
            Some(at) if now.duration_since(at) < Duration::from_millis(600) => {
                window.focus(&self.focus, cx);
                self.info = Some((
                    "Left the terminal. Shortcuts work again; click it to type there.".into(),
                    now,
                ));
                cx.notify();
                true
            }
            _ => {
                self.term_escape = Some(now);
                false
            }
        }
    }

    /// The rows the arrow keys move through on this screen, top to bottom.
    pub(crate) fn nav_list(&self, cx: &App) -> Vec<u64> {
        match self.view {
            View::Inbox => self.needs_you().into_iter().map(|(s, _)| s.id).collect(),
            View::Home => {
                let mut out = Vec::new();
                for bucket in [Bucket::NeedsYou, Bucket::Running, Bucket::Review] {
                    out.extend(
                        self.state
                            .sessions
                            .iter()
                            .filter(|s| self.on_home(s) && self.bucket(s) == Some(bucket))
                            .map(|s| s.id),
                    );
                }
                out
            }
            _ if self.mode() == Mode::Sessions && self.show_sidebar => {
                let filter = self.search.read(cx).value().to_lowercase();
                self.sidebar_rows(&filter, cx)
                    .into_iter()
                    .flat_map(|(_, rows)| rows.into_iter().map(|s| s.id))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Arrow keys, Enter and the menu key on lists, when no text box has the keyboard.
    /// Returns whether the key was used.
    pub(crate) fn list_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ctx_menu.is_some() {
            return self.ctx_key(&keystroke.key, window, cx);
        }
        if !self.focus.is_focused(window)
            || self.palette.is_some()
            || self.tour.is_some()
            || keystroke.modifiers.modified()
        {
            return false;
        }
        match keystroke.key.as_str() {
            "up" | "down" => {
                let list = self.nav_list(cx);
                let at = self.cursor.or(self.selected.filter(|id| list.contains(id)));
                let step = if keystroke.key == "up" { -1 } else { 1 };
                self.cursor = step_in(&list, at, step, false);
                self.cursor.is_some()
            }
            "enter" => match self.cursor.take() {
                Some(id) => {
                    self.select(id);
                    true
                }
                None => false,
            },
            "menu" => {
                self.keyboard_context_menu(window);
                true
            }
            _ => false,
        }
    }

    // ---------- mode rail ----------

    fn rail_item(&self, mode: Mode, badge: Option<u32>, cx: &mut Context<Self>) -> AnyElement {
        let on = self.mode() == mode;
        let name = match self.scoped_project() {
            Some(p) if mode.per_project() => format!("{} · {}", mode.label(), p.name),
            _ => mode.label().to_string(),
        };
        let tip = match keymap::label(cx, mode.shortcut()) {
            keys if keys.is_empty() => name,
            keys => format!("{name}  {keys}"),
        };
        div()
            .id(SharedString::from(format!("rail-{}", mode.label())))
            .relative()
            .size(px(38.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .cursor_pointer()
            .when(on, |el| el.bg(tint(c_active())))
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.go(mode, window, cx);
                cx.notify();
            }))
            .child(icon(
                mode.glyph(),
                18.,
                if on { c_text() } else { c_muted() },
            ))
            .when(on, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(-7.))
                        .top(px(10.))
                        .w(px(3.))
                        .h(px(18.))
                        .rounded_full()
                        .bg(rgb(c_blue())),
                )
            })
            .when_some(badge.filter(|_| !on), |el, color| {
                el.child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .right(px(6.))
                        .size(px(7.))
                        .rounded_full()
                        .bg(rgb(color)),
                )
            })
            .into_any_element()
    }

    pub(crate) fn mode_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self.counts();
        let mut top = div()
            .v_flex()
            .items_center()
            .gap_1()
            .relative()
            .child(tour::anchor(&self.anchors, "nav"));
        for mode in Mode::MAIN {
            // Amber: something there needs you. Blue: agents are working, or work waits
            // to be reviewed.
            let badge = match mode {
                Mode::Sessions if counts.needs_you > 0 => Some(c_amber()),
                Mode::Sessions if counts.running > 0 => Some(c_blue()),
                Mode::Review if counts.review > 0 => Some(c_blue()),
                Mode::GitHub if self.gh.needs_you() > 0 => Some(c_amber()),
                _ => None,
            };
            top = top.child(self.rail_item(mode, badge, cx));
        }
        top = top
            .child(div().my_1().w(px(22.)).h(px(1.)).bg(divider()))
            .child(self.project_group(cx));
        // The Inbox opens from the bell in the top bar, so the rail doesn't repeat it.
        div()
            .w(px(RAIL_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .items_center()
            .py_2()
            .border_r_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .child(top)
            .child(div().flex_1())
            .child(
                div()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .child(self.rail_item(Mode::Notes, None, cx))
                    .child(
                        div()
                            .relative()
                            .child(tour::anchor(&self.anchors, "settings"))
                            .child(self.rail_item(Mode::Settings, None, cx)),
                    ),
            )
    }

    /// Design and Architecture under the badge of the project they show, so they read as
    /// that project's pages. The badge opens the project switcher.
    fn project_group(&self, cx: &mut Context<Self>) -> AnyElement {
        let project = self.scoped_project();
        let tip: SharedString = match project {
            Some(p) => format!("{}: switch project", p.name).into(),
            None => "All projects: switch project".into(),
        };
        let badge = div()
            .id("rail-project")
            .size(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_active())))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| {
                this.menu = Some("project-switch".into());
                cx.notify();
            }))
            .child(match project {
                Some(p) => project_mark(p, 20.).into_any_element(),
                None => icon(IconName::LayoutGrid, 15., c_muted()).into_any_element(),
            });
        let mut group = div()
            .v_flex()
            .items_center()
            .gap_1()
            .py(px(3.))
            .w(px(44.))
            .rounded_xl()
            .bg(tint(c_subtle()))
            .child(badge);
        for mode in Mode::PLAN {
            group = group.child(self.rail_item(mode, None, cx));
        }
        group.into_any_element()
    }

    /// The project the per-project modes show: none under All projects.
    pub(crate) fn scoped_project(&self) -> Option<&Project> {
        if self.all_scope() {
            return None;
        }
        self.current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
    }

    /// All projects is picked and the page on screen has a view of every project: Home's
    /// lists, or the project chooser on Design and Architecture.
    pub(crate) fn all_scope(&self) -> bool {
        self.home_all && (self.mode() == Mode::Home || self.mode().per_project())
    }

    // ---------- top bar ----------

    fn bar_pill(&self, id: &'static str) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(26.))
            .px(px(9.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .rounded_full()
            .border_1()
            .border_color(rgb(c_border()))
            .text_size(px(12.))
            .text_color(rgb(c_secondary()))
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
    }

    /// The project switcher's list, opened under its chip.
    pub(crate) fn project_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.current_project();
        let mut list = div().v_flex().p_1().child(
            div()
                .id("project-all")
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(13.))
                .hover(|el| el.bg(tint(c_active())))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.menu = None;
                    this.home_all = true;
                    // Design and Architecture stay put and offer every project to pick.
                    if this.mode().per_project() {
                        this.save();
                    } else {
                        this.go(Mode::Home, window, cx);
                    }
                    cx.notify();
                }))
                .child(icon(IconName::LayoutGrid, 15., c_secondary()))
                .child(div().flex_1().child("All projects"))
                .when(self.all_scope(), |el| {
                    el.child(icon(IconName::Check, 13., c_text()))
                }),
        );
        for project in &self.state.projects {
            let pid = project.id;
            let missing = !self.demo && !std::path::Path::new(&project.path).is_dir();
            let sessions = self
                .state
                .sessions
                .iter()
                .filter(|s| s.project_id == pid && !s.archived && s.canvas.is_empty())
                .count();
            list = list.child(
                div()
                    .id(("project-pick", pid as usize))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        this.switch_project(pid, window, cx);
                        cx.notify();
                    }))
                    .child(project_mark(project, 18.))
                    .child(div().flex_1().truncate().child(project.name.clone()))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(if missing { c_amber() } else { c_muted() }))
                            .child(match sessions {
                                _ if missing => "folder missing".into(),
                                0 => String::new(),
                                1 => "1 session".into(),
                                n => format!("{n} sessions"),
                            }),
                    )
                    .when(current == Some(pid) && !self.all_scope(), |el| {
                        el.child(icon(IconName::Check, 13., c_text()))
                    }),
            );
        }
        list = list
            .child(div().h(px(1.)).my_1().bg(rgb(c_border())))
            .child(
                div()
                    .id("project-add")
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        this.pick_project(cx);
                    }))
                    .child(icon(IconName::FolderPlus, 14., c_muted()))
                    .child("New project…"),
            );
        div()
            .absolute()
            .top(px(34.))
            .left_0()
            .child(
                deferred(
                    anchored().snap_to_window_with_margin(px(8.)).child(
                        div()
                            .id("project-menu")
                            .w(px(280.))
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .rounded_lg()
                            .border_1()
                            .border_color(float_edge())
                            .bg(float(c_raised()))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            }))
                            .child(list),
                    ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    /// Switch the top bar to a project. Sessions opens its latest session; every other
    /// mode stays where it is and shows that project. A project without sessions lands on
    /// its Home with the New session form open.
    pub(crate) fn switch_project(&mut self, pid: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.project = Some(pid);
        self.new_project = Some(pid);
        self.home_all = false;
        self.cursor = None;
        if !self.expanded.contains(&pid) {
            self.expanded.push(pid);
        }
        let latest = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid && !s.archived && s.canvas.is_empty())
            .map(|s| s.id)
            .max();
        match (self.mode(), latest) {
            (Mode::GitHub, _) => self.load_github(),
            (Mode::Sessions, Some(id)) => self.select(id),
            (Mode::Sessions, None) => {
                self.view = View::Home;
                self.open_new_session();
            }
            _ => {}
        }
        self.save();
        window.focus(&self.focus, cx);
    }

    /// Where the open session's agent runs, as the top bar and status bar show it: the
    /// name and a dot that is green when reachable, amber while unknown, red when not.
    pub(crate) fn runs_on(&self) -> (String, u32, String) {
        let conn = match self.current() {
            Some(s) => s.connection,
            None => self
                .current_project()
                .map(|pid| self.project_connection(pid))
                .unwrap_or(connections::LOCAL),
        };
        if conn == connections::LOCAL {
            return (
                "Local".into(),
                c_green(),
                "Agents run on this computer".into(),
            );
        }
        let Some(c) = connections::get(conn) else {
            return (
                "Removed connection".into(),
                c_red(),
                "This session's connection was removed. Move it somewhere else.".into(),
            );
        };
        let (color, state) = match connections::test_result(conn) {
            Some((Some(true), _)) => (c_green(), "reachable at the last test"),
            Some((Some(false), _)) => (c_red(), "failed its last test"),
            Some((None, _)) => (c_amber(), "being tested"),
            None => (c_amber(), "not tested yet"),
        };
        let tip = format!("Runs on {}, {state}. Click to move it.", c.name);
        (c.name, color, tip)
    }

    /// The popover under the Agents pill: what runs now, to open or stop.
    fn agents_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().v_flex().p_1();
        let running: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| self.is_running(s))
            .collect();
        if running.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child("No agents are working right now."),
            );
        }
        for s in running {
            let id = s.id;
            let provider = agent::provider(&s.provider);
            let when = self
                .started
                .get(&id)
                .map_or("Working".to_string(), |t| elapsed(t.elapsed()));
            list = list.child(
                div()
                    .id(("agents-row", id as usize))
                    .h(px(34.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.select(id);
                        cx.notify();
                    }))
                    .child(self.provider_mark(provider, 13.))
                    .child(div().flex_1().min_w_0().truncate().child(s.title.clone()))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(when),
                    )
                    .when(self.handles.contains_key(&Target::Session(id)), |el| {
                        el.child(
                            self.ghost_button_sized(
                                ("agents-stop", id as usize),
                                IconName::CircleStop,
                                "Stop",
                                24.,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.stop_session(id);
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            );
        }
        self.popover("agents-menu", 320., list, cx)
    }

    /// A floating list under a top-bar control that closes when clicked outside.
    fn popover(
        &self,
        id: &'static str,
        width: f32,
        list: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .absolute()
            .top(px(34.))
            .left_0()
            .child(
                deferred(
                    anchored().snap_to_window_with_margin(px(8.)).child(
                        div()
                            .id(id)
                            .w(px(width))
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .rounded_lg()
                            .border_1()
                            .border_color(float_edge())
                            .bg(float(c_raised()))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            }))
                            .child(list),
                    ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    pub(crate) fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self.counts();
        let inbox = counts.needs_you + self.gh.needs_you();
        let lead = if cfg!(target_os = "macos") { 80. } else { 8. };
        let project = self.scoped_project();
        let mode = self.mode();
        // The branch and session crumb belong to the open session, when it is this project's.
        let session = self
            .current()
            .filter(|s| mode == Mode::Sessions && Some(s.project_id) == project.map(|p| p.id));
        let branch = session
            .map(|s| (s.id, s.branch.clone()))
            .filter(|(_, b)| !b.is_empty());
        let crumb = session
            .filter(|_| self.view == View::Session)
            .map(|s| s.title.clone());
        let menu_open = self.menu.as_deref() == Some("project-switch");
        let agents_open = self.menu.as_deref() == Some("agents");
        let (where_name, where_color, where_tip) = self.runs_on();
        let where_session = self.current().map(|s| s.id);
        div()
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(lead))
            .pr_2()
            .border_b_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .window_control_area(WindowControlArea::Drag)
            // Three zones: the two sides share what the search box leaves, so the box
            // sits in the middle of the window on every page instead of moving with the
            // breadcrumb's length.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .overflow_hidden()
                    .when(mode == Mode::Sessions, |el| {
                        el.child(
                            self.ghost_button("toggle-sidebar", IconName::PanelLeft, "Toggle sidebar")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_sidebar = !this.show_sidebar;
                                    this.save();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .child(
                                div()
                                    .id("project-switch")
                                    .h(px(30.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .when(menu_open, |el| el.bg(tint(c_active())))
                                    .hover(|el| el.bg(tint(c_subtle())))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.menu = if this.menu.as_deref() == Some("project-switch") {
                                            None
                                        } else {
                                            Some("project-switch".into())
                                        };
                                        cx.notify();
                                    }))
                                    .when_some(project, |el, p| el.child(project_mark(p, 20.)))
                                    .when(self.all_scope(), |el| {
                                        el.child(icon(IconName::LayoutGrid, 15., c_secondary()))
                                    })
                                    .child(div().max_w(px(200.)).truncate().child(match project {
                                        Some(p) => p.name.clone(),
                                        None if self.all_scope() => "All projects".to_string(),
                                        None => "No project".to_string(),
                                    }))
                                    .child(icon(IconName::ChevronDown, 12., c_muted())),
                            )
                            .when(menu_open, |el| el.child(self.project_menu(cx))),
                    )
                    .when_some(branch, |el, (sid, branch)| {
                        el.child(
                            div()
                                .id("top-branch")
                                .h(px(24.))
                                .px(px(7.))
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(5.))
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(c_border()))
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .cursor_pointer()
                                .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                                .tooltip(|window, cx| {
                                    Tooltip::new("Branch: changes, terminal, worktrees").build(window, cx)
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    self.on_context(ctxmenu::CtxTarget::Branch(sid), cx),
                                )
                                .child(icon(IconName::GitBranch, 12., c_muted()))
                                .child(div().max_w(px(180.)).truncate().child(branch)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_shrink(1.)
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .child(icon(IconName::ChevronRight, 12., c_muted()))
                            .child(
                                div()
                                    .id("crumb-mode")
                                    .flex_none()
                                    .px_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        // The mode's own top page: Sessions lists every session.
                                        if mode == Mode::Sessions {
                                            this.view = View::AllSessions;
                                            this.save();
                                        } else {
                                            this.go(mode, window, cx);
                                        }
                                        cx.notify();
                                    }))
                                    .child(mode.label()),
                            )
                            .when_some(crumb, |el, title| {
                                el.child(icon(IconName::ChevronRight, 12., c_muted())).child(
                                    div()
                                        .id("crumb-session")
                                        .min_w_0()
                                        .max_w(px(220.))
                                        .truncate()
                                        .px_1()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .text_color(rgb(c_text()))
                                        .hover(|el| el.bg(tint(c_subtle())))
                                        .tooltip(|window, cx| {
                                            Tooltip::new("Rename this session").build(window, cx)
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.start_rename(window, cx);
                                            cx.notify();
                                        }))
                                        .child(title),
                                )
                            }),
                    )
            )
            .child(
                div().flex_shrink(1.).min_w(px(160.)).flex().justify_center().child(
                    div()
                        .id("top-search")
                        .relative()
                        .child(tour::anchor(&self.anchors, "palette"))
                        .w(px(420.))
                        .min_w(px(160.))
                        .flex_shrink(1.)
                        .h(px(28.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .bg(tint(c_subtle()))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .hover(|el| el.border_color(rgb(c_secondary())))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_palette(window, cx);
                            cx.notify();
                        }))
                        .child(icon(IconName::Search, 13., c_muted()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child("Search sessions, projects, files, commands"),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px(px(5.))
                                .rounded_sm()
                                .border_1()
                                .border_color(rgb(c_border()))
                                .text_size(px(11.))
                                .child(keymap::label(cx, "quick-open")),
                        ),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .relative()
                            .child(
                                self.bar_pill("top-running")
                                    .when(agents_open, |el| el.bg(tint(c_active())))
                                    .tooltip(|window, cx| {
                                        Tooltip::new("Agents working now: open or stop them")
                                            .build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.menu = if this.menu.as_deref() == Some("agents") {
                                            None
                                        } else {
                                            Some("agents".into())
                                        };
                                        cx.notify();
                                    }))
                                    .child(if counts.running > 0 {
                                        self.status_dot(Status::Working, "top-run-dot", 7.)
                                    } else {
                                        div()
                                            .size(px(7.))
                                            .rounded_full()
                                            .bg(rgb(c_muted()))
                                            .into_any_element()
                                    })
                                    .child(match counts.running {
                                        0 => "No agents running".to_string(),
                                        1 => "1 agent running".to_string(),
                                        n => format!("{n} agents running"),
                                    }),
                            )
                            .when(agents_open, |el| el.child(self.agents_menu(cx))),
                    )
                    .when(counts.spend_cents > 0.0, |el| {
                        let total = counts.spend_cents;
                        el.child(
                            self.bar_pill("top-spend")
                                .cursor_default()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(format!(
                                        "Spend reported by the agents today. ${:.2} across every session. {}",
                                        total / 100.0,
                                        agent::cost_note()
                                    ))
                                    .build(window, cx)
                                })
                                .child(format!("${:.2} today", counts.spend_today / 100.0)),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .child(
                                self.ghost_button("top-inbox", IconName::Bell, "Inbox")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.go(Mode::Inbox, window, cx);
                                        cx.notify();
                                    })),
                            )
                            .when(inbox > 0, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .top(px(1.))
                                        .right(px(0.))
                                        .min_w(px(14.))
                                        .h(px(14.))
                                        .px(px(3.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(rgb(c_amber()))
                                        .text_size(px(9.5))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0xffffff))
                                        .child(inbox.to_string()),
                                )
                            }),
                    )
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
                                        this.toggle_panel();
                                        cx.notify();
                                    },
                                )),
                            )
                        },
                    )
                    .child(
                        self.bar_pill("top-where")
                            .text_color(rgb(c_text()))
                            .tooltip(move |window, cx| {
                                Tooltip::new(where_tip.clone()).build(window, cx)
                            })
                            .map(|el| match where_session {
                                Some(sid) => el.on_mouse_down(
                                    MouseButton::Left,
                                    self.on_context(ctxmenu::CtxTarget::RunsOn(sid), cx),
                                ),
                                None => el.on_click(cx.listener(|this, _, _, cx| {
                                    this.open_settings();
                                    this.settings_page = SettingsPage::Connections;
                                    cx.notify();
                                })),
                            })
                            .child(div().size(px(7.)).rounded_full().bg(rgb(where_color)))
                            .child(where_name),
                    ),
            )
    }

    /// The list of every open tab, from the strip's overflow button.
    fn tabs_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().v_flex().p_1();
        for &id in &self.tabs {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let active = self.selected == Some(id) && self.view == View::Session;
            list = list.child(
                div()
                    .id(("tabs-row", id as usize))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(active, |el| el.bg(tint(c_active())))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
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
                        self.ghost_button_sized(
                            ("tabs-x", id as usize),
                            IconName::X,
                            "Close tab",
                            20.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close_tab(id);
                            cx.notify();
                        })),
                    ),
            );
        }
        div()
            .absolute()
            .top(px(30.))
            .right_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id("tabs-menu")
                                .w(px(300.))
                                .max_h(px(400.))
                                .overflow_y_scroll()
                                .rounded_lg()
                                .border_1()
                                .border_color(float_edge())
                                .bg(float(c_raised()))
                                .shadow_lg()
                                .occlude()
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.menu = None;
                                    cx.notify();
                                }))
                                .child(list),
                        ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    /// Open sessions as tabs, over the session they belong to. The strip scrolls when the
    /// tabs don't fit, the button at its end lists them all, and tabs can be dragged into
    /// a new order.
    pub(crate) fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs = div()
            .id("tab-row")
            .flex()
            .items_end()
            .h_full()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll);
        for &id in &self.tabs {
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = agent::provider(&session.provider);
            let active =
                self.selected == Some(id) && self.view == View::Session && !self.browser_main;
            let working = self.handles.contains_key(&Target::Session(id));
            let dot = match self.need(session) {
                _ if working => Some(c_blue()),
                Some(Need::Blocked) => Some(c_amber()),
                Some(Need::Failed) => Some(c_red()),
                Some(Need::Finished) => Some(c_blue()),
                None => None,
            };
            let dragged = DraggedTab {
                id,
                title: session.title.clone().into(),
            };
            tabs = tabs.child(tab_slot(
                active,
                div()
                    .id(("tab", id as usize))
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Tab(id), cx),
                    )
                    .on_drag(dragged, |tab, _, _, cx| cx.new(|_| tab.clone()))
                    .drag_over::<DraggedTab>(|el, _, _, _| el.border_color(rgb(c_blue())))
                    .on_drop(cx.listener(move |this, tab: &DraggedTab, _, cx| {
                        move_tab(&mut this.tabs, tab.id, id);
                        this.save();
                        cx.notify();
                    }))
                    .group("tab")
                    .flex_none()
                    .w(px(180.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .map(|el| tab_shape(el, active))
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
                            .child(match dot {
                                Some(color) => div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(rgb(color))
                                    .group_hover("tab", |el| el.opacity(0.))
                                    .into_any_element(),
                                None => icon(IconName::X, 11., c_muted()).into_any_element(),
                            }),
                    ),
            ));
        }
        if self.browser_open {
            let active = self.browser_main && self.view == View::Session;
            tabs = tabs.child(tab_slot(
                active,
                div()
                    .id("tab-browser")
                    .group("tab")
                    .flex_none()
                    .w(px(140.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .map(|el| tab_shape(el, active))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_browser();
                        cx.notify();
                    }))
                    .child(icon(IconName::Globe, 12., c_secondary()))
                    .child(div().flex_1().min_w_0().truncate().child("Browser"))
                    .child(
                        div()
                            .id("tab-browser-x")
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .hover(|el| el.bg(rgb(c_border())))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.browser_open = false;
                                this.browser_main = false;
                                cx.notify();
                            }))
                            .child(icon(IconName::X, 11., c_muted())),
                    ),
            ));
        }
        let (added, removed) = self
            .changes
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
        let show_stats = self.view == View::Session && !self.changes.is_empty();
        let menu_open = self.menu.as_deref() == Some("tabs");
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_end()
            .bg(pane(c_sidebar()))
            // Every part of the strip draws its own bottom edge except the open tab, which
            // runs straight into the page below.
            .child(strip_edge(div().w(px(8.))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_end()
                    .child(tabs.relative().child(tour::anchor(&self.anchors, "tabs")))
                    .child(strip_edge(div().flex_1())),
            )
            .when(show_stats, |el| {
                el.child(
                    strip_edge(div())
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .text_size(px(12.))
                        .font_family(self.mono_font.clone())
                        .child(div().text_color(rgb(c_add())).child(format!("+{added}")))
                        .child(div().text_color(rgb(c_del())).child(format!("−{removed}"))),
                )
            })
            .when(self.tabs.len() > 1, |el| {
                el.child(
                    strip_edge(div())
                        .flex()
                        .items_center()
                        .relative()
                        .child(
                            self.ghost_button_sized(
                                "tabs-all",
                                IconName::ChevronDown,
                                "All open tabs",
                                26.,
                            )
                            .when(menu_open, |el| el.bg(tint(c_active())))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.menu = if this.menu.as_deref() == Some("tabs") {
                                    None
                                } else {
                                    Some("tabs".into())
                                };
                                cx.notify();
                            })),
                        )
                        .when(menu_open, |el| el.child(self.tabs_menu(cx))),
                )
            })
            .child(strip_edge(div().w(px(8.))))
    }

    // ---------- Home ----------

    fn stat_card(&self, label: &'static str, value: String, hint: String, color: u32) -> Div {
        div()
            .flex_1()
            .min_w_0()
            .p_4()
            .v_flex()
            .gap_1()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_subtle()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(div().size(px(7.)).rounded_full().bg(rgb(color)))
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(24.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(value),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .truncate()
                    .child(hint),
            )
    }

    fn home_row(&self, session: &Session, note: String, cx: &mut Context<Self>) -> AnyElement {
        let id = session.id;
        let provider = agent::provider(&session.provider);
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        div()
            .id(("home-row", id as usize))
            .px_3()
            .py(px(9.))
            .flex()
            .items_center()
            .gap_3()
            .border_t_1()
            .border_color(rgb(c_border()))
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())))
            .when(self.cursor == Some(id), |el| el.bg(tint(c_selected())))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::Session(id), cx),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                // Work waiting for review opens on Review, where it can be shipped.
                let waiting = this
                    .state
                    .session(id)
                    .is_some_and(|s| this.bucket(s) == Some(Bucket::Review));
                if waiting {
                    this.review(id);
                } else {
                    this.select(id);
                }
                cx.notify();
            }))
            .child(self.provider_mark(provider, 14.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(format!("{project}  ·  {}", session.branch)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(note),
            )
            .into_any_element()
    }

    fn home_panel(
        &self,
        title: &'static str,
        count: usize,
        rows: Vec<AnyElement>,
        empty: &'static str,
    ) -> Div {
        div()
            .flex_1()
            .min_w_0()
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .overflow_hidden()
            .child(
                div()
                    .px_3()
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(count.to_string()),
                    ),
            )
            .children(rows)
            .when(count == 0, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_4()
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child(empty),
                )
            })
    }

    /// A small bordered button with an icon, for Home's actions.
    pub(crate) fn home_button(
        &self,
        id: &'static str,
        glyph: IconName,
        label: &'static str,
        hint: String,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .h(px(32.))
            .px_3()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()))
            .cursor_pointer()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .hover(|el| el.bg(tint(c_subtle())))
            .child(icon(glyph, 14., c_text()))
            .child(label)
            .when(!hint.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(hint),
                )
            })
    }

    /// Home: what needs you, what runs and what waits for review, in the top bar's project
    /// or, with "All projects", everywhere.
    pub(crate) fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut needs: Vec<AnyElement> = Vec::new();
        let mut running: Vec<AnyElement> = Vec::new();
        let mut review: Vec<AnyElement> = Vec::new();
        for s in self.state.sessions.iter().filter(|s| self.on_home(s)) {
            match self.bucket(s) {
                Some(Bucket::NeedsYou) => {
                    let note = self.need(s).map_or("", Need::label);
                    needs.push(self.home_row(s, note.into(), cx));
                }
                Some(Bucket::Running) => {
                    let note = self
                        .started
                        .get(&s.id)
                        .map_or("Working".to_string(), |t| elapsed(t.elapsed()));
                    running.push(self.home_row(s, note, cx));
                }
                Some(Bucket::Review) => {
                    let (_, a, r) = self.stats.get(&s.id).copied().unwrap_or_default();
                    review.push(self.home_row(s, format!("+{a} −{r}"), cx));
                }
                _ => {}
            }
        }
        let (n_needs, n_running, n_review) = (needs.len(), running.len(), review.len());
        let spend_today = self.counts().spend_today;
        let project = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .filter(|_| !self.home_all)
            .cloned();
        let missing = project
            .as_ref()
            .filter(|p| !self.demo && !std::path::Path::new(&p.path).is_dir())
            .map(|p| (p.id, p.path.clone()));
        let no_projects = self.state.projects.is_empty();
        let scope = |label: &'static str, all: bool| {
            let on = self.home_all == all;
            div()
                .id(if all { "home-all" } else { "home-one" })
                .h(px(26.))
                .px_3()
                .flex()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                .when(!on, |el| {
                    el.text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.home_all = all;
                    this.cursor = None;
                    this.save();
                    cx.notify();
                }))
                .child(label)
        };
        let links: [(&'static str, IconName, &'static str, View); 4] = [
            (
                "home-history",
                IconName::RotateCcwClock,
                "Agent history",
                View::History,
            ),
            (
                "home-activity",
                IconName::Activity,
                "Timeline",
                View::Activity,
            ),
            (
                "home-notes",
                IconName::NotebookPen,
                "Project notes",
                View::Notes,
            ),
            (
                "home-isolated",
                IconName::GitBranch,
                "Isolated sessions",
                View::Worktrees,
            ),
        ];
        let mut link_row = div().flex().flex_wrap().gap_2();
        for (id, glyph, label, view) in links {
            link_row = link_row.child(
                self.home_button(id, glyph, label, String::new())
                    .font_weight(FontWeight::NORMAL)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        match view {
                            View::History => this.open_history(cx),
                            View::Notes => {
                                this.notes_project = this.current_project();
                                this.open_notes(window, cx);
                            }
                            _ => this.view = view,
                        }
                        cx.notify();
                    })),
            );
        }
        let body = div()
            .v_flex()
            .gap_4()
            .when_some(missing, |el, (pid, path)| {
                el.child(
                    div()
                        .p_4()
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(c_amber()))
                        .bg(rgb(settings::mix(c_amber(), c_canvas(), 0.1)))
                        .child(icon(IconName::TriangleAlert, 18., c_amber()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .v_flex()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("This project's folder is missing"),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(rgb(c_secondary()))
                                        .child(format!(
                                            "{path} was moved or deleted. Point the project at its new place, or remove it from Runo."
                                        )),
                                ),
                        )
                        .child(
                            self.button("home-relink", "Relink…", true)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.relink_project(pid, cx);
                                })),
                        )
                        .child(
                            self.button("home-remove", "Remove…", false)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.ask_remove_project(pid, window, cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.stat_card(
                        "Needs you",
                        n_needs.to_string(),
                        "blocked, or finished unseen".into(),
                        c_amber(),
                    ))
                    .child(self.stat_card(
                        "Running",
                        n_running.to_string(),
                        "agents working now".into(),
                        c_blue(),
                    ))
                    .child(self.stat_card(
                        "Ready to review",
                        n_review.to_string(),
                        "idle with changes".into(),
                        c_green(),
                    ))
                    .child(self.stat_card(
                        "Spend today",
                        format!("${:.2}", spend_today / 100.0),
                        "reported by Claude Code and OpenCode, all projects".into(),
                        c_muted(),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_start()
                    .child(self.home_panel(
                        "Needs you",
                        n_needs,
                        needs,
                        "Nothing is waiting on you.",
                    ))
                    .child(self.home_panel(
                        "Running",
                        n_running,
                        running,
                        "No agents are working right now.",
                    )),
            )
            .child(self.home_panel(
                "Ready to review",
                n_review,
                review,
                "Sessions with changes to commit show up here.",
            ))
            .child(link_row);
        let title = match &project {
            Some(p) => p.name.clone(),
            None if no_projects => "Welcome to Runo".into(),
            None => "All projects".into(),
        };
        let subtitle = match &project {
            _ if no_projects => "Add a project folder to get started.".to_string(),
            Some(p) => p.path.clone(),
            None => match self.state.projects.len() {
                1 => "1 project".to_string(),
                n => format!("{n} projects"),
            },
        };
        div()
            .flex_1()
            .min_w_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .max_w(px(1080.))
                    .mx_auto()
                    .p_8()
                    .v_flex()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(20.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(12.5))
                                            .text_color(rgb(c_muted()))
                                            .child(subtitle),
                                    ),
                            )
                            .when(self.state.projects.len() > 1, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .p(px(2.))
                                        .gap(px(2.))
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(rgb(c_border()))
                                        .child(scope("This project", false))
                                        .child(scope("All projects", true)),
                                )
                            })
                            .child(
                                self.home_button(
                                    "home-add",
                                    IconName::FolderPlus,
                                    "Add project",
                                    String::new(),
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.pick_project(cx))),
                            )
                            .when(!no_projects, |el| {
                                el.child(
                                    self.home_button(
                                        "home-new",
                                        IconName::SquarePen,
                                        "New session",
                                        keymap::label(cx, "new-session"),
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            if let Some(pid) = this.current_project() {
                                                this.new_project = Some(pid);
                                            }
                                            this.open_new_session();
                                            cx.notify();
                                        },
                                    )),
                                )
                            }),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    // ---------- Design and Architecture ----------

    /// Open the New session dialog for project `pid` with `task` filled in and `context`
    /// to go with it.
    pub(crate) fn start_with_context(
        &mut self,
        pid: u64,
        task: &str,
        context: StartContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_project = Some(pid);
        self.new_task
            .update(cx, |t, cx| t.set_value(task, window, cx));
        if context.in_place {
            self.new_isolated = false;
        }
        self.new_context = Some(context);
        self.open_new_session();
        cx.notify();
    }

    /// Give a just-created session the context it was started with: sent ahead of its
    /// first prompt, with the images attached when its agent reads them here.
    pub(crate) fn apply_context(&mut self, id: u64, context: &StartContext) {
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        session.handoff = if session.handoff.is_empty() {
            context.text.clone()
        } else {
            format!("{}\n\n{}", session.handoff, context.text)
        };
        let images =
            session.connection == connections::LOCAL && agent::takes_images(&session.provider);
        if images && !context.images.is_empty() {
            self.launch_images.insert(id, context.images.clone());
        }
    }

    /// The context waiting on the New session dialog, with a way to drop it.
    pub(crate) fn context_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let context = self.new_context.as_ref()?;
        let images = context.images.len();
        Some(
            div()
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(tint(c_subtle()))
                .text_size(px(12.5))
                .child(icon(IconName::Paperclip, 13., c_secondary()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(format!("{} goes with this task", context.label)),
                )
                .when(images > 0, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(rgb(c_muted()))
                            .child(icon(IconName::Image, 12., c_muted()))
                            .child(images.to_string()),
                    )
                })
                .child(
                    self.ghost_button_sized("context-drop", IconName::X, "Start without it", 22.)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.new_context = None;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}

impl RunoApp {
    /// The project a Design or Architecture page shows, beside its title: a switcher with
    /// the same list as the top bar's.
    pub(crate) fn mode_project_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        let project = self.scoped_project();
        let open = self.menu.as_deref() == Some("mode-project");
        div()
            .relative()
            .min_w_0()
            .child(
                div()
                    .id("mode-project")
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .when(open, |el| el.bg(tint(c_active())))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = if this.menu.as_deref() == Some("mode-project") {
                            None
                        } else {
                            Some("mode-project".into())
                        };
                        cx.notify();
                    }))
                    .child(match project {
                        Some(p) => project_mark(p, 18.).into_any_element(),
                        None => icon(IconName::LayoutGrid, 14., c_secondary()).into_any_element(),
                    })
                    .child(div().max_w(px(260.)).truncate().child(match project {
                        Some(p) => p.name.clone(),
                        None => "All projects".to_string(),
                    }))
                    .child(icon(IconName::ChevronDown, 12., c_muted())),
            )
            .when(open, |el| el.child(self.project_menu(cx)))
            .into_any_element()
    }
}

impl RunoApp {
    /// Design and Architecture under All projects: every project with what it has, to
    /// pick one, instead of guessing which project is meant.
    pub(crate) fn project_chooser(&self, mode: Mode, cx: &mut Context<Self>) -> AnyElement {
        let mut grid = div().flex().flex_wrap().gap_3();
        for project in &self.state.projects {
            let pid = project.id;
            let root = std::path::Path::new(&project.path);
            let missing = !self.demo && !root.is_dir();
            let summary = if missing {
                "Folder missing".to_string()
            } else {
                chooser_summary(mode, root, self.arch.maps.get(&pid), self.demo)
            };
            let sessions = self
                .state
                .sessions
                .iter()
                .filter(|s| s.project_id == pid && !s.archived && s.canvas.is_empty())
                .count();
            grid = grid.child(
                div()
                    .id(("chooser-project", pid as usize))
                    .w(px(280.))
                    .p_4()
                    .v_flex()
                    .gap_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(pane(c_canvas()))
                    .cursor_pointer()
                    .hover(|el| el.border_color(rgb(c_muted())).bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_project(pid, window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(project_mark(project, 22.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(project.name.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(if missing { c_amber() } else { c_secondary() }))
                            .child(summary),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .truncate()
                            .child(match sessions {
                                0 => project.path.clone(),
                                1 => format!("1 session · {}", project.path),
                                n => format!("{n} sessions · {}", project.path),
                            }),
                    ),
            );
        }
        let (start, line) = if mode == Mode::Design {
            (
                "New design project",
                "Start from a brief, sketches or screenshots, with no code yet.",
            )
        } else {
            (
                "New architecture project",
                "Sketch the system before writing it.",
            )
        };
        grid = grid.child(
            div()
                .id("chooser-new")
                .w(px(280.))
                .p_4()
                .v_flex()
                .gap_2()
                .rounded_xl()
                .border_1()
                .border_dashed()
                .border_color(rgb(c_border()))
                .cursor_pointer()
                .text_color(rgb(c_secondary()))
                .hover(|el| el.border_color(rgb(c_muted())).text_color(rgb(c_text())))
                .on_click(cx.listener(move |this, _, _, cx| this.start_project(mode, cx)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(icon(IconName::Plus, 15., c_secondary()))
                        .child(start),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child(line),
                ),
        );
        grid.into_any_element()
    }
}

/// One line on a chooser card: what the project has for this mode.
fn chooser_summary(
    mode: Mode,
    root: &std::path::Path,
    map: Option<&architecture::ProjectMap>,
    demo: bool,
) -> String {
    if demo {
        return "Sample project".into();
    }
    if mode == Mode::Design {
        let refs = std::fs::read_dir(root.join(design::FOLDER))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| turns::is_image(&e.path()))
            .count();
        let notes = design::notes_path(root).is_file();
        match (refs, notes) {
            (0, false) => "No references or notes yet".into(),
            (0, true) => "Design notes written".into(),
            (1, n) => format!("1 reference{}", if n { " · notes written" } else { "" }),
            (r, n) => format!("{r} references{}", if n { " · notes written" } else { "" }),
        }
    } else {
        let notes = architecture::notes_path(root).is_file();
        let shape = map.map(|m| match m.packages.len() {
            0 => format!("{} files", m.files),
            1 => format!("1 package · {} files", m.files),
            n => format!("{n} packages · {} files", m.files),
        });
        match (shape, notes) {
            (Some(shape), true) => format!("{shape} · notes written"),
            (Some(shape), false) => shape,
            (None, true) => "Notes written".into(),
            (None, false) => "No notes yet".into(),
        }
    }
}

/// What a session started from Design or Architecture brings along: named on the New
/// session dialog, sent ahead of the task, with its images attached.
#[derive(Clone, Debug, Default)]
pub(crate) struct StartContext {
    pub label: String,
    pub text: String,
    pub images: Vec<String>,
    /// Run in the project folder rather than a worktree, so the edits show where they
    /// were asked for (the architecture canvas previews them).
    pub in_place: bool,
}

/// The Design and Architecture pages' frame: the mode's mark and title over the body.
pub(crate) fn mode_frame(
    mode: Mode,
    title: &str,
    project: Option<AnyElement>,
    subtitle: &str,
    body: impl IntoElement,
    action: Option<AnyElement>,
) -> AnyElement {
    let color = if mode == Mode::Design {
        c_green()
    } else {
        c_blue()
    };
    div()
        .flex_1()
        .min_w_0()
        .overflow_y_scrollbar()
        .child(
            div()
                .max_w(px(1480.))
                .mx_auto()
                .p_8()
                .v_flex()
                .gap_6()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .size(px(44.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(rgb(settings::mix(color, c_canvas(), 0.3)))
                                .bg(rgb(settings::mix(
                                    color,
                                    c_canvas(),
                                    if is_dark() { 0.2 } else { 0.1 },
                                )))
                                .child(icon(mode.glyph(), 21., color)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .v_flex()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_size(px(22.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(title.to_string()),
                                        )
                                        .children(project),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(rgb(c_secondary()))
                                        .child(subtitle.to_string()),
                                ),
                        )
                        .children(action),
                )
                .child(body),
        )
        .into_any_element()
}

/// A project's square badge: its icon on its colour, or its initial.
/// A piece of the tab strip that carries the strip's bottom edge.
fn strip_edge(el: Div) -> Div {
    el.h_full().border_b_1().border_color(divider())
}

/// Where a tab sits in the strip, with the gap after it. Only the open tab's slot has no
/// bottom edge.
fn tab_slot(active: bool, tab: Stateful<Div>) -> Div {
    let slot = div().h_full().flex_none().flex().items_end().child(tab);
    if active {
        slot.child(strip_edge(div().w(px(4.))))
    } else {
        strip_edge(slot.pr_1())
    }
}

/// A tab's outline: the open tab is the page's own colour with no bottom edge, so it
/// joins the page under the strip; the others float above the strip's edge.
fn tab_shape(el: Stateful<Div>, active: bool) -> Stateful<Div> {
    if active {
        el.h(px(32.))
            .rounded_t_md()
            .border_t_1()
            .border_l_1()
            .border_r_1()
            .border_color(divider())
            .bg(glass_fill(c_canvas()))
            .text_color(rgb(c_text()))
    } else {
        el.h(px(28.))
            .mb(px(3.))
            .rounded_md()
            .border_1()
            .border_color(transparent_black())
            .text_color(rgb(c_secondary()))
            .hover(|el| el.bg(tint(c_subtle())))
    }
}

pub(crate) fn project_mark(project: &Project, size: f32) -> Div {
    let color = projects::color_of(project).unwrap_or(c_secondary());
    let initial = project
        .name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.26))
        .bg(rgb(settings::mix(color, c_canvas(), 0.78)))
        .text_size(px(size * 0.55))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(color))
        .child(initial)
}

/// What a session asked from a canvas stores to say which: see `Session::canvas`.
pub(crate) fn canvas_name(view: View) -> &'static str {
    match view {
        View::Design => "design",
        _ => "architecture",
    }
}

/// The canvas a session was asked from, if it was.
pub(crate) fn canvas_of(s: &Session) -> Option<View> {
    match s.canvas.as_str() {
        "design" => Some(View::Design),
        "architecture" => Some(View::Architecture),
        _ => None,
    }
}

pub(crate) fn canvas_title(view: View) -> &'static str {
    match view {
        View::Design => "Design",
        _ => "Architecture",
    }
}

impl RunoApp {
    /// Open a session where it belongs: one asked from the Design or Architecture canvas
    /// opens that canvas, any other its chat.
    pub(crate) fn open_session(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pid, view)) = self
            .state
            .session(id)
            .and_then(|s| Some((s.project_id, canvas_of(s)?)))
        else {
            self.select(id);
            return;
        };
        self.menu = None;
        self.project = Some(pid);
        self.home_all = false;
        if let Some(session) = self.state.session_mut(id) {
            session.unread = false;
        }
        self.view = view;
        self.save();
        window.focus(&self.focus, cx);
    }

    /// The agent chats asked from a canvas of project `pid`, newest first.
    fn canvas_sessions(&self, pid: u64, view: View) -> Vec<&Session> {
        let name = canvas_name(view);
        let mut out: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid && s.canvas == name && !s.archived)
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.id));
        out
    }

    /// On a canvas's header: the chats of the agents asked from it, to read what they did
    /// and said, or carry on talking to one. Hidden until there is one.
    pub(crate) fn canvas_chats(
        &self,
        pid: u64,
        view: View,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let sessions = self.canvas_sessions(pid, view);
        if sessions.is_empty() {
            return None;
        }
        let open = self.menu.as_deref() == Some("canvas-chats");
        let busy = sessions.iter().any(|s| self.is_running(s));
        let mut list = div().v_flex().p_1().child(
            div()
                .px_2()
                .pt_1()
                .pb_2()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .whitespace_normal()
                .child(format!(
                    "Agents asked from the {} canvas. Open one to read its chat or ask it more.",
                    canvas_title(view)
                )),
        );
        for s in &sessions {
            let id = s.id;
            let p = agent::provider(&s.provider);
            let state = if self.is_running(s) {
                "Working".to_string()
            } else if self.need(s).is_some() {
                "Needs you".to_string()
            } else {
                let turns = s.messages.iter().filter(|m| m.role == Role::User).count();
                match turns {
                    1 => "1 ask".to_string(),
                    n => format!("{n} asks"),
                }
            };
            list = list.child(
                div()
                    .id(("canvas-chat", id as usize))
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.select(id);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_none()
                            .text_color(rgb(p.color))
                            .child(p.mark),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(div().truncate().text_size(px(13.)).child(s.title.clone()))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(format!("{} · {state}", p.name)),
                            ),
                    )
                    .when(s.unread, |el| {
                        el.child(div().size(px(7.)).rounded_full().bg(rgb(c_blue())))
                    }),
            );
        }
        Some(
            div()
                .relative()
                .flex_none()
                .child(
                    div()
                        .id("canvas-chats")
                        .h(px(30.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .when(open, |el| el.bg(tint(c_active())))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .tooltip(crate::archdraw::view::tip(
                            "The agents' chats behind this canvas",
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu = if this.menu.as_deref() == Some("canvas-chats") {
                                None
                            } else {
                                Some("canvas-chats".into())
                            };
                            cx.notify();
                        }))
                        .child(icon(IconName::MessagesSquare, 14., c_secondary()))
                        .child("Chats")
                        .child(
                            div()
                                .px(px(5.))
                                .rounded_full()
                                .bg(tint(c_active()))
                                .text_size(px(11.))
                                .text_color(rgb(if busy { c_blue() } else { c_muted() }))
                                .child(sessions.len().to_string()),
                        ),
                )
                .when(open, |el| {
                    el.child(
                        div().absolute().top(px(34.)).right_0().child(
                            deferred(
                                anchored().snap_to_window_with_margin(px(8.)).child(
                                    div()
                                        .id("canvas-chats-menu")
                                        .w(px(320.))
                                        .max_h(px(360.))
                                        .overflow_y_scroll()
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(float_edge())
                                        .bg(float(c_raised()))
                                        .shadow_lg()
                                        .occlude()
                                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                            this.menu = None;
                                            cx.notify();
                                        }))
                                        .child(list),
                                ),
                            )
                            .with_priority(2),
                        ),
                    )
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    use store::Message;

    #[test]
    fn only_design_and_architecture_show_one_project() {
        assert!(Mode::Design.per_project() && Mode::Architecture.per_project());
        assert!(Mode::MAIN.iter().all(|m| !m.per_project()));
    }

    #[test]
    fn every_rail_place_owns_its_views() {
        // Each place on the rail opens a view that lights that same place up again.
        for (view, mode) in [
            (View::Review, Mode::Review),
            (View::GitHub, Mode::GitHub),
            (View::History, Mode::History),
            (View::Notes, Mode::Notes),
            (View::Automations, Mode::Playbooks),
            (View::Session, Mode::Sessions),
        ] {
            assert_eq!(Mode::of(view), mode);
        }
        let rail: Vec<Mode> = Mode::MAIN
            .iter()
            .chain(Mode::PLAN.iter())
            .copied()
            .collect();
        for mode in &rail {
            assert_eq!(rail.iter().filter(|m| *m == mode).count(), 1);
            assert!(
                keymap::SHORTCUTS.iter().any(|s| s.id == mode.shortcut()),
                "{} has no shortcut",
                mode.label()
            );
        }
    }

    #[test]
    fn one_rule_for_needs_you() {
        let hidden = std::collections::HashSet::new();
        let mut s = Session {
            id: 7,
            messages: vec![Message::new(Role::Agent, "done")],
            ..Session::default()
        };
        assert_eq!(need_of(&s, false, &hidden), None);
        // Finished or failed unseen, until opened.
        s.unread = true;
        assert_eq!(need_of(&s, false, &hidden), Some(Need::Finished));
        s.status = Status::Failed;
        assert_eq!(need_of(&s, false, &hidden), Some(Need::Failed));
        s.unread = false;
        assert_eq!(need_of(&s, false, &hidden), None);
        // Blocked on a permission, even after it was opened, until dismissed.
        s.messages[0].denied = vec!["Bash".into()];
        assert_eq!(need_of(&s, false, &hidden), Some(Need::Blocked));
        assert_eq!(need_of(&s, true, &hidden), None);
        let dismissed: std::collections::HashSet<String> = ["s:7:0".to_string()].into();
        assert_eq!(need_of(&s, false, &dismissed), None);
        s.archived = true;
        assert_eq!(need_of(&s, false, &hidden), None);
    }

    #[test]
    fn arrows_step_through_lists() {
        let list = [4, 5, 6];
        assert_eq!(step_in(&list, None, 1, false), Some(4));
        assert_eq!(step_in(&list, None, -1, false), Some(6));
        assert_eq!(step_in(&list, Some(4), 1, false), Some(5));
        assert_eq!(step_in(&list, Some(6), 1, false), Some(6));
        assert_eq!(step_in(&list, Some(6), 1, true), Some(4));
        assert_eq!(step_in(&list, Some(4), -1, true), Some(6));
        assert_eq!(step_in(&list, Some(99), 1, true), Some(4));
        assert_eq!(step_in(&[], None, 1, true), None);
    }

    #[test]
    fn tabs_move_to_where_they_are_dropped() {
        let mut tabs = vec![1, 2, 3, 4];
        move_tab(&mut tabs, 4, 2);
        assert_eq!(tabs, vec![1, 4, 2, 3]);
        move_tab(&mut tabs, 1, 3);
        assert_eq!(tabs, vec![4, 2, 3, 1]);
        move_tab(&mut tabs, 9, 2);
        assert_eq!(tabs, vec![4, 2, 3, 1]);
    }
}
