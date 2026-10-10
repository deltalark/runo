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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            agents: HashMap::new(),
            theme: "system".into(),
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
    let home = std::env::var_os("RUNO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".runo"))
        })
        .unwrap_or_else(|| PathBuf::from(".runo"));
    home.join("settings.json")
}

impl AppSettings {
    pub fn agent(&self, id: &str) -> AgentOptions {
        self.agents.get(id).cloned().unwrap_or_default()
    }

    pub fn load() -> Self {
        std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Push the values that hot paths read (colours, animation switch…) into statics.
    pub fn apply(&self) {
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
        NOTIFY.store(self.notifications, Ordering::Relaxed);
        CONFIRM_DELETE.store(self.confirm_delete, Ordering::Relaxed);
        REMOVE_WORKTREE.store(self.remove_worktree_on_delete, Ordering::Relaxed);
        AUTOSCROLL.store(self.autoscroll, Ordering::Relaxed);
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
static ACCENT: AtomicU32 = AtomicU32::new(0x3b5bdb);
static DIFF: AtomicU8 = AtomicU8::new(0);
static ANIMATIONS: AtomicBool = AtomicBool::new(true);
static NOTIFY: AtomicBool = AtomicBool::new(false);
static CONFIRM_DELETE: AtomicBool = AtomicBool::new(true);
static REMOVE_WORKTREE: AtomicBool = AtomicBool::new(true);
static AUTOSCROLL: AtomicBool = AtomicBool::new(true);

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

fn pick(light: u32, dark: u32) -> u32 {
    if is_dark() {
        dark
    } else {
        light
    }
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x * t + y * (1.0 - t)).round() as u32) & 0xff
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

pub fn c_canvas() -> u32 {
    pick(0xffffff, 0x14171c)
}
pub fn c_sidebar() -> u32 {
    pick(0xf8f9fb, 0x181b21)
}
pub fn c_subtle() -> u32 {
    pick(0xf3f5f8, 0x20242b)
}
pub fn c_border() -> u32 {
    pick(0xe6eaf0, 0x2b303a)
}
pub fn c_text() -> u32 {
    pick(0x1b2433, 0xe7eaf0)
}
pub fn c_secondary() -> u32 {
    pick(0x5d6a7c, 0xa3aebe)
}
pub fn c_muted() -> u32 {
    pick(0x93a0b1, 0x6f7b8d)
}
pub fn c_term_bg() -> u32 {
    pick(0xfafbfc, 0x111419)
}
pub fn c_blue() -> u32 {
    let accent = ACCENT.load(Ordering::Relaxed);
    if is_dark() {
        mix(accent, 0xffffff, 0.8)
    } else {
        accent
    }
}
pub fn c_selected() -> u32 {
    mix(c_blue(), c_canvas(), if is_dark() { 0.18 } else { 0.10 })
}
pub fn c_green() -> u32 {
    pick(0x1f9d55, 0x3ccf7a)
}
pub fn c_amber() -> u32 {
    pick(0xf0a020, 0xf3b23f)
}
pub fn c_red() -> u32 {
    pick(0xd0454c, 0xf0686e)
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
    pub provider_paths: Vec<(&'static str, Option<PathBuf>)>,
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

fn scan_mcp(out: &mut Vec<McpInfo>) {
    let Some(home) = home() else { return };
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
    scan_mcp(&mut found.mcp);
    found.provider_paths = crate::agent::PROVIDERS
        .iter()
        .map(|p| (p.id, which(p.bin)))
        .collect();
    found
}
