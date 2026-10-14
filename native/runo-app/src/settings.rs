//! User settings (persisted), the live colour palette, and discovery of skills and MCP servers.

use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

/// Per-agent customisation. Fields that a given CLI has no equivalent for are ignored.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentOptions {
    /// Run this executable instead of the one found on PATH.
    pub binary: String,
    /// Model used when a session has none selected (any name the CLI accepts).
    pub model_override: String,
    /// Extra arguments, shell-style quoting.
    pub extra_args: String,
    /// One `KEY=VALUE` per line.
    pub env: String,
    /// Claude: low…max. Codex: model_reasoning_effort.
    pub effort: String,
    /// Claude: text appended to the system prompt.
    pub append_prompt: String,
    /// Claude: comma-separated tool rules.
    pub allowed_tools: String,
    pub disallowed_tools: String,
    /// Claude and Codex: comma-separated extra directories the agent may access.
    pub add_dirs: String,
    pub fallback_model: String,
    /// Codex config profile.
    pub profile: String,
    /// OpenCode: model variant (reasoning effort).
    pub variant: String,
    /// OpenCode: named agent.
    pub sub_agent: String,
    /// OpenCode: show thinking blocks.
    pub thinking: bool,
    /// Codex: also write the default model into ~/.codex/config.toml (after a backup).
    pub save_to_config: bool,
    /// Default models of single projects, by project id; they win over `model_override`.
    pub project_models: HashMap<String, String>,
}

impl AgentOptions {
    /// These options with the project's own default model, when it has one.
    pub fn for_project(&self, project: u64) -> AgentOptions {
        let mut opts = self.clone();
        if let Some(model) = self
            .project_models
            .get(&project.to_string())
            .filter(|m| !m.trim().is_empty())
        {
            opts.model_override = model.trim().to_string();
        }
        opts
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub agents: HashMap<String, AgentOptions>,
    pub theme: String,
    pub accent: String,
    pub diff_colors: String,
    pub animations: bool,
    /// Saved under a new name so the old default (off) no longer applies.
    #[serde(rename = "desktop_notifications")]
    pub notifications: bool,
    pub confirm_delete: bool,
    pub remove_worktree_on_delete: bool,
    pub send_on_enter: bool,
    pub autoscroll: bool,
    pub default_mode: String,
    pub default_provider: String,
    /// Shell command run in every new worktree, e.g. `npm install`.
    pub worktree_setup: String,
    /// Stop the computer from sleeping while an agent runs.
    pub keep_awake: bool,
    /// Add a line crediting the agent to commits made from Runo.
    pub attribution: bool,
    /// Saved shell commands shown as buttons over the terminal, one `Name = command` per line.
    pub quick_commands: String,
    /// Group the sidebar by status (Needs you, Running, Ready to review, Done) instead of project.
    pub group_by_status: bool,
    /// Translucent window over a blurred desktop: "off", "subtle", "frosted" or "clear".
    pub glass: String,
    /// Cards, menus and dialogs are translucent too.
    pub glass_cards: bool,
    /// Interface font: "bundled" (Runo's own, tuned for sharp small text) or "system".
    pub font: String,
    /// macOS font smoothing, which thickens text strokes. Off keeps text crisp and light.
    pub font_smoothing: bool,
    /// Pass Runo's memory MCP server to agents that accept one per run.
    pub memory_server: bool,
    /// The first-run tour was finished or skipped.
    pub tour_seen: bool,
    /// The "What's new" card about the Workbench layout was read.
    pub whats_new_seen: bool,
    /// How new sessions open: "chat" or "cli" (the agent's own CLI, where supported).
    pub default_view: String,
    /// How wide the conversation runs: "comfortable", "wide" or "full".
    pub content_width: String,
    /// Terminal text size in points.
    pub term_font_size: u32,
    /// Terminal font family; empty uses the theme's monospace font.
    pub term_font: String,
    /// Mouse-wheel speed in the terminal: "normal", "fast" or "faster".
    pub term_scroll: String,
    /// Right-click in the terminal: "menu" or "paste".
    pub term_right_click: String,
    /// Lines a terminal keeps above the screen, for new shells.
    pub term_history: u32,
    /// MCP servers (`<app>/<name>`) handed to every agent that doesn't load them itself.
    pub mcp_shared: Vec<String>,
    /// Show pages in the Browser tab with a headless Chromium.
    pub browser_enabled: bool,
    /// The built-in browser card was answered.
    pub browser_asked: bool,
    /// Shortcuts changed in Settings > Keybindings: shortcut id → keys ("cmd-shift-k").
    pub keybindings: HashMap<String, String>,
    /// Places sessions can run besides this machine (Cove servers, SSH servers).
    pub connections: Vec<crate::connections::Connection>,
    /// Back up each remote session checkpoint to the project's GitHub remote.
    pub sync_backup: bool,
    /// Include untracked files in those backups.
    pub sync_backup_untracked: bool,
    /// End a turn on a connection at the first command it isn't allowed to run.
    pub stop_on_denial: bool,
    /// Minutes an agent waits for an answer when it asks for permission mid-turn.
    pub approval_wait_mins: u32,
    /// Runs of each scheduled playbook to keep; 0 keeps them all.
    pub playbook_keep_runs: u32,
    /// Run a scheduled playbook once on launch when it came due while Runo was closed.
    pub playbook_catch_up: bool,
    /// Run scheduled playbooks while Runo is closed, from a launchd agent (macOS).
    pub playbooks_while_closed: bool,
    /// Ask before a playbook an agent asked for starts.
    pub confirm_agent_playbooks: bool,
    /// Give agents Cove's own tools (`cove mcp`): scratch VMs, commands, files, ports.
    pub cove_tools: bool,
    /// Minutes without output before a running agent is flagged as maybe stuck; 0 never.
    pub stuck_after_mins: u32,
    /// Spending limits in US dollars, per session and per day; 0 for none.
    pub session_limit_usd: u32,
    pub daily_limit_usd: u32,
    /// Agents that may run at the same time on this machine; 0 for no cap.
    pub max_running: u32,
    /// Look for a newer Runo on GitHub now and then.
    pub check_runo_updates: bool,
}

/// `Name = command` lines; a line without `=` uses the command as its name.
pub fn parse_quick_commands(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once('=') {
            Some((name, cmd)) if !name.trim().is_empty() && !cmd.trim().is_empty() => {
                (name.trim().to_string(), cmd.trim().to_string())
            }
            _ => (l.to_string(), l.to_string()),
        })
        .collect()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            agents: HashMap::new(),
            theme: "dark".into(),
            accent: "indigo".into(),
            diff_colors: "default".into(),
            animations: true,
            notifications: true,
            confirm_delete: true,
            remove_worktree_on_delete: true,
            send_on_enter: true,
            autoscroll: true,
            default_mode: "edit".into(),
            default_provider: "claude".into(),
            worktree_setup: String::new(),
            keep_awake: true,
            attribution: false,
            quick_commands: "Test = cargo test\nBuild = cargo build\nStatus = git status --short"
                .into(),
            group_by_status: false,
            glass: "off".into(),
            glass_cards: true,
            font: "bundled".into(),
            font_smoothing: false,
            memory_server: true,
            tour_seen: false,
            whats_new_seen: false,
            default_view: "chat".into(),
            content_width: "full".into(),
            term_font_size: 12,
            term_font: String::new(),
            term_scroll: "normal".into(),
            term_right_click: "menu".into(),
            term_history: 10_000,
            mcp_shared: Vec::new(),
            browser_enabled: false,
            browser_asked: false,
            keybindings: HashMap::new(),
            connections: Vec::new(),
            sync_backup: false,
            sync_backup_untracked: false,
            stop_on_denial: true,
            approval_wait_mins: 5,
            playbook_keep_runs: 10,
            playbook_catch_up: true,
            playbooks_while_closed: false,
            confirm_agent_playbooks: true,
            cove_tools: true,
            stuck_after_mins: 10,
            session_limit_usd: 0,
            daily_limit_usd: 0,
            max_running: 0,
            check_runo_updates: true,
        }
    }
}

impl Global for AppSettings {}

pub const ACCENTS: [(&str, &str, u32); 7] = [
    ("indigo", "Indigo (default)", 0x3b5bdb),
    ("blue", "Blue", 0x2f80ed),
    ("purple", "Purple", 0x8b5cf6),
    ("pink", "Pink", 0xe0488b),
    ("red", "Red", 0xe5484d),
    ("orange", "Orange", 0xe08a1e),
    ("green", "Green", 0x1f9d63),
];

fn settings_path() -> PathBuf {
    crate::store::data_dir().join("settings.json")
}

impl AppSettings {
    pub fn agent(&self, id: &str) -> AgentOptions {
        self.agents.get(id).cloned().unwrap_or_default()
    }

    /// The worktree setup command for a project: its own, else the one in Settings.
    pub fn setup_for(&self, project_setup: &str) -> String {
        let own = project_setup.trim();
        if own.is_empty() {
            self.worktree_setup.trim().to_string()
        } else {
            own.to_string()
        }
    }

    pub fn load() -> Self {
        crate::store::read_json_or_backup(&settings_path())
    }

    pub fn save(&self) {
        crate::store::write_json(&settings_path(), self);
    }

    /// Push the values that hot paths read (colours, animation switch…) into statics.
    pub fn apply(&self) {
        crate::caps::set_binaries(
            self.agents
                .iter()
                .map(|(id, a)| (id.clone(), a.binary.clone()))
                .collect(),
        );
        let accent = ACCENTS
            .iter()
            .find(|(id, _, _)| *id == self.accent)
            .map(|a| a.2)
            .unwrap_or(ACCENTS[0].2);
        ACCENT.store(accent, Ordering::Relaxed);
        DIFF.store(
            match self.diff_colors.as_str() {
                "colorblind" => 1,
                "contrast" => 2,
                _ => 0,
            },
            Ordering::Relaxed,
        );
        ANIMATIONS.store(self.animations, Ordering::Relaxed);
        GLASS.store(glass_alpha(&self.glass), Ordering::Relaxed);
        GLASS_CARDS.store(self.glass_cards, Ordering::Relaxed);
        MEMORY_SERVER.store(self.memory_server, Ordering::Relaxed);
        NOTIFY.store(self.notifications, Ordering::Relaxed);
        CONFIRM_DELETE.store(self.confirm_delete, Ordering::Relaxed);
        REMOVE_WORKTREE.store(self.remove_worktree_on_delete, Ordering::Relaxed);
        AUTOSCROLL.store(self.autoscroll, Ordering::Relaxed);
        KEEP_AWAKE.store(self.keep_awake, Ordering::Relaxed);
        crate::mcp::set_shared(&self.mcp_shared);
        crate::connections::set_all(&self.connections);
        COVE_TOOLS.store(self.cove_tools, Ordering::Relaxed);
        SYNC_BACKUP.store(self.sync_backup, Ordering::Relaxed);
        SYNC_BACKUP_UNTRACKED.store(self.sync_backup_untracked, Ordering::Relaxed);
        STOP_ON_DENIAL.store(self.stop_on_denial, Ordering::Relaxed);
        KEEP_RUNS.store(self.playbook_keep_runs, Ordering::Relaxed);
        STUCK_AFTER.store(self.stuck_after_mins, Ordering::Relaxed);
        SESSION_LIMIT.store(self.session_limit_usd, Ordering::Relaxed);
        DAILY_LIMIT.store(self.daily_limit_usd, Ordering::Relaxed);
        MAX_RUNNING.store(self.max_running, Ordering::Relaxed);
        CATCH_UP.store(self.playbook_catch_up, Ordering::Relaxed);
        crate::pbrunner::sync_launch_agent(self.playbooks_while_closed);
        crate::terminal::configure(
            self.term_font_size,
            match self.term_scroll.as_str() {
                "fast" => 2,
                "faster" => 4,
                _ => 1,
            },
            self.term_right_click == "paste",
        );
        crate::pty::set_scrollback(self.term_history as usize);
    }

    /// The widest the conversation column gets, or None to fill the window.
    pub fn content_max_width(&self) -> Option<f32> {
        match self.content_width.as_str() {
            "comfortable" => Some(780.),
            "wide" => Some(1120.),
            _ => None,
        }
    }
}

/// Apply a change to the global settings, persist it and repaint.
pub fn update(cx: &mut App, f: impl FnOnce(&mut AppSettings)) {
    let mut settings = cx.global::<AppSettings>().clone();
    f(&mut settings);
    settings.apply();
    settings.save();
    cx.set_global(settings);
    cx.refresh_windows();
}

// ---------- live palette ----------

static DARK: AtomicBool = AtomicBool::new(false);
/// Opacity of the window background in glass mode (255 when glass is off).
static GLASS: AtomicU8 = AtomicU8::new(255);
static GLASS_CARDS: AtomicBool = AtomicBool::new(true);
static MEMORY_SERVER: AtomicBool = AtomicBool::new(true);
static ACCENT: AtomicU32 = AtomicU32::new(0x3b5bdb);
static DIFF: AtomicU8 = AtomicU8::new(0);
static ANIMATIONS: AtomicBool = AtomicBool::new(true);
static NOTIFY: AtomicBool = AtomicBool::new(false);
static CONFIRM_DELETE: AtomicBool = AtomicBool::new(true);
static REMOVE_WORKTREE: AtomicBool = AtomicBool::new(true);
static AUTOSCROLL: AtomicBool = AtomicBool::new(true);
static KEEP_AWAKE: AtomicBool = AtomicBool::new(true);
static COVE_TOOLS: AtomicBool = AtomicBool::new(true);
static SYNC_BACKUP: AtomicBool = AtomicBool::new(false);
static SYNC_BACKUP_UNTRACKED: AtomicBool = AtomicBool::new(false);
static STOP_ON_DENIAL: AtomicBool = AtomicBool::new(true);
static KEEP_RUNS: AtomicU32 = AtomicU32::new(10);
static STUCK_AFTER: AtomicU32 = AtomicU32::new(10);
static SESSION_LIMIT: AtomicU32 = AtomicU32::new(0);
static DAILY_LIMIT: AtomicU32 = AtomicU32::new(0);
static MAX_RUNNING: AtomicU32 = AtomicU32::new(0);

/// Minutes of silence before an agent is flagged as maybe stuck; 0 never.
pub fn stuck_after_mins() -> u32 {
    STUCK_AFTER.load(Ordering::Relaxed)
}

/// Spending limits in US cents (session, day); 0 for none.
pub fn spend_limits() -> (f64, f64) {
    (
        SESSION_LIMIT.load(Ordering::Relaxed) as f64 * 100.,
        DAILY_LIMIT.load(Ordering::Relaxed) as f64 * 100.,
    )
}

/// Agents allowed to run at once; 0 for no cap.
pub fn max_running() -> usize {
    MAX_RUNNING.load(Ordering::Relaxed) as usize
}
static CATCH_UP: AtomicBool = AtomicBool::new(true);

pub fn cove_tools() -> bool {
    COVE_TOOLS.load(Ordering::Relaxed)
}

pub fn sync_backup() -> bool {
    SYNC_BACKUP.load(Ordering::Relaxed)
}

pub fn sync_backup_untracked() -> bool {
    SYNC_BACKUP_UNTRACKED.load(Ordering::Relaxed)
}

pub fn playbook_keep_runs() -> usize {
    KEEP_RUNS.load(Ordering::Relaxed) as usize
}

pub fn playbook_catch_up() -> bool {
    CATCH_UP.load(Ordering::Relaxed)
}

pub fn stop_on_denial() -> bool {
    STOP_ON_DENIAL.load(Ordering::Relaxed)
}

pub fn keep_awake() -> bool {
    KEEP_AWAKE.load(Ordering::Relaxed)
}

pub fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
}
pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}
pub fn anim_on() -> bool {
    ANIMATIONS.load(Ordering::Relaxed)
}
pub fn notify_on() -> bool {
    NOTIFY.load(Ordering::Relaxed)
}
pub fn confirm_delete() -> bool {
    CONFIRM_DELETE.load(Ordering::Relaxed)
}
pub fn remove_worktree() -> bool {
    REMOVE_WORKTREE.load(Ordering::Relaxed)
}
pub fn autoscroll() -> bool {
    AUTOSCROLL.load(Ordering::Relaxed)
}

/// Opacity of the content surface for each glass style. Sidebars and panels are
/// derived from it, so each style keeps the same layering.
pub fn glass_alpha(style: &str) -> u8 {
    match style {
        "subtle" => 248,
        "frosted" => 236,
        "clear" => 216,
        _ => 255,
    }
}

pub fn memory_server() -> bool {
    MEMORY_SERVER.load(Ordering::Relaxed)
}

pub fn glass_on() -> bool {
    GLASS.load(Ordering::Relaxed) < 255
}

fn with_alpha(color: u32, alpha: u8) -> Rgba {
    rgba((color << 8) | alpha as u32)
}

/// The content surface: the most solid layer in glass mode, so text stays crisp.
pub fn glass_fill(color: u32) -> Rgba {
    with_alpha(color, GLASS.load(Ordering::Relaxed))
}

/// Opacity of the sidebar layer for the current glass style: the most see-through,
/// like the sidebar material in macOS apps. Dark surfaces need more body, or light
/// text over a thin layer fades into the desktop behind it.
fn sidebar_alpha() -> u8 {
    let (light, dark) = match GLASS.load(Ordering::Relaxed) {
        255 => return 255,
        a if a >= 248 => (204, 232),
        a if a >= 236 => (164, 212),
        _ => (124, 188),
    };
    if is_dark() {
        dark
    } else {
        light
    }
}

/// Sidebars, title and status bars: the most translucent layer in glass mode.
pub fn pane(color: u32) -> Rgba {
    if glass_on() {
        with_alpha(color, sidebar_alpha())
    } else {
        rgb(color)
    }
}

/// Side panels next to the content, such as changes, files and the terminal: between
/// the sidebar and the content.
pub fn panel(color: u32) -> Rgba {
    if glass_on() {
        let (side, content) = (sidebar_alpha() as u16, GLASS.load(Ordering::Relaxed) as u16);
        with_alpha(color, ((side + content * 2) / 3) as u8)
    } else {
        rgb(color)
    }
}

/// The window behind every surface: clear in glass mode so the desktop shows through.
pub fn window_fill(color: u32) -> Rgba {
    if glass_on() {
        rgba(0)
    } else {
        rgb(color)
    }
}

/// Dividers between the window's regions: a faint hairline in glass mode instead of a
/// solid grey line, which looks heavy over the blur.
pub fn divider() -> Rgba {
    if !glass_on() {
        rgb(c_border())
    } else if is_dark() {
        rgba(0xffffff1a)
    } else {
        rgba(0x0000001a)
    }
}

/// Hover, selection and other row fills. In glass mode they are a see-through shade of
/// the surface instead of a solid colour: a solid fill on a translucent layer gets a
/// ragged, mis-shaded rim where its rounded edges are antialiased.
pub fn tint(color: u32) -> Rgba {
    if !glass_on() {
        return rgb(color);
    }
    let lum = |c: u32| (((c >> 16) & 0xff) + ((c >> 8) & 0xff) + (c & 0xff)) as f32 / 3.0;
    let (base, fill) = (lum(c_canvas()), lum(color));
    // The desktop behind already shades a translucent layer, so a lighter touch reads
    // the same as the solid fill does on an opaque window.
    let alpha = |a: f32| (a.clamp(0.0, 1.0) * 0.6 * 255.0).round() as u32;
    if fill <= base {
        rgba(alpha((base - fill) / base.max(1.0)))
    } else {
        rgba(0xffffff00 | alpha((fill - base) / (255.0 - base).max(1.0)))
    }
}

/// Dialogs, menus and popups floating over the app. Only the window itself is blurred,
/// so these stay nearly solid: text behind a thin card would show through unblurred.
pub fn float(color: u32) -> Rgba {
    if glass_on() && GLASS_CARDS.load(Ordering::Relaxed) {
        with_alpha(color, 246)
    } else {
        rgb(color)
    }
}

/// The edge of a floating surface: a soft light rim in glass mode, like frosted glass.
pub fn float_edge() -> Rgba {
    if glass_on() && GLASS_CARDS.load(Ordering::Relaxed) {
        if is_dark() {
            rgba(0xffffff26)
        } else {
            rgba(0xffffffb3)
        }
    } else {
        rgb(c_border())
    }
}

fn pick(light: u32, dark: u32) -> u32 {
    if is_dark() {
        dark
    } else {
        light
    }
}

pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x * t + y * (1.0 - t)).round() as u32) & 0xff
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

// A neutral zinc scale: colour comes from the accent and status hues only.
pub fn c_canvas() -> u32 {
    pick(0xffffff, 0x18181b)
}
pub fn c_sidebar() -> u32 {
    pick(0xf7f7f8, 0x121214)
}
pub fn c_subtle() -> u32 {
    pick(0xf1f1f3, 0x222226)
}
pub fn c_border() -> u32 {
    pick(0xe5e5e8, 0x2a2a2f)
}
pub fn c_text() -> u32 {
    pick(0x18181b, 0xe9e9ec)
}
/// Secondary and muted text are a step stronger in glass mode: the translucent layers
/// let the desktop through under the text too, which would otherwise wash it out.
pub fn c_secondary() -> u32 {
    if glass_on() {
        pick(0x45454d, 0xbdbdc6)
    } else {
        pick(0x55555e, 0xa1a1aa)
    }
}
pub fn c_muted() -> u32 {
    if glass_on() {
        pick(0x7c7c86, 0x93939d)
    } else {
        pick(0x9b9ba4, 0x6e6e78)
    }
}
pub fn c_term_bg() -> u32 {
    pick(0xfafafa, 0x111113)
}
/// Neutral fill for the selected row, so selection does not compete with status colours.
pub fn c_active() -> u32 {
    pick(0xe9e9ec, 0x27272c)
}
/// Raised surfaces such as the composer, menus and cards.
pub fn c_raised() -> u32 {
    pick(0xffffff, 0x1f1f23)
}
pub fn c_blue() -> u32 {
    let accent = ACCENT.load(Ordering::Relaxed);
    if is_dark() {
        mix(accent, 0xffffff, 0.8)
    } else {
        accent
    }
}
/// Background of a selected row or option: neutral, so the accent stays for text and focus.
pub fn c_selected() -> u32 {
    c_active()
}
pub fn c_green() -> u32 {
    pick(0x16a34a, 0x4ade80)
}
pub fn c_amber() -> u32 {
    pick(0xf0a020, 0xf3b23f)
}
pub fn c_red() -> u32 {
    pick(0xdc2626, 0xf87171)
}

/// Text colour for added / removed lines, honouring the diff palette.
pub fn c_add() -> u32 {
    match DIFF.load(Ordering::Relaxed) {
        1 => pick(0x1f6feb, 0x58a6ff),
        2 => pick(0x0a6b2d, 0x5fe08a),
        _ => c_green(),
    }
}
pub fn c_del() -> u32 {
    match DIFF.load(Ordering::Relaxed) {
        1 => pick(0xd9730d, 0xf0883e),
        2 => pick(0xa40e1a, 0xff7b72),
        _ => c_red(),
    }
}
pub fn c_add_bg() -> u32 {
    let strength = if DIFF.load(Ordering::Relaxed) == 2 {
        0.28
    } else {
        0.14
    };
    mix(c_add(), c_canvas(), strength)
}
pub fn c_del_bg() -> u32 {
    let strength = if DIFF.load(Ordering::Relaxed) == 2 {
        0.28
    } else {
        0.14
    };
    mix(c_del(), c_canvas(), strength)
}

// ---------- desktop notification ----------

pub fn notify(title: &str, body: &str) {
    if !notify_on() {
        return;
    }
    let (title, body) = (title.replace('"', "'"), body.replace('"', "'"));
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!("display notification \"{body}\" with title \"{title}\""),
        ])
        .spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("notify-send")
        .args([&title, &body])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = (title, body);
}

// ---------- discovery ----------

#[derive(Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub source: &'static str,
}

#[derive(Clone)]
pub struct McpInfo {
    pub name: String,
    pub provider: &'static str,
    pub transport: &'static str,
    pub enabled: bool,
    pub detail: String,
    pub path: PathBuf,
    /// How to start or reach it, for sharing with other agents.
    pub spec: crate::mcp::McpSpec,
}

#[derive(Clone, Default)]
pub struct Discovered {
    pub skills: Vec<SkillInfo>,
    pub mcp: Vec<McpInfo>,
    pub hooks: Vec<HookInfo>,
    /// Config files worth editing, as (label, path); they may not exist yet.
    pub files: Vec<(&'static str, PathBuf)>,
    pub provider_paths: Vec<(&'static str, Option<PathBuf>)>,
}

#[derive(Clone)]
pub struct HookInfo {
    pub event: String,
    pub commands: Vec<String>,
    pub scope: &'static str,
    pub path: PathBuf,
}

/// Claude Code hooks: `{"hooks": {"Event": [{"matcher": .., "hooks": [{"command": ..}]}]}}`.
fn scan_hooks(path: &Path, scope: &'static str, out: &mut Vec<HookInfo>) {
    let Some(value) = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        return;
    };
    let Some(events) = value.get("hooks").and_then(|v| v.as_object()) else {
        return;
    };
    for (event, groups) in events {
        let commands = groups
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|g| {
                let matcher = g.get("matcher").and_then(|m| m.as_str()).unwrap_or("");
                g.get("hooks")
                    .and_then(|h| h.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(move |h| {
                        let cmd = h.get("command").and_then(|c| c.as_str())?;
                        Some(if matcher.is_empty() || matcher == "*" {
                            cmd.to_string()
                        } else {
                            format!("{matcher}: {cmd}")
                        })
                    })
            })
            .collect();
        out.push(HookInfo {
            event: event.clone(),
            commands,
            scope,
            path: path.to_path_buf(),
        });
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn frontmatter(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            return Some(
                rest.trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_string(),
            );
        }
    }
    None
}

/// Start a skill in `dir`: a new folder with a SKILL.md to fill in. Returns the file.
pub fn new_skill(dir: &Path) -> std::io::Result<PathBuf> {
    let name = (1..)
        .map(|n| {
            if n == 1 {
                "new-skill".to_string()
            } else {
                format!("new-skill-{n}")
            }
        })
        .find(|n| !dir.join(n).exists())
        .expect("some name is free");
    let folder = dir.join(&name);
    std::fs::create_dir_all(&folder)?;
    let file = folder.join("SKILL.md");
    std::fs::write(
        &file,
        format!(
            "---\nname: {name}\ndescription: What this skill does and when the agent should use it.\n---\n\nSteps for the agent to follow.\n"
        ),
    )?;
    Ok(file)
}

/// Where new personal skills go.
pub fn skills_home() -> Option<PathBuf> {
    home().map(|h| h.join(".claude").join("skills"))
}

fn scan_skills(dir: &Path, source: &'static str, out: &mut Vec<SkillInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path().join("SKILL.md");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let name = frontmatter(&text, "name")
                .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
            let description = frontmatter(&text, "description").unwrap_or_default();
            out.push(SkillInfo {
                name,
                description,
                path,
                source,
            });
        }
    }
}

fn scan_mcp(home: &Path, out: &mut Vec<McpInfo>) {
    // Claude Code: user-scoped servers in ~/.claude.json
    let claude = home.join(".claude.json");
    if let Some(value) = std::fs::read_to_string(&claude)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    {
        if let Some(servers) = value.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, cfg) in servers {
                let spec = crate::mcp::spec_from_json(cfg);
                out.push(McpInfo {
                    name: name.clone(),
                    provider: "Claude Code",
                    transport: spec.transport(),
                    enabled: true,
                    detail: spec.detail(),
                    path: claude.clone(),
                    spec,
                });
            }
        }
    }
    // Codex: [mcp_servers.<name>] tables in ~/.codex/config.toml
    let codex = home.join(".codex").join("config.toml");
    if let Ok(text) = std::fs::read_to_string(&codex) {
        let mut current: Option<McpInfo> = None;
        // Inside `[mcp_servers.<name>.env]` or `.http_headers`.
        let mut sub: Option<&str> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(header) = line
                .strip_prefix("[mcp_servers.")
                .and_then(|h| h.strip_suffix(']'))
            {
                if header.contains('.') && !header.starts_with('"') {
                    // A sub-table of the current server, such as `.env`.
                    sub = if header.ends_with(".env") {
                        Some("env")
                    } else if header.ends_with(".http_headers") {
                        Some("headers")
                    } else {
                        Some("other")
                    };
                    continue;
                }
                sub = None;
                if let Some(done) = current.take() {
                    out.push(done);
                }
                current = Some(McpInfo {
                    name: header.trim_matches('"').to_string(),
                    provider: "Codex",
                    transport: "stdio",
                    enabled: true,
                    detail: String::new(),
                    path: codex.clone(),
                    spec: Default::default(),
                });
            } else if line.starts_with('[') {
                sub = None;
                if let Some(done) = current.take() {
                    out.push(done);
                }
            } else if let Some(info) = current.as_mut() {
                if let Some(kind) = sub {
                    if let Some((k, v)) = line.split_once('=') {
                        let pair = (
                            k.trim().trim_matches('"').to_string(),
                            v.trim().trim_matches('"').to_string(),
                        );
                        match kind {
                            "env" => info.spec.env.push(pair),
                            "headers" => info.spec.headers.push(pair),
                            _ => {}
                        }
                    }
                    continue;
                }
                let value = |key: &str| {
                    line.strip_prefix(key)
                        .and_then(|r| r.trim_start().strip_prefix('='))
                        .map(|v| v.trim().trim_matches('"').to_string())
                };
                if let Some(cmd) = value("command") {
                    info.detail = cmd.clone();
                    info.spec.command = cmd;
                } else if let Some(args) = line
                    .strip_prefix("args")
                    .and_then(|r| r.trim_start().strip_prefix('='))
                {
                    info.spec.args = serde_json::from_str(args.trim()).unwrap_or_default();
                } else if let Some(url) = value("url") {
                    info.transport = "http";
                    info.detail = url.clone();
                    info.spec.url = url;
                } else if value("enabled").as_deref() == Some("false") {
                    info.enabled = false;
                }
            }
        }
        if let Some(done) = current {
            out.push(done);
        }
    }
    for (provider, path, field) in crate::mcp::other_sources(home) {
        crate::mcp::scan_json(provider, &path, field, out);
    }
    crate::mcp::scan_json(crate::mcp::OWN, &crate::mcp::own_path(), "mcpServers", out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
}

fn which(bin: &str) -> Option<PathBuf> {
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        suffixes
            .iter()
            .map(|s| dir.join(format!("{bin}{s}")))
            .find(|p| p.is_file())
    })
}

fn provider_mcp_label(provider: &str) -> &'static str {
    match provider {
        "Claude Desktop" => "Claude Desktop MCP servers",
        "Cursor" => "Cursor MCP servers",
        "Windsurf" => "Windsurf MCP servers",
        "VS Code" => "VS Code MCP servers",
        "Cline" => "Cline MCP servers",
        "Gemini CLI" => "Gemini CLI MCP servers",
        "Qwen Code" => "Qwen Code MCP servers",
        "Amp" => "Amp MCP servers",
        "LM Studio" => "LM Studio MCP servers",
        _ => "OpenCode MCP servers",
    }
}

pub fn discover(project_dir: Option<&str>) -> Discovered {
    let mut found = Discovered::default();
    if let Some(home) = home() {
        scan_skills(
            &home.join(".claude").join("skills"),
            "claude",
            &mut found.skills,
        );
        scan_skills(
            &home.join(".agents").join("skills"),
            "agents",
            &mut found.skills,
        );
        scan_skills(
            &home.join(".codex").join("skills"),
            "codex",
            &mut found.skills,
        );
    }
    if let Some(dir) = project_dir {
        scan_skills(
            &Path::new(dir).join(".claude").join("skills"),
            "project",
            &mut found.skills,
        );
    }
    found.skills.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(home) = home() {
        scan_mcp(&home, &mut found.mcp);
        let user = home.join(".claude").join("settings.json");
        scan_hooks(&user, "user", &mut found.hooks);
        found.files.extend([
            ("Claude settings and hooks", user),
            ("Claude MCP servers", home.join(".claude.json")),
            ("Codex config", home.join(".codex").join("config.toml")),
        ]);
        for (provider, path, _) in crate::mcp::other_sources(&home) {
            if found.mcp.iter().any(|m| m.provider == provider) {
                found.files.push((provider_mcp_label(provider), path));
            }
        }
    }
    if let Some(dir) = project_dir {
        crate::mcp::scan_json(
            "Project",
            &Path::new(dir).join(".mcp.json"),
            "mcpServers",
            &mut found.mcp,
        );
    }
    crate::mcp::set_found(&found.mcp);
    if let Some(dir) = project_dir {
        let project = Path::new(dir).join(".claude").join("settings.json");
        scan_hooks(&project, "project", &mut found.hooks);
        found.files.extend([
            ("Project Claude settings and hooks", project),
            ("Project MCP servers", Path::new(dir).join(".mcp.json")),
            (
                "Project agent instructions",
                Path::new(dir).join("AGENTS.md"),
            ),
            (
                "Project Claude instructions",
                Path::new(dir).join("CLAUDE.md"),
            ),
        ]);
    }
    found.provider_paths = crate::agent::PROVIDERS
        .iter()
        .map(|p| {
            let bin = crate::caps::binary(p);
            let configured = Path::new(&bin);
            let path = if bin.contains(std::path::MAIN_SEPARATOR) {
                configured.is_file().then(|| configured.to_path_buf())
            } else {
                which(&bin)
            };
            (p.id, path)
        })
        .collect();
    found
}

#[cfg(test)]
mod tests {
    use super::{
        frontmatter, mix, new_skill, parse_quick_commands, scan_hooks, scan_mcp, scan_skills,
        AppSettings,
    };
    use std::path::PathBuf;

    #[test]
    fn quick_commands_parse_names() {
        let parsed = parse_quick_commands("Test = cargo test\n# note\n\nls -la\n= bad");
        assert_eq!(
            parsed,
            [
                ("Test".to_string(), "cargo test".to_string()),
                ("ls -la".to_string(), "ls -la".to_string()),
                ("= bad".to_string(), "= bad".to_string()),
            ]
        );
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_project_setup_command_comes_first() {
        let s = AppSettings {
            worktree_setup: " npm ci ".into(),
            ..Default::default()
        };
        assert_eq!(s.setup_for("cargo fetch"), "cargo fetch");
        assert_eq!(s.setup_for("  "), "npm ci");
        assert_eq!(AppSettings::default().setup_for(""), "");
    }

    #[test]
    fn partial_settings_keep_defaults() {
        let s: AppSettings =
            serde_json::from_str(r#"{"theme":"dark","agents":{"claude":{"effort":"high"}}}"#)
                .unwrap();
        assert_eq!(s.theme, "dark");
        assert!(s.send_on_enter && s.confirm_delete && s.animations);
        assert_eq!(s.default_provider, "claude");
        assert_eq!(s.agent("claude").effort, "high");
        assert_eq!(s.agent("codex").effort, "");
    }

    #[test]
    fn colour_mixing() {
        assert_eq!(mix(0xffffff, 0x000000, 1.0), 0xffffff);
        assert_eq!(mix(0xffffff, 0x000000, 0.0), 0x000000);
        assert_eq!(mix(0xff0000, 0x0000ff, 0.5), 0x800080);
    }

    #[test]
    fn reads_skill_frontmatter() {
        let text = "---\nname: Review\ndescription: \"Checks diffs\"\n---\nbody\nname: ignored";
        assert_eq!(frontmatter(text, "name").as_deref(), Some("Review"));
        assert_eq!(
            frontmatter(text, "description").as_deref(),
            Some("Checks diffs")
        );
        assert_eq!(frontmatter(text, "missing"), None);
        assert_eq!(frontmatter("no frontmatter", "name"), None);

        let dir = temp("skills");
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("a/SKILL.md"), text).unwrap();
        std::fs::create_dir_all(dir.join("plain")).unwrap();
        std::fs::write(dir.join("plain/SKILL.md"), "just text").unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        let mut found = Vec::new();
        scan_skills(&dir, "claude", &mut found);
        found.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            found.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Review", "plain"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_mcp_servers_in_claude_and_codex_configs() {
        let home = temp("mcp");
        std::fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"files":{"command":"npx"},"remote":{"url":"https://x.dev/mcp"},"odd":{}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "model = \"x\"\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n[mcp_servers.docs.env]\nKEY = \"v\"\n[mcp_servers.web]\nurl = \"https://w.dev\"\nenabled = false\n[profiles.work]\nmodel = \"y\"\n",
        )
        .unwrap();
        let mut found = Vec::new();
        scan_mcp(&home, &mut found);
        let summary: Vec<(String, &str, &str, bool, String)> = found
            .iter()
            .map(|m| {
                (
                    m.name.clone(),
                    m.provider,
                    m.transport,
                    m.enabled,
                    m.detail.clone(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "docs".to_string(),
                    "Codex",
                    "stdio",
                    true,
                    "docs-mcp".to_string()
                ),
                (
                    "files".to_string(),
                    "Claude Code",
                    "stdio",
                    true,
                    "npx".to_string()
                ),
                (
                    "odd".to_string(),
                    "Claude Code",
                    "unknown",
                    true,
                    String::new()
                ),
                (
                    "remote".to_string(),
                    "Claude Code",
                    "http",
                    true,
                    "https://x.dev/mcp".to_string()
                ),
                (
                    "web".to_string(),
                    "Codex",
                    "http",
                    false,
                    "https://w.dev".to_string()
                ),
            ]
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn hooks_are_listed_per_event() {
        let dir = std::env::temp_dir().join(format!("runo-hooks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"hooks":{"PostToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"cargo fmt"}]}],"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        scan_hooks(&path, "user", &mut out);
        out.sort_by(|a, b| a.event.cmp(&b.event));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].commands, ["Edit: cargo fmt"]);
        assert_eq!(out[1].commands, ["say done"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_new_skill_gets_a_folder_of_its_own() {
        let dir = temp("new-skill");
        let first = new_skill(&dir).unwrap();
        let second = new_skill(&dir).unwrap();
        assert!(first.ends_with("new-skill/SKILL.md"));
        assert!(second.ends_with("new-skill-2/SKILL.md"));
        let mut out = Vec::new();
        scan_skills(&dir, "claude", &mut out);
        assert_eq!(out.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
