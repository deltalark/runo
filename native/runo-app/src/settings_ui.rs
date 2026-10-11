//! The Settings screen: grouped navigation, searchable pages, titled sections of rows.

use super::*;
use gpui_kit::component::switch::Switch;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    General,
    Appearance,
    Keybindings,
    Chat,
    Providers,
    Claude,
    Codex,
    OpenCode,
    MoreAgents,
    Mcp,
    Skills,
    Hooks,
    Worktrees,
}

impl SettingsPage {
    const GROUPS: [(&'static str, &'static [SettingsPage]); 3] = [
        (
            "App",
            &[
                SettingsPage::General,
                SettingsPage::Appearance,
                SettingsPage::Keybindings,
            ],
        ),
        (
            "Agents",
            &[
                SettingsPage::Chat,
                SettingsPage::Providers,
                SettingsPage::Claude,
                SettingsPage::Codex,
                SettingsPage::OpenCode,
                SettingsPage::MoreAgents,
                SettingsPage::Mcp,
                SettingsPage::Skills,
                SettingsPage::Hooks,
            ],
        ),
        ("Workspace", &[SettingsPage::Worktrees]),
    ];

    fn all() -> impl Iterator<Item = SettingsPage> {
        Self::GROUPS
            .iter()
            .flat_map(|(_, pages)| pages.iter().copied())
    }

    /// The page this one is listed under in the navigation tree.
    fn parent(self) -> Option<SettingsPage> {
        match self {
            SettingsPage::Claude
            | SettingsPage::Codex
            | SettingsPage::OpenCode
            | SettingsPage::MoreAgents => Some(SettingsPage::Providers),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Keybindings => "Keybindings",
            Self::Chat => "Chat",
            Self::Providers => "Providers",
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::MoreAgents => "More agents",
            Self::Mcp => "MCP",
            Self::Skills => "Skills",
            Self::Hooks => "Hooks",
            Self::Worktrees => "Worktrees",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::General => "Alerts, motion and how deleting works.",
            Self::Appearance => "Theme, accent color and how diffs are colored.",
            Self::Keybindings => "The shortcuts Runo responds to. ⌘ on macOS, Ctrl elsewhere.",
            Self::Chat => "How the composer behaves and what new sessions start with.",
            Self::Providers => "The agent CLIs on this machine and what each one supports.",
            Self::Claude => "Everything Runo can pass to the claude CLI, plus what it detected.",
            Self::Codex => "Everything Runo can pass to codex exec, plus what it detected.",
            Self::OpenCode => "Everything Runo can pass to opencode run, plus what it detected.",
            Self::MoreAgents => "Cursor, Gemini, Copilot, Amp, Droid, Qwen Code and Grok. Runo maps its permission levels onto each CLI's flags.",
            Self::Mcp => "MCP servers found across your agents' config files.",
            Self::Skills => "Agent skills discovered in project, personal and agent folders.",
            Self::Hooks => "Commands Claude Code runs on agent events, and the config files behind them.",
            Self::Worktrees => "Where isolated sessions and Runo's data live.",
        }
    }

    fn agent(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            Self::OpenCode => Some("opencode"),
            _ => None,
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::Keybindings => IconName::Keyboard,
            Self::Chat => IconName::MessageSquare,
            Self::Providers => IconName::Layers,
            Self::Mcp => IconName::Plug,
            Self::Skills => IconName::Sparkles,
            Self::Hooks => IconName::Webhook,
            Self::Worktrees => IconName::GitBranch,
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
    Chip(String, u32),
    Keys(String),
    /// Open the file in Runo's editor, or reveal it.
    Edit(PathBuf),
}

struct Row {
    title: String,
    desc: String,
    ctl: Ctl,
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
    }
}

fn switch_row(title: &str, desc: &str, on: bool, set: fn(&mut AppSettings, bool)) -> Row {
    row(title, desc, Ctl::Switch(on, Rc::new(set)))
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
        Ctl::Button("Re-detect".into(), Rc::new(|_, _| caps::refresh())),
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
            section("Alerts", "How Runo reaches you while you are looking somewhere else.", vec![
                switch_row("Notifications", "Show a desktop notification when an agent finishes or fails (macOS and Linux).", st.notifications, |s, v| s.notifications = v),
            ]),
            section("Behavior", "Motion and safety around destructive actions.", vec![
                switch_row("Animations", "Fade, slide and pulse effects. Turn off for instant changes.", st.animations, |s, v| s.animations = v),
                switch_row("Confirm before deleting", "Ask for a second click before deleting a session.", st.confirm_delete, |s, v| s.confirm_delete = v),
                switch_row("Remove worktree with session", "Also delete the session's git worktree when the session is deleted. Uncommitted changes in it are lost.", st.remove_worktree_on_delete, |s, v| s.remove_worktree_on_delete = v),
                switch_row("Keep the computer awake", "Hold off sleep while any agent is running (caffeinate on macOS, systemd-inhibit on Linux).", st.keep_awake, |s, v| s.keep_awake = v),
                switch_row("Group sidebar by status", "Show Pinned, Working, Needs review and Done groups instead of projects.", st.group_by_status, |s, v| s.group_by_status = v),
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
                    "Runs in the new worktree's Terminal, for example npm install. Leave empty to skip.",
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
            section("Theme", "System follows your OS appearance.", vec![
                row("Theme", "Light, dark, or match the system.", Ctl::Segment(options(&[("system", "System"), ("light", "Light"), ("dark", "Dark")]), st.theme.clone(), Rc::new(|s, v| s.theme = v))),
                row("Accent color", "Used for selection, links and active states.", Ctl::Accent(st.accent.clone())),
                row("Glass", "A translucent window over a blurred desktop, like macOS glass. Clear shows the most of what is behind.", Ctl::Segment(options(&[("off", "Off"), ("subtle", "Subtle"), ("frosted", "Frosted"), ("clear", "Clear")]), st.glass.clone(), Rc::new(|s, v| s.glass = v))),
                row("Interface font", "Runo's own font is tuned for sharp text at small sizes. System uses your operating system's font.", Ctl::Segment(options(&[("bundled", "Runo"), ("system", "System")]), st.font.clone(), Rc::new(|s, v| s.font = v))),
                row("Glass menus and dialogs", "Popups, menus and dialogs are translucent with a light rim too.", Ctl::Switch(st.glass_cards, Rc::new(|s, v| s.glass_cards = v))),
                row("Diff colors", "Colorblind uses blue and orange instead of green and red. High contrast uses stronger tints.", Ctl::Segment(options(&[("default", "Default"), ("colorblind", "Colorblind"), ("contrast", "High contrast")]), st.diff_colors.clone(), Rc::new(|s, v| s.diff_colors = v))),
            ]),
        ],
        SettingsPage::Keybindings => {
            let m = MOD_KEY.trim_end_matches('+');
            vec![
                section("Workspace", "Available everywhere in the window.", vec![
                    row("Quick open", "Jump to any session, file or command.", Ctl::Keys(format!("{m} K"))),
                    row("New session", "Open the New session form.", Ctl::Keys(format!("{m} N"))),
                    row("Settings", "Open this screen.", Ctl::Keys(format!("{m} ,"))),
                    row("Changes panel", "Show or hide the Changes, Files and Terminal panel.", Ctl::Keys(format!("{m} B"))),
                    row("Sidebar", "Show or hide the sessions sidebar.", Ctl::Keys(format!("{m} \\"))),
                    row("Stop agent", "Stop the agent running in the selected session.", Ctl::Keys(format!("{m} ."))),
                    row("Close", "Close a menu, cancel a delete, or go back to the workspace.", Ctl::Keys("Esc".into())),
                ]),
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
                switch_row("Memory server", "Give Claude Code and Codex Runo's memory tools: read the session memory, search earlier sessions, save notes.", st.memory_server, |s, v| s.memory_server = v),
            ]),
            section("New sessions", "What a session starts with before you change it in the composer.", vec![
                row("Default view", "How new sessions open. CLI shows the agent's own command line on the same conversation (Claude Code and Codex); switch any time from the session header at no token cost.", Ctl::Segment(vec![("chat".into(), "Chat".into()), ("cli".into(), "CLI".into())], st.default_view.clone(), Rc::new(|s, v| s.default_view = v))),
                row("Default agent", "Preselected in New session.", Ctl::Dropdown(agent::PROVIDERS.iter().map(|p| (p.id.to_string(), p.name.to_string())).collect(), st.default_provider.clone(), Rc::new(|s, v| s.default_provider = v))),
                row("Default permission level", "Maps to each agent's closest mode: auto edit, read-only, or full access.", Ctl::Segment(options(&[("edit", "Auto edit"), ("plan", "Plan only"), ("full", "Full access")]), st.default_mode.clone(), Rc::new(|s, v| s.default_mode = v))),
            ]),
        ],
        SettingsPage::Providers => {
            let mut rows = Vec::new();
            for p in agent::PROVIDERS.iter() {
                let path = data.provider_paths.iter().find(|(id, _)| *id == p.id).and_then(|(_, path)| path.clone());
                let caps = caps::get(p.id);
                rows.push(row(
                    p.name,
                    &path.map(|p| p.display().to_string()).unwrap_or_else(|| format!("`{}` was not found on PATH", p.bin)),
                    if caps.installed {
                        Ctl::Chip(caps.version.clone(), 0)
                    } else {
                        let pid = p.id;
                        Ctl::Button("Install…".into(), Rc::new(move |this, _| this.ask_install(pid)))
                    },
                ));
            }
            vec![
                section("Agent CLIs", "Runo drives the agent CLIs on your computer. Missing ones can be installed from here with each vendor's official installer.", rows),
                section("Detection", "Models, modes and reasoning levels come from the CLIs themselves.", vec![row(
                    "Re-detect all agents",
                    "Run detection again, for example after installing or updating a CLI.",
                    Ctl::Button("Re-detect".into(), Rc::new(|_, _| caps::refresh())),
                )]),
            ]
        }
        SettingsPage::Claude => vec![
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
        SettingsPage::Codex => vec![
            detected_section("codex"),
            launch_section("codex", st, "Any model your account supports. Empty uses the model in ~/.codex/config.toml."),
            section("Behavior", "Passed to codex exec on every run.", vec![
                agent_text("codex", st, "Config profile", "A profile from ~/.codex/config.toml (-p).", "None", |o| o.profile.clone(), |o, v| o.profile = v),
                agent_text("codex", st, "Extra directories", "Comma-separated writable folders besides the project (--add-dir).", "None", |o| o.add_dirs.clone(), |o, v| o.add_dirs = v),
                agent_choice("codex", st, "Default reasoning", "model_reasoning_effort when the composer has none selected.", std::iter::once(("".to_string(), "Default".to_string())).chain(caps::get("codex").models.first().map(|m| m.efforts.clone()).unwrap_or_default().iter().map(|c| (c.value.clone(), c.label.clone()))).collect(), |o| o.effort.clone(), |o, v| o.effort = v),
            ]),
        ],
        SettingsPage::OpenCode => vec![
            detected_section("opencode"),
            launch_section("opencode", st, "In provider/model form, for example opencode/big-pickle."),
            section("Behavior", "Passed to opencode run on every run.", vec![
                agent_text("opencode", st, "Default agent", "Run with a named OpenCode agent (--agent). Plan only uses the plan agent when this is empty.", "None", |o| o.sub_agent.clone(), |o, v| o.sub_agent = v),
                agent_text("opencode", st, "Model variant", "Provider-specific reasoning effort such as high, max or minimal (--variant).", "None", |o| o.variant.clone(), |o, v| o.variant = v),
                row("Show thinking", "Include thinking blocks in the output (--thinking).", Ctl::Switch(st.agent("opencode").thinking, Rc::new(|s, v| s.agents.entry("opencode".into()).or_default().thinking = v))),
            ]),
        ],
        SettingsPage::MoreAgents => {
            let mut sections = Vec::new();
            for p in agent::PROVIDERS.iter().skip(3) {
                let mut detected = detected_section(p.id);
                detected.title = p.name.to_string();
                sections.push(detected);
                let mut launch = launch_section(p.id, st, "Passed with the CLI's model flag.");
                launch.title = format!("{} launch", p.name);
                sections.push(launch);
            }
            sections
        }
        SettingsPage::Mcp => {
            let mut sections = Vec::new();
            for provider in ["Claude", "Codex"] {
                let rows: Vec<Row> = data
                    .mcp
                    .iter()
                    .filter(|m| m.provider == provider)
                    .map(|m| {
                        row(
                            &m.name,
                            &format!("{} · {}{}", m.transport, if m.enabled { "Configured" } else { "Disabled" }, if m.detail.is_empty() { String::new() } else { format!(" · {}", m.detail) }),
                            Ctl::Edit(m.path.clone()),
                        )
                    })
                    .collect();
                if !rows.is_empty() {
                    sections.push(section(&format!("{provider} servers"), &format!("Read from the {provider} config file. Reveal shows where to edit them."), rows));
                }
            }
            let files: Vec<Row> = data
                .files
                .iter()
                .filter(|(label, _)| label.contains("MCP") || label.contains("Codex"))
                .map(|(label, path)| row(label, &path.display().to_string(), Ctl::Edit(path.clone())))
                .collect();
            sections.push(section("Config files", "Edit servers directly. Files that do not exist yet are created when you save.", files));
            if data.mcp.is_empty() {
                sections.insert(0, section("MCP servers", "Servers are read from ~/.claude.json and ~/.codex/config.toml.", vec![row("No servers found", "Add one with your agent's own commands and it will appear here.", Ctl::Chip("0".into(), 3))]));
            }
            sections
        }
        SettingsPage::Skills => {
            let mut sections = Vec::new();
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
            if sections.is_empty() {
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
        SettingsPage::Worktrees => vec![section("Location", "Isolated sessions get their own git worktree and branch.", vec![
            row("Worktree folder", "Created next to each repository as .runo-worktrees/<repo>/<branch>.", Ctl::Chip(".runo-worktrees".into(), 3)),
            row("Runo data", "Projects and sessions are saved here. Settings live next to it in settings.json.", Ctl::Chip("~/.runo/state.json".into(), 3)),
        ])],
    }
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
                                    .font_weight(FontWeight::SEMIBOLD)
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
                    .when(open, |el| {
                        el.child(fade(
                            div()
                                .absolute()
                                .top(px(40.))
                                .right_0()
                                .min_w(px(220.))
                                .max_h(px(320.))
                                .rounded_xl()
                                .border_1()
                                .border_color(rgb(c_border()))
                                .bg(rgb(c_canvas()))
                                .shadow_lg()
                                .occlude()
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.settings_menu = None;
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .id(("setting-dd-scroll", ix))
                                        .max_h(px(320.))
                                        .overflow_y_scroll()
                                        .child(list),
                                ),
                            SharedString::from(format!("dd-pop-{ix}")),
                            140,
                        ))
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
                    .font_weight(FontWeight::SEMIBOLD)
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
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(part.to_string()),
                    );
                }
                keys.into_any_element()
            }
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
            card = card.child(
                div()
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
                                    .font_weight(FontWeight::SEMIBOLD)
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
                            .font_weight(FontWeight::BOLD)
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
                    .font_weight(FontWeight::SEMIBOLD)
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
                    .font_weight(FontWeight::SEMIBOLD)
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
                let children: Vec<SettingsPage> = pages[i + 1..]
                    .iter()
                    .copied()
                    .take_while(|p| p.parent() == Some(page))
                    .collect();
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
                    i += children.len();
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
                            .font_weight(FontWeight::BOLD)
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
                            .font_weight(FontWeight::SEMIBOLD)
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
            .child(fade(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
                SharedString::from(format!(
                    "settings-page-{}",
                    if query.is_empty() {
                        self.settings_page.label()
                    } else {
                        "search"
                    }
                )),
                220,
            ));
        div()
            .size_full()
            .flex()
            .child(nav)
            .child(main)
            .into_any_element()
    }
}
