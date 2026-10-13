//! The Settings screen: grouped navigation, searchable pages, titled sections of rows.

use super::*;
use crate::connections::{self, Connection};
use gpui_kit::component::switch::Switch;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    General,
    Appearance,
    Terminal,
    Keybindings,
    Chat,
    Providers,
    /// One agent's settings, by provider id. Installed agents are listed under Providers.
    Agent(&'static str),
    Mcp,
    Skills,
    Hooks,
    Worktrees,
    Connections,
}

impl SettingsPage {
    const GROUPS: [(&'static str, &'static [SettingsPage]); 3] = [
        (
            "App",
            &[
                SettingsPage::General,
                SettingsPage::Appearance,
                SettingsPage::Terminal,
                SettingsPage::Keybindings,
            ],
        ),
        (
            "Agents",
            &[
                SettingsPage::Chat,
                SettingsPage::Providers,
                SettingsPage::Mcp,
                SettingsPage::Skills,
                SettingsPage::Hooks,
            ],
        ),
        (
            "Workspace",
            &[SettingsPage::Connections, SettingsPage::Worktrees],
        ),
    ];

    /// The pages in the Settings sidebar, in order.
    pub(crate) fn pages() -> impl Iterator<Item = SettingsPage> {
        Self::GROUPS
            .iter()
            .flat_map(|(_, pages)| pages.iter().copied())
    }

    /// Every page, including each agent's, for searching.
    fn all() -> impl Iterator<Item = SettingsPage> {
        Self::GROUPS
            .iter()
            .flat_map(|(_, pages)| pages.iter().copied())
            .chain(agent::PROVIDERS.iter().map(|p| SettingsPage::Agent(p.id)))
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal",
            Self::Keybindings => "Keybindings",
            Self::Chat => "Chat",
            Self::Providers => "Providers",
            Self::Agent(id) => agent::provider(id).name,
            Self::Mcp => "MCP",
            Self::Skills => "Skills",
            Self::Hooks => "Hooks",
            Self::Worktrees => "Worktrees",
            Self::Connections => "Connections",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::General => "Alerts, the built-in browser, motion and how deleting works.",
            Self::Appearance => "Theme, glass, fonts and how wide conversations run.",
            Self::Terminal => "Text size, font, scrolling and mouse behavior in the terminal panel and agent CLIs.",
            Self::Keybindings => "The shortcuts Runo responds to. ⌘ on macOS, Ctrl elsewhere. Inside a terminal on Linux and Windows, add Shift (Ctrl+Shift+K), or press Esc twice to leave it.",
            Self::Chat => "How the composer behaves and what new sessions start with.",
            Self::Providers => "The agent CLIs on this machine. Pick one to change its settings, or install another.",
            Self::Agent("claude") => "Everything Runo can pass to the claude CLI, plus what it detected.",
            Self::Agent("codex") => "Everything Runo can pass to codex exec, plus what it detected.",
            Self::Agent("opencode") => "Everything Runo can pass to opencode run, plus what it detected.",
            Self::Agent(_) => "What Runo detected from this CLI and what it passes on every run. Runo maps its permission levels onto the CLI's own flags.",
            Self::Mcp => "MCP servers found across your agents' config files.",
            Self::Skills => "Agent skills discovered in project, personal and agent folders.",
            Self::Hooks => "Commands Claude Code runs on agent events, and the config files behind them.",
            Self::Worktrees => "Where isolated sessions and Runo's data live.",
            Self::Connections => "Places sessions can run besides this machine: Cove VMs and your own servers. Sessions can move between them at any time.",
        }
    }

    fn agent(self) -> Option<&'static str> {
        match self {
            Self::Agent(id) => Some(id),
            _ => None,
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::Terminal => IconName::SquareTerminal,
            Self::Keybindings => IconName::Keyboard,
            Self::Chat => IconName::MessageSquare,
            Self::Providers => IconName::Layers,
            Self::Mcp => IconName::Plug,
            Self::Skills => IconName::Sparkles,
            Self::Hooks => IconName::Webhook,
            Self::Worktrees => IconName::GitBranch,
            Self::Connections => IconName::Server,
            _ => IconName::Bot,
        }
    }
}

type SetBool = Rc<dyn Fn(&mut AppSettings, bool)>;
type SetStr = Rc<dyn Fn(&mut AppSettings, String)>;
type Action = Rc<dyn Fn(&mut RunoApp, &mut Context<RunoApp>)>;

enum Ctl {
    Switch(bool, SetBool),
    Segment(Vec<(String, String)>, String, SetStr),
    Dropdown(Vec<(String, String)>, String, SetStr),
    Text {
        key: String,
        value: String,
        placeholder: String,
        set: SetStr,
    },
    Accent(String),
    Button(String, Action),
    /// Several buttons side by side; the last one is the primary.
    Buttons(Vec<(String, Action)>),
    Chip(String, u32),
    Keys(String),
    /// A shortcut that can be changed: click to record new keys.
    Shortcut(&'static str),
    /// Open the file in Runo's editor, or reveal it.
    Edit(PathBuf),
    /// A version chip and a chevron, for rows that open another page.
    Open(String),
    /// An MCP server: share it with Runo's sessions (key, shared), plus Edit and Reveal.
    Mcp(Option<(String, bool)>, PathBuf),
}

struct Row {
    title: String,
    desc: String,
    ctl: Ctl,
    /// Clicking the row opens this page.
    open: Option<SettingsPage>,
}

struct Section {
    title: String,
    desc: String,
    rows: Vec<Row>,
}

fn section(title: &str, desc: &str, rows: Vec<Row>) -> Section {
    Section {
        title: title.into(),
        desc: desc.into(),
        rows,
    }
}

fn row(title: &str, desc: &str, ctl: Ctl) -> Row {
    Row {
        title: title.into(),
        desc: desc.into(),
        ctl,
        open: None,
    }
}

fn switch_row(title: &str, desc: &str, on: bool, set: fn(&mut AppSettings, bool)) -> Row {
    row(title, desc, Ctl::Switch(on, Rc::new(set)))
}

/// A path with the home folder shown as ~.
fn tilde(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match home.and_then(|h| path.strip_prefix(h).ok().map(|p| p.display().to_string())) {
        Some(rest) => format!("~/{rest}"),
        None => path.display().to_string(),
    }
}

fn options(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(v, l)| (v.to_string(), l.to_string()))
        .collect()
}

fn agent_text(
    pid: &'static str,
    st: &AppSettings,
    title: &str,
    desc: &str,
    placeholder: &str,
    get: fn(&AgentOptions) -> String,
    set: fn(&mut AgentOptions, String),
) -> Row {
    row(
        title,
        desc,
        Ctl::Text {
            key: format!("{pid}.{title}"),
            value: get(&st.agent(pid)),
            placeholder: placeholder.into(),
            set: Rc::new(move |s, v| set(s.agents.entry(pid.to_string()).or_default(), v)),
        },
    )
}

fn agent_choice(
    pid: &'static str,
    st: &AppSettings,
    title: &str,
    desc: &str,
    opts: Vec<(String, String)>,
    get: fn(&AgentOptions) -> String,
    set: fn(&mut AgentOptions, String),
) -> Row {
    row(
        title,
        desc,
        Ctl::Dropdown(
            opts,
            get(&st.agent(pid)),
            Rc::new(move |s, v| set(s.agents.entry(pid.to_string()).or_default(), v)),
        ),
    )
}

fn list_values(choices: &[caps::Choice]) -> String {
    if choices.is_empty() {
        "None reported".into()
    } else {
        choices
            .iter()
            .map(|c| c.value.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn detected_section(pid: &'static str) -> Section {
    let caps = caps::get(pid);
    let mut rows = vec![row(
        "Installed version",
        "Read from the CLI with --version.",
        if caps.installed {
            Ctl::Chip(caps.version.clone(), 0)
        } else if caps.detected {
            Ctl::Button(
                "Install…".into(),
                Rc::new(move |this, _| this.ask_install(pid)),
            )
        } else {
            Ctl::Chip("Detecting…".into(), 2)
        },
    )];
    if !caps.models.is_empty() {
        let names: Vec<&str> = caps
            .models
            .iter()
            .take(10)
            .map(|m| m.choice.value.as_str())
            .collect();
        let more = caps.models.len().saturating_sub(10);
        rows.push(row(
            "Models",
            &format!(
                "{}{}",
                names.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ),
            Ctl::Chip(format!("{} found", caps.models.len()), 3),
        ));
    }
    match pid {
        "claude" => {
            rows.push(row(
                "Permission modes",
                &list_values(&caps.modes),
                Ctl::Chip(format!("{}", caps.modes.len()), 3),
            ));
            rows.push(row(
                "Effort levels",
                &list_values(&caps.efforts),
                Ctl::Chip(format!("{}", caps.efforts.len()), 3),
            ));
        }
        "codex" => {
            rows.push(row(
                "Sandbox modes",
                &list_values(&caps.modes),
                Ctl::Chip(format!("{}", caps.modes.len()), 3),
            ));
            rows.push(row(
                "Approval policies",
                &list_values(&caps.approvals),
                Ctl::Chip(format!("{}", caps.approvals.len()), 3),
            ));
            if let Some(m) = caps.models.first() {
                rows.push(row(
                    "Reasoning levels",
                    &format!("{}: {}", m.choice.label, list_values(&m.efforts)),
                    Ctl::Chip(format!("{}", m.efforts.len()), 3),
                ));
                if !m.tiers.is_empty() {
                    rows.push(row(
                        "Speed tiers",
                        &m.tiers
                            .iter()
                            .map(|t| format!("{} ({})", t.label, t.value))
                            .collect::<Vec<_>>()
                            .join(", "),
                        Ctl::Chip(format!("{}", m.tiers.len()), 3),
                    ));
                }
            }
        }
        _ => {
            rows.push(row(
                "Agents",
                &list_values(&caps.agents),
                Ctl::Chip(format!("{}", caps.agents.len()), 3),
            ));
            rows.push(row(
                "Permission styles",
                &list_values(&caps.modes),
                Ctl::Chip(format!("{}", caps.modes.len()), 3),
            ));
        }
    }
    rows.push(row(
        "Re-detect options",
        "Ask the CLI again after updating or signing in. Menus in the composer refresh automatically.",
        Ctl::Button("Re-detect".into(), Rc::new(|_, _| install::redetect())),
    ));
    section("Detected from the CLI", "Runo asks each CLI what it supports instead of hard-coding it, so new models and modes appear on their own.", rows)
}

fn launch_section(pid: &'static str, st: &AppSettings, model_hint: &str) -> Section {
    section(
        "Launch",
        "Applied every time a session of this agent runs. Choices made in the composer take priority.",
        vec![
            agent_text(pid, st, "Default model", model_hint, "Use the CLI's own default", |o| o.model_override.clone(), |o, v| o.model_override = v),
            agent_text(pid, st, "Executable", "Full path to run instead of the one found on PATH.", "Use PATH", |o| o.binary.clone(), |o, v| o.binary = v),
            agent_text(pid, st, "Extra arguments", "Added to every command with shell-style quoting, for example --verbose.", "None", |o| o.extra_args.clone(), |o, v| o.extra_args = v),
            row(
                "Environment",
                "Variables for the agent process as KEY=VALUE, separated by semicolons.",
                Ctl::Text {
                    key: format!("{pid}.env"),
                    value: st.agent(pid).env.replace('\n', ";"),
                    placeholder: "KEY=value;OTHER=value".into(),
                    set: Rc::new(move |s, v| s.agents.entry(pid.to_string()).or_default().env = v.replace(';', "\n")),
                },
            ),
        ],
    )
}

fn page_sections(page: SettingsPage, st: &AppSettings, data: &Discovered) -> Vec<Section> {
    match page {
        SettingsPage::General => vec![
            section("Getting started", "A one-minute walk through the window and what each part does.", vec![
                row("Welcome tour", "Highlights each part of Runo with a short explanation. Also in the palette as Take the tour.", Ctl::Button("Take the tour".into(), Rc::new(|this, _| this.start_tour()))),
            ]),
            section("Browser", "Show pages in the Browser tab with a headless copy of the Chrome, Edge, Brave or Chromium on this machine, using its own profile.", vec![
                switch_row("Built-in browser", "Adds the Browser tab: open your dev server beside the conversation and click and type in it. Off, the tab is hidden.", st.browser_enabled, |s, v| s.browser_enabled = v),
                match browser_engine::find_chromium() {
                    Some(path) => row("Uses", &tilde(&path), Ctl::Chip(browser_engine::browser_name(&path), 0)),
                    None => row("Uses", "No Chromium-based browser found in Applications. Downloading Chrome for Testing is coming next.", Ctl::Chip("Not found".into(), 1)),
                },
            ]),
            section("Alerts", "How Runo reaches you while you are looking somewhere else.", vec![
                switch_row("Notifications", "Show a desktop notification when an agent finishes or fails (macOS and Linux).", st.notifications, |s, v| s.notifications = v),
            ]),
            section("Behavior", "Motion and safety around destructive actions.", vec![
                switch_row("Animations", "Fade, slide and pulse effects. Turn off for instant changes.", st.animations, |s, v| s.animations = v),
                switch_row("Confirm before deleting", "Ask for a second click before deleting a session.", st.confirm_delete, |s, v| s.confirm_delete = v),
                switch_row("Remove worktree with session", "Also delete the session's git worktree when the session is deleted. Uncommitted changes in it are lost.", st.remove_worktree_on_delete, |s, v| s.remove_worktree_on_delete = v),
                switch_row("Keep the computer awake", "Hold off sleep while any agent is running (caffeinate on macOS, systemd-inhibit on Linux).", st.keep_awake, |s, v| s.keep_awake = v),
                switch_row("Group sidebar by status", "Show Needs you, Running, Ready to review and Done groups instead of projects.", st.group_by_status, |s, v| s.group_by_status = v),
            ]),
            section("Git", "How Runo commits for you.", vec![
                switch_row("Credit the agent in commits", "Adds a line such as \"Made with Claude Code in Runo\" to commits made from the Changes panel.", st.attribution, |s, v| s.attribution = v),
                row(
                    "Quick commands",
                    "Buttons over the Terminal. Write Name = command, separated by semicolons.",
                    Ctl::Text {
                        key: "quick_commands".into(),
                        value: st.quick_commands.replace('\n', "; "),
                        placeholder: "Test = cargo test; Lint = cargo clippy".into(),
                        set: Rc::new(|s, v| s.quick_commands = v.split(';').map(str::trim).collect::<Vec<_>>().join("\n")),
                    },
                ),
            ]),
            section("New worktrees", "Prepare each isolated worktree before the agent starts. Ignored files listed in the repository's .worktreeinclude (gitignore patterns, such as .env) are copied in automatically.", vec![
                row(
                    "Setup command",
                    "Runs in the new worktree's Terminal, for example npm install, for projects without their own (set one in a project's Edit dialog). Leave empty to skip.",
                    Ctl::Text {
                        key: "worktree_setup".into(),
                        value: st.worktree_setup.clone(),
                        placeholder: "None".into(),
                        set: Rc::new(|s, v| s.worktree_setup = v),
                    },
                ),
            ]),
        ],
        SettingsPage::Appearance => vec![
            section("Interface", "System follows your OS appearance.", vec![
                row("Theme", "Light, dark, or match the system.", Ctl::Segment(options(&[("system", "System"), ("light", "Light"), ("dark", "Dark")]), st.theme.clone(), Rc::new(|s, v| s.theme = v))),
                row("Accent color", "Used for selection, links and active states.", Ctl::Accent(st.accent.clone())),
                row("Glass", "A translucent window over a blurred desktop, like macOS glass. Clear shows the most of what is behind.", Ctl::Segment(options(&[("off", "Off"), ("subtle", "Subtle"), ("frosted", "Frosted"), ("clear", "Clear")]), st.glass.clone(), Rc::new(|s, v| s.glass = v))),
                row("Interface font", "Runo's own font is tuned for sharp text at small sizes. System uses your operating system's font.", Ctl::Segment(options(&[("bundled", "Runo"), ("system", "System")]), st.font.clone(), Rc::new(|s, v| s.font = v))),
                row("Font smoothing", "Thickens text strokes, like older macOS. Off keeps text crisp and light. Applies after you restart Runo.", Ctl::Switch(st.font_smoothing, Rc::new(|s, v| s.font_smoothing = v))),
                row("Glass menus and dialogs", "Popups, menus and dialogs are translucent with a light rim too.", Ctl::Switch(st.glass_cards, Rc::new(|s, v| s.glass_cards = v))),
                row("Diff colors", "Colorblind uses blue and orange instead of green and red. High contrast uses stronger tints.", Ctl::Segment(options(&[("default", "Default"), ("colorblind", "Colorblind"), ("contrast", "High contrast")]), st.diff_colors.clone(), Rc::new(|s, v| s.diff_colors = v))),
            ]),
            section("Conversation", "How replies, errors and code use the space in a session.", vec![
                row("Content width", "Full uses the whole width of the window so long output needs less scrolling. Comfortable keeps lines short for reading.", Ctl::Segment(options(&[("comfortable", "Comfortable"), ("wide", "Wide"), ("full", "Full")]), st.content_width.clone(), Rc::new(|s, v| s.content_width = v))),
            ]),
        ],
        SettingsPage::Terminal => vec![
            section("Typography", "Applies to shell tabs and to agents running in their own CLI.", vec![
                row("Font size", "Text size in the terminal grid.", Ctl::Segment(options(&[("11", "11"), ("12", "12"), ("13", "13"), ("14", "14"), ("16", "16")]), st.term_font_size.to_string(), Rc::new(|s, v| s.term_font_size = v.parse().unwrap_or(12)))),
                row(
                    "Font family",
                    "Any monospace font installed on this computer, for example JetBrains Mono or Menlo.",
                    Ctl::Text {
                        key: "term_font".into(),
                        value: st.term_font.clone(),
                        placeholder: "Theme monospace font".into(),
                        set: Rc::new(|s, v| s.term_font = v.trim().to_string()),
                    },
                ),
            ]),
            section("Interaction", "How the mouse works in the terminal.", vec![
                row("Scroll speed", "Lines moved per wheel step. Fast suits long logs.", Ctl::Segment(options(&[("normal", "Normal"), ("fast", "Fast"), ("faster", "Faster")]), st.term_scroll.clone(), Rc::new(|s, v| s.term_scroll = v))),
                row("Scrollback", "Lines each new terminal keeps to scroll back through and find in.", Ctl::Segment(options(&[("2000", "2,000"), ("10000", "10,000"), ("50000", "50,000")]), st.term_history.to_string(), Rc::new(|s, v| s.term_history = v.parse().unwrap_or(10_000)))),
                row("Right-click", "Show the terminal menu, or paste the clipboard straight in like many terminals do.", Ctl::Segment(options(&[("menu", "Menu"), ("paste", "Paste")]), st.term_right_click.clone(), Rc::new(|s, v| s.term_right_click = v))),
            ]),
        ],
        SettingsPage::Keybindings => {
            let m = MOD_KEY.trim_end_matches('+');
            vec![
                section("Workspace", "Available everywhere in the window. Click a shortcut, then press the keys you want.", {
                    let mut rows: Vec<Row> = crate::keymap::SHORTCUTS
                        .iter()
                        .map(|s| row(s.title, s.desc, Ctl::Shortcut(s.id)))
                        .collect();
                    rows.push(row("Close", "Close a menu, cancel a delete, or go back to the workspace.", Ctl::Keys("Esc".into())));
                    if !st.keybindings.is_empty() {
                        rows.push(row(
                            "Reset all shortcuts",
                            &format!("{} changed from the defaults.", st.keybindings.len()),
                            Ctl::Button("Reset all".into(), Rc::new(|this, cx| this.reset_shortcut(None, cx))),
                        ));
                    }
                    rows
                }),
                section("Composer", "Change the send key in Chat settings.", vec![
                    row("Send message", "Sends the prompt to the session's agent.", Ctl::Keys(if st.send_on_enter { "Enter".into() } else { format!("{m} Enter") })),
                    row("New line", "Insert a line break in the composer.", Ctl::Keys(if st.send_on_enter { "Shift Enter".into() } else { "Enter".into() })),
                    row("Start session", "In the New session form.", Ctl::Keys(format!("{m} Enter"))),
                    row("Copy selection", "Works in the conversation, terminal output and file previews.", Ctl::Keys(format!("{m} C"))),
                ]),
            ]
        }
        SettingsPage::Chat => vec![
            section("Composer", "What the composer does with what you type.", vec![
                row("Send message with", "The other key inserts a new line.", Ctl::Segment(options(&[("enter", "Enter"), ("cmd", "⌘ / Ctrl Enter")]), if st.send_on_enter { "enter".into() } else { "cmd".into() }, Rc::new(|s, v| s.send_on_enter = v == "enter"))),
                switch_row("Follow output", "Scroll the conversation to the latest output while an agent works.", st.autoscroll, |s, v| s.autoscroll = v),
            ]),
            section("Memory", "Runo keeps a summary of each session in .runo/memory.md inside its folder, kept out of git.", vec![
                switch_row("Runo tools for agents", "Give Claude Code and Codex Runo's tools: read the session memory, search earlier sessions, save notes, and use playbooks (as /mcp__runo__… commands in Claude Code).", st.memory_server, |s, v| s.memory_server = v),
            ]),
            section("New sessions", "What a session starts with before you change it in the composer.", vec![
                row("Default view", "How new sessions open. CLI shows the agent's own command line on the same conversation (Claude Code and Codex); switch any time from the session header at no token cost.", Ctl::Segment(vec![("chat".into(), "Chat".into()), ("cli".into(), "CLI".into())], st.default_view.clone(), Rc::new(|s, v| s.default_view = v))),
                row("Default agent", "Preselected in New session.", Ctl::Dropdown(agent::PROVIDERS.iter().map(|p| (p.id.to_string(), p.name.to_string())).collect(), st.default_provider.clone(), Rc::new(|s, v| s.default_provider = v))),
                row("Default permission level", "Maps to each agent's closest mode: auto edit, read-only, or full access.", Ctl::Segment(options(&[("edit", "Auto edit"), ("plan", "Plan only"), ("full", "Full access")]), st.default_mode.clone(), Rc::new(|s, v| s.default_mode = v))),
            ]),
            section("Playbooks", "Saved tasks that run as their own sessions, by hand, on a schedule or when an agent asks.", vec![
                switch_row("Ask before agents start playbooks", "When an agent asks Runo to start a playbook, show what it may do and wait for you. Off starts it right away.", st.confirm_agent_playbooks, |s, v| s.confirm_agent_playbooks = v),
                row("Scheduled runs to keep", "Older runs of each scheduled playbook are deleted, with their worktrees. Pinned and unread runs stay.", Ctl::Dropdown(options(&[("5", "5"), ("10", "10"), ("20", "20"), ("50", "50"), ("0", "All")]), st.playbook_keep_runs.to_string(), Rc::new(|s, v| s.playbook_keep_runs = v.parse().unwrap_or(10)))),
            ]),
        ],
        SettingsPage::Providers => {
            let _ = &data.provider_paths;
            let (mut installed, mut available) = (Vec::new(), Vec::new());
            let managers = crate::shellenv::managers();
            let looked = if managers.is_empty() {
                "your PATH".to_string()
            } else {
                format!("your PATH, including {}", managers.join(", "))
            };
            for p in agent::PROVIDERS.iter() {
                let caps = caps::get(p.id);
                let pid = p.id;
                if caps.installed {
                    let path = install::agent_path(p);
                    let desc = match &path {
                        Some(path) => match crate::shellenv::found_via(path) {
                            Some(via) => format!("{} · via {via}", tilde(path)),
                            None => tilde(path),
                        },
                        None => String::new(),
                    };
                    let mut r = row(p.name, &desc, Ctl::Open(caps.version.clone()));
                    r.open = Some(SettingsPage::Agent(p.id));
                    installed.push(r);
                } else {
                    available.push(row(
                        p.name,
                        &if caps.detected {
                            format!("`{}` wasn't found in {looked}. Already installed somewhere else? Locate it.", p.bin)
                        } else {
                            "Checking whether it's installed…".into()
                        },
                        if caps.detected {
                            Ctl::Buttons(vec![
                                ("Locate…".into(), Rc::new(move |this: &mut RunoApp, cx: &mut Context<RunoApp>| this.locate_agent(pid, cx)) as Action),
                                ("Install…".into(), Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| this.ask_install(pid)) as Action),
                            ])
                        } else {
                            Ctl::Chip("Detecting…".into(), 2)
                        },
                    ));
                }
            }
            let mut sections = Vec::new();
            let node = match install::node() {
                Some((version, path)) => row(
                    "Node.js",
                    &match crate::shellenv::found_via(&path) {
                        Some(via) => format!("{} · via {via}. Agents installed with npm use it.", tilde(&path)),
                        None => format!("{}. Agents installed with npm use it.", tilde(&path)),
                    },
                    Ctl::Chip(if version.is_empty() { "Found".into() } else { version }, 0),
                ),
                None => row(
                    "Node.js",
                    &format!("Not found in {looked}. Codex, Gemini, Copilot, Amp and Qwen install with npm, which comes with Node.js from nodejs.org."),
                    Ctl::Chip("Not found".into(), 1),
                ),
            };
            let folders = crate::shellenv::folders();
            sections.push(section("Environment", "Runo reads your login shell's PATH, so it finds the same programs your terminal does, including Node.js from nvm, fnm, Volta, asdf or Homebrew.", vec![
                node,
                row(
                    "Search path",
                    &format!(
                        "{} folders{}.",
                        folders.len(),
                        if managers.is_empty() { String::new() } else { format!(", including {}", managers.join(", ")) }
                    ),
                    Ctl::Button("Re-detect".into(), Rc::new(|this, _| {
                        this.info = Some(("Looking for agents and Node.js again…".into(), std::time::Instant::now()));
                        install::redetect();
                    })),
                ),
            ]));
            if !installed.is_empty() {
                sections.push(section("Installed", "Pick an agent to change its model, flags and environment. Installed agents are also listed under Providers on the left.", installed));
            }
            if !available.is_empty() {
                sections.push(section("More agents", "Install any of these with the vendor's official installer, or point Runo at a copy you already have. Once found it appears in the list above and in the sidebar.", available));
            }
            sections
        }
        SettingsPage::Agent("claude") => vec![
            detected_section("claude"),
            launch_section("claude", st, "Model alias or full name, used when the session has none selected."),
            section("Behavior", "Passed to claude on every run.", vec![
                agent_text("claude", st, "Append to system prompt", "Extra instructions added to Claude's system prompt (--append-system-prompt).", "None", |o| o.append_prompt.clone(), |o, v| o.append_prompt = v),
                agent_text("claude", st, "Allowed tools", "Comma-separated rules that never ask, for example Read,Edit,Bash(git diff:*).", "None", |o| o.allowed_tools.clone(), |o, v| o.allowed_tools = v),
                agent_text("claude", st, "Denied tools", "Comma-separated rules that are always blocked.", "None", |o| o.disallowed_tools.clone(), |o, v| o.disallowed_tools = v),
                agent_text("claude", st, "Extra directories", "Comma-separated folders Claude may access besides the project (--add-dir).", "None", |o| o.add_dirs.clone(), |o, v| o.add_dirs = v),
                agent_text("claude", st, "Fallback model", "Used automatically when the main model is overloaded (--fallback-model).", "None", |o| o.fallback_model.clone(), |o, v| o.fallback_model = v),
                agent_choice("claude", st, "Default effort", "How much thinking Claude does when the composer has none selected.", std::iter::once(("".to_string(), "Default".to_string())).chain(caps::get("claude").efforts.iter().map(|c| (c.value.clone(), c.label.clone()))).collect(), |o| o.effort.clone(), |o, v| o.effort = v),
            ]),
        ],
        SettingsPage::Agent("codex") => vec![
            detected_section("codex"),
            launch_section("codex", st, "Any model your account supports. Empty uses the model in ~/.codex/config.toml."),
            section("Behavior", "Passed to codex exec on every run.", vec![
                agent_text("codex", st, "Config profile", "A profile from ~/.codex/config.toml (-p).", "None", |o| o.profile.clone(), |o, v| o.profile = v),
                agent_text("codex", st, "Extra directories", "Comma-separated writable folders besides the project (--add-dir).", "None", |o| o.add_dirs.clone(), |o, v| o.add_dirs = v),
                agent_choice("codex", st, "Default reasoning", "model_reasoning_effort when the composer has none selected.", std::iter::once(("".to_string(), "Default".to_string())).chain(caps::get("codex").models.first().map(|m| m.efforts.clone()).unwrap_or_default().iter().map(|c| (c.value.clone(), c.label.clone()))).collect(), |o| o.effort.clone(), |o, v| o.effort = v),
            ]),
        ],
        SettingsPage::Agent("opencode") => vec![
            detected_section("opencode"),
            launch_section("opencode", st, "In provider/model form, for example opencode/big-pickle."),
            section("Behavior", "Passed to opencode run on every run.", vec![
                agent_text("opencode", st, "Default agent", "Run with a named OpenCode agent (--agent). Plan only uses the plan agent when this is empty.", "None", |o| o.sub_agent.clone(), |o, v| o.sub_agent = v),
                agent_text("opencode", st, "Model variant", "Provider-specific reasoning effort such as high, max or minimal (--variant).", "None", |o| o.variant.clone(), |o, v| o.variant = v),
                row("Show thinking", "Include thinking blocks in the output (--thinking).", Ctl::Switch(st.agent("opencode").thinking, Rc::new(|s, v| s.agents.entry("opencode".into()).or_default().thinking = v))),
            ]),
        ],
        SettingsPage::Agent(id) => vec![
            detected_section(id),
            launch_section(id, st, "Passed with the CLI's model flag."),
        ],
        SettingsPage::Mcp => {
            let mut sections = Vec::new();
            let shared = data
                .mcp
                .iter()
                .filter(|m| st.mcp_shared.contains(&crate::mcp::key(m)))
                .count();
            sections.push(section(
                "Use across agents",
                "Each agent loads its own servers. Turn on Use in Runo to hand a server from any app to Claude Code and Codex sessions too.",
                vec![row(
                    "Shared servers",
                    if shared == 0 {
                        "None yet. Switch one on below."
                    } else {
                        "Passed to every new run of Claude Code and Codex that doesn't already have them."
                    },
                    Ctl::Chip(format!("{shared} shared"), if shared == 0 { 3 } else { 0 }),
                )],
            ));
            let own_path = crate::mcp::own_path();
            let mut own: Vec<Row> = data
                .mcp
                .iter()
                .filter(|m| m.provider == crate::mcp::OWN)
                .map(|m| {
                    let (name, path) = (m.name.clone(), own_path.clone());
                    let edit = own_path.clone();
                    row(
                        &m.name,
                        &if m.transport == "unknown" {
                            "Not set up yet. Edit it to give a command or a URL.".to_string()
                        } else {
                            format!("{} · {}{}", m.transport, if m.enabled { "On" } else { "Disabled" }, if m.detail.is_empty() { String::new() } else { format!(" · {}", m.detail) })
                        },
                        Ctl::Buttons(vec![
                            ("Edit".into(), Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| this.edit_path(&edit)) as Action),
                            ("Remove".into(), Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| {
                                if let Err(e) = crate::mcp::remove_own(&path, &name) {
                                    this.notice = Some(format!("Couldn't remove {name}: {e}"));
                                }
                                this.open_settings();
                            }) as Action),
                        ]),
                    )
                })
                .collect();
            let add_path = own_path.clone();
            own.push(row(
                "Add a server",
                "Adds a blank entry to Runo's list and opens it to fill in a command and arguments, or a URL.",
                Ctl::Buttons(vec![("Add…".into(), Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| match crate::mcp::add_own(&add_path) {
                    Ok(_) => this.edit_path(&add_path),
                    Err(e) => this.notice = Some(format!("Couldn't add a server: {e}")),
                }) as Action)]),
            ));
            sections.push(section(
                "Runo's servers",
                &format!("Given to every Claude Code and Codex session, on this computer and on connections. Kept in {}.", tilde(&own_path)),
                own,
            ));
            let mut providers: Vec<&'static str> = Vec::new();
            for m in data.mcp.iter().filter(|m| m.provider != crate::mcp::OWN) {
                if !providers.contains(&m.provider) {
                    providers.push(m.provider);
                }
            }
            let order = |p: &str| match p {
                "Claude Code" => 0,
                "Codex" => 1,
                "Project" => 99,
                _ => 2,
            };
            providers.sort_by_key(|p| (order(p), p.to_string()));
            for provider in providers {
                let servers: Vec<&McpInfo> = data.mcp.iter().filter(|m| m.provider == provider).collect();
                let path = servers.first().map(|m| tilde(&m.path)).unwrap_or_default();
                let rows: Vec<Row> = servers
                    .iter()
                    .map(|m| {
                        let key = crate::mcp::key(m);
                        let share = (provider != "Project" && m.transport != "unknown")
                            .then(|| (key.clone(), st.mcp_shared.contains(&key)));
                        row(
                            &m.name,
                            &format!("{} · {}{}", m.transport, if m.enabled { "Configured" } else { "Disabled" }, if m.detail.is_empty() { String::new() } else { format!(" · {}", m.detail) }),
                            Ctl::Mcp(share, m.path.clone()),
                        )
                    })
                    .collect();
                let title = if provider == "Project" { "This project (.mcp.json)".to_string() } else { format!("{provider} servers") };
                sections.push(section(&title, &format!("From {path}"), rows));
            }
            let files: Vec<Row> = data
                .files
                .iter()
                .filter(|(label, _)| label.contains("MCP") || label.contains("Codex"))
                .map(|(label, path)| row(label, &path.display().to_string(), Ctl::Edit(path.clone())))
                .collect();
            sections.push(section("Config files", "Edit servers directly. Files that do not exist yet are created when you save.", files));
            if data.mcp.is_empty() {
                sections.insert(1, section("MCP servers", "Runo looks in Claude Code, Codex, Claude Desktop, Cursor, VS Code, Windsurf, Cline, Gemini CLI, Qwen Code, Amp, LM Studio and OpenCode.", vec![row("No servers found", "Add one in any of those apps and it will appear here.", Ctl::Chip("0".into(), 3))]));
            }
            sections
        }
        SettingsPage::Skills => {
            let mut sections = vec![section(
                "Create",
                "A skill is a folder with a SKILL.md: a name, a description saying when to use it, and the steps.",
                vec![row(
                    "New skill",
                    "Starts one in ~/.claude/skills and opens it in Runo's editor.",
                    Ctl::Buttons(vec![("New skill…".into(), Rc::new(|this: &mut RunoApp, _: &mut Context<RunoApp>| {
                        match settings::skills_home().map(|dir| settings::new_skill(&dir)) {
                            Some(Ok(file)) => this.edit_path(&file),
                            Some(Err(e)) => this.notice = Some(format!("Couldn't create a skill: {e}")),
                            None => this.notice = Some("Couldn't find your home folder.".into()),
                        }
                    }) as Action)]),
                )],
            )];
            for source in ["project", "claude", "agents", "codex"] {
                let rows: Vec<Row> = data
                    .skills
                    .iter()
                    .filter(|k| k.source == source)
                    .map(|k| row(&k.name, &k.description.chars().take(200).collect::<String>(), Ctl::Edit(k.path.clone())))
                    .collect();
                if !rows.is_empty() {
                    let title = match source {
                        "project" => "Project skills",
                        "claude" => "Claude skills (~/.claude/skills)",
                        "agents" => "Shared agent skills (~/.agents/skills)",
                        _ => "Codex skills (~/.codex/skills)",
                    };
                    sections.push(section(title, &format!("{} found", rows.len()), rows));
                }
            }
            if sections.len() == 1 {
                sections.push(section("Skills", "Skills live in SKILL.md files under your agents' skills folders.", vec![row("No skills found", "Create one in ~/.claude/skills, ~/.agents/skills or ~/.codex/skills.", Ctl::Chip("0".into(), 3))]));
            }
            sections
        }
        SettingsPage::Hooks => {
            let mut sections = Vec::new();
            for scope in ["project", "user"] {
                let rows: Vec<Row> = data
                    .hooks
                    .iter()
                    .filter(|h| h.scope == scope)
                    .map(|h| {
                        let n = h.commands.len();
                        let summary = format!("{n} command{} · {}", if n == 1 { "" } else { "s" }, h.commands.join(" · "));
                        row(&h.event, &summary.chars().take(200).collect::<String>(), Ctl::Edit(h.path.clone()))
                    })
                    .collect();
                if !rows.is_empty() {
                    let title = if scope == "project" { "Project hooks" } else { "Your hooks (~/.claude/settings.json)" };
                    sections.push(section(title, &format!("{} events", rows.len()), rows));
                }
            }
            if sections.is_empty() {
                sections.push(section("Hooks", "Hooks run a shell command when the agent uses a tool, finishes, or asks for permission.", vec![row("No hooks found", "Add a \"hooks\" block to a settings file below, for example PostToolUse with matcher Edit running your formatter.", Ctl::Chip("0".into(), 3))]));
            }
            let files: Vec<Row> = data
                .files
                .iter()
                .filter(|(label, _)| !label.contains("MCP") && !label.contains("Codex"))
                .map(|(label, path)| row(label, &path.display().to_string(), Ctl::Edit(path.clone())))
                .collect();
            sections.push(section("Config files", "Open in Runo's editor. Files that do not exist yet are created when you save.", files));
            sections
        }
        SettingsPage::Connections => connection_sections(st),
        SettingsPage::Worktrees => vec![section("Location", "Isolated sessions get their own git worktree and branch.", vec![
            row("Worktree folder", "Created next to each repository as .runo-worktrees/<repo>/<branch>.", Ctl::Chip(".runo-worktrees".into(), 3)),
            row("Runo data", "Projects and sessions are saved here. Settings live next to it in settings.json.", Ctl::Chip("~/.runo/state.json".into(), 3)),
        ])],
    }
}

/// A text field bound to one field of the connection with this id.
fn conn_text(
    c: &Connection,
    title: &str,
    desc: &str,
    placeholder: &str,
    value: String,
    set: fn(&mut Connection, String),
) -> Row {
    let id = c.id;
    row(
        title,
        desc,
        Ctl::Text {
            key: format!("conn.{id}.{title}"),
            value,
            placeholder: placeholder.into(),
            set: Rc::new(move |s, v| {
                if let Some(c) = s.connections.iter_mut().find(|c| c.id == id) {
                    set(c, v);
                }
            }),
        },
    )
}

fn conn_choice(
    c: &Connection,
    title: &str,
    desc: &str,
    opts: Vec<(String, String)>,
    value: String,
    set: fn(&mut Connection, String),
) -> Row {
    let id = c.id;
    row(
        title,
        desc,
        Ctl::Dropdown(
            opts,
            value,
            Rc::new(move |s, v| {
                if let Some(c) = s.connections.iter_mut().find(|c| c.id == id) {
                    set(c, v);
                }
            }),
        ),
    )
}

fn conn_switch(
    c: &Connection,
    title: &str,
    desc: &str,
    on: bool,
    set: fn(&mut Connection, bool),
) -> Row {
    let id = c.id;
    row(
        title,
        desc,
        Ctl::Switch(
            on,
            Rc::new(move |s, v| {
                if let Some(c) = s.connections.iter_mut().find(|c| c.id == id) {
                    set(c, v);
                }
            }),
        ),
    )
}

fn connection_sections(st: &AppSettings) -> Vec<Section> {
    let mut out = Vec::new();
    let mut list: Vec<Row> = vec![row(
        "This machine",
        "Sessions run directly on your computer. Always available.",
        Ctl::Chip("Built in".into(), 0),
    )];
    for c in &st.connections {
        let status = match connections::test_result(c.id) {
            Some((None, msg)) => msg,
            Some((Some(true), msg)) => msg,
            Some((Some(false), msg)) => format!("Couldn't connect: {msg}"),
            None => c.summary(),
        };
        let id = c.id;
        list.push(row(
            &c.name,
            &status,
            Ctl::Button(
                "Test".into(),
                Rc::new(move |this, cx| this.test_connection(id, cx)),
            ),
        ));
    }
    list.push(row(
        "Add a connection",
        "A Cove server runs each session in its own VM. An SSH server is any Linux or macOS machine you can ssh into.",
        Ctl::Buttons(vec![
            (
                "SSH server".into(),
                Rc::new(|this, cx| {
                    this.add_connection(connections::Kind::Ssh, cx);
                }),
            ),
            (
                "Cove".into(),
                Rc::new(|this, cx| {
                    this.add_connection(connections::Kind::Cove, cx);
                }),
            ),
        ]),
    ));
    out.push(section(
        "Connections",
        "Pick one under Runs on when you start a session, or move a running session from its header. On a server or a remote Cove, Runo syncs the session's files there before each agent turn and back after it, uncommitted changes and the agent's conversation included.",
        list,
    ));

    for c in &st.connections {
        let id = c.id;
        let mut rows = vec![conn_text(
            c,
            "Name",
            "Shown under Runs on.",
            "Build box",
            c.name.clone(),
            |c, v| c.name = v,
        )];
        match c.kind {
            connections::Kind::Ssh => {
                rows.push(conn_text(c, "Host", "A host from ~/.ssh/config, or user@host. Runo uses your SSH agent and config, and never turns off host key checks.", "anish@build-box", c.host.clone(), |c, v| c.host = v));
                rows.push(conn_text(
                    c,
                    "Port",
                    "Empty for the default.",
                    "22",
                    if c.port == 0 {
                        String::new()
                    } else {
                        c.port.to_string()
                    },
                    |c, v| c.port = v.trim().parse().unwrap_or(0),
                ));
                rows.push(conn_text(
                    c,
                    "Key file",
                    "Empty uses your SSH agent.",
                    "~/.ssh/id_ed25519",
                    c.identity.clone(),
                    |c, v| c.identity = v,
                ));
                rows.push(conn_text(
                    c,
                    "Folder",
                    "Where session copies live on the server, under the home folder.",
                    "runo-work",
                    c.workdir.clone(),
                    |c, v| c.workdir = v,
                ));
            }
            _ => {
                rows.push(conn_text(c, "Server", "Empty for Cove on this machine, which shares the session folder with the VM. A URL (http://my-vps:7457) uses a Cove server elsewhere; the session's files are synced to it.", "Cove on this machine", c.url.clone(), |c, v| c.url = v));
                if c.remote() {
                    rows.push(conn_text(
                        c,
                        "API key",
                        "Run cove key on the server.",
                        "",
                        c.api_key.clone(),
                        |c, v| c.api_key = v,
                    ));
                }
                rows.push(conn_choice(
                    c,
                    "CPUs",
                    "For each session's VM.",
                    options(&[("1", "1"), ("2", "2"), ("4", "4"), ("8", "8")]),
                    c.cpus.to_string(),
                    |c, v| c.cpus = v.parse().unwrap_or(2),
                ));
                rows.push(conn_choice(
                    c,
                    "Memory",
                    "For each session's VM.",
                    options(&[
                        ("2048", "2 GB"),
                        ("4096", "4 GB"),
                        ("8192", "8 GB"),
                        ("16384", "16 GB"),
                    ]),
                    c.memory_mb.to_string(),
                    |c, v| c.memory_mb = v.parse().unwrap_or(4096),
                ));
                rows.push(row(
                    "Network",
                    "Off keeps the VM offline, so an agent can't reach the internet. The agent CLI then has to be in the image already.",
                    Ctl::Segment(options(&[("full", "Full"), ("off", "Off")]), c.network.clone(), Rc::new(move |s, v| {
                        if let Some(c) = s.connections.iter_mut().find(|c| c.id == id) {
                            c.network = v;
                        }
                    })),
                ));
                rows.push(conn_choice(c, "Stop when idle", "Stop a session's VM when nothing has run in it for a while. It starts again on the next message.", options(&[("0", "Never"), ("15", "After 15 min"), ("30", "After 30 min"), ("60", "After 1 hour"), ("240", "After 4 hours")]), c.idle_stop.to_string(), |c, v| c.idle_stop = v.parse().unwrap_or(0)));
            }
        }
        rows.push(conn_text(c, "Keys passed in", "Environment variables from this machine the agent gets there, comma separated. Nothing else reaches it.", connections::DEFAULT_KEYS, c.keys.clone(), |c, v| c.keys = v));
        rows.push(conn_switch(c, "GitHub access", "Give the agent your GitHub login (from gh auth token) for its own work there, such as cloning or opening PRs. Syncing never needs it. Passed per command, never saved there.", c.github, |c, v| c.github = v));
        rows.push(conn_switch(c, "Agents may use it", "Lets agents in any session run commands here with Runo's run_on tool, for research, untrusted code or heavy jobs.", c.agents, |c, v| c.agents = v));
        rows.push(row(
            "Remove",
            "Sessions that run here move back to this machine, and their copies there are deleted.",
            Ctl::Button(
                "Remove".into(),
                Rc::new(move |this, cx| this.remove_connection(id, cx)),
            ),
        ));
        out.push(section(&c.name, &c.summary(), rows));
    }

    let installed = crate::cove::binary().is_some();
    let mut cove_rows = vec![if !installed {
        row(
            "Cove",
            "Not installed. Cove gives each session its own Linux VM on this machine or your server.",
            Ctl::Button(
                "Get Cove".into(),
                Rc::new(|_, cx| cx.open_url("https://github.com/deltalark/cove#install")),
            ),
        )
    } else if crate::cove::available() {
        row(
            "Cove",
            "Installed, and its server is running.",
            Ctl::Chip("Ready".into(), 2),
        )
    } else {
        row(
            "Cove",
            "Installed, but its server isn't running. Start it with cove serve.",
            Ctl::Chip("Not running".into(), 1),
        )
    }];
    cove_rows.push(switch_row(
        "Let agents use Cove",
        "Gives agents on this machine Cove's own tools (cove mcp), so they can make scratch VMs, run commands, move files and share ports on their own.",
        st.cove_tools,
        |s, v| s.cove_tools = v,
    ));
    cove_rows.push(switch_row(
        "Back up checkpoints to GitHub",
        "After each turn on a remote connection, push the session's files to the project's origin as refs/runo/sessions/<id>, which no branch list shows. Never for public repositories, and deleted with the session.",
        st.sync_backup,
        |s, v| s.sync_backup = v,
    ));
    if st.sync_backup {
        cove_rows.push(switch_row(
            "Back up untracked files too",
            "Off keeps new files that git doesn't track yet (notes, local config, secrets) out of the backup.",
            st.sync_backup_untracked,
            |s, v| s.sync_backup_untracked = v,
        ));
    }
    cove_rows.push(switch_row(
        "Stop at the first blocked command",
        "A turn on a server or Cove can't ask you mid-turn. When the agent tries something it isn't allowed to (Claude Code), the turn stops there and shows Allow and retry, instead of carrying on without it for hours.",
        st.stop_on_denial,
        |s, v| s.stop_on_denial = v,
    ));
    out.push(section(
        "Cove",
        "Self-hosted microVMs for agents: github.com/deltalark/cove.",
        cove_rows,
    ));

    let vms = if installed {
        crate::cove::vms()
    } else {
        Vec::new()
    };
    if !vms.is_empty() {
        let rows = vms
            .iter()
            .map(|vm| {
                let (stop, rm) = (vm.name.clone(), vm.name.clone());
                let mut buttons: Vec<(String, Action)> = Vec::new();
                if vm.state() == "running" {
                    buttons.push((
                        "Stop".into(),
                        Rc::new(move |_, _| crate::cove::vm_action("stop", &stop)),
                    ));
                }
                buttons.push((
                    "Delete".into(),
                    Rc::new(move |_, _| crate::cove::vm_action("rm", &rm)),
                ));
                row(
                    &vm.name,
                    &format!(
                        "{} · {} CPU · {} GB",
                        vm.state(),
                        vm.cpus,
                        vm.memory_mb / 1024
                    ),
                    Ctl::Buttons(buttons),
                )
            })
            .collect();
        out.push(section(
            "VMs on this machine",
            "Every Cove VM, including ones agents made. runo-<n> belongs to session n.",
            rows,
        ));
    }
    out
}

impl RunoApp {
    fn ensure_input(
        &mut self,
        key: &str,
        value: &str,
        placeholder: &str,
        set: SetStr,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(entity) = self.settings_inputs.get(key) {
            return entity.clone();
        }
        let entity = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder.to_string())
                .default_value(value.to_string())
        });
        let sub = cx.subscribe_in(&entity, window, move |_, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                let set = set.clone();
                settings::update(cx, move |s| set(s, value));
            }
        });
        self._subscriptions.push(sub);
        self.settings_inputs.insert(key.to_string(), entity.clone());
        entity
    }

    fn control(
        &mut self,
        ix: usize,
        ctl: Ctl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match ctl {
            Ctl::Switch(on, set) => Switch::new(("setting-switch", ix))
                .checked(on)
                .on_click(move |value, _, cx| {
                    let (set, value) = (set.clone(), *value);
                    settings::update(cx, move |s| set(s, value));
                })
                .into_any_element(),
            Ctl::Segment(opts, current, set) => {
                let mut group = div()
                    .flex()
                    .items_center()
                    .p(px(3.))
                    .gap(px(2.))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(rgb(c_canvas()));
                for (j, (value, label)) in opts.into_iter().enumerate() {
                    let selected = value == current;
                    let set = set.clone();
                    group = group.child(
                        div()
                            .id(("setting-seg", ix * 16 + j))
                            .h(px(28.))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded_md()
                            .cursor_pointer()
                            .text_sm()
                            .when(selected, |el| {
                                el.bg(tint(c_selected()))
                                    .text_color(rgb(c_blue()))
                                    .font_weight(FontWeight::MEDIUM)
                            })
                            .when(!selected, |el| {
                                el.text_color(rgb(c_secondary()))
                                    .hover(|el| el.bg(tint(c_subtle())))
                            })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let (set, value) = (set.clone(), value.clone());
                                settings::update(cx, move |s| set(s, value));
                            }))
                            .child(label),
                    );
                }
                group.into_any_element()
            }
            Ctl::Dropdown(opts, current, set) => {
                let key = format!("dd-{ix}");
                let open = self.settings_menu.as_deref() == Some(key.as_str());
                let label = opts
                    .iter()
                    .find(|(v, _)| *v == current)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_else(|| current.clone());
                let toggle_key = key.clone();
                let mut list = div().v_flex().p_1();
                for (j, (value, text)) in opts.iter().cloned().enumerate() {
                    let selected = value == current;
                    let set = set.clone();
                    list = list.child(
                        div()
                            .id(("setting-dd-item", ix * 64 + j))
                            .h(px(32.))
                            .px_2()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .rounded_md()
                            .cursor_pointer()
                            .text_sm()
                            .when(selected, |el| {
                                el.bg(tint(c_selected())).text_color(rgb(c_blue()))
                            })
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings_menu = None;
                                let (set, value) = (set.clone(), value.clone());
                                settings::update(cx, move |s| set(s, value));
                            }))
                            .child(text)
                            .when(selected, |el| {
                                el.child(icon(IconName::Check, 14., c_blue()))
                            }),
                    );
                }
                div()
                    .relative()
                    .child(
                        div()
                            .id(("setting-dd", ix))
                            .h(px(34.))
                            .min_w(px(200.))
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .rounded_lg()
                            .border_1()
                            .cursor_pointer()
                            .text_sm()
                            .bg(rgb(c_canvas()))
                            .border_color(rgb(if open { c_blue() } else { c_border() }))
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings_menu =
                                    if this.settings_menu.as_deref() == Some(toggle_key.as_str()) {
                                        None
                                    } else {
                                        Some(toggle_key.clone())
                                    };
                                cx.notify();
                            }))
                            .child(label)
                            .child(icon(
                                if open {
                                    IconName::ChevronUp
                                } else {
                                    IconName::ChevronDown
                                },
                                14.,
                                c_muted(),
                            )),
                    )
                    // Deferred so the list paints above the rows that follow it. The
                    // anchor sits on the trigger's top edge with a spacer over the
                    // trigger, so the list opens below it, or flips to open above
                    // it when there is no room underneath.
                    .when(open, |el| {
                        el.child(
                            div().absolute().top_0().right_0().child(
                                deferred(
                                    anchored().anchor(Anchor::TopRight).child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .pb_1()
                                            .child(div().h(px(40.)))
                                            .child(fade(
                                                div()
                                                    .min_w(px(220.))
                                                    .max_h(px(320.))
                                                    .rounded_xl()
                                                    .border_1()
                                                    .border_color(rgb(c_border()))
                                                    .bg(rgb(c_canvas()))
                                                    .shadow_lg()
                                                    .occlude()
                                                    .on_mouse_down_out(cx.listener(
                                                        |this, _, _, cx| {
                                                            this.settings_menu = None;
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .child(
                                                        div()
                                                            .id(("setting-dd-scroll", ix))
                                                            .max_h(px(320.))
                                                            .overflow_y_scroll()
                                                            .child(list),
                                                    ),
                                                SharedString::from(format!("dd-pop-{ix}")),
                                                140,
                                            )),
                                    ),
                                )
                                .with_priority(1),
                            ),
                        )
                    })
                    .into_any_element()
            }
            Ctl::Text {
                key,
                value,
                placeholder,
                set,
            } => {
                let input = self.ensure_input(&key, &value, &placeholder, set, window, cx);
                div()
                    .w(px(340.))
                    .child(Input::new(&input).bordered(true).h(px(34.)))
                    .into_any_element()
            }
            Ctl::Accent(current) => {
                let mut dots = div().flex().items_center().gap_2();
                for (j, (id, name, color)) in settings::ACCENTS.iter().enumerate() {
                    let selected = *id == current;
                    let id = id.to_string();
                    dots = dots.child(
                        div()
                            .id(("setting-accent", ix * 16 + j))
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .border_2()
                            .border_color(if selected {
                                rgb(*color).into()
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let id = id.clone();
                                settings::update(cx, move |s| s.accent = id);
                            }))
                            .child(div().size(px(18.)).rounded_full().bg(rgb(*color)))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(*name).build(window, cx)
                            }),
                    );
                }
                dots.into_any_element()
            }
            Ctl::Button(label, action) => self
                .button(("setting-btn", ix), label, false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    action(this, cx);
                    cx.notify();
                }))
                .into_any_element(),
            Ctl::Buttons(buttons) => {
                let last = buttons.len().saturating_sub(1);
                let mut group = div().flex().items_center().gap_2();
                for (j, (label, action)) in buttons.into_iter().enumerate() {
                    group = group.child(
                        self.button(("setting-btns", ix * 8 + j), label, j == last)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                action(this, cx);
                                cx.notify();
                            })),
                    );
                }
                group.into_any_element()
            }
            Ctl::Chip(text, kind) => {
                let color = match kind {
                    0 => c_green(),
                    1 => c_red(),
                    2 => c_amber(),
                    _ => c_secondary(),
                };
                div()
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(color))
                    .bg(rgb(c_canvas()))
                    .border_1()
                    .border_color(rgb(c_border()))
                    .child(text)
                    .into_any_element()
            }
            Ctl::Keys(text) => {
                let mut keys = div().flex().items_center().gap_1();
                for part in text.split(' ') {
                    keys = keys.child(
                        div()
                            .min_w(px(26.))
                            .h(px(26.))
                            .px_2()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .bg(rgb(c_canvas()))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(part.to_string()),
                    );
                }
                keys.into_any_element()
            }
            Ctl::Shortcut(id) => {
                let custom = cx.global::<AppSettings>().keybindings.clone();
                let recording = self.key_capture == Some(id);
                let changed = custom.contains_key(id);
                let keys = crate::keymap::keys_for(&custom, id);
                let cap = |text: String, accent: bool| {
                    div()
                        .min_w(px(26.))
                        .h(px(26.))
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if accent { c_blue() } else { c_border() }))
                        .bg(rgb(c_canvas()))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .child(text)
                };
                let mut shown = div().flex().items_center().gap_1();
                if recording {
                    shown = shown.child(
                        div()
                            .h(px(28.))
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(c_blue()))
                            .bg(tint(c_selected()))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_blue()))
                            .child(
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(rgb(c_blue()))
                                    .with_animation(
                                        "key-rec",
                                        Animation::new(std::time::Duration::from_millis(1000))
                                            .repeat()
                                            .with_easing(pulsating_between(0.3, 1.)),
                                        |el, t| el.opacity(t),
                                    ),
                            )
                            .child("Press the new shortcut…"),
                    );
                } else {
                    for (j, combo) in keys.iter().enumerate() {
                        if j > 0 {
                            shown = shown.child(
                                div()
                                    .px_1()
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .child("or"),
                            );
                        }
                        for part in crate::keymap::display(combo) {
                            shown = shown.child(cap(part, changed));
                        }
                    }
                }
                let note = recording.then(|| self.key_note.clone()).flatten();
                div()
                    .v_flex()
                    .items_end()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(changed && !recording, |el| {
                                el.child(
                                    self.ghost_button_sized(
                                        ("key-reset", ix),
                                        IconName::RotateCcw,
                                        "Reset to default",
                                        26.,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.reset_shortcut(Some(id), cx);
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .child(
                                div()
                                    .id(("key-edit", ix))
                                    .p(px(3.))
                                    .rounded_lg()
                                    .cursor_pointer()
                                    .border_1()
                                    .border_color(gpui_kit::transparent_black())
                                    .hover(|el| {
                                        el.border_color(rgb(c_border())).bg(tint(c_subtle()))
                                    })
                                    .tooltip(move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(if recording {
                                            "Press Esc to cancel"
                                        } else {
                                            "Click to change"
                                        })
                                        .build(window, cx)
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.key_capture = if this.key_capture == Some(id) {
                                            None
                                        } else {
                                            Some(id)
                                        };
                                        this.key_note = None;
                                        cx.notify();
                                    }))
                                    .child(shown),
                            )
                            .when(recording, |el| {
                                el.child(
                                    self.button(("key-cancel", ix), "Cancel", false)
                                        .h(px(28.))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.key_capture = None;
                                            this.key_note = None;
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .when_some(note, |el, note| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(11.5))
                                .text_color(rgb(c_amber()))
                                .child(icon(IconName::TriangleAlert, 11., c_amber()))
                                .child(note),
                        )
                    })
                    .into_any_element()
            }
            Ctl::Mcp(share, path) => {
                let reveal = path.clone();
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(share, |el, (key, on)| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child("Use in Runo")
                                .child(Switch::new(("mcp-share", ix)).checked(on).on_click(
                                    move |value, _, cx| {
                                        let (key, value) = (key.clone(), *value);
                                        settings::update(cx, move |s| {
                                            s.mcp_shared.retain(|k| *k != key);
                                            if value {
                                                s.mcp_shared.push(key);
                                            }
                                        });
                                    },
                                )),
                        )
                    })
                    .child(
                        self.button(("setting-edit", ix), "Edit", false)
                            .child(icon(IconName::Pencil, 12., c_secondary()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.edit_path(&path);
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(("setting-reveal", ix), "Reveal", false)
                            .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                    )
                    .into_any_element()
            }
            Ctl::Open(version) => div()
                .flex()
                .items_center()
                .gap_3()
                .when(!version.is_empty(), |el| {
                    el.child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .bg(rgb(c_raised()))
                            .text_xs()
                            .text_color(rgb(c_green()))
                            .child(version),
                    )
                })
                .child(icon(IconName::ChevronRight, 14., c_muted()))
                .into_any_element(),
            Ctl::Edit(path) => {
                let exists = path.exists();
                let edit = path.clone();
                div()
                    .flex()
                    .gap_2()
                    .child(
                        self.button(
                            ("setting-edit", ix),
                            if exists { "Edit" } else { "Create" },
                            false,
                        )
                        .child(icon(IconName::Pencil, 12., c_secondary()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit_path(&edit);
                            cx.notify();
                        })),
                    )
                    .when(exists, |el| {
                        el.child(
                            self.button(("setting-reveal", ix), "Reveal", false)
                                .on_click(move |_, _, cx| cx.reveal_path(&path)),
                        )
                    })
                    .into_any_element()
            }
        }
    }

    fn settings_card(
        &mut self,
        sec: Section,
        counter: &mut usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut card = div()
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_subtle()))
            .overflow_hidden();
        let count = sec.rows.len();
        for (i, r) in sec.rows.into_iter().enumerate() {
            *counter += 1;
            let control = self.control(*counter, r.ctl, window, cx);
            let open = r.open;
            card = card.child(
                div()
                    .id(("setting-row", *counter))
                    .when_some(open, |el, page| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(tint(c_active())))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.settings_page = page;
                                this.settings_search
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                cx.notify();
                            }))
                    })
                    .px_5()
                    .py_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_8()
                    .when(i + 1 < count, |el| {
                        el.border_b_1().border_color(rgb(c_border()))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(c_text()))
                                    .child(r.title),
                            )
                            .when(!r.desc.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(c_secondary()))
                                        .whitespace_normal()
                                        .child(r.desc),
                                )
                            }),
                    )
                    .child(div().flex_none().child(control)),
            );
        }
        div()
            .v_flex()
            .gap_3()
            .child(
                div()
                    .v_flex()
                    .gap(px(2.))
                    .px_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child(sec.title),
                    )
                    .when(!sec.desc.is_empty(), |el| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(rgb(c_secondary()))
                                .child(sec.desc),
                        )
                    }),
            )
            .child(card)
            .into_any_element()
    }

    fn settings_nav_item(&self, page: SettingsPage, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.settings_page == page;
        let lead: AnyElement = match page.agent() {
            Some(id) => self
                .provider_badge(agent::provider(id), 20.)
                .into_any_element(),
            None => icon(
                page.icon(),
                16.,
                if selected { c_blue() } else { c_secondary() },
            )
            .into_any_element(),
        };
        div()
            .id(SharedString::from(format!("snav-{}", page.label())))
            .h(px(34.))
            .px_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded_lg()
            .cursor_pointer()
            .text_sm()
            .when(selected, |el| {
                el.bg(tint(c_selected()))
                    .text_color(rgb(c_blue()))
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!selected, |el| {
                el.text_color(rgb(c_text()))
                    .hover(|el| el.bg(tint(c_subtle())))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_page = page;
                this.settings_menu = None;
                cx.notify();
            }))
            .child(
                div()
                    .w(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(lead),
            )
            .child(page.label())
    }

    pub(crate) fn settings_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let st = cx.global::<AppSettings>().clone();
        let data = self.discovered.clone().unwrap_or_default();
        let query = self.settings_search.read(cx).value().trim().to_lowercase();

        // Navigation
        let mut nav = div()
            .w(px(252.))
            .flex_none()
            .h_full()
            .v_flex()
            .p_3()
            .border_r_1()
            .border_color(divider())
            .bg(pane(c_sidebar()));
        let mut groups = div()
            .id("settings-nav")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .v_flex()
            .gap_4();
        for (title, pages) in SettingsPage::GROUPS.iter() {
            let group = div().v_flex().gap(px(2.)).child(
                div()
                    .px_3()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_muted()))
                    .child(title.to_uppercase()),
            );
            // Pages hang off their group like a file tree; agent pages hang off Providers.
            let mut items = div()
                .ml(px(14.))
                .pl(px(6.))
                .border_l_1()
                .border_color(rgb(c_border()))
                .v_flex()
                .gap(px(2.));
            let mut i = 0;
            while i < pages.len() {
                let page = pages[i];
                items = items.child(self.settings_nav_item(page, cx));
                // Installed agents hang off Providers.
                let children: Vec<SettingsPage> = if page == SettingsPage::Providers {
                    agent::PROVIDERS
                        .iter()
                        .filter(|p| caps::get(p.id).installed)
                        .map(|p| SettingsPage::Agent(p.id))
                        .collect()
                } else {
                    Vec::new()
                };
                if !children.is_empty() {
                    let last = children.len() - 1;
                    let mut sub = div().ml(px(24.)).v_flex().gap(px(2.));
                    for (n, child) in children.iter().enumerate() {
                        sub = sub.child(
                            div()
                                .relative()
                                // The elbow into this row.
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(-12.))
                                        .top(px(-2.))
                                        .w(px(10.))
                                        .h(px(20.))
                                        .border_l_1()
                                        .border_b_1()
                                        .border_color(rgb(c_border()))
                                        .rounded_bl(px(6.)),
                                )
                                // The trunk on to the next sibling.
                                .when(n < last, |el| {
                                    el.child(
                                        div()
                                            .absolute()
                                            .left(px(-12.))
                                            .top(px(-2.))
                                            .bottom(px(-2.))
                                            .w(px(1.))
                                            .bg(rgb(c_border())),
                                    )
                                })
                                .child(self.settings_nav_item(*child, cx)),
                        );
                    }
                    items = items.child(sub);
                }
                i += 1;
            }
            groups = groups.child(group.child(items));
        }
        nav = nav.child(groups).child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(rgb(c_border()))
                .child(
                    div()
                        .id("settings-back")
                        .h(px(38.))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded_lg()
                        .cursor_pointer()
                        .text_sm()
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.view = View::Session;
                            cx.notify();
                        }))
                        .child(icon(IconName::ArrowLeft, 16., c_secondary()))
                        .child("Back to workspace"),
                ),
        );

        // Content
        let mut counter = 0usize;
        let (title, subtitle, sections): (String, String, Vec<Section>) = if query.is_empty() {
            (
                self.settings_page.label().into(),
                self.settings_page.subtitle().into(),
                page_sections(self.settings_page, &st, &data),
            )
        } else {
            let mut found = Vec::new();
            for page in SettingsPage::all() {
                for sec in page_sections(page, &st, &data) {
                    let rows: Vec<Row> = sec
                        .rows
                        .into_iter()
                        .filter(|r| {
                            r.title.to_lowercase().contains(&query)
                                || r.desc.to_lowercase().contains(&query)
                        })
                        .collect();
                    if !rows.is_empty() {
                        found.push(Section {
                            title: format!("{}  ›  {}", page.label(), sec.title),
                            desc: String::new(),
                            rows,
                        });
                    }
                }
            }
            (
                "Search results".into(),
                format!("Matches for “{query}” across all settings."),
                found,
            )
        };
        let mut body = div()
            .max_w(px(920.))
            .mx_auto()
            .px_10()
            .py_8()
            .v_flex()
            .gap_8()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child(title.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child(subtitle),
                    ),
            );
        if sections.is_empty() {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(rgb(c_muted()))
                    .child("No settings match your search."),
            );
        }
        for sec in sections {
            body = body.child(self.settings_card(sec, &mut counter, window, cx));
        }

        let header = div()
            .h(px(52.))
            .flex_none()
            .px_8()
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(rgb(c_border()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(div().text_color(rgb(c_muted())).child("Settings"))
                    .child(icon(IconName::ChevronRight, 14., c_muted()))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_text()))
                            .child(if query.is_empty() {
                                self.settings_page.label().to_string()
                            } else {
                                "Search".to_string()
                            }),
                    ),
            )
            .child(
                div()
                    .w(px(300.))
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(tint(c_subtle()))
                    .child(icon(IconName::Search, 14., c_muted()))
                    .child(
                        Input::new(&self.settings_search)
                            .appearance(false)
                            .h(px(30.)),
                    ),
            );

        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(header)
            .child({
                // Pages cross-fade inside Settings, but opening Settings shows it at once.
                let still = self.settings_switching();
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body)
                    .animated(
                        SharedString::from(format!(
                            "settings-page-{}",
                            if query.is_empty() {
                                self.settings_page.label()
                            } else {
                                "search"
                            }
                        )),
                        Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                        move |el, d| if still { el } else { el.opacity(d) },
                    )
            });
        div()
            .size_full()
            .flex()
            .child(nav)
            .child(main)
            .into_any_element()
    }
}
