//! The Settings screen: grouped navigation, searchable pages, titled sections of rows.

use super::*;
use crate::cleanup;
use crate::connections::{self, Connection};
use gpui_kit::component::switch::Switch;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    /// Where Settings opens: a card per area with what stands out in it.
    Overview,
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
    Phone,
    Safety,
}

/// The tabs across the top of Settings. Each holds one or more pages, picked from the
/// row under the tabs when there is more than one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    Overview,
    General,
    Look,
    Agents,
    Connections,
    Safety,
    Keys,
}

impl SettingsTab {
    const ALL: [SettingsTab; 7] = [
        SettingsTab::Overview,
        SettingsTab::General,
        SettingsTab::Look,
        SettingsTab::Agents,
        SettingsTab::Connections,
        SettingsTab::Safety,
        SettingsTab::Keys,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::General => "General",
            Self::Look => "Look",
            Self::Agents => "Agents",
            Self::Connections => "Connections",
            Self::Safety => "Safety",
            Self::Keys => "Keys",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Overview => IconName::LayoutGrid,
            Self::General => IconName::Settings,
            Self::Look => IconName::Palette,
            Self::Agents => IconName::Layers,
            Self::Connections => IconName::Server,
            Self::Safety => IconName::ShieldCheck,
            Self::Keys => IconName::Keyboard,
        }
    }

    /// The tab's pages, in order; the first is where the tab opens.
    fn pages(self) -> &'static [SettingsPage] {
        match self {
            Self::Overview => &[SettingsPage::Overview],
            Self::General => &[SettingsPage::General, SettingsPage::Worktrees],
            Self::Look => &[SettingsPage::Appearance, SettingsPage::Terminal],
            Self::Agents => &[
                SettingsPage::Providers,
                SettingsPage::Chat,
                SettingsPage::Mcp,
                SettingsPage::Skills,
                SettingsPage::Hooks,
            ],
            Self::Connections => &[SettingsPage::Connections, SettingsPage::Phone],
            Self::Safety => &[SettingsPage::Safety],
            Self::Keys => &[SettingsPage::Keybindings],
        }
    }

    fn of(page: SettingsPage) -> Self {
        match page {
            SettingsPage::Agent(_) => Self::Agents,
            page => Self::ALL
                .into_iter()
                .find(|t| t.pages().contains(&page))
                .unwrap_or(Self::Overview),
        }
    }
}

impl SettingsPage {
    /// Every page with a tab of its own or a place under one, in order.
    pub(crate) fn pages() -> impl Iterator<Item = SettingsPage> {
        SettingsTab::ALL
            .into_iter()
            .flat_map(|t| t.pages().iter().copied())
    }

    /// Every page, including each agent's, for searching.
    fn all() -> impl Iterator<Item = SettingsPage> {
        Self::pages().chain(agent::PROVIDERS.iter().map(|p| SettingsPage::Agent(p.id)))
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
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
            Self::Phone => "Phone",
            Self::Safety => "Safety",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Overview => "Everything at a glance. Pick a card or a tab to change it.",
            Self::General => "Alerts, the built-in browser, the Runo assistant, motion and how deleting works.",
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
            Self::Phone => "Follow every session from the Runo mobile app and answer it: permission requests, plans, questions, follow-ups and Stop.",
            Self::Safety => "Keep agents from running away with time, money or tokens.",
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
            Self::Overview => IconName::LayoutGrid,
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
            Self::Phone => IconName::Smartphone,
            Self::Safety => IconName::ShieldCheck,
            _ => IconName::Bot,
        }
    }
}

type SetBool = Rc<dyn Fn(&mut AppSettings, bool)>;
type SetStr = Rc<dyn Fn(&mut AppSettings, String)>;
type Action = Rc<dyn Fn(&mut RunoApp, &mut Context<RunoApp>)>;
type Listener = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

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
    /// The window glass styles as cards, each showing a small window over a desktop.
    Glass(String),
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
    /// A text field that isn't a setting; a form reads it when it is saved.
    Field {
        key: String,
        value: String,
        placeholder: String,
    },
    /// Like `Field`, several lines tall.
    Area {
        key: String,
        value: String,
        placeholder: String,
    },
    /// This text as a QR code, for a phone's camera.
    Qr(String),
    /// A note (such as a size) and a checkbox; clicking runs the action.
    Check(bool, String, Action),
}

/// The form for adding or editing one of Runo's MCP servers.
pub(crate) struct McpForm {
    /// The server's name before this edit; None when adding.
    was: Option<String>,
    draft: crate::mcp::OwnDraft,
    error: Option<String>,
}

/// Field keys of the MCP server form.
const MCP_FORM: [&str; 4] = [
    "mcp.form.name",
    "mcp.form.command",
    "mcp.form.url",
    "mcp.form.env",
];

fn mcp_form_section(form: &McpForm) -> Section {
    let field = |key: &str, value: &str, placeholder: &str| Ctl::Field {
        key: key.into(),
        value: value.into(),
        placeholder: placeholder.into(),
    };
    let d = &form.draft;
    let mut rows = vec![
        row(
            "Name",
            "How agents see it, as the prefix of its tools.",
            field(MCP_FORM[0], &d.name, "my-server"),
        ),
        row(
            "Command",
            "The program and its arguments, for a server that runs on this computer.",
            field(
                MCP_FORM[1],
                &d.command,
                "npx -y @modelcontextprotocol/server-filesystem ~/code",
            ),
        ),
        row(
            "URL",
            "Or the address of a server that runs elsewhere.",
            field(MCP_FORM[2], &d.url, "https://example.com/mcp"),
        ),
        row(
            "Environment",
            "For a command: one KEY=value per line.",
            Ctl::Area {
                key: MCP_FORM[3].into(),
                value: d.env.clone(),
                placeholder: "API_KEY=…".into(),
            },
        ),
    ];
    rows.push(row(
        form.error.as_deref().unwrap_or(""),
        "",
        Ctl::Buttons(vec![
            (
                "Cancel".into(),
                Rc::new(|this: &mut RunoApp, _: &mut Context<RunoApp>| this.mcp_form = None)
                    as Action,
            ),
            (
                "Save".into(),
                Rc::new(|this: &mut RunoApp, cx: &mut Context<RunoApp>| this.save_mcp_form(cx))
                    as Action,
            ),
        ]),
    ));
    let title = match &form.was {
        Some(name) => format!("Edit {name}"),
        None => "Add a server".to_string(),
    };
    section(
        &title,
        "Saved to Runo's list. Headers and other fields not shown here are kept.",
        rows,
    )
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
pub(crate) fn tilde(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match home.and_then(|h| path.strip_prefix(h).ok().map(|p| p.display().to_string())) {
        Some(rest) => format!("~/{rest}"),
        None => path.display().to_string(),
    }
}

/// Automatic, then each Chromium-based browser installed, by path. A chosen browser that
/// has since been removed stays listed so the setting still shows.
fn browser_options(chosen: &str) -> Vec<(String, String)> {
    let mut out = vec![("auto".to_string(), "Automatic".to_string())];
    let mut paths = browser_engine::installed();
    if chosen != "auto" && !chosen.is_empty() && !paths.iter().any(|p| p.as_os_str() == chosen) {
        paths.push(std::path::PathBuf::from(chosen));
    }
    out.extend(paths.into_iter().map(|p| {
        let name = browser_engine::browser_name(&p);
        (p.to_string_lossy().to_string(), name)
    }));
    out
}

/// The agents the assistant can answer with: the default, then each one installed, and
/// the chosen one even if it has since been removed.
fn assistant_agents(st: &AppSettings) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), "Same as new sessions".to_string())];
    out.extend(
        agent::PROVIDERS
            .iter()
            .filter(|p| {
                p.id == st.assistant_agent || agent::available(p.bin, &st.agent(p.id).binary)
            })
            .map(|p| (p.id.to_string(), p.name.to_string())),
    );
    out
}

/// The models the assistant's agent offers. The first means the agent's default, or on
/// Claude, a fast small model.
fn assistant_models(st: &AppSettings) -> Vec<(String, String)> {
    let i = crate::assistant::assistant_provider(st);
    let first = if agent::PROVIDERS[i].id == "claude" {
        "Fast default · Haiku"
    } else {
        "Default"
    };
    let mut out = vec![(String::new(), first.to_string())];
    out.extend(
        crate::agent_pick::AgentPick::models(i)
            .into_iter()
            .filter(|c| !c.value.is_empty())
            .map(|c| (c.value, c.label)),
    );
    if !st.assistant_model.is_empty() && !out.iter().any(|(v, _)| *v == st.assistant_model) {
        out.push((st.assistant_model.clone(), st.assistant_model.clone()));
    }
    out
}

/// A concurrency setting's choices: Auto, showing what it comes to on this computer, the
/// given numbers, and `none` for no limit.
fn limit_options(auto: usize, numbers: &[usize], none: &str) -> Vec<(String, String)> {
    let auto = format!(
        "Auto ({auto} at a time on {})",
        hardware::describe(hardware::here())
    );
    std::iter::once(("auto".to_string(), auto))
        .chain(numbers.iter().map(|n| (n.to_string(), n.to_string())))
        .chain([("0".to_string(), none.to_string())])
        .collect()
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
    if crate::signin::get(pid) == Some(false) {
        rows.push(row(
            "Signed out",
            &agent::sign_in_help(pid),
            Ctl::Button(
                "Sign in".into(),
                Rc::new(move |this, cx| this.sign_in_here(pid, cx)),
            ),
        ));
    }
    rows.push(row(
        "Re-detect options",
        "Ask the CLI again after updating or signing in. Menus in the composer refresh automatically.",
        Ctl::Button("Re-detect".into(), Rc::new(|_, _| install::redetect())),
    ));
    section("Detected from the CLI", "Runo asks each CLI what it supports instead of hard-coding it, so new models and modes appear on their own.", rows)
}

/// Every copy of an agent's CLI on this computer, to pick the one Runo runs, when there
/// is more than one.
fn copies_section(pid: &'static str, title: &str) -> Option<Section> {
    let p = agent::PROVIDERS.iter().find(|p| p.id == pid)?;
    let copies = crate::copies::get(pid);
    if copies.len() < 2 {
        return None;
    }
    let using = crate::copies::in_use(p, &copies).map(|c| c.real.clone());
    let first = crate::updater::resolve(p.bin).and_then(|r| std::fs::canonicalize(r).ok());
    let newest = crate::copies::newest(&copies).map(|c| c.version.clone());
    let caps = caps::get(pid);
    let mut rows = Vec::new();
    if let Some((old, new)) = crate::copies::shadow(
        &copies,
        copies.iter().find(|c| Some(&c.real) == using.as_ref()),
    ) {
        let path = new.path.to_string_lossy().into_owned();
        rows.push(row(
            "An older copy is in use",
            &format!(
                "Runo runs {} {} at {}, but {} at {} is newer.",
                p.name,
                old.version,
                tilde(&old.path),
                new.version,
                tilde(&new.path)
            ),
            Ctl::Buttons(vec![(
                "Use newest".into(),
                Rc::new(move |this: &mut RunoApp, cx: &mut Context<RunoApp>| {
                    this.use_copy(pid, path.clone(), cx)
                }) as Action,
            )]),
        ));
    }
    for copy in &copies {
        let in_use = Some(&copy.real) == using.as_ref();
        let mut desc = tilde(&copy.path);
        if copy.link {
            desc.push_str(&format!(" → {}", tilde(&copy.real)));
        }
        if first.as_ref() == Some(&copy.real) {
            desc.push_str(" · first on PATH");
        }
        if let Some(app) = &copy.owner {
            desc.push_str(&format!(" · comes with {app}, which updates it"));
        }
        if in_use {
            desc.push_str(" · Runo runs this one");
        }
        let outdated = newest
            .as_ref()
            .is_some_and(|n| crate::alerts::is_newer(n, &copy.version))
            || crate::alerts::is_newer(&caps.client_version, &copy.version);
        let mut buttons: Vec<(String, Action)> = Vec::new();
        if copy.link {
            let c = copy.clone();
            buttons.push((
                "Remove this copy…".into(),
                Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| {
                    this.ask_remove_copy(pid, &c)
                }) as Action,
            ));
        }
        if in_use && outdated {
            buttons.push((
                "Update".into(),
                Rc::new(move |this: &mut RunoApp, cx: &mut Context<RunoApp>| {
                    this.update_agent(pid, cx)
                }) as Action,
            ));
        }
        if !in_use {
            let path = copy.path.to_string_lossy().into_owned();
            buttons.push((
                "Use this".into(),
                Rc::new(move |this: &mut RunoApp, cx: &mut Context<RunoApp>| {
                    this.use_copy(pid, path.clone(), cx)
                }) as Action,
            ));
        }
        let version = if copy.version.is_empty() {
            "Unknown version".to_string()
        } else {
            format!("{} {}", p.name, copy.version)
        };
        rows.push(row(
            &version,
            &desc,
            if buttons.is_empty() {
                Ctl::Chip("In use".into(), 0)
            } else {
                Ctl::Buttons(buttons)
            },
        ));
    }
    Some(section(title, "Every copy found on your PATH and in the folders installers use. Each launch runs the one picked here.", rows))
}

/// The agent's models, from its own list, with which ones tested as working and a
/// button to make one the default.
fn models_section(pid: &'static str, st: &AppSettings) -> Option<Section> {
    let caps = caps::get(pid);
    crate::caps::all_models(&caps).next()?;
    let name = agent::provider(pid).name;
    let current = st.agent(pid).model_override;
    let today = crate::store::today();
    let mut rows = Vec::new();
    if crate::modelcheck::can_test(pid) {
        rows.push(row(
            "Test models",
            &format!("Sends \"Reply with the single word OK.\" to each model below with the {name} Runo runs, one at a time, and marks which ones work."),
            if crate::modelcheck::testing(pid) {
                Ctl::Chip("Testing…".into(), 2)
            } else {
                Ctl::Button("Test".into(), Rc::new(move |this, cx| this.test_models(pid, cx)))
            },
        ));
    }
    if pid == "codex" {
        rows.push(row(
            "Also save to ~/.codex/config.toml",
            "When you pick a default here, also set model in Codex's own config, so codex in a terminal uses it too. Only that line changes, after a backup.",
            Ctl::Switch(st.agent("codex").save_to_config, Rc::new(|s, v| s.agents.entry("codex".into()).or_default().save_to_config = v)),
        ));
    }
    if !current.is_empty() {
        rows.push(row(
            &format!("Default: {current}"),
            "Runo passes it on every run where the session picks no model, over the CLI's own config.",
            Ctl::Button("Use the CLI's default".into(), Rc::new(move |this, cx| this.set_default_model(pid, String::new(), cx))),
        ));
    }
    let all: Vec<&caps::ModelInfo> = crate::caps::all_models(&caps).collect();
    for m in all.iter().take(crate::fixes::MODELS_SHOWN) {
        let mut desc = vec![m.choice.value.clone()];
        if caps
            .hidden_models
            .iter()
            .any(|h| h.choice.value == m.choice.value)
        {
            desc.push("hidden from the CLI's menu".into());
        }
        let note = caps::model_note(pid, m, &caps.version, &today);
        if !note.is_empty() {
            desc.push(note);
        }
        if let Some(v) = crate::modelcheck::verdict(pid, &m.choice.value) {
            desc.push(v.label().into());
        }
        let value = m.choice.value.clone();
        rows.push(row(
            &m.choice.label,
            &desc.join(" · "),
            if current == value {
                Ctl::Chip("Default".into(), 0)
            } else {
                Ctl::Button(
                    "Use as default".into(),
                    Rc::new(move |this, cx| this.set_default_model(pid, value.clone(), cx)),
                )
            },
        ));
    }
    let more = all.len().saturating_sub(crate::fixes::MODELS_SHOWN);
    Some(section(
        "Models",
        &if more > 0 {
            format!("From {name}'s own model list; {more} more aren't shown. Type any of them under Launch.")
        } else {
            format!(
                "From {name}'s own model list, including models it hides and ones being retired."
            )
        },
        rows,
    ))
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
            section("Runo assistant", "A Runo mark in the corner of the window that answers questions about Runo from its built-in guide and what's happening in the app, and asks before changing anything.", vec![
                switch_row("Show the Runo assistant", "Click the mark or press the Ask Runo shortcut, then ask how to do something. Off, the mark is hidden.", st.assistant, |s, v| s.assistant = v),
                row("Answers with", "An agent writes answers from the guide and what's happening in the app, and can offer to do things for you; it counts toward usage. Guide only answers offline with the best matching article.", Ctl::Dropdown(options(&[("agent", "An agent"), ("guide", "Guide only")]), st.assistant_answers.clone(), Rc::new(|s, v| s.assistant_answers = v))),
                row("Assistant agent", "Which agent answers the assistant's questions.", Ctl::Dropdown(assistant_agents(st), st.assistant_agent.clone(), Rc::new(|s, v| { if s.assistant_agent != v { s.assistant_model.clear(); } s.assistant_agent = v; }))),
                row("Assistant model", "The model it answers on. A small, fast one keeps answers quick and cheap.", Ctl::Dropdown(assistant_models(st), st.assistant_model.clone(), Rc::new(|s, v| s.assistant_model = v))),
                switch_row("Nudges", "While the assistant is closed, its mark offers the next step when a run finishes, a race is ready to compare or the clash radar finds something.", st.assistant_nudges, |s, v| s.assistant_nudges = v),
            ]),
            section("Browser", "Show pages in the Browser tab with a headless copy of a Chromium-based browser on this machine (Chrome, Edge, Brave, Arc, Vivaldi, Opera and others), using its own profile.", vec![
                switch_row("Built-in browser", "Adds the Browser tab: open your dev server beside the conversation and click and type in it. Off, the tab is hidden.", st.browser_enabled, |s, v| s.browser_enabled = v),
                row("Browser", "Automatic uses your default browser when it's Chromium-based, otherwise the first one found. Safari and Firefox can't be driven this way. A change applies the next time Runo starts the browser.", Ctl::Dropdown(browser_options(&st.browser), st.browser.clone(), Rc::new(|s, v| s.browser = v))),
                match browser_engine::find_chromium() {
                    Some(path) => row("Uses", &tilde(&path), Ctl::Chip(browser_engine::browser_name(&path), 0)),
                    None => row("Uses", "No Chromium-based browser found. Install Chrome, Edge, Brave or another, or set CHROME_PATH.", Ctl::Chip("Not found".into(), 1)),
                },
            ]),
            section("Alerts", "How Runo reaches you while you are looking somewhere else.", {
                let mut rows = vec![
                    switch_row("Notifications", "Show a desktop notification when an agent finishes, fails or needs you. Clicking it opens the session.", st.notifications, |s, v| s.notifications = v),
                ];
                if cfg!(target_os = "macos") {
                    rows.push(switch_row("Show badge on Dock icon", "A red count on Runo's Dock icon of what needs you: agents waiting, runs you haven't seen, Crew replies and reviews. It clears as you open them.", st.dock_badge, |s, v| s.dock_badge = v));
                    rows.push(switch_row("Show Runo in the menu bar", "A Runo glyph by the clock counts what needs you and what's working. Click it to allow or stop agents and Ask Runo without opening the window.", st.menu_bar, |s, v| s.menu_bar = v));
                    if st.menu_bar {
                        rows.push(switch_row("Keep running in the menu bar", "⌘Q and closing the window hide Runo to the menu bar, and agents keep working. Quit from the menu bar to quit.", st.run_in_background, |s, v| s.run_in_background = v));
                    }
                }
                rows
            }),
            section("Behavior", "Motion, sleep and how the sidebar groups sessions.", vec![
                switch_row("Animations", "Fade, slide and pulse effects. Turn off for instant changes.", st.animations, |s, v| s.animations = v),
                switch_row("Keep the computer awake", "Hold off sleep while any agent is running (caffeinate on macOS, systemd-inhibit on Linux).", st.keep_awake, |s, v| s.keep_awake = v),
                switch_row("Group sidebar by status", "Show Needs you, Running, Ready to review and Done groups instead of projects.", st.group_by_status, |s, v| s.group_by_status = v),
                switch_row("Show deleted threads", "Keep a line in Crew's chat where a thread you deleted was.", st.show_deleted_threads, |s, v| s.show_deleted_threads = v),
                switch_row("Thread titles beside numbers", "In Crew's chat, #34 shows the thread's title next to it. The number alone still opens the thread, and shows its title on hover.", st.thread_ref_titles, |s, v| s.thread_ref_titles = v),
            ]),
            section("Rail", "Pin places out on the rail so they're one click away. The rest stay in its ⋯ menu.", vec![
                switch_row("Playbooks", "Saved tasks you run in any project.", st.pinned("Playbooks"), |s, v| s.pin("Playbooks", v)),
                switch_row("History", "Past Claude Code and Codex conversations on this machine.", st.pinned("History"), |s, v| s.pin("History", v)),
                switch_row("Notes", "Each project's notes.", st.pinned("Notes"), |s, v| s.pin("Notes", v)),
                switch_row("Inbox", "What needs you, also under the bell in the top bar.", st.pinned("Inbox"), |s, v| s.pin("Inbox", v)),
            ]),
            section("Updates", "Runo looks for a newer release on GitHub and offers to download it. Nothing installs without you.", vec![
                switch_row("Check for Runo updates", "Checks at start and every few hours.", st.check_runo_updates, |s, v| s.check_runo_updates = v),
                {
                    let (found, running) = crate::applog::last_check();
                    let desc = if running {
                        "Checking GitHub…".to_string()
                    } else if found.is_empty() {
                        format!("You have Runo {}.", env!("CARGO_PKG_VERSION"))
                    } else {
                        found
                    };
                    row("Check for updates", &desc, Ctl::Button("Check now".into(), Rc::new(|this, cx| this.check_runo_now(cx))))
                },
                row(
                    "Logs and crash reports",
                    &format!("Each launch writes a log, and crashes leave a report, in {}. Copy diagnostics copies your version, system, agent CLIs, settings without secrets and the end of the log, to paste into a bug report.", tilde(&crate::applog::logs_dir())),
                    Ctl::Buttons(vec![
                        ("Show".into(), Rc::new(|_, cx| cx.reveal_path(&crate::applog::logs_dir())) as Action),
                        (
                            "Copy diagnostics".into(),
                            Rc::new(|this: &mut RunoApp, cx: &mut Context<RunoApp>| {
                                cx.write_to_clipboard(ClipboardItem::new_string(crate::applog::diagnostics()));
                                this.info = Some(("Copied diagnostics. Look them over before you share them.".into(), std::time::Instant::now()));
                            }) as Action,
                        ),
                    ]),
                ),
            ]),
            section("Git", "How Runo commits for you.", vec![
                switch_row("Credit the agent in commits", "Adds a line such as \"Made with Claude Code in Runo\" to commits made from the Changes panel.", st.attribution, |s, v| s.attribution = v),
            ]),
            section("Design", "Bring work in from other design tools.", vec![
                row(
                    "Figma access token",
                    "Lets Design import frames from Figma. Make one in Figma under Settings > Security > Personal access tokens, with read access to files. It stays on this computer.",
                    Ctl::Text {
                        key: "figma_token".into(),
                        value: st.figma_token.clone(),
                        placeholder: "figd_…".into(),
                        set: Rc::new(|s, v| s.figma_token = v.trim().to_string()),
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
        SettingsPage::Overview => Vec::new(),
        SettingsPage::Safety => vec![
            section("Limits", "Once one is reached, new turns wait until you raise it or send anyway.", vec![
                row("Flag a quiet agent after", "An agent that prints nothing for this long gets a card with Stop. A long build or test run can be quiet too.", Ctl::Dropdown(options(&[("5", "5 minutes"), ("10", "10 minutes"), ("20", "20 minutes"), ("60", "1 hour"), ("0", "Never")]), st.stuck_after_mins.to_string(), Rc::new(|s, v| s.stuck_after_mins = v.parse().unwrap_or(10)))),
                row("Spending limit per session", &format!("Counts the cost agents report. Once a session reaches it, new turns wait until you raise the limit or send anyway. {}", agent::cost_note()), Ctl::Dropdown(options(&[("0", "No limit"), ("1", "$1"), ("5", "$5"), ("10", "$10"), ("25", "$25"), ("50", "$50"), ("100", "$100")]), st.session_limit_usd.to_string(), Rc::new(|s, v| s.session_limit_usd = v.parse().unwrap_or(0)))),
                row("Spending limit per day", "Across every session. New turns wait once today's reported cost reaches it; agents that report no cost don't count toward it, so Turns per day and Tokens per day below hold them instead.", Ctl::Dropdown(options(&[("0", "No limit"), ("5", "$5"), ("10", "$10"), ("25", "$25"), ("50", "$50"), ("100", "$100"), ("250", "$250")]), st.daily_limit_usd.to_string(), Rc::new(|s, v| s.daily_limit_usd = v.parse().unwrap_or(0)))),
                row("Turns per session", "Works for every agent. Once a session has had this many turns, its next one waits until you raise the limit or send anyway.", Ctl::Dropdown(options(&[("0", "No limit"), ("10", "10"), ("20", "20"), ("50", "50"), ("100", "100"), ("200", "200")]), st.session_turn_limit.to_string(), Rc::new(|s, v| s.session_turn_limit = v.parse().unwrap_or(0)))),
                row("Turns per day", "Across every session and every agent. New turns wait once this many have started today.", Ctl::Dropdown(options(&[("0", "No limit"), ("25", "25"), ("50", "50"), ("100", "100"), ("200", "200"), ("500", "500")]), st.daily_turn_limit.to_string(), Rc::new(|s, v| s.daily_turn_limit = v.parse().unwrap_or(0)))),
                row("Tokens per day", "Across every session. Counts the input and output tokens agents report, not cached ones; agents that report no tokens, such as Grok, don't count toward it. New turns wait once today's count reaches it.", Ctl::Dropdown(options(&[("0", "No limit"), ("1000", "1M"), ("5000", "5M"), ("10000", "10M"), ("25000", "25M"), ("50000", "50M"), ("100000", "100M")]), st.daily_token_limit_k.to_string(), Rc::new(|s, v| s.daily_token_limit_k = v.parse().unwrap_or(0)))),
                row("Agents running at once", "Turns past this many wait for a free slot, shown as Waiting for a slot, and start in order.", Ctl::Dropdown(options(&[("0", "No cap"), ("1", "1"), ("2", "2"), ("3", "3"), ("4", "4"), ("6", "6"), ("8", "8")]), st.max_running.to_string(), Rc::new(|s, v| s.max_running = v.parse().unwrap_or(0)))),
                row("Checks running at once", "The checks Runo runs after each turn (set in a project's Edit dialog) across every session. Past this many, the others show Waiting for another check and start in order as running ones finish. Auto allows one set per 4 cores and per 6 GB of memory, from 1 to 6, since each is usually a full build and test run.", Ctl::Dropdown(limit_options(hardware::checks_auto(hardware::here()), &[1, 2, 4], "No limit"), st.checks_at_once.clone(), Rc::new(|s, v| s.checks_at_once = v))),
                switch_row("Run checks at low priority", "Checks and everything they start get low CPU and disk priority, so the app, the agents and your own tools stay quick while they run. Off runs them at normal priority.", st.checks_low_priority, |s, v| s.checks_low_priority = v),
                row("Checks after each turn", "What Runo runs after a turn that changed files, for projects that don't pick their own in their Edit dialog. Quick runs each language's fast check, such as cargo check, a typecheck or lint script, go vet or ruff, and leaves tests to CI. Full runs the tests too. Commands typed in a project stay as they are.", Ctl::Segment(options(&[("quick", "Quick"), ("full", "Full")]), st.check_level.clone(), Rc::new(|s, v| s.check_level = v))),
                row("Crew threads running at once", "Threads Crew starts past this many wait for a slot and start in order as others finish; Start now on a waiting thread lets it go ahead. Sessions you start yourself never wait for it. Each thread is a full agent, often with subagents of its own, so a few at once keep a plan's usage window from running out. Auto allows one per 3 cores and per 4 GB of memory, from 2 to 8, for the builds and tests the agents run.", Ctl::Dropdown(limit_options(hardware::crew_auto(hardware::here()), &[1, 2, 3, 4, 6, 8], "No cap"), st.crew_threads.clone(), Rc::new(|s, v| s.crew_threads = v))),
                row("Hold threads and playbooks past", "Once an agent's plan has used this share of its 5-hour or weekly window, new turns of Crew threads and playbook runs wait until it resets, so what you start yourself still has room. A used-up window always holds every turn of that agent until it resets, whatever this says.", Ctl::Dropdown(options(&[("0", "Never"), ("70", "70%"), ("80", "80%"), ("90", "90%"), ("95", "95%")]), st.plan_hold_percent.to_string(), Rc::new(|s, v| s.plan_hold_percent = v.parse().unwrap_or(0)))),
            ]),
            section("Permissions", "What agents may do without asking you first.", vec![
                row("Default permission level", "Maps to each agent's closest mode: auto edit, read-only, or full access.", Ctl::Segment(options(&[("edit", "Auto edit"), ("plan", "Plan only"), ("full", "Full access")]), st.default_mode.clone(), Rc::new(|s, v| s.default_mode = v))),
                switch_row("Ask before agents start playbooks", "When an agent asks Runo to start a playbook, show what it may do and wait for you. Off starts it right away.", st.confirm_agent_playbooks, |s, v| s.confirm_agent_playbooks = v),
            ]),
            section("Deleting", "Safety around actions that can't be undone.", vec![
                switch_row("Confirm before deleting", "Ask in a popup before deleting a session.", st.confirm_delete, |s, v| s.confirm_delete = v),
                switch_row("Remove worktree with session", "Also delete the session's git worktree when the session is deleted. Uncommitted changes in it are lost.", st.remove_worktree_on_delete, |s, v| s.remove_worktree_on_delete = v),
            ]),
        ],
        SettingsPage::Appearance => vec![
            section("Colors", "System follows your OS appearance.", vec![
                row("Theme", "Light, dark, or match the system.", Ctl::Segment(options(&[("system", "System"), ("light", "Light"), ("dark", "Dark")]), st.theme.clone(), Rc::new(|s, v| s.theme = v))),
                row("Color theme", "Each theme has a light and a dark version; Theme above picks which one shows.", Ctl::Dropdown(crate::themes::THEMES.iter().map(|t| (t.id.to_string(), t.name.to_string())).collect(), st.color_theme.clone(), Rc::new(|s, v| s.color_theme = v))),
                row("Accent color", "Used for selection, links and active states.", Ctl::Accent(st.accent.clone())),
            ]),
            section("Background", "A fine pattern behind your work, warming into your accent at the top-left.", vec![
                row("Pattern", "Dots, lines or small marks on one grid across the window.", Ctl::Dropdown(crate::pattern::Pattern::ALL.iter().map(|p| (p.id().to_string(), p.label().to_string())).collect(), st.pattern.clone(), Rc::new(|s, v| s.pattern = v))),
                switch_row("On the content", "The page and the terminal, where you read and work.", st.pattern_content, |s, v| s.pattern_content = v),
                switch_row("On the side panels", "The sidebar and the panels beside the content, such as Changes and Files.", st.pattern_panels, |s, v| s.pattern_panels = v),
                switch_row("On the frame", "The top bar, the rail and the status bar round your work. With glass on and all three areas on, it also shows through the panels.", st.pattern_frame, |s, v| s.pattern_frame = v),
            ]),
            section("Glass", "A translucent window over a blurred desktop, like macOS glass.", vec![
                row("Window glass", "", Ctl::Glass(st.glass.clone())),
                row("Menus and dialogs too", "Popups, menus and dialogs are translucent with a light rim.", Ctl::Switch(st.glass_cards, Rc::new(|s, v| s.glass_cards = v))),
            ]),
            section("Type", "", vec![
                row("Interface font", "Runo's own font is tuned for sharp text at small sizes. System uses your operating system's font.", Ctl::Segment(options(&[("bundled", "Runo"), ("system", "System")]), st.font.clone(), Rc::new(|s, v| s.font = v))),
                row("Font smoothing", "Thickens text strokes, like older macOS. Off keeps text crisp and light. Applies after you restart Runo.", Ctl::Switch(st.font_smoothing, Rc::new(|s, v| s.font_smoothing = v))),
            ]),
            section("Code and conversation", "How diffs, replies, errors and code use the space in a session.", vec![
                row("Diff colors", "Colorblind uses blue and orange instead of green and red. High contrast uses stronger tints.", Ctl::Segment(options(&[("default", "Default"), ("colorblind", "Colorblind"), ("contrast", "High contrast")]), st.diff_colors.clone(), Rc::new(|s, v| s.diff_colors = v))),
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
                row(
                    "Quick commands",
                    "The Terminal's Run menu. Write Name = command, separated by semicolons.",
                    Ctl::Text {
                        key: "quick_commands".into(),
                        value: st.quick_commands.replace('\n', "; "),
                        placeholder: "Test = cargo test; Lint = cargo clippy".into(),
                        set: Rc::new(|s, v| s.quick_commands = v.split(';').map(str::trim).collect::<Vec<_>>().join("\n")),
                    },
                ),
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
                row("Wait for permission answers", "With Runo tools on, Claude Code asks you mid-turn before using a tool it isn't allowed, and waits this long for Allow or Deny before carrying on without it. Turns on servers keep waiting while Runo is closed, so you can answer when it reconnects.", Ctl::Dropdown(options(&[("2", "2 minutes"), ("5", "5 minutes"), ("15", "15 minutes"), ("60", "1 hour"), ("480", "8 hours"), ("0", "Wait indefinitely")]), st.approval_wait_mins.to_string(), Rc::new(|s, v| s.approval_wait_mins = v.parse().unwrap_or(5)))),
            ]),
            section("New sessions", "What a session starts with before you change it in the composer.", vec![
                row("Default view", "How new sessions open. CLI shows the agent's own command line on the same conversation (Claude Code and Codex); switch any time from the session header at no token cost.", Ctl::Segment(vec![("chat".into(), "Chat".into()), ("cli".into(), "CLI".into())], st.default_view.clone(), Rc::new(|s, v| s.default_view = v))),
                row("Default agent", "Preselected in New session.", Ctl::Dropdown(agent::PROVIDERS.iter().map(|p| (p.id.to_string(), p.name.to_string())).collect(), st.default_provider.clone(), Rc::new(|s, v| s.default_provider = v))),
            ]),
            section("Playbooks", "Saved tasks that run as their own sessions, by hand, on a schedule or when an agent asks.", vec![
                row("Scheduled runs to keep", "Older runs of each scheduled playbook are deleted, with their worktrees. Pinned and unread runs stay.", Ctl::Dropdown(options(&[("5", "5"), ("10", "10"), ("20", "20"), ("50", "50"), ("0", "All")]), st.playbook_keep_runs.to_string(), Rc::new(|s, v| s.playbook_keep_runs = v.parse().unwrap_or(10)))),
                switch_row("Catch up on missed runs", "When a scheduled playbook came due while Runo was closed, run it once when Runo opens. Off skips it until its next time.", st.playbook_catch_up, |s, v| s.playbook_catch_up = v),
            ].into_iter().chain(cfg!(any(target_os = "macos", target_os = "linux", windows)).then(|| {
                switch_row("Run while Runo is closed", "A background job (launchd on macOS, a systemd user timer on Linux, Task Scheduler on Windows) checks every 5 minutes and runs due playbooks in their own worktree, after the setup command, with the agent's settings and the playbook's permissions. The runs show up as sessions when Runo opens. Playbooks on a connection still wait for Runo.", st.playbooks_while_closed, |s, v| s.playbooks_while_closed = v)
            })).chain(st.playbooks_while_closed.then(crate::pbrunner::job_note).flatten().map(|note| {
                row("Background job", &note, Ctl::Chip("Not set up".into(), 1))
            })).collect()),
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
                    let mut desc = match &path {
                        Some(path) => match crate::shellenv::found_via(path) {
                            Some(via) => format!("{} · via {via}", tilde(path)),
                            None => tilde(path),
                        },
                        None => String::new(),
                    };
                    if let Some((old, new)) = crate::copies::shadowed(p) {
                        desc.push_str(&format!(" · Runs {}, but {} is on this computer", old.version, new.version));
                    }
                    match crate::signin::get(p.id) {
                        Some(true) => desc.push_str(" · Signed in"),
                        Some(false) => desc.push_str(&format!(" · {}", agent::sign_in_help(p.id))),
                        None => {}
                    }
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
                sections.push(section("Installed", "Pick an agent to change its model, flags and environment. Installed agents also have a page of their own in the tabs above.", installed));
            }
            for p in agent::PROVIDERS.iter() {
                sections.extend(copies_section(p.id, &format!("{} copies", p.name)));
            }
            if !available.is_empty() {
                sections.push(section("More agents", "Install any of these with the vendor's official installer, or point Runo at a copy you already have. Once found it appears in the list above and gets a page of its own.", available));
            }
            sections
        }
        SettingsPage::Agent("claude") => vec![
            detected_section("claude"),
        ].into_iter().chain(copies_section("claude", "Copies on this computer")).chain(models_section("claude", st)).chain(vec![
            launch_section("claude", st, "Model alias or full name, used when the session has none selected."),
            section("Behavior", "Passed to claude on every run.", vec![
                agent_text("claude", st, "Append to system prompt", "Extra instructions added to Claude's system prompt (--append-system-prompt).", "None", |o| o.append_prompt.clone(), |o, v| o.append_prompt = v),
                agent_text("claude", st, "Allowed tools", "Comma-separated rules that never ask, for example Read,Edit,Bash(git diff:*).", "None", |o| o.allowed_tools.clone(), |o, v| o.allowed_tools = v),
                agent_text("claude", st, "Denied tools", "Comma-separated rules that are always blocked.", "None", |o| o.disallowed_tools.clone(), |o, v| o.disallowed_tools = v),
                agent_text("claude", st, "Extra directories", "Comma-separated folders Claude may access besides the project (--add-dir).", "None", |o| o.add_dirs.clone(), |o, v| o.add_dirs = v),
                agent_text("claude", st, "Fallback model", "Used automatically when the main model is overloaded (--fallback-model).", "None", |o| o.fallback_model.clone(), |o, v| o.fallback_model = v),
                agent_text("claude", st, "Subagent model", "The model Claude's subagents run on, such as sonnet or haiku, while the session keeps its own. A smaller one makes searches and reviews it hands off much cheaper. Empty lets each subagent use the session's model.", "Same as the session", |o| o.subagent_model.clone(), |o, v| o.subagent_model = v.trim().to_string()),
                agent_choice("claude", st, "Default effort", "How much thinking Claude does when the composer has none selected.", std::iter::once(("".to_string(), "Default".to_string())).chain(caps::get("claude").efforts.iter().map(|c| (c.value.clone(), c.label.clone()))).collect(), |o| o.effort.clone(), |o, v| o.effort = v),
            ]),
        ]).collect(),
        SettingsPage::Agent("codex") => vec![
            detected_section("codex"),
        ].into_iter().chain(copies_section("codex", "Copies on this computer")).chain(models_section("codex", st)).chain(vec![
            launch_section("codex", st, "Any model your account supports. Empty uses the model in ~/.codex/config.toml."),
            section("Behavior", "Passed to codex exec on every run.", vec![
                agent_text("codex", st, "Config profile", "A profile from ~/.codex/config.toml (-p).", "None", |o| o.profile.clone(), |o, v| o.profile = v),
                agent_text("codex", st, "Extra directories", "Comma-separated writable folders besides the project (--add-dir).", "None", |o| o.add_dirs.clone(), |o, v| o.add_dirs = v),
                agent_choice("codex", st, "Default reasoning", "model_reasoning_effort when the composer has none selected.", std::iter::once(("".to_string(), "Default".to_string())).chain(caps::get("codex").models.first().map(|m| m.efforts.clone()).unwrap_or_default().iter().map(|c| (c.value.clone(), c.label.clone()))).collect(), |o| o.effort.clone(), |o, v| o.effort = v),
            ]),
        ]).collect(),
        SettingsPage::Agent("opencode") => vec![
            detected_section("opencode"),
        ].into_iter().chain(copies_section("opencode", "Copies on this computer")).chain(models_section("opencode", st)).chain(vec![
            launch_section("opencode", st, "In provider/model form, for example opencode/big-pickle."),
            section("Behavior", "Passed to opencode run on every run.", vec![
                agent_text("opencode", st, "Default agent", "Run with a named OpenCode agent (--agent). Plan only uses the plan agent when this is empty.", "None", |o| o.sub_agent.clone(), |o, v| o.sub_agent = v),
                agent_text("opencode", st, "Model variant", "Provider-specific reasoning effort such as high, max or minimal (--variant).", "None", |o| o.variant.clone(), |o, v| o.variant = v),
                row("Show thinking", "Include thinking blocks in the output (--thinking).", Ctl::Switch(st.agent("opencode").thinking, Rc::new(|s, v| s.agents.entry("opencode".into()).or_default().thinking = v))),
            ]),
        ]).collect(),
        SettingsPage::Agent(id) => std::iter::once(detected_section(id))
            .chain(copies_section(id, "Copies on this computer"))
            .chain(models_section(id, st))
            .chain([launch_section(id, st, "Passed with the CLI's model flag.")])
            .collect(),
        SettingsPage::Mcp => {
            let mut sections = Vec::new();
            let shared = data
                .mcp
                .iter()
                .filter(|m| st.mcp_shared.contains(&crate::mcp::key(m)))
                .count();
            sections.push(section(
                "Use across agents",
                "Each agent loads its own servers. Turn on Use in Runo to hand a server from any app to the other agents' sessions too. Grok takes none.",
                vec![row(
                    "Shared servers",
                    if shared == 0 {
                        "None yet. Switch one on below."
                    } else {
                        "Passed to every new run of an agent that doesn't already have them."
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
                    let draft = crate::mcp::OwnDraft::from_spec(&m.name, &m.spec);
                    row(
                        &m.name,
                        &if m.transport == "unknown" {
                            "Not set up yet. Edit it to give a command or a URL.".to_string()
                        } else {
                            format!("{} · {}{}", m.transport, if m.enabled { "On" } else { "Disabled" }, if m.detail.is_empty() { String::new() } else { format!(" · {}", m.detail) })
                        },
                        Ctl::Buttons(vec![
                            ("Edit".into(), Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| this.open_mcp_form(Some(draft.clone()))) as Action),
                            ("Remove".into(), Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| {
                                if let Err(e) = crate::mcp::remove_own(&path, &name) {
                                    this.fail("mcp", format!("Couldn't remove {name}: {e}"));
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
                "Give a command and arguments, or a URL. Edit the file for headers and other fields.",
                Ctl::Buttons(vec![
                    ("Edit file".into(), Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| this.edit_path(&add_path)) as Action),
                    ("Add…".into(), Rc::new(|this: &mut RunoApp, _: &mut Context<RunoApp>| this.open_mcp_form(None)) as Action),
                ]),
            ));
            sections.push(section(
                "Runo's servers",
                &format!("Given to every agent's sessions except Grok's, on this computer and on connections. Kept in {}.", tilde(&own_path)),
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
                            Some(Err(e)) => this.fail("settings", format!("Couldn't create a skill: {e}")),
                            None => this.fail("settings", "Couldn't find your home folder.".into()),
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
        SettingsPage::Phone => phone_sections(st),
        SettingsPage::Worktrees => vec![section("Location", "Isolated sessions get their own git worktree and branch.", vec![
            row("Worktree folder", "Created next to each repository as .runo-worktrees/<repo>/<branch>.", Ctl::Chip(".runo-worktrees".into(), 3)),
            row("Runo data", "Projects are saved in state.json and each session in its own file under sessions. Settings live next to them in settings.json.", Ctl::Chip(tilde(&store::data_dir()), 3)),
            switch_row("Remove build output when a thread is done", "Once a thread is resolved, archived or merged, delete the build and dependency folders git ignores in its worktree (target, node_modules, .venv, dist, build and the like). The worktree, its code and branch stay, so it can be opened and built again. Skipped while anything runs there.", st.remove_build_output, |s, v| s.remove_build_output = v),
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

/// One row per agent Cove can keep the login of: signed in on that server or not, with
/// Sign in or Sign out. Agents not installed here show only once signed in there.
/// Set up: install or update Cove on the server over SSH, open the tunnel and pair.
fn server_setup_row(c: &Connection) -> Row {
    use crate::coveserver::Step;
    let id = c.id;
    let paired = !c.api_key.trim().is_empty();
    let set_up = |label: &str| {
        Ctl::Button(
            label.into(),
            Rc::new(move |this: &mut RunoApp, cx: &mut Context<RunoApp>| {
                this.set_up_cove_server(id, cx)
            }),
        )
    };
    match crate::coveserver::step(id) {
        Some(Step::Running(text)) => row("Set up", &text, Ctl::Chip("Working…".into(), 1)),
        Some(Step::NeedsPassword) => row(
            "Set up",
            "sudo on the server asks for a password. Continue opens a terminal window where you type it once; Runo does the rest and carries on here.",
            Ctl::Button(
                "Continue in Terminal".into(),
                Rc::new(move |this, cx| this.set_up_cove_in_terminal(id, cx)),
            ),
        ),
        Some(Step::Failed(e)) => row("Set up", &e, set_up("Try again")),
        _ if paired => row(
            "Set up",
            "Set up and paired. Runo keeps the SSH tunnel open by itself. Set up again updates Cove there.",
            set_up("Set up again"),
        ),
        _ => row(
            "Set up",
            "Checks the SSH login, installs Cove there (or updates it), opens a tunnel and pairs. Then sign the agents in below.",
            set_up("Set up"),
        ),
    }
}

fn agent_login_rows(c: &Connection) -> Vec<Row> {
    let id = c.id;
    let known = crate::cove::logins(c);
    if let Some(Err(e)) = &known {
        return vec![row("Agent logins", e, Ctl::Chip("Unavailable".into(), 0))];
    }
    let logins = known.and_then(Result::ok);
    crate::agent::PROVIDERS
        .iter()
        .filter(|p| crate::cove::keeps_login(p.id))
        .filter_map(|p| {
            let login = logins
                .as_ref()
                .and_then(|l| l.iter().find(|l| l.agent == p.id));
            let signed_in = login.is_some_and(|l| l.signed_in);
            if !signed_in && !crate::caps::get(p.id).installed {
                return None;
            }
            let provider = p.id;
            let title = format!("{} login", p.name);
            Some(match (&logins, signed_in) {
                (None, _) => row(&title, "Checking the server…", Ctl::Chip("…".into(), 0)),
                (Some(_), true) => row(
                    &title,
                    "Signed in on this server. Every session's VM here gets this login, and tokens the agent refreshes come back.",
                    Ctl::Button(
                        "Sign out".into(),
                        Rc::new(move |this, cx| this.sign_out_on_connection(id, provider, cx)),
                    ),
                ),
                (Some(_), false) if crate::cove::has_local_login(provider) => row(
                    &title,
                    &format!(
                        "Not signed in on this server. This Mac is signed in to {}: copy its login there, or sign in there with another account.",
                        p.name
                    ),
                    Ctl::Buttons(vec![
                        (
                            "Sign in".into(),
                            Rc::new(move |this, cx| this.sign_in_on_connection(id, provider, cx)),
                        ),
                        (
                            "Use this Mac's login".into(),
                            Rc::new(move |this, cx| this.import_login_to(id, provider, cx)),
                        ),
                    ]),
                ),
                (Some(_), false) => row(
                    &title,
                    &format!(
                        "Not signed in on this server. Sign in once in a terminal there, with {}'s own login, and every session's VM here uses it.",
                        p.name
                    ),
                    Ctl::Button(
                        "Sign in".into(),
                        Rc::new(move |this, cx| this.sign_in_on_connection(id, provider, cx)),
                    ),
                ),
            })
        })
        .collect()
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
                rows.push(conn_text(
                    c,
                    "SSH login",
                    "A Linux server or VPS you can ssh into with a key, such as ubuntu@203.0.113.7. Runo installs Cove there, keeps a tunnel to it open and pairs by itself. Empty for Cove on this Mac.",
                    "ubuntu@203.0.113.7",
                    c.ssh.clone(),
                    |c, v| c.ssh = v,
                ));
                if c.ssh.trim().is_empty() {
                    rows.push(conn_text(c, "Server", "Empty for Cove on this machine, which shares the session folder with the VM. A URL (http://my-vps:7457) uses a Cove server elsewhere; the session's files are synced to it.", "Cove on this machine", c.url.clone(), |c, v| c.url = v));
                    if c.remote() {
                        rows.push(conn_text(
                            c,
                            "Pairing code",
                            "Run cove pair on the server and type the code it prints. Runo gets a key of its own, which cove keys rm there revokes.",
                            "ABCD-EFGH",
                            c.pair_code.clone(),
                            |c, v| c.pair_code = v,
                        ));
                        let paired = match connections::pair_result(id) {
                            Some((_, text)) => text,
                            None if c.api_key.trim().is_empty() => "Not paired yet.".into(),
                            None => "Runo has a key for this server.".into(),
                        };
                        rows.push(row(
                            "Pair",
                            &paired,
                            Ctl::Button(
                                "Pair".into(),
                                Rc::new(move |this, cx| this.pair_connection(id, cx)),
                            ),
                        ));
                        rows.push(conn_text(
                            c,
                            "API key",
                            "Filled in by pairing. Or run cove key on the server and paste it here.",
                            "",
                            c.api_key.clone(),
                            |c, v| c.api_key = v,
                        ));
                    }
                } else {
                    rows.push(conn_text(
                        c,
                        "Key file",
                        "Empty uses your SSH agent and ~/.ssh/config.",
                        "~/.ssh/id_ed25519",
                        c.identity.clone(),
                        |c, v| c.identity = v,
                    ));
                    rows.push(server_setup_row(c));
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
                rows.push(conn_choice(c, "When idle", "Pause or stop a session's VM when nothing has run in it for a while. It carries on with the next message.", options(&[("0", "Keep running"), ("15", "After 15 min"), ("30", "After 30 min"), ("60", "After 1 hour"), ("240", "After 4 hours")]), c.idle_stop.to_string(), |c, v| c.idle_stop = v.parse().unwrap_or(0)));
                if c.idle_stop > 0 {
                    rows.push(row(
                        "Idle VMs",
                        "Pause keeps the VM's memory and running programs, so it picks up exactly where it was, and on a Linux server it uses no memory while paused. Stop shuts it down; the next message boots it again.",
                        Ctl::Segment(
                            options(&[("pause", "Pause"), ("stop", "Stop")]),
                            if c.idle_pause { "pause" } else { "stop" }.into(),
                            Rc::new(move |s, v| {
                                if let Some(c) = s.connections.iter_mut().find(|c| c.id == id) {
                                    c.idle_pause = v == "pause";
                                }
                            }),
                        ),
                    ));
                }
            }
        }
        if c.kind == connections::Kind::Cove {
            rows.extend(agent_login_rows(c));
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
    let setup = crate::cove::setup_status();
    let busy = crate::cove::setting_up();
    // While installing or starting, and after a failure, say what happened.
    let said = |default: &str| match &setup {
        Some((None, text)) | Some((Some(false), text)) => text.clone(),
        _ => default.to_string(),
    };
    let mut cove_rows = vec![if !installed {
        row(
            "Cove",
            &said("Not installed. Cove gives each session its own Linux VM, on this Mac or on your server. Install sets it up here in one step (vfkit and the guest agent included) and starts it at login."),
            if busy {
                Ctl::Chip("Installing…".into(), 1)
            } else {
                Ctl::Buttons(vec![
                    (
                        "Learn more".into(),
                        Rc::new(|_, cx| cx.open_url("https://github.com/deltalark/cove#readme")),
                    ),
                    (
                        "Install".into(),
                        Rc::new(|this, cx| this.set_up_cove(false, cx)),
                    ),
                ])
            },
        )
    } else if crate::cove::available() {
        row(
            "Cove",
            &match &setup {
                Some((Some(true), text)) => text.clone(),
                _ => "Installed, and its server is running.".into(),
            },
            Ctl::Chip("Ready".into(), 2),
        )
    } else {
        row(
            "Cove",
            &said("Installed, but its server isn't running."),
            if busy {
                Ctl::Chip("Starting…".into(), 1)
            } else {
                Ctl::Button(
                    "Start".into(),
                    Rc::new(|this, cx| this.set_up_cove(true, cx)),
                )
            },
        )
    }];
    let install_line = crate::cove::server_install_command();
    cove_rows.push(row(
        "Run Cove on a server",
        "For sessions that keep going while your Mac sleeps. Add server, type its SSH login (such as ubuntu@203.0.113.7) and press Set up: Runo installs Cove there, keeps an SSH tunnel to it open, pairs, and then helps you sign the agents in. Nothing new is opened on the server.",
        Ctl::Buttons(vec![
            (
                "Copy install command".into(),
                Rc::new(move |_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(install_line.clone()))
                }),
            ),
            (
                "Add server".into(),
                Rc::new(|this, cx| this.add_cove_server(cx)),
            ),
        ]),
    ));
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
        "When the agent tries something it isn't allowed to (Claude Code) and nobody answers its permission request in time, or Runo's tools are off, the turn stops there and shows Allow and retry, instead of carrying on without it for hours.",
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

fn phone_sections(st: &AppSettings) -> Vec<Section> {
    let base = store::data_dir();
    let phones = phone::phones(&base);
    let listening = match phone::bridge() {
        phone::Bridge::On { bound, .. } if bound.ip().is_loopback() => row(
            "Listening on",
            "Only this computer can reach it: no Tailscale address was found. Start Tailscale, then turn the bridge off and on, or choose Local network.",
            Ctl::Chip(bound.to_string(), 2),
        ),
        phone::Bridge::On { bound, .. } if bound.ip().is_unspecified() => row(
            "Listening on",
            "Every network this computer is on, unencrypted.",
            Ctl::Chip(bound.to_string(), 2),
        ),
        phone::Bridge::On { bound, .. } => row(
            "Listening on",
            "This computer's Tailscale address, which only devices signed in to your tailnet reach.",
            Ctl::Chip(bound.to_string(), 0),
        ),
        phone::Bridge::Failed(error) => {
            row("Listening on", &error, Ctl::Chip("Not running".into(), 1))
        }
        phone::Bridge::Off => row(
            "Listening on",
            "Nothing while the bridge is off.",
            Ctl::Chip("Off".into(), 3),
        ),
    };
    let mut out = vec![section(
        "Phone bridge",
        "Runs inside Runo while it is open, the same API as runo --phone. Actions from the phone are carried out here as if you typed them.",
        vec![
            switch_row(
                "Phone bridge",
                "Lets paired phones reach this computer. Stays on across launches.",
                st.phone_bridge,
                |s, v| s.phone_bridge = v,
            ),
            row(
                "Reachable from",
                "Tailscale keeps the connection private and encrypted; without it only this computer can reach the bridge. Local network listens on every network over plain HTTP, where anyone on the same Wi-Fi can read a phone's key.",
                Ctl::Segment(
                    options(&[
                        ("tailscale", "Tailscale"),
                        ("everywhere", "Local network (unencrypted)"),
                    ]),
                    if st.phone_everywhere { "everywhere" } else { "tailscale" }.into(),
                    Rc::new(|s, v| s.phone_everywhere = v == "everywhere"),
                ),
            ),
            listening,
        ],
    )];

    let mut pairing = Vec::new();
    if let Some(shown) = phone::shown(&phones) {
        let title = format!("Pairing link for {}", shown.name);
        let hide: Action = Rc::new(|_, _| phone::show(None));
        match phone::link_for(&shown) {
            Some(link) => {
                let copied = link.clone();
                pairing.push(row(
                    &title,
                    &link,
                    Ctl::Buttons(vec![
                        ("Hide".into(), hide),
                        (
                            "Copy".into(),
                            Rc::new(move |this: &mut RunoApp, cx: &mut Context<RunoApp>| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                                this.info = Some((
                                    "Copied the pairing link.".into(),
                                    std::time::Instant::now(),
                                ));
                            }) as Action,
                        ),
                    ]),
                ));
                pairing.push(row(
                    "Scan it with the Runo app",
                    "Or open the link on the phone. Anyone with it can use this bridge as you, so remove the phone below if it leaks.",
                    Ctl::Qr(link),
                ));
            }
            None => pairing.push(row(
                &title,
                "Turn the bridge on to see it: the link carries the address the bridge listens on.",
                Ctl::Buttons(vec![("Hide".into(), hide)]),
            )),
        }
    }
    pairing.push(row(
        "Pair a new phone",
        "Makes a link with a key of its own, so each phone can be removed without unpairing the others.",
        Ctl::Button(
            "Pair a new phone".into(),
            Rc::new(|_, _| {
                let phone = phone::pair(&store::data_dir());
                phone::show(Some(phone.token));
            }),
        ),
    ));
    out.push(section(
        "Pair a phone",
        "Scan the code or open the link with the Runo mobile app.",
        pairing,
    ));

    let mut paired: Vec<Row> = phones
        .iter()
        .map(|p| {
            let (show, gone, name) = (p.token.clone(), p.token.clone(), p.name.clone());
            let when = p.paired_on();
            let desc = if when.is_empty() {
                "Paired before each phone had its own key.".to_string()
            } else {
                format!("Paired {when}")
            };
            row(
                &p.name,
                &desc,
                Ctl::Buttons(vec![
                    (
                        "Remove".into(),
                        Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| {
                            phone::unpair(&store::data_dir(), &gone);
                            this.info = Some((
                                format!("Removed {name}. Its link no longer works."),
                                std::time::Instant::now(),
                            ));
                        }) as Action,
                    ),
                    (
                        "Show link".into(),
                        Rc::new(move |_: &mut RunoApp, _: &mut Context<RunoApp>| {
                            phone::show(Some(show.clone()))
                        }) as Action,
                    ),
                ]),
            )
        })
        .collect();
    if paired.is_empty() {
        paired.push(row(
            "No phones paired",
            "Pair one above.",
            Ctl::Chip("0".into(), 3),
        ));
    }
    out.push(section(
        "Paired phones",
        &format!(
            "Removing a phone revokes only its key; its next request is refused. Kept in {}.",
            tilde(&base.join("phone.json"))
        ),
        paired,
    ));
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
                // An access token stays hidden as it's typed.
                .masked(key.ends_with("_token"))
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

    /// A text field that isn't bound to a setting, made once per key.
    fn ensure_field(
        &mut self,
        key: &str,
        value: &str,
        placeholder: &str,
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
        self.settings_inputs.insert(key.to_string(), entity.clone());
        entity
    }

    fn ensure_area(
        &mut self,
        key: &str,
        value: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some(entity) = self.settings_areas.get(key) {
            return entity.clone();
        }
        let entity = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(placeholder.to_string())
                .default_value(value.to_string())
        });
        self.settings_areas.insert(key.to_string(), entity.clone());
        entity
    }

    /// Open the MCP server form, filled in from `draft` when editing.
    fn open_mcp_form(&mut self, draft: Option<crate::mcp::OwnDraft>) {
        for key in MCP_FORM {
            self.settings_inputs.remove(key);
            self.settings_areas.remove(key);
        }
        self.mcp_form = Some(McpForm {
            was: draft.as_ref().map(|d| d.name.clone()),
            draft: draft.unwrap_or_default(),
            error: None,
        });
    }

    fn save_mcp_form(&mut self, cx: &mut Context<Self>) {
        let field = |this: &Self, key: &str| {
            this.settings_inputs
                .get(key)
                .map(|e| e.read(cx).value().to_string())
                .unwrap_or_default()
        };
        let draft = crate::mcp::OwnDraft {
            name: field(self, MCP_FORM[0]),
            command: field(self, MCP_FORM[1]),
            url: field(self, MCP_FORM[2]),
            env: self
                .settings_areas
                .get(MCP_FORM[3])
                .map(|e| e.read(cx).value().to_string())
                .unwrap_or_default(),
        };
        let Some(form) = self.mcp_form.as_mut() else {
            return;
        };
        match crate::mcp::save_own(&crate::mcp::own_path(), form.was.as_deref(), &draft) {
            Ok(()) => {
                self.mcp_form = None;
                self.open_settings();
            }
            Err(error) => {
                // The bad line may hold a secret value, so it isn't repeated.
                let shown = if error.contains("KEY=value") {
                    "an environment line isn't KEY=value"
                } else {
                    error.as_str()
                };
                crate::applog::warn("mcp", format!("could not save the MCP server: {shown}"));
                form.draft = draft;
                form.error = Some(error);
            }
        }
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
                let mut group = crate::shell::seg_track();
                for (j, (value, label)) in opts.into_iter().enumerate() {
                    let selected = value == current;
                    let set = set.clone();
                    group = group.child(
                        crate::shell::seg_choice(
                            ("setting-seg", ix * 16 + j),
                            format!("setting-seg-{ix}-{j}"),
                            selected,
                        )
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
                                                    .border_color(divider())
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
            Ctl::Field {
                key,
                value,
                placeholder,
            } => {
                let input = self.ensure_field(&key, &value, &placeholder, window, cx);
                div()
                    .w(px(340.))
                    .child(Input::new(&input).bordered(true).h(px(34.)))
                    .into_any_element()
            }
            Ctl::Area {
                key,
                value,
                placeholder,
            } => {
                let area = self.ensure_area(&key, &value, &placeholder, window, cx);
                div()
                    .w(px(340.))
                    .child(Textarea::new(&area).bordered(true).h(px(90.)))
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
            Ctl::Glass(current) => {
                let mut cards = div().flex().gap(px(10.));
                for (j, (value, name, note, alpha)) in [
                    ("off", "Off", "Solid", 1.),
                    ("subtle", "Subtle", "A hint of the desktop", 0.82),
                    ("frosted", "Frosted", "Blurred", 0.6),
                    ("clear", "Clear", "The most see-through", 0.38),
                ]
                .into_iter()
                .enumerate()
                {
                    let on = value == current;
                    let key = format!("setting-glass-{ix}-{j}");
                    let window_fill = |c: u32| {
                        let mut fill = rgb(c);
                        fill.a = alpha;
                        fill
                    };
                    let bar = |w: f32| {
                        div()
                            .h(px(5.))
                            .w(relative(w))
                            .rounded_full()
                            .bg(tint(c_active()))
                    };
                    cards = cards.child(
                        div()
                            .id(("setting-glass", ix * 16 + j))
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap(px(8.))
                            .p(px(7.))
                            .rounded(px(11.))
                            .cursor_pointer()
                            .bg(tint(c_canvas()))
                            .border_1()
                            .border_color(if on { rgb(c_blue()) } else { divider() })
                            .when(!on, |el| el.hover(|el| el.bg(tint(c_active()))))
                            .when(on, |el| el.child(crate::shell::accent_bracket(key, 11.)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                settings::update(cx, move |s| s.glass = value.into());
                            }))
                            .child(
                                // A desktop: the wallpaper's colours, with a window over it.
                                div()
                                    .relative()
                                    .h(px(72.))
                                    .rounded(px(7.))
                                    .overflow_hidden()
                                    .bg(linear_gradient(
                                        120.,
                                        linear_color_stop(rgb(0x5a3d8a), 0.),
                                        linear_color_stop(rgb(0xd9875a), 1.),
                                    ))
                                    .child(
                                        div()
                                            .absolute()
                                            .top(px(8.))
                                            .right(px(10.))
                                            .size(px(44.))
                                            .rounded_full()
                                            .bg(rgb(0xf5d0a9)),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .inset(px(8.))
                                            .left(px(12.))
                                            .rounded(px(5.))
                                            .bg(window_fill(c_canvas()))
                                            .p(px(7.))
                                            .v_flex()
                                            .gap(px(5.))
                                            .child(bar(0.6))
                                            .child(
                                                div()
                                                    .h(px(11.))
                                                    .rounded(px(3.))
                                                    .bg(window_fill(c_active())),
                                            )
                                            .child(bar(0.8)),
                                    ),
                            )
                            .child(
                                div()
                                    .px(px(2.))
                                    .v_flex()
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(if on {
                                                c_text()
                                            } else {
                                                c_secondary()
                                            }))
                                            .child(name),
                                    )
                                    .child(div().text_xs().text_color(rgb(c_muted())).child(note)),
                            ),
                    );
                }
                cards.into_any_element()
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
            Ctl::Check(on, note, action) => div()
                .flex()
                .items_center()
                .gap_3()
                .when(!note.is_empty(), |el| {
                    el.child(div().text_xs().text_color(rgb(c_secondary())).child(note))
                })
                .child(
                    div()
                        .id(("setting-check", ix))
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .border_1()
                        .cursor_pointer()
                        .when(on, |el| {
                            el.bg(rgb(c_blue())).border_color(rgb(c_blue())).child(icon(
                                IconName::Check,
                                12.,
                                0xffffff,
                            ))
                        })
                        .when(!on, |el| {
                            el.border_color(divider())
                                .bg(rgb(c_canvas()))
                                .hover(|el| el.border_color(rgb(c_secondary())))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            action(this, cx);
                            cx.notify();
                        })),
                )
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
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(color))
                    .bg(rgb(c_canvas()))
                    .border_1()
                    .border_color(divider())
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
                            .border_color(divider())
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
                                    .animated(
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
                                    .hover(|el| el.border_color(divider()).bg(tint(c_subtle())))
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
                            .border_color(divider())
                            .bg(rgb(c_raised()))
                            .text_xs()
                            .text_color(rgb(c_green()))
                            .child(version),
                    )
                })
                .child(icon(IconName::ChevronRight, 14., c_muted()))
                .into_any_element(),
            Ctl::Qr(text) => {
                let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
                    return div().into_any_element();
                };
                // Black on white with a quiet zone, whatever the theme, so cameras read it.
                let module = 4.;
                let mut grid = div()
                    .v_flex()
                    .p(px(module * 3.))
                    .rounded_md()
                    .bg(rgb(0xffffff));
                for line in code.to_colors().chunks(code.width().max(1)) {
                    let mut runs: Vec<(bool, usize)> = Vec::new();
                    for dark in line.iter().map(|c| *c == qrcode::Color::Dark) {
                        match runs.last_mut() {
                            Some((d, n)) if *d == dark => *n += 1,
                            _ => runs.push((dark, 1)),
                        }
                    }
                    let mut strip = div().flex().h(px(module));
                    for (dark, n) in runs {
                        strip = strip.child(
                            div()
                                .w(px(module * n as f32))
                                .h_full()
                                .when(dark, |el| el.bg(rgb(0x000000))),
                        );
                    }
                    grid = grid.child(strip);
                }
                grid.into_any_element()
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
        let mut card = div().v_flex();
        for (i, r) in sec.rows.into_iter().enumerate() {
            *counter += 1;
            // A picker of previews is too wide to sit beside its title, so it goes under.
            let stacked = matches!(r.ctl, Ctl::Glass(_));
            let control = self.control(*counter, r.ctl, window, cx);
            let open = r.open;
            if i > 0 {
                card = card.child(crate::shell::card_rule());
            }
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
                    .py(px(11.))
                    .flex()
                    .when(stacked, |el| el.flex_col().items_stretch().gap_3())
                    .when(!stacked, |el| el.items_center().justify_between().gap_8())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap(px(2.))
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
                                        .text_color(rgb(c_muted()))
                                        .whitespace_normal()
                                        .child(r.desc),
                                )
                            }),
                    )
                    .child(div().flex_none().child(control)),
            );
        }
        // Wide windows put the section's name on a tab notched into the card's top left,
        // so no line holds only a title.
        let wide = window.viewport_size().width > px(1100.);
        let fill = tint(c_subtle());
        let label = div()
            .v_flex()
            .gap(px(3.))
            .when(wide, |el| {
                el.px(px(20.)).py(px(14.)).rounded_l(px(NOTCH_R)).bg(fill)
            })
            .when(!wide, |el| el.px_1())
            .child(
                div()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child(sec.title),
            )
            .when(!sec.desc.is_empty(), |el| {
                el.child(div().text_xs().text_color(rgb(c_muted())).child(sec.desc))
            });
        let card = card.flex_1().min_w_0();
        if wide {
            // Stretched, so a card shorter than its tab still reaches past the notch.
            div()
                .flex()
                .child(
                    div()
                        .w(px(230.))
                        .flex_none()
                        .v_flex()
                        .child(label)
                        .child(div().flex().justify_end().child(notch(fill))),
                )
                .child(card.rounded_tr(px(NOTCH_R)).rounded_b(px(NOTCH_R)).bg(fill))
                .into_any_element()
        } else {
            div()
                .v_flex()
                .gap(px(10.))
                .child(label)
                .child(crate::shell::card().overflow_hidden().child(card))
                .into_any_element()
        }
    }

    /// One tab across the top of Settings, chosen on a raised pill with the accent's
    /// bracket at its edge, the way the open session tab is marked.
    fn settings_tab(&self, tab: SettingsTab, cx: &mut Context<Self>) -> AnyElement {
        let on = SettingsTab::of(self.settings_page) == tab;
        let key = format!("stab-{}", tab.label());
        // An installed agent that isn't signed in is the one thing a tab calls out.
        let alert = tab == SettingsTab::Agents
            && agent::PROVIDERS
                .iter()
                .any(|p| caps::get(p.id).installed && signin::get(p.id) == Some(false));
        crate::shell::seg_choice(SharedString::from(key.clone()), key.clone(), on)
            .h(px(30.))
            .px(px(12.))
            .gap(px(6.))
            .when(on, |el| {
                el.child(crate::shell::accent_bracket(format!("{key}-rule"), 7.))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.settings_page = tab.pages()[0];
                this.settings_menu = None;
                this.settings_search
                    .update(cx, |s, cx| s.set_value("", window, cx));
                cx.notify();
            }))
            .child(icon(
                tab.icon(),
                14.,
                if on { c_text() } else { c_secondary() },
            ))
            .child(tab.label())
            .when(alert, |el| {
                el.child(div().size(px(6.)).rounded_full().bg(rgb(c_amber())))
            })
            .into_any_element()
    }

    /// The pages of the open tab as underlined tabs along the page's title line; the
    /// Agents tab lists each installed agent after Providers.
    fn settings_subnav(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let tab = SettingsTab::of(self.settings_page);
        let mut pages: Vec<SettingsPage> = tab.pages().to_vec();
        if tab == SettingsTab::Agents {
            let agents = agent::PROVIDERS
                .iter()
                .filter(|p| caps::get(p.id).installed)
                .map(|p| SettingsPage::Agent(p.id));
            pages.splice(1..1, agents);
        }
        let many = pages.len() > 1;
        pages
            .into_iter()
            .map(|page| {
                let on = self.settings_page == page;
                let lead = page.agent().map(|id| {
                    self.provider_badge(agent::provider(id), 18.)
                        .into_any_element()
                });
                page_tab(format!("ssub-{}", page.label()), lead, page.label(), on)
                    .when(many, |el| {
                        el.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings_page = page;
                                this.settings_menu = None;
                                cx.notify();
                            }))
                    })
                    .into_any_element()
            })
            .collect()
    }

    pub(crate) fn settings_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let st = cx.global::<AppSettings>().clone();
        let data = self.discovered.clone().unwrap_or_default();
        let query = self.settings_search.read(cx).value().trim().to_lowercase();

        // The header: a tab per area and the search, over the page.
        let mut tabs = crate::shell::seg_track();
        for tab in SettingsTab::ALL {
            tabs = tabs.child(self.settings_tab(tab, cx));
        }
        let header = div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(tabs)
            .child(div().flex_1())
            .child(
                div()
                    .w(px(260.))
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(9.))
                    .bg(tint(c_subtle()))
                    .child(icon(IconName::Search, 14., c_muted()))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.settings_search)
                                .appearance(false)
                                .h(px(30.)),
                        ),
                    ),
            )
            .child(
                div()
                    .id("settings-back")
                    .size(px(34.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.view = View::Session;
                        cx.notify();
                    }))
                    .child(icon(IconName::X, 16., c_secondary()))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Back to workspace  Esc")
                            .build(window, cx)
                    }),
            );

        // Content
        let overview = query.is_empty() && self.settings_page == SettingsPage::Overview;
        let mut body = div()
            .w_full()
            .max_w(px(1480.))
            .mx_auto()
            .px(px(24.))
            .pt(px(16.))
            .pb_8()
            .v_flex()
            .gap(px(20.));
        if overview {
            body = body.child(self.settings_overview(&st, &data, window, cx));
        } else {
            let mut counter = 0usize;
            let (lead, title, subtitle, sections): (AnyElement, String, String, Vec<Section>) =
                if query.is_empty() {
                    (
                        tile(SettingsTab::of(self.settings_page).icon(), 28.).into_any_element(),
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
                                    title: format!(
                                        "{}  ›  {}  ›  {}",
                                        SettingsTab::of(page).label(),
                                        page.label(),
                                        sec.title
                                    ),
                                    desc: String::new(),
                                    rows,
                                });
                            }
                        }
                    }
                    let matches: usize = found.iter().map(|s| s.rows.len()).sum();
                    (
                        tile(IconName::Search, 28.).into_any_element(),
                        "Search results".into(),
                        format!(
                            "{} for “{query}”. Change them right here.",
                            plural(matches, "match")
                        ),
                        found,
                    )
                };
            let tabs = if query.is_empty() {
                self.settings_subnav(cx)
            } else {
                vec![page_tab("ssub-search".into(), None, title, true).into_any_element()]
            };
            body = body.child(
                // One line over a rule: the page's mark, its pages as tabs, and what the
                // open one holds.
                div()
                    .flex()
                    .items_center()
                    .gap(px(22.))
                    .border_b_1()
                    .border_color(divider())
                    .child(div().pb(px(8.)).child(lead))
                    .children(tabs)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pb(px(8.))
                            .flex()
                            .justify_end()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .text_color(rgb(c_muted()))
                                    .child(subtitle),
                            ),
                    ),
            );
            if sections.is_empty() && !query.is_empty() {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(rgb(c_muted()))
                        .child("No settings match your search."),
                );
            }
            let mut sections = sections;
            if let Some(form) = self
                .mcp_form
                .as_ref()
                .filter(|_| query.is_empty() && self.settings_page == SettingsPage::Mcp)
            {
                // Just above Runo's own list.
                let at = sections
                    .iter()
                    .position(|s| s.title == "Runo's servers")
                    .unwrap_or(sections.len());
                sections.insert(at, mcp_form_section(form));
            }
            if query.is_empty() && self.settings_page == SettingsPage::Worktrees {
                sections.extend(self.cleanup_sections());
            }
            for sec in sections {
                body = body.child(self.settings_card(sec, &mut counter, window, cx));
            }
        }

        // Each page shows at once; keyed by page so each keeps its own scroll.
        let scroll = div()
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
                |el, _| el,
            );
        let page = div()
            .size_full()
            .relative()
            .v_flex()
            .child(crate::home::home_glow())
            .child(
                div()
                    .relative()
                    .flex_none()
                    .px(px(24.))
                    .pt(px(16.))
                    .v_flex()
                    .gap(px(12.))
                    .child(header),
            )
            .child(scroll);
        div()
            .size_full()
            .flex()
            .child(
                // The page paints the island's corners over its own fill.
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .relative()
                    .flex()
                    .bg(glass_fill(c_canvas()))
                    .child(page)
                    .child(crate::shell::frame_corner(
                        true,
                        true,
                        glass_fill(c_canvas()),
                    ))
                    .child(crate::shell::frame_corner(
                        true,
                        false,
                        glass_fill(c_canvas()),
                    ))
                    // No bottom-left corner: the rail's Settings tab joins the island
                    // there, level with its bottom edge.
                    .child(crate::shell::frame_corner(
                        false,
                        false,
                        glass_fill(c_canvas()),
                    )),
            )
            .into_any_element()
    }

    /// Settings > Overview: a card per area showing what is set and what needs you, each
    /// leading to its page.
    fn settings_overview(
        &mut self,
        st: &AppSettings,
        data: &Discovered,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Look: a small window in the current look, what is picked and the accent.
        let label_of = |items: &[(&str, &str)], value: &str| {
            items
                .iter()
                .find(|(v, _)| *v == value)
                .map(|(_, l)| l.to_string())
                .unwrap_or_else(|| value.to_string())
        };
        let theme = label_of(
            &[("system", "System"), ("light", "Light"), ("dark", "Dark")],
            &st.theme,
        );
        let glass = label_of(
            &[
                ("off", "Off"),
                ("subtle", "Subtle"),
                ("frosted", "Frosted"),
                ("clear", "Clear"),
            ],
            &st.glass,
        );
        let font = label_of(&[("bundled", "Runo"), ("system", "System")], &st.font);
        let fact = |name: &'static str, value: String| {
            div()
                .flex_1()
                .min_w_0()
                .v_flex()
                .px(px(10.))
                .py(px(7.))
                .rounded(px(9.))
                .bg(tint(c_subtle()))
                .child(div().text_xs().text_color(rgb(c_muted())).child(name))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_text()))
                        .child(value),
                )
        };
        let accent = self.control(9000, Ctl::Accent(st.accent.clone()), window, cx);
        let look = ov_card("ov-look", IconName::Palette, "Look", "Customize")
            .on_click(open_page(SettingsPage::Appearance, cx))
            .flex_1()
            .child(look_preview())
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(fact("Theme", theme))
                    .child(fact("Glass", glass))
                    .child(fact("Font", font)),
            )
            .child(accent);

        // Agents: each installed one with its version and whether it is signed in.
        let mut agents = ov_card("ov-agents", IconName::Layers, "Agents", "All agents")
            .on_click(open_page(SettingsPage::Providers, cx))
            .flex_1();
        let installed: Vec<&agent::Provider> = agent::PROVIDERS
            .iter()
            .filter(|p| caps::get(p.id).installed)
            .collect();
        if installed.is_empty() {
            agents = agents.child(ov_note(
                "No agent CLIs found yet. Install one from Providers.",
            ));
        }
        for (i, p) in installed.iter().take(4).enumerate() {
            let caps = caps::get(p.id);
            let mut detail = Vec::new();
            if !caps.version.is_empty() {
                detail.push(caps.version.trim_start_matches('v').to_string());
            }
            if !caps.default_model.is_empty() {
                detail.push(caps.default_model.clone());
            }
            let state = match signin::get(p.id) {
                Some(false) => status_pill("Not signed in", c_amber()),
                Some(true) => status_pill("Signed in", c_green()),
                None => status_pill("Installed", c_muted()),
            };
            agents = agents.child(ov_line(
                i > 0,
                self.provider_badge(p, 26.).into_any_element(),
                p.name.to_string(),
                detail.join(" · "),
                state,
            ));
        }

        // Connections: this machine, then each server, with how its last test went.
        let mut conns = ov_card("ov-conns", IconName::Server, "Connections", "Manage")
            .on_click(open_page(SettingsPage::Connections, cx))
            .flex_1()
            .child(ov_line(
                false,
                small_tile(IconName::Cpu),
                "This machine".into(),
                "Always available".into(),
                status_pill("Ready", c_green()),
            ));
        for c in st.connections.iter().take(3) {
            let state = match connections::test_result(c.id) {
                Some((Some(true), _)) => status_pill("Connected", c_green()),
                Some((Some(false), _)) => status_pill("Can't reach", c_red()),
                Some((None, _)) => status_pill("Testing…", c_muted()),
                None => status_pill("Not tested", c_muted()),
            };
            let glyph = if c.remote() {
                IconName::Cloud
            } else {
                IconName::Server
            };
            conns = conns.child(ov_line(
                true,
                small_tile(glyph),
                c.name.clone(),
                c.summary(),
                state,
            ));
        }

        // Safety: today against the limits, and how many agents may run at once.
        let today = store::today();
        let spent = self.state.spend.get(&today).copied().unwrap_or_default();
        let day = self.state.used_on(&today);
        let limits = settings::limits();
        let dollars = |cents: f64| format!("${:.2}", cents / 100.);
        let meter =
            |name: &'static str, used: String, of: Option<String>, share: f32| {
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child(name)
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(c_text()))
                                            .child(used),
                                    )
                                    .child(div().text_color(rgb(c_muted())).child(
                                        of.map_or("no limit".into(), |of| format!("of {of}")),
                                    )),
                            ),
                    )
                    .child(
                        div().h(px(6.)).rounded_full().bg(tint(c_subtle())).child(
                            div()
                                .h_full()
                                .w(relative(share.clamp(0., 1.)))
                                .rounded_full()
                                .bg(rgb(if share >= 0.9 { c_amber() } else { c_blue() })),
                        ),
                    )
            };
        let on_off = |on: bool, yes: &str, no: &str| if on { yes } else { no }.to_string();
        let share = |used: f64, of: f64| if of > 0. { (used / of) as f32 } else { 0. };
        let running = self.counts().running;
        let cap = settings::max_running();
        let safety = ov_card("ov-safety", IconName::ShieldCheck, "Safety", "Limits")
            .on_click(open_page(SettingsPage::Safety, cx))
            .flex_1()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .gap(px(22.))
                    .child(meter(
                        "Est. spend today",
                        dollars(spent),
                        (limits.day_cents > 0.).then(|| dollars(limits.day_cents)),
                        share(spent, limits.day_cents),
                    ))
                    .child(meter(
                        "Turns today",
                        day.turns.to_string(),
                        (limits.day_turns > 0).then(|| limits.day_turns.to_string()),
                        share(day.turns as f64, limits.day_turns as f64),
                    ))
                    .child(meter(
                        "Tokens today",
                        crate::health::token_count(day.tokens),
                        (limits.day_tokens > 0)
                            .then(|| crate::health::token_count(limits.day_tokens)),
                        share(day.tokens as f64, limits.day_tokens as f64),
                    ))
                    .child(meter(
                        "Running now",
                        running.to_string(),
                        (cap > 0).then(|| cap.to_string()),
                        share(running as f64, cap as f64),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(fact(
                        "Permission level",
                        label_of(
                            &[
                                ("edit", "Auto edit"),
                                ("plan", "Plan only"),
                                ("full", "Full access"),
                            ],
                            &st.default_mode,
                        ),
                    ))
                    .child(fact(
                        "Playbooks",
                        on_off(st.confirm_agent_playbooks, "Ask first", "Start right away"),
                    ))
                    .child(fact(
                        "Deleting",
                        on_off(st.confirm_delete, "Confirm first", "No popup"),
                    ))
                    .child(fact(
                        "Worktree on delete",
                        on_off(st.remove_worktree_on_delete, "Removed", "Kept"),
                    )),
            );

        // The small cards along the bottom.
        let phones = phone::phones(&store::data_dir());
        let phone_card = ov_card("ov-phone", IconName::Smartphone, "Phone", "")
            .on_click(open_page(SettingsPage::Phone, cx))
            .flex_1()
            .child(ov_big(if phones.is_empty() {
                "Not paired".into()
            } else {
                plural(phones.len(), "phone")
            }))
            .child(ov_note(match (phones.is_empty(), phone::bridge()) {
                (true, _) => "Follow and answer sessions from the Runo app",
                (false, phone::Bridge::Off) => "Paired · the bridge is off",
                (false, _) => "Paired · the bridge is on",
            }));
        let (left, size) = self
            .cleanup
            .items
            .as_ref()
            .map(|items| (items.len(), items.iter().map(|i| i.size).sum::<u64>()))
            .unwrap_or_default();
        let worktrees = ov_card("ov-worktrees", IconName::GitBranch, "Worktrees", "")
            .on_click(open_page(SettingsPage::Worktrees, cx))
            .flex_1()
            .when(left > 0, |el| {
                el.border_1().border_color(rgba((c_amber() << 8) | 0x66))
            })
            .child(ov_big(match &self.cleanup.items {
                None => "Not scanned".into(),
                Some(_) if left == 0 => "All tidy".into(),
                Some(_) => format!("{left} left over"),
            }))
            .child(ov_note(&match &self.cleanup.items {
                None => "Find worktrees no session uses".to_string(),
                Some(_) if left == 0 => "Every worktree belongs to a session".to_string(),
                Some(_) => format!("{} you can free", cleanup::human(size)),
            }));
        let changed = st.keybindings.len();
        let keys = ov_card("ov-keys", IconName::Keyboard, "Keys", "")
            .on_click(open_page(SettingsPage::Keybindings, cx))
            .flex_1()
            .child(ov_big(if changed == 0 {
                "Defaults".into()
            } else {
                format!("{changed} changed")
            }))
            .child(ov_note(&format!(
                "{} shortcuts you can change",
                crate::keymap::SHORTCUTS.len()
            )));
        let tools = ov_card("ov-tools", IconName::Plug, "Tools", "")
            .on_click(open_page(SettingsPage::Mcp, cx))
            .flex_1()
            .child(ov_big(format!(
                "{} · {} · {}",
                data.mcp.len(),
                data.skills.len(),
                data.hooks.len()
            )))
            .child(ov_note("MCP servers · skills · hooks"));

        // Every on/off setting outside the agents' own pages, to flip right here.
        let mut grid = div().grid().grid_cols(3).gap_x(px(28.));
        let mut n = 0usize;
        for page in SettingsTab::ALL
            .iter()
            .filter(|t| **t != SettingsTab::Agents)
            .flat_map(|t| t.pages().iter().copied())
            .chain([SettingsPage::Chat])
        {
            for r in page_sections(page, st, data)
                .into_iter()
                .flat_map(|s| s.rows)
            {
                if !matches!(r.ctl, Ctl::Switch(..)) {
                    continue;
                }
                let control = self.control(9100 + n, r.ctl, window, cx);
                grid = grid.child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .py(px(9.))
                        .when(n >= 3, |el| el.border_t_1().border_color(divider()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .v_flex()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(c_text()))
                                        .truncate()
                                        .child(r.title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(c_muted()))
                                        .child(page.label()),
                                ),
                        )
                        .child(control),
                );
                n += 1;
            }
        }
        let switches = div()
            .v_flex()
            .gap(px(4.))
            .p(px(16.))
            .rounded(px(14.))
            .bg(tint(c_subtle()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(IconName::ToggleRight, 15., c_muted()))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child("Quick switches"),
                    ),
            )
            .child(grid);

        div()
            .v_flex()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().w(relative(0.3)).flex().child(look))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap(px(12.))
                            .child(div().flex().gap(px(12.)).child(agents).child(conns))
                            .child(safety),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(phone_card)
                    .child(worktrees)
                    .child(keys)
                    .child(tools),
            )
            .child(switches)
            .into_any_element()
    }
}

/// A page's tab on the title line: its name, the open one underlined in the accent.
fn page_tab(
    key: String,
    lead: Option<AnyElement>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(key))
        .relative()
        .pb(px(8.))
        .flex()
        .items_center()
        .gap(px(7.))
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(if on { c_text() } else { c_muted() }))
        .when(!on, |el| el.hover(|el| el.text_color(rgb(c_secondary()))))
        .children(lead)
        .child(label.into())
        .when(on, |el| {
            el.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(-1.))
                    .h(px(2.))
                    .rounded_full()
                    .bg(rgb(c_blue())),
            )
        })
}

const NOTCH_R: f32 = 12.;

/// The inward curve under a section's tab, where the tab meets its card: a square in the
/// card's fill with a quarter circle cut from its bottom left.
fn notch(fill: Rgba) -> Div {
    let r = NOTCH_R;
    div().size(px(r)).flex_none().child(
        canvas(
            |_, _, _| (),
            move |b, _, window, _| {
                let k = 0.5523;
                let p = |x: f32, y: f32| point(b.origin.x + px(x), b.origin.y + px(y));
                let mut f = PathBuilder::fill();
                f.move_to(p(r, 0.));
                f.line_to(p(r, r));
                f.cubic_bezier_to(p(0., 0.), p(r, r - k * r), p(k * r, 0.));
                f.close();
                if let Ok(path) = f.build() {
                    window.paint_path(path, Hsla::from(fill));
                }
            },
        )
        .size_full(),
    )
}

/// Open a page of Settings when clicked.
fn open_page(page: SettingsPage, cx: &mut Context<RunoApp>) -> Listener {
    Box::new(cx.listener(move |this, _, _, cx| {
        this.settings_page = page;
        cx.notify();
    }))
}

/// A rounded square in a wash of the accent, holding an icon: the mark of a page.
fn tile(glyph: IconName, size: f32) -> Div {
    div()
        .size(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.28))
        .bg(rgb(mix(
            c_blue(),
            c_canvas(),
            if is_dark() { 0.2 } else { 0.12 },
        )))
        .child(icon(glyph, size * 0.5, c_blue()))
}

/// A quiet square holding an icon, at the start of a line in a card.
fn small_tile(glyph: IconName) -> AnyElement {
    div()
        .size(px(26.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .bg(tint(c_subtle()))
        .child(icon(glyph, 14., c_secondary()))
        .into_any_element()
}

/// A card on Settings > Overview: its icon and name, and on the right a link to its page.
fn ov_card(
    id: &'static str,
    glyph: IconName,
    title: &'static str,
    link: &'static str,
) -> Stateful<Div> {
    div()
        .id(id)
        .min_w_0()
        .v_flex()
        .gap(px(10.))
        .p(px(16.))
        .rounded(px(14.))
        .bg(tint(c_subtle()))
        .cursor_pointer()
        .hover(|el| el.bg(tint(c_active())))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(glyph, 15., c_muted()))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_text()))
                        .child(title),
                )
                .when(!link.is_empty(), |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_blue()))
                            .child(link)
                            .child(icon(IconName::ChevronRight, 12., c_blue())),
                    )
                }),
        )
}

/// One line in an Overview card: a mark, a name over a detail, and a status on the right.
fn ov_line(rule: bool, lead: AnyElement, name: String, detail: String, end: AnyElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .when(rule, |el| {
            el.pt(px(10.)).border_t_1().border_color(divider())
        })
        .child(lead)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .v_flex()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_text()))
                        .child(name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(c_muted()))
                        .truncate()
                        .child(detail),
                ),
        )
        .child(end)
}

/// A dot and a word in a soft pill of the same colour, such as "Signed in".
fn status_pill(text: &'static str, color: u32) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(9.))
        .rounded_full()
        .bg(rgb(mix(
            color,
            c_canvas(),
            if is_dark() { 0.16 } else { 0.12 },
        )))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(c_text()))
        .child(div().size(px(7.)).rounded_full().bg(rgb(color)))
        .child(text)
        .into_any_element()
}

/// The headline of a small Overview card.
fn ov_big(text: String) -> Div {
    div()
        .text_size(px(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_text()))
        .child(text)
}

fn ov_note(text: &str) -> Div {
    div()
        .text_xs()
        .text_color(rgb(c_muted()))
        .child(text.to_string())
}

/// A small window drawn in the current theme, glass and accent: the Look card's preview.
fn look_preview() -> Div {
    let bar = |w: f32, color: Rgba| div().h(px(6.)).w(relative(w)).rounded_full().bg(color);
    let line = tint(c_active());
    div()
        .h(px(150.))
        .rounded(px(10.))
        .p(px(10.))
        .bg(rgb(c_rail()))
        .flex()
        .child(div().w(px(18.)).flex_none())
        .child(
            div()
                .w(px(70.))
                .flex_none()
                .rounded_l(px(7.))
                .bg(pane(c_sheet()))
                .p(px(7.))
                .v_flex()
                .gap(px(5.))
                .child(
                    div()
                        .relative()
                        .h(px(14.))
                        .rounded(px(4.))
                        .bg(tint(c_active()))
                        .child(crate::shell::accent_bracket("look-preview-rule", 4.)),
                )
                .child(bar(0.8, line))
                .child(bar(0.6, line)),
        )
        .child(
            div()
                .flex_1()
                .rounded_r(px(7.))
                .bg(glass_fill(c_canvas()))
                .p(px(10.))
                .v_flex()
                .gap(px(7.))
                .child(
                    div().flex().justify_end().child(
                        div()
                            .w(relative(0.5))
                            .h(px(14.))
                            .rounded(px(5.))
                            .bg(tint(c_subtle())),
                    ),
                )
                .child(bar(0.9, line))
                .child(bar(0.7, line))
                .child(bar(0.8, line))
                .child(
                    div()
                        .mt_1()
                        .h(px(26.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgb(c_blue())),
                ),
        )
}

/// Settings > Worktrees > Clean up: what the last scan found and what is picked.
#[derive(Default)]
pub(crate) struct CleanupView {
    scanning: bool,
    removing: bool,
    /// None until the first scan.
    items: Option<Vec<cleanup::Item>>,
    /// Keys of the picked items.
    picked: HashSet<String>,
    /// "Remove selected" was pressed and waits for a yes.
    confirming: bool,
    /// How the last removal went, and whether all of it worked.
    result: Option<(String, bool)>,
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

impl RunoApp {
    /// The projects to scan and what every session, archived ones included, holds on to.
    fn cleanup_inputs(&self) -> (Vec<cleanup::Project>, Vec<cleanup::Use>) {
        let projects: Vec<cleanup::Project> = self
            .state
            .projects
            .iter()
            .map(|p| cleanup::Project {
                name: p.name.clone(),
                path: p.path.clone(),
            })
            .collect();
        let uses = self
            .state
            .sessions
            .iter()
            .map(|s| cleanup::Use {
                project: self
                    .state
                    .projects
                    .iter()
                    .find(|p| p.id == s.project_id)
                    .map(|p| p.path.clone())
                    .unwrap_or_default(),
                dir: s.dir.clone(),
                branch: s.branch.clone(),
            })
            .collect();
        (projects, uses)
    }

    /// Look for leftover worktrees, folders and branches in every project, off the UI
    /// thread. Everything that can't lose work starts picked.
    pub(crate) fn scan_cleanup(&mut self) {
        if self.cleanup.scanning || self.cleanup.removing {
            return;
        }
        self.cleanup.scanning = true;
        self.cleanup.confirming = false;
        let (projects, uses) = self.cleanup_inputs();
        self.bg(
            move || cleanup::scan(&projects, &uses, true),
            |this, items: Vec<cleanup::Item>, cx| {
                this.cleanup.picked = items
                    .iter()
                    .filter(|i| !i.careful())
                    .map(cleanup::Item::key)
                    .collect();
                applog::info(
                    "worktree",
                    format!(
                        "Clean up scan found {} ({})",
                        plural(items.len(), "item"),
                        cleanup::human(items.iter().map(|i| i.size).sum())
                    ),
                );
                this.cleanup.items = Some(items);
                this.cleanup.scanning = false;
                cx.notify();
            },
        );
    }

    fn picked_cleanup(&self) -> Vec<cleanup::Item> {
        self.cleanup
            .items
            .iter()
            .flatten()
            .filter(|i| self.cleanup.picked.contains(&i.key()))
            .cloned()
            .collect()
    }

    /// Remove the picked items, checking again first that no session uses them.
    fn remove_cleanup(&mut self) {
        self.cleanup.confirming = false;
        let picked = self.picked_cleanup();
        if picked.is_empty() || self.cleanup.removing {
            return;
        }
        self.cleanup.removing = true;
        let (projects, uses) = self.cleanup_inputs();
        self.bg(
            move || cleanup::remove(picked, &projects, &uses),
            |this, results: Vec<(cleanup::Item, Result<(), String>)>, cx| {
                let (mut removed, mut freed, mut failed) = (0, 0u64, Vec::new());
                for (item, result) in &results {
                    let what = match item.found.what {
                        cleanup::What::Branch => format!("branch {}", item.found.branch),
                        cleanup::What::Missing => {
                            format!("missing worktree {}", item.found.path.display())
                        }
                        cleanup::What::Folder => format!("folder {}", item.found.path.display()),
                        cleanup::What::Worktree => {
                            format!("worktree {}", item.found.path.display())
                        }
                    };
                    match result {
                        Ok(()) => {
                            removed += 1;
                            freed += item.size;
                            applog::info(
                                "worktree",
                                format!("Clean up removed {what} in project {:?}", item.project),
                            );
                        }
                        Err(error) => {
                            applog::warn(
                                "worktree",
                                format!(
                                    "Clean up could not remove {what} in project {:?}: {error}",
                                    item.project
                                ),
                            );
                            failed.push(format!("{}: {error}", item.title()));
                        }
                    }
                }
                let mut text = format!(
                    "Removed {}, freeing {}.",
                    plural(removed, "item"),
                    cleanup::human(freed)
                );
                if !failed.is_empty() {
                    text.push_str(&format!(
                        " {} could not be removed: {}",
                        failed.len(),
                        failed.join("; ")
                    ));
                    this.fail("worktree", text.clone());
                }
                this.cleanup.result = Some((text, failed.is_empty()));
                this.cleanup.removing = false;
                this.scan_cleanup();
                cx.notify();
            },
        );
    }

    fn cleanup_row(&self, item: &cleanup::Item) -> Row {
        let key = item.key();
        let on = self.cleanup.picked.contains(&key);
        let note = match item.found.what {
            cleanup::What::Worktree | cleanup::What::Folder => cleanup::human(item.size),
            cleanup::What::Missing => "folder gone".into(),
            cleanup::What::Branch => "branch".into(),
        };
        row(
            &item.title(),
            &item.detail(),
            Ctl::Check(
                on,
                note,
                Rc::new(move |this: &mut RunoApp, _: &mut Context<RunoApp>| {
                    if !this.cleanup.picked.remove(&key) {
                        this.cleanup.picked.insert(key.clone());
                    }
                    this.cleanup.confirming = false;
                }),
            ),
        )
    }

    /// The Clean up sections of the Worktrees page.
    fn cleanup_sections(&self) -> Vec<Section> {
        let view = &self.cleanup;
        let scan: Action = Rc::new(|this: &mut RunoApp, _: &mut Context<RunoApp>| {
            this.cleanup.result = None;
            this.scan_cleanup();
        });
        let mut rows = Vec::new();
        let Some(items) = view.items.as_ref() else {
            rows.push(row(
                "Find what's left behind",
                "Looks in every project for worktrees and folders under .runo-worktrees and runo/ branches that no session uses, archived ones included, and how much space they take.",
                if view.scanning {
                    Ctl::Chip("Scanning…".into(), 3)
                } else {
                    Ctl::Button("Scan".into(), scan)
                },
            ));
            return vec![section("Clean up", "Sessions remove their worktree when they are deleted or cleaned up after a merge. Anything left behind another way shows up here.", rows)];
        };
        let picked = self.picked_cleanup();
        let picked_size: u64 = picked.iter().map(|i| i.size).sum();
        let total: u64 = items.iter().map(|i| i.size).sum();
        let summary = if items.is_empty() {
            "Nothing to clean up: every worktree and runo/ branch belongs to a session.".to_string()
        } else {
            format!(
                "{} found, {} in all. {} picked, {}.",
                plural(items.len(), "item"),
                cleanup::human(total),
                picked.len(),
                cleanup::human(picked_size)
            )
        };
        let busy = if view.scanning {
            Some("Scanning…")
        } else if view.removing {
            Some("Removing…")
        } else {
            None
        };
        rows.push(row(
            "Left behind",
            &summary,
            match busy {
                Some(text) => Ctl::Chip(text.into(), 3),
                None if picked.is_empty() => Ctl::Button("Scan again".into(), scan),
                None => Ctl::Buttons(vec![
                    ("Scan again".into(), scan),
                    (
                        "Remove selected".into(),
                        Rc::new(|this: &mut RunoApp, _: &mut Context<RunoApp>| {
                            this.cleanup.confirming = true;
                        }),
                    ),
                ]),
            },
        ));
        if view.confirming && busy.is_none() && !picked.is_empty() {
            let careful = picked.iter().filter(|i| i.careful()).count();
            let mut desc = "Worktrees are removed with git worktree remove --force, so changes in them are lost, and branches are deleted with git branch -D. This can't be undone.".to_string();
            if careful > 0 {
                desc.push_str(&format!(
                    " {} of them hold work not on the default branch or never committed.",
                    careful
                ));
            }
            rows.push(row(
                &format!(
                    "Remove {} ({})?",
                    plural(picked.len(), "item"),
                    cleanup::human(picked_size)
                ),
                &desc,
                Ctl::Buttons(vec![
                    (
                        "Cancel".into(),
                        Rc::new(|this: &mut RunoApp, _: &mut Context<RunoApp>| {
                            this.cleanup.confirming = false;
                        }),
                    ),
                    (
                        "Remove".into(),
                        Rc::new(|this: &mut RunoApp, _: &mut Context<RunoApp>| {
                            this.remove_cleanup();
                        }),
                    ),
                ]),
            ));
        }
        if let Some((result, ok)) = &view.result {
            rows.push(row(
                "Last clean up",
                result,
                if *ok {
                    Ctl::Chip("Done".into(), 0)
                } else {
                    Ctl::Chip("Some failed".into(), 2)
                },
            ));
        }
        rows.extend(
            items
                .iter()
                .filter(|i| !i.careful())
                .map(|i| self.cleanup_row(i)),
        );
        let mut out = vec![section(
            "Clean up",
            "Worktrees, folders and runo/ branches that no session uses, archived ones included. Merged branches and worktrees without changes start picked.",
            rows,
        )];
        let careful: Vec<Row> = items
            .iter()
            .filter(|i| i.careful())
            .map(|i| self.cleanup_row(i))
            .collect();
        if !careful.is_empty() {
            out.push(section(
                "Unmerged work",
                "⚠ These hold commits the default branch doesn't have, or changes never committed. Removing them loses that work, so they are never picked for you.",
                careful,
            ));
        }
        out
    }
}
