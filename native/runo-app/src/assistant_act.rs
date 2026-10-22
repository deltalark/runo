//! What the Runo assistant can do in the app, and what it knows of it. The answering
//! agent offers actions as `ACTION:` lines of JSON; each becomes a card the person runs
//! with a click, described from Runo's own state rather than the agent's words, so a card
//! always says what it will really do. Every action goes through the same code the
//! buttons elsewhere in the app use, so the assistant can never do more than the person
//! could by hand.

use super::*;
use serde::Deserialize;

/// The most actions one answer can offer.
pub(crate) const MAX_ACTIONS: usize = 3;
/// The most sessions described to the agent with each question, newest first.
const SESSIONS_SHOWN: usize = 20;

/// Something the assistant offers to do.
#[derive(Clone, Debug, PartialEq, Deserialize)]
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
    /// Change one of a few settings.
    Setting {
        key: String,
        value: serde_json::Value,
    },
}

/// What became of an action card.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Ready,
    Done(String),
    Failed(String),
    Dismissed,
}

/// An `ACTION:` line's action, or nothing when the line isn't one or doesn't parse.
pub(crate) fn parse(line: &str) -> Option<Action> {
    let json = line
        .trim()
        .strip_prefix("ACTION:")?
        .trim()
        .trim_matches('`');
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
            Action::Setting { key, value } => {
                let (title, detail) = match setting(key, value)? {
                    SettingChange::Theme(t) => ("Change the theme".to_string(), t),
                    SettingChange::Animations(on) => (
                        format!("Turn animations {}", if on { "on" } else { "off" }),
                        String::new(),
                    ),
                    SettingChange::DefaultAgent(i) => (
                        "Change the default agent".to_string(),
                        agent::PROVIDERS[i].name.to_string(),
                    ),
                };
                change(IconName::Settings, title, detail)
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
            Action::Setting { key, value } => {
                let change = setting(&key, &value)?;
                settings::update(cx, move |s| match change {
                    SettingChange::Theme(t) => s.theme = t,
                    SettingChange::Animations(on) => s.animations = on,
                    SettingChange::DefaultAgent(i) => {
                        s.default_provider = agent::PROVIDERS[i].id.to_string()
                    }
                });
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
        assert_eq!(parse("Start a race."), None);
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
