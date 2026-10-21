//! Home: one prompt to start work from, which also searches sessions, projects, playbooks
//! and past conversations as you type; ideas to start from; what's waiting, in a line;
//! and the sessions to pick up again.

use crate::agent_pick::AgentPick;
use crate::archdraw::view::{col, tip};
use crate::sessions::HitsFor;
use crate::shell::{accent_bracket, cascade, hover_wash, live_dot, pop, seg_choice, seg_track};
use crate::shell::{Bucket, Need};
use crate::*;
use chrono::Timelike;
use gpui_kit::component::input;

/// How many sessions "Pick up where you left off" shows.
const RECENT: usize = 6;

/// Home's prompt and what goes with it.
pub(crate) struct HomeState {
    pub prompt: Entity<TextareaState>,
    /// The agent the prompt starts.
    pub pick: AgentPick,
    /// Whether a session started here gets its own branch and folder.
    pub isolated: bool,
    /// The row under the prompt the arrow keys have picked; 0 starts a session.
    pub hit: usize,
    /// The status the recent list is narrowed to, by its pill.
    pub filter: Option<Bucket>,
    /// Whether Home was on screen at the last paint, so arriving there focuses the prompt.
    pub shown: bool,
    /// The project Git's state was last read for from Home.
    pub git_for: Option<u64>,
}

impl HomeState {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<RunoApp>) -> (Self, Subscription) {
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe a task to start, or search sessions and history…")
                .submit_on_enter(true)
        });
        let sub = cx.subscribe_in(&prompt, window, |this, _, event, window, cx| match event {
            InputEvent::Change => {
                this.home.hit = 0;
                let query = this.home.prompt.read(cx).value().to_string();
                this.search_transcripts(query, HitsFor::Home);
                cx.notify();
            }
            InputEvent::PressEnter { shift, secondary } => {
                let go = if cx.global::<AppSettings>().send_on_enter {
                    !*shift
                } else {
                    *secondary
                };
                if go {
                    this.home_choose(window, cx);
                }
            }
            _ => {}
        });
        let home = Self {
            prompt,
            pick: AgentPick::new(cx),
            isolated: true,
            hit: 0,
            filter: None,
            shown: false,
            git_for: None,
        };
        (home, sub)
    }
}

/// A row under Home's prompt while something is typed in it.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum HomeHit {
    /// Start a session with what was typed.
    Start,
    Session(u64),
    /// A session whose conversation mentions what was typed, with the line it's in.
    Said(u64, String),
    Project(u64),
    Playbook(u64),
}

/// A one-tap start on Home.
enum Idea {
    /// Put this task in the prompt, to send or change first.
    Fill(&'static str),
    Open(u64),
    Review(u64),
    Playbook(u64),
    Git(GitStep),
}

/// What Home can do about what Git found in the top bar project's folder.
#[derive(Clone, Copy)]
enum GitStep {
    /// Open the clash page for a merge or rebase that stopped on clashing lines.
    SortOut,
    /// Show the clash radar's first finding in Branches.
    SeeClash,
    /// Bring in the commits waiting on the remote, then show Git.
    BringIn,
}

/// What Git has for Home: a merge or rebase stopped on clashing lines, clashes the radar
/// sees coming, and commits waiting on the remote.
pub(crate) struct GitNews {
    /// "rebase" or "merge", and how many files still clash.
    stopped: Option<(&'static str, usize)>,
    hits: Vec<crate::radar::Hit>,
    behind: usize,
}

impl GitNews {
    fn clashes(&self) -> usize {
        self.hits.len() + usize::from(self.stopped.is_some())
    }

    fn is_empty(&self) -> bool {
        self.clashes() == 0 && self.behind == 0
    }
}

/// "1 update", "3 updates".
fn updates(n: usize) -> String {
    if n == 1 {
        "1 update".into()
    } else {
        format!("{n} updates")
    }
}

/// The starting tasks Home offers in every project.
const IDEAS: [(IconName, &str); 4] = [
    (IconName::FlaskConical, "Run the tests and fix what fails"),
    (IconName::Bug, "Find and fix a bug in the recent changes"),
    (IconName::BookOpen, "Write or update the README"),
    (
        IconName::Lightbulb,
        "Explain how this project fits together",
    ),
];

/// The greeting for this time of day.
fn greeting() -> &'static str {
    match chrono::Local::now().hour() {
        5..=11 => "Good morning.",
        12..=17 => "Good afternoon.",
        _ => "Good evening.",
    }
}

/// `text` with each character coloured along a gradient from `from` through `via` to
/// `to`, for Home's headline.
fn gradient_text(text: &'static str, from: u32, via: u32, to: u32) -> StyledText {
    let n = text.chars().count().max(2) as f32 - 1.;
    let runs = text.char_indices().enumerate().map(|(i, (at, ch))| {
        let t = i as f32 / n;
        let color = if t < 0.5 {
            settings::mix(via, from, t * 2.)
        } else {
            settings::mix(to, via, (t - 0.5) * 2.)
        };
        (
            at..at + ch.len_utf8(),
            HighlightStyle {
                color: Some(rgb(color).into()),
                ..Default::default()
            },
        )
    });
    StyledText::new(text).with_highlights(runs.collect::<Vec<_>>())
}

/// Where `text` first contains `needle` (lowercase), ignoring case.
fn match_range(text: &str, needle: &str) -> Option<std::ops::Range<usize>> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; leave such text unmarked.
    (!needle.is_empty() && lower.len() == text.len())
        .then(|| lower.find(needle))
        .flatten()
        .map(|at| at..at + needle.len())
        .filter(|r| text.is_char_boundary(r.start) && text.is_char_boundary(r.end))
}

/// `text` with the first place it contains `needle` (lowercase) marked.
fn marked(text: String, needle: &str) -> StyledText {
    let range = match_range(&text, needle);
    let styled = StyledText::new(text);
    match range {
        Some(r) => styled.with_highlights([(
            r,
            HighlightStyle {
                background_color: Some(col(c_amber(), 0.3).into()),
                ..Default::default()
            },
        )]),
        None => styled,
    }
}

/// The violet and orange the headline and the prompt's ring blend the accent into.
const VIOLET: u32 = 0x8b5cf6;
const EMBER: u32 = 0xd97757;

impl RunoApp {
    /// Sessions for "Pick up where you left off": what needs you, then what runs, then
    /// what waits for review, then the latest, in Home's scope and narrowed by its pill.
    pub(crate) fn home_recent(&self) -> Vec<&Session> {
        let rank = |b: Option<Bucket>| match b {
            Some(Bucket::NeedsYou) => 0,
            Some(Bucket::Running) => 1,
            Some(Bucket::Review) => 2,
            _ => 3,
        };
        let mut list: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| self.on_home(s) && !s.archived && s.canvas.is_empty())
            .filter(|s| self.home.filter.is_none_or(|f| self.bucket(s) == Some(f)))
            .collect();
        list.sort_by_key(|s| (rank(self.bucket(s)), std::cmp::Reverse(s.id)));
        list.truncate(RECENT);
        list
    }

    /// The rows under the prompt for what is typed in it: starting it first, then the
    /// sessions, projects and playbooks it names, then conversations that mention it.
    pub(crate) fn home_hits(&self, cx: &App) -> Vec<HomeHit> {
        let query = self.home.prompt.read(cx).value().trim().to_string();
        if query.is_empty() {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        let has = |s: &str| s.to_lowercase().contains(&needle);
        let mut out = vec![HomeHit::Start];
        let mut sessions: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.canvas.is_empty() && !s.archived)
            .filter(|s| has(&s.title) || has(&s.branch))
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.id));
        out.extend(sessions.iter().take(4).map(|s| HomeHit::Session(s.id)));
        out.extend(
            self.state
                .projects
                .iter()
                .filter(|p| has(&p.name))
                .take(2)
                .map(|p| HomeHit::Project(p.id)),
        );
        out.extend(
            self.state
                .automations
                .iter()
                .filter(|a| has(&a.name))
                .take(2)
                .map(|a| HomeHit::Playbook(a.id)),
        );
        let hits = &self.finder.home_hits;
        if hits.query == query {
            let said: Vec<HomeHit> = hits
                .found
                .iter()
                .filter(|(id, _)| !out.contains(&HomeHit::Session(*id)))
                .filter(|(id, _)| self.state.session(*id).is_some_and(|s| !s.archived))
                .take(3)
                .map(|(id, said)| HomeHit::Said(*id, said.clone()))
                .collect();
            out.extend(said);
        }
        out
    }

    /// Do what the picked row under the prompt does.
    pub(crate) fn home_choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hits = self.home_hits(cx);
        let Some(hit) = hits.get(self.home.hit).or(hits.first()).cloned() else {
            return;
        };
        if hit == HomeHit::Start {
            self.home_start(window, cx);
            return;
        }
        self.home_clear(window, cx);
        match hit {
            HomeHit::Start => {}
            HomeHit::Session(id) | HomeHit::Said(id, _) => self.open_session(id, window, cx),
            HomeHit::Project(pid) => self.switch_project(pid, window, cx),
            HomeHit::Playbook(id) => {
                let here = self.current_project();
                self.run_playbook(id, here, cx);
            }
        }
        cx.notify();
    }

    fn home_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.home
            .prompt
            .update(cx, |p, cx| p.set_value("", window, cx));
        self.home.hit = 0;
    }

    /// Start a session in the top bar's project with what's in the prompt, on the agent,
    /// branch and folder picked under it, and open it.
    pub(crate) fn home_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.home.prompt.read(cx).value().trim().to_string();
        if task.is_empty() {
            return;
        }
        let Some(pid) = self
            .current_project()
            .or_else(|| self.state.projects.first().map(|p| p.id))
        else {
            self.pick_project(cx);
            return;
        };
        // The prompt starts one agent, with nothing carried over from the dialog.
        let race = std::mem::replace(&mut self.race_mode, false);
        let context = self.new_context.take();
        self.new_project = Some(pid);
        self.new_provider = self.home.pick.agent;
        self.new_model = Some(self.home.pick.model.clone()).filter(|m| !m.is_empty());
        self.new_isolated = self.home.isolated;
        self.new_connection = self.project_connection(pid);
        self.new_base = None;
        self.new_task
            .update(cx, |t, cx| t.set_value(task.clone(), window, cx));
        self.branch.update(cx, |b, cx| b.set_value("", window, cx));
        let before = self.state.next_id;
        self.create_session(window, cx);
        self.race_mode = race;
        self.new_context = context;
        if self.state.next_id != before {
            self.home_clear(window, cx);
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Move the picked row under the prompt; returns whether there were rows to move in.
    fn home_move(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        let n = self.home_hits(cx).len();
        if n == 0 {
            return false;
        }
        self.home.hit = if down {
            (self.home.hit + 1) % n
        } else {
            (self.home.hit + n - 1) % n
        };
        cx.notify();
        true
    }

    /// Focus the prompt when Home comes on screen.
    pub(crate) fn home_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on = self.view == View::Home && !self.state.projects.is_empty();
        if on && !self.home.shown {
            self.home.prompt.update(cx, |p, cx| p.focus(window, cx));
        }
        // What Git has to say about the project's folder, read again for each project,
        // the way the Git place reads it.
        let pid = self.current_project().filter(|_| on);
        if pid.is_some() && (!self.home.shown || self.home.git_for != pid) {
            self.load_repo();
        }
        self.home.git_for = pid;
        self.home.shown = on;
    }

    /// Home: a big prompt to start from, ideas, a line of what's waiting, and recent work.
    pub(crate) fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let counts = self.counts();
        let in_scope = |b: Bucket| {
            self.state
                .sessions
                .iter()
                .filter(|s| self.on_home(s) && self.bucket(s) == Some(b))
                .count()
        };
        let (n_needs, n_running, n_review) = (
            in_scope(Bucket::NeedsYou),
            in_scope(Bucket::Running),
            in_scope(Bucket::Review),
        );
        let project = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned();
        let missing = project
            .as_ref()
            .filter(|p| !self.home_all && !self.demo && !std::path::Path::new(&p.path).is_dir())
            .map(|p| (p.id, p.path.clone()));
        let no_projects = self.state.projects.is_empty();
        let typed = !self.home.prompt.read(cx).value().trim().is_empty();
        let git = self.home_git();

        let mut col_ = div()
            .w(px(760.))
            .max_w_full()
            .mx_auto()
            .px_6()
            .pt(px(72.))
            .pb_10()
            .v_flex()
            .child(self.home_hero(project.as_ref(), no_projects, cx));
        if let Some((pid, path)) = missing {
            col_ = col_.child(self.home_missing(pid, path, cx));
        }
        if no_projects {
            col_ = col_.child(self.home_welcome(cx));
        } else {
            col_ = col_.child(self.home_prompt(project.as_ref(), typed, cx));
            if typed {
                col_ = col_.child(self.home_results(cx));
            } else {
                if let Some(banner) = git.as_ref().and_then(|g| self.home_git_banner(g, cx)) {
                    col_ = col_.child(banner);
                }
                col_ = col_
                    .child(self.home_ideas(n_needs, n_review, git.as_ref(), cx))
                    .child(self.home_status(
                        n_needs,
                        n_running,
                        n_review,
                        counts.spend_today,
                        git.as_ref(),
                        cx,
                    ))
                    .child(self.home_recent_view(cx));
            }
        }
        div()
            .id("home")
            .flex_1()
            .min_w_0()
            .relative()
            .overflow_hidden()
            .child(home_glow())
            .child(
                div()
                    .id("home-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .child(col_),
            )
            .into_any_element()
    }

    /// The project line and the big greeting above the prompt.
    fn home_hero(
        &self,
        project: Option<&Project>,
        no_projects: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scope = |label: &'static str, all: bool| {
            let id = if all { "home-all" } else { "home-one" };
            seg_choice(id, format!("seg-fill-{id}"), self.home_all == all)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.home_all = all;
                    this.cursor = None;
                    this.save();
                    cx.notify();
                }))
                .child(label)
        };
        let accent = c_blue();
        let ask: &'static str = if no_projects {
            "Let's get you building."
        } else {
            "What should we build?"
        };
        div()
            .v_flex()
            .mb(px(26.))
            .child(
                div()
                    .h(px(32.))
                    .mb(px(18.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .when_some(project.filter(|_| !self.home_all), |el, p| {
                        el.child(crate::shell::project_mark(p, 18.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(c_secondary()))
                                    .child(p.name.clone()),
                            )
                            .child(
                                div().min_w_0().truncate().text_color(rgb(c_muted())).child(
                                    crate::settings_ui::tilde(std::path::Path::new(&p.path)),
                                ),
                            )
                    })
                    .when(self.home_all, |el| {
                        el.child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(c_secondary()))
                                .child(format!("All projects  ·  {}", self.state.projects.len())),
                        )
                    })
                    .child(div().flex_1())
                    .when(self.state.projects.len() > 1, |el| {
                        el.child(
                            seg_track()
                                .child(scope("This project", false))
                                .child(scope("All projects", true)),
                        )
                    })
                    .when(!no_projects, |el| {
                        el.child(
                            self.home_button(
                                "home-add",
                                IconName::FolderPlus,
                                "Add project",
                                String::new(),
                            )
                            .h(px(30.))
                            .font_weight(FontWeight::NORMAL)
                            .on_click(cx.listener(|this, _, _, cx| this.pick_project(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(px(40.))
                    .line_height(px(46.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if no_projects {
                        "Welcome to Runo."
                    } else {
                        greeting()
                    }),
            )
            .child(
                div()
                    .text_size(px(40.))
                    .line_height(px(48.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(gradient_text(
                        ask,
                        accent,
                        settings::mix(VIOLET, accent, 0.6),
                        EMBER,
                    )),
            )
            .animated(
                "home-hero",
                Animation::new(Duration::from_millis(420)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d).mt(px(10. * (1. - d))),
            )
    }

    /// The prompt: a big box in a soft gradient ring, with the project, agent and folder
    /// chips and Start under the text.
    fn home_prompt(
        &self,
        project: Option<&Project>,
        typed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = c_blue();
        let ring_to = if typed {
            EMBER
        } else {
            settings::mix(VIOLET, accent, 0.6)
        };
        let is_git = project.is_some_and(|p| git::is_repo(&p.path));
        let project_open = self.menu.as_deref() == Some("home-project");
        let project_chip = div()
            .relative()
            .child(
                prompt_chip("home-project-chip", project_open)
                    .child(icon(IconName::Folder, 13., c_secondary()))
                    .child(
                        project
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| "Pick a project".into()),
                    )
                    .child(icon(IconName::ChevronDown, 11., c_muted()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = if this.menu.as_deref() == Some("home-project") {
                            None
                        } else {
                            Some("home-project".into())
                        };
                        this.home.pick.open = false;
                        cx.notify();
                    })),
            )
            .when(project_open, |el| el.child(self.home_project_menu(cx)));
        // The agent picker's own chip, on the same tile as the chips beside it.
        let agent_chip = div()
            .relative()
            .h(px(28.))
            .px(px(2.))
            .flex()
            .items_center()
            .rounded(px(8.))
            .bg(tint(c_subtle()))
            .child(
                self.home
                    .pick
                    .chip("home", cx, |t: &mut Self| &mut t.home.pick),
            )
            .children(
                self.home
                    .pick
                    .menu("home", "", cx, |t: &mut Self| &mut t.home.pick)
                    .map(|menu| {
                        div().absolute().top(px(30.)).left_0().child(
                            deferred(
                                anchored()
                                    .anchor(Anchor::TopLeft)
                                    .snap_to_window_with_margin(px(8.))
                                    .child(
                                        div()
                                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                                this.home.pick.open = false;
                                                cx.notify();
                                            }))
                                            .child(menu),
                                    ),
                            )
                            .with_priority(2),
                        )
                    }),
            );
        let isolated = self.home.isolated && is_git;
        let folder_chip = prompt_chip("home-isolated-chip", false)
            .child(icon(
                if isolated {
                    IconName::GitBranch
                } else {
                    IconName::FolderOpen
                },
                12.,
                c_secondary(),
            ))
            .child(if isolated { "Isolated" } else { "In place" })
            .when(is_git, |el| {
                el.tooltip(tip(if isolated {
                    "Runs on its own branch in its own folder. Click to work in the project folder."
                } else {
                    "Works in the project folder. Click to give it its own branch and folder."
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.home.isolated = !this.home.isolated;
                    cx.notify();
                }))
            });
        let start = div()
            .id("home-start")
            .h(px(32.))
            .pl(px(14.))
            .pr(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .rounded(px(9.))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .when(typed, |el| {
                el.bg(rgb(c_text()))
                    .text_color(rgb(c_canvas()))
                    .cursor_pointer()
                    .hover(|el| el.opacity(0.9))
                    .active(|el| el.opacity(0.75))
            })
            .when(!typed, |el| {
                el.bg(tint(c_active())).text_color(rgb(c_muted()))
            })
            .child("Start")
            .child(div().text_size(px(11.)).opacity(0.7).child(
                if cx.global::<AppSettings>().send_on_enter {
                    "↵"
                } else {
                    "⌘↵"
                },
            ))
            .on_click(cx.listener(|this, _, window, cx| this.home_start(window, cx)));
        let inner = div()
            .id("home-prompt")
            .rounded(px(17.))
            .bg(float(c_raised()))
            .px(px(18.))
            .pt(px(14.))
            .pb(px(12.))
            .v_flex()
            .gap(px(10.))
            .text_size(px(16.))
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| {
                this.home.prompt.update(cx, |p, cx| p.focus(window, cx));
            }))
            // The arrows pick a row under the prompt while there are rows, and Escape clears
            // the prompt, then leaves it so the arrows walk the cards below.
            .capture_action(cx.listener(|this, _: &input::MoveDown, _, cx| {
                if this.home_move(true, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::MoveUp, _, cx| {
                if this.home_move(false, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Escape, window, cx| {
                if this.home.prompt.read(cx).value().is_empty() {
                    window.focus(&this.focus, cx);
                } else {
                    this.home_clear(window, cx);
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                Textarea::new(&self.home.prompt)
                    .appearance(false)
                    .bordered(false)
                    .h(px(58.)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(project_chip)
                    .child(agent_chip)
                    .child(folder_chip)
                    .child(div().flex_1())
                    .child(start),
            );
        // The ring: a gradient a hair larger than the box, with a soft glow under it.
        div()
            .relative()
            .child(
                div()
                    .absolute()
                    .top(px(16.))
                    .left(px(40.))
                    .right(px(40.))
                    .bottom(px(-10.))
                    .rounded(px(30.))
                    .shadow(vec![BoxShadow {
                        color: col(accent, if is_dark() { 0.35 } else { 0.22 }).into(),
                        offset: point(px(0.), px(10.)),
                        blur_radius: px(44.),
                        spread_radius: px(0.),
                        inset: false,
                    }]),
            )
            .child(
                div()
                    .relative()
                    .p(px(1.5))
                    .rounded(px(18.5))
                    .bg(linear_gradient(
                        100.,
                        linear_color_stop(col(accent, 0.85), 0.),
                        linear_color_stop(col(ring_to, 0.75), 1.),
                    ))
                    .child(inner),
            )
            .animated(
                "home-prompt-in",
                Animation::new(Duration::from_millis(480)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d).mt(px(14. * (1. - d))),
            )
    }

    /// The projects to start in, from the prompt's project chip.
    fn home_project_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.current_project();
        let mut list = div().p_1().w(px(240.)).v_flex();
        for p in &self.state.projects {
            let pid = p.id;
            list = list.child(
                menu_row(
                    ("home-project-pick", pid as usize),
                    p.name.clone(),
                    crate::settings_ui::tilde(std::path::Path::new(&p.path)),
                    current == Some(pid),
                    Some(crate::shell::project_mark(p, 16.).into_any_element()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.menu = None;
                    this.switch_project(pid, window, cx);
                    this.home.prompt.update(cx, |p, cx| p.focus(window, cx));
                    cx.notify();
                })),
            );
        }
        list = list.child(div().my_1().h(px(1.)).bg(divider())).child(
            menu_row(
                "home-project-add",
                "Add project…".into(),
                String::new(),
                false,
                Some(icon(IconName::FolderPlus, 14., c_secondary()).into_any_element()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.menu = None;
                this.pick_project(cx);
            })),
        );
        crate::sessions::drop_menu("home-project", list, cx)
    }

    /// What starting, and what matches, look like as you type.
    fn home_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.home.prompt.read(cx).value().trim().to_string();
        let needle = query.to_lowercase();
        let hits = self.home_hits(cx);
        let picked = self.home.hit.min(hits.len().saturating_sub(1));
        let project_name = |pid: u64| {
            self.state
                .projects
                .iter()
                .find(|p| p.id == pid)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        };
        let mut list = div().v_flex().p(px(6.));
        let mut last_group = "";
        for (i, hit) in hits.iter().enumerate() {
            let group = match hit {
                HomeHit::Start => "",
                HomeHit::Session(_) => "Sessions",
                HomeHit::Project(_) => "Projects",
                HomeHit::Playbook(_) => "Playbooks",
                HomeHit::Said(..) => "In conversations",
            };
            if group != last_group && !group.is_empty() {
                list = list.child(
                    div()
                        .px(px(10.))
                        .pt(px(10.))
                        .pb(px(4.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_muted()))
                        .child(group.to_uppercase()),
                );
            }
            last_group = group;
            let (lead, title, meta, right): (AnyElement, AnyElement, String, Option<AnyElement>) =
                match hit {
                    HomeHit::Start => {
                        let provider = &agent::PROVIDERS[self.home.pick.agent];
                        let isolated = self.home.isolated
                            && self
                                .current_project()
                                .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
                                .is_some_and(|p| git::is_repo(&p.path));
                        (
                            icon(IconName::Sparkles, 14., c_blue()).into_any_element(),
                            div()
                                .flex()
                                .min_w_0()
                                .gap_1()
                                .child(div().flex_none().child("Start a session:"))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(query.clone()),
                                )
                                .into_any_element(),
                            format!(
                                "{}  ·  {}  ·  {}",
                                self.current_project().map(project_name).unwrap_or_default(),
                                provider.name,
                                if isolated {
                                    "its own branch"
                                } else {
                                    "in the project folder"
                                }
                            ),
                            Some(key_hint(if cx.global::<AppSettings>().send_on_enter {
                                "↵"
                            } else {
                                "⌘↵"
                            })),
                        )
                    }
                    HomeHit::Session(id) | HomeHit::Said(id, _) => {
                        let Some(s) = self.state.session(*id) else {
                            continue;
                        };
                        let provider = agent::provider(&s.provider);
                        let (title, meta) = match hit {
                            HomeHit::Said(_, said) => (
                                marked(said.clone(), &needle).into_any_element(),
                                format!("{}  ·  {}", s.title, project_name(s.project_id)),
                            ),
                            _ => (
                                marked(s.title.clone(), &needle).into_any_element(),
                                format!(
                                    "{}  ·  {}",
                                    project_name(s.project_id),
                                    self.bucket(s).map_or("Archived", Bucket::label)
                                ),
                            ),
                        };
                        let diff = self
                            .stats
                            .get(id)
                            .filter(|st| st.1 + st.2 > 0)
                            .map(|&(_, a, r)| diff_label(a, r));
                        (
                            div()
                                .text_color(rgb(provider.color))
                                .child(provider.mark)
                                .into_any_element(),
                            title,
                            meta,
                            diff,
                        )
                    }
                    HomeHit::Project(pid) => {
                        let Some(p) = self.state.projects.iter().find(|p| p.id == *pid) else {
                            continue;
                        };
                        (
                            crate::shell::project_mark(p, 16.).into_any_element(),
                            marked(p.name.clone(), &needle).into_any_element(),
                            crate::settings_ui::tilde(std::path::Path::new(&p.path)),
                            None,
                        )
                    }
                    HomeHit::Playbook(id) => {
                        let Some(a) = self.state.automations.iter().find(|a| a.id == *id) else {
                            continue;
                        };
                        (
                            icon(IconName::Play, 13., c_secondary()).into_any_element(),
                            marked(a.name.clone(), &needle).into_any_element(),
                            "Run this playbook".into(),
                            None,
                        )
                    }
                };
            let on = i == picked;
            list = list.child(cascade(
                div()
                    .id(("home-hit", i))
                    .relative()
                    .px(px(10.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(9.))
                    .cursor_pointer()
                    .when(on, |el| {
                        el.bg(col(c_blue(), if is_dark() { 0.18 } else { 0.08 }))
                    })
                    .when(!on, |el| el.hover(|el| el.bg(tint(c_active()))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.home.hit = i;
                        this.home_choose(window, cx);
                    }))
                    .child(
                        div()
                            .size(px(26.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.))
                            .bg(tint(c_subtle()))
                            .child(lead),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(div().min_w_0().truncate().text_size(px(13.)).child(title))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .child(meta),
                            ),
                    )
                    .children(right),
                format!("home-hit-{i}"),
                i,
            ));
        }
        let foot = div()
            .mt_1()
            .px(px(16.))
            .py(px(9.))
            .flex()
            .gap_4()
            .border_t_1()
            .border_color(divider())
            .text_size(px(11.5))
            .text_color(rgb(c_muted()))
            .child(foot_hint("↑ ↓", "choose"))
            .child(foot_hint("↵", "open"))
            .child(foot_hint("⇧↵", "new line"))
            .child(foot_hint("esc", "clear"));
        div()
            .mt(px(10.))
            .rounded(px(14.))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_raised()))
            .shadow_lg()
            .v_flex()
            .child(list)
            .child(foot)
            .into_any_element()
    }

    /// One-tap starts under the prompt: what's waiting first, then common tasks and the
    /// project's playbooks.
    fn home_ideas(
        &self,
        n_needs: usize,
        n_review: usize,
        git: Option<&GitNews>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Each idea with the colour that marks it as waiting on the person, if it is.
        let mut ideas: Vec<(IconName, String, Idea, Option<u32>)> = Vec::new();
        if let Some(git) = git {
            if git.stopped.is_some() {
                ideas.push((
                    IconName::TriangleAlert,
                    "Sort out the clashing lines".into(),
                    Idea::Git(GitStep::SortOut),
                    Some(c_red()),
                ));
            } else if let Some(hit) = git.hits.first() {
                ideas.push((
                    IconName::TriangleAlert,
                    format!(
                        "Sort out the clash in {}",
                        crate::radar::file_names(&hit.files)
                    ),
                    Idea::Git(GitStep::SeeClash),
                    Some(c_red()),
                ));
            }
            if git.behind > 0 && git.stopped.is_none() {
                ideas.push((
                    IconName::ArrowDown,
                    format!("Bring in {}", updates(git.behind)),
                    Idea::Git(GitStep::BringIn),
                    Some(c_green()),
                ));
            }
        }
        let first = |b: Bucket| {
            self.state
                .sessions
                .iter()
                .filter(|s| self.on_home(s) && self.bucket(s) == Some(b))
                .map(|s| s.id)
                .max()
        };
        if let Some(id) = first(Bucket::NeedsYou) {
            let label = if n_needs == 1 {
                "Answer the agent that needs you".to_string()
            } else {
                format!("Answer the {n_needs} agents that need you")
            };
            ideas.push((IconName::Hand, label, Idea::Open(id), Some(c_green())));
        }
        if let Some(id) = first(Bucket::Review) {
            let label = if n_review == 1 {
                "Review the change waiting".to_string()
            } else {
                format!("Review the {n_review} changes waiting")
            };
            ideas.push((
                IconName::GitCompare,
                label,
                Idea::Review(id),
                Some(c_green()),
            ));
        }
        for (glyph, text) in IDEAS {
            ideas.push((glyph, text.to_string(), Idea::Fill(text), None));
        }
        let here = self.current_project();
        for a in self
            .state
            .automations
            .iter()
            .filter(|a| Some(a.project_id) == here)
            .take(2)
        {
            ideas.push((
                IconName::Play,
                format!("Run {}", a.name),
                Idea::Playbook(a.id),
                None,
            ));
        }
        let mut row = div()
            .mt(px(22.))
            .flex()
            .flex_wrap()
            .justify_center()
            .gap_2();
        for (i, (glyph, label, idea, hot)) in ideas.into_iter().enumerate() {
            let id = ElementId::from(("home-idea", i));
            let chip = div()
                .id(id.clone())
                .relative()
                .h(px(30.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded_full()
                .border_1()
                .border_color(hot.map_or(divider(), |c| col(c, 0.45)))
                .text_size(px(12.5))
                .text_color(rgb(if hot.is_some() {
                    c_text()
                } else {
                    c_secondary()
                }))
                .cursor_pointer()
                .hover(|el| el.text_color(rgb(c_text())))
                .active(|el| el.opacity(0.75));
            let chip = hover_wash(chip, id, tint(c_active()), 15.)
                .child(icon(glyph, 13., hot.unwrap_or(c_muted())))
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    match &idea {
                        Idea::Fill(text) => {
                            this.home.prompt.update(cx, |p, cx| {
                                p.set_value(*text, window, cx);
                                p.focus(window, cx);
                            });
                        }
                        Idea::Open(id) => this.open_session(*id, window, cx),
                        Idea::Review(id) => this.review(*id),
                        Idea::Playbook(id) => {
                            let here = this.current_project();
                            this.run_playbook(*id, here, cx);
                        }
                        Idea::Git(step) => this.home_git_step(*step),
                    }
                    cx.notify();
                }));
            row = row.child(cascade(chip, format!("home-idea-{i}"), i + 2));
        }
        row.into_any_element()
    }

    /// What's waiting, in one line of pills; each narrows the list below to its sessions.
    fn home_status(
        &self,
        n_needs: usize,
        n_running: usize,
        n_review: usize,
        spend_today: f64,
        git: Option<&GitNews>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let git_pill = |id: &'static str, text: String, color: u32, step: GitStep| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded_full()
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(rgb(c_secondary()))
                .bg(col(color, if is_dark() { 0.18 } else { 0.11 }))
                .hover(|el| el.text_color(rgb(c_text())))
                .child(div().size(px(7.)).rounded_full().bg(rgb(color)))
                .child(text)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.home_git_step(step);
                    cx.notify();
                }))
        };
        let clashes = git.map_or(0, GitNews::clashes);
        let behind = git.filter(|g| g.stopped.is_none()).map_or(0, |g| g.behind);
        let pill = |bucket: Bucket, n: usize, label: &'static str, color: u32| {
            let on = self.home.filter == Some(bucket);
            let lit =
                on || (bucket == Bucket::Review && n > 0) || (bucket == Bucket::NeedsYou && n > 0);
            div()
                .id(SharedString::from(format!("home-pill-{label}")))
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded_full()
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(rgb(c_secondary()))
                .bg(if lit {
                    col(color, if is_dark() { 0.18 } else { 0.11 })
                } else {
                    tint(c_subtle())
                })
                .when(on, |el| el.border_1().border_color(col(color, 0.6)))
                .hover(|el| el.text_color(rgb(c_text())))
                .child(if bucket == Bucket::Running && n > 0 {
                    live_dot(color, 7., "home-pill-live")
                } else {
                    div()
                        .size(px(7.))
                        .rounded_full()
                        .bg(rgb(color))
                        .into_any_element()
                })
                .child(pop(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_text()))
                        .child(n.to_string()),
                    format!("home-pill-{label}-{n}"),
                ))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.home.filter = if this.home.filter == Some(bucket) {
                        None
                    } else {
                        Some(bucket)
                    };
                    this.cursor = None;
                    cx.notify();
                }))
        };
        div()
            .mt(px(30.))
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(6.))
            .child(pill(
                Bucket::NeedsYou,
                n_needs,
                if n_needs == 1 {
                    "needs you"
                } else {
                    "need you"
                },
                c_amber(),
            ))
            .child(pill(Bucket::Running, n_running, "running", c_blue()))
            .child(pill(Bucket::Review, n_review, "ready to review", c_green()))
            .when(clashes > 0, |el| {
                let step = if git.is_some_and(|g| g.stopped.is_some()) {
                    GitStep::SortOut
                } else {
                    GitStep::SeeClash
                };
                el.child(git_pill(
                    "home-pill-clash",
                    if clashes == 1 {
                        "1 clash ahead".into()
                    } else {
                        format!("{clashes} clashes ahead")
                    },
                    c_red(),
                    step,
                ))
            })
            .when(behind > 0, |el| {
                el.child(git_pill(
                    "home-pill-behind",
                    format!("{} to bring in", updates(behind)),
                    c_secondary(),
                    GitStep::BringIn,
                ))
            })
            .child(
                div()
                    .id("home-pill-spend")
                    .h(px(28.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .rounded_full()
                    .bg(tint(c_subtle()))
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child(format!("${:.2}", spend_today / 100.0)),
                    )
                    .child("today")
                    .tooltip(tip(
                        "Spend reported by Claude Code and OpenCode, all projects",
                    )),
            )
            .animated(
                "home-status-in",
                Animation::new(Duration::from_millis(QUICK_HOME)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d),
            )
    }

    /// "Pick up where you left off": recent sessions as cards, two to a row, with links to
    /// the history, timeline, notes and isolated sessions beside the heading.
    fn home_recent_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let links: [(&'static str, IconName, &'static str, View); 4] = [
            (
                "home-history",
                IconName::RotateCcwClock,
                "History",
                View::History,
            ),
            (
                "home-activity",
                IconName::Activity,
                "Timeline",
                View::Activity,
            ),
            ("home-notes", IconName::NotebookPen, "Notes", View::Notes),
            (
                "home-isolated",
                IconName::GitBranch,
                "Isolated",
                View::Worktrees,
            ),
        ];
        let mut head = div()
            .px_1()
            .mb(px(10.))
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_muted()))
                    .child(match self.home.filter {
                        Some(b) => b.label().to_uppercase(),
                        None => "PICK UP WHERE YOU LEFT OFF".into(),
                    }),
            );
        for (id, glyph, label, view) in links {
            let wash = ElementId::from(SharedString::from(id));
            let link = div()
                .id(wash.clone())
                .relative()
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .gap(px(5.))
                .rounded(px(7.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(rgb(c_secondary()))
                .hover(|el| el.text_color(rgb(c_text())));
            head = head.child(
                hover_wash(link, wash, tint(c_active()), 7.)
                    .child(icon(glyph, 13., c_muted()))
                    .child(label)
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
        let recent = self.home_recent();
        let mut grid = div().flex().flex_wrap().gap_2();
        for (i, s) in recent.iter().enumerate() {
            grid = grid.child(cascade(
                self.home_card(s, cx),
                format!("home-card-{}", s.id),
                i + 4,
            ));
        }
        let empty = recent.is_empty();
        div()
            .mt(px(34.))
            .v_flex()
            .child(head)
            .child(grid)
            .when(empty, |el| {
                el.child(
                    div()
                        .py_6()
                        .flex()
                        .justify_center()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child(if self.home.filter.is_some() {
                            "Nothing here right now."
                        } else {
                            "Sessions you start show up here."
                        }),
                )
            })
            .into_any_element()
    }

    /// A recent session: its agent, title, project and state, and Review or Open.
    fn home_card(&self, s: &Session, cx: &mut Context<Self>) -> AnyElement {
        let id = s.id;
        let provider = agent::provider(&s.provider);
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == s.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let bucket = self.bucket(s);
        let (dot, state) = match bucket {
            Some(Bucket::NeedsYou) => (
                c_amber(),
                self.need(s).map_or("Needs you", Need::label).to_string(),
            ),
            Some(Bucket::Running) => (
                c_blue(),
                self.started.get(&id).map_or("Working".to_string(), |t| {
                    format!("Working  ·  {}", elapsed(t.elapsed()))
                }),
            ),
            Some(Bucket::Review) => (c_green(), "Ready to review".to_string()),
            _ => (
                c_muted(),
                s.messages
                    .last()
                    .map(|m| m.time.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Idle".into()),
            ),
        };
        let review = bucket == Some(Bucket::Review);
        let chosen = self.cursor == Some(id);
        let action = div()
            .flex_none()
            .h(px(24.))
            .px(px(9.))
            .flex()
            .items_center()
            .rounded(px(7.))
            .border_1()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .bg(rgb(c_raised()))
            .when(review, |el| {
                el.border_color(col(c_green(), 0.45))
                    .text_color(rgb(c_green()))
                    .child("Review")
            })
            .when(!review, |el| {
                el.border_color(divider())
                    .text_color(rgb(c_secondary()))
                    .child(if bucket == Some(Bucket::NeedsYou) {
                        "Answer"
                    } else {
                        "Open"
                    })
            });
        let card = div()
            .id(("home-card", id as usize))
            .relative()
            // Two to a row, with the gap between them.
            .w(px(352.))
            .flex_grow(1.)
            .px(px(12.))
            .py(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(12.))
            .bg(tint(c_subtle()))
            .cursor_pointer()
            .when(chosen, |el| {
                el.bg(tint(c_active()))
                    .child(accent_bracket(format!("home-card-bracket-{id}"), 12.))
            })
            .hover(|el| el.bg(tint(c_active())))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::Session(id), cx),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                // Work waiting for review opens on Review, where it can be shipped.
                let waiting = this
                    .state
                    .session(id)
                    .is_some_and(|s| this.bucket(s) == Some(Bucket::Review));
                if waiting {
                    this.review(id);
                } else {
                    this.open_session(id, window, cx);
                }
                cx.notify();
            }))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(self.provider_badge(provider, 30.))
                    .child(
                        div()
                            .absolute()
                            .right(px(-2.))
                            .bottom(px(-2.))
                            .p(px(2.))
                            .rounded_full()
                            .bg(rgb(c_canvas()))
                            .child(if bucket == Some(Bucket::Running) {
                                live_dot(dot, 7., format!("home-card-live-{id}"))
                            } else {
                                div()
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(rgb(dot))
                                    .into_any_element()
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap(px(1.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(s.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(format!("{project}  ·  {state}")),
                    ),
            )
            .child(action);
        card.into_any_element()
    }

    /// What Git found in the top bar project's folder, when Home shows one project.
    fn home_git(&self) -> Option<GitNews> {
        if self.home_all || self.demo {
            return None;
        }
        let dir = self.git_place_dir()?;
        let repo = self.git_repo.as_ref().filter(|r| r.dir == dir);
        let pid = self.current_project();
        // The radar's sessions belong to the project it last looked at.
        let hits: Vec<crate::radar::Hit> = self
            .git_radar
            .iter()
            .flatten()
            .filter(|h| {
                [&h.a, &h.b].iter().all(|side| {
                    side.session.is_none_or(|id| {
                        self.state
                            .session(id)
                            .is_some_and(|s| Some(s.project_id) == pid)
                    })
                })
            })
            .cloned()
            .collect();
        let news = GitNews {
            stopped: repo
                .and_then(|r| r.clash.as_ref())
                .map(|c| (c.kind, c.files.len())),
            hits,
            behind: repo.map_or(0, |r| r.state.behind),
        };
        (!news.is_empty()).then_some(news)
    }

    /// Take one of Home's Git steps.
    fn home_git_step(&mut self, step: GitStep) {
        match step {
            GitStep::SortOut => self.open_review(),
            GitStep::SeeClash => {
                self.open_branches();
                self.git_radar_sel = self.git_radar.as_ref().filter(|h| !h.is_empty()).map(|_| 0);
            }
            GitStep::BringIn => {
                self.git_sync(crate::gitplace::SyncStep::Pull);
                self.open_review();
            }
        }
    }

    /// A banner for a clash: a merge or rebase stopped on clashing lines, or two lines of
    /// work the radar sees changing the same lines, with what to do about it.
    fn home_git_banner(&self, git: &GitNews, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (title, line, primary): (String, String, GitStep) = if let Some((kind, n)) = git.stopped
        {
            (
                format!(
                    "The {kind} stopped on clashing lines in {n} file{}",
                    if n == 1 { "" } else { "s" }
                ),
                "Pick what to keep in each spot. Nothing is saved until you finish.".into(),
                GitStep::SortOut,
            )
        } else {
            let hit = git.hits.first()?;
            let lines = hit
                .files
                .first()
                .map(|f| crate::radar::around(&f.lines))
                .unwrap_or_default();
            let more = match git.hits.len() {
                1 => String::new(),
                2 => " 1 more clash is in Branches.".into(),
                n => format!(" {} more clashes are in Branches.", n - 1),
            };
            (
                format!(
                    "{} and {} both change {} {lines}",
                    hit.a.label,
                    hit.b.label,
                    crate::radar::file_names(&hit.files)
                )
                .trim_end()
                .to_string(),
                format!("Clash radar found it before anyone merged. Nothing is broken yet.{more}"),
                GitStep::SeeClash,
            )
        };
        // Whoever can bring the other's committed work in now, while it's small.
        let ask = git
            .stopped
            .is_none()
            .then(|| git.hits.first())
            .flatten()
            .and_then(|hit| {
                [(&hit.a, &hit.b), (&hit.b, &hit.a)]
                    .into_iter()
                    .find_map(|(me, other)| {
                        me.session
                            .filter(|_| other.committed)
                            .map(|id| (id, other.clone(), hit.files.clone()))
                    })
            });
        let button = |id: &'static str, label: &'static str, strong: bool| {
            div()
                .id(id)
                .flex_none()
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(8.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .active(|el| el.opacity(0.75))
                .when(strong, |el| {
                    el.bg(rgb(c_text()))
                        .text_color(rgb(c_canvas()))
                        .hover(|el| el.opacity(0.9))
                })
                .when(!strong, |el| {
                    el.border_1()
                        .border_color(divider())
                        .bg(rgb(c_raised()))
                        .hover(|el| el.bg(tint(c_subtle())))
                })
                .child(label)
        };
        let banner = div()
            .mt(px(18.))
            .px(px(14.))
            .py(px(11.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(14.))
            .border_1()
            .border_color(col(c_red(), 0.45))
            .bg(col(c_red(), if is_dark() { 0.1 } else { 0.06 }))
            .child(icon(IconName::TriangleAlert, 15., c_red()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(line),
                    ),
            )
            .child(
                button(
                    "home-git-see",
                    if git.stopped.is_some() {
                        "Sort it out"
                    } else {
                        "See both"
                    },
                    ask.is_none(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.home_git_step(primary);
                    cx.notify();
                })),
            )
            .when_some(ask, |el, (id, other, files)| {
                el.child(
                    button("home-git-ask", "Ask the agent to line them up", true).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.radar_ask(id, &other, &files, cx);
                            cx.notify();
                        }),
                    ),
                )
            });
        Some(rise(banner, "home-git-banner", 320))
    }

    /// The warning when the project's folder has gone, with ways to fix it.
    fn home_missing(&self, pid: u64, path: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .mb(px(18.))
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
            )
            .into_any_element()
    }

    /// With no projects yet: one big way to add the first.
    fn home_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("home-welcome")
            .p_8()
            .v_flex()
            .items_center()
            .gap_3()
            .rounded(px(18.))
            .border_1()
            .border_dashed()
            .border_color(col(c_blue(), 0.5))
            .bg(col(c_blue(), if is_dark() { 0.08 } else { 0.04 }))
            .cursor_pointer()
            .hover(|el| el.bg(col(c_blue(), if is_dark() { 0.12 } else { 0.07 })))
            .on_click(cx.listener(|this, _, _, cx| this.pick_project(cx)))
            .child(icon(IconName::FolderPlus, 26., c_blue()))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Add a project folder"),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(c_secondary()))
                    .child(
                    "Pick a folder with your code. Then tell an agent what to build, right here.",
                ),
            )
            .into_any_element()
    }
}

/// How long Home's status line takes to fade in.
const QUICK_HOME: u64 = 360;

/// The soft accent light at the top of Home, behind the greeting and the prompt.
fn home_glow() -> AnyElement {
    let accent = c_blue();
    let blob = |left: f32, top: f32, w: f32, h: f32, color: u32, a: f32| {
        div()
            .absolute()
            .left(relative(left))
            .top(px(top))
            .w(px(w))
            .h(px(h))
            .rounded_full()
            .shadow(vec![BoxShadow {
                color: col(color, a).into(),
                offset: point(px(0.), px(0.)),
                blur_radius: px(160.),
                spread_radius: px(40.),
                inset: false,
            }])
    };
    let strength = if is_dark() { 0.22 } else { 0.12 };
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(blob(0.3, -180., 420., 220., accent, strength))
        .child(blob(0.52, -140., 320., 180., EMBER, strength * 0.6))
        .animated(
            "home-glow",
            Animation::new(Duration::from_millis(900)).with_easing(ease_out_quint()),
            |el, d| el.opacity(d),
        )
}

/// A chip in the prompt's bottom row.
fn prompt_chip(id: &'static str, open: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .rounded(px(8.))
        .bg(tint(c_subtle()))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(c_secondary()))
        .cursor_pointer()
        .hover(|el| el.bg(tint(c_active())).text_color(rgb(c_text())))
        .when(open, |el| el.bg(tint(c_active())))
}

/// A key in a small box, as on the rows under the prompt.
fn key_hint(key: &'static str) -> AnyElement {
    div()
        .flex_none()
        .px(px(5.))
        .rounded(px(5.))
        .border_1()
        .border_color(divider())
        .text_size(px(11.))
        .text_color(rgb(c_muted()))
        .child(key)
        .into_any_element()
}

fn foot_hint(key: &'static str, what: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .child(key_hint(key))
        .child(what)
}

/// Lines added and removed, in green and red.
fn diff_label(added: usize, removed: usize) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .gap(px(6.))
        .text_size(px(12.))
        .child(div().text_color(rgb(c_green())).child(format!("+{added}")))
        .child(div().text_color(rgb(c_red())).child(format!("−{removed}")))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn finds_the_first_match_ignoring_case() {
        assert_eq!(match_range("Add a Login page", "login"), Some(6..11));
        assert_eq!(match_range("login, then Login", "login"), Some(0..5));
        assert_eq!(match_range("Add a page", "login"), None);
        assert_eq!(match_range("Add a page", ""), None);
    }

    #[test]
    fn leaves_text_unmarked_when_lowercasing_changes_its_length() {
        // "İ" lowercases to two characters, so byte offsets would not line up.
        assert_eq!(match_range("İstanbul login", "login"), None);
    }
}
