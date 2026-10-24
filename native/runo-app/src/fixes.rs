//! Fixing an agent's setup from inside Runo: picking which copy of its CLI runs, updating
//! or removing a copy, setting and testing its default model, signing in from the
//! terminal panel, and the fixes a failed turn offers inline.

use super::*;
use crate::modelcheck::Problem;
use std::path::PathBuf;

/// A copy of an agent's CLI waiting for the person to confirm its removal.
pub(crate) struct RemoveCopy {
    pub provider: &'static str,
    pub path: PathBuf,
    pub real: PathBuf,
    pub owner: Option<String>,
}

fn static_provider(id: &str) -> Option<&'static agent::Provider> {
    agent::PROVIDERS.iter().find(|p| p.id == id)
}

impl RunoApp {
    /// Run this copy of an agent's CLI from now on.
    pub(crate) fn use_copy(
        &mut self,
        provider: &'static str,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let name = agent::provider(provider).name;
        settings::update(cx, |s| {
            s.agents.entry(provider.to_string()).or_default().binary = path.clone();
        });
        self.update_runs.remove(provider);
        self.updates.retain(|u| u.provider != provider);
        caps::refresh();
        self.info = Some((
            format!("{name} now runs {path}."),
            std::time::Instant::now(),
        ));
        cx.notify();
    }

    /// Update the copy Runo runs with the updater on the alerts card. Without a known
    /// release, it updates to the one that wrote the CLI's model list, or the latest.
    pub(crate) fn update_agent(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        if !self.updates.iter().any(|u| u.provider == provider) {
            let caps = caps::get(provider);
            let latest = if alerts::is_newer(&caps.client_version, &caps.version) {
                caps::short_version(&caps.client_version)
            } else {
                "latest".to_string()
            };
            self.updates.push(alerts::AgentUpdate {
                provider,
                current: caps.version.clone(),
                latest,
            });
        }
        self.update_runs.remove(provider);
        self.run_update(provider, false, cx);
        cx.notify();
    }

    /// Ask before removing a link to an agent's CLI; the card names the path.
    pub(crate) fn ask_remove_copy(&mut self, provider: &'static str, copy: &copies::Install) {
        self.remove_copy = Some(RemoveCopy {
            provider,
            path: copy.path.clone(),
            real: copy.real.clone(),
            owner: copy.owner.clone(),
        });
    }

    fn remove_copy_now(&mut self, cx: &mut Context<Self>) {
        let Some(remove) = self.remove_copy.take() else {
            return;
        };
        match copies::remove_link(&remove.path) {
            Ok(()) => {
                let path = remove.path.to_string_lossy().into_owned();
                if cx
                    .global::<AppSettings>()
                    .agent(remove.provider)
                    .binary
                    .trim()
                    == path
                {
                    settings::update(cx, |s| {
                        s.agents
                            .entry(remove.provider.to_string())
                            .or_default()
                            .binary
                            .clear();
                    });
                }
                caps::refresh();
                self.info = Some((format!("Removed {path}."), std::time::Instant::now()));
            }
            Err(e) => self.fail("fixes", e),
        }
        cx.notify();
    }

    /// Set the model Runo passes to an agent when a session picks none, and also in
    /// Codex's config file when that is switched on.
    pub(crate) fn set_default_model(
        &mut self,
        provider: &'static str,
        model: String,
        cx: &mut Context<Self>,
    ) {
        let save = provider == "codex" && cx.global::<AppSettings>().agent(provider).save_to_config;
        settings::update(cx, |s| {
            s.agents
                .entry(provider.to_string())
                .or_default()
                .model_override = model.clone();
        });
        let name = agent::provider(provider).name;
        let shown = if model.is_empty() {
            format!("{name} uses its own default model again.")
        } else {
            format!("{name} now uses {model} by default.")
        };
        if save && !model.is_empty() {
            let saved = modelcheck::save_codex_model(&model);
            caps::refresh();
            match saved {
                Ok(Some(backup)) => {
                    self.info = Some((
                        format!(
                            "{shown} Saved to Codex's config too; the old one is in {}.",
                            backup.display()
                        ),
                        std::time::Instant::now(),
                    ))
                }
                Ok(None) => {
                    self.info = Some((
                        format!("{shown} Saved to Codex's config too."),
                        std::time::Instant::now(),
                    ))
                }
                Err(e) => self.fail("fixes", e),
            }
        } else {
            self.info = Some((shown, std::time::Instant::now()));
        }
        cx.notify();
    }

    /// Test each of an agent's models with a tiny prompt, in the background.
    pub(crate) fn test_models(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let Some(p) = static_provider(provider) else {
            return;
        };
        let caps = caps::get(provider);
        let models: Vec<String> = caps::all_models(&caps)
            .take(MODELS_SHOWN)
            .map(|m| m.choice.value.clone())
            .collect();
        if models.is_empty() {
            self.fail("fixes", format!("{} listed no models to test.", p.name));
            return;
        }
        let env = cx.global::<AppSettings>().agent(provider).env;
        self.info = Some((
            format!("Testing {} {} models, one at a time…", models.len(), p.name),
            std::time::Instant::now(),
        ));
        modelcheck::test_all(p, models, env);
        cx.notify();
    }

    /// Sign an agent in with its own login command, in the terminal panel of the session
    /// on screen; sign-in is checked again when the command exits. Without a session on
    /// this computer, the computer's terminal app opens instead.
    pub(crate) fn sign_in_here(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        // A session on a Cove server signs in there, for every session on it.
        if let Some(id) = self.selected.filter(|id| {
            self.state
                .session(*id)
                .and_then(|s| connections::get(s.connection))
                .is_some_and(|c| c.kind == connections::Kind::Cove)
                && cove::keeps_login(provider)
        }) {
            self.sign_in_on_server(id, provider, cx);
            return;
        }
        let local = self.selected.filter(|id| {
            self.state
                .session(*id)
                .is_some_and(|s| s.sandbox().is_none())
        });
        let Some(session) = local else {
            self.sign_in_agent(provider, cx);
            return;
        };
        let p = agent::provider(provider);
        let bin = install::agent_path(p)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.bin.to_string());
        let line = install::sign_in_command(provider, &bin);
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", &line]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", &line]);
            c
        };
        command.env_remove("CLAUDE_CODE_CHILD_SESSION");
        let dir = self
            .state
            .session(session)
            .map(|s| s.dir.clone())
            .unwrap_or_default();
        self.next_shell += 1;
        let shell_id = self.next_shell;
        let tx = self.tx.clone();
        let (rows, cols) = self.term_grid();
        match pty::Shell::spawn_program(
            shell_id,
            session,
            &dir,
            rows,
            cols,
            Some(pty::Program::of(&command)),
            move |sid| {
                let _ = tx.send(Event::Pty(sid));
            },
        ) {
            Ok(mut shell) => {
                shell.title = format!("Sign in to {}", p.name);
                self.shells.push(shell);
                self.sign_ins.insert(shell_id, provider);
                self.view = View::Session;
                self.show_panel = true;
                self.shell_tab = Some(shell_id);
                self.focus_shell = 2;
                let typed = match provider {
                    "claude" | "copilot" => " Type /login there.",
                    _ => "",
                };
                self.info = Some((
                    format!("Signing in to {} in the terminal panel.{typed} Runo checks again when it finishes.", p.name),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.fail("fixes", format!("Couldn't start `{line}`: {e}"));
            }
        }
        cx.notify();
    }

    /// Sign an agent in on the Cove server session `id` runs on, in a terminal tab of
    /// that session: the agent's own login runs in a small VM there, and the server then
    /// keeps the login for every VM on it.
    pub(crate) fn sign_in_on_server(
        &mut self,
        id: u64,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some((dir, conn)) = self
            .state
            .session(id)
            .and_then(|s| Some((s.dir.clone(), connections::get(s.connection)?)))
        else {
            return;
        };
        let p = agent::provider(provider);
        let command = match cove::sign_in_command(&conn, provider) {
            Ok(c) => c,
            Err(e) => {
                self.fail("fixes", e);
                cx.notify();
                return;
            }
        };
        self.next_shell += 1;
        let shell_id = self.next_shell;
        let tx = self.tx.clone();
        let (rows, cols) = self.term_grid();
        match pty::Shell::spawn_program(
            shell_id,
            id,
            &dir,
            rows,
            cols,
            Some(pty::Program::of(&command)),
            move |sid| {
                let _ = tx.send(Event::Pty(sid));
            },
        ) {
            Ok(mut shell) => {
                shell.title = format!("Sign in to {} on {}", p.name, conn.name);
                self.shells.push(shell);
                self.sign_ins.insert(shell_id, provider);
                self.server_sign_ins.insert(shell_id, id);
                self.selected = Some(id);
                self.view = View::Session;
                self.show_panel = true;
                self.shell_tab = Some(shell_id);
                self.focus_shell = 2;
                self.info = Some((
                    format!(
                        "Signing in to {} on {} in the terminal panel. Follow its steps there; Cove keeps the login for every session on {}.",
                        p.name, conn.name, conn.name
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => self.fail("fixes", format!("Couldn't start Cove's sign-in: {e}")),
        }
        cx.notify();
    }

    /// Sign in on a Cove connection from Settings: in the terminal panel of the session on
    /// screen when it runs there, otherwise in the computer's terminal app.
    pub(crate) fn sign_in_on_connection(
        &mut self,
        conn: u64,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected.filter(|id| {
            self.state
                .session(*id)
                .is_some_and(|s| s.connection == conn)
        }) {
            self.sign_in_on_server(id, provider, cx);
            return;
        }
        let Some(c) = connections::get(conn) else {
            return;
        };
        let name = agent::provider(provider).name;
        let line = match cove::sign_in_command(&c, provider) {
            Ok(command) => {
                let mut parts: Vec<String> = command
                    .get_envs()
                    .filter_map(|(k, v)| {
                        Some(format!(
                            "{}={}",
                            k.to_string_lossy(),
                            connections::quote(&v?.to_string_lossy())
                        ))
                    })
                    .collect();
                parts.push(connections::quote(&command.get_program().to_string_lossy()));
                parts.extend(
                    command
                        .get_args()
                        .map(|a| connections::quote(&a.to_string_lossy())),
                );
                parts.join(" ")
            }
            Err(e) => {
                self.fail("fixes", e);
                cx.notify();
                return;
            }
        };
        cove::forget_logins();
        self.info = Some((
            match install::open_in_terminal(&line) {
                Ok(()) => format!(
                    "Sign in to {name} in the terminal window. Cove keeps the login for every session on {}.",
                    c.name
                ),
                Err(e) => format!("Couldn't open a terminal: {e}"),
            },
            std::time::Instant::now(),
        ));
        cx.notify();
    }

    /// Copy this Mac's login for an agent to a Cove connection's server, in the background.
    pub(crate) fn import_login_to(
        &mut self,
        conn: u64,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(c) = connections::get(conn) else {
            return;
        };
        let name = agent::provider(provider).name;
        let place = c.name.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let _ = tx.send(cove::import_login(&c, provider));
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let answer = match rx.try_recv() {
                Ok(a) => a,
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Err(_) => Err("Copying the login stopped.".into()),
            };
            let _ = this.update(cx, |this, cx| {
                match answer {
                    Ok(()) => {
                        this.info = Some((
                            format!("{place} uses this Mac's {name} login now, in every VM."),
                            std::time::Instant::now(),
                        ))
                    }
                    Err(e) => this.fail("fixes", format!("Couldn't copy the {name} login: {e}")),
                }
                cx.notify();
            });
            break;
        })
        .detach();
    }

    /// Sign an agent out on a Cove connection's server and its VMs, in the background.
    pub(crate) fn sign_out_on_connection(
        &mut self,
        conn: u64,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(c) = connections::get(conn) else {
            return;
        };
        let name = agent::provider(provider).name;
        let place = c.name.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let _ = tx.send(cove::sign_out(&c, provider));
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let answer = match rx.try_recv() {
                Ok(a) => a,
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Err(_) => Err("Signing out stopped.".into()),
            };
            let _ = this.update(cx, |this, cx| {
                match answer {
                    Ok(()) => {
                        this.info = Some((
                            format!("Signed out of {name} on {place} and its VMs."),
                            std::time::Instant::now(),
                        ))
                    }
                    Err(e) => this.fail("fixes", format!("Couldn't sign out of {name}: {e}")),
                }
                cx.notify();
            });
            break;
        })
        .detach();
    }

    /// A shell exited: when it was signing an agent in, check sign-in again.
    pub(crate) fn sign_in_exited(&mut self, shell: u64) {
        if !self.shells.iter().any(|s| s.id == shell && s.exited()) {
            return;
        }
        if let Some(session) = self.server_sign_ins.remove(&shell) {
            let provider = self.sign_ins.remove(&shell).unwrap_or_default();
            cove::forget_logins();
            let ok = self
                .shells
                .iter()
                .find(|s| s.id == shell)
                .and_then(|s| s.success())
                .unwrap_or(false);
            let name = agent::provider(provider).name;
            let place = self
                .state
                .session(session)
                .map(|s| connections::name(s.connection))
                .unwrap_or_default();
            self.info = Some((
                if ok {
                    format!("{name} is signed in on {place}. Send your message again.")
                } else {
                    format!("{name} didn't sign in on {place}. The terminal tab says why.")
                },
                std::time::Instant::now(),
            ));
            return;
        }
        if let Some(provider) = self.sign_ins.remove(&shell) {
            signin::refresh(true);
            caps::refresh();
            self.info = Some((
                format!(
                    "Checking whether {} is signed in now…",
                    agent::provider(provider).name
                ),
                std::time::Instant::now(),
            ));
        }
    }

    /// Run the last prompt again with another model, which the session keeps.
    pub(crate) fn retry_with_model(&mut self, id: u64, model: String, cx: &mut Context<Self>) {
        if let Some(s) = self.state.session_mut(id) {
            s.set_model(model);
        }
        self.fix_models = None;
        self.save();
        self.retry_last(id, cx);
        cx.notify();
    }

    /// Make `model` the default of this session's project, or of every session.
    fn make_default(&mut self, id: u64, model: String, project: bool, cx: &mut Context<Self>) {
        let Some((provider, project_id)) = self
            .state
            .session(id)
            .and_then(|s| Some((static_provider(&s.provider)?.id, s.project_id)))
        else {
            return;
        };
        if project {
            settings::update(cx, |s| {
                s.agents
                    .entry(provider.to_string())
                    .or_default()
                    .project_models
                    .insert(project_id.to_string(), model.clone());
            });
            let name = self
                .state
                .projects
                .iter()
                .find(|p| p.id == project_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            self.info = Some((
                format!(
                    "{model} is now {}'s default in {name}.",
                    agent::provider(provider).name
                ),
                std::time::Instant::now(),
            ));
        } else {
            self.set_default_model(provider, model, cx);
        }
        cx.notify();
    }

    /// A failed turn on a Cove server whose agent isn't signed in there: sign in there.
    fn server_sign_in_fix(&self, id: u64, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.state.session(id)?;
        let conn = connections::get(session.connection)?;
        let p = static_provider(&session.provider)?;
        if conn.kind != connections::Kind::Cove || !cove::keeps_login(p.id) {
            return None;
        }
        let text = session
            .messages
            .iter()
            .rev()
            .take_while(|m| m.role != Role::User)
            .filter(|m| m.role == Role::System && m.meta.is_empty())
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let caps = caps::get(p.id);
        let problem = modelcheck::classify(p.id, &text, &caps.version, &caps.client_version)?;
        if !matches!(problem, Problem::SignedOut) {
            return None;
        }
        let provider = p.id;
        let why = match cove::signed_in_there(&conn, provider) {
            Some(true) => format!(
                "{} is signed in on {} now. Send your message again.",
                p.name, conn.name
            ),
            _ => format!(
                "{} isn't signed in on {}. Sign in once there, and every session on it uses that login.",
                p.name, conn.name
            ),
        };
        let button = self
            .button(
                "fix-sign-in-server",
                format!("Sign in on {}", conn.name),
                true,
            )
            .h(px(24.))
            .px_2()
            .text_size(px(12.))
            .on_click(cx.listener(move |this, _, _, cx| this.sign_in_on_server(id, provider, cx)));
        Some(
            div()
                .v_flex()
                .gap_1()
                .px_2()
                .pb_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .child(why),
                )
                .child(div().flex().flex_wrap().gap_1().child(button)),
        )
    }

    /// What a failed turn can do about its agent's setup, shown under "The last turn
    /// didn't finish": sign in, another model, another copy of the CLI, or an update.
    /// A turn the plan's usage limit cut short: carry on from where it stopped once the
    /// window resets, or now. Carrying on keeps what the turn did; Retry would redo it.
    fn limit_fixes(
        &self,
        id: u64,
        name: &str,
        provider: &str,
        limit: &crate::planlimit::PlanLimit,
        cx: &mut Context<Self>,
    ) -> Div {
        let now = usage::now_secs();
        let until = crate::planlimit::held_until(provider, now).or(limit.resets_at);
        let window = crate::planlimit::window_name(&limit.kind);
        let queued = self
            .state
            .session(id)
            .is_some_and(|s| s.queue.iter().any(|q| q.text == crate::planlimit::CONTINUE));
        let why = match (until, queued) {
            (Some(t), true) => format!(
                "{name}'s {window} limit is used up. This session carries on from where it stopped at {}.",
                crate::planlimit::reset_clock(t, now)
            ),
            (Some(t), false) => format!(
                "{name}'s {window} limit is used up until {}. Carry on keeps what this turn already did.",
                crate::planlimit::reset_clock(t, now)
            ),
            (None, _) => format!(
                "{name}'s {window} limit is used up. Carry on keeps what this turn already did."
            ),
        };
        let small = |el: Stateful<Div>| el.h(px(24.)).px_2().text_size(px(12.));
        let mut row = div().flex().flex_wrap().gap_1();
        if !queued {
            let label = match until {
                Some(t) => format!("Carry on at {}", crate::planlimit::reset_clock(t, now)),
                None => "Carry on when it resets".to_string(),
            };
            row = row.child(
                small(self.button("fix-continue-later", label, true)).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.continue_after_limit(id, false, cx);
                        cx.notify();
                    },
                )),
            );
        }
        row = row.child(
            small(self.button("fix-continue-now", "Carry on now", queued)).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.continue_after_limit(id, true, cx);
                    cx.notify();
                },
            )),
        );
        div()
            .v_flex()
            .gap_1()
            .px_2()
            .pb_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(why),
            )
            .child(row)
    }

    pub(crate) fn turn_fixes(&self, id: u64, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.state.session(id)?;
        if session.elsewhere() {
            return self.server_sign_in_fix(id, cx);
        }
        let p = static_provider(&session.provider)?;
        let provider = p.id;
        let text = session
            .messages
            .iter()
            .rev()
            .take_while(|m| m.role != Role::User)
            .filter(|m| m.role == Role::System && m.meta.is_empty())
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(limit) = crate::planlimit::from_text(&text, chrono::Local::now()) {
            return Some(self.limit_fixes(id, p.name, provider, &limit, cx));
        }
        let caps = caps::get(provider);
        let problem = modelcheck::classify(provider, &text, &caps.version, &caps.client_version)?;
        let chosen = session
            .selections()
            .get("model")
            .cloned()
            .unwrap_or_default();
        let opts = cx
            .global::<AppSettings>()
            .agent(provider)
            .for_project(session.project_id);
        let failing = match &problem {
            Problem::NoAccess(m) | Problem::NeedsNewer(m) if !m.is_empty() => m.clone(),
            _ if !chosen.is_empty() => chosen.clone(),
            _ => opts.model_override.clone(),
        };
        let name = p.name;
        let version = caps::short_version(&caps.version);
        let small = |el: Stateful<Div>| el.h(px(24.)).px_2().text_size(px(12.));
        let mut why;
        let mut row = div().flex().flex_wrap().gap_1();
        let mut primary = true;
        let mut first = || std::mem::replace(&mut primary, false);
        let alternative = modelcheck::alternative(provider, &failing);
        let newer = copies::newer_than(p, &caps.version);
        let owner = updater::resolve(&caps::binary(p))
            .and_then(|path| std::fs::canonicalize(path).ok())
            .and_then(|real| updater::owner_app(&real));
        let retry_with = |row: Div, model: String, primary: bool| {
            row.child(
                small(self.button(
                    SharedString::from(format!("fix-retry-{model}")),
                    format!("Retry with {model}"),
                    primary,
                ))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.retry_with_model(id, model.clone(), cx)),
                ),
            )
        };
        let update = |row: Div, primary: bool| {
            row.child(
                small(self.button("fix-update", format!("Update {name}"), primary))
                    .child(icon(
                        IconName::Download,
                        11.,
                        if primary { c_canvas() } else { c_secondary() },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.update_agent(provider, cx))),
            )
        };
        match &problem {
            Problem::SignedOut => {
                why = format!("{name} isn't signed in.");
                row = row.child(
                    small(self.button("fix-sign-in", "Sign in", first())).on_click(
                        cx.listener(move |this, _, _, cx| this.sign_in_here(provider, cx)),
                    ),
                );
            }
            Problem::NeedsNewer(_) => {
                why = format!(
                    "{name} {version} is too old for {}.",
                    if failing.is_empty() {
                        "this model"
                    } else {
                        &failing
                    }
                );
                if let Some(copy) = newer.clone() {
                    let path = copy.path.to_string_lossy().into_owned();
                    row = row.child(
                        small(self.button(
                            "fix-use-copy",
                            format!("Use {name} {} and retry", copy.version),
                            first(),
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.use_copy(provider, path.clone(), cx);
                            this.retry_last(id, cx);
                        })),
                    );
                }
                if let Some(app) = &owner {
                    why.push_str(&format!(
                        " This copy comes with {app}, so only {app} can update it."
                    ));
                    if alerts::npm_package(provider).is_some() {
                        row = row.child(
                            small(self.button("fix-own-copy", "Install for Runo", first()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.install_own_copy(provider, cx);
                                    cx.notify();
                                })),
                        );
                    }
                } else {
                    row = update(row, first());
                }
                if let Some(model) = alternative.clone() {
                    row = retry_with(row, model, first());
                }
            }
            Problem::NoAccess(_) => {
                why = if failing.is_empty() {
                    format!("Your {name} account can't use this model.")
                } else {
                    format!("Your {name} account can't use {failing}.")
                };
                match alternative.clone() {
                    Some(model) => row = retry_with(row, model, first()),
                    None if !chosen.is_empty() => {
                        row = row.child(
                            small(self.button(
                                "fix-default",
                                format!("Retry with {name}'s default"),
                                first(),
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.retry_with_model(id, String::new(), cx),
                            )),
                        );
                    }
                    None => {}
                }
                if let Some(copy) = newer.clone() {
                    let path = copy.path.to_string_lossy().into_owned();
                    row = row.child(
                        small(self.button(
                            "fix-use-copy",
                            format!("Use {name} {}", copy.version),
                            false,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.use_copy(provider, path.clone(), cx)
                        })),
                    );
                }
                if owner.is_none() {
                    row = update(row, false);
                }
            }
        }
        if !matches!(problem, Problem::SignedOut) {
            let open = self.fix_models == Some(id);
            row = row.child(
                small(self.button(
                    "fix-pick",
                    if open {
                        "Hide models"
                    } else {
                        "Pick a model…"
                    },
                    false,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.fix_models = if this.fix_models == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                    cx.notify();
                })),
            );
        }
        let mut col = div()
            .v_flex()
            .gap_1()
            .px_2()
            .pb_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(why),
            )
            .child(row);
        if !matches!(problem, Problem::SignedOut) && self.fix_models == Some(id) {
            let today = store::today();
            let mut list = div().flex().flex_wrap().gap_1();
            for m in caps::all_models(&caps).take(MODELS_SHOWN) {
                let value = m.choice.value.clone();
                let note = match modelcheck::verdict(provider, &value) {
                    Some(v) => v.label().to_string(),
                    None => caps::model_note(provider, m, &caps.version, &today),
                };
                let label = if note.is_empty() {
                    m.choice.label.clone()
                } else {
                    format!("{} · {note}", m.choice.label)
                };
                list = list.child(
                    small(self.button(
                        SharedString::from(format!("fix-model-{value}")),
                        label,
                        false,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.retry_with_model(id, value.clone(), cx)
                    })),
                );
            }
            col = col.child(list).child(
                div().flex().child(
                    small(self.button(
                        "fix-test",
                        if modelcheck::testing(provider) {
                            "Testing…"
                        } else {
                            "Test models"
                        },
                        false,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.test_models(provider, cx))),
                ),
            );
        }
        // The model that worked can become the default from here too.
        if let Some(model) = alternative.filter(|_| !matches!(problem, Problem::SignedOut)) {
            let (project, everywhere) = (model.clone(), model.clone());
            col = col.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child(format!("Make {model} the default for"))
                    .child(
                        small(self.button("fix-default-project", "This project", false)).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.make_default(id, project.clone(), true, cx)
                            }),
                        ),
                    )
                    .child(
                        small(self.button(
                            "fix-default-all",
                            format!("All {name} sessions"),
                            false,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.make_default(id, everywhere.clone(), false, cx)
                        })),
                    ),
            );
        }
        Some(col)
    }

    /// A line over the composer when the model a session uses is being retired, with a
    /// one-click switch to the model the CLI suggests instead.
    pub(crate) fn retiring_bar(&self, id: u64, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.state.session(id)?;
        let p = static_provider(&session.provider)?;
        let opts = cx
            .global::<AppSettings>()
            .agent(p.id)
            .for_project(session.project_id);
        let model = session
            .selections()
            .get("model")
            .cloned()
            .filter(|m| !m.is_empty())
            .or_else(|| Some(opts.model_override).filter(|m| !m.is_empty()))
            .or_else(|| Some(caps::get(p.id).config_model).filter(|m| !m.is_empty()))?;
        let key = format!("retire:{id}:{model}");
        if self.alerts_hidden.contains(&key) {
            return None;
        }
        let caps = caps::get(p.id);
        let info = caps::all_models(&caps).find(|m| m.choice.value == model)?;
        if info.retires.is_empty() || info.upgrade_to.is_empty() {
            return None;
        }
        let note = caps::model_note(p.id, info, &caps.version, &store::today());
        let to = info.upgrade_to.clone();
        let to_label = caps::all_models(&caps)
            .find(|m| m.choice.value == to)
            .map(|m| m.choice.label.clone())
            .unwrap_or_else(|| to.clone());
        Some(
            div()
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .bg(tint(c_subtle()))
                .text_size(px(12.5))
                .child(icon(IconName::TriangleAlert, 12., c_amber()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(c_secondary()))
                        .child(format!("{}: {note}.", info.choice.label)),
                )
                .child(
                    self.button("retire-switch", format!("Switch to {to_label}"), false)
                        .h(px(22.))
                        .px_2()
                        .text_size(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(s) = this.state.session_mut(id) {
                                s.set_model(to.clone());
                            }
                            this.save();
                            cx.notify();
                        })),
                )
                .child(
                    self.button("retire-later", "Not now", false)
                        .h(px(22.))
                        .px_2()
                        .text_size(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.alerts_hidden.insert(key.clone());
                            cx.notify();
                        })),
                ),
        )
    }

    /// The confirm card for removing a link to an agent's CLI.
    pub(crate) fn remove_copy_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let remove = self.remove_copy.as_ref()?;
        let p = agent::provider(remove.provider);
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let keeps = match &remove.owner {
            Some(app) => format!("The file it leads to stays, so {app} keeps working:"),
            None => "The file it leads to stays:".to_string(),
        };
        let mono = |text: String| {
            div()
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(tint(c_subtle()))
                .border_1()
                .border_color(divider())
                .font_family(self.mono_font.clone())
                .text_size(px(12.))
                .child(text)
        };
        let card = div()
            .id("remove-copy-card")
            .w(px(480.))
            .max_w(relative(0.94))
            .p_6()
            .v_flex()
            .gap_3()
            .rounded(px(crate::shell::ISLAND_R))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.provider_badge(p, 36.))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Remove this copy of {}?", p.name)),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(c_secondary()))
                    .child("Runo deletes this link:"),
            )
            .child(mono(remove.path.display().to_string()))
            .child(div().text_sm().text_color(rgb(c_secondary())).child(keeps))
            .child(mono(remove.real.display().to_string()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(self.button("remove-copy-cancel", "Cancel", false).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.remove_copy = None;
                            cx.notify();
                        }),
                    ))
                    .child(
                        self.button("remove-copy-go", "Remove link", true)
                            .on_click(cx.listener(|this, _, _, cx| this.remove_copy_now(cx))),
                    ),
            );
        Some(
            fade(
                div()
                    .id("remove-copy-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(backdrop)
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.remove_copy = None;
                        cx.notify();
                    }))
                    .child(rise(card, "remove-copy-rise", 300)),
                "remove-copy-fade",
                200,
            )
            .into_any_element(),
        )
    }
}

/// How many models Settings lists, tests and offers on a failed turn.
pub(crate) const MODELS_SHOWN: usize = 40;
