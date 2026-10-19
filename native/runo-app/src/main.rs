mod agent;
mod agent_pick;
mod agentcmds;
mod alerts;
mod applog;
mod approvals;
mod archdraw;
mod architecture;
mod askbar;
mod assistant;
mod browser;
mod browser_engine;
mod caps;
mod chatctl;
mod cleanup;
mod cliview;
mod connections;
mod connections_ui;
mod copies;
mod cove;
mod ctxmenu;
mod design;
mod designdraw;
mod desktop_notify;
mod detach;
mod files;
mod fileview;
mod fixes;
mod gemini;
mod git;
mod github;
mod handoff;
mod health;
mod history;
mod inbox;
mod install;
mod keymap;
mod mcp;
mod memory_mcp;
mod modelcheck;
mod models;
mod notes;
mod pbfile;
mod pbrunner;
mod pbshare;
mod persist;
mod phone;
mod playbooks;
mod projects;
mod pty;
mod review;
mod schedule;
mod sessions;
mod settings;
mod settings_ui;
mod shell;
mod shellenv;
mod ship;
mod signin;
mod store;
mod stream;
mod terminal;
mod tour;
mod turns;
mod updater;
mod usage;
mod usage_ui;
mod workspace;

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
use settings_ui::{McpForm, SettingsPage};
use shell::Mode;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;
use store::{Message, Note, Project, Role, Session, State, Status, StepState};

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
        SaveFile,
        Dismiss,
        QuickOpen,
        ToggleView,
        GoHome,
        GoSessions,
        GoDesign,
        GoArchitecture,
        GoPlaybooks,
        GoReviews,
        GoReview,
        GoHistory,
        GoNotes,
        GoInbox,
        CloseTab,
        NextTab,
        PrevTab,
        FocusComposer,
        ArchiveSession,
        ContextMenu,
        ShowTour,
        AskRuno,
        Quit
    ]
);

/// Global shortcuts, as set in Settings > Keybindings.
fn bind_keys(cx: &mut App) {
    keymap::install(cx);
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum View {
    Home,
    Design,
    Architecture,
    Session,
    NewSession,
    Inbox,
    AllSessions,
    History,
    Worktrees,
    Settings,
    Notes,
    Activity,
    Automations,
    GitHub,
    Editor,
    /// Changes waiting to be reviewed and shipped, one session at a time.
    Review,
}

/// What a quick-open entry does when chosen.
#[derive(Clone)]
enum Jump {
    /// Handled by the entry's own Action.
    Action,
    View(View),
    Session(u64),
    File(String),
    Tour,
    /// Run a playbook in the current project.
    Playbook(u64),
    /// Switch the top bar to a project.
    Project(u64),
    AddProject,
    Settings(SettingsPage),
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum RightTab {
    Changes,
    Files,
}

struct RunoApp {
    state: State,
    demo: bool,
    selected: Option<u64>,
    view: View,
    right_tab: RightTab,
    /// The Browser is open as a tab beside the session tabs, and whether it is the one shown.
    browser_open: bool,
    browser_main: bool,
    /// Interactive shells, across all sessions.
    shells: Vec<pty::Shell>,
    /// The shell tab shown in the terminal panel, when one is selected.
    shell_tab: Option<u64>,
    next_shell: u64,
    /// Quick commands run in their own terminal tabs: shell id → the run.
    tasks: HashMap<u64, terminal::Task>,
    /// Agent CLIs open in the CLI view, by session.
    cli_runs: HashMap<u64, cliview::CliRun>,
    /// The agent whose install confirm card is open.
    install_confirm: Option<&'static str>,
    /// A copy of an agent's CLI waiting for the person to confirm its removal.
    remove_copy: Option<fixes::RemoveCopy>,
    /// Terminal tabs signing an agent in: shell id → agent.
    sign_ins: HashMap<u64, &'static str>,
    /// The session whose failed turn shows the model list.
    fix_models: Option<u64>,
    project_form: Option<projects::ProjectForm>,
    project_name: Entity<InputState>,
    /// The project form's worktree setup command.
    project_setup: Entity<InputState>,
    /// The New project popup's folder path, typed or filled in by Browse.
    project_path: Entity<InputState>,
    /// The project form's repository URL, when cloning.
    clone_url: Entity<InputState>,
    /// A freshly installed CLI found off PATH, to save as the agent's binary.
    pending_binary: Option<(&'static str, String)>,
    cli_focus: FocusHandle,
    /// Frames to wait before focusing a CLI that just opened.
    focus_cli: u8,
    /// The session whose view last switched between Chat and CLI, and when.
    view_changed: Option<(u64, std::time::Instant)>,
    term_focus: FocusHandle,
    /// Flips each time Expand is pressed, so the panel animates.
    term_tall: bool,
    /// The terminal panel's height, and the height it is easing from.
    term_h: f32,
    term_from: f32,
    /// While the panel's top edge is dragged: the pointer's y and the height at the start.
    term_drag: Option<(f32, f32)>,
    cell_w: f32,
    /// Font family and size the terminal cell width was measured for.
    cell_key: Option<(SharedString, u32)>,
    focus_shell: u8,
    menu: Option<String>,
    /// The agent whose models show beside the composer's agent menu.
    menu_agent: Option<usize>,
    /// The mode the last frame showed, so moving between modes swaps the page instead of
    /// sliding its panels open.
    shown_mode: Option<Mode>,
    settings_page: SettingsPage,
    settings_search: Entity<InputState>,
    settings_inputs: HashMap<String, Entity<InputState>>,
    settings_areas: HashMap<String, Entity<TextareaState>>,
    /// The open form for adding or editing one of Runo's MCP servers.
    mcp_form: Option<McpForm>,
    /// Settings > Worktrees > Clean up.
    cleanup: settings_ui::CleanupView,
    settings_menu: Option<String>,
    discovered: Option<Discovered>,
    applied_dark: Option<bool>,
    applied_font: Option<&'static str>,
    applied_glass: Option<bool>,
    applied_enter: Option<bool>,
    changes: Vec<git::Change>,
    dir_is_git: bool,
    selected_file: Option<String>,
    diff: String,
    files: Vec<String>,
    /// The file shown in the Files tab (or the main area), and its editor.
    open_file: Option<fileview::OpenFile>,
    pending_file: Option<String>,
    file_editor: Entity<gpui_kit::component::input::EditorState>,
    file_preview: bool,
    file_dirty: bool,
    /// The editor's text while it has unsaved changes.
    file_text: String,
    /// Waiting on a Save / Discard answer before opening another file or closing.
    file_leave: Option<fileview::Leave>,
    /// The open file changed on disk while it had unsaved changes.
    file_conflict: bool,
    /// Unsaved edits of sessions switched away from, with their files.
    unsaved: HashMap<u64, (fileview::OpenFile, String)>,
    /// Where the main-area editor returns to.
    editor_back: View,
    /// The view shown behind the New session dialog.
    new_back: View,
    terminal: HashMap<u64, Vec<String>>,
    handles: HashMap<Target, Handle>,
    tx: Sender<Event>,
    chat_scroll: ScrollHandle,
    /// Recent error output of each session's run, for failure messages and the log.
    stderr: HashMap<u64, Vec<String>>,
    /// Parsers for runs that print JSON rather than plain text.
    streams: HashMap<u64, stream::Parser>,
    search: Entity<InputState>,
    prompt: Entity<TextareaState>,
    new_task: Entity<TextareaState>,
    branch: Entity<InputState>,
    address: Entity<InputState>,
    browser_recent: Vec<String>,
    browser_ports: Vec<u16>,
    browser_scanning: bool,
    /// Dev-server ports found where each session on a connection runs, or why not.
    remote_ports: HashMap<u64, Result<Vec<u16>, String>>,
    remote_scanning: std::collections::HashSet<u64>,
    /// Ports on connections forwarded to this machine: (session, port there).
    forwards: HashMap<(u64, u16), connections::Forward>,
    forwarding: Option<(u64, u16)>,
    /// The headless Chromium behind the Browser tab, started on first use.
    browser: Option<std::sync::Arc<browser_engine::Engine>>,
    browser_view: browser::BrowserView,
    browser_focus: FocusHandle,
    new_provider: usize,
    new_project: Option<u64>,
    new_isolated: bool,
    /// Run the new session in its own Cove VM.
    /// Where the next session runs: a connection id, or 0 for this machine.
    new_connection: u64,
    /// Race mode in the New session form, and the agents picked for it.
    race_mode: bool,
    /// The New session dialog lists every agent, not just the main ones.
    new_more_agents: bool,
    /// The branch a new isolated session starts from; None for the default branch.
    new_base: Option<String>,
    /// The New session form's project and its branches, for the base branch menu.
    new_branches: Option<sessions::Branches>,
    /// Search, filters and sort of the Sessions page, and matches in conversations.
    finder: sessions::Finder,
    /// Newer releases of installed agents, from the last update check.
    updates: Vec<alerts::AgentUpdate>,
    /// Agent updates and installs running now, just finished, or failed.
    update_runs: HashMap<&'static str, alerts::UpdateRun>,
    update_checked: Option<std::time::Instant>,
    /// Popups the person hid ("s:<session>:<message>" or "u:<agent>:<version>").
    alerts_hidden: std::collections::HashSet<String>,
    alerts_expanded: bool,
    /// Permission requests agents are waiting on mid-turn.
    approvals: Vec<approvals::Request>,
    /// Tools the person always allowed for a session, answered without asking again.
    always_allowed: std::collections::HashSet<(u64, String)>,
    /// Tools the person refused during a session's turn; they don't come back as
    /// "Allow" cards when the turn ends.
    refused: HashMap<u64, Vec<String>>,
    /// Switching agents, waiting for the person to confirm.
    switch_plan: Option<handoff::SwitchPlan>,
    switch_preview: bool,
    /// The open right-click menu: where, and on what.
    ctx_menu: Option<(Point<Pixels>, ctxmenu::CtxTarget)>,
    /// The first-run tour's current step.
    tour: Option<usize>,
    /// Where the tour's highlighted parts are drawn this frame, and where they were last frame.
    anchors: tour::Anchors,
    tour_rects: HashMap<&'static str, Bounds<Pixels>>,
    /// Past Claude and Codex conversations found on this machine.
    history: Vec<history::Past>,
    /// The history list is being read.
    history_loading: bool,
    /// Change counts (files, added, removed) of each session racing the selected one.
    race_stats: HashMap<u64, (usize, usize, usize)>,
    new_race: Vec<usize>,
    expanded: Vec<u64>,
    /// Sessions open as tabs in the work area, in tab order.
    tabs: Vec<u64>,
    tab_scroll: ScrollHandle,
    /// The terminal panel's Output box, so it can follow new output.
    term_scroll: ScrollHandle,
    /// The session and number of lines the Output box last showed, to notice new ones.
    term_seen: std::cell::Cell<(u64, usize)>,
    /// The project in the top bar; the open session's project unless the person switched.
    project: Option<u64>,
    /// The row picked with the arrow keys in the sidebar, Home or the Inbox.
    cursor: Option<u64>,
    /// The right-click menu entry picked with the arrow keys.
    ctx_index: Option<usize>,
    sidebar_width: f32,
    /// The sidebar's edge is being dragged.
    resizing_sidebar: bool,
    /// Width of the right panel (Changes, Files, terminal), dragged from its left edge.
    panel_width: f32,
    resizing_panel: bool,
    /// The panel was hidden because the window got narrow, not by the person.
    panel_auto_hidden: bool,
    /// Home lists every project instead of the one in the top bar.
    home_all: bool,
    /// Where the window is, saved with the layout.
    window_place: Option<store::WindowPlace>,
    /// When Escape was last pressed in a terminal; a second press leaves it.
    term_escape: Option<std::time::Instant>,
    /// Agent replies whose step list is expanded, by (session, message index).
    open_steps: std::collections::HashSet<(u64, usize)>,
    show_sidebar: bool,
    /// When each panel last opened or closed, so it can slide instead of snapping.
    panel_anim: HashMap<&'static str, (bool, std::time::Instant)>,
    /// Whether the last frame showed Settings; panels snap rather than slide across that change.
    was_settings: bool,
    /// When Settings last opened or closed; views appear without motion right after.
    settings_switch_at: Option<std::time::Instant>,
    /// Change counts (files, added, removed) per session, for the sidebar.
    stats: HashMap<u64, (usize, usize, usize)>,
    confirm_delete: Option<u64>,
    /// Diff line a review note is being written for: path, line number, code.
    note_at: Option<(String, usize, String)>,
    note_input: Entity<InputState>,
    /// Tasks waiting for their worktree setup command to finish.
    pending_start: HashMap<u64, String>,
    /// Terminal tabs running a setup command: shell id → session.
    setups: HashMap<u64, u64>,
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
    /// The last notice written to the log, so each is written once.
    logged_notice: Option<String>,
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
    /// Runs stopped because their CLI waited for a browser sign-in.
    signed_out: std::collections::HashSet<u64>,
    /// Sessions whose agent reported an error in its output this turn.
    stream_failed: std::collections::HashSet<u64>,
    /// Files changed by each session's latest turn, once it finished (for Undo / Keep).
    turn_files: HashMap<u64, usize>,
    /// Images attached to each session's unsent prompt.
    attachments: HashMap<u64, Vec<String>>,
    /// Images that go with a new session's first prompt.
    launch_images: HashMap<u64, Vec<String>>,
    /// Context from Design or Architecture for the session the New session dialog starts.
    new_context: Option<shell::StartContext>,
    /// The model for the next session made, picked on a canvas's AI bar.
    new_model: Option<String>,
    /// The Design and Architecture pages.
    design: design::DesignPage,
    /// The Runo assistant in the corner, when turned on in Settings.
    assistant: assistant::Assistant,
    arch: architecture::ArchPage,
    /// Each session's unsent prompt while another session is on screen, and the session
    /// whose text the composer holds now.
    drafts: HashMap<u64, String>,
    draft_for: Option<u64>,
    /// Answers picked so far for an agent's several questions, by (session, message).
    question_picks: HashMap<(u64, usize), Vec<Option<String>>>,
    /// When closing the window was asked while agents ran, so a second close goes through.
    close_armed: Option<std::time::Instant>,
    /// Dropped files waiting to be added to the prompt as @mentions.
    pending_mentions: Vec<String>,
    /// The `@query` being typed in the composer.
    mention: Option<String>,
    /// The `/query` typed at the start of the prompt, which lists playbooks.
    slash: Option<String>,
    /// The agent's own commands for the `/` menu, read for one session when the menu opens.
    agent_commands: Option<(u64, Vec<agentcmds::Command>)>,
    /// A sent message being edited in the composer: (session, message index).
    editing: Option<(u64, usize)>,
    /// A commit message written by the agent, waiting to be put in the commit box.
    commit_suggestion: Option<String>,
    writing_commit: bool,
    /// Diff view options: every changed file at once, whole files, wrapped lines.
    diff_all: bool,
    diff_whole: bool,
    diff_wrap: bool,
    all_diffs: Vec<(String, String)>,
    /// Folded unchanged runs the person opened, by (path, first row).
    diff_expanded: std::collections::HashSet<(String, usize)>,
    notes_input: Entity<TextareaState>,
    notes_project: Option<u64>,
    /// True while the notes box is filled programmatically, so it is not saved back.
    loading_notes: bool,
    /// Remove project dialog, and its confirm-by-name field.
    remove_project: Option<projects::RemoveProject>,
    remove_name: Entity<InputState>,
    /// A playbook picked from the `/` menu; the next message runs it.
    picked_playbook: Option<u64>,
    /// The shortcut being recorded in Settings > Keybindings, and why keys were refused.
    key_capture: Option<&'static str>,
    key_note: Option<String>,
    /// The playbook editor while it is open, its extra fields and the page's search.
    pb_editor: Option<playbooks::PlaybookEditor>,
    auto_desc: Entity<InputState>,
    pb_query: Entity<InputState>,
    pb_confirm_delete: Option<u64>,
    /// A read-only playbook session whose plan is waiting for "Allow edits".
    pb_widen: Option<u64>,
    /// Playbooks agents asked to start, waiting for the person to agree.
    pb_asks: Vec<store::PlaybookRequest>,
    /// Installing playbooks from a link, repository or file, and its source box.
    pb_install: Option<pbshare::PlaybookInstall>,
    pb_source: Entity<InputState>,
    /// The form asking for a playbook's inputs before it runs.
    pb_run: Option<pbshare::PlaybookRun>,
    /// Inputs filled in for the next start of a playbook, and for the picked `/` playbook.
    pb_values: Option<(u64, Vec<(String, String)>)>,
    picked_values: Vec<(String, String)>,
    /// Project notes: write, read or both, and the search box.
    notes_mode: notes::NotesMode,
    notes_query: Entity<InputState>,
    auto_name: Entity<InputState>,
    auto_prompt: Entity<TextareaState>,
    auto_every: Entity<InputState>,
    auto_project: Option<u64>,
    auto_provider: usize,
    gh: github::GhData,
    /// Results of background git and gh work, applied on the UI thread.
    ui_tx: Sender<ship::UiJob>,
    /// Bumped by each refresh, so an older scan finishing late is dropped.
    refresh_seq: u64,
    /// The selected session's branch against its base branch.
    branch_info: ship::BranchInfo,
    /// The Changes panel shows everything since the branch left its base, commits included.
    diff_base: bool,
    /// What the shown diffs compare with: HEAD or the branch's merge base.
    diff_against: String,
    /// Each session's pull request, and the sessions whose PR is being looked up.
    pr_status: HashMap<u64, github::PrStatus>,
    pr_loading: std::collections::HashSet<u64>,
    /// The merge methods each project's GitHub repository allows, read once a run, and
    /// the projects being read.
    merge_methods: HashMap<u64, Vec<github::MergeMethod>>,
    merge_methods_loading: std::collections::HashSet<u64>,
    pr_form: Option<ship::PrForm>,
    pr_title: Entity<InputState>,
    pr_base: Entity<InputState>,
    pr_branch: Entity<InputState>,
    pr_body: Entity<TextareaState>,
    /// A git step running for a session (commit, push, update, merge), by session.
    git_busy: HashMap<u64, String>,
    /// A git step waiting for confirmation.
    confirm_git: Option<ship::Confirm>,
    /// A racer just kept, offered a merge or pull request.
    kept: Option<u64>,
    /// Sessions whose worktree is still being made, with the turns waiting for it.
    preparing: HashMap<u64, Vec<ship::Deferred>>,
    /// The Files tab's filter, its search-in-files results, and the open folders.
    file_query: Entity<InputState>,
    file_hits: Option<(Vec<git::Hit>, bool)>,
    file_searching: bool,
    files_open: std::collections::HashSet<String>,
    /// Holds off system sleep while agents run.
    awake: Option<std::process::Child>,
    /// Saving, quiet agents, limits, slots, crash reports and Runo updates.
    health: health::Health,
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
        unstaged: true,
        ..Default::default()
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

/// One row of a composer menu: an optional lead, a label with an optional description
/// under it, and a check when it is the current choice.
fn menu_row(
    id: impl Into<ElementId>,
    label: String,
    desc: String,
    selected: bool,
    lead: Option<AnyElement>,
) -> Stateful<Div> {
    div()
        .id(id)
        // Rows keep their full height: a menu column with a fixed height scrolls instead
        // of squeezing them, which would draw each description over the next row.
        .flex_none()
        .min_h(px(28.))
        .px_2()
        .py(px(5.))
        .flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .cursor_pointer()
        .text_size(px(13.))
        .hover(|el| el.bg(tint(c_active())))
        .when_some(lead, |el, lead| el.child(lead))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .v_flex()
                .child(div().whitespace_normal().child(label))
                .when(!desc.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .max_w(px(280.))
                            .whitespace_normal()
                            .child(desc),
                    )
                }),
        )
        .when(selected, |el| {
            el.child(icon(IconName::Check, 13., c_text()))
        })
}

/// The model a session's next run uses, by name: the one picked, else the agent's default
/// when it says which, else empty.
fn run_model(provider: &str, sel: &HashMap<String, String>) -> String {
    let picked = sel.get("model").cloned().unwrap_or_default();
    if picked.is_empty() {
        return caps::get(provider).default_model;
    }
    caps::controls(provider, sel)
        .into_iter()
        .find(|c| c.id == "model")
        .and_then(|c| c.choices.into_iter().find(|ch| ch.value == picked))
        .map(|ch| ch.label)
        .unwrap_or(picked)
}

/// A short line naming what the next message runs on: "Codex · gpt-5.5 · High reasoning
/// · Fast". Settings left at their defaults are left out, except the model.
fn run_note(provider: &agent::Provider, sel: &HashMap<String, String>) -> String {
    let mut parts = vec![provider.name.to_string()];
    let model = run_model(provider.id, sel);
    parts.push(if model.is_empty() {
        "Default model".to_string()
    } else {
        model
    });
    for control in caps::controls(provider.id, sel) {
        let value = sel.get(control.id).cloned().unwrap_or_default();
        if value.is_empty() || !matches!(control.id, "effort" | "tier") {
            continue;
        }
        let Some(choice) = control.choices.iter().find(|c| c.value == value) else {
            continue;
        };
        parts.push(if control.id == "effort" {
            format!("{} {}", choice.label, control.label.to_lowercase())
        } else {
            choice.label.clone()
        });
    }
    parts.join(" · ")
}

/// The surface of a composer menu: solid, raised and scrolling when long.
fn menu_box(key: &str, list: impl IntoElement) -> Div {
    div()
        .rounded_lg()
        .border_1()
        .border_color(rgb(c_border()))
        .bg(rgb(c_raised()))
        .shadow_lg()
        .occlude()
        .child(
            div()
                .id(SharedString::from(format!("popup-scroll-{key}")))
                .max_h(px(340.))
                .overflow_y_scroll()
                .child(list),
        )
}

/// Fade in while sliding up a few pixels: cards and toasts arriving.
fn rise<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>, ms: u64) -> AnyElement {
    el.animated(
        id,
        Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d).top(px(8. * (1. - d))),
    )
}

/// Slide in from the right: panes opening beside the main view.
fn slide_in<E: IntoElement + Styled + 'static>(
    el: E,
    id: impl Into<ElementId>,
    ms: u64,
) -> AnyElement {
    el.animated(
        id,
        Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint()),
        |el, d| el.opacity(d).left(px(24. * (1. - d))),
    )
}

/// How long panels take to slide open or closed.
const PANEL_MS: u64 = 280;

/// A side panel that slides open and closed: its width animates while its content keeps
/// its full size and is clipped, so it reads as the panel moving, not squashing.
/// `motion` is the panel's last change, when that was recent.
fn panel_slide(
    id: &str,
    motion: Option<(bool, std::time::Instant)>,
    open: bool,
    width: f32,
    from_left: bool,
    content: impl FnOnce() -> AnyElement,
) -> Option<AnyElement> {
    if !open && motion.is_none() {
        return None;
    }
    let wrapper = div()
        .flex_none()
        .h_full()
        .overflow_hidden()
        .flex()
        .when(from_left, |el| el.justify_end())
        .child(
            div()
                .w(px(width))
                .h_full()
                .flex_none()
                .flex()
                .child(content()),
        );
    Some(match motion {
        Some((opening, at)) => wrapper.w(px(if opening { width } else { 0. })).animated(
            SharedString::from(format!("{id}-{opening}-{:?}", at)),
            Animation::new(Duration::from_millis(PANEL_MS)).with_easing(ease_in_out_cubic),
            move |el, d| {
                let shown = if opening { d } else { 1. - d };
                el.w(px(width * shown)).opacity(0.25 + 0.75 * shown)
            },
        ),
        None => wrapper.w(px(width)).into_any_element(),
    })
}

/// Smooth start and end, for things that move across the screen.
fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// Runo's app icon, for showing inside the app.
fn app_icon() -> std::sync::Arc<Image> {
    static ICON: std::sync::OnceLock<std::sync::Arc<Image>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        std::sync::Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../assets/icon/png/runo-128.png").to_vec(),
        ))
    })
    .clone()
}

/// A friendly empty state: a softly breathing icon, a title and a hint.
fn empty_hint(id: &'static str, name: IconName, title: &str, body: &str) -> AnyElement {
    rise(
        div()
            .w_full()
            .py_10()
            .v_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .size(px(48.))
                    .mb_1()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(tint(c_subtle()))
                    .border_1()
                    .border_color(rgb(c_border()))
                    .child(
                        icon(name, 20., c_secondary()).animated(
                            (id, 1usize),
                            Animation::new(Duration::from_millis(2400))
                                .repeat()
                                .with_easing(pulsating_between(0.55, 1.)),
                            |el, t| el.opacity(t),
                        ),
                    ),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_text()))
                    .child(title.to_string()),
            )
            .child(
                div()
                    .max_w(px(360.))
                    .text_center()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child(body.to_string()),
            ),
        (id, 0usize),
        420,
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

/// "Working" row: a small orbit (a comet circling a breathing core) and the label with
/// a soft accent glint sweeping across it, then the elapsed time. With animations off
/// both hold still: the comet rests on the ring and the label is plain.
fn working_indicator(elapsed: Option<String>) -> impl IntoElement {
    const BOX: f32 = 16.;
    const RING: f32 = 6.;
    // Brighter in dark mode, where the accent alone sits too close to the canvas.
    let accent = if is_dark() {
        mix(c_blue(), 0xffffff, 0.6)
    } else {
        c_blue()
    };
    // Where a point `t` of the way round the orbit sits, as the top-left of a dot of `size`.
    let at = |t: f32, size: f32| {
        let a = (t - 0.25) * std::f32::consts::TAU;
        let c = BOX / 2. - size / 2.;
        (px(c + RING * a.cos()), px(c + RING * a.sin()))
    };
    let mut orbit = div()
        .relative()
        .flex_none()
        .size(px(BOX))
        .child(
            div()
                .absolute()
                .left(px(BOX / 2. - RING))
                .top(px(BOX / 2. - RING))
                .size(px(RING * 2.))
                .rounded_full()
                .border_1()
                .border_color(rgb(mix(accent, c_canvas(), 0.22))),
        )
        .child(
            div()
                .absolute()
                .left(px(BOX / 2. - 2.))
                .top(px(BOX / 2. - 2.))
                .size(px(4.))
                .rounded_full()
                .bg(rgb(accent))
                .animated(
                    "working-core",
                    Animation::new(Duration::from_millis(1600))
                        .repeat()
                        .with_easing(pulsating_between(0.35, 1.)),
                    |el, t| el.opacity(t),
                ),
        );
    // The comet: a head and a fading tail a few steps behind it.
    for (i, (lag, size, alpha)) in [(0., 3.5, 1.), (0.07, 2.6, 0.55), (0.14, 1.8, 0.28)]
        .into_iter()
        .enumerate()
    {
        let (x, y) = at(-lag, size);
        orbit = orbit.child(
            div()
                .absolute()
                .left(x)
                .top(y)
                .size(px(size))
                .rounded_full()
                .bg(rgb(accent))
                .opacity(alpha)
                .animated(
                    ("working-comet", i),
                    Animation::new(Duration::from_millis(1100))
                        .repeat()
                        .with_easing(ease_in_out_cubic),
                    move |el, t| {
                        let (x, y) = at(t - lag, size);
                        el.left(x).top(y)
                    },
                ),
        );
    }
    // The label, one letter at a time so a glint can pass along it.
    let word = "Working";
    let base = c_secondary();
    let n = word.chars().count() as f32;
    let mut label = div().flex().items_center();
    for (i, ch) in word.chars().enumerate() {
        label = label.child(div().text_color(rgb(base)).child(ch.to_string()).animated(
            ("working-glint", i),
            Animation::new(Duration::from_millis(2000)).repeat(),
            move |el, t| {
                // The glint travels from before the first letter to past the last one.
                let pos = t * (n + 6.) - 3.;
                let near = (1. - (i as f32 - pos).abs() / 2.5).max(0.);
                el.text_color(rgb(mix(accent, base, near * near)))
            },
        ));
    }
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(orbit)
        .child(label)
        .when_some(elapsed, |row, e| {
            row.child(div().text_color(rgb(c_muted())).child(format!("· {e}")))
        })
}

/// Most files listed for a folder that is not a git repository; the Files tab says so
/// when a folder has more.
const PLAIN_FILES_MAX: usize = 20_000;

/// File listing for folders that are not git repositories (capped at `PLAIN_FILES_MAX`).
fn plain_files(dir: &str) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if out.len() >= PLAIN_FILES_MAX {
                return;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target") {
                continue;
            }
            if path.is_dir() {
                if depth < 12 {
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

/// "38k tokens · 19% context" for an agent turn.
fn usage_label(usage: &store::Usage) -> String {
    let k = |n: u64| {
        if n >= 1000 {
            format!("{:.0}k", n as f64 / 1000.0)
        } else {
            n.to_string()
        }
    };
    let window = if usage.window > 0 {
        usage.window
    } else {
        200_000
    };
    let pct = (usage.context() as f64 / window as f64 * 100.0).round() as u64;
    format!("{} tokens · {pct}% context", k(usage.context()))
}

/// Runs of more than 8 unchanged lines, as (first, end) row indexes, keeping 3 lines of
/// context next to each change so long unchanged stretches can be folded away.
fn collapsible_runs(rows: &[git::DiffRow]) -> Vec<(usize, usize)> {
    const KEEP: usize = 3;
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if rows[i].kind != git::LineKind::Context {
            i += 1;
            continue;
        }
        let start = i;
        while i < rows.len() && rows[i].kind == git::LineKind::Context {
            i += 1;
        }
        let lead = if start == 0 { 0 } else { KEEP };
        let tail = if i == rows.len() { 0 } else { KEEP };
        if i - start > lead + tail + 2 {
            out.push((start + lead, i - tail));
        }
    }
    out
}

fn icon(name: IconName, size: f32, color: u32) -> Icon {
    Icon::new(name).size(px(size)).text_color(rgb(color))
}

impl RunoApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        caps::refresh();
        // Find MCP servers in every app's config, so shared ones reach the first run.
        std::thread::spawn(|| {
            settings::discover(None);
        });
        let (tx, rx) = channel::<Event>();
        let (ui_tx, ui_rx) = channel::<ship::UiJob>();
        // Background git work hands its results back here.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
            let mut jobs = Vec::new();
            loop {
                match ui_rx.try_recv() {
                    Ok(job) => jobs.push(job),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            if jobs.is_empty() {
                continue;
            }
            let alive = this.update(cx, |this, cx| {
                for job in jobs {
                    job(this, cx);
                }
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
        let mut ticks = 0u32;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            ticks = ticks.wrapping_add(1);
            let events: Vec<Event> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            if events.is_empty() {
                // Repaint about once a second while agents run, for their elapsed time.
                let tick = ticks.is_multiple_of(20);
                let repaint = caps::take_updated();
                // Every ~2s, re-read git status for the session being watched while its
                // agent works, so the Changes panel fills in as files are edited.
                let rescan = ticks.is_multiple_of(40);
                // Every ~10s when idle, so terminal git commands and outside edits show
                // up; the pull request's checks every minute.
                let idle_scan = ticks.is_multiple_of(200);
                let pr_check = ticks % 1200 == 600;
                let check = ticks % 1200 == 2;
                // Ten seconds in, once agents are detected, then every five minutes: each agent's plan usage for the status bar.
                if ticks % 6000 == 200 {
                    usage::refresh(true);
                }
                let alive = this.update(cx, |this, cx| {
                    if check && !this.demo {
                        this.maybe_check_updates();
                    }
                    // Every ~5s, look for dev servers so the header's Browser button
                    // appears once the app is being served, and goes when it stops.
                    if ticks % 100 == 50
                        && !this.demo
                        && this.view == View::Session
                        && this.selected.is_some()
                        && cx.global::<AppSettings>().browser_enabled
                    {
                        this.scan_ports();
                    }
                    let watching = this.view == View::Session
                        && this
                            .selected
                            .is_some_and(|id| this.started.contains_key(&id));
                    let on_session = this.view == View::Session && this.selected.is_some();
                    if pr_check && on_session && !this.demo {
                        if let Some(id) = this.selected {
                            this.load_pr_status(id);
                        }
                    }
                    if tick && !this.demo {
                        let before = (this.file_conflict, this.pending_file.is_some());
                        this.watch_open_file();
                        if before != (this.file_conflict, this.pending_file.is_some()) {
                            cx.notify();
                        }
                    }
                    if ((rescan && watching) || (idle_scan && on_session)) && !this.demo {
                        this.refresh();
                        cx.notify();
                    } else if repaint
                        || (tick && (!this.started.is_empty() || this.info.is_some()))
                        || (ticks.is_multiple_of(5)
                            && this.update_runs.values().any(|r| r.failed.is_none()))
                    {
                        this.prune_update_runs();
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
        {
            let mut conns = cx.global::<AppSettings>().connections.clone();
            if connections::migrate(&mut state.sessions, &mut conns) {
                settings::update(cx, |s| s.connections = conns);
                state.save();
            }
        }
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
        let project_name = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let file_editor = Self::new_file_editor(window, cx);
        let file_sub = cx.subscribe_in(&file_editor, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.file_changed(cx);
                cx.notify();
            }
        });
        let address = cx.new(|cx| InputState::new(window, cx).placeholder("localhost:3000"));
        let address_sub = cx.subscribe_in(&address, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_address(true, cx);
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
        let (design, design_subs) = design::DesignPage::new(window, cx);
        let (assistant, assistant_subs) = assistant::Assistant::new(window, cx);
        let (arch, arch_subs) = architecture::ArchPage::new(window, cx);
        let notes_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Write anything worth remembering about this project…")
        });
        let notes_sub = cx.subscribe_in(&notes_input, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.notes_changed(cx);
            }
        });
        let auto_name = cx.new(|cx| InputState::new(window, cx).placeholder("Check dependencies"));
        let auto_prompt = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(
                "What the agent should do, as you would write it in a message. For example: Check for outdated dependencies, update the safe ones, run the tests and list anything that needs a decision.",
            )
        });
        let auto_every =
            cx.new(|cx| InputState::new(window, cx).placeholder("Empty runs it by hand only"));
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
                    this.update_mention(cx);
                    cx.notify();
                    false
                }
                _ => false,
            };
            if send {
                // With @mention or /playbook suggestions open, Enter picks the first one;
                // ⌘/Ctrl+Enter on a playbook starts it as its own session instead.
                if let Some(first) = this.slash_matches().into_iter().next() {
                    let new_session = matches!(
                        event,
                        InputEvent::PressEnter {
                            secondary: true,
                            ..
                        }
                    );
                    this.pick_slash(first, new_session, window, cx);
                } else if let Some(first) = this.mention_matches().into_iter().next() {
                    this.pick_mention(&first, window, cx);
                } else {
                    this.send_message(window, cx);
                }
                cx.notify();
            }
        });

        let mut terminal = HashMap::new();
        if demo {
            terminal.insert(
                11,
                vec![
                    "$ cargo check -p runo".to_string(),
                    "    Finished dev profile in 0.97s".to_string(),
                    "$ git status --short".to_string(),
                    " M src/sidebar/navigation.rs".to_string(),
                ],
            );
        }
        let mut app = Self {
            expanded: state.projects.iter().map(|p| p.id).collect(),
            tabs: Vec::new(),
            tab_scroll: ScrollHandle::new(),
            term_scroll: ScrollHandle::new(),
            term_seen: Default::default(),
            project: None,
            cursor: None,
            ctx_index: None,
            sidebar_width: SIDEBAR_WIDTH,
            resizing_sidebar: false,
            panel_width: PANEL_WIDTH,
            resizing_panel: false,
            panel_auto_hidden: false,
            home_all: false,
            window_place: None,
            term_escape: None,
            open_steps: Default::default(),
            show_sidebar: true,
            panel_anim: HashMap::new(),
            was_settings: false,
            settings_switch_at: None,
            stats: HashMap::new(),
            new_project: state.projects.first().map(|p| p.id),
            view: View::Session,
            state,
            demo,
            selected,
            right_tab: RightTab::Changes,
            browser_open: false,
            browser_main: false,
            shells: Vec::new(),
            shell_tab: None,
            next_shell: 0,
            tasks: HashMap::new(),
            cli_runs: HashMap::new(),
            install_confirm: None,
            remove_copy: None,
            sign_ins: HashMap::new(),
            fix_models: None,
            project_form: None,
            project_name,
            project_setup: cx
                .new(|cx| InputState::new(window, cx).placeholder("npm install (optional)")),
            project_path: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Type or paste a folder path, such as ~/code/app")
            }),
            clone_url: cx.new(|cx| {
                InputState::new(window, cx).placeholder("https://github.com/owner/repo.git")
            }),
            pending_binary: None,
            cli_focus: cx.focus_handle(),
            focus_cli: 0,
            view_changed: None,
            term_focus: cx.focus_handle(),
            term_tall: false,
            term_h: terminal::TERM_HEIGHT,
            term_from: terminal::TERM_HEIGHT,
            term_drag: None,
            cell_w: 7.2,
            cell_key: None,
            focus_shell: 0,
            menu: None,
            menu_agent: None,
            shown_mode: None,
            settings_page: SettingsPage::General,
            settings_search,
            settings_inputs: HashMap::new(),
            settings_areas: HashMap::new(),
            mcp_form: None,
            cleanup: Default::default(),
            settings_menu: None,
            discovered: None,
            applied_dark: None,
            applied_font: None,
            applied_glass: None,
            applied_enter: None,
            changes: Vec::new(),
            dir_is_git: true,
            selected_file: None,
            diff: String::new(),
            files: Vec::new(),
            open_file: None,
            pending_file: None,
            file_editor,
            file_preview: false,
            file_dirty: false,
            file_text: String::new(),
            file_leave: None,
            file_conflict: false,
            unsaved: HashMap::new(),
            editor_back: View::Session,
            new_back: View::Session,
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
            branch: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Branch (optional, named after the task)")
            }),
            address,
            browser_recent: Vec::new(),
            browser_ports: Vec::new(),
            browser_scanning: false,
            remote_ports: HashMap::new(),
            remote_scanning: Default::default(),
            forwards: HashMap::new(),
            forwarding: None,
            browser: None,
            browser_view: browser::BrowserView::default(),
            browser_focus: cx.focus_handle(),
            new_provider: agent::PROVIDERS
                .iter()
                .position(|p| p.id == cx.global::<AppSettings>().default_provider)
                .unwrap_or(0),
            new_isolated: true,
            new_connection: 0,
            new_more_agents: false,
            new_base: None,
            new_branches: None,
            finder: sessions::Finder::new(window, cx),
            updates: Vec::new(),
            update_runs: HashMap::new(),
            update_checked: None,
            alerts_hidden: Default::default(),
            alerts_expanded: false,
            approvals: Vec::new(),
            always_allowed: Default::default(),
            refused: HashMap::new(),
            switch_plan: None,
            switch_preview: false,
            ctx_menu: None,
            tour: (!cx.global::<AppSettings>().tour_seen).then_some(0),
            anchors: Default::default(),
            tour_rects: HashMap::new(),
            race_mode: false,
            history: Vec::new(),
            history_loading: false,
            race_stats: HashMap::new(),
            new_race: Vec::new(),
            confirm_delete: None,
            note_at: None,
            note_input: note_input.clone(),
            pending_start: HashMap::new(),
            setups: HashMap::new(),
            palette: None,
            palette_jumps: Vec::new(),
            renaming: false,
            rename: rename.clone(),
            confirm_discard: None,
            commit_msg: commit_msg.clone(),
            sync: git::Sync::default(),
            notice: None,
            logged_notice: None,
            info: None,
            mono_font: cx.theme().mono_font_family.clone(),
            show_panel: true,
            focus: cx.focus_handle(),
            panel_wide: None,
            started: HashMap::new(),
            stopping: Default::default(),
            signed_out: Default::default(),
            stream_failed: Default::default(),
            turn_files: HashMap::new(),
            attachments: HashMap::new(),
            launch_images: HashMap::new(),
            new_context: None,
            new_model: None,
            design,
            assistant,
            arch,
            drafts: HashMap::new(),
            draft_for: None,
            question_picks: HashMap::new(),
            close_armed: None,
            pending_mentions: Vec::new(),
            mention: None,
            slash: None,
            agent_commands: None,
            editing: None,
            commit_suggestion: None,
            writing_commit: false,
            awake: None,
            health: Default::default(),
            notes_input: notes_input.clone(),
            notes_project: None,
            loading_notes: false,
            remove_project: None,
            remove_name: cx.new(|cx| InputState::new(window, cx).placeholder("Project name")),
            picked_playbook: None,
            key_capture: None,
            key_note: None,
            pb_editor: None,
            auto_desc: cx.new(|cx| {
                InputState::new(window, cx).placeholder("One line on what it does (optional)")
            }),
            pb_query: cx.new(|cx| InputState::new(window, cx).placeholder("Search playbooks")),
            pb_confirm_delete: None,
            pb_widen: None,
            pb_asks: Vec::new(),
            pb_install: None,
            pb_source: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("https://github.com/you/playbooks, a link to a file, or a path")
            }),
            pb_run: None,
            pb_values: None,
            picked_values: Vec::new(),
            notes_mode: notes::NotesMode::Write,
            notes_query: cx.new(|cx| InputState::new(window, cx).placeholder("Search notes")),
            auto_name: auto_name.clone(),
            auto_prompt: auto_prompt.clone(),
            auto_every: auto_every.clone(),
            auto_project: None,
            auto_provider: 0,
            gh: Default::default(),
            ui_tx,
            refresh_seq: 0,
            branch_info: Default::default(),
            diff_base: false,
            diff_against: "HEAD".into(),
            pr_status: HashMap::new(),
            pr_loading: Default::default(),
            merge_methods: HashMap::new(),
            merge_methods_loading: Default::default(),
            pr_form: None,
            pr_title: cx.new(|cx| InputState::new(window, cx).placeholder("Pull request title")),
            pr_base: cx.new(|cx| InputState::new(window, cx).placeholder("main")),
            pr_branch: cx.new(|cx| InputState::new(window, cx).placeholder("runo/my-change")),
            pr_body: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("What changed and why (optional)")
            }),
            git_busy: HashMap::new(),
            file_query: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Filter files · Enter searches inside them")
            }),
            file_hits: None,
            file_searching: false,
            files_open: Default::default(),
            confirm_git: None,
            kept: None,
            preparing: HashMap::new(),
            diff_all: false,
            diff_whole: false,
            diff_wrap: false,
            all_diffs: Vec::new(),
            diff_expanded: Default::default(),
            _subscriptions: vec![
                search_sub,
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
        // Recording a shortcut in Settings takes the next keys before any binding runs.
        let me = cx.entity().downgrade();
        let keys_sub = cx.intercept_keystrokes(move |event, window, cx| {
            let keystroke = event.keystroke.clone();
            let _ = me.update(cx, |this, cx| {
                if this.key_capture.is_some() && this.view == View::Settings {
                    this.capture_key(&keystroke, cx);
                    cx.stop_propagation();
                } else {
                    this.key_capture = None;
                    if keystroke.key == "escape"
                        && !keystroke.modifiers.modified()
                        && this.leave_terminal(window, cx)
                    {
                        cx.stop_propagation();
                    }
                }
            });
        });
        // Search boxes on the Playbooks and Project notes pages filter as you type.
        let pb_query_sub = cx.subscribe_in(&app.pb_query, window, repaint);
        let file_query_sub = cx.subscribe_in(
            &app.file_query,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.file_hits = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.search_files(cx);
                    cx.notify();
                }
                _ => {}
            },
        );
        let pb_source_sub = cx.subscribe_in(&app.pb_source, window, repaint);
        let notes_query_sub = cx.subscribe_in(&app.notes_query, window, repaint);
        let finder_sub = cx.subscribe_in(
            &app.finder.query.clone(),
            window,
            |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    this.search_transcripts(query, sessions::HitsFor::List);
                    cx.notify();
                }
            },
        );
        let remove_name_sub = cx.subscribe_in(&app.remove_name, window, repaint);
        let clone_url_sub = cx.subscribe_in(&app.clone_url, window, repaint);
        let project_path_sub = cx.subscribe_in(
            &app.project_path,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.project_path_typed(cx);
                }
            },
        );
        let auto_name_sub = cx.subscribe_in(&app.auto_name, window, repaint);
        let auto_prompt_sub = cx.subscribe_in(
            &app.auto_prompt,
            window,
            |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        app._subscriptions.extend([
            branch_sub,
            task_sub,
            notes_sub,
            file_sub,
            address_sub,
            keys_sub,
            pb_query_sub,
            file_query_sub,
            pb_source_sub,
            notes_query_sub,
            finder_sub,
            remove_name_sub,
            clone_url_sub,
            project_path_sub,
            auto_name_sub,
            auto_prompt_sub,
        ]);
        // Coming back to the window picks up git commands run elsewhere.
        let activation_sub = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() && !this.demo {
                this.refresh();
                cx.notify();
            }
        });
        app._subscriptions.push(activation_sub);
        app._subscriptions.extend(design_subs);
        app._subscriptions.extend(assistant_subs);
        app._subscriptions.extend(arch_subs);
        cx.set_global(MainApp(cx.entity().downgrade()));
        app.init_github(window, cx);
        app.restore_layout();
        // The Browser tab's live page: pick up frames at display rate while it is open.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            if this.update(cx, |this, cx| this.pump_browser(cx)).is_err() {
                break;
            }
        })
        .detach();
        // Chromium would exit when its pipe closes, but stop it right away.
        cx.on_app_quit(|this, _| {
            this.browser = None;
            this.before_quit();
            async {}
        })
        .detach();
        // Remember where the window is, and save the layout when it closes.
        let bounds_sub = cx.observe_window_bounds(window, |this, window, _| {
            let (bounds, maximized) = match window.window_bounds() {
                WindowBounds::Windowed(b) => (b, false),
                WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => (b, true),
            };
            this.window_place = Some(store::WindowPlace {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
                maximized,
            });
        });
        app._subscriptions.push(bounds_sub);
        // Closing the window while agents run here asks first: they stop with it. The
        // layout is saved either way.
        let app_entity = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            app_entity
                .update(cx, |this, cx| {
                    this.save_now();
                    this.may_close(cx)
                })
                .unwrap_or(true)
        });
        // Saves, turns waiting for a slot, crash reports and Runo updates.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            if this.update(cx, |this, cx| this.health_tick(cx)).is_err() {
                return;
            }
        })
        .detach();
        // Agents asking for permission mid-turn are shown within half a second.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            if this.update(cx, |this, cx| this.poll_approvals(cx)).is_err() {
                return;
            }
        })
        .detach();
        // Playbooks agents asked for and requests from the phone start within a couple of
        // seconds; scheduled playbooks are checked twice a minute.
        cx.spawn(async move |this, cx| loop {
            for _ in 0..15 {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if this
                    .update(cx, |this, cx| {
                        this.take_playbook_requests(cx);
                        this.take_phone_requests(cx);
                    })
                    .is_err()
                {
                    return;
                }
            }
            if this
                .update(cx, |this, cx| {
                    this.run_due_automations(cx);
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        let ids: Vec<u64> = app.state.sessions.iter().map(|s| s.id).collect();
        for id in ids {
            app.measure(id);
        }
        if let Some(id) = app.selected.filter(|id| !app.tabs.contains(id)) {
            app.tabs.push(id);
        }
        app.resume_remote_turns();
        app.resume_local_turns();
        app.send_waiting_queues(cx);
        app.refresh();
        app.chat_scroll.scroll_to_bottom();
        window.focus(&app.focus, cx);
        app
    }

    // ---------- state & events ----------

    /// Mark the state changed; it is saved within half a second, off the UI thread.
    fn save(&mut self) {
        if !self.demo {
            self.health.save_due = true;
        }
    }

    /// Show an error in the status bar and write it to the log, tagged with `area` and
    /// what was on screen.
    pub(crate) fn fail(&mut self, area: &str, text: String) {
        applog::warn(area, format!("{text} ({})", self.log_context()));
        self.logged_notice = Some(text.clone());
        self.notice = Some(text);
    }

    /// Which session and project were on screen, for a log line. Never the chat itself.
    pub(crate) fn log_context(&self) -> String {
        let project = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| format!("project {:?}", p.name));
        let session = self.current().map(|s| self.session_label(s.id));
        match (session, project) {
            (Some(s), Some(p)) => format!("{s}, {p}"),
            (Some(s), None) => s,
            (None, Some(p)) => p,
            (None, None) => "no session open".into(),
        }
    }

    /// `session #7 (claude, on a connection)`, for a log line.
    pub(crate) fn session_label(&self, id: u64) -> String {
        match self.state.session(id) {
            Some(s) => format!(
                "session #{id} ({}{})",
                s.provider,
                if s.elsewhere() {
                    ", on a connection"
                } else {
                    ""
                }
            ),
            None => format!("session #{id}"),
        }
    }

    /// One log line for a turn that ended: how, after how long, and on failure the line
    /// of the agent's error output that says why.
    fn log_turn_end(
        &self,
        id: u64,
        ok: bool,
        stopped: bool,
        signed_out: bool,
        took: Option<Duration>,
        stderr: &[String],
    ) {
        let outcome = if ok {
            "finished"
        } else if stopped {
            "stopped"
        } else {
            "failed"
        };
        let mut line = format!("{}: turn {outcome}", self.session_label(id));
        if let Some(took) = took {
            line.push_str(&format!(" after {:.1}s", took.as_secs_f64()));
        }
        if ok || stopped {
            applog::info("turn", line);
            return;
        }
        if signed_out {
            line.push_str(", the agent isn't signed in");
        }
        // The agent's error output, else the error Runo showed for it.
        let mut why = applog::gist(&stderr.join("\n"));
        if why.is_empty() {
            why = self
                .state
                .session(id)
                .and_then(|s| {
                    s.messages
                        .iter()
                        .rev()
                        .take_while(|m| m.role != Role::User)
                        .find(|m| m.role == Role::System)
                })
                .map(|m| applog::gist(&m.text.replace("```text", "").replace("```", "")))
                .unwrap_or_default();
        }
        if !why.is_empty() {
            line.push_str(&format!(": {why}"));
        }
        applog::warn("turn", line);
    }

    /// Notices set without `fail` are written to the log once they are shown.
    fn log_notice(&mut self) {
        if self.notice != self.logged_notice {
            if let Some(text) = &self.notice {
                applog::warn("notice", format!("{text} ({})", self.log_context()));
            }
            self.logged_notice = self.notice.clone();
        }
    }

    fn current(&self) -> Option<&Session> {
        self.selected.and_then(|id| self.state.session(id))
    }

    /// Re-read the selected session's git state (changes, files, branch, races) on a
    /// background thread; the results land a moment later.
    fn refresh(&mut self) {
        let Some(session) = self.current() else {
            self.changes.clear();
            self.files.clear();
            return;
        };
        if self.demo {
            self.dir_is_git = true;
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
        let id = session.id;
        let racers: Vec<(u64, String)> = if session.race != 0 {
            self.state
                .sessions
                .iter()
                .filter(|s| s.race == session.race)
                .map(|s| (s.id, s.dir.clone()))
                .collect()
        } else {
            Vec::new()
        };
        let request = ship::ScanRequest {
            dir: session.dir.clone(),
            base: session.base.clone(),
            since_base: self.diff_base,
            selected_file: self.selected_file.clone(),
            whole: self.diff_whole,
            all: self.diff_all,
            racers,
        };
        let asked = self.selected_file.clone();
        self.refresh_seq += 1;
        let seq = self.refresh_seq;
        self.bg(
            move || ship::scan(request),
            move |this, scan, _| {
                if this.refresh_seq == seq && this.selected == Some(id) {
                    this.apply_scan(id, scan, asked);
                }
            },
        );
    }

    /// Files changed and lines added/removed in a session's folder, for its sidebar card.
    fn measure(&mut self, id: u64) {
        let Some(dir) = self.state.session(id).map(|s| s.dir.clone()) else {
            return;
        };
        if self.demo {
            self.stats.insert(id, git::totals(&demo_changes()));
            return;
        }
        self.bg(
            move || git::is_repo(&dir).then(|| git::totals(&git::changes(&dir))),
            move |this, totals, _| {
                if let Some(totals) = totals {
                    this.stats.insert(id, totals);
                }
            },
        );
    }

    fn open_change(&mut self, path: &str) {
        if !self.demo {
            if let Some(dir) = self.current().map(|s| s.dir.clone()) {
                self.diff = self.load_diff(&dir, path);
            }
        }
        self.selected_file = Some(path.to_string());
    }

    fn close_tab(&mut self, id: u64) {
        let Some(pos) = self.tabs.iter().position(|t| *t == id) else {
            return;
        };
        self.tabs.remove(pos);
        self.save();
        if self.selected == Some(id) {
            match self.tabs.get(pos).or(self.tabs.last()).copied() {
                Some(next) => self.select(next),
                // The last tab closed: stay in Sessions, on the list of sessions.
                None => {
                    self.selected = None;
                    if Mode::of(self.view) == Mode::Sessions {
                        self.view = View::AllSessions;
                    }
                    self.refresh();
                }
            }
        }
    }

    fn select(&mut self, id: u64) {
        if let Some(old) = self.selected.filter(|old| *old != id) {
            self.stash_open_file(old);
        }
        self.selected = Some(id);
        self.browser_main = false;
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
        }
        if let Some(pos) = self.tabs.iter().position(|t| *t == id) {
            self.tab_scroll.scroll_to_item(pos);
        }
        self.cursor = None;
        if let Some(pid) = self.state.session(id).map(|s| s.project_id) {
            self.project = Some(pid);
        }
        self.renaming = false;
        self.note_at = None;
        if let Some(session) = self.state.session_mut(id) {
            session.unread = false;
        }
        // Also remembers the open tabs for the next launch.
        self.save();
        self.view = View::Session;
        self.selected_file = None;
        self.diff.clear();
        if self.open_file.is_some() && !self.file_dirty {
            self.open_file = None;
        }
        self.confirm_delete = None;
        self.confirm_discard = None;
        self.notice = None;
        self.info = None;
        self.menu = None;
        self.pr_form = None;
        self.confirm_git = None;
        self.file_hits = None;
        self.refresh();
        self.load_pr_status(id);
        self.chat_scroll.scroll_to_bottom();
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        self.health.saw(&event);
        match event {
            Event::Line(Target::Session(id), line) => {
                if agent::waits_for_sign_in(&line) {
                    self.sign_in_wait(id, &line);
                }
                let updates = match self.streams.get_mut(&id) {
                    Some(parser) => parser.feed(&line),
                    None => vec![stream::Update::Text(line)],
                };
                let plain = !self.streams.contains_key(&id);
                let refused = self.refused.get(&id).cloned().unwrap_or_default();
                // The reply's run summary names the model it ran on.
                let model = updates
                    .iter()
                    .any(|u| matches!(u, stream::Update::Summary(_)))
                    .then(|| {
                        self.state
                            .session(id)
                            .map(|s| run_model(&s.provider, &s.selections()))
                            .unwrap_or_default()
                    })
                    .filter(|m| !m.is_empty());
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
                            stream::Update::Started { id, label } => {
                                m.running_steps.push((m.steps.len(), id));
                                m.steps.push(label);
                            }
                            stream::Update::Finished { id, ok } => {
                                // The step may sit in an earlier reply when an error
                                // message came between.
                                for m in session.messages.iter_mut().rev() {
                                    if m.finish_step(&id, ok) {
                                        break;
                                    }
                                }
                            }
                            stream::Update::Summary(summary) => {
                                m.meta = match &model {
                                    Some(model) => format!("{model} · {summary}"),
                                    None => summary,
                                }
                            }
                            stream::Update::Error(error) => {
                                applog::warn(
                                    "agent",
                                    format!(
                                        "session #{id} ({}): the agent reported an error: {}",
                                        session.provider,
                                        applog::gist(&error)
                                    ),
                                );
                                self.stream_failed.insert(id);
                                session.messages.push(Message::new(Role::System, error))
                            }
                            stream::Update::Notice(text) => {
                                let turn = session
                                    .messages
                                    .iter()
                                    .rposition(|m| m.role == Role::User)
                                    .unwrap_or(0);
                                let seen = session.messages[turn..]
                                    .iter()
                                    .any(|m| m.meta == "warning" && m.text == text);
                                if !seen {
                                    applog::warn(
                                        "agent",
                                        format!(
                                            "session #{id} ({}): {}",
                                            session.provider,
                                            applog::gist(&text)
                                        ),
                                    );
                                    // Ahead of the reply, which carries on below it.
                                    let mut warning = Message::new(Role::System, text);
                                    warning.meta = "warning".into();
                                    let at = session.messages.len() - 1;
                                    session.messages.insert(at, warning);
                                }
                            }
                            stream::Update::Usage(usage) => m.usage = usage,
                            stream::Update::Denied(tools) => {
                                m.denied =
                                    tools.into_iter().filter(|t| !refused.contains(t)).collect()
                            }
                            stream::Update::Plan(plan) => m.plan = plan,
                            stream::Update::Questions(questions) => m.questions = questions,
                            stream::Update::Conversation(conversation) => {
                                session.agent_session = conversation
                            }
                        }
                    }
                }
                if self.selected == Some(id) && autoscroll() {
                    self.chat_scroll.scroll_to_bottom();
                }
            }
            Event::Err(Target::Session(id), line) => {
                if agent::waits_for_sign_in(&line) {
                    self.sign_in_wait(id, &line);
                }
                // Grok names its conversation on stderr; the next turn continues it.
                if let Some(session) = self.state.session_mut(id) {
                    if let Some(found) = agent::stderr_conversation(&session.provider, &line) {
                        session.agent_session = found;
                    }
                }
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
                // Taken here so a stopped run that still exits cleanly doesn't leave the
                // flag behind for the next real failure.
                let stopped = self.stopping.remove(&id);
                let took = self.started.get(&id).map(|start| start.elapsed());
                self.handles.remove(&Target::Session(id));
                self.streams.remove(&id);
                self.end_approvals(id);
                if let Some(start) = self.started.remove(&id) {
                    let since =
                        std::time::SystemTime::now() - start.elapsed() - Duration::from_secs(2);
                    self.find_copilot_session(id, since);
                }
                self.measure(id);
                self.record_spend(id);
                // A session asked from a canvas is on screen while its canvas is.
                let on_screen = (self.selected == Some(id) && self.view == View::Session)
                    || self.state.session(id).is_some_and(|s| {
                        shell::canvas_of(s) == Some(self.view) && self.project == Some(s.project_id)
                    });
                let all: Vec<String> = self
                    .stderr
                    .get(&id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|l| alerts::clean_line(&l))
                    .filter(|l| !l.starts_with(connections::WARNING))
                    .collect();
                let signed_out = self.signed_out.remove(&id);
                // Some CLIs report a failed turn in their output and still exit cleanly.
                let reported = self.stream_failed.remove(&id);
                let mut hold = false;
                let sync_warning = self.sync_warning(id);
                if let Some(session) = self.state.session_mut(id) {
                    // A CLI that exits cleanly without replying may have refused to run
                    // because it isn't signed in; say so instead of reporting an empty reply.
                    let silent = session.messages.last().is_some_and(|m| {
                        m.role == Role::Agent && m.text.is_empty() && m.steps.is_empty()
                    });
                    let errors: String = session.messages
                        [session.messages.len().saturating_sub(3)..]
                        .iter()
                        .filter(|m| m.role == Role::System)
                        .map(|m| m.text.as_str())
                        .chain(all.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let sign_in = if (silent || !ok || signed_out || reported) && !stopped {
                        agent::sign_in_hint(&session.provider, &errors)
                    } else {
                        None
                    };
                    let ok = ok && sign_in.is_none() && !reported;
                    // A reply that never started is dropped, leaving the error.
                    if reported {
                        let at = session
                            .messages
                            .iter()
                            .rposition(|m| m.role != Role::System);
                        if let Some(at) = at.filter(|&at| {
                            let m = &session.messages[at];
                            m.role == Role::Agent && m.text.is_empty() && m.steps.is_empty()
                        }) {
                            session.messages.remove(at);
                        }
                    }
                    session.status = if ok {
                        Status::Done
                    } else if stopped {
                        Status::Stopped
                    } else {
                        Status::Failed
                    };
                    session.remote_turn = false;
                    session.unread = !on_screen;
                    if silent {
                        if ok {
                            let compacted = session
                                .messages
                                .iter()
                                .rev()
                                .find(|m| m.role == Role::User)
                                .is_some_and(|m| m.text.starts_with("/compact"));
                            if let Some(m) = session.messages.last_mut() {
                                m.text = if compacted {
                                    "Compacted the conversation.".into()
                                } else {
                                    "(The agent finished without any output.)".into()
                                };
                            }
                        } else {
                            session.messages.pop();
                        }
                    }
                    let body = if alerts::waiting_on(session).is_some() {
                        "The agent needs your answer"
                    } else if ok {
                        "Agent finished"
                    } else if stopped {
                        "Agent stopped"
                    } else {
                        "Agent failed"
                    };
                    desktop_notify::send(cx, Some(id), &session.title, body);
                    if let Some(warning) = sync_warning {
                        session.messages.push(warning);
                    }
                    if !stopped && !session.elsewhere() {
                        if sign_in.is_some() {
                            signin::turn_showed(&session.provider, false);
                        } else if ok {
                            signin::turn_showed(&session.provider, true);
                        }
                    }
                    if let Some(hint) = sign_in {
                        hold = true;
                        let turn = session
                            .messages
                            .iter()
                            .rposition(|m| m.role == Role::User)
                            .unwrap_or(0);
                        // Said already before the turn started.
                        if !session.messages[turn..].iter().any(|m| m.text == hint) {
                            session.messages.push(Message::new(Role::System, hint));
                        }
                    } else if !ok {
                        let mut lines: Vec<String> = Vec::new();
                        for line in all.iter().filter(|l| {
                            l.starts_with("ERROR:")
                                || l.starts_with("Error:")
                                || l.starts_with("error:")
                        }) {
                            let line = stream::readable_error(line);
                            if !lines.contains(&line) {
                                lines.push(line);
                            }
                        }
                        let tail: Vec<String> = if lines.is_empty() {
                            all.iter().rev().take(12).rev().cloned().collect()
                        } else {
                            lines
                        };
                        // The agent's own error is already shown above; only a tip is added.
                        let log = if reported {
                            errors.clone()
                        } else {
                            tail.join("\n")
                        };
                        let mut text = String::from(if stopped {
                            "Stopped."
                        } else if reported {
                            ""
                        } else {
                            "The agent stopped with an error."
                        });
                        if !log.trim().is_empty() && !stopped && !reported {
                            text.push_str(&format!(
                                "\n```text\n{}\n```",
                                log.replace("```", "'''")
                            ));
                        }
                        if let Some(tip) = modelcheck::tip(&session.provider, &log) {
                            text.push_str(&format!("\n{tip}"));
                        }
                        let text = text.trim_start().to_string();
                        if !text.is_empty() {
                            let mut m = Message::new(Role::System, text);
                            // A stop the person asked for reads as a note, not an error.
                            if stopped {
                                m.meta = "stopped".into();
                            }
                            session.messages.push(m);
                        }
                    }
                }
                let ok = self
                    .state
                    .session(id)
                    .is_some_and(|s| s.status == Status::Done);
                self.log_turn_end(id, ok, stopped, hold, took, &all);
                self.finish_turn(id, ok, stopped, hold, cx);
                self.design_turn_done(id, stopped, cx);
                self.playbook_turn_done(id, ok);
                self.write_memory(id);
                self.save();
                if self.selected == Some(id) {
                    self.refresh();
                    if autoscroll() {
                        self.chat_scroll.scroll_to_bottom();
                    }
                }
            }
            Event::Output(key, result) => self.handle_output(key, result),
            // A shell printed or exited; repaint it.
            Event::Pty(id) => {
                self.cli_exited(id);
                self.sign_in_exited(id);
                if self.setups.contains_key(&id) {
                    self.setup_exited(id, cx);
                }
                // A quick command's tab says how it went in its own words.
                self.task_exited(id);
                if let Some(shell) = self.shells.iter().find(|s| s.id == id && s.exited()) {
                    if self.shell_tab == Some(id) && !self.tasks.contains_key(&id) {
                        self.info = Some((
                            format!("{} exited.", shell.title),
                            std::time::Instant::now(),
                        ));
                    }
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

    /// The worktree setup command finished: start the agent waiting on it.
    fn setup_done(&mut self, id: u64, ok: bool, cx: &mut Context<Self>) {
        let Some(task) = self.pending_start.remove(&id) else {
            return;
        };
        if !ok {
            self.terminal
                .entry(id)
                .or_default()
                .push("Setup failed; starting the agent anyway so you can fix it together.".into());
            self.info = Some((
                "The setup command failed; starting the agent anyway so you can fix it together."
                    .into(),
                std::time::Instant::now(),
            ));
        }
        if let Some(provider) = self.state.session(id).map(|s| s.provider.clone()) {
            let opts = cx.global::<AppSettings>().agent(&provider);
            self.start_agent(id, task, &opts);
        }
        if self.selected == Some(id) {
            self.refresh();
        }
    }

    fn start_agent(&mut self, id: u64, prompt: String, opts: &AgentOptions) {
        let images = self.launch_images.remove(&id).unwrap_or_default();
        self.start_turn(id, prompt, images, opts);
    }

    /// Demo projects are not real folders, so ask for a real repository instead.
    fn demo_guard(&mut self, cx: &mut Context<Self>) -> bool {
        if self.demo {
            self.pick_project(cx);
        }
        self.demo
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit_prompt(window, cx);
    }

    /// Show a file of the current session in the Files tab; applied on the next frame.
    fn open_preview(&mut self, path: &str) {
        self.pending_file = Some(path.to_string());
    }

    pub(crate) fn open_settings(&mut self) {
        self.view = View::Settings;
        signin::refresh(false);
        let dir = self.current().filter(|_| !self.demo).map(|s| s.dir.clone());
        self.discovered = Some(settings::discover(dir.as_deref()));
    }

    fn stop(&mut self) {
        if let Some(id) = self.selected {
            self.stop_session(id);
        }
    }

    /// Agents still running on this machine; turns on connections keep going without Runo.
    fn local_runs(&self) -> usize {
        self.handles
            .iter()
            .filter(|(t, h)| matches!(t, Target::Session(_)) && !h.runner())
            .count()
    }

    /// Whether the window may close now. With agents running here, the first close only
    /// warns; closing again within a few seconds goes ahead.
    fn may_close(&mut self, cx: &mut Context<Self>) -> bool {
        let running = self.local_runs();
        if running == 0
            || self
                .close_armed
                .is_some_and(|at| at.elapsed() < Duration::from_secs(10))
        {
            return true;
        }
        self.close_armed = Some(std::time::Instant::now());
        self.fail("session", format!(
            "{running} agent{} still running. Quitting Runo stops {}. Close or quit again to quit anyway.",
            if running == 1 { " is" } else { "s are" },
            if running == 1 { "it" } else { "them" }
        ));
        cx.notify();
        false
    }

    /// Runo is quitting: keep the turns made in open CLIs, and stop agents and commands
    /// running on this machine together with everything they started.
    fn before_quit(&mut self) {
        self.keep_cli_turns();
        for handle in self.handles.values().filter(|h| !h.runner()) {
            handle.kill();
        }
        self.save_now();
        State::flush();
    }

    /// Stop a session's agent, and everything it started.
    pub(crate) fn stop_session(&mut self, id: u64) {
        if let Some(handle) = self.handles.get(&Target::Session(id)) {
            self.stopping.insert(id);
            handle.kill();
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

    /// Save the new title, or with `None` just leave rename mode. A blank name is refused
    /// and the box stays open.
    fn finish_rename(&mut self, title: Option<String>) {
        if !self.renaming {
            return;
        }
        if title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            self.notice =
                Some("A session needs a name. Type one, or press Esc to keep the old one.".into());
            return;
        }
        self.renaming = false;
        let Some(title) = title else {
            return;
        };
        self.notice = None;
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
        self.finder.palette_hits.found.clear();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus, cx);
    }

    fn palette_jump(&mut self, jump: Jump, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        match jump {
            Jump::Action => {}
            Jump::Tour => self.start_tour(),
            Jump::Playbook(id) => {
                let here = self.current_project();
                self.run_playbook(id, here, cx);
            }
            Jump::View(view) => match view {
                View::History => self.open_history(cx),
                View::Notes => self.open_notes(window, cx),
                View::GitHub => self.load_github(),
                View::Review => self.open_review(),
                _ => self.view = view,
            },
            Jump::Session(id) => self.open_session(id, window, cx),
            Jump::Project(pid) => self.switch_project(pid, window, cx),
            Jump::AddProject => self.pick_project(cx),
            Jump::Settings(page) => {
                self.open_settings();
                self.settings_page = page;
            }
            Jump::File(path) => {
                self.view = View::Session;
                self.show_panel = true;
                self.open_preview(&path);
                self.right_tab = RightTab::Files;
            }
        }
    }

    /// The palette's rows: commands, playbooks, projects, sessions, settings pages, then
    /// files of the open session. Each group and the jumps behind its rows are built side by
    /// side, so a chosen row always finds its own jump.
    fn palette_view(&mut self, state: Entity<CommandState>, cx: &mut Context<Self>) -> AnyElement {
        let mut groups: Vec<(CommandGroup, Vec<Jump>)> = Vec::new();
        let mut commands = CommandGroup::new().label("Commands");
        let mut command_jumps = Vec::new();
        let actions: [(&str, IconName, Box<dyn Action>); 9] = [
            ("New session", IconName::Plus, Box::new(NewSession)),
            ("Open settings", IconName::Settings, Box::new(OpenSettings)),
            (
                "Switch between Chat and CLI",
                IconName::SquareTerminal,
                Box::new(ToggleView),
            ),
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
            ("Close tab", IconName::X, Box::new(CloseTab)),
            (
                "Archive session",
                IconName::Archive,
                Box::new(ArchiveSession),
            ),
            ("Quit Runo", IconName::LogOut, Box::new(Quit)),
        ];
        for (label, name, action) in actions {
            commands = commands.item(CommandItem::new().label(label).icon(name).action(action));
            command_jumps.push(Jump::Action);
        }
        for (label, name, view) in [
            ("Go to Home", IconName::House, View::Home),
            ("Go to Inbox", IconName::Inbox, View::Inbox),
            ("Go to Sessions", IconName::Layers, View::AllSessions),
            (
                "Go to Isolated sessions",
                IconName::GitBranch,
                View::Worktrees,
            ),
            (
                "Go to Agent history",
                IconName::RotateCcwClock,
                View::History,
            ),
            ("Go to Review", IconName::GitCompare, View::Review),
            ("Go to GitHub", IconName::GitPullRequest, View::GitHub),
            ("Go to Playbooks", IconName::BookOpen, View::Automations),
            ("Go to Timeline", IconName::Activity, View::Activity),
            ("Go to Design", IconName::PanelsTopLeft, View::Design),
            ("Go to Architecture", IconName::Network, View::Architecture),
            ("Go to Project notes", IconName::NotebookPen, View::Notes),
        ] {
            commands = commands.item(CommandItem::new().label(label).icon(name));
            command_jumps.push(Jump::View(view));
        }
        commands = commands.item(
            CommandItem::new()
                .label("Add a project folder…")
                .icon(IconName::FolderPlus)
                .keywords(["new project", "open folder", "clone repository"]),
        );
        command_jumps.push(Jump::AddProject);
        commands = commands.item(
            CommandItem::new()
                .label("Take the tour")
                .icon(IconName::Sparkles)
                .keywords(["welcome", "help", "guide", "onboarding"]),
        );
        command_jumps.push(Jump::Tour);
        if Self::assistant_enabled(cx) {
            commands = commands.item(
                CommandItem::new()
                    .label("Ask Runo")
                    .icon(IconName::CircleQuestionMark)
                    .keywords(["help", "assistant", "how do i", "question"])
                    .action(Box::new(AskRuno)),
            );
            command_jumps.push(Jump::Action);
        }
        groups.push((commands, command_jumps));

        let mut playbooks = CommandGroup::new().label("Playbooks");
        let mut playbook_jumps = Vec::new();
        let here = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
            .unwrap_or_default();
        for auto in &self.state.automations {
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == auto.project_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| here.clone());
            playbooks = playbooks.item(
                CommandItem::new()
                    .label(format!("Run {}  ·  {project}", auto.name))
                    .icon(IconName::Play)
                    .keywords(["playbook", "run", "automation"]),
            );
            playbook_jumps.push(Jump::Playbook(auto.id));
        }
        groups.push((playbooks, playbook_jumps));

        let mut projects = CommandGroup::new().label("Projects");
        let mut project_jumps = Vec::new();
        for project in &self.state.projects {
            projects = projects.item(
                CommandItem::new()
                    .label(format!("Switch to {}", project.name))
                    .icon(IconName::Folder)
                    .keywords(["project", project.path.as_str()]),
            );
            project_jumps.push(Jump::Project(project.id));
        }
        groups.push((projects, project_jumps));

        groups.extend(self.palette_sessions());

        let mut pages = CommandGroup::new().label("Settings");
        let mut page_jumps = Vec::new();
        for page in SettingsPage::pages() {
            pages = pages.item(
                CommandItem::new()
                    .label(format!("Settings: {}", page.label()))
                    .icon(page.icon())
                    .keywords(["settings", "preferences"]),
            );
            page_jumps.push(Jump::Settings(page));
        }
        groups.push((pages, page_jumps));

        // The open session's files, from any mode.
        let title = self.current().map(|s| s.title.clone());
        let mut files = CommandGroup::new().label(match &title {
            Some(t) => format!("Files in {t}"),
            None => "Files".to_string(),
        });
        let mut file_jumps = Vec::new();
        if title.is_some() {
            for path in self.files.iter().take(3000) {
                files = files.item(CommandItem::new().label(path.clone()).icon(IconName::File));
                file_jumps.push(Jump::File(path.clone()));
            }
        }
        groups.push((files, file_jumps));

        let mut command = Command::new(&state);
        let mut jumps = Vec::new();
        for (group, group_jumps) in groups {
            command = command.group(group);
            jumps.push(group_jumps);
        }
        self.palette_jumps = jumps;

        let this = cx.entity().downgrade();
        let cancel = this.clone();
        let searcher = this.clone();
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
                    // A fixed height: the list scrolls inside it and the card does not
                    // jump in size as results come and go while typing.
                    .h(px(480.))
                    .v_flex()
                    .rounded_xl()
                    .border_1()
                    .border_color(float_edge())
                    .bg(float(c_canvas()))
                    .shadow_lg()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        command
                            .bordered(false)
                            // The palette paints the surface, so the list and the rest of
                            // the card are one material, translucent in glass mode.
                            .bg(transparent_black())
                            .flex_1()
                            .min_h_0()
                            .max_h(px(480.))
                            .placeholder("Jump to a session, project, file, setting or command…")
                            .on_query(move |query, _, cx| {
                                let query = query.to_string();
                                let _ = searcher.update(cx, |this, cx| {
                                    this.search_transcripts(query, sessions::HitsFor::Palette);
                                    cx.notify();
                                });
                            })
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
        let provider = session.provider.clone();
        let opts = cx.global::<AppSettings>().agent(&provider);
        let before = session.messages.len();
        self.start_agent(id, prompt, &opts);
        // The notes go only once the follow-up really started (or is waiting for the
        // worktree); a failed start keeps them for another try.
        let sent = self.preparing.contains_key(&id)
            || self
                .state
                .session(id)
                .is_some_and(|s| s.messages.len() > before && s.status == Status::Working);
        if sent {
            if let Some(session) = self.state.session_mut(id) {
                session.notes.clear();
            }
            self.save();
        }
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
                if let Some(id) = self.selected {
                    self.state.log(id, format!("Discarded {path}"));
                }
                self.refresh();
            }
            Err(error) => self.fail("session", format!("Could not discard {path}: {error}")),
        }
    }

    fn add_project(
        &mut self,
        path: &std::path::Path,
        name: String,
        color: String,
        icon: String,
        start_session: bool,
    ) -> Result<(), String> {
        let path_str = path.to_string_lossy().into_owned();
        if !path.is_dir() {
            return Err(format!("{path_str} is not a folder."));
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
            return Err("That folder is already a project.".into());
        }
        let id = self.state.next_id();
        let name = if name.is_empty() {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path_str.clone())
        } else {
            name
        };
        self.state.projects.push(Project {
            id,
            name,
            path: path_str,
            color,
            icon,
            ..Default::default()
        });
        self.expanded.push(id);
        self.new_project = Some(id);
        if start_session {
            self.open_new_session();
        }
        self.save();
        Ok(())
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
            self.fail("session", "Add a project first.".into());
            return;
        };
        if task.is_empty() {
            self.fail("session", "Describe the task first.".into());
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
            Some(git::branch_slug(&task))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("runo/race-{}", self.state.next_id + 1))
        } else {
            typed.clone()
        };
        let base = self.new_base.clone();
        for provider in providers {
            let isolated = (self.new_isolated || racing) && is_git;
            // An untyped name comes from the task, made unique among branches and sessions.
            let branch = if racing {
                let wanted = format!("{race_base}-{}", agent::PROVIDERS[provider].id);
                self.task_branch(&project, "", wanted, |n| git::has_branch(&project.path, n))
            } else {
                typed.clone()
            };
            match self.spawn_session_from(
                &project,
                provider,
                &branch,
                isolated,
                is_git,
                &task,
                base.clone(),
                None,
                cx,
            ) {
                Ok(id) => {
                    if let Some(model) = self.new_model.clone().filter(|m| !m.is_empty()) {
                        if let Some(session) = self.state.session_mut(id) {
                            session.set_model(model);
                        }
                    }
                    if self.new_connection != connections::LOCAL
                        && connections::get(self.new_connection).is_some()
                    {
                        if let Some(session) = self.state.session_mut(id) {
                            session.connection = self.new_connection;
                        }
                    }
                    created.push(id)
                }
                Err(error) => {
                    self.fail("session", error);
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
        // An agent editing a diagram in place is watched from the canvas it was asked on.
        let mut stay = None;
        let before = self.selected;
        if let Some(context) = self.new_context.take() {
            for &id in &created {
                self.apply_context(id, &context);
            }
            stay = context
                .in_place
                .then_some(self.new_back)
                .filter(|v| matches!(v, View::Architecture | View::Design));
            if let Some(view) = stay {
                for &id in &created {
                    if let Some(session) = self.state.session_mut(id) {
                        session.canvas = shell::canvas_name(view).into();
                    }
                }
            }
        }
        self.new_task
            .update(cx, |t, cx| t.set_value("", window, cx));
        self.branch.update(cx, |b, cx| b.set_value("", window, cx));
        self.new_branches = None;
        if !self.expanded.contains(&project.id) {
            self.expanded.push(project.id);
        }
        // Starting each one selects it; land on the first.
        for &id in &created {
            self.select(id);
            self.launch(id, task.clone(), cx);
        }
        self.select(first);
        if let Some(view) = stay {
            // Watched from the canvas, so it takes no tab; its chat opens from there.
            self.tabs.retain(|t| !created.contains(t));
            self.selected = before.filter(|b| !created.contains(b));
            self.view = view;
            self.save();
        }
    }

    /// Whether the New session form will race several agents.
    fn racing(&self) -> bool {
        self.race_mode && self.new_race.len() > 1
    }

    /// Create one session without starting the agent. An isolated session's worktree is
    /// made in the background; turns started before it is ready wait for it.
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
        self.spawn_session_for(
            project,
            provider,
            typed_branch,
            isolated,
            is_git,
            task,
            None,
            cx,
        )
    }

    /// `spawn_session`, checking out pull request `pr` in the new worktree.
    #[allow(clippy::too_many_arguments)]
    fn spawn_session_for(
        &mut self,
        project: &Project,
        provider: usize,
        typed_branch: &str,
        isolated: bool,
        is_git: bool,
        task: &str,
        pr: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        self.spawn_session_from(
            project,
            provider,
            typed_branch,
            isolated,
            is_git,
            task,
            None,
            pr,
            cx,
        )
    }

    /// `spawn_session_for`, starting the branch from `base` instead of the default branch.
    #[allow(clippy::too_many_arguments)]
    fn spawn_session_from(
        &mut self,
        project: &Project,
        provider: usize,
        typed_branch: &str,
        isolated: bool,
        is_git: bool,
        task: &str,
        base: Option<String>,
        pr: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        // A repository with no commits has nothing to branch from: run in the folder itself.
        let unborn = isolated && is_git && !self.demo && !git::has_commits(&project.path);
        let isolated = isolated && !unborn;
        let id = self.state.next_id();
        let base = match base {
            Some(base) if is_git => base,
            _ if is_git => git::default_branch(&project.path),
            _ => String::new(),
        };
        let (branch, dir) = if !is_git {
            ("local folder".to_string(), project.path.clone())
        } else if isolated {
            // Named after the task when no name was given, such as runo/fix-login-redirect.
            let branch = if typed_branch.is_empty() {
                self.task_branch(project, task, format!("runo/session-{id}"), |n| {
                    git::has_branch(&project.path, n)
                })
            } else {
                typed_branch.to_string()
            };
            let dir = git::worktree_path(&project.path, &branch);
            if dir.exists() {
                return Err(format!(
                    "Could not create worktree: {} already exists",
                    dir.display()
                ));
            }
            (branch, dir.to_string_lossy().into_owned())
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
            branch: branch.clone(),
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
            base,
            ..Default::default()
        });
        if unborn {
            if let Some(session) = self.state.session_mut(id) {
                session.messages.push(Message::new(
                    Role::System,
                    "This repository has no commits yet, so the session runs in the project folder instead of its own worktree. Make a first commit to isolate the next one.",
                ));
            }
        }
        applog::info(
            "session",
            format!(
                "{} created in project {:?}{}",
                self.session_label(id),
                project.name,
                if isolated {
                    ", in its own worktree"
                } else {
                    ""
                }
            ),
        );
        if isolated && is_git && !self.demo {
            self.make_worktree(id, project.path.clone(), branch, pr);
        }
        Ok(id)
    }

    /// Start a new session's agent, after the worktree setup command when there is one.
    /// The session must be selected, since setup runs in its Terminal.
    pub(crate) fn launch(&mut self, id: u64, task: String, cx: &mut Context<Self>) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let (isolated, provider) = (session.isolated, session.provider.clone());
        // The worktree is still being made; it launches when ready.
        if self.preparing.contains_key(&id) {
            self.pending_start.insert(id, task);
            return;
        }
        // The project's own setup command, else the one in Settings.
        let own = self
            .project_of(session)
            .map(|p| p.setup.clone())
            .unwrap_or_default();
        let setup = cx.global::<AppSettings>().setup_for(&own);
        if isolated && !setup.is_empty() && !self.demo {
            self.pending_start.insert(id, task);
            if self.run_setup_in(id, &setup, cx) {
                self.info = Some((
                    "Running the worktree setup command first, in a terminal tab.".into(),
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
        // CLI-first people get the agent's own CLI, with the task as its first prompt.
        // Only CLIs known to take a first prompt on the command line open this way, and
        // playbook runs stay in Chat, where their limits hold.
        if cx.global::<AppSettings>().default_view == "cli"
            && agent::imports_cli_turns(&provider)
            && self
                .state
                .session(id)
                .is_some_and(|s| s.playbook == 0 && cliview::cli_blocker(s).is_none())
        {
            // Context a session was started with goes ahead of the task here too.
            let mut task = task;
            if let Some(s) = self.state.session_mut(id) {
                s.cli = true;
                s.status = Status::Done;
                let brief = std::mem::take(&mut s.handoff);
                if !brief.is_empty() {
                    task = format!("{brief}\n\n## Your next instruction\n{task}");
                }
                self.launch_images.remove(&id);
            }
            self.view_changed = Some((id, std::time::Instant::now()));
            self.open_cli(id, &task, cx);
            self.save();
            return;
        }
        let opts = cx.global::<AppSettings>().agent(&provider);
        self.start_agent(id, task, &opts);
    }

    fn open_history(&mut self, cx: &mut Context<Self>) {
        self.view = View::History;
        let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
        else {
            return;
        };
        // Hundreds of transcripts are read; do it off the UI thread.
        self.history_loading = true;
        let scan = cx
            .background_executor()
            .spawn(async move { history::scan(std::path::Path::new(&home)) });
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            let _ = this.update(cx, |this, cx| {
                this.history = found;
                this.history_loading = false;
                cx.notify();
            });
        })
        .detach();
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
            self.fail("session", format!("{} no longer exists.", past.cwd));
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
                    ..Default::default()
                });
                id
            }
        };
        let id = self.state.next_id();
        let provider = agent::provider(past.provider);
        // The conversation's earlier turns, read from the CLI's transcript, so the chat
        // shows where it left off.
        let prefix = format!("{}/", past.cwd.trim_end_matches('/'));
        let mut earlier: Vec<Message> = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .and_then(|home| {
                history::transcript(std::path::Path::new(&home), past.provider, &past.id)
            })
            .map(|path| history::turns(past.provider, &history::read_from(&path, 0)))
            .unwrap_or_default();
        for m in &mut earlier {
            for step in &mut m.steps {
                *step = step.replace(&prefix, "");
            }
        }
        let picked_up = format!(
            "Picked up the {} conversation “{}” from {}.",
            provider.name,
            past.title,
            history::ago(past.modified),
        );
        let messages = if earlier.iter().any(|m| m.role == Role::Agent) {
            earlier.push(Message::now(
                Role::System,
                format!("{picked_up} Send a message to continue it."),
            ));
            earlier
        } else {
            vec![Message::now(
                Role::Agent,
                format!(
                    "{picked_up} Earlier messages stay in {}; send a message to continue it.",
                    provider.name
                ),
            )]
        };
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
            messages,
            agent_session: past.id.clone(),
            conversation_start: 0,
            pinned: false,
            archived: false,
            unread: false,
            notes: Vec::new(),
            race: 0,
            ..Default::default()
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
                    .hover(|el| el.bg(tint(c_subtle())))
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
                                    .font_weight(FontWeight::MEDIUM)
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
        if self.history.is_empty() && self.history_loading {
            rows = rows.child(
                div()
                    .py_10()
                    .flex()
                    .justify_center()
                    .text_sm()
                    .text_color(rgb(c_muted()))
                    .child("Looking for past conversations…"),
            );
        } else if self.history.is_empty() {
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
                            .font_weight(FontWeight::MEDIUM)
                            .child("No past conversations found"),
                    )
                    .child(div().text_xs().text_color(rgb(c_muted())).child(
                        "Claude Code and Codex conversations from this machine show up here.",
                    )),
            );
        }
        self.page(
            "Agent history",
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
        self.prune_shells();
        self.tabs.retain(|t| *t != id);
        self.stats.remove(&id);
        self.started.remove(&id);
        self.pending_start.remove(&id);
        self.preparing.remove(&id);
        self.pr_status.remove(&id);
        if !self.demo {
            connections::cleanup_session(&session);
        }
        // The worktree, the branch when nothing in it is unmerged, and Undo snapshots.
        self.remove_session_files(&session, false);
        self.terminal.remove(&id);
        self.confirm_delete = None;
        // Deleting another session from its menu keeps the open one open.
        if self.selected == Some(id) {
            self.selected = self
                .tabs
                .last()
                .copied()
                .or_else(|| self.state.sessions.first().map(|s| s.id));
            if self.selected.is_none() && self.view == View::Session {
                self.view = View::AllSessions;
            }
        }
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
            // Stopped on purpose: neutral, not an error.
            Status::Stopped => c_muted(),
        }
    }

    fn status_label(status: Status) -> &'static str {
        status.label()
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
                    .hover(|el| el.bg(tint(c_subtle())))
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
            .hover(|el| el.bg(tint(c_subtle())))
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
            .hover(|el| el.bg(tint(c_subtle())))
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
                    .bg(tint(c_selected()))
                    .text_color(rgb(c_text()))
            })
            .when(!selected, |el| {
                el.border_color(rgb(c_border()))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())))
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
            .font_weight(FontWeight::SEMIBOLD)
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
            .font_weight(FontWeight::SEMIBOLD)
            .child(provider.mark)
    }

    // ---------- title bar ----------

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
            .when(selected, |el| el.bg(tint(c_active())))
            .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
            .on_click(cx.listener(move |this, _, window, cx| {
                match view {
                    View::Settings => this.open_settings(),
                    View::History => this.open_history(cx),
                    View::Notes => this.open_notes(window, cx),
                    View::GitHub => this.load_github(),
                    _ => this.view = view,
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
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(0xffffff))
                        .child(count.to_string()),
                )
            })
    }

    /// Small icon button shown on hover over a row.
    fn row_action(
        &self,
        id: impl Into<ElementId>,
        size: f32,
        name: IconName,
        tip: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_active())))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .on_click(on_click)
            .child(icon(name, 12., c_secondary()))
    }

    /// A session in the sidebar: title and age, then agent, branch and change counts.
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
        let color = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .and_then(projects::color_of);
        // "local folder" is where most sessions run, so it only adds noise; a branch
        // or worktree still shows.
        let branch = (!session.branch.is_empty() && session.branch != "local folder")
            .then(|| session.branch.clone());
        div()
            .id(("session", id as usize))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::Session(id), cx),
            )
            .relative()
            .group("session-row")
            .mx_1()
            .px_2()
            .py(px(6.))
            .v_flex()
            .gap(px(2.))
            .rounded_md()
            .cursor_pointer()
            .when(selected, |el| {
                el.bg(match color {
                    Some(c) => rgba((c << 8) | 0x24),
                    None => tint(c_active()),
                })
            })
            .when(!selected, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .when(self.cursor == Some(id), |el| el.bg(tint(c_selected())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(id);
                cx.notify();
            }))
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
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(rgb(if selected || unread {
                                c_text()
                            } else {
                                c_secondary()
                            }))
                            .child(session.title.clone()),
                    )
                    .when(pinned, |el| el.child(icon(IconName::Pin, 11., c_muted())))
                    .child(if let Some(need) = self.need(session) {
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(match need {
                                shell::Need::Blocked => c_amber(),
                                shell::Need::Failed => c_red(),
                                shell::Need::Finished => c_blue(),
                            }))
                            .into_any_element()
                    } else if working {
                        self.status_dot(Status::Working, ("dot", id as usize), 7.)
                    } else {
                        div().into_any_element()
                    })
                    .child(
                        // The age and the hover actions share one slot, so the actions
                        // replace the age instead of landing on top of it.
                        div()
                            .flex_none()
                            .relative()
                            .min_w(px(40.))
                            .flex()
                            .justify_end()
                            .text_size(px(11.5))
                            .child(
                                div()
                                    .text_color(rgb(if working { c_blue() } else { c_muted() }))
                                    .group_hover("session-row", |el| el.opacity(0.))
                                    .child(when),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .right_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(2.))
                                    .opacity(0.)
                                    .group_hover("session-row", |el| el.opacity(1.))
                                    .child(self.row_action(
                                        ("pin", id as usize),
                                        22.,
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
                                        22.,
                                        IconName::Archive,
                                        "Archive",
                                        cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_archive(id);
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    ),
            )
            .child(
                // The agent and where it works share one quiet line under the title.
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(self.provider_mark(provider, 11.))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(120.))
                            .truncate()
                            .child(match model {
                                Some(m) => format!("{} · {m}", provider.name),
                                None => provider.name.to_string(),
                            }),
                    )
                    .when_some(branch, |el, branch| {
                        el.child(icon(IconName::GitBranch, 11., c_muted()))
                            .child(div().min_w_0().truncate().child(branch))
                    })
                    .child(div().flex_1())
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
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.sidebar_width;
        let filter = self.search.read(cx).value().to_lowercase();
        let mut projects = div().v_flex().pb_2();
        let mut first = true;
        for project in &self.state.projects {
            let pid = project.id;
            let open = self.expanded.contains(&pid);
            let missing = !self.demo && !std::path::Path::new(&project.path).is_dir();
            let sessions = self.project_sessions(project, &filter);
            if !filter.is_empty() && sessions.is_empty() {
                continue;
            }
            let color = projects::color_of(project);
            // Each project is one block: its header, then its sessions hung off a
            // rail in the project's color. Space between blocks keeps them apart.
            let mut group = div().v_flex().mx_1().when(!first, |el| el.mt(px(10.)));
            first = false;
            group = group.child(
                div()
                    .id(("project", pid as usize))
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Project(pid), cx),
                    )
                    .group("project")
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(pos) = this.expanded.iter().position(|p| *p == pid) {
                            this.expanded.remove(pos);
                        } else {
                            this.expanded.push(pid);
                        }
                        this.save();
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
                            .size(px(20.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .bg(match color {
                                Some(c) => rgba((c << 8) | 0x2e),
                                None => tint(c_subtle()),
                            })
                            .child(icon(
                                projects::icon_of(project),
                                12.,
                                color.unwrap_or(c_secondary()),
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child(project.name.clone()),
                    )
                    .when(missing, |el| {
                        el.child(
                            div()
                                .id(("project-missing", pid as usize))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Folder missing: relink or remove it from Home",
                                    )
                                    .build(window, cx)
                                })
                                .child(icon(IconName::TriangleAlert, 12., c_amber())),
                        )
                    })
                    .when(!open, |el| {
                        // A folded project still says when a session inside needs you
                        // or is running.
                        let need = sessions.iter().find_map(|s| self.need(s));
                        let working = sessions.iter().any(|s| s.status == Status::Working);
                        el.when_some(need, |el, need| {
                            el.child(div().size(px(7.)).flex_none().rounded_full().bg(rgb(
                                match need {
                                    shell::Need::Blocked => c_amber(),
                                    shell::Need::Failed => c_red(),
                                    shell::Need::Finished => c_blue(),
                                },
                            )))
                        })
                        .when(need.is_none() && working, |el| {
                            el.child(self.status_dot(
                                Status::Working,
                                ("project-dot", pid as usize),
                                7.,
                            ))
                        })
                    })
                    .when(!sessions.is_empty(), |el| {
                        el.child(
                            div()
                                .px(px(6.))
                                .h(px(16.))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(tint(c_subtle()))
                                .text_size(px(10.5))
                                .text_color(rgb(c_muted()))
                                .group_hover("project", |el| el.opacity(0.))
                                .child(sessions.len().to_string()),
                        )
                    })
                    .child(
                        div()
                            .id(("project-new", pid as usize))
                            .absolute()
                            .right(px(6.))
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
                                this.new_connection = this.project_connection(pid);
                                this.open_new_session();
                                cx.notify();
                            }))
                            .child(icon(IconName::Plus, 13., c_secondary())),
                    )
                    .relative(),
            );
            if open || !filter.is_empty() {
                // Sessions hang off a rail under the chevron, in the project's color.
                let mut rows = div()
                    .v_flex()
                    .gap(px(2.))
                    .ml(px(14.))
                    .mt(px(2.))
                    .mb_1()
                    .pl(px(2.))
                    .border_l_2()
                    .border_color(match color {
                        Some(c) => rgba((c << 8) | 0x73),
                        None => rgb(c_border()),
                    });
                if sessions.is_empty() {
                    rows = rows.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child("No sessions yet"),
                    );
                }
                for session in sessions {
                    rows = rows.child(self.session_row(session, cx));
                }
                // Opening a project folds its sessions down into place.
                group = group.child(rows.animated(
                    ("project-rows", pid as usize),
                    Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                    |el, d| el.opacity(d).top(px(-6. * (1. - d))),
                ));
            }
            projects = projects.child(group);
        }
        if cx.global::<AppSettings>().group_by_status {
            projects = self.status_groups(&filter, cx);
        }
        div()
            .w(px(width))
            .flex_none()
            .h_full()
            .relative()
            .v_flex()
            .border_r_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .child(
                div()
                    .p_2()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .id("new-session")
                            .relative()
                            .child(tour::anchor(&self.anchors, "new-session"))
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
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_new_session();
                                cx.notify();
                            }))
                            .child(icon(IconName::SquarePen, 14., c_text()))
                            .child(div().flex_1().child("New session"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(c_muted()))
                                    .child(keymap::label(cx, "new-session")),
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
                            .bg(tint(c_subtle()))
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
                            .relative()
                            .child(
                                self.nav_item(
                                    "all-sessions",
                                    IconName::Layers,
                                    "All sessions",
                                    self.state
                                        .sessions
                                        .iter()
                                        .filter(|s| s.canvas.is_empty())
                                        .count(),
                                    View::AllSessions,
                                    cx,
                                ),
                            )
                            .child(
                                self.nav_item(
                                    "worktrees",
                                    IconName::GitBranch,
                                    "Isolated",
                                    self.state
                                        .sessions
                                        .iter()
                                        .filter(|s| s.isolated && s.canvas.is_empty())
                                        .count(),
                                    View::Worktrees,
                                    cx,
                                ),
                            )
                            .child(self.nav_item(
                                "needs-you",
                                IconName::Inbox,
                                "Needs you",
                                self.counts().needs_you,
                                View::Inbox,
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
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_muted()))
                            .child("PROJECTS"),
                    )
                    .child(
                        div()
                            .id("add-project")
                            .p_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|el| el.bg(tint(c_subtle())))
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
                    .relative()
                    .child(tour::anchor(&self.anchors, "projects"))
                    .child(div().size_full().overflow_y_scrollbar().child(projects)),
            )
            .child(
                // Drag the edge to make the sidebar narrower or wider.
                div()
                    .id("sidebar-edge")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(5.))
                    .cursor_col_resize()
                    .when(self.resizing_sidebar, |el| el.bg(rgb(c_blue())))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.resizing_sidebar = true;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
    }

    /// One tool step: a pulsing dot while it runs, then a check, or a red cross when
    /// its tool failed.
    fn step_row(&self, text: &str, state: StepState, key: (u64, usize, usize)) -> impl IntoElement {
        let mark = match state {
            StepState::Running => div()
                .size(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .bg(rgb(c_blue()))
                        .animated(
                            SharedString::from(format!("step-run-{}-{}-{}", key.0, key.1, key.2)),
                            Animation::new(Duration::from_millis(1200))
                                .repeat()
                                .with_easing(pulsating_between(0.3, 1.)),
                            |el, t| el.opacity(t),
                        ),
                )
                .into_any_element(),
            StepState::Failed => icon(IconName::X, 12., c_red()).into_any_element(),
            StepState::Done => icon(IconName::Check, 12., c_muted()).into_any_element(),
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .h(px(24.))
            .text_size(px(12.5))
            .child(mark)
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
                    .hover(|el| el.bg(tint(c_subtle())))
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
                    .bg(tint(c_subtle()))
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
            let sid = self.selected.unwrap_or(0);
            let can_undo = !message.checkpoint.is_empty() && !working;
            let mut images = div().flex().flex_wrap().justify_end().gap_2();
            for path in &message.images {
                images = images.child(
                    img(std::path::PathBuf::from(path))
                        .max_w(px(220.))
                        .max_h(px(160.))
                        .rounded_lg()
                        .object_fit(ObjectFit::Contain),
                );
            }
            return fade(
                div()
                    .id(("message", index))
                    .group("user-msg")
                    .w_full()
                    .v_flex()
                    .items_end()
                    .gap_1()
                    .when(!message.images.is_empty(), |el| el.child(images))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_1()
                            .when(can_undo, |el| {
                                el.child(
                                    self.ghost_button_sized(
                                        ("restore", index),
                                        IconName::RotateCcw,
                                        "Restore files to before this message",
                                        24.,
                                    )
                                    .mt(px(6.))
                                    .opacity(0.)
                                    .group_hover("user-msg", |el| el.opacity(1.))
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.undo_to(sid, index);
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .when(!working, |el| {
                                el.child(
                                    self.ghost_button_sized(
                                        ("edit-msg", index),
                                        IconName::Pencil,
                                        "Edit and send again",
                                        24.,
                                    )
                                    .mt(px(6.))
                                    .opacity(0.)
                                    .group_hover("user-msg", |el| el.opacity(1.))
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.start_edit(sid, index, window, cx);
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .child(if message.playbook.is_empty() {
                                div()
                                    .max_w(px(560.))
                                    .px(px(14.))
                                    .py(px(10.))
                                    .rounded(px(14.))
                                    .bg(tint(c_active()))
                                    .text_size(px(13.5))
                                    .text_color(rgb(c_text()))
                                    .whitespace_normal()
                                    .child(markdown(message.text.clone()))
                                    .into_any_element()
                            } else {
                                self.playbook_message(index, message, cx)
                            }),
                    ),
            )
            .into_any_element();
        }
        if let Some(divider) = (message.role == Role::System)
            .then(|| {
                self.switch_divider(index, &message.meta)
                    .or_else(|| self.cli_divider(index, &message.meta))
                    .or_else(|| self.context_divider(index, &message.meta))
            })
            .flatten()
        {
            return divider;
        }
        if message.role == Role::System && message.meta == "sync" {
            return self.sync_card(index, message, cx);
        }
        if message.role == Role::System
            && matches!(
                message.meta.as_str(),
                "moved" | "note" | "warning" | "stopped"
            )
        {
            return fade(
                div()
                    .id(("message", index))
                    .px_3()
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .bg(tint(c_subtle()))
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child(match message.meta.as_str() {
                        "note" => icon(IconName::Info, 13., c_blue()),
                        "stopped" => icon(IconName::CircleStop, 13., c_muted()),
                        "warning" => icon(IconName::TriangleAlert, 13., c_amber()),
                        _ => icon(IconName::Server, 13., c_blue()),
                    })
                    .child(div().flex_1().min_w_0().child(message.text.clone())),
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
        let is_last = self
            .current()
            .and_then(|s| s.messages.iter().rposition(|m| m.role == Role::Agent))
            == Some(index);
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
                div().child(self.step_row(step, message.step_state(i, live), (key.0, index, i))),
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
                .when(!message.text.is_empty(), |el| {
                    el.child(
                        div()
                            .w_full()
                            .whitespace_normal()
                            .text_size(px(13.5))
                            .text_color(rgb(c_text()))
                            .child(markdown(message.text.clone())),
                    )
                })
                .when(!message.plan.is_empty(), |el| {
                    el.child(rise(
                        div().child(self.plan_card(index, &message.plan, is_last, cx)),
                        ("plan-rise", index),
                        320,
                    ))
                })
                .when(!message.questions.is_empty(), |el| {
                    el.child(rise(
                        div().child(self.question_card(
                            index,
                            &message.questions,
                            is_last && !working,
                            cx,
                        )),
                        ("question-rise", index),
                        320,
                    ))
                })
                .when(!message.denied.is_empty(), |el| {
                    el.child(rise(
                        div().child(self.denied_card(
                            index,
                            &message.denied,
                            is_last && !working,
                            cx,
                        )),
                        ("denied-rise", index),
                        320,
                    ))
                })
                .when(show_changes && !working, |el| {
                    el.child(rise(
                        div().child(self.changes_card(cx)),
                        ("changes-rise", index),
                        300,
                    ))
                })
                .when(is_last && !working, |el| {
                    el.children(
                        self.turn_bar(index, cx)
                            .map(|bar| rise(div().child(bar), ("turn-rise", index), 360)),
                    )
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
                            .when(!message.usage.is_empty(), |el| {
                                el.child("·").child(usage_label(&message.usage))
                            })
                            .when(!live, |el| {
                                let sid = self.selected.unwrap_or(0);
                                el.child(
                                    self.ghost_button_sized(
                                        ("fork", index),
                                        IconName::GitFork,
                                        "Fork from here: a new session with this conversation and the files as they were",
                                        22.,
                                    )
                                    .opacity(0.)
                                    .group_hover("message", |el| el.opacity(1.))
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.fork_from(sid, index, cx);
                                            cx.notify();
                                        },
                                    )),
                                )
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
            // Committed work counts too: everything since the branch left its base.
            let changes = self.race_stats.get(&id).copied();
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
                        el.border_color(rgb(c_blue())).bg(tint(c_selected()))
                    })
                    .when(!current, |el| {
                        el.border_color(rgb(c_border()))
                            .hover(|el| el.bg(tint(c_subtle())))
                    })
                    .when(racer.archived, |el| el.opacity(0.5))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(
                            "Open this agent's session with its whole diff against the base branch",
                        )
                        .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.diff_base = true;
                        this.right_tab = RightTab::Changes;
                        this.show_panel = true;
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
            .bg(tint(c_subtle()))
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
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("Race between {} agents", racers.len())),
                    )
                    .when(others > 0, |el| {
                        el.child(
                            self.button("keep-racer", "Keep this one", false)
                                .h(px(28.))
                                .text_xs()
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Stop the other agents and delete their worktrees and branches, then merge or open a PR for this one.",
                                    )
                                    .build(window, cx)
                                })
                                .child(icon(IconName::Trophy, 13., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirm_git = Some(ship::Confirm::Keep(me));
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(chips)
            .when(
                matches!(self.confirm_git, Some(ship::Confirm::Keep(k)) if k == me),
                |el| el.children(self.confirm_card(me, cx)),
            )
    }

    fn session_view(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.current() else {
            return self.empty_state(cx).into_any_element();
        };
        let provider = agent::provider(&session.provider);
        let working = self.handles.contains_key(&Target::Session(session.id));
        let id = session.id;
        let isolated = session.isolated;
        let last_agent = session.messages.iter().rposition(|m| m.role == Role::Agent);
        let mut list = div()
            .w_full()
            .when_some(cx.global::<AppSettings>().content_max_width(), |el, w| {
                el.max_w(px(w))
            })
            .mx_auto()
            .v_flex()
            .gap_6()
            .px_8()
            .pt_6()
            .pb_8();
        if session.race != 0 {
            list = list.child(self.race_strip(session, cx));
        }
        if let Some(card) = self.kept_card(id, cx) {
            list = list.child(card);
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
            list = list.child(
                div()
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Message(i), cx),
                    )
                    .child(self.message_view(
                        i,
                        message,
                        handoff::provider_at(session, i),
                        show,
                        working && Some(i) == last_agent,
                        cx,
                    )),
            );
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
                    .bg(tint(c_subtle()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(icon(IconName::Loader, 14., c_blue()))
                            .child("Preparing the worktree"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child(format!(
                                "The setup command is running in the Setup terminal tab, where you can answer what it asks. {} starts on “{}” when it finishes.",
                                provider.name,
                                task.lines().next().unwrap_or_default()
                            )),
                    ),
            );
        }
        // While the agent works, the chat ends with what it is doing and for how long.
        if working && !self.pending_start.contains_key(&id) {
            let since = self.started.get(&id).map(|start| elapsed(start.elapsed()));
            list = list.child(div().text_size(px(13.)).child(working_indicator(since)));
        }
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
                    // A long title gives way, so the branch beside it stays readable.
                    .min_w(px(64.))
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
                            .font_weight(FontWeight::MEDIUM)
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
                // The branch keeps room for a short name like "local folder" and a
                // long one is cut, with the full name in the tooltip.
                div()
                    .id("session-branch")
                    .flex()
                    .min_w(px(96.))
                    .flex_shrink(1.)
                    .tooltip({
                        let branch = session.branch.clone();
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(branch.clone())
                                .build(window, cx)
                        }
                    })
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child(icon(IconName::GitBranch, 12., c_muted()).flex_none())
                    .child(
                        div()
                            .max_w(px(160.))
                            .truncate()
                            .child(session.branch.clone()),
                    )
                    .when(isolated, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded_sm()
                                .bg(tint(c_subtle()))
                                .text_size(px(11.))
                                .child("worktree"),
                        )
                    }),
            )
            .child(div().flex_1())
            // Asked from a canvas: back to it, where its changes show.
            .children(shell::canvas_of(session).map(|view| {
                let id = session.id;
                div()
                    .id("hdr-canvas")
                    .h(px(26.))
                    .px_2()
                    .mr_2()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(5.))
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_session(id, window, cx);
                        cx.notify();
                    }))
                    .child(icon(
                        if view == View::Design {
                            Mode::Design.glyph()
                        } else {
                            Mode::Architecture.glyph()
                        },
                        12.,
                        c_secondary(),
                    ))
                    .child(format!("Open {}", shell::canvas_title(view)))
            }))
            .child(
                div()
                    .id("hdr-playbooks")
                    .h(px(26.))
                    .px_2()
                    .mr_2()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(5.))
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_mouse_down(
                        MouseButton::Left,
                        self.on_context(ctxmenu::CtxTarget::Playbooks(session.project_id), cx),
                    )
                    .child(icon(IconName::Play, 11., c_secondary()))
                    .child("Playbooks"),
            )
            // Only offered once something is being served for this session.
            .when(
                cx.global::<AppSettings>().browser_enabled && self.app_serving(session),
                |el| {
                    el.child(
                        div()
                            .id("hdr-browser")
                            .h(px(26.))
                            .px_2()
                            .mr_2()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(5.))
                            .rounded_md()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Open your app in a Browser tab",
                                )
                                .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_browser();
                                cx.notify();
                            }))
                            .child(icon(IconName::Globe, 13., c_secondary())),
                    )
                },
            )
            .children(self.runs_on_badge(session, cx))
            .child(self.view_toggle(session, cx))
            .when(!working, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .child(self.status_dot(session.status, "hdr-dot", 6.))
                        .child(Self::status_label(session.status)),
                )
            })
            .child(
                self.ghost_button("delete", IconName::Trash, "Delete session")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if confirm_delete() {
                            this.confirm_delete = Some(id);
                        } else {
                            this.delete_session(id);
                        }
                        cx.notify();
                    })),
            );
        // Chat and CLI are two views of one conversation; the swap crossfades.
        let cli = session.cli;
        let body = if cli {
            self.cli_body(session, window, cx)
        } else {
            div()
                .flex_1()
                .min_h_0()
                .v_flex()
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
        };
        let body = div().flex_1().min_h_0().v_flex().child(body).animated(
            SharedString::from(format!("view-{id}-{cli}")),
            Animation::new(Duration::from_millis(280)).with_easing(ease_out_quint()),
            move |el, d| {
                el.opacity(0.2 + 0.8 * d)
                    .top(px(if cli { 10. } else { -10. } * (1. - d)))
            },
        );
        let main = div().flex_1().min_w_0().v_flex().child(header).child(body);
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(main)
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
            .when(open, |el| el.bg(tint(c_active())).text_color(rgb(c_text())))
            .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
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

    /// The rows of a composer menu; picking one runs `on_pick` and closes the menu.
    fn menu_rows(
        &self,
        items: Vec<(String, String, bool, Option<AnyElement>)>,
        on_pick: impl Fn(&mut Self, usize) + 'static + Clone,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().v_flex().p_1();
        for (i, (label, desc, selected, lead)) in items.into_iter().enumerate() {
            let on_pick = on_pick.clone();
            list = list.child(
                menu_row(("menu-item", i), label, desc, selected, lead).on_click(cx.listener(
                    move |this, _, _, cx| {
                        on_pick(this, i);
                        this.menu = None;
                        this.menu_agent = None;
                        cx.notify();
                    },
                )),
            );
        }
        list
    }

    /// A composer menu opening upward from its chip. It is deferred so it paints above
    /// everything around it, the composer's own outline included, and its surface is solid
    /// so nothing behind it shows through.
    fn menu_popup(
        &self,
        key: &str,
        content: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .pb(px(6.))
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.menu = None;
                                    this.menu_agent = None;
                                    cx.notify();
                                }))
                                .child(content)
                                // Fade in while rising a few pixels.
                                .animated(
                                    SharedString::from(format!("menu-{key}")),
                                    Animation::new(Duration::from_millis(180))
                                        .with_easing(ease_out_quint()),
                                    |el, d| el.opacity(d).top(px(6. * (1. - d))),
                                ),
                        ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    /// The agent menu: agents on the left, and beside them the models of the one under the
    /// pointer (the session's own agent at first), so agent and model are one pick. Only
    /// agents that are set up are listed; a note points to Settings for the rest.
    fn agent_menu(&self, session: &Session, cx: &mut Context<Self>) -> AnyElement {
        let sid = session.id;
        let current = agent::PROVIDERS
            .iter()
            .position(|p| p.id == session.provider)
            .unwrap_or(0);
        let shown = self.menu_agent.unwrap_or(current);
        let from_bin = cx.global::<AppSettings>().agent(&session.provider).binary;
        // Agents still being detected stay listed until Runo knows they're missing.
        let listed = |i: usize| {
            let caps = caps::get(agent::PROVIDERS[i].id);
            i == current || caps.installed || !caps.detected
        };
        let hidden = (0..agent::PROVIDERS.len()).filter(|&i| !listed(i)).count();

        let mut agents = div().v_flex().p_1();
        for (i, p) in agent::PROVIDERS.iter().enumerate() {
            if !listed(i) {
                continue;
            }
            let from_bin = from_bin.clone();
            agents = agents.child(
                menu_row(
                    ("agent-item", i),
                    p.name.to_string(),
                    String::new(),
                    i == current,
                    Some(self.provider_badge(p, 18.).into_any_element()),
                )
                .when(i == shown, |el| el.bg(tint(c_subtle())))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && this.menu_agent != Some(i) {
                        this.menu_agent = Some(i);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.request_switch(sid, i, &from_bin);
                    this.menu = None;
                    this.menu_agent = None;
                    cx.notify();
                }))
                .child(icon(IconName::ChevronRight, 12., c_muted())),
            );
        }
        if hidden > 0 {
            agents = agents.child(
                div()
                    .id("agent-setup-note")
                    .mt_1()
                    .pt(px(6.))
                    .pb(px(4.))
                    .px_2()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .max_w(px(220.))
                    .whitespace_normal()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .cursor_pointer()
                    .hover(|el| el.text_color(rgb(c_text())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        this.menu_agent = None;
                        this.open_settings();
                        this.settings_page = SettingsPage::Providers;
                        cx.notify();
                    }))
                    .child(
                        "Set up other agents in Settings › Providers to make them available here.",
                    ),
            );
        }

        let p = &agent::PROVIDERS[shown];
        let sel = if shown == current {
            session.selections()
        } else {
            HashMap::new()
        };
        let picked = sel.get("model").cloned().unwrap_or_default();
        let caps = caps::get(p.id);
        let choices = caps::controls(p.id, &sel)
            .into_iter()
            .find(|c| c.id == "model")
            .map(|c| c.choices)
            .unwrap_or_else(|| {
                vec![caps::Choice {
                    value: String::new(),
                    label: "Default".into(),
                    desc: if caps.detected {
                        String::new()
                    } else {
                        "Detecting models…".into()
                    },
                }]
            });
        let mut models = div().v_flex().p_1().child(
            div()
                .px_2()
                .pt_1()
                .pb(px(4.))
                .text_size(px(11.))
                .text_color(rgb(c_muted()))
                .child(format!("{} models", p.name)),
        );
        for (j, choice) in choices.into_iter().enumerate() {
            let from_bin = from_bin.clone();
            let value = choice.value.clone();
            models = models.child(
                menu_row(
                    SharedString::from(format!("model-item-{shown}-{j}")),
                    choice.label,
                    choice.desc,
                    shown == current && choice.value == picked,
                    None,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pick_model(sid, shown, value.clone(), &from_bin);
                    this.menu = None;
                    this.menu_agent = None;
                    cx.notify();
                })),
            );
        }

        let content = div()
            .flex()
            .items_end()
            .gap_1()
            .child(menu_box("agents", agents).min_w(px(190.)))
            .child(menu_box(&format!("models-{shown}"), models).min_w(px(230.)));
        self.menu_popup("provider", content, cx)
    }

    /// Run a session on `model` of agent `to`, switching agents first when it is another one.
    fn pick_model(&mut self, sid: u64, to: usize, model: String, from_bin: &str) {
        let target = agent::PROVIDERS[to].id;
        if self
            .state
            .session(sid)
            .is_some_and(|s| s.provider != target)
        {
            self.request_switch(sid, to, from_bin);
        }
        if let Some(s) = self.state.session_mut(sid).filter(|s| s.provider == target) {
            s.set_model(model);
        } else if let Some(plan) = self
            .switch_plan
            .as_mut()
            .filter(|p| p.session == sid && p.to == to)
        {
            // The switch waits for confirmation; the model applies once it goes through.
            plan.model = Some(model);
        }
        self.save();
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
            popup = Some(("provider".into(), self.agent_menu(session, cx)));
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
            let rows = self.menu_rows(
                items,
                move |this, i| {
                    if let Some(s) = this.state.session_mut(sid) {
                        let v = values[i].clone();
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
            );
            popup = Some((
                id.to_string(),
                self.menu_popup(id, menu_box(id, rows).min_w(px(220.)), cx),
            ));
        }

        // The agent chip carries the model too: once one is picked it shows by name beside the
        // agent's mark, and the line under the composer spells out both. The model has no chip
        // of its own; it is picked from the agent menu.
        let picked_model = sel.get("model").cloned().unwrap_or_default();
        let model_label = controls
            .iter()
            .find(|c| c.id == "model")
            .and_then(|c| c.choices.iter().find(|ch| ch.value == picked_model))
            .map(|ch| ch.label.clone())
            .filter(|_| !picked_model.is_empty())
            .or_else(|| (!picked_model.is_empty()).then(|| picked_model.clone()));
        let agent_label = model_label.unwrap_or_else(|| provider.name.to_string());
        let mut chips = div().flex().flex_wrap().items_center().gap_1().child(
            div()
                .relative()
                .child(tour::anchor(&self.anchors, "agent-chip"))
                // A new agent fades in when the session switches.
                .child(fade(
                    div().child(self.menu_button(
                        SharedString::from("chip-provider"),
                        "provider",
                        Some(self.provider_mark(provider, 13.).into_any_element()),
                        None,
                        agent_label,
                        cx,
                    )),
                    SharedString::from(format!("chip-provider-{}", provider.id)),
                    300,
                ))
                .when(popup.as_ref().is_some_and(|(k, _)| k == "provider"), |el| {
                    el.children(popup.take().map(|(_, p)| p))
                }),
        );
        for control in controls.iter().filter(|c| c.id != "model") {
            let mut current = sel.get(control.id).cloned().unwrap_or_default();
            if control.id == "mode" {
                // The level the agent really runs at, for CLIs without the one picked.
                current = agent::effective_mode(&session.provider, &current).to_string();
            }
            // Left at its default, a control shows just its name ("Reasoning"); once changed
            // it shows the choice ("Fast", "Auto edit", "High reasoning"), with its name in
            // the tooltip. Either way the row stays short enough for one line.
            let label = if current.is_empty() {
                control.label.clone()
            } else {
                let choice = control
                    .choices
                    .iter()
                    .find(|c| c.value == current)
                    .map(|c| c.label.clone())
                    .unwrap_or_else(|| {
                        agent::generic_mode_label(&current)
                            .map(str::to_string)
                            .unwrap_or_else(|| current.clone())
                    });
                if control.id == "effort" {
                    format!("{choice} {}", control.label.to_lowercase())
                } else {
                    choice
                }
            };
            let caption = control.label.clone();
            chips = chips.child(
                div()
                    .relative()
                    .child(
                        self.menu_button(
                            SharedString::from(format!("chip-{}", control.id)),
                            control.id,
                            None,
                            None,
                            label,
                            cx,
                        )
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(caption.clone())
                                .build(window, cx)
                        }),
                    )
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

        let picked = self
            .picked_playbook
            .and_then(|id| self.state.automations.iter().find(|a| a.id == id));
        let ready = !empty_prompt || !self.attached().is_empty() || picked.is_some();
        // Attach sits beside Send, so the option chips get the row to themselves. Only
        // agents that can read images get it.
        let attach = agent::takes_images(&session.provider).then(|| {
            self.ghost_button_sized(
                "attach",
                IconName::Paperclip,
                "Attach an image or file (pastes a copied image)",
                26.,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.attach(cx);
                cx.notify();
            }))
        });
        div()
            .w_full()
            .max_w(px(780.))
            .mx_auto()
            .px_6()
            .pb_3()
            .v_flex()
            .gap(px(6.))
            .children(self.composer_extras(cx))
            .child(
                div()
                    .id("composer-box")
                    .relative()
                    .child(tour::anchor(&self.anchors, "composer"))
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
                    .drag_over::<ExternalPaths>(|el, _, _, _| el.border_color(rgb(c_blue())))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                        this.attach_paths(paths.paths());
                        cx.notify();
                    }))
                    .when_some(picked, |el, a| el.child(self.playbook_chip(a, cx)))
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
                            .children(attach)
                            // While the agent works the send button becomes Stop; a
                            // typed message still goes out (queued) from its own button.
                            .when(working, |el| {
                                el.child(
                                    div()
                                        .id("composer-stop")
                                        .size(px(28.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .cursor_pointer()
                                        .bg(rgb(c_text()))
                                        .hover(|el| el.opacity(0.85))
                                        .tooltip(|window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                "Stop the agent",
                                            )
                                            .build(window, cx)
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.stop();
                                            cx.notify();
                                        }))
                                        .child(
                                            div().size(px(10.)).rounded(px(2.)).bg(rgb(c_canvas())),
                                        ),
                                )
                            })
                            .when(!working || ready, |el| {
                                el.child(
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
                                        .tooltip(move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(if working {
                                                "Queue: sent when the agent finishes"
                                            } else {
                                                "Send"
                                            })
                                            .build(window, cx)
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.send_message(window, cx);
                                            cx.notify();
                                        }))
                                        .child(icon(
                                            IconName::ArrowUp,
                                            15.,
                                            if ready { c_canvas() } else { c_muted() },
                                        )),
                                )
                            }),
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
                            })
                            // What the next message runs on, so it is never a guess.
                            .child(div().px_1().child("·"))
                            .child(self.provider_mark(provider, 11.))
                            .child(run_note(provider, &sel)),
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
                        .bg(tint(c_subtle()))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
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
                    .bg(tint(c_selected()))
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
                            .font_weight(FontWeight::SEMIBOLD)
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
                                this.open_new_session();
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
                    .child(tip(keymap::label(cx, "new-session"), "New session"))
                    .child(tip(keymap::label(cx, "settings"), "Settings"))
                    .child(tip(keymap::label(cx, "panel"), "Show or hide the changes panel"))
                    .child(tip(keymap::label(cx, "stop"), "Stop the running agent")),
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
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    fn open_new_session(&mut self) {
        signin::refresh(false);
        if self.view != View::NewSession {
            self.new_back = if self.view == View::Settings {
                View::Session
            } else {
                self.view
            };
        }
        self.view = View::NewSession;
        if let Some(pid) = self.new_project {
            self.new_connection = self.project_connection(pid);
        }
    }

    /// A project's default "Runs on", if that connection still exists.
    fn project_connection(&self, pid: u64) -> u64 {
        self.state
            .projects
            .iter()
            .find(|p| p.id == pid)
            .map(|p| p.connection)
            .filter(|c| connections::get(*c).is_some())
            .unwrap_or(connections::LOCAL)
    }

    fn close_new_session(&mut self) {
        if self.view == View::NewSession {
            self.view = self.new_back;
        }
        self.new_context = None;
    }

    /// "Delete this session?" over the current view, on a dimmed backdrop.
    fn delete_session_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.confirm_delete?;
        let session = self.state.session(id)?;
        let running = self.handles.contains_key(&Target::Session(id));
        let worktree = session.isolated && !self.demo && remove_worktree();
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let mut what = vec!["Its conversation is removed from Runo."];
        if running {
            what.push("The running agent stops.");
        }
        what.push(if worktree {
            "Its worktree is deleted, with any uncommitted work in it."
        } else {
            "Files in the project folder stay as they are."
        });
        let card = div()
            .id("delete-session-card")
            .w(px(440.))
            .max_w(relative(0.94))
            .p_6()
            .v_flex()
            .gap_4()
            .rounded_2xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(36.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .bg(rgb(settings::mix(c_red(), c_canvas(), 0.12)))
                            .child(icon(IconName::Trash, 18., c_red())),
                    )
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Delete this session?"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(rgb(c_muted()))
                                    .child(session.title.clone()),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(c_secondary()))
                    .child(what.join(" ")),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("delete-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_delete = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "delete-go",
                            if worktree {
                                "Delete session & worktree"
                            } else {
                                "Delete session"
                            },
                            false,
                        )
                        .border_color(rgb(c_red()))
                        .text_color(rgb(c_red()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_session(id);
                            cx.notify();
                        })),
                    ),
            );
        Some(
            fade(
                div()
                    .id("delete-session-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(backdrop)
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_delete = None;
                        cx.notify();
                    }))
                    .child(rise(card, "delete-session-rise", 300)),
                "delete-session-fade",
                200,
            )
            .into_any_element(),
        )
    }

    /// New session as a dialog over the current view, on a dimmed backdrop.
    fn new_session_modal(&self, height: f32, cx: &mut Context<Self>) -> AnyElement {
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.28)
        };
        fade(
            div()
                .id("new-session-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(backdrop)
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_new_session();
                    cx.notify();
                }))
                .child(rise(
                    div()
                        .id("new-session-card")
                        .w(px(760.))
                        .max_w(relative(0.94))
                        .max_h(px(height.clamp(360., 820.)))
                        .relative()
                        .v_flex()
                        .rounded_2xl()
                        .border_1()
                        .border_color(float_edge())
                        .bg(float(c_canvas()))
                        .shadow_2xl()
                        .overflow_hidden()
                        // Clicks inside the dialog must not reach the backdrop.
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(self.new_session_view(cx))
                        .child(
                            div().absolute().top_4().right_4().child(
                                self.ghost_button_sized(
                                    "new-session-close",
                                    IconName::X,
                                    "Close (Esc)",
                                    28.,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.close_new_session();
                                        cx.notify();
                                    },
                                )),
                            ),
                        ),
                    "new-session-rise",
                    320,
                )),
            "new-session-fade",
            200,
        )
    }

    fn new_session_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = |t: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(c_muted()))
                .child(t)
        };
        let card = |id: (&'static str, usize), selected: bool| {
            div()
                .id(id)
                .flex_1()
                .min_w(px(200.))
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_3()
                .rounded_xl()
                .border_1()
                .cursor_pointer()
                .when(selected, |el| {
                    el.border_color(rgb(c_blue())).bg(tint(c_selected()))
                })
                .when(!selected, |el| {
                    el.border_color(rgb(c_border()))
                        .hover(|el| el.bg(tint(c_subtle())))
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

        // The main agents (the first three, anything installed, anything picked)
        // show first; the rest wait behind "More agents".
        let installed_all: Vec<bool> = agent::PROVIDERS
            .iter()
            .map(|p| agent::available(p.bin, &cx.global::<AppSettings>().agent(p.id).binary))
            .collect();
        let major = |i: usize| {
            i < 3
                || installed_all[i]
                || if self.race_mode {
                    self.new_race.contains(&i)
                } else {
                    self.new_provider == i
                }
        };
        let hidden = (0..agent::PROVIDERS.len()).filter(|i| !major(*i)).count();
        let mut providers = div().flex().flex_wrap().gap_2();
        for (i, p) in agent::PROVIDERS.iter().enumerate() {
            if !self.new_more_agents && !major(i) {
                continue;
            }
            let caps = caps::get(p.id);
            let installed = installed_all[i];
            let signed_out = installed && signin::get(p.id) == Some(false);
            let detail = if !installed {
                "Not installed".to_string()
            } else if signed_out {
                "Not signed in".to_string()
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
                    .child(self.provider_badge(p, 26.))
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(p.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(if !installed {
                                        c_red()
                                    } else if signed_out {
                                        c_amber()
                                    } else {
                                        c_muted()
                                    }))
                                    .child(detail),
                            ),
                    )
                    .when(!installed, |el| el.child(self.install_button(p.id, cx))),
            );
        }

        if hidden > 0 {
            let more = self.new_more_agents;
            providers = providers.child(
                div()
                    .id("more-agents")
                    .flex_1()
                    .min_w(px(200.))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded_xl()
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .text_sm()
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.new_more_agents = !this.new_more_agents;
                        cx.notify();
                    }))
                    .child(icon(
                        if more {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        },
                        14.,
                        c_secondary(),
                    ))
                    .child(if more {
                        "Fewer agents".to_string()
                    } else {
                        format!("More agents ({hidden})")
                    }),
            );
        }

        let project = self
            .new_project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id));
        let is_git = self.demo || project.is_some_and(|p| git::is_repo(&p.path));
        let racing = self.racing() && is_git;
        let isolated = (self.new_isolated || racing) && is_git;
        let typed = git::sanitize_branch(&self.branch.read(cx).value());
        let task_text = self.new_task.read(cx).value().to_string();
        let branch = if racing {
            format!(
                "{}-*",
                if typed.is_empty() {
                    Some(git::branch_slug(&task_text))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("runo/race-{}", self.state.next_id + 1))
                } else {
                    typed
                }
            )
        } else if typed.is_empty() {
            let fallback = format!("runo/session-{}", self.state.next_id + 1);
            match project {
                Some(p) => self.task_branch(p, &task_text, fallback, |n| self.listed_branch(n)),
                None => fallback,
            }
        } else {
            typed
        };
        let from = self
            .new_base
            .clone()
            .unwrap_or_else(|| self.listed_default().to_string());
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
            Some(p) if isolated && !from.is_empty() => format!(
                "Creates branch {branch} from {from} in {}",
                git::worktree_path(&p.path, &branch).display()
            ),
            Some(p) if isolated => format!(
                "Creates branch {branch} in {}",
                git::worktree_path(&p.path, &branch).display()
            ),
            Some(p) if is_git => format!("Runs on the current branch, directly in {}", p.path),
            Some(p) => format!("Runs directly in {}", p.path),
        };
        let cove_ready = !self.demo && cove::available();
        let conns = connections::all();
        let where_to = match connections::get(self.new_connection) {
            Some(c) if project.is_some() && c.remote() => {
                format!("{where_to}, synced to {} where the agent runs", c.name)
            }
            Some(c) if project.is_some() => format!("{where_to}, inside a VM on {}", c.name),
            _ => where_to,
        };
        let provider = &agent::PROVIDERS[self.new_provider];
        let task_empty = self.new_task.read(cx).value().trim().is_empty();
        let ready = project.is_some() && !task_empty && (!self.race_mode || racing);

        let runs_on = |id: usize, title: &'static str, desc: &'static str, on: bool| {
            card(("runs-on", id), on)
                .child(icon(
                    if id == 0 {
                        IconName::Cpu
                    } else {
                        IconName::ShieldCheck
                    },
                    18.,
                    if on { c_blue() } else { c_secondary() },
                ))
                .child(
                    div()
                        .v_flex()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                        .child(div().text_xs().text_color(rgb(c_muted())).child(desc)),
                )
        };
        let runs_on_conn = |id: usize,
                            title: SharedString,
                            desc: SharedString,
                            kind: connections::Kind,
                            on: bool| {
            card(("runs-on", id), on)
                .child(icon(
                    if kind == connections::Kind::Ssh {
                        IconName::Server
                    } else {
                        IconName::ShieldCheck
                    },
                    18.,
                    if on { c_blue() } else { c_secondary() },
                ))
                .child(
                    div()
                        .v_flex()
                        .min_w_0()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(c_muted()))
                                .truncate()
                                .child(desc),
                        ),
                )
        };
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
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
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
                    .px_8()
                    .py_6()
                    .v_flex()
                    .gap_3()
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
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
                                        .hover(|el| el.bg(tint(c_subtle())))
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
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(Input::new(&self.branch).bordered(true).h(px(36.))))
                                .when(!self.demo, |el| el.child(self.base_picker(cx))),
                        )
                    })
                    .when(project.is_some() && (cove_ready || !conns.is_empty()), |el| {
                        let mut choices = div().flex().flex_wrap().gap_2().child(
                            runs_on(0, "This computer", "Directly on your machine", self.new_connection == connections::LOCAL)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.new_connection = connections::LOCAL;
                                    cx.notify();
                                })),
                        );
                        for (i, c) in conns.iter().enumerate() {
                            let id = c.id;
                            let summary: SharedString = c.summary().into();
                            choices = choices.child(
                                runs_on_conn(i + 1, c.name.clone().into(), summary, c.kind, self.new_connection == id)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.new_connection = id;
                                        cx.notify();
                                    })),
                            );
                        }
                        if cove_ready && !conns.iter().any(|c| c.kind == connections::Kind::Cove && !c.remote()) {
                            choices = choices.child(
                                runs_on(99, "Cove VM", "Its own Linux VM, away from your files and keys", false)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let id = this.add_connection(connections::Kind::Cove, cx);
                                        this.new_connection = id;
                                        cx.notify();
                                    })),
                            );
                        }
                        let project_id = project.map(|p| p.id).unwrap_or_default();
                        let is_default = project.is_some_and(|p| p.connection == self.new_connection);
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(label("RUNS ON"))
                                .when(!is_default, |el| {
                                    el.child(
                                        div()
                                            .id("runs-on-default")
                                            .text_xs()
                                            .text_color(rgb(c_blue()))
                                            .cursor_pointer()
                                            .child("Make this the project's default")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let target = this.new_connection;
                                                if let Some(p) = this.state.projects.iter_mut().find(|p| p.id == project_id) {
                                                    p.connection = target;
                                                }
                                                this.save();
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                        .child(choices)
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
                    .child(Textarea::new(&self.new_task).bordered(true).h(px(110.)))
                    .children(self.context_chip(cx))
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
        let query = self.finder.query.read(cx).value().trim().to_string();
        let sessions = sessions::filtered(
            &self.state,
            &self.finder.filter,
            &query,
            &self.finder.list_hits,
            worktrees_only,
        );
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
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_secondary()))
                        .child("Archived"),
                );
            }
            let (archived, pinned, unread) = (session.archived, session.pinned, session.unread);
            rows = rows.child(stagger(
                div()
                    .id(("row", id as usize))
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Session(id), cx),
                    )
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
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_session(id, window, cx);
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
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(session.title.clone()),
                                    ),
                            )
                            .child(div().truncate().text_xs().text_color(rgb(c_muted())).child(
                                if let Some(said) = self.finder.list_hits.snippet(&query, id) {
                                    format!("{project}  ·  “{said}”")
                                } else if let Some(view) = shell::canvas_of(session) {
                                    format!(
                                        "{project}  ·  Asked from {}",
                                        shell::canvas_title(view)
                                    )
                                } else if worktrees_only && !self.demo {
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
                                    22.,
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
                                22.,
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
        let narrowed = !query.is_empty() || self.finder.filter != Default::default();
        if sessions.is_empty() && narrowed {
            rows = rows.child(empty_hint(
                "list-none",
                IconName::Search,
                "No sessions match",
                "Try other words, or clear the filters.",
            ));
        } else if sessions.is_empty() {
            rows = rows.child(
                div()
                    .py_10()
                    .v_flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::Layers, 28., c_muted()))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Nothing here yet"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(c_muted()))
                            .child("Start a session to see it here."),
                    ),
            );
        }
        self.page(
            match kind {
                View::Worktrees => "Isolated sessions",
                _ => "Sessions",
            },
            div()
                .v_flex()
                .gap_4()
                .when(!self.state.sessions.is_empty(), |el| {
                    el.child(self.list_toolbar(worktrees_only, cx))
                })
                .child(rows),
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
                el.bg(tint(c_active())).text_color(rgb(c_text()))
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

    /// The selected file's diff, or every changed file one after another.
    fn diff_view(&self, cx: &mut Context<Self>) -> Div {
        if !self.diff_all {
            let path = self.selected_file.clone().unwrap_or_default();
            return self.diff_lines(&path, &self.diff, cx);
        }
        let mut col = div().v_flex();
        for (path, diff) in &self.all_diffs {
            let id = path.clone();
            col = col
                .child(
                    div()
                        .id(SharedString::from(format!("all-{path}")))
                        .h(px(28.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_y_1()
                        .border_color(divider())
                        .bg(inset(c_sidebar()))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .font_family(self.mono_font.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.diff_all = false;
                            this.open_change(&id);
                            cx.notify();
                        }))
                        .child(icon(IconName::File, 12., c_muted()))
                        .child(path.clone()),
                )
                .child(self.diff_lines(path, diff, cx));
        }
        col
    }

    fn diff_toggle(
        &self,
        id: &'static str,
        name: IconName,
        tip: &'static str,
        on: bool,
        flip: fn(&mut Self),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        self.ghost_button_sized(id, name, tip, 24.)
            .when(on, |el| el.bg(tint(c_active())))
            .on_click(cx.listener(move |this, _, _, cx| {
                flip(this);
                this.reload_diff();
                cx.notify();
            }))
    }

    fn load_diff(&self, dir: &str, path: &str) -> String {
        git::file_diff_context(dir, path, self.diff_whole, &self.diff_against)
    }

    /// Re-read the diff after a view option changed.
    fn reload_diff(&mut self) {
        if self.demo {
            return;
        }
        let Some(dir) = self.current().map(|s| s.dir.clone()) else {
            return;
        };
        if let Some(path) = self.selected_file.clone() {
            self.diff = self.load_diff(&dir, &path);
        }
        self.all_diffs = if self.diff_all {
            self.changes
                .iter()
                .map(|c| (c.path.clone(), self.load_diff(&dir, &c.path)))
                .collect()
        } else {
            Vec::new()
        };
    }

    fn diff_lines(&self, path: &str, diff: &str, cx: &mut Context<Self>) -> Div {
        use git::LineKind;
        let path = path.to_string();
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
        let rows = git::diff_rows(diff);
        let hidden = collapsible_runs(&rows);
        let mut skip_to = 0;
        // Element ids must stay unique when several files are shown together.
        let base = self
            .changes
            .iter()
            .position(|c| c.path == path)
            .unwrap_or(0)
            * 100_000;
        const MAX_ROWS: usize = 4000;
        let more = rows.len().saturating_sub(MAX_ROWS);
        for (local, row) in rows.into_iter().enumerate().take(MAX_ROWS) {
            let i = base + local;
            if local < skip_to {
                continue;
            }
            if let Some(&(start, end)) = hidden.iter().find(|(s, _)| *s == local) {
                let key = (path.clone(), start);
                if !self.diff_expanded.contains(&key) {
                    skip_to = end;
                    col = col.child(
                        div()
                            .id(("dl-fold", i))
                            .h(px(22.))
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .bg(tint(c_subtle()))
                            .text_color(rgb(c_muted()))
                            .hover(|el| el.text_color(rgb(c_text())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.diff_expanded.insert(key.clone());
                                cx.notify();
                            }))
                            .child(icon(IconName::ChevronsUpDown, 12., c_muted()))
                            .child(format!("{} unchanged lines", end - start)),
                    );
                    continue;
                }
            }
            if row.kind == LineKind::Hunk {
                col = col.child(
                    div()
                        .id(("dl", i))
                        .mt_1()
                        .px_3()
                        .py(px(3.))
                        .text_color(rgb(c_blue()))
                        .bg(tint(c_selected()))
                        .whitespace_nowrap()
                        .child(row.text),
                );
                continue;
            }
            let (bg, mark, color) = match row.kind {
                LineKind::Added => (Some(c_add()), "+", c_add()),
                LineKind::Removed => (Some(c_del()), "−", c_del()),
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
                    .when_some(bg, |el, bg| el.bg(diff_fill(bg)))
                    .when(can_note, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(tint(c_selected())))
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
                            .min_w_0()
                            .when(!self.diff_wrap, |el| el.whitespace_nowrap())
                            .when(self.diff_wrap, |el| el.whitespace_normal())
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
                                .hover(|el| el.bg(tint(c_subtle())))
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
                                .hover(|el| el.bg(tint(c_subtle())))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.note_at = None;
                                    cx.notify();
                                }))
                                .child("Cancel"),
                        ),
                );
            }
        }
        if more > 0 {
            col = col.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(rgb(c_muted()))
                    .italic()
                    .child(format!(
                        "{more} more line{} not shown. Open the file to see all of it.",
                        if more == 1 { "" } else { "s" }
                    )),
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
                .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child("Changes"))
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
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Change(change.path.clone()), cx),
                    )
                    .group("change-row")
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(tint(c_active())))
                    .when(!selected, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.demo {
                            this.selected_file = Some(path.clone());
                        } else if let Some(dir) = this.current().map(|s| s.dir.clone()) {
                            this.diff = this.load_diff(&dir, &path);
                            this.selected_file = Some(path.clone());
                        }
                        cx.notify();
                    }))
                    .when(!self.diff_base, |el| {
                        let (stage_path, staged) = (change.path.clone(), change.staged);
                        let partly = change.staged && change.unstaged;
                        el.child(
                            div()
                                .id(("stage", i))
                                .size(px(14.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .border_1()
                                .cursor_pointer()
                                .when(staged, |el| {
                                    el.bg(rgb(c_blue())).border_color(rgb(c_blue()))
                                })
                                .when(!staged, |el| el.border_color(rgb(c_border())))
                                .hover(|el| el.border_color(rgb(c_blue())))
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(if partly {
                                        "Partly staged. Click to unstage"
                                    } else if staged {
                                        "Staged. Click to unstage"
                                    } else {
                                        "Stage this file; Commit then takes only staged files"
                                    })
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_stage(&stage_path, staged);
                                    cx.notify();
                                }))
                                .when(staged, |el| {
                                    el.child(icon(
                                        if partly {
                                            IconName::Minus
                                        } else {
                                            IconName::Check
                                        },
                                        10.,
                                        c_canvas(),
                                    ))
                                }),
                        )
                    })
                    .child(
                        div()
                            .w(px(12.))
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
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
                                    .hover(|el| el.bg(tint(c_subtle())))
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
                            .when(change.binary, |el| {
                                el.child(div().text_color(rgb(c_muted())).child("binary"))
                            })
                            .when(!change.binary, |el| {
                                el.child(
                                    div()
                                        .text_color(rgb(c_add()))
                                        .child(format!("+{}", change.added)),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(c_del()))
                                        .child(format!("−{}", change.removed)),
                                )
                            })
                            .when(!self.diff_base, |el| {
                                el.child(
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
                                        .hover(|el| el.bg(tint(c_subtle())))
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
                            })
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
            // Shipping happens on Review; a session's panel only points there.
            .child(div().p_3().child(if self.view == View::Review {
                self.ship_bar(cx).into_any_element()
            } else {
                self.review_strip(cx)
            }))
            .child(
                div()
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
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
                    .when(!self.branch_info.base_ref.is_empty(), |el| {
                        let label = if self.diff_base {
                            format!("Since {}", self.branch_info.base)
                        } else {
                            "Uncommitted".to_string()
                        };
                        el.child(
                            div()
                                .id("changes-scope")
                                .px_2()
                                .h(px(20.))
                                .flex()
                                .items_center()
                                .rounded_md()
                                .cursor_pointer()
                                .font_weight(FontWeight::NORMAL)
                                .text_color(rgb(c_secondary()))
                                .bg(tint(c_subtle()))
                                .hover(|el| el.text_color(rgb(c_text())))
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Switch between uncommitted changes and everything the branch changed since it left its base, commits included",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.diff_base = !this.diff_base;
                                    this.selected_file = None;
                                    this.refresh();
                                    cx.notify();
                                }))
                                .child(label),
                        )
                    })
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
                            .font_weight(FontWeight::MEDIUM)
                            .child(if self.diff_base {
                                format!("No changes since {}", self.branch_info.base)
                            } else {
                                "No uncommitted changes".to_string()
                            }),
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
                        .border_color(divider())
                        .bg(inset(c_canvas()))
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
                                        .child(if self.diff_all {
                                            format!("All changes · {} files", self.changes.len())
                                        } else {
                                            selected.clone()
                                        }),
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
                                .child(self.diff_toggle(
                                    "diff-all",
                                    IconName::Layers,
                                    "Show every changed file",
                                    self.diff_all,
                                    |this| this.diff_all = !this.diff_all,
                                    cx,
                                ))
                                .child(self.diff_toggle(
                                    "diff-whole",
                                    IconName::UnfoldVertical,
                                    "Show whole files (unchanged runs fold)",
                                    self.diff_whole,
                                    |this| this.diff_whole = !this.diff_whole,
                                    cx,
                                ))
                                .child(self.diff_toggle(
                                    "diff-wrap",
                                    IconName::TextWrap,
                                    "Wrap long lines",
                                    self.diff_wrap,
                                    |this| this.diff_wrap = !this.diff_wrap,
                                    cx,
                                ))
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
                                .child(self.diff_view(cx)),
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

    /// The Output tab: what Runo itself printed for the session, such as setup and push logs.
    fn output_tab(&self, selected: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("term-output")
            .flex_none()
            .h(px(22.))
            .px_2()
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .when(selected, |el| {
                el.bg(tint(c_active())).text_color(rgb(c_text()))
            })
            .when(!selected, |el| {
                el.text_color(rgb(c_muted()))
                    .hover(|el| el.text_color(rgb(c_text())))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.shell_tab = None;
                cx.notify();
            }))
            .child(icon(
                IconName::FileText,
                12.,
                if selected { c_text() } else { c_muted() },
            ))
            .child("Output")
    }

    /// Nothing open yet: start a shell, or one of the quick commands.
    fn terminal_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let folder = self
            .current()
            .and_then(|s| {
                std::path::Path::new(&s.dir)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "the session folder".into());
        let commands = settings::parse_quick_commands(&cx.global::<AppSettings>().quick_commands);
        let mut actions = div().flex().flex_wrap().justify_center().gap_2().child(
            self.button("term-empty-shell", "New shell", true)
                .h(px(26.))
                .text_size(px(12.))
                .on_click(cx.listener(|this, _, _, cx| this.new_shell(cx))),
        );
        for (i, (name, command)) in commands.into_iter().enumerate().take(4) {
            actions = actions.child(
                self.button(("term-empty-run", i), format!("Run {name}"), false)
                    .h(px(26.))
                    .text_size(px(12.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_task(name.clone(), command.clone(), cx)
                    })),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .items_center()
            .justify_center()
            .gap_3()
            .px_4()
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .text_center()
                    .child(format!("Terminals here open in {folder}.")),
            )
            .child(actions)
            .into_any_element()
    }

    fn terminal_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.selected.unwrap_or(0);
        let shell = self.active_shell();
        let has_output = self.has_output(id);
        let has_commands =
            !settings::parse_quick_commands(&cx.global::<AppSettings>().quick_commands).is_empty();
        // A quick command still going in the tab on screen can be stopped from the header.
        let running_task = shell
            .filter(|sh| !sh.exited() && self.tasks.contains_key(&sh.id))
            .map(|sh| sh.id);
        let shell_tabs: Vec<Div> = self
            .session_shells(id)
            .map(|sh| self.shell_tab_button(sh, cx))
            .collect();
        let body = match shell {
            Some(sh) => self.shell_view(sh, window, cx),
            None if has_output => {
                // New output keeps the newest line in view, unless the person scrolled
                // up to read; a session's output opens at its end.
                let lines = self.terminal[&id].len();
                let (seen_id, seen) = self.term_seen.replace((id, lines));
                if seen_id != id
                    || (lines != seen
                        && terminal::follows_tail(
                            f32::from(self.term_scroll.offset().y),
                            f32::from(self.term_scroll.max_offset().y),
                        ))
                {
                    self.term_scroll.scroll_to_bottom();
                }
                let text: Vec<&str> = self.terminal[&id]
                    .iter()
                    .rev()
                    .take(300)
                    .rev()
                    .map(String::as_str)
                    .collect();
                fade(
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .child(
                            div()
                                .id("term-out")
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.term_scroll)
                                .child(
                                    div()
                                        .v_flex()
                                        .font_family(self.mono_font.clone())
                                        .text_xs()
                                        .p_3()
                                        .child(
                                            TextView::markdown(
                                                SharedString::from(format!("term-{id}")),
                                                format!(
                                                    "```text\n{}\n```",
                                                    text.join("\n").replace("```", "'''")
                                                ),
                                            )
                                            .selectable(true),
                                        ),
                                ),
                        )
                        .vertical_scrollbar(&self.term_scroll),
                    SharedString::from(format!("termbody-{id}")),
                    220,
                )
                .into_any_element()
            }
            None => self.terminal_empty(cx),
        };
        let panel = div()
            .h(px(self.term_height()))
            .flex_none()
            .relative()
            .child(tour::anchor(&self.anchors, "terminal"))
            .v_flex()
            .border_t_1()
            .border_color(divider())
            .bg(window_fill(c_term_bg()))
            .child(
                div()
                    .h(px(32.))
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        // Tabs scroll sideways rather than push the buttons out of a narrow panel.
                        div()
                            .id("term-tabs")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_1()
                            .overflow_x_scroll()
                            .when(has_output, |el| {
                                el.child(self.output_tab(shell.is_none(), cx))
                            })
                            .children(shell_tabs)
                            .child(
                                self.ghost_button_sized(
                                    "shell-new",
                                    IconName::Plus,
                                    "New shell",
                                    22.,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.new_shell(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when_some(running_task, |el, sid| {
                                el.child(
                                    self.button("term-stop", "Stop", false)
                                        .h(px(22.))
                                        .px_2()
                                        .text_size(px(12.))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(sh) =
                                                this.shells.iter().find(|s| s.id == sid)
                                            {
                                                sh.write(b"\x03");
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .when(has_commands, |el| {
                                el.child(
                                    div()
                                        .id("term-run")
                                        .h(px(22.))
                                        .px_2()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(c_secondary()))
                                        .hover(|el| {
                                            el.bg(tint(c_active())).text_color(rgb(c_text()))
                                        })
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            self.on_context(ctxmenu::CtxTarget::Run, cx),
                                        )
                                        .child(icon(IconName::Play, 10., c_secondary()))
                                        .child("Run")
                                        .child(icon(IconName::ChevronDown, 10., c_muted())),
                                )
                            })
                            .when(shell.is_none() && has_output, |el| {
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
                            })
                            .child(
                                self.ghost_button_sized(
                                    "term-tall",
                                    IconName::ChevronsUpDown,
                                    if self.term_h < terminal::TERM_TALL - 1. {
                                        "Expand terminal"
                                    } else {
                                        "Shrink terminal"
                                    },
                                    22.,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.toggle_term_size();
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            )
            .child(body)
            .child(
                // Drag the top edge to make the panel taller or shorter.
                div()
                    .id("term-edge")
                    .absolute()
                    .top(px(-2.))
                    .left_0()
                    .right_0()
                    .h(px(5.))
                    .cursor_row_resize()
                    .when(self.term_drag.is_some(), |el| el.bg(rgb(c_blue())))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.term_drag = Some((f32::from(ev.position.y), this.term_h));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        // Expand and shrink ease between the two heights.
        match self.panel_motion("term") {
            Some((tall, at)) => {
                let (from, to) = (self.term_from, self.term_h);
                panel
                    .animated(
                        SharedString::from(format!("term-size-{tall}-{at:?}")),
                        Animation::new(Duration::from_millis(PANEL_MS))
                            .with_easing(ease_in_out_cubic),
                        move |el, d| el.h(px(from + (to - from) * d)),
                    )
                    .into_any_element()
            }
            None => panel.into_any_element(),
        }
    }

    fn right_pane(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(self.panel_width))
            .flex_none()
            .h_full()
            .relative()
            .child(tour::anchor(&self.anchors, "right-pane"))
            .child(
                // Drag the edge to make the panel narrower or wider.
                div()
                    .id("panel-edge")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(px(5.))
                    .cursor_col_resize()
                    .when(self.resizing_panel, |el| el.bg(rgb(c_blue())))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.resizing_panel = true;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .v_flex()
            .border_l_1()
            .border_color(divider())
            .bg(panel(c_sidebar()))
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
                        RightTab::Files => self.files_tab(cx),
                    }),
                SharedString::from(format!(
                    "rtab-{}-{}",
                    self.right_tab as u8,
                    self.selected.unwrap_or(0)
                )),
                260,
            ))
            .child(self.terminal_panel(window, cx))
    }
}

impl RunoApp {
    /// Note when a panel opens or closes, so it can animate.
    fn track_panel(&mut self, key: &'static str, open: bool, snap: bool) {
        let now = std::time::Instant::now();
        match self.panel_anim.get(key) {
            Some((was, _)) if *was == open => {}
            // The first time a panel is seen it just appears, and so does a change made by
            // opening or leaving Settings, where sliding looks awkward.
            _ if snap => {
                self.panel_anim
                    .insert(key, (open, now - Duration::from_secs(10)));
            }
            None => {
                self.panel_anim
                    .insert(key, (open, now - Duration::from_secs(10)));
            }
            Some(_) => {
                self.panel_anim.insert(key, (open, now));
            }
        }
    }

    /// Settings just opened or closed, so views switch without motion.
    pub(crate) fn settings_switching(&self) -> bool {
        self.settings_switch_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(PANEL_MS + 120))
    }

    /// The panel's last change, while it is still animating.
    fn panel_motion(&self, key: &str) -> Option<(bool, std::time::Instant)> {
        self.panel_anim
            .get(key)
            .copied()
            .filter(|(_, at)| anim_on() && at.elapsed() < Duration::from_millis(PANEL_MS + 120))
    }
}

impl Render for RunoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_playbook_run(window, cx);
        self.prepare_playbook_editor(window, cx);
        let st = cx.global::<AppSettings>().clone();
        // Opening or leaving Settings swaps the whole window, so it happens without motion.
        let in_settings = self.view == View::Settings;
        let settings_snap = in_settings != self.was_settings;
        self.was_settings = in_settings;
        if settings_snap {
            self.settings_switch_at = Some(std::time::Instant::now());
        }
        let dark = match st.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        set_dark(dark);
        self.measure_cell(window, cx);
        self.tour_rects = self.anchors.take();
        self.sync_project_form(window, cx);
        self.sync_new_branches();
        if let Some((id, path)) = self.pending_binary.take() {
            settings::update(cx, |s| {
                let mut options = s.agent(id);
                options.binary = path;
                s.agents.insert(id.into(), options);
            });
            // Detect again with the new file, now that it is in the settings.
            caps::refresh();
        }
        // Keep shortcuts working when nothing has focus, such as right after launch.
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        if let Some(path) = self.pending_file.take() {
            self.open_or_ask(&path, window, cx);
        }
        self.restore_open_file(window, cx);
        if !st.browser_enabled {
            self.browser_open = false;
            self.browser_main = false;
        }
        if self.browser_main {
            self.sync_browser_address(window, cx);
        }
        // Design and Architecture follow the project in the top bar.
        match self.view {
            View::Design => self.prepare_design(window, cx),
            View::Architecture => self.prepare_architecture(window, cx),
            _ => {
                if std::mem::take(&mut self.design.shown) {
                    self.watch_design(None, cx);
                }
                self.arch.shown = false;
            }
        }
        // A new shell takes focus once its element has been drawn.
        if let Some(id) = self.selected.filter(|_| self.view == View::Session) {
            let wants = self.state.session(id).is_some_and(|s| s.cli);
            if wants && !self.cli_runs.contains_key(&id) {
                if self.handles.contains_key(&Target::Session(id)) {
                    // A chat turn is running (for example one picked up on a connection
                    // after a restart); two writers would fork the conversation.
                    if let Some(s) = self.state.session_mut(id) {
                        s.cli = false;
                    }
                    self.info = Some((
                        "The agent is still answering in Chat. Switch to the CLI when it's done."
                            .into(),
                        std::time::Instant::now(),
                    ));
                } else {
                    self.open_cli(id, "", cx);
                }
            }
        }
        if self.focus_cli > 0 {
            self.focus_cli -= 1;
            if self.focus_cli == 0 {
                window.focus(&self.cli_focus, cx);
            }
        }
        if self.focus_shell > 0 {
            self.focus_shell -= 1;
            if self.focus_shell == 0 {
                window.focus(&self.term_focus, cx);
            }
            cx.notify();
        }
        // Glass: let the desktop show through, blurred, behind the translucent surfaces.
        let glass = glass_on();
        if self.applied_glass != Some(glass) {
            self.applied_glass = Some(glass);
            // Reload the theme so its code-block shade matches the new glass setting.
            self.applied_dark = None;
            // Blurred maps to vibrancy on macOS, acrylic on Windows 10 and 11, and the
            // compositor's blur on Linux; without a compositor the window stays opaque.
            window.set_background_appearance(if glass {
                WindowBackgroundAppearance::Blurred
            } else {
                WindowBackgroundAppearance::Opaque
            });
        }
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
            if glass {
                gpui_kit::component::Theme::update(cx, |theme| theme.muted = code_wash().into());
            }
            // A theme switch reloads the theme file's fonts; set the UI font again.
            self.applied_font = None;
        }
        let font = ui_font(&st.font);
        if self.applied_font != Some(font) {
            self.applied_font = Some(font);
            gpui_kit::component::Theme::update(cx, |theme| theme.font_family = font.into());
            window.refresh();
        }
        self.apply_pending(window, cx);
        if self.applied_enter != Some(st.send_on_enter) {
            self.applied_enter = Some(st.send_on_enter);
            self.prompt
                .update(cx, |p, cx| p.set_submit_on_enter(st.send_on_enter, cx));
        }
        // New session is a dialog; the view it was opened from stays behind it.
        let shown = if self.view == View::NewSession {
            self.new_back
        } else {
            self.view
        };
        let still = self.settings_switching();
        let center = match shown {
            View::Home => self.home_view(cx),
            View::Design => self.design_view(window, cx),
            View::Architecture => self.architecture_view(cx),
            // The Browser tab takes the work area while it is the tab shown.
            View::Session if self.browser_main && self.current().is_some() => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .v_flex()
                .child(self.browser_tab(cx))
                .into_any_element(),
            View::Session | View::NewSession => self.session_view(window, cx),
            View::Inbox => self.inbox_view(cx),
            View::AllSessions => self.list_view(View::AllSessions, cx),
            View::History => self.history_view(cx),
            View::Worktrees => self.list_view(View::Worktrees, cx),
            View::Settings => self.settings_view(window, cx),
            View::Notes => self.notes_view(cx),
            View::Activity => self.activity_view(cx),
            View::Automations => self.automations_view(cx),
            View::GitHub => self.github_view(cx),
            View::Review => self.review_view(cx),
            View::Editor => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .v_flex()
                .child(self.file_view(true, cx))
                .into_any_element(),
        };
        // The side panel needs room: a narrow window hides it, and widening again brings it
        // back only if it was hidden for that reason, never one the person closed.
        let wide = window.viewport_size().width >= px(1180.);
        if self.panel_wide != Some(wide) {
            self.panel_wide = Some(wide);
            if !wide && self.show_panel {
                self.show_panel = false;
                self.panel_auto_hidden = true;
            } else if wide && self.panel_auto_hidden {
                self.show_panel = true;
                self.panel_auto_hidden = false;
            }
        }
        let show_tabs = self.mode() == Mode::Sessions && !self.tabs.is_empty();
        let show_right = shown == View::Session && self.current().is_some() && self.show_panel;
        let show_left = self.mode() == Mode::Sessions && self.show_sidebar;
        // Moving to another mode swaps the whole page: its panels are simply there and fade
        // in with it. Sliding is kept for opening and closing a panel in place.
        let mode = self.mode();
        let nav_snap = self.shown_mode.replace(mode).is_some_and(|m| m != mode);
        self.track_panel("sidebar", show_left, settings_snap || nav_snap);
        self.track_panel("right", show_right, settings_snap || nav_snap);
        self.track_panel("term", self.term_tall, false);
        let left_motion = self.panel_motion("sidebar");
        let right_motion = self.panel_motion("right");
        let page_fade = |el: AnyElement, id: &str| {
            div().flex_none().h_full().flex().child(el).animated(
                SharedString::from(format!("{id}-{mode:?}")),
                Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                move |el, d| {
                    if still {
                        el
                    } else {
                        el.opacity(0.35 + 0.65 * d)
                    }
                },
            )
        };
        let left = panel_slide(
            "sidebar-slide",
            left_motion,
            show_left,
            self.sidebar_width,
            true,
            || self.sidebar(cx).into_any_element(),
        )
        .map(|el| page_fade(el, "sidebar-page"));
        let right = panel_slide(
            "right-slide",
            right_motion,
            show_right,
            self.panel_width,
            false,
            || self.right_pane(window, cx).into_any_element(),
        )
        .map(|el| page_fade(el, "right-page"));
        if self
            .info
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(3))
        {
            self.info = None;
        }
        self.log_notice();
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
        let (where_name, where_color, _) = self.runs_on();
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
            .on_action(cx.listener(|this, _: &AskRuno, window, cx| {
                if this.palette.is_some() {
                    this.close_palette(window, cx);
                }
                this.toggle_assistant(window, cx);
            }))
            .on_action(cx.listener(|this, _: &QuickOpen, window, cx| {
                this.open_palette(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SaveFile, _, cx| this.save_file(cx)))
            .on_action(cx.listener(|this, _: &ToggleView, _, cx| {
                if let Some(s) = this.current().filter(|_| this.view == View::Session) {
                    let (id, cli) = (s.id, s.cli);
                    this.set_cli_view(id, !cli, cx);
                }
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.tour.is_some() && this.tour_key(&e.keystroke.key, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &GoHome, window, cx| {
                this.go(Mode::Home, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoSessions, window, cx| {
                this.go(Mode::Sessions, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoDesign, window, cx| {
                this.go(Mode::Design, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoArchitecture, window, cx| {
                this.go(Mode::Architecture, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoPlaybooks, window, cx| {
                this.go(Mode::Playbooks, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoReviews, window, cx| {
                this.go(Mode::GitHub, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoReview, window, cx| {
                this.go(Mode::Review, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoHistory, window, cx| {
                this.go(Mode::History, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoNotes, window, cx| {
                this.go(Mode::Notes, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewSession, _, cx| {
                this.open_new_session();
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
                this.toggle_panel();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                // The sidebar belongs to Sessions; elsewhere the shortcut opens Sessions with it.
                if this.mode() != Mode::Sessions {
                    this.show_sidebar = true;
                    this.go(Mode::Sessions, window, cx);
                } else {
                    this.show_sidebar = !this.show_sidebar;
                }
                this.save();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                if this.tour.is_some() {
                    this.end_tour(cx);
                } else if this.remove_copy.is_some() {
                    this.remove_copy = None;
                } else if this.install_confirm.is_some() {
                    this.install_confirm = None;
                } else if this.project_form.is_some() {
                    this.project_form = None;
                } else if this.remove_project.is_some() {
                    this.remove_project = None;
                } else if this.pb_run.is_some() {
                    this.pb_run = None;
                } else if this.pb_install.is_some() {
                    this.pb_install = None;
                } else if this.pb_editor.is_some() {
                    this.pb_editor = None;
                } else if this.ctx_menu.is_some() {
                    this.ctx_menu = None;
                } else if this.switch_plan.is_some() {
                    this.switch_plan = None;
                } else if this.palette.is_some() {
                    this.close_palette(window, cx);
                } else if this.assistant.open {
                    this.close_assistant(window, cx);
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
                } else if this.view == View::NewSession {
                    this.close_new_session();
                } else if this.cursor.is_some() {
                    this.cursor = None;
                } else if this.view == View::Editor {
                    // Closing the file goes back to where it was opened, in the same mode.
                    this.view = this.editor_back;
                }
                // Escape never switches modes.
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowTour, _, cx| {
                this.start_tour();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &GoInbox, window, cx| {
                this.go(Mode::Inbox, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                if let Some(id) = this.selected.filter(|_| this.mode() == Mode::Sessions) {
                    this.close_tab(id);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                this.step_tab(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                this.step_tab(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| {
                this.focus_composer(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ArchiveSession, _, cx| {
                if let Some(id) = this.cursor.or(this.selected) {
                    this.toggle_archive(id);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ContextMenu, window, cx| {
                this.keyboard_context_menu(window);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if this.list_key(&e.keystroke, window, cx) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                if let Some((y0, h0)) = this.term_drag {
                    if e.pressed_button == Some(MouseButton::Left) {
                        // Leave the Changes and Files tabs above it some room.
                        let max = (f32::from(window.viewport_size().height) - 260.)
                            .clamp(terminal::TERM_MIN, terminal::TERM_MAX);
                        this.term_h = (h0 + y0 - f32::from(e.position.y))
                            .clamp(terminal::TERM_MIN, max)
                            .round();
                    } else {
                        this.term_drag = None;
                        this.save();
                    }
                    cx.notify();
                }
                if this.resizing_panel {
                    if e.pressed_button == Some(MouseButton::Left) {
                        let total = f32::from(window.viewport_size().width);
                        let x = total - f32::from(e.position.x);
                        // Leave the chat room for its header and composer.
                        let sidebar = if this.show_sidebar { this.sidebar_width } else { 0. };
                        let max = (total - shell::RAIL_WIDTH - sidebar - CHAT_MIN)
                            .clamp(PANEL_MIN, PANEL_MAX);
                        this.panel_width = x.clamp(PANEL_MIN, max);
                    } else {
                        this.resizing_panel = false;
                        this.save();
                    }
                    cx.notify();
                }
                if this.resizing_sidebar {
                    if e.pressed_button == Some(MouseButton::Left) {
                        let x = f32::from(e.position.x) - shell::RAIL_WIDTH;
                        this.sidebar_width = x.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                    } else {
                        this.resizing_sidebar = false;
                        this.save();
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resizing_sidebar || this.resizing_panel || this.term_drag.is_some() {
                        this.resizing_sidebar = false;
                        this.resizing_panel = false;
                        this.term_drag = None;
                        this.save();
                        cx.notify();
                    }
                }),
            )
            .bg(window_fill(c_canvas()))
            .text_color(rgb(c_text()))
            .text_size(px(13.))
            .font_family(cx.theme().font_family.clone())
            .child(self.top_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.mode_rail(cx))
                    .children(left)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            // Pages with their own list beside the content paint the two
                            // layers themselves, so the list is as clear as the sidebar.
                            .when(!matches!(self.view, View::Review | View::Settings), |el| {
                                el.bg(glass_fill(c_canvas()))
                            })
                            .when(show_tabs, |el| el.child(self.tab_strip(cx)))
                            .child(div().flex_1().min_h_0().flex().child(center))
                            .animated(
                        SharedString::from(format!(
                            "center-{}-{}",
                            shown as u8,
                            self.selected.unwrap_or(0)
                        )),
                        Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                        move |el, delta| {
                            if still {
                                el
                            } else {
                                el.opacity(0.35 + 0.65 * delta).top(px(6. * (1. - delta)))
                            }
                        },
                    ))
                    .children(right),
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
                    .border_color(divider())
                    .bg(pane(c_sidebar()))
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
                                        .hover(|el| el.bg(tint(c_subtle())))
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
                            .children(self.usage_status(cx))
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
                            .when_some(
                                self.current()
                                    .filter(|_| self.view == View::Session)
                                    .and_then(|s| {
                                        s.messages
                                            .iter()
                                            .rev()
                                            .find(|m| !m.usage.is_empty())
                                            .map(|m| m.usage)
                                            .map(|u| {
                                                let cost: f64 = s
                                                    .messages
                                                    .iter()
                                                    .map(|m| m.usage.cost_cents)
                                                    .sum();
                                                let reports = agent::reports_cost(&s.provider);
                                                let about = if reports {
                                                    format!("session cost ${:.2}", cost / 100.0)
                                                } else {
                                                    format!(
                                                        "{} reports no cost, so spending limits don't count it",
                                                        agent::provider(&s.provider).name
                                                    )
                                                };
                                                (u, cost, reports, about)
                                            })
                                    }),
                                |el, (usage, cost, reports, about)| {
                                    let window = if usage.window > 0 { usage.window } else { 200_000 };
                                    let pct = (usage.context() as f32 / window as f32).min(1.0);
                                    el.child(
                                        div()
                                            .id("ctx-meter")
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .tooltip(move |window, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(format!(
                                                    "Context used by the latest turn · {about}"
                                                ))
                                                .build(window, cx)
                                            })
                                            .child(
                                                div()
                                                    .w(px(48.))
                                                    .h(px(4.))
                                                    .rounded_full()
                                                    .bg(tint(c_active()))
                                                    .child(
                                                        div()
                                                            .h_full()
                                                            .rounded_full()
                                                            .w(px(48. * pct))
                                                            .bg(rgb(if pct > 0.8 {
                                                                c_amber()
                                                            } else {
                                                                c_secondary()
                                                            })),
                                                    ),
                                            )
                                            .child(format!("{:.0}%", pct * 100.))
                                            .when(cost > 0.0, |el| {
                                                el.child(format!("· ${:.2}", cost / 100.0))
                                            })
                                            .when(!reports, |el| el.child("· no cost reported")),
                                    )
                                },
                            )
                            .when(!summary.is_empty(), |el| el.child(summary))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(div().size(px(6.)).rounded_full().bg(rgb(where_color)))
                                    .child(where_name),
                            ),
                    ),
            )
            .when(self.view == View::NewSession, |el| {
                el.child(self.new_session_modal(
                    f32::from(window.viewport_size().height) - 96.,
                    cx,
                ))
            })
            .children(self.alerts_view(cx))
            .children(self.assistant_view(cx))
            .children(self.switch_modal(cx))
            .children(palette)
            .children(self.ctx_menu_view(cx))
            .children(self.install_modal(cx))
            .children(self.remove_copy_modal(cx))
            .children(self.project_modal(cx))
            .children(self.remove_project_modal(cx))
            .children(self.delete_session_modal(cx))
            .children(self.playbook_run_form(cx))
            .children(self.playbook_ask_modal(cx))
            .children(self.tour_view(window, cx))
            .children(self.browser_offer(cx))
            .children(self.whats_new_view(cx));
        fade(root, "app-root", 450)
    }
}

/// The UI typeface, bundled so the app looks the same on every platform.
/// Turn macOS font smoothing on or off for Runo. The renderer reads Runo's own
/// `AppleFontSmoothing` preference once, the first time it draws text; with smoothing on,
/// CoreGraphics thickens every glyph whose colour isn't near black, which makes grey
/// and light text look bold. Off draws text with plain antialiasing, crisp and light.
#[cfg(target_os = "macos")]
fn set_font_smoothing(on: bool) {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFPreferencesCurrentApplication: Ref;
        fn CFStringCreateWithCString(alloc: Ref, s: *const std::ffi::c_char, encoding: u32) -> Ref;
        fn CFNumberCreate(alloc: Ref, kind: isize, value: *const c_void) -> Ref;
        fn CFPreferencesSetAppValue(key: Ref, value: Ref, app: Ref);
        fn CFRelease(r: Ref);
    }
    const UTF8: u32 = 0x0800_0100;
    const SINT32: isize = 3;
    let level: i32 = if on { 2 } else { 0 };
    unsafe {
        let key = CFStringCreateWithCString(std::ptr::null(), c"AppleFontSmoothing".as_ptr(), UTF8);
        let value = CFNumberCreate(std::ptr::null(), SINT32, (&level as *const i32).cast());
        if !key.is_null() && !value.is_null() {
            CFPreferencesSetAppValue(key, value, kCFPreferencesCurrentApplication);
        }
        for r in [key, value] {
            if !r.is_null() {
                CFRelease(r);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_font_smoothing(_on: bool) {}

/// Show Runo's icon in the Dock when it runs outside an app bundle (`cargo run`), where
/// macOS would otherwise show a generic executable icon. A bundled Runo.app already has it.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Id;
        fn objc_msgSend();
    }
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}
    static PNG: &[u8] = include_bytes!("../assets/icon/png/runo-512.png");
    unsafe {
        let send0: unsafe extern "C" fn(Id, Id) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        let send1: unsafe extern "C" fn(Id, Id, Id) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        let send_bytes: unsafe extern "C" fn(Id, Id, *const c_void, usize) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        let data = send_bytes(
            objc_getClass(c"NSData".as_ptr()),
            sel_registerName(c"dataWithBytes:length:".as_ptr()),
            PNG.as_ptr().cast(),
            PNG.len(),
        );
        if data.is_null() {
            return;
        }
        let image = send0(
            objc_getClass(c"NSImage".as_ptr()),
            sel_registerName(c"alloc".as_ptr()),
        );
        let image = send1(image, sel_registerName(c"initWithData:".as_ptr()), data);
        if image.is_null() {
            return;
        }
        let app = send0(
            objc_getClass(c"NSApplication".as_ptr()),
            sel_registerName(c"sharedApplication".as_ptr()),
        );
        send1(
            app,
            sel_registerName(c"setApplicationIconImage:".as_ptr()),
            image,
        );
        send0(image, sel_registerName(c"release".as_ptr()));
    }
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}

/// Runo's own interface font, bundled with the app: Geist, which stays crisp at small
/// sizes on non-Retina displays.
const BUNDLED_FONT: &str = "Geist";

/// The interface font for the "Interface font" setting. "system" is the platform's UI
/// font where it can be named; elsewhere Runo's own font.
fn ui_font(setting: &str) -> &'static str {
    match setting {
        "system" if cfg!(target_os = "macos") => ".SystemUIFont",
        "system" if cfg!(windows) => "Segoe UI",
        _ => BUNDLED_FONT,
    }
}

fn load_fonts(cx: &mut App) {
    let fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Regular.otf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.otf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Geist-SemiBold.otf")),
        std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Bold.otf")),
    ];
    let _ = cx.text_system().add_fonts(fonts);
}

const SIDEBAR_WIDTH: f32 = 264.;
/// How narrow and wide the sidebar can be dragged.
const SIDEBAR_MIN: f32 = 200.;
const SIDEBAR_MAX: f32 = 400.;
const PANEL_WIDTH: f32 = 460.;
const PANEL_MIN: f32 = 320.;
const PANEL_MAX: f32 = 900.;
/// Narrowest the chat gets when the right panel is dragged wider.
const CHAT_MIN: f32 = 600.;

fn main() {
    // `runo --mcp-memory <folder>`: serve the memory tools to an agent, without a window.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--mcp-memory") {
        memory_mcp::serve(args.get(2).map(String::as_str).unwrap_or("."));
        return;
    }
    // `runo --phone [address]`: the API Runo's mobile app talks to, without a window.
    if args.get(1).map(String::as_str) == Some("--phone") {
        std::process::exit(phone::serve(args.get(2).map(String::as_str)));
    }
    // `runo --run-due-playbooks`: scheduled playbooks while Runo is closed (launchd).
    if args.get(1).map(String::as_str) == Some("--run-due-playbooks") {
        shellenv::init();
        std::process::exit(pbrunner::run_due());
    }
    // `runo --connection-run <file>`: one agent turn on a remote connection, and
    // `--connection-shell <file>`: a terminal there.
    if let Some(mode @ ("--connection-run" | "--connection-shell")) =
        args.get(1).map(String::as_str)
    {
        shellenv::init();
        let file = args.get(2).map(String::as_str).unwrap_or("");
        std::process::exit(if mode == "--connection-run" {
            connections::connection_run(file)
        } else {
            connections::connection_shell(file)
        });
    }
    // Before any thread starts: find agents and Node.js where the person's terminal would.
    shellenv::init();
    applog::init();
    detach::enable(store::data_dir());
    let app = application().with_assets(assets::AllAssets);
    // Clicking the Dock icon with the window closed (macOS keeps Runo running) opens it again.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_main_window(cx);
        }
    });
    app.run(|cx| {
        init(cx);
        load_fonts(cx);
        let settings = AppSettings::load();
        settings.apply();
        // Before any text is drawn: the renderer reads this once.
        set_font_smoothing(settings.font_smoothing);
        set_dock_icon();
        cx.set_global(settings);
        bind_keys(cx);
        cx.on_action(quit);
        desktop_notify::install(cx);
        cx.set_menus(app_menus());
        open_main_window(cx);
    });
}

/// The open main window's app, so quitting can warn about running agents first.
struct MainApp(WeakEntity<RunoApp>);

impl Global for MainApp {}

/// Quit from the shortcut, the menu or the command palette. With agents running in the
/// window, this warns first, the same as closing the window.
fn quit(_: &Quit, cx: &mut App) {
    let app = cx.try_global::<MainApp>().and_then(|m| m.0.upgrade());
    let go = app.is_none_or(|app| {
        app.update(cx, |this, cx| {
            this.save();
            this.may_close(cx)
        })
    });
    if go {
        cx.quit();
    }
}

/// A clicked notification: bring the window forward on its session.
fn open_session_from_notification(id: u64, cx: &mut App) {
    let Some(app) = cx.try_global::<MainApp>().and_then(|m| m.0.upgrade()) else {
        open_main_window(cx);
        return;
    };
    app.update(cx, |this, cx| {
        if this.state.session(id).is_some() {
            this.select(id);
            cx.notify();
        }
    });
    for window in cx.windows() {
        let _ = window.update(cx, |_, window, _| window.activate_window());
    }
}

/// The menu bar: the app menu with Settings and Quit, and the main commands with their
/// shortcuts.
fn app_menus() -> Vec<Menu> {
    vec![
        Menu::new("Runo").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Take the tour", ShowTour),
            MenuItem::separator(),
            MenuItem::action("Quit Runo", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New session", NewSession),
            MenuItem::action("Quick open…", QuickOpen),
            MenuItem::separator(),
            MenuItem::action("Save file", SaveFile),
            MenuItem::action("Close tab", CloseTab),
            MenuItem::action("Archive session", ArchiveSession),
        ]),
        Menu::new("View").items([
            MenuItem::action("Home", GoHome),
            MenuItem::action("Sessions", GoSessions),
            MenuItem::action("Review", GoReview),
            MenuItem::action("GitHub", GoReviews),
            MenuItem::action("Playbooks", GoPlaybooks),
            MenuItem::action("History", GoHistory),
            MenuItem::action("Notes", GoNotes),
            MenuItem::action("Inbox", GoInbox),
            MenuItem::separator(),
            MenuItem::action("Toggle sidebar", ToggleSidebar),
            MenuItem::action("Toggle Changes panel", TogglePanel),
            MenuItem::action("Chat or CLI", ToggleView),
        ]),
        Menu::new("Session").items([
            MenuItem::action("Next tab", NextTab),
            MenuItem::action("Previous tab", PrevTab),
            MenuItem::action("Focus the message box", FocusComposer),
            MenuItem::action("Stop agent", StopAgent),
        ]),
    ]
}

/// Open Runo's window where it was last time, or centred on first launch.
fn open_main_window(cx: &mut App) {
    let glass = settings::glass_alpha(&cx.global::<AppSettings>().glass) < 255;
    let saved = store::saved_window().and_then(|w| {
        let bounds = Bounds::new(point(px(w.x), px(w.y)), size(px(w.width), px(w.height)));
        // A window on a display that is gone opens centred instead.
        cx.displays()
            .iter()
            .any(|d| d.bounds().intersects(&bounds))
            .then_some(if w.maximized {
                WindowBounds::Maximized(bounds)
            } else {
                WindowBounds::Windowed(bounds)
            })
    });
    let options = WindowOptions {
        // Transparency has to be asked for when the window is created on some platforms.
        window_background: if glass {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        },
        window_bounds: Some(saved.unwrap_or_else(|| {
            WindowBounds::Windowed(Bounds::centered(None, size(px(1560.), px(960.)), cx))
        })),
        titlebar: Some(TitlebarOptions {
            title: Some("Runo".into()),
            appears_transparent: cfg!(target_os = "macos"),
            traffic_light_position: Some(point(px(16.), px(16.))),
        }),
        // Matches StartupWMClass in assets/linux/runo.desktop, so docks show the icon.
        app_id: Some("runo".into()),
        ..Default::default()
    };
    let (handle, _) = open_window(options, cx, |window, cx| {
        cx.new(|cx| RunoApp::new(window, cx))
    })
    .expect("failed to open Runo window");
    // The component library's root paints the theme's opaque background over the
    // whole window, which hides the glass. Runo paints its own surface, so clear it.
    let _ = handle.update(cx, |_, window, cx| {
        gpui_kit::base::Root::update(window, cx, |root, _, cx| {
            root.style().background = Some(gpui_kit::transparent_black().into());
            cx.notify();
        })
    });
}
