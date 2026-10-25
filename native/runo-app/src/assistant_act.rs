//! What the Runo assistant can do in the app, and what it knows of it. The answering
//! agent offers actions as `ACTION:` lines of JSON; each becomes a card the person runs
//! with a click, described from Runo's own state rather than the agent's words, so a card
//! always says what it will really do. Every action goes through the same code the
//! buttons elsewhere in the app use, so the assistant can never do more than the person
//! could by hand.

use super::*;
use serde::{Deserialize, Serialize};

/// The most actions one answer can offer.
pub(crate) const MAX_ACTIONS: usize = 3;
/// The most sessions described to the agent with each question, newest first.
const SESSIONS_SHOWN: usize = 20;

/// Something the assistant offers to do.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "do", rename_all = "lowercase")]
pub(crate) enum Action {
    /// Go to a place in the app, named the way guide articles name them.
    Open { place: String },
    /// Open a session.
    Session { id: u64 },
    /// Switch the top bar to a project.
    Project { name: String },
    /// Start one agent on a task.
    Start {
        task: String,
        #[serde(default)]
        agent: String,
        #[serde(default)]
        isolated: Option<bool>,
        #[serde(default)]
        project: String,
    },
    /// Start several agents on the same task, each in its own worktree.
    Race {
        task: String,
        agents: Vec<String>,
        #[serde(default)]
        project: String,
    },
    /// Run a playbook in the current project.
    Playbook { name: String },
    /// Send a message to a session's agent; it waits in the queue while the agent works.
    Send { session: u64, text: String },
    /// Stop a session's agent.
    Stop { session: u64 },
    /// Allow a permission request once.
    Approve { request: String },
    /// Refuse a permission request.
    Deny { request: String },
    /// Run one of Quick open's commands, by its id.
    Command { id: String },
    /// Change a setting, or press a button in Settings, by the names Settings shows: the
    /// page and the row's title (or its button's label). The older form names one of a
    /// few settings by `key`.
    Setting {
        #[serde(default)]
        key: String,
        #[serde(default)]
        page: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        value: serde_json::Value,
    },
    /// Keep a preference or fact for later chats, in the current project.
    Remember { note: String },
    /// Drop something remembered.
    Forget { note: String },
}

/// What became of an action card.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) enum Outcome {
    Ready,
    Done(String),
    Failed(String),
    Dismissed,
}

/// An `ACTION:` line's action, or nothing when the line isn't one or doesn't parse.
pub(crate) fn parse(line: &str) -> Option<Action> {
    let rest = line.trim().strip_prefix("ACTION:")?;
    // The object alone, whatever the agent wrapped it in.
    let json = &rest[rest.find('{')?..=rest.rfind('}')?];
    serde_json::from_str(json).ok()
}

/// The agent an id or name means, as its place in `agent::PROVIDERS`.
fn provider_index(name: &str) -> Option<usize> {
    let name = name.trim().to_lowercase();
    agent::PROVIDERS
        .iter()
        .position(|p| p.id == name || p.name.to_lowercase() == name)
}

/// A setting the assistant may change, checked: the key, and the value to store.
#[derive(Debug, PartialEq)]
enum SettingChange {
    Theme(String),
    Animations(bool),
    DefaultAgent(usize),
}

fn setting(key: &str, value: &serde_json::Value) -> Result<SettingChange, String> {
    match key {
        "theme" => match value.as_str() {
            Some(v @ ("light" | "dark" | "system")) => Ok(SettingChange::Theme(v.to_string())),
            _ => Err("The theme is light, dark or system.".into()),
        },
        "animations" => value
            .as_bool()
            .map(SettingChange::Animations)
            .ok_or_else(|| "Animations are on or off.".into()),
        "default_agent" => value
            .as_str()
            .and_then(provider_index)
            .map(SettingChange::DefaultAgent)
            .ok_or_else(|| "That isn't an agent Runo knows.".into()),
        _ => Err(format!("The assistant can't change {key}.")),
    }
}

/// What changing a setting found by name does.
enum Apply {
    Bool(crate::settings_ui::SetBool, bool),
    Str(crate::settings_ui::SetStr, String),
    Press(crate::settings_ui::Action),
}

/// The value the agent gave for a switch.
fn on_off(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::String(s) => match s.trim().to_lowercase().as_str() {
            "true" | "on" | "yes" => Some(true),
            "false" | "off" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Words in a setting's name or section that mean it guards what agents may do, or holds
/// a secret. The assistant never offers to change those: an answer can be steered by
/// text the agent read, so loosening a guard is left to the person, in Settings.
const GUARDED_WORDS: [&str; 12] = [
    "permission",
    "approv",
    "sandbox",
    "full access",
    "trust",
    "bypass",
    "dangerous",
    "token",
    "api key",
    "password",
    "secret",
    "environment",
];

/// The buttons in Settings the assistant may press: each only checks, detects, tests or
/// shows something, or picks a model. Signing in or out, installing, pairing, setting up
/// and removing stay with the person.
const PRESSABLE: [&str; 8] = [
    "Check now",
    "Take the tour",
    "Re-detect",
    "Test",
    "Use as default",
    "Use the CLI's default",
    "Scan",
    "Scan again",
];

/// Why the assistant won't change the setting `title` (in `section` on `page`), or
/// press its button `button`, or nothing when it may offer to.
fn guarded(
    page: crate::settings_ui::SettingsPage,
    section: &str,
    title: &str,
    button: Option<&str>,
) -> Option<String> {
    use crate::settings_ui::SettingsPage;
    let place = crate::settings_ui::place(page);
    let why = || Some(format!("{title} is in {place}; change it there yourself."));
    // Safety holds the limits and permission level; Connections and Phone decide what
    // can reach this computer and what agents may run elsewhere.
    if matches!(
        page,
        SettingsPage::Safety | SettingsPage::Connections | SettingsPage::Phone
    ) {
        return why();
    }
    let words = format!("{section} {title}").to_lowercase();
    if GUARDED_WORDS.iter().any(|w| words.contains(w)) {
        return why();
    }
    match button {
        Some(label)
            if !PRESSABLE
                .iter()
                .any(|p| p.eq_ignore_ascii_case(label.trim())) =>
        {
            Some(format!("{label} in {place} is for you to press there."))
        }
        _ => None,
    }
}

/// A line of text cut to `max` characters, on one line.
fn excerpt(text: &str, max: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        format!("{}…", one.chars().take(max).collect::<String>())
    }
}

/// Whether `path` is the top of a git checkout, from the disk alone: cards are drawn on
/// every frame, so they can't ask git. Starting the session checks properly.
fn looks_like_repo(path: &str) -> bool {
    std::path::Path::new(path).join(".git").exists()
}

/// How an action card looks: its icon, what it does, and the details under that.
pub(crate) struct Card {
    pub icon: IconName,
    pub title: String,
    pub detail: String,
    /// Whether it changes something, rather than only going somewhere.
    pub changes: bool,
}

impl RunoApp {
    fn project_named(&self, name: &str) -> Option<&Project> {
        let name = name.trim().to_lowercase();
        self.state
            .projects
            .iter()
            .find(|p| p.name.to_lowercase() == name)
    }

    /// The project an action starts work in: the one it names, else the top bar's, else
    /// the first.
    fn action_project(&self, name: &str) -> Result<&Project, String> {
        if !name.trim().is_empty() {
            return self
                .project_named(name)
                .ok_or_else(|| format!("There's no project called {name}."));
        }
        self.current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .or_else(|| self.state.projects.first())
            .ok_or_else(|| "Add a project first.".into())
    }

    fn action_agent(&self, name: &str, cx: &App) -> Result<usize, String> {
        let name = if name.trim().is_empty() {
            cx.global::<AppSettings>().default_provider.clone()
        } else {
            name.to_string()
        };
        let i = provider_index(&name).ok_or_else(|| format!("Runo doesn't know {name}."))?;
        let p = &agent::PROVIDERS[i];
        if !agent::installed(p.bin) {
            return Err(format!("{} isn't installed.", p.name));
        }
        Ok(i)
    }

    fn action_session(&self, id: u64) -> Result<&Session, String> {
        self.state
            .sessions
            .iter()
            .find(|s| s.id == id && !s.archived)
            .ok_or_else(|| format!("There's no session #{id}."))
    }

    fn action_playbook(&self, name: &str) -> Result<&store::Automation, String> {
        let name = name.trim().to_lowercase();
        let all = &self.state.automations;
        all.iter()
            .find(|a| a.name.to_lowercase() == name)
            .or_else(|| all.iter().find(|a| a.name.to_lowercase().contains(&name)))
            .ok_or_else(|| format!("There's no playbook called {name}."))
    }

    fn action_request(&self, id: &str) -> Result<&approvals::Request, String> {
        self.approvals
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| "That request isn't waiting any more.".into())
    }

    /// The setting on `page` called `name`, and what setting it to `value` (or pressing
    /// it, for a button) does: the card's title and detail, and the change.
    fn named_setting(
        &self,
        page: &str,
        name: &str,
        value: &serde_json::Value,
        cx: &App,
    ) -> Result<(String, String, Apply), String> {
        use crate::settings_ui::Knob;
        let data = self.discovered.clone().unwrap_or_default();
        let found = crate::settings_ui::find_setting(page, name, &data, cx)
            .ok_or_else(|| format!("Settings has nothing called {name}."))?;
        let place = crate::settings_ui::place(found.page);
        let button = match &found.knob {
            Knob::Button(label, _) => Some(label.as_str()),
            _ => None,
        };
        if let Some(why) = guarded(found.page, &found.section, &found.title, button) {
            return Err(why);
        }
        let title = found.title;
        match found.knob {
            Knob::Switch(now, set) => {
                let on = on_off(value).ok_or_else(|| format!("{title} is on or off."))?;
                if on == now {
                    return Err(format!(
                        "{title} is already {}.",
                        if on { "on" } else { "off" }
                    ));
                }
                let word = |b: bool| if b { "On" } else { "Off" };
                Ok((
                    format!("Turn {} {}", title, if on { "on" } else { "off" }),
                    format!("{place} · {} → {}", word(now), word(on)),
                    Apply::Bool(set, on),
                ))
            }
            Knob::Choice(opts, now, set) => {
                let wanted = match value {
                    serde_json::Value::String(v) => v.trim().to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Number(n) => n.to_string(),
                    _ => String::new(),
                };
                let (v, label) = opts
                    .iter()
                    .find(|(v, l)| {
                        v.eq_ignore_ascii_case(&wanted) || l.eq_ignore_ascii_case(&wanted)
                    })
                    .cloned()
                    .ok_or_else(|| {
                        let names: Vec<&str> = opts.iter().map(|(_, l)| l.as_str()).collect();
                        format!("{title} is one of {}.", names.join(", "))
                    })?;
                if v == now {
                    return Err(format!("{title} is already {label}."));
                }
                let was = opts
                    .iter()
                    .find(|(v, _)| *v == now)
                    .map(|(_, l)| l.as_str())
                    .unwrap_or(if now.is_empty() {
                        "Default"
                    } else {
                        now.as_str()
                    });
                Ok((
                    format!("Set {title} to {label}"),
                    format!("{place} · {was} → {label}"),
                    Apply::Str(set, v),
                ))
            }
            Knob::Text(_) => Err(format!(
                "{title} is typed in {place}; open it there to change it."
            )),
            Knob::Button(label, press) => {
                if !value.is_null() {
                    return Err(format!("{title} is a button, not a setting."));
                }
                Ok((label, format!("{place} → {title}"), Apply::Press(press)))
            }
        }
    }

    /// The card for `action`, written from Runo's state, or why it can't be done now.
    pub(crate) fn action_card(&self, action: &Action, cx: &App) -> Result<Card, String> {
        let go = |icon, title: String, detail: String| Card {
            icon,
            title,
            detail,
            changes: false,
        };
        let change = |icon, title: String, detail: String| Card {
            icon,
            title,
            detail,
            changes: true,
        };
        Ok(match action {
            Action::Open { place } => {
                if crate::assistant::jump_for(place).is_none() {
                    return Err(format!("Runo has no place called {place}."));
                }
                go(
                    IconName::ArrowUpRight,
                    crate::assistant::open_label(place),
                    String::new(),
                )
            }
            Action::Session { id } => {
                let s = self.action_session(*id)?;
                go(
                    IconName::ArrowUpRight,
                    format!("Open \"{}\"", s.title),
                    format!("{} · {}", agent::provider(&s.provider).name, s.branch),
                )
            }
            Action::Project { name } => {
                let p = self
                    .project_named(name)
                    .ok_or_else(|| format!("There's no project called {name}."))?;
                go(
                    IconName::Folder,
                    format!("Switch to {}", p.name),
                    crate::settings_ui::tilde(std::path::Path::new(&p.path)),
                )
            }
            Action::Start {
                task,
                agent,
                isolated,
                project,
            } => {
                let p = self.action_project(project)?;
                let a = self.action_agent(agent, cx)?;
                let worktree = isolated.unwrap_or(true) && looks_like_repo(&p.path);
                change(
                    IconName::Play,
                    format!("Start {}", agent::PROVIDERS[a].name),
                    format!(
                        "{} · {} · \"{}\"",
                        p.name,
                        if worktree {
                            "own worktree"
                        } else {
                            "project folder"
                        },
                        excerpt(task, 80)
                    ),
                )
            }
            Action::Race {
                task,
                agents,
                project,
            } => {
                let p = self.action_project(project)?;
                if !looks_like_repo(&p.path) {
                    return Err(format!(
                        "{} isn't a git repository, so it can't race.",
                        p.name
                    ));
                }
                let mut names = Vec::new();
                for a in agents {
                    let i = self.action_agent(a, cx)?;
                    if !names.contains(&agent::PROVIDERS[i].name) {
                        names.push(agent::PROVIDERS[i].name);
                    }
                }
                if names.len() < 2 {
                    return Err("A race needs two or more agents.".into());
                }
                change(
                    IconName::Swords,
                    format!("Race {}", names.join(" and ")),
                    format!("{} · a worktree each · \"{}\"", p.name, excerpt(task, 80)),
                )
            }
            Action::Playbook { name } => {
                let pb = self.action_playbook(name)?;
                change(
                    IconName::Play,
                    format!("Run playbook \"{}\"", pb.name),
                    agent::provider(&pb.provider).name.to_string(),
                )
            }
            Action::Send { session, text } => {
                let s = self.action_session(*session)?;
                change(
                    IconName::Send,
                    format!("Send to \"{}\"", s.title),
                    format!("\"{}\"", excerpt(text, 90)),
                )
            }
            Action::Stop { session } => {
                let s = self.action_session(*session)?;
                if !self.is_running(s) {
                    return Err(format!("\"{}\" isn't running.", s.title));
                }
                change(
                    IconName::CircleStop,
                    format!("Stop \"{}\"", s.title),
                    agent::provider(&s.provider).name.to_string(),
                )
            }
            Action::Approve { request } | Action::Deny { request } => {
                let r = self.action_request(request)?;
                let who = self
                    .approval_session(r)
                    .and_then(|id| self.state.sessions.iter().find(|s| s.id == id))
                    .map(|s| format!(" in \"{}\"", s.title))
                    .unwrap_or_default();
                let allow = matches!(action, Action::Approve { .. });
                change(
                    if allow { IconName::Check } else { IconName::X },
                    format!("{} once{who}", if allow { "Allow" } else { "Deny" }),
                    excerpt(&approvals::describe(&r.tool, &r.input), 90),
                )
            }
            Action::Command { id } => {
                let c = commands::find(id).ok_or_else(|| format!("Runo has no command {id}."))?;
                let keys = c.keys(cx);
                let detail = if c.id == "quit" {
                    match self.local_runs() {
                        0 => keys,
                        1 => "1 agent is running here; Runo asks once more before stopping it."
                            .to_string(),
                        n => format!(
                            "{n} agents are running here; Runo asks once more before stopping them."
                        ),
                    }
                } else {
                    keys
                };
                let card = Card {
                    icon: c.icon,
                    title: c.label.to_string(),
                    detail,
                    changes: matches!(c.id, "quit" | "stop" | "archive" | "close-tab"),
                };
                return Ok(card);
            }
            Action::Setting {
                key,
                page,
                name,
                value,
            } if !name.trim().is_empty() => {
                let (title, detail, _) = self.named_setting(page, name, value, cx)?;
                change(IconName::Settings, title, detail)
            }
            Action::Setting { key, value, .. } => {
                let (title, detail) = match setting(key, value)? {
                    SettingChange::Theme(t) => (
                        "Change the theme".to_string(),
                        format!("{} → {t}", cx.global::<AppSettings>().theme),
                    ),
                    SettingChange::Animations(on) => {
                        let word = |b: bool| if b { "On" } else { "Off" };
                        (
                            format!("Turn animations {}", if on { "on" } else { "off" }),
                            format!(
                                "{} → {}",
                                word(cx.global::<AppSettings>().animations),
                                word(on)
                            ),
                        )
                    }
                    SettingChange::DefaultAgent(i) => {
                        let now = &cx.global::<AppSettings>().default_provider;
                        let was = agent::PROVIDERS
                            .iter()
                            .find(|p| p.id == now)
                            .map_or(now.as_str(), |p| p.name);
                        (
                            "Change the default agent".to_string(),
                            format!("{was} → {}", agent::PROVIDERS[i].name),
                        )
                    }
                };
                change(IconName::Settings, title, detail)
            }
            Action::Remember { note } => {
                if note.trim().is_empty() {
                    return Err("There's nothing to remember.".into());
                }
                let project = self.assistant_project_name();
                change(
                    IconName::Brain,
                    "Remember this".into(),
                    if project.is_empty() {
                        format!("\"{}\"", excerpt(note, 90))
                    } else {
                        format!("\"{}\" · in {project}", excerpt(note, 80))
                    },
                )
            }
            Action::Forget { note } => {
                if !self.assistant.kept.notes.iter().any(|n| n.text == *note) {
                    return Err("I don't remember that.".into());
                }
                change(
                    IconName::Eraser,
                    "Forget this".into(),
                    format!("\"{}\"", excerpt(note, 90)),
                )
            }
        })
    }

    /// Do `action`, through the same code the app's own buttons use. Says what was done,
    /// or why not.
    pub(crate) fn run_action(
        &mut self,
        action: &Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        // Checked again: things may have moved on since the card was drawn.
        let card = self.action_card(action, cx)?;
        match action.clone() {
            Action::Open { place } => {
                let jump = crate::assistant::jump_for(&place).ok_or("Nowhere to go.")?;
                self.assistant.open = false;
                if place == "new-session" {
                    window.dispatch_action(Box::new(NewSession), cx);
                } else {
                    self.palette_jump(jump, window, cx);
                }
            }
            Action::Session { id } => {
                self.assistant.open = false;
                self.open_session(id, window, cx);
            }
            Action::Project { name } => {
                let pid = self
                    .project_named(&name)
                    .map(|p| p.id)
                    .ok_or("No project.")?;
                self.switch_project(pid, window, cx);
            }
            Action::Start {
                task,
                agent,
                isolated,
                project,
            } => {
                let pid = self.action_project(&project)?.id;
                let provider = self.action_agent(&agent, cx)?;
                self.start_from_assistant(pid, vec![provider], &task, isolated, window, cx)?;
            }
            Action::Race {
                task,
                agents,
                project,
            } => {
                let pid = self.action_project(&project)?.id;
                let mut providers = Vec::new();
                for a in &agents {
                    let i = self.action_agent(a, cx)?;
                    if !providers.contains(&i) {
                        providers.push(i);
                    }
                }
                self.start_from_assistant(pid, providers, &task, Some(true), window, cx)?;
            }
            Action::Playbook { name } => {
                let id = self.action_playbook(&name)?.id;
                let here = self.current_project();
                self.run_playbook(id, here, cx);
            }
            Action::Send { session, text } => {
                // Sent from the session's own composer, so a busy agent queues it and
                // a Crew chat routes it; whatever the person had typed there stays.
                self.open_session(session, window, cx);
                let draft = self.prompt.read(cx).value().to_string();
                let picked = self.picked_playbook.take();
                self.prompt
                    .update(cx, |p, cx| p.set_value(text.clone(), window, cx));
                self.submit_prompt(window, cx);
                self.prompt
                    .update(cx, |p, cx| p.set_value(draft, window, cx));
                self.picked_playbook = picked;
            }
            Action::Stop { session } => self.stop_session(session),
            Action::Approve { request } | Action::Deny { request } => {
                let r = self.action_request(&request)?.clone();
                let allow = matches!(action, Action::Approve { .. });
                self.answer_approval(&r, allow.then_some(false), cx);
            }
            Action::Command { id } => {
                let c = commands::find(&id).ok_or("No such command.")?;
                if c.id == "quit" {
                    // Quits for real even when Runo keeps running in the menu bar, with
                    // the usual warning first when agents are running.
                    cx.defer(crate::menubar::quit);
                } else {
                    self.assistant.open = false;
                    match c.run {
                        commands::Run::Action(action) => window.dispatch_action(action(), cx),
                        _ => self.palette_jump(c.jump(), window, cx),
                    }
                }
            }
            Action::Setting {
                page, name, value, ..
            } if !name.trim().is_empty() => match self.named_setting(&page, &name, &value, cx)?.2 {
                Apply::Bool(set, on) => settings::update(cx, move |s| set(s, on)),
                Apply::Str(set, v) => settings::update(cx, move |s| set(s, v)),
                Apply::Press(press) => press(self, cx),
            },
            Action::Setting { key, value, .. } => {
                let change = setting(&key, &value)?;
                settings::update(cx, move |s| match change {
                    SettingChange::Theme(t) => s.theme = t,
                    SettingChange::Animations(on) => s.animations = on,
                    SettingChange::DefaultAgent(i) => {
                        s.default_provider = agent::PROVIDERS[i].id.to_string()
                    }
                });
            }
            Action::Remember { note } => {
                self.assistant_remember(&note)?;
            }
            Action::Forget { note } => {
                self.assistant_forget(&note)?;
            }
        }
        cx.notify();
        Ok(card.title)
    }

    /// Start `providers` on `task` in project `pid` the way New session does: one agent,
    /// or a race of several, each in its own worktree. Nothing picked in the New session
    /// form is changed.
    fn start_from_assistant(
        &mut self,
        pid: u64,
        providers: Vec<usize>,
        task: &str,
        isolated: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let task_before = self.new_task.read(cx).value().to_string();
        let branch_before = self.branch.read(cx).value().to_string();
        let kept = (
            self.new_project,
            self.new_provider,
            self.new_isolated,
            self.new_connection,
            self.race_mode,
            self.new_race.clone(),
            self.new_model.take(),
            self.new_base.take(),
            self.new_context.take(),
        );
        self.new_project = Some(pid);
        self.new_provider = providers[0];
        self.new_isolated = isolated.unwrap_or(true);
        self.new_connection = self.project_connection(pid);
        self.race_mode = providers.len() > 1;
        self.new_race = providers;
        self.new_task
            .update(cx, |t, cx| t.set_value(task.to_string(), window, cx));
        self.branch.update(cx, |b, cx| b.set_value("", window, cx));
        let before = self.state.next_id;
        self.create_session(window, cx);
        let started = self.state.next_id != before;
        (
            self.new_project,
            self.new_provider,
            self.new_isolated,
            self.new_connection,
            self.race_mode,
            self.new_race,
            self.new_model,
            self.new_base,
            self.new_context,
        ) = kept;
        self.new_task
            .update(cx, |t, cx| t.set_value(task_before, window, cx));
        self.branch
            .update(cx, |b, cx| b.set_value(branch_before, window, cx));
        if started {
            self.assistant.open = false;
            Ok(())
        } else {
            Err("Runo couldn't start it; the reason is in the corner.".into())
        }
    }

    /// What the answering agent is told of the app: projects, agents, sessions and how
    /// they are doing, what waits on the person, playbooks and clashes. Titles, branches
    /// and short excerpts only: no file contents, and no more of any conversation than
    /// its last line.
    pub(crate) fn assistant_snapshot(&self, cx: &App) -> String {
        let st = cx.global::<AppSettings>();
        let here = self.current_project();
        let mut out = String::new();
        let projects: Vec<String> = self
            .state
            .projects
            .iter()
            .map(|p| {
                if Some(p.id) == here {
                    format!("{} (current)", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();
        out.push_str(&format!(
            "Projects: {}\n",
            if projects.is_empty() {
                "none yet".to_string()
            } else {
                projects.join(", ")
            }
        ));
        let agents: Vec<String> = agent::PROVIDERS
            .iter()
            .filter(|p| agent::installed(p.bin))
            .map(|p| format!("{} ({})", p.name, p.id))
            .collect();
        out.push_str(&format!(
            "Agents installed: {}; default: {}\n",
            if agents.is_empty() {
                "none".to_string()
            } else {
                agents.join(", ")
            },
            st.default_provider
        ));
        let mut sessions: Vec<&Session> =
            self.state.sessions.iter().filter(|s| !s.archived).collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.id));
        if !sessions.is_empty() {
            out.push_str("Sessions, newest first:\n");
        }
        for s in sessions.iter().take(SESSIONS_SHOWN) {
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == s.project_id)
                .map(|p| p.name.as_str())
                .unwrap_or("?");
            let state = if self.is_running(s) {
                "running"
            } else {
                Self::status_label(s.status)
            };
            let last = s
                .messages
                .iter()
                .rev()
                .find(|m| !m.text.trim().is_empty())
                .map(|m| format!("; last: \"{}\"", excerpt(&m.text, 140)))
                .unwrap_or_default();
            out.push_str(&format!(
                "- #{} \"{}\" · {} · branch {} · {} · project {}{}{}\n",
                s.id,
                s.title,
                agent::provider(&s.provider).name,
                s.branch,
                state.to_lowercase(),
                project,
                if s.isolated { " · own worktree" } else { "" },
                last
            ));
        }
        let mut waiting = Vec::new();
        for r in &self.approvals {
            let who = self
                .approval_session(r)
                .map(|id| format!(" in #{id}"))
                .unwrap_or_default();
            waiting.push(format!(
                "- permission request {}{who}: {}",
                r.id,
                excerpt(&approvals::describe(&r.tool, &r.input), 120)
            ));
        }
        for (id, _, w) in self.waiting_sessions() {
            let what = match w {
                alerts::Waiting::Denied(_) => "was refused a tool and stopped",
                alerts::Waiting::Questions(_) => "has questions",
                alerts::Waiting::Plan => "has a plan to approve",
                alerts::Waiting::Decision(_) => "needs a decision",
            };
            waiting.push(format!("- #{id} {what}"));
        }
        if !waiting.is_empty() {
            out.push_str("Waiting on the person:\n");
            out.push_str(&waiting.join("\n"));
            out.push('\n');
        }
        let playbooks: Vec<&str> = self
            .state
            .automations
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        if !playbooks.is_empty() {
            out.push_str(&format!("Playbooks: {}\n", playbooks.join("; ")));
        }
        if let Some(hits) = &self.git_radar {
            out.push_str(&format!(
                "Clash radar: {} places where two lines of work change the same lines\n",
                hits.len()
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn action_lines_parse() {
        assert_eq!(
            parse(r#"ACTION: {"do":"race","task":"fix it","agents":["claude","codex"]}"#),
            Some(Action::Race {
                task: "fix it".into(),
                agents: vec!["claude".into(), "codex".into()],
                project: String::new(),
            })
        );
        assert_eq!(
            parse(r#"ACTION: `{"do":"stop","session":4}`"#),
            Some(Action::Stop { session: 4 })
        );
        assert_eq!(
            parse(r#"ACTION: {"do":"start","task":"t"}"#),
            Some(Action::Start {
                task: "t".into(),
                agent: String::new(),
                isolated: None,
                project: String::new(),
            })
        );
        assert_eq!(parse("ACTION: {\"do\":\"format-disk\"}"), None);
        assert_eq!(parse("ACTION: not json"), None);
        assert_eq!(
            parse(r#"ACTION: {"do":"stop","session":4}\n"#),
            Some(Action::Stop { session: 4 })
        );
        assert_eq!(parse("Start a race."), None);
    }

    #[test]
    fn commands_and_named_settings_parse() {
        assert_eq!(
            parse(r#"ACTION: {"do":"command","id":"quit"}"#),
            Some(Action::Command { id: "quit".into() })
        );
        assert_eq!(
            parse(
                r#"ACTION: {"do":"setting","page":"General","name":"Show Runo in the menu bar","value":true}"#
            ),
            Some(Action::Setting {
                key: String::new(),
                page: "General".into(),
                name: "Show Runo in the menu bar".into(),
                value: serde_json::json!(true),
            })
        );
        // A button has no value.
        assert_eq!(
            parse(r#"ACTION: {"do":"setting","page":"General","name":"Check for updates"}"#),
            Some(Action::Setting {
                key: String::new(),
                page: "General".into(),
                name: "Check for updates".into(),
                value: serde_json::Value::Null,
            })
        );
        // The older form still reads.
        assert_eq!(
            parse(r#"ACTION: {"do":"setting","key":"theme","value":"dark"}"#),
            Some(Action::Setting {
                key: "theme".into(),
                page: String::new(),
                name: String::new(),
                value: serde_json::json!("dark"),
            })
        );
    }

    #[test]
    fn guards_and_secrets_stay_with_the_person() {
        use crate::settings_ui::SettingsPage as P;
        // Whole pages that decide what can reach this computer, or what agents may do.
        for page in [P::Safety, P::Connections, P::Phone] {
            assert!(guarded(page, "Limits", "Turns per day", None).is_some());
        }
        // Rows about permissions or secrets, on any page.
        assert!(guarded(P::Agent("claude"), "Permissions", "Default mode", None).is_some());
        assert!(guarded(P::General, "", "Wait for permission answers", None).is_some());
        assert!(guarded(P::Agent("codex"), "Run", "Sandbox", None).is_some());
        assert!(guarded(P::Mcp, "", "API key", None).is_some());
        // Buttons outside the short list.
        assert!(guarded(P::Agent("claude"), "", "Signed in", Some("Sign out")).is_some());
        assert!(guarded(P::Providers, "", "Codex", Some("Install…")).is_some());
        // The everyday ones still work.
        assert!(guarded(P::Appearance, "Window", "Font smoothing", None).is_none());
        assert!(guarded(
            P::General,
            "Updates",
            "Check for updates",
            Some("Check now")
        )
        .is_none());
        assert!(guarded(P::Agent("claude"), "Defaults", "Default effort", None).is_none());
    }

    #[test]
    fn tokens_and_pairing_links_are_hidden_from_the_agent() {
        use crate::settings_ui::redact;
        let link = crate::phone::pair_link("http://100.64.0.2:7631", &"ab12".repeat(16));
        assert_eq!(redact(&link), "[hidden]");
        assert_eq!(
            redact("Key sk-ant-api03-abcdefghijklmnop1234567890 is set."),
            "Key [hidden] is set."
        );
        // Ordinary words, versions and paths stay.
        let plain = "Runo 0.5.11 keeps its data in ~/.runo/sessions, see https://runo.dev/guide";
        assert_eq!(redact(plain), plain);
    }

    #[test]
    fn switches_take_on_and_off_in_several_words() {
        use serde_json::json;
        assert_eq!(on_off(&json!(true)), Some(true));
        assert_eq!(on_off(&json!("Off")), Some(false));
        assert_eq!(on_off(&json!("on")), Some(true));
        assert_eq!(on_off(&json!(3)), None);
    }

    #[test]
    fn agents_are_known_by_id_or_name() {
        assert_eq!(
            provider_index("Codex").map(|i| agent::PROVIDERS[i].id),
            Some("codex")
        );
        assert_eq!(
            provider_index("claude").map(|i| agent::PROVIDERS[i].id),
            Some("claude")
        );
        assert_eq!(provider_index("skynet"), None);
    }

    #[test]
    fn only_some_settings_change() {
        use serde_json::json;
        assert_eq!(
            setting("theme", &json!("dark")),
            Ok(SettingChange::Theme("dark".into()))
        );
        assert!(setting("theme", &json!("purple")).is_err());
        assert_eq!(
            setting("animations", &json!(false)),
            Ok(SettingChange::Animations(false))
        );
        assert!(setting("default_mode", &json!("full")).is_err());
    }

    #[test]
    fn excerpts_stay_on_one_short_line() {
        assert_eq!(excerpt("a\n  b", 10), "a b");
        assert_eq!(excerpt("abcdef", 3), "abc…");
    }
}
