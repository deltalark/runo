//! Workspace-level features: sidebar status groups, saved layout, quick commands,
//! project notes, the activity feed and playbooks.

use super::*;

impl RunoApp {
    /// Remember open tabs and panels so the next launch looks the same.
    pub(crate) fn store_layout(&mut self) {
        self.state.layout = store::Layout {
            tabs: {
                let mut seen = std::collections::HashSet::new();
                self.tabs
                    .iter()
                    .copied()
                    .filter(|id| seen.insert(*id))
                    .collect()
            },
            selected: self.selected,
            hide_sidebar: !self.show_sidebar,
            hide_panel: !self.show_panel,
        };
    }

    pub(crate) fn restore_layout(&mut self) {
        let layout = self.state.layout.clone();
        let exists = |id: &u64| self.state.session(*id).is_some();
        let tabs: Vec<u64> = layout.tabs.into_iter().filter(exists).collect();
        if !tabs.is_empty() {
            self.tabs = tabs;
        }
        let mut seen = std::collections::HashSet::new();
        self.tabs.retain(|id| seen.insert(*id));
        if let Some(id) = layout.selected.filter(exists) {
            self.selected = Some(id);
            if !self.tabs.contains(&id) {
                self.tabs.push(id);
            }
        }
        self.show_sidebar = !layout.hide_sidebar;
        if layout.hide_panel {
            self.show_panel = false;
            // Keep the window-width rule from opening it again on the first frame.
            self.panel_wide = Some(true);
        }
    }

    /// Sessions grouped by what needs attention, instead of by project.
    pub(crate) fn status_groups(&self, filter: &str, cx: &mut Context<Self>) -> Div {
        let visible: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| !s.archived)
            .filter(|s| {
                filter.is_empty()
                    || s.title.to_lowercase().contains(filter)
                    || s.branch.to_lowercase().contains(filter)
            })
            .collect();
        let group = |s: &Session| -> usize {
            if s.pinned {
                0
            } else if s.status == Status::Working {
                1
            } else if s.unread || s.status == Status::Failed {
                2
            } else {
                3
            }
        };
        let names = ["Pinned", "Working", "Needs review", "Done"];
        let mut col = div().v_flex().gap(px(2.)).pb_2();
        for (g, name) in names.iter().enumerate() {
            let members: Vec<&Session> =
                visible.iter().copied().filter(|s| group(s) == g).collect();
            if members.is_empty() {
                continue;
            }
            col = col.child(
                div()
                    .mt_2()
                    .mx_1()
                    .px_2()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_secondary()))
                    .child(*name)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(members.len().to_string()),
                    ),
            );
            for s in members {
                col = col.child(self.session_row(s, cx));
            }
        }
        col
    }

    /// Saved commands shown as buttons over the Terminal.
    pub(crate) fn quick_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let commands = settings::parse_quick_commands(&cx.global::<AppSettings>().quick_commands);
        if commands.is_empty() {
            return None;
        }
        let mut row = div().flex().items_center().gap_1().overflow_hidden();
        for (i, (name, command)) in commands.into_iter().enumerate().take(6) {
            let tip = command.clone();
            row = row.child(
                div()
                    .id(("quick", i))
                    .h(px(22.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_active())).text_color(rgb(c_text())))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cli_tab = false;
                        this.run_terminal(command.clone(), cx);
                        cx.notify();
                    }))
                    .child(icon(IconName::Play, 10., c_muted()))
                    .child(name),
            );
        }
        Some(row.into_any_element())
    }

    // ---------- Notes ----------

    pub(crate) fn notes_project(&self) -> Option<u64> {
        self.notes_project
            .filter(|id| self.state.projects.iter().any(|p| p.id == *id))
            .or_else(|| self.current().map(|s| s.project_id))
            .or_else(|| self.state.projects.first().map(|p| p.id))
    }

    pub(crate) fn open_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view = View::Notes;
        let text = self
            .notes_project()
            .and_then(|id| self.state.notes.get(&id).cloned())
            .unwrap_or_default();
        self.loading_notes = true;
        self.notes_input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        self.loading_notes = false;
    }

    pub(crate) fn notes_changed(&mut self, cx: &mut Context<Self>) {
        if self.loading_notes {
            return;
        }
        if let Some(id) = self.notes_project() {
            let text = self.notes_input.read(cx).value().to_string();
            if text.trim().is_empty() {
                self.state.notes.remove(&id);
            } else {
                self.state.notes.insert(id, text);
            }
            self.state.notes_meta.entry(id).or_default().updated = unix_now();
            self.save();
        }
    }

    // ---------- Activity ----------

    pub(crate) fn activity_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().v_flex();
        let mut day = String::new();
        for (i, event) in self.state.activity.iter().rev().take(300).enumerate() {
            let (date, time) = event.time.split_once(' ').unwrap_or(("", &event.time));
            if date != day {
                day = date.to_string();
                rows = rows.child(
                    div()
                        .mt_4()
                        .mb_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_muted()))
                        .child(day.clone()),
                );
            }
            let sid = event.session;
            let exists = self.state.session(sid).is_some();
            let t = event.text.as_str();
            let (mark, color) = match () {
                _ if t.starts_with("Failed") => (IconName::CircleX, c_red()),
                _ if t.starts_with("Finished") => (IconName::CircleCheck, c_green()),
                _ if t.starts_with("Stopped") => (IconName::CircleStop, c_amber()),
                _ if t.starts_with("Started") => (IconName::Play, c_blue()),
                _ if t.starts_with("Committed") => (IconName::GitCommitHorizontal, c_green()),
                _ if t.starts_with("Updated") => (IconName::Download, c_green()),
                _ if t.starts_with("Pushed") => (IconName::CloudUpload, c_blue()),
                _ if t.starts_with("Undid") => (IconName::Undo2, c_amber()),
                _ if t.starts_with("Discarded") => (IconName::Trash, c_amber()),
                _ if t.starts_with("Edited") => (IconName::Pencil, c_secondary()),
                _ if t.starts_with("Automation") => (IconName::Timer, c_blue()),
                _ => (IconName::Dot, c_muted()),
            };
            let provider = self
                .state
                .session(sid)
                .map(|s| agent::provider(&s.provider).name)
                .unwrap_or("");
            rows = rows.child(stagger(
                div()
                    .id(("activity", i))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_md()
                    .when(exists, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select(sid);
                                cx.notify();
                            }))
                    })
                    .child(
                        div()
                            .w(px(40.))
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(time.to_string()),
                    )
                    .child(icon(mark, 13., color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .child(event.text.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(provider),
                    ),
                ("activity-in", i),
                i,
            ));
        }
        if self.state.activity.is_empty() {
            rows = rows.child(empty_hint(
                "activity-empty",
                IconName::Activity,
                "No activity yet",
                "Agent runs, commits, undos and edits show up here as they happen.",
            ));
        }
        self.page("Timeline", rows)
    }

    // ---------- Playbooks ----------

    /// Start every scheduled playbook whose time has come. Called on a timer. A playbook
    /// whose last run is still working waits for the next time, and runs missed while
    /// Runo was closed become one run that says so.
    pub(crate) fn run_due_automations(&mut self, cx: &mut Context<Self>) {
        if self.demo {
            return;
        }
        let now = unix_now();
        let due: Vec<(u64, u64)> = self
            .state
            .automations
            .iter()
            .filter(|a| a.due(now))
            .map(|a| (a.id, (now - a.last_run) / (a.every_minutes * 60).max(1)))
            .collect();
        for (id, intervals) in due {
            let busy = self.state.sessions.iter().any(|s| {
                s.playbook == id
                    && (s.status == Status::Working
                        || self.handles.contains_key(&Target::Session(s.id))
                        || self.pending_start.contains_key(&s.id))
            });
            if busy {
                if let Some(a) = self.state.automations.iter_mut().find(|a| a.id == id) {
                    a.last_run = now;
                    a.record(store::PlaybookRun {
                        at: now,
                        status: "skipped".into(),
                        note: "The last run was still working.".into(),
                        ..Default::default()
                    });
                }
                self.save();
                continue;
            }
            let note = (intervals >= 2).then(|| {
                let missed = intervals - 1;
                format!(
                    "{missed} earlier run{} {} missed while Runo was closed; this one covers {}.",
                    if missed == 1 { "" } else { "s" },
                    if missed == 1 { "was" } else { "were" },
                    if missed == 1 { "it" } else { "them" }
                )
            });
            // On a timer nobody is there to fill in inputs; their defaults are used.
            self.start_playbook(id, None, None, note, cx);
            self.prune_playbook_runs(id);
        }
    }

    /// Delete the oldest runs of a scheduled playbook past the number Settings keeps.
    /// Pinned, unread and working runs stay.
    fn prune_playbook_runs(&mut self, playbook: u64) {
        let keep = settings::playbook_keep_runs();
        if keep == 0 {
            return;
        }
        let mut runs: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.playbook == playbook)
            .map(|s| s.id)
            .collect();
        runs.sort_unstable();
        let old = runs.len().saturating_sub(keep);
        for id in runs.into_iter().take(old) {
            let removable = self.state.session(id).is_some_and(|s| {
                !s.pinned && !s.unread && s.status != Status::Working
            }) && !self.handles.contains_key(&Target::Session(id));
            if removable {
                self.delete_session(id);
            }
        }
    }

    /// A playbook's session finished a turn: note how its run went.
    pub(crate) fn playbook_turn_done(&mut self, id: u64, ok: bool) {
        let Some(playbook) = self.state.session(id).map(|s| s.playbook).filter(|p| *p != 0)
        else {
            return;
        };
        let error = (!ok)
            .then(|| {
                self.state.session(id).and_then(|s| {
                    s.messages
                        .iter()
                        .rev()
                        .find(|m| m.role == Role::System && m.meta.is_empty())
                        .map(|m| m.text.lines().next().unwrap_or("").to_string())
                })
            })
            .flatten();
        if let Some(run) = self
            .state
            .automations
            .iter_mut()
            .find(|a| a.id == playbook)
            .and_then(|a| a.runs.iter_mut().rev().find(|r| r.session == id))
        {
            run.status = if ok { "done" } else { "failed" }.into();
            if let Some(error) = error {
                run.note = error;
            }
        }
    }

    /// The project a playbook started by hand runs in: the open session's, the one picked
    /// for a new session, or the first.
    pub(crate) fn current_project(&self) -> Option<u64> {
        self.current()
            .map(|s| s.project_id)
            .or(self.new_project)
            .filter(|id| self.state.projects.iter().any(|p| p.id == *id))
            .or_else(|| self.state.projects.first().map(|p| p.id))
    }

    /// Start the playbooks agents asked for through Runo's MCP server.
    pub(crate) fn take_playbook_requests(&mut self, cx: &mut Context<Self>) {
        if self.demo {
            return;
        }
        let ask = cx.global::<AppSettings>().confirm_agent_playbooks;
        for request in store::take_requests(&store::inbox_dir()) {
            let Some(auto) = self
                .state
                .automations
                .iter()
                .find(|a| a.id == request.playbook)
            else {
                continue;
            };
            // An agent told only to plan can't get around that through a playbook.
            let planning = self
                .state
                .sessions
                .iter()
                .find(|s| !request.dir.is_empty() && s.dir == request.dir)
                .is_some_and(|s| {
                    s.opts.get("mode").is_some_and(|m| m == "plan")
                        || (!s.limits.is_empty() && !s.limits.iter().any(|p| p == "edit"))
                });
            if planning && (auto.may("edit") || auto.may("shell")) {
                self.notice = Some(format!(
                    "{} is only planning, so it can't start “{}”, which may edit files or run commands.",
                    request.from, auto.name
                ));
                cx.notify();
                continue;
            }
            if ask {
                self.pb_asks.push(request);
            } else {
                self.start_playbook(
                    request.playbook,
                    Some(request.project),
                    Some(&request.from),
                    None,
                    cx,
                );
            }
            cx.notify();
        }
    }

    /// Answer the first agent request waiting for confirmation.
    pub(crate) fn answer_playbook_ask(&mut self, start: bool, cx: &mut Context<Self>) {
        if self.pb_asks.is_empty() {
            return;
        }
        let request = self.pb_asks.remove(0);
        if start {
            self.start_playbook(
                request.playbook,
                Some(request.project),
                Some(&request.from),
                None,
                cx,
            );
        }
        cx.notify();
    }

    /// The card asking whether an agent may start a playbook.
    pub(crate) fn playbook_ask_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let request = self.pb_asks.first()?;
        let auto = self
            .state
            .automations
            .iter()
            .find(|a| a.id == request.playbook)?;
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == request.project)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "its project".into());
        let may = if auto.permissions.is_empty() {
            "It declares no limits, so it runs like any session.".to_string()
        } else {
            match crate::playbooks::permission_summary(&auto.permissions) {
                None => "It only reads files.".into(),
                Some(text) => format!("It may: {}.", text.to_lowercase()),
            }
        };
        let warning = limits_warning(&auto.provider, &auto.permissions);
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let card = div()
            .id("pb-ask")
            .w(px(440.))
            .max_w(relative(0.94))
            .p_5()
            .v_flex()
            .gap_3()
            .rounded_2xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{} wants to start a playbook", request.from)),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .child(format!("“{}” in {project}, as a new session. {may}", auto.name)),
            )
            .when_some(warning, |el, w| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(c_amber()))
                        .child(icon(IconName::ShieldAlert, 13., c_amber()))
                        .child(w),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("pb-ask-no", "Don't start", false)
                            .on_click(cx.listener(|this, _, _, cx| this.answer_playbook_ask(false, cx))),
                    )
                    .child(
                        self.button("pb-ask-yes", "Start it", true)
                            .child(icon(IconName::Play, 11., c_canvas()))
                            .on_click(cx.listener(|this, _, _, cx| this.answer_playbook_ask(true, cx))),
                    ),
            );
        Some(
            div()
                .id("pb-ask-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(backdrop)
                .occlude()
                .child(rise(card, "pb-ask-rise", 260))
                .into_any_element(),
        )
    }

    /// Run a playbook as a new session: in its own project when it has one, else in `in_project`.
    pub(crate) fn run_playbook(
        &mut self,
        playbook: u64,
        in_project: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if self.demo_guard(cx) {
            return;
        }
        // Inputs to fill in, or limits the agent can't be held to, are shown first.
        let asks = self
            .state
            .automations
            .iter()
            .find(|a| a.id == playbook)
            .is_some_and(|a| {
                !a.run_inputs().is_empty() || limits_warning(&a.provider, &a.permissions).is_some()
            });
        let filled = self
            .pb_values
            .as_ref()
            .is_some_and(|(id, _)| *id == playbook);
        if asks && !filled {
            self.ask_playbook_inputs(playbook, in_project, false);
            return;
        }
        self.start_playbook(playbook, in_project, None, None, cx);
    }

    /// Start a playbook; `from` names the agent that asked, which leaves the person's view
    /// where it is. `note` goes into its run history. It runs on the project's default
    /// connection, after the worktree setup command.
    fn start_playbook(
        &mut self,
        playbook: u64,
        in_project: Option<u64>,
        from: Option<&str>,
        note: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut auto) = self
            .state
            .automations
            .iter()
            .find(|a| a.id == playbook)
            .cloned()
        else {
            return;
        };
        let values = match self.pb_values.take() {
            Some((id, values)) if id == playbook => values,
            _ => Vec::new(),
        };
        auto.prompt = crate::pbfile::fill(&auto.prompt, &auto.run_inputs(), &values);
        let now = unix_now();
        if let Some(a) = self.state.automations.iter_mut().find(|a| a.id == playbook) {
            a.last_run = now;
            a.ran = now;
        }
        let record = |this: &mut Self, run: store::PlaybookRun| {
            if let Some(a) = this.state.automations.iter_mut().find(|a| a.id == playbook) {
                a.record(run);
            }
        };
        let target = Some(auto.project_id)
            .filter(|id| *id != 0)
            .or(in_project)
            .or_else(|| self.current_project());
        let Some(project) = target
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            self.notice = Some(format!(
                "Playbook “{}”: add a project to run it in.",
                auto.name
            ));
            record(
                self,
                store::PlaybookRun {
                    at: now,
                    status: "not started".into(),
                    note: "There was no project to run it in.".into(),
                    ..Default::default()
                },
            );
            self.save();
            return;
        };
        let provider = agent::PROVIDERS
            .iter()
            .position(|p| p.id == auto.provider)
            .unwrap_or(0);
        let is_git = git::is_repo(&project.path);
        match self.spawn_session(&project, provider, "", is_git, is_git, &auto.prompt, cx) {
            Ok(id) => {
                let connection = self.project_connection(project.id);
                if let Some(s) = self.state.session_mut(id) {
                    s.title = format!(
                        "{} · {}",
                        auto.name,
                        chrono::Local::now().format("%b %-d %H:%M")
                    );
                    // Every turn of the session keeps to the playbook's limits.
                    s.playbook = auto.id;
                    s.limits = auto.permissions.clone();
                    s.connection = connection;
                }
                if !self.expanded.contains(&project.id) {
                    self.expanded.push(project.id);
                }
                self.state.log(
                    id,
                    match from {
                        Some(agent) => format!("Playbook: {} (started by {agent})", auto.name),
                        None => format!("Playbook: {}", auto.name),
                    },
                );
                if let Some(note) = &note {
                    if let Some(s) = self.state.session_mut(id) {
                        s.messages.push(Message::new(Role::System, note.clone()));
                        if let Some(m) = s.messages.last_mut() {
                            m.meta = "note".into();
                        }
                    }
                }
                record(
                    self,
                    store::PlaybookRun {
                        at: now,
                        session: id,
                        status: "running".into(),
                        note: note.clone().unwrap_or_default(),
                    },
                );
                if from.is_none() {
                    self.select(id);
                }
                // The worktree setup command runs first, as for any new session.
                let setup = cx.global::<AppSettings>().worktree_setup.trim().to_string();
                let isolated = self.state.session(id).is_some_and(|s| s.isolated);
                let mut started = false;
                if isolated && !setup.is_empty() {
                    self.pending_start.insert(id, auto.prompt.clone());
                    self.run_terminal_in(id, setup, cx);
                    started = self.handles.contains_key(&Target::Terminal(id));
                    if !started {
                        self.pending_start.remove(&id);
                    }
                }
                if !started {
                    let opts = cx.global::<AppSettings>().agent(&auto.provider);
                    self.start_agent(id, auto.prompt.clone(), &opts);
                }
                self.tag_playbook(id, &auto.name, "");
                self.measure(id);
                if let Some(agent) = from {
                    self.info = Some((
                        format!(
                            "{agent} started the playbook “{}” in {}.",
                            auto.name, project.name
                        ),
                        std::time::Instant::now(),
                    ));
                }
            }
            Err(error) => {
                self.notice = Some(format!("Playbook “{}” could not start: {error}", auto.name));
                record(
                    self,
                    store::PlaybookRun {
                        at: now,
                        status: "failed".into(),
                        note: error,
                        ..Default::default()
                    },
                );
                if auto.scheduled() {
                    notify(&auto.name, "The scheduled playbook could not start");
                }
            }
        }
        self.save();
    }
}

/// Starting points offered on the Playbooks page.
pub(crate) const PLAYBOOK_TEMPLATES: [(&str, &str); 6] = [
    ("Find bugs", "Review the code in this project for bugs: logic errors, unhandled errors, race conditions and edge cases. List each finding with the file, line and a suggested fix, most serious first. Don't change any files."),
    ("Scan for vulnerabilities", "Audit this project for security issues: injection, unsafe deserialization, secrets in code, weak authentication, and dependencies with known vulnerabilities. Rank findings by severity with file, line and a fix. Don't change any files."),
    ("Update the docs", "Bring the README and docs in line with the code as it is now: fix outdated commands, options and examples, and document anything new. Keep the existing tone."),
    ("Add missing tests", "Find important code paths without tests and add focused tests for them, following the project's existing test style. Run the tests and make sure they pass."),
    ("Review my changes", "Review the uncommitted and unpushed changes in this repository as a careful reviewer would: correctness, naming, missing tests and anything risky. Don't change any files."),
    ("Upgrade dependencies", "Check this project's dependencies for newer versions. Upgrade the safe ones (patch and minor), run the build and tests, and list major upgrades that need a decision."),
];

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Limits an agent can't be held to, as "change files", "run commands" or "use the
/// network": a playbook with these limits warns before it runs on that agent.
pub(crate) fn unheld_limits(provider: &str, permissions: &[String]) -> Vec<&'static str> {
    if permissions.is_empty() {
        return Vec::new();
    }
    let may = |what: &str| permissions.iter().any(|p| p == what);
    let mut out = Vec::new();
    // Read-only runs in each agent's plan or read-only mode; Grok's CLI has none.
    if !may("edit") && provider == "grok" {
        out.push("change files");
    }
    // Shell: Claude Code and OpenCode turn it off, Copilot denies it, and Gemini and
    // Qwen ask for every command, which a run nobody watches refuses.
    if !may("shell") && !matches!(provider, "claude" | "opencode" | "copilot" | "gemini" | "qwen")
    {
        out.push("run commands");
    }
    // Network: Codex's sandbox has none. Elsewhere only the web tools can be turned off,
    // so commands the agent may run can still reach the network.
    if !may("network") {
        let held = match provider {
            "codex" => true,
            "claude" | "opencode" => !may("shell"),
            _ => false,
        };
        if !held {
            out.push("use the network");
        }
    }
    out
}

/// The warning for limits an agent can't be held to, if any.
pub(crate) fn limits_warning(provider: &str, permissions: &[String]) -> Option<String> {
    let unheld = unheld_limits(provider, permissions);
    (!unheld.is_empty()).then(|| {
        format!(
            "{} can't be held to every limit: it could still {}.",
            agent::provider(provider).name,
            unheld.join(" or ")
        )
    })
}

/// Hold a session started from a playbook to what the playbook declared, on every turn:
/// without `edit` it runs in plan mode, and each agent gets what it has to turn off
/// commands and the network. Returns the options for the turn.
pub(crate) fn held_options(
    session: &mut store::Session,
    opts: &settings::AgentOptions,
) -> settings::AgentOptions {
    let mut opts = opts.clone();
    let limits = session.limits.clone();
    if limits.is_empty() {
        return opts;
    }
    let may = |what: &str| limits.iter().any(|p| p == what);
    let mode = session.opts.get("mode").cloned().unwrap_or_default();
    if !may("edit") {
        session.opts.insert("mode".into(), "plan".into());
        // An OpenCode agent picked by name would replace its plan agent.
        session.opts.remove("agent");
        opts.sub_agent.clear();
    } else if mode == "full" && !(may("shell") && may("network")) {
        session.opts.insert("mode".into(), "edit".into());
    }
    playbook_limits(&session.provider, &limits, &mut opts);
    opts
}

/// Turn off what `permissions` leaves out, in the ways each agent's CLI offers.
pub(crate) fn playbook_limits(
    provider: &str,
    permissions: &[String],
    opts: &mut settings::AgentOptions,
) {
    if permissions.is_empty() {
        return;
    }
    let may = |what: &str| permissions.iter().any(|p| p == what);
    let extra = |opts: &mut settings::AgentOptions, more: &str| {
        opts.extra_args = format!("{} {more}", opts.extra_args).trim().to_string();
    };
    match provider {
        "claude" => {
            let mut off: Vec<&str> = Vec::new();
            if !may("shell") {
                off.push("Bash");
            }
            if !may("network") {
                off.extend(["WebFetch", "WebSearch"]);
                if may("shell") {
                    off.extend(["Bash(curl:*)", "Bash(wget:*)"]);
                }
            }
            let mut list: Vec<String> = opts
                .disallowed_tools
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            for tool in off {
                if !list.iter().any(|t| t == tool) {
                    list.push(tool.to_string());
                }
            }
            opts.disallowed_tools = list.join(",");
        }
        "codex" if !may("network") => {
            extra(opts, "-c sandbox_workspace_write.network_access=false")
        }
        "copilot" if !may("shell") => extra(opts, "--deny-tool shell"),
        "opencode" => {
            let mut permission = serde_json::Map::new();
            if !may("edit") {
                permission.insert("edit".into(), "deny".into());
            }
            if !may("shell") {
                permission.insert("bash".into(), "deny".into());
            }
            if !may("network") {
                permission.insert("webfetch".into(), "deny".into());
            }
            if !permission.is_empty() {
                let config = serde_json::json!({ "permission": permission });
                opts.env = format!("{}\nOPENCODE_CONFIG_CONTENT={config}", opts.env)
                    .trim()
                    .to_string();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{held_options, playbook_limits, unheld_limits};
    use crate::{settings, store};

    fn may(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn each_agent_says_which_limits_it_cant_keep() {
        let read = may(&["read"]);
        assert!(unheld_limits("claude", &read).is_empty());
        assert!(unheld_limits("codex", &read).contains(&"run commands"));
        assert!(!unheld_limits("codex", &read).contains(&"use the network"));
        assert!(unheld_limits("grok", &read).contains(&"change files"));
        assert_eq!(
            unheld_limits("claude", &may(&["read", "edit", "shell"])),
            ["use the network"]
        );
        assert!(unheld_limits("grok", &[]).is_empty(), "no list, no limits");
    }

    #[test]
    fn limits_turn_off_what_was_left_out() {
        let mut opts = settings::AgentOptions {
            disallowed_tools: "Bash".into(),
            ..Default::default()
        };
        playbook_limits("claude", &may(&["read", "edit"]), &mut opts);
        assert_eq!(opts.disallowed_tools, "Bash,WebFetch,WebSearch");
        let mut opts = settings::AgentOptions::default();
        playbook_limits("codex", &may(&["read", "shell"]), &mut opts);
        assert_eq!(
            opts.extra_args,
            "-c sandbox_workspace_write.network_access=false"
        );
        let mut opts = settings::AgentOptions::default();
        playbook_limits("opencode", &may(&["read"]), &mut opts);
        assert!(opts.env.starts_with("OPENCODE_CONFIG_CONTENT="));
        assert!(opts.env.contains("\"bash\":\"deny\""));
    }

    #[test]
    fn a_read_only_playbook_stays_in_plan_mode_on_every_turn() {
        let mut session = store::Session {
            provider: "claude".into(),
            limits: may(&["read"]),
            ..Default::default()
        };
        session.opts.insert("mode".into(), "full".into());
        let opts = held_options(&mut session, &settings::AgentOptions::default());
        assert_eq!(session.opts["mode"], "plan");
        assert!(opts.disallowed_tools.contains("Bash"));
        session.limits = may(&["read", "edit", "shell"]);
        session.opts.insert("mode".into(), "full".into());
        held_options(&mut session, &settings::AgentOptions::default());
        assert_eq!(session.opts["mode"], "edit", "full access would reach the network");
    }
}
