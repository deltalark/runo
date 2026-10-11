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
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub agents: HashMap<String, AgentOptions>,
    pub theme: String,
    pub accent: String,
    pub diff_colors: String,
    pub animations: bool,
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
    /// Group the sidebar by status (Pinned, Working, Needs review, Done) instead of project.
    pub group_by_status: bool,
    /// Translucent window over a blurred desktop: "off", "subtle", "frosted" or "clear".
    pub glass: String,
    /// Cards, menus and dialogs are translucent too.
    pub glass_cards: bool,
    /// Pass Runo's memory MCP server to agents that accept one per run.
    pub memory_server: bool,
    /// The first-run tour was finished or skipped.
    pub tour_seen: bool,
    /// How new sessions open: "chat" or "cli" (the agent's own CLI, where supported).
    pub default_view: String,
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
            notifications: false,
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
            memory_server: true,
            tour_seen: false,
            default_view: "chat".into(),
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

/// Window opacity for each glass style.
pub fn glass_alpha(style: &str) -> u8 {
    match style {
        "subtle" => 225,
        "frosted" => 190,
        "clear" => 150,
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

/// The window's base surface: see-through in glass mode.
pub fn glass_fill(color: u32) -> Rgba {
    with_alpha(color, GLASS.load(Ordering::Relaxed))
}

/// Sidebars, title and status bars: a light tint over the base surface in glass mode.
pub fn pane(color: u32) -> Rgba {
    if glass_on() {
        with_alpha(color, 110)
    } else {
        rgb(color)
    }
}

/// Dialogs, menus and popups floating over the app.
pub fn float(color: u32) -> Rgba {
    if glass_on() && GLASS_CARDS.load(Ordering::Relaxed) {
        with_alpha(color, 222)
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
pub fn c_secondary() -> u32 {
    pick(0x55555e, 0xa1a1aa)
}
pub fn c_muted() -> u32 {
    pick(0x9b9ba4, 0x6e6e78)
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
                let (transport, detail) = match (
                    cfg.get("command").and_then(|v| v.as_str()),
                    cfg.get("url").and_then(|v| v.as_str()),
                ) {
                    (Some(cmd), _) => ("stdio", cmd.to_string()),
                    (None, Some(url)) => ("http", url.to_string()),
                    _ => ("unknown", String::new()),
                };
                out.push(McpInfo {
                    name: name.clone(),
                    provider: "Claude",
                    transport,
                    enabled: true,
                    detail,
                    path: claude.clone(),
                });
            }
        }
    }
    // Codex: [mcp_servers.<name>] tables in ~/.codex/config.toml
    let codex = home.join(".codex").join("config.toml");
    if let Ok(text) = std::fs::read_to_string(&codex) {
        let mut current: Option<McpInfo> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(header) = line
                .strip_prefix("[mcp_servers.")
                .and_then(|h| h.strip_suffix(']'))
            {
                if header.contains('.') && !header.starts_with('"') {
                    continue; // sub-table such as `.env`
                }
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
                });
            } else if line.starts_with('[') {
                if let Some(done) = current.take() {
                    out.push(done);
                }
            } else if let Some(info) = current.as_mut() {
                let value = |key: &str| {
                    line.strip_prefix(key)
                        .and_then(|r| r.trim_start().strip_prefix('='))
                        .map(|v| v.trim().trim_matches('"').to_string())
                };
                if let Some(cmd) = value("command") {
                    info.detail = cmd;
                } else if let Some(url) = value("url") {
                    info.transport = "http";
                    info.detail = url;
                } else if value("enabled").as_deref() == Some("false") {
                    info.enabled = false;
                }
            }
        }
        if let Some(done) = current {
            out.push(done);
        }
    }
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
    }
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
        frontmatter, mix, parse_quick_commands, scan_hooks, scan_mcp, scan_skills, AppSettings,
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
                    "Claude",
                    "stdio",
                    true,
                    "npx".to_string()
                ),
                ("odd".to_string(), "Claude", "unknown", true, String::new()),
                (
                    "remote".to_string(),
                    "Claude",
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
}
