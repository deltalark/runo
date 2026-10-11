//! Workspace-level features: sidebar status groups, saved layout, quick commands,
//! project notes, the activity feed and scheduled automations.

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
                    .font_weight(FontWeight::SEMIBOLD)
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
                    .hover(|el| el.bg(rgb(c_active())).text_color(rgb(c_text())))
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
            self.save();
        }
    }

    pub(crate) fn notes_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.notes_project();
        let mut chips = div().flex().flex_wrap().gap_2();
        for (i, project) in self.state.projects.iter().enumerate() {
            let pid = project.id;
            let has = self.state.notes.contains_key(&pid);
            chips = chips.child(
                self.chip(
                    ("notes-project", i),
                    if has {
                        format!("{} ·", project.name)
                    } else {
                        project.name.clone()
                    },
                    current == Some(pid),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.notes_project = Some(pid);
                    this.open_notes(window, cx);
                    cx.notify();
                })),
            );
        }
        self.page(
            "Notes",
            div()
                .v_flex()
                .gap_3()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(c_secondary()))
                        .child("Notes for each project: decisions, commands, things to tell the agent. Saved as you type."),
                )
                .child(chips)
                .child(if current.is_some() {
                    div()
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .bg(rgb(c_raised()))
                        .text_size(px(13.5))
                        .child(
                            Textarea::new(&self.notes_input)
                                .appearance(false)
                                .bordered(false)
                                .h(px(420.)),
                        )
                        .into_any_element()
                } else {
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(c_muted()))
                        .child("Add a project to keep notes for it.")
                        .into_any_element()
                }),
        )
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
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_muted()))
                        .child(day.clone()),
                );
            }
            let sid = event.session;
            let exists = self.state.session(sid).is_some();
            let (mark, color) = if event.text.starts_with("Failed") {
                (IconName::CircleX, c_red())
            } else if event.text.starts_with("Finished") {
                (IconName::CircleCheck, c_green())
            } else if event.text.starts_with("Started") {
                (IconName::Play, c_blue())
            } else {
                (IconName::Dot, c_muted())
            };
            let provider = self
                .state
                .session(sid)
                .map(|s| agent::provider(&s.provider).name)
                .unwrap_or("");
            rows = rows.child(
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
                            .hover(|el| el.bg(rgb(c_subtle())))
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
            );
        }
        if self.state.activity.is_empty() {
            rows = rows.child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(c_muted()))
                    .child("Agent runs show up here as they start and finish."),
            );
        }
        self.page("Activity", rows)
    }

    // ---------- Automations ----------

    /// Start every automation whose time has come. Called on a timer.
    pub(crate) fn run_due_automations(&mut self, cx: &mut Context<Self>) {
        if self.demo {
            return;
        }
        let now = unix_now();
        let due: Vec<store::Automation> = self
            .state
            .automations
            .iter()
            .filter(|a| a.due(now))
            .cloned()
            .collect();
        for auto in due {
            self.run_automation(auto.id, cx);
        }
    }

    pub(crate) fn run_automation(&mut self, auto_id: u64, cx: &mut Context<Self>) {
        let Some(auto) = self
            .state
            .automations
            .iter()
            .find(|a| a.id == auto_id)
            .cloned()
        else {
            return;
        };
        if let Some(a) = self.state.automations.iter_mut().find(|a| a.id == auto_id) {
            a.last_run = unix_now();
        }
        let Some(project) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == auto.project_id)
            .cloned()
        else {
            self.notice = Some(format!("Automation “{}”: its project is gone.", auto.name));
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
                self.state.log(id, format!("Automation: {}", auto.name));
                let opts = cx.global::<AppSettings>().agent(&auto.provider);
                self.start_agent(id, auto.prompt.clone(), &opts);
                self.measure(id);
            }
            Err(error) => {
                self.notice = Some(format!(
                    "Automation “{}” could not start: {error}",
                    auto.name
                ));
            }
        }
        self.save();
    }

    pub(crate) fn add_automation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let name = self.auto_name.read(cx).value().trim().to_string();
        let prompt = self.auto_prompt.read(cx).value().trim().to_string();
        let every: u64 = self.auto_every.read(cx).value().trim().parse().unwrap_or(0);
        let Some(project_id) = self
            .auto_project
            .or_else(|| self.state.projects.first().map(|p| p.id))
        else {
            self.notice = Some("Add a project first.".into());
            return;
        };
        if prompt.is_empty() || every == 0 {
            self.notice = Some("Write a prompt and how many minutes between runs.".into());
            return;
        }
        let id = self.state.next_id();
        self.state.automations.push(store::Automation {
            id,
            name: if name.is_empty() {
                prompt
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect()
            } else {
                name
            },
            project_id,
            provider: agent::PROVIDERS[self.auto_provider].id.into(),
            prompt,
            every_minutes: every,
            // The first run waits one interval.
            last_run: unix_now(),
            enabled: true,
        });
        for input in [&self.auto_name, &self.auto_prompt, &self.auto_every] {
            input.update(cx, |i, cx| i.set_value("", window, cx));
        }
        self.notice = None;
        self.save();
    }

    pub(crate) fn automations_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().v_flex().gap_2();
        for (i, auto) in self.state.automations.iter().enumerate() {
            let id = auto.id;
            let provider = agent::provider(&auto.provider);
            let project = self
                .state
                .projects
                .iter()
                .find(|p| p.id == auto.project_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "missing project".into());
            let next = if auto.enabled {
                let at = auto.last_run + auto.every_minutes * 60;
                let mins = at.saturating_sub(unix_now()) / 60;
                format!("next run in {mins} min")
            } else {
                "paused".into()
            };
            list = list.child(
                div()
                    .id(("auto", i))
                    .p_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .when(!auto.enabled, |el| el.opacity(0.6))
                    .child(self.provider_badge(provider, 24.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(auto.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .child(format!(
                                        "Every {} min · {project} · {next}",
                                        auto.every_minutes
                                    )),
                            ),
                    )
                    .child(
                        self.button(("auto-run", i), "Run now", false)
                            .h(px(26.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_automation(id, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            ("auto-toggle", i),
                            if auto.enabled { "Pause" } else { "Resume" },
                            false,
                        )
                        .h(px(26.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(a) = this.state.automations.iter_mut().find(|a| a.id == id)
                            {
                                a.enabled = !a.enabled;
                                a.last_run = unix_now();
                            }
                            this.save();
                            cx.notify();
                        })),
                    )
                    .child(
                        self.ghost_button_sized(("auto-del", i), IconName::Trash, "Delete", 26.)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state.automations.retain(|a| a.id != id);
                                this.save();
                                cx.notify();
                            })),
                    ),
            );
        }
        let mut projects = div().flex().flex_wrap().gap_2();
        let picked_project = self
            .auto_project
            .or_else(|| self.state.projects.first().map(|p| p.id));
        for (i, p) in self.state.projects.iter().enumerate() {
            let pid = p.id;
            projects = projects.child(
                self.chip(
                    ("auto-project", i),
                    p.name.clone(),
                    picked_project == Some(pid),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.auto_project = Some(pid);
                    cx.notify();
                })),
            );
        }
        let mut agents = div().flex().flex_wrap().gap_2();
        for (i, p) in agent::PROVIDERS.iter().enumerate() {
            agents = agents.child(
                self.chip(
                    ("auto-agent", i),
                    p.name.to_string(),
                    self.auto_provider == i,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.auto_provider = i;
                    cx.notify();
                })),
            );
        }
        let label = |text: &'static str| {
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_muted()))
                .child(text)
        };
        self.page(
            "Automations",
            div()
                .v_flex()
                .gap_4()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(c_secondary()))
                        .child("Prompts that start a new session on a schedule, such as a nightly dependency check. They run while Runo is open."),
                )
                .child(list)
                .child(
                    div()
                        .p_4()
                        .v_flex()
                        .gap_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(c_border()))
                        .bg(rgb(c_raised()))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("New automation"),
                        )
                        .child(label("NAME"))
                        .child(Input::new(&self.auto_name).bordered(true).h(px(30.)))
                        .child(label("PROMPT"))
                        .child(Input::new(&self.auto_prompt).bordered(true).h(px(30.)))
                        .child(label("EVERY (MINUTES)"))
                        .child(div().w(px(160.)).child(Input::new(&self.auto_every).bordered(true).h(px(30.))))
                        .child(label("PROJECT"))
                        .child(projects)
                        .child(label("AGENT"))
                        .child(agents)
                        .child(
                            div().child(
                                self.button("auto-add", "Add automation", true)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_automation(window, cx);
                                        cx.notify();
                                    })),
                            ),
                        ),
                ),
        )
    }
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
