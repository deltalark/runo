//! Workspace-level features: sidebar status groups, saved layout, quick commands,
//! project notes, the activity feed and playbooks.

use super::*;

impl RunoApp {
    /// Remember open tabs, panels, the mode and the window so the next launch looks the same.
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
            hide_panel: !self.show_panel && !self.panel_auto_hidden,
            mode: match self.mode() {
                Mode::Settings => String::new(),
                mode => mode.label().to_string(),
            },
            split: self.split,
            right_tab: match self.right_tab {
                RightTab::Changes => "changes",
                RightTab::Browser => "browser",
                RightTab::Files => "files",
            }
            .to_string(),
            expanded: Some(self.expanded.clone()),
            project: self.project,
            sidebar_width: self.sidebar_width,
            home_all: self.home_all,
            window: self.window_place,
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
        self.split = layout
            .split
            .filter(exists)
            .filter(|id| Some(*id) != self.selected);
        self.show_sidebar = !layout.hide_sidebar;
        self.show_panel = !layout.hide_panel;
        self.right_tab = match layout.right_tab.as_str() {
            "browser" => RightTab::Browser,
            "files" => RightTab::Files,
            _ => RightTab::Changes,
        };
        if let Some(expanded) = layout.expanded {
            self.expanded = expanded
                .into_iter()
                .filter(|id| self.state.projects.iter().any(|p| p.id == *id))
                .collect();
        }
        self.project = layout
            .project
            .filter(|id| self.state.projects.iter().any(|p| p.id == *id));
        if let Some(pid) = self.project {
            self.new_project = Some(pid);
        }
        if layout.sidebar_width > 0. {
            self.sidebar_width = layout.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        }
        self.home_all = layout.home_all;
        self.window_place = layout.window;
        self.view = match layout.mode.as_str() {
            "Home" => View::Home,
            "Design" => View::Design,
            "Architecture" => View::Architecture,
            "Playbooks" => View::Automations,
            "Inbox" => View::Inbox,
            "Reviews" => {
                self.load_github();
                View::GitHub
            }
            _ if self.selected.is_none() && !self.state.sessions.is_empty() => View::AllSessions,
            _ => View::Session,
        };
    }

    /// A project's sessions as the sidebar lists them: live ones matching the filter,
    /// pinned first.
    pub(crate) fn project_sessions(&self, project: &Project, filter: &str) -> Vec<&Session> {
        let mut sessions: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == project.id && !s.archived)
            .filter(|s| {
                filter.is_empty()
                    || s.title.to_lowercase().contains(filter)
                    || s.branch.to_lowercase().contains(filter)
                    || project.name.to_lowercase().contains(filter)
            })
            .collect();
        sessions.sort_by_key(|s| !s.pinned);
        sessions
    }

    /// The sessions the sidebar shows, in order, under their group: by status when the
    /// sidebar is grouped that way, else under each open project.
    pub(crate) fn sidebar_rows(&self, filter: &str, cx: &App) -> Vec<(String, Vec<&Session>)> {
        if cx.global::<AppSettings>().group_by_status {
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
            return shell::Bucket::ALL
                .iter()
                .map(|b| {
                    let mut members: Vec<&Session> = visible
                        .iter()
                        .copied()
                        .filter(|s| self.bucket(s) == Some(*b))
                        .collect();
                    members.sort_by_key(|s| !s.pinned);
                    (b.label().to_string(), members)
                })
                .filter(|(_, m)| !m.is_empty())
                .collect();
        }
        self.state
            .projects
            .iter()
            .filter(|p| self.expanded.contains(&p.id) || !filter.is_empty())
            .map(|p| (p.name.clone(), self.project_sessions(p, filter)))
            .collect()
    }

    /// Sessions grouped by what needs attention, instead of by project.
    pub(crate) fn status_groups(&self, filter: &str, cx: &mut Context<Self>) -> Div {
        let mut col = div().v_flex().gap(px(2.)).pb_2();
        for (name, members) in self.sidebar_rows(filter, cx) {
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
                    .child(name)
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

    /// Start every scheduled playbook whose time has come. Called on a timer.
    pub(crate) fn run_due_automations(&mut self, cx: &mut Context<Self>) {
        if self.demo {
            return;
        }
        let now = unix_now();
        let due: Vec<u64> = self
            .state
            .automations
            .iter()
            .filter(|a| a.due(now))
            .map(|a| a.id)
            .collect();
        // On a timer nobody is there to fill in inputs; their defaults are used.
        for id in due {
            self.start_playbook(id, None, None, cx);
        }
    }

    /// The project in the top bar, which a playbook started by hand runs in: the one the
    /// person switched to, the open session's, the one picked for a new session, or the first.
    pub(crate) fn current_project(&self) -> Option<u64> {
        self.project
            .filter(|id| self.state.projects.iter().any(|p| p.id == *id))
            .or(self.current().map(|s| s.project_id))
            .or(self.new_project)
            .filter(|id| self.state.projects.iter().any(|p| p.id == *id))
            .or_else(|| self.state.projects.first().map(|p| p.id))
    }

    /// Start the playbooks agents asked for through Runo's MCP server.
    pub(crate) fn take_playbook_requests(&mut self, cx: &mut Context<Self>) {
        if self.demo {
            return;
        }
        for request in store::take_requests(&store::inbox_dir()) {
            self.start_playbook(
                request.playbook,
                Some(request.project),
                Some(&request.from),
                cx,
            );
            cx.notify();
        }
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
        let asks = self
            .state
            .automations
            .iter()
            .find(|a| a.id == playbook)
            .is_some_and(|a| !a.run_inputs().is_empty());
        let filled = self
            .pb_values
            .as_ref()
            .is_some_and(|(id, _)| *id == playbook);
        if asks && !filled {
            self.ask_playbook_inputs(playbook, in_project, false);
            return;
        }
        self.start_playbook(playbook, in_project, None, cx);
    }

    /// Start a playbook; `from` names the agent that asked, which leaves the person's view
    /// where it is.
    fn start_playbook(
        &mut self,
        playbook: u64,
        in_project: Option<u64>,
        from: Option<&str>,
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
        if let Some(a) = self.state.automations.iter_mut().find(|a| a.id == playbook) {
            a.last_run = unix_now();
            a.ran = a.last_run;
        }
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
                if let Some(s) = self.state.session_mut(id) {
                    s.title = format!(
                        "{} · {}",
                        auto.name,
                        chrono::Local::now().format("%b %-d %H:%M")
                    );
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
                let mut opts = cx.global::<AppSettings>().agent(&auto.provider);
                playbook_limits(&auto, &mut opts);
                if !auto.may("edit") {
                    if let Some(s) = self.state.session_mut(id) {
                        s.opts.insert("mode".into(), "plan".into());
                    }
                }
                self.start_agent(id, auto.prompt.clone(), &opts);
                self.tag_playbook(id, &auto.name, "");
                self.measure(id);
                match from {
                    Some(agent) => {
                        self.info = Some((
                            format!(
                                "{agent} started the playbook “{}” in {}.",
                                auto.name, project.name
                            ),
                            std::time::Instant::now(),
                        ));
                    }
                    None => self.select(id),
                }
            }
            Err(error) => {
                self.notice = Some(format!("Playbook “{}” could not start: {error}", auto.name));
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

/// Hold a playbook to what it declared: without `edit` it runs read-only (set on the
/// session), and on Claude Code, without `shell` or `network` those tools are off.
pub(crate) fn playbook_limits(auto: &store::Automation, opts: &mut settings::AgentOptions) {
    if auto.provider != "claude" {
        return;
    }
    let mut off: Vec<&str> = Vec::new();
    if !auto.may("shell") {
        off.push("Bash");
    }
    if !auto.may("network") {
        off.extend(["WebFetch", "WebSearch"]);
    }
    if off.is_empty() {
        return;
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
