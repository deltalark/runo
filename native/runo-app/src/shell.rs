//! The Workbench frame around every screen: the mode rail on the left, the top bar
//! with the project switcher, search and live counters, the tab strip over a session,
//! and the Home page. Design and Architecture have their places on the rail; their
//! pages say what each mode will hold until the modes themselves land.

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
    Playbooks,
    Reviews,
    Inbox,
    Settings,
}

impl Mode {
    /// The modes at the top of the rail, in order; ⌘1 to ⌘6 jump to them.
    pub(crate) const MAIN: [Mode; 6] = [
        Mode::Home,
        Mode::Sessions,
        Mode::Design,
        Mode::Architecture,
        Mode::Playbooks,
        Mode::Reviews,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Home => "Home",
            Mode::Sessions => "Sessions",
            Mode::Design => "Design",
            Mode::Architecture => "Architecture",
            Mode::Playbooks => "Playbooks",
            Mode::Reviews => "Reviews",
            Mode::Inbox => "Inbox",
            Mode::Settings => "Settings",
        }
    }

    fn glyph(self) -> IconName {
        match self {
            Mode::Home => IconName::House,
            Mode::Sessions => IconName::CodeXml,
            Mode::Design => IconName::PanelsTopLeft,
            Mode::Architecture => IconName::Network,
            Mode::Playbooks => IconName::BookOpen,
            Mode::Reviews => IconName::GitPullRequest,
            Mode::Inbox => IconName::Inbox,
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
            View::GitHub => Mode::Reviews,
            View::Inbox => Mode::Inbox,
            View::Settings => Mode::Settings,
            View::Session
            | View::NewSession
            | View::AllSessions
            | View::History
            | View::Worktrees
            | View::Notes
            | View::Activity
            | View::Editor => Mode::Sessions,
        }
    }
}

/// Where a session sits on Home.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bucket {
    NeedsYou,
    Running,
    Review,
}

/// Totals shown in the top bar and on Home.
#[derive(Default)]
pub(crate) struct Counts {
    pub running: usize,
    /// Sessions that finished unseen or failed.
    pub needs_you: usize,
    /// Idle sessions with uncommitted changes.
    pub review: usize,
    pub spend_cents: f64,
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
            Mode::Playbooks => self.view = View::Automations,
            Mode::Reviews => self.load_github(),
            Mode::Inbox => self.view = View::Inbox,
            Mode::Settings => self.open_settings(),
        }
        window.focus(&self.focus, cx);
    }

    /// Failed or unseen sessions need the person; running ones are left alone; idle ones
    /// with uncommitted changes wait for review.
    pub(crate) fn bucket(&self, s: &Session) -> Option<Bucket> {
        if s.archived {
            None
        } else if s.unread || s.status == Status::Failed {
            Some(Bucket::NeedsYou)
        } else if self.started.contains_key(&s.id) || s.status == Status::Working {
            Some(Bucket::Running)
        } else if self.stats.get(&s.id).is_some_and(|st| st.0 > 0) {
            Some(Bucket::Review)
        } else {
            None
        }
    }

    pub(crate) fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for s in &self.state.sessions {
            match self.bucket(s) {
                Some(Bucket::NeedsYou) => counts.needs_you += 1,
                Some(Bucket::Running) => counts.running += 1,
                Some(Bucket::Review) => counts.review += 1,
                None => {}
            }
        }
        counts.spend_cents = self
            .state
            .sessions
            .iter()
            .flat_map(|s| s.messages.iter())
            .map(|m| m.usage.cost_cents)
            .sum();
        counts
    }

    // ---------- mode rail ----------

    fn rail_item(&self, mode: Mode, badge: Option<u32>, cx: &mut Context<Self>) -> AnyElement {
        let on = self.mode() == mode;
        let tip = match Mode::MAIN.iter().position(|m| *m == mode) {
            Some(i) => format!("{}  {MOD_KEY}{}", mode.label(), i + 1),
            None => mode.label().to_string(),
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
            let badge = match mode {
                Mode::Sessions if counts.running > 0 => Some(c_blue()),
                Mode::Reviews if !self.gh.prs.is_empty() => Some(c_amber()),
                _ => None,
            };
            top = top.child(self.rail_item(mode, badge, cx));
        }
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
                    .child(self.rail_item(
                        Mode::Inbox,
                        (counts.needs_you > 0).then_some(c_blue()),
                        cx,
                    ))
                    .child(
                        div()
                            .relative()
                            .child(tour::anchor(&self.anchors, "settings"))
                            .child(self.rail_item(Mode::Settings, None, cx)),
                    ),
            )
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
    fn project_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.current_project();
        let mut list = div().v_flex().p_1();
        for project in &self.state.projects {
            let pid = project.id;
            let sessions = self
                .state
                .sessions
                .iter()
                .filter(|s| s.project_id == pid && !s.archived)
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
                    .child(div().text_size(px(11.5)).text_color(rgb(c_muted())).child(
                        match sessions {
                            0 => String::new(),
                            1 => "1 session".into(),
                            n => format!("{n} sessions"),
                        },
                    ))
                    .when(current == Some(pid), |el| {
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
                    .child("Add a project folder…"),
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

    /// Switch the frame to a project: open its latest session, or start a new one there.
    pub(crate) fn switch_project(&mut self, pid: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.new_project = Some(pid);
        if !self.expanded.contains(&pid) {
            self.expanded.push(pid);
        }
        let latest = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid && !s.archived)
            .map(|s| s.id)
            .max();
        match latest {
            Some(id) => self.select(id),
            None => self.open_new_session(),
        }
        window.focus(&self.focus, cx);
    }

    pub(crate) fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self.counts();
        let lead = if cfg!(target_os = "macos") { 80. } else { 8. };
        let project = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id));
        let branch = self
            .current()
            .filter(|_| self.mode() == Mode::Sessions)
            .map(|s| s.branch.clone())
            .filter(|b| !b.is_empty());
        let menu_open = self.menu.as_deref() == Some("project-switch");
        let mode = self.mode();
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
            .child(
                self.ghost_button("toggle-sidebar", IconName::PanelLeft, "Toggle sidebar")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sidebar = !this.show_sidebar;
                        this.save();
                        cx.notify();
                    })),
            )
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
                            .child(div().max_w(px(200.)).truncate().child(
                                project.map_or("No project".to_string(), |p| p.name.clone()),
                            ))
                            .child(icon(IconName::ChevronDown, 12., c_muted())),
                    )
                    .when(menu_open, |el| el.child(self.project_menu(cx))),
            )
            .when_some(branch, |el, branch| {
                el.child(
                    div()
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
                        .child(icon(IconName::GitBranch, 12., c_muted()))
                        .child(div().max_w(px(180.)).truncate().child(branch)),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child(icon(IconName::ChevronRight, 12., c_muted()))
                    .child(mode.label()),
            )
            .child(
                div().flex_1().min_w_0().flex().justify_center().child(
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
                                .child("Search sessions, files, commands"),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px(px(5.))
                                .rounded_sm()
                                .border_1()
                                .border_color(rgb(c_border()))
                                .text_size(px(11.))
                                .child(format!("{MOD_KEY}K")),
                        ),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        self.bar_pill("top-running")
                            .tooltip(|window, cx| {
                                Tooltip::new("Agents working now").build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.go(Mode::Sessions, window, cx);
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
                    .when(counts.spend_cents > 0.0, |el| {
                        el.child(
                            self.bar_pill("top-spend")
                                .cursor_default()
                                .tooltip(|window, cx| {
                                    Tooltip::new("Spend reported by the agents, all sessions")
                                        .build(window, cx)
                                })
                                .child(format!("${:.2}", counts.spend_cents / 100.0)),
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
                            .when(counts.needs_you > 0, |el| {
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
                                        .bg(rgb(c_blue()))
                                        .text_size(px(9.5))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0xffffff))
                                        .child(counts.needs_you.to_string()),
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
                                        this.show_panel = !this.show_panel;
                                        this.save();
                                        cx.notify();
                                    },
                                )),
                            )
                        },
                    )
                    .child(
                        self.bar_pill("top-where")
                            .cursor_default()
                            .text_color(rgb(c_text()))
                            .tooltip(|window, cx| {
                                Tooltip::new("Agents run on this computer").build(window, cx)
                            })
                            .child(div().size(px(7.)).rounded_full().bg(rgb(c_green())))
                            .child("Local"),
                    ),
            )
    }

    /// Open sessions as tabs, over the session they belong to.
    pub(crate) fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Tab(id), cx),
                    )
                    .group("tab")
                    .h(px(28.))
                    .mb(px(3.))
                    .mr_1()
                    .min_w(px(120.))
                    .max_w(px(210.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(transparent_black())
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(active, |el| {
                        el.bg(glass_fill(c_canvas()))
                            .text_color(rgb(c_text()))
                            .border_color(divider())
                    })
                    .when(!active, |el| {
                        el.text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
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
            .h(px(36.))
            .flex_none()
            .flex()
            .items_end()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_end()
                    .child(tabs.relative().child(tour::anchor(&self.anchors, "tabs"))),
            )
            .when(show_stats, |el| {
                el.child(
                    div()
                        .h_full()
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
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(id);
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

    pub(crate) fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let counts = self.counts();
        let mut needs: Vec<AnyElement> = Vec::new();
        let mut running: Vec<AnyElement> = Vec::new();
        let mut review: Vec<AnyElement> = Vec::new();
        for s in &self.state.sessions {
            match self.bucket(s) {
                Some(Bucket::NeedsYou) => {
                    let note = if s.status == Status::Failed {
                        "Failed"
                    } else {
                        "Finished"
                    };
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
                None => {}
            }
        }
        let (n_needs, n_running, n_review) = (needs.len(), running.len(), review.len());
        let project = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone());
        let body = div()
            .v_flex()
            .gap_4()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.stat_card(
                        "Needs you",
                        counts.needs_you.to_string(),
                        "finished unseen or failed".into(),
                        c_amber(),
                    ))
                    .child(self.stat_card(
                        "Running",
                        counts.running.to_string(),
                        "agents working now".into(),
                        c_blue(),
                    ))
                    .child(self.stat_card(
                        "Ready to review",
                        counts.review.to_string(),
                        "idle with changes".into(),
                        c_green(),
                    ))
                    .child(self.stat_card(
                        "Spend",
                        format!("${:.2}", counts.spend_cents / 100.0),
                        "reported by the agents".into(),
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
            .child(
                div().flex().child(
                    div()
                        .id("home-new")
                        .h(px(32.))
                        .px_3()
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
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_new_session();
                            cx.notify();
                        }))
                        .child(icon(IconName::SquarePen, 14., c_text()))
                        .child("New session")
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .child(format!("{MOD_KEY}N")),
                        ),
                ),
            );
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
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(project.unwrap_or_else(|| "Home".into())),
                            )
                            .child(div().text_size(px(12.5)).text_color(rgb(c_muted())).child(
                                match self.state.projects.len() {
                                    0 => "Add a project folder to get started.".to_string(),
                                    1 => "1 project".to_string(),
                                    n => format!("{n} projects"),
                                },
                            )),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    // ---------- Design and Architecture ----------

    /// The Design and Architecture pages, until those modes land: what each will hold.
    pub(crate) fn mode_page(&self, mode: Mode) -> AnyElement {
        let (title, body, points): (&str, &str, [&str; 4]) = match mode {
            Mode::Design => (
                "Design",
                "Design the app before it is built: a brief, user flows, a design system and clickable screens that agents then build from.",
                [
                    "Brief and flow map of every screen",
                    "Design system from your tokens",
                    "Canvas with a design agent and Try 3 directions",
                    "Build this, then check the build against the design",
                ],
            ),
            _ => (
                "Architecture",
                "Draw or generate the system's architecture from the code, keep it in the repo, and check every change against it.",
                [
                    "Generate containers and flows from the code",
                    "Views: context, containers, sequence",
                    "Rules checked after each agent turn",
                    "Branch diffs, and draw.io import and export",
                ],
            ),
        };
        let mut list = div().v_flex().gap_2().mt_2();
        for (i, point) in points.iter().enumerate() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .text_color(rgb(c_secondary()))
                    .child(
                        div()
                            .size(px(20.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child((i + 1).to_string()),
                    )
                    .child(point.to_string()),
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .v_flex()
                    .items_center()
                    .max_w(px(460.))
                    .child(empty_hint(
                        if mode == Mode::Design {
                            "design-page"
                        } else {
                            "arch-page"
                        },
                        mode.glyph(),
                        title,
                        body,
                    ))
                    .child(list)
                    .child(
                        div()
                            .mt_4()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child("This mode is on its way. Its place in the app is ready."),
                    ),
            )
            .into_any_element()
    }
}

/// A project's square badge: its icon on its colour, or its initial.
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
