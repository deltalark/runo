//! The Playbooks page: saved tasks as cards, a template gallery and the playbook editor.

use super::*;
use workspace::{unix_now, PLAYBOOK_TEMPLATES};

/// The playbook editor while it is open.
pub(crate) struct PlaybookEditor {
    /// The playbook being changed, or None for a new one.
    pub editing: Option<u64>,
    /// Minutes between runs; 0 runs it by hand.
    pub every: u64,
    /// The schedule is typed in minutes instead of picked from the presets.
    pub custom: bool,
    pub error: Option<String>,
    /// What it may do, in `pbfile::PERMISSIONS` order.
    pub permissions: Vec<String>,
    /// Labels and defaults as saved, to fill new input rows from.
    pub declared: Vec<crate::pbfile::Input>,
    /// A label box and a default box per `{{key}}` seen in the instructions.
    pub inputs: Vec<(String, Entity<InputState>, Entity<InputState>)>,
}

/// The shortest custom schedule, in minutes.
const MIN_EVERY: u64 = 10;

/// Schedule presets in the editor: label and minutes.
const SCHEDULES: [(&str, u64); 4] = [
    ("By hand", 0),
    ("Hourly", 60),
    ("Every 6 hours", 360),
    ("Daily", 1440),
];

/// An icon and color that fit what the playbook does, from its name and instructions.
pub(crate) fn look(name: &str, prompt: &str) -> (IconName, u32) {
    let text = format!("{name} {prompt}").to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["vulnerab", "security", "secret", "audit"]) {
        (IconName::ShieldCheck, 0xf59e0b)
    } else if has(&["bug", "crash", "error"]) {
        (IconName::Bug, 0xef4444)
    } else if has(&["test"]) {
        (IconName::FlaskConical, 0x22c55e)
    } else if has(&["review", "pull request"]) {
        (IconName::GitPullRequest, 0x8b5cf6)
    } else if has(&["depend", "upgrade", "outdated", "package"]) {
        (IconName::PackageCheck, 0x14b8a6)
    } else if has(&["doc", "readme", "changelog"]) {
        (IconName::FileText, 0x3b82f6)
    } else if has(&["refactor", "clean", "lint", "format"]) {
        (IconName::Sparkles, 0xec4899)
    } else {
        (IconName::Zap, c_blue())
    }
}

/// "just now", "12 min ago", "3 h ago", "2 days ago".
pub(crate) fn ago(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => {
            let days = secs / 86_400;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
    }
}

/// "45 min", "6 h", "1 day", for schedules.
pub(crate) fn every_label(minutes: u64) -> String {
    match minutes {
        0 => "By hand".into(),
        m if m % 1440 == 0 => {
            let d = m / 1440;
            if d == 1 {
                "Daily".into()
            } else {
                format!("Every {d} days")
            }
        }
        60 => "Hourly".into(),
        m if m % 60 == 0 => format!("Every {} h", m / 60),
        m => format!("Every {m} min"),
    }
}

/// "in 12 min", "in 3 h", "now".
fn until(at: u64, now: u64) -> String {
    let secs = at.saturating_sub(now);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("in {} min", secs / 60),
        3600..=86_399 => format!("in {} h", secs / 3600),
        _ => format!("in {} days", secs / 86_400),
    }
}

impl RunoApp {
    /// The playbook picked from the `/` menu, shown in the composer above the text box.
    pub(crate) fn playbook_chip(
        &self,
        a: &store::Automation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (glyph, color) = look(&a.name, &a.prompt);
        let provider = agent::provider(&a.provider);
        let summary = a.summary();
        div()
            .px_1()
            .pt_1()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("picked-playbook")
                    .h(px(30.))
                    .pl(px(4.))
                    .pr(px(4.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(settings::mix(color, c_border(), 0.45)))
                    .bg(rgb(settings::mix(
                        color,
                        c_canvas(),
                        if is_dark() { 0.14 } else { 0.07 },
                    )))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(summary.clone())
                            .build(window, cx)
                    })
                    .child(Self::playbook_tile(glyph, color, 22.))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(a.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("playbook · {}", provider.name)),
                    )
                    .child(
                        self.ghost_button_sized(
                            "unpick-playbook",
                            IconName::X,
                            "Remove playbook",
                            20.,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.picked_playbook = None;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("Add details below, or just send."),
            )
            .into_any_element()
    }

    /// A user message that ran a playbook: its name, anything typed with it, and the full
    /// instructions on request.
    pub(crate) fn playbook_message(
        &self,
        index: usize,
        message: &store::Message,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Only the name shows: a playbook's instructions can be long, and they
        // live on its page, which the name opens.
        let id = self
            .state
            .automations
            .iter()
            .find(|a| a.name == message.playbook)
            .map(|a| a.id);
        let (glyph, color) = look(&message.playbook, &message.text);
        let name = div()
            .id(("pb-msg-link", index))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .child(
                div()
                    .truncate()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(id.is_some(), |el| el.text_color(rgb(c_blue())))
                    .child(message.playbook.clone()),
            )
            .when_some(id, |el, id| {
                el.cursor_pointer()
                    .hover(|el| el.underline())
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Open this playbook")
                            .build(window, cx)
                    })
                    .child(icon(IconName::ArrowUpRight, 12., c_blue()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.view = View::Automations;
                        this.open_playbook_editor(Some(id), window, cx);
                        cx.notify();
                    }))
            })
            .when(id.is_none(), |el| {
                el.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child("deleted"),
                )
            });
        div()
            .max_w(px(560.))
            .v_flex()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(settings::mix(color, c_border(), 0.4)))
            .bg(tint(c_raised()))
            .overflow_hidden()
            .child(
                div()
                    .px_3()
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(rgb(settings::mix(
                        color,
                        c_canvas(),
                        if is_dark() { 0.12 } else { 0.06 },
                    )))
                    .child(Self::playbook_tile(glyph, color, 24.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child("Ran playbook"),
                    )
                    .child(name),
            )
            .when(!message.note.is_empty(), |el| {
                el.child(
                    div()
                        .px(px(14.))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(rgb(c_border()))
                        .text_size(px(13.5))
                        .child(message.note.clone()),
                )
            })
            .into_any_element()
    }

    pub(crate) fn push_playbook(
        &mut self,
        name: String,
        prompt: String,
        description: String,
        project_id: u64,
        every: u64,
    ) {
        let id = self.state.next_id();
        self.state.automations.push(store::Automation {
            id,
            name,
            project_id,
            provider: agent::PROVIDERS[self.auto_provider].id.into(),
            prompt,
            description,
            every_minutes: every,
            // A scheduled first run waits one interval.
            last_run: if every > 0 { unix_now() } else { 0 },
            enabled: true,
            ran: 0,
            ..Default::default()
        });
        self.save();
    }

    /// Open the editor, filled from a playbook when one is given.
    pub(crate) fn open_playbook_editor(
        &mut self,
        editing: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = editing.and_then(|id| self.state.automations.iter().find(|a| a.id == id));
        let (name, desc, prompt, every) = match existing {
            Some(a) => {
                self.auto_provider = agent::PROVIDERS
                    .iter()
                    .position(|p| p.id == a.provider)
                    .unwrap_or(0);
                self.auto_project = (a.project_id != 0).then_some(a.project_id);
                (
                    a.name.clone(),
                    a.description.clone(),
                    a.prompt.clone(),
                    a.every_minutes,
                )
            }
            None => {
                self.auto_project = None;
                let default = &cx.global::<AppSettings>().default_provider;
                self.auto_provider = agent::PROVIDERS
                    .iter()
                    .position(|p| p.id == *default)
                    .unwrap_or(0);
                Default::default()
            }
        };
        let custom = every != 0 && !SCHEDULES.iter().any(|(_, m)| *m == every);
        self.auto_name
            .update(cx, |i, cx| i.set_value(name, window, cx));
        self.auto_desc
            .update(cx, |i, cx| i.set_value(desc, window, cx));
        self.auto_prompt
            .update(cx, |i, cx| i.set_value(prompt, window, cx));
        self.auto_every.update(cx, |i, cx| {
            i.set_value(
                if custom {
                    every.to_string()
                } else {
                    String::new()
                },
                window,
                cx,
            )
        });
        self.auto_name.update(cx, |i, cx| i.focus(window, cx));
        let all = || {
            crate::pbfile::PERMISSIONS
                .iter()
                .map(|p| p.0.to_string())
                .collect::<Vec<_>>()
        };
        let permissions = editing
            .and_then(|id| self.state.automations.iter().find(|a| a.id == id))
            .map(|a| a.permissions.clone())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(all);
        let declared = editing
            .and_then(|id| self.state.automations.iter().find(|a| a.id == id))
            .map(|a| a.inputs.clone())
            .unwrap_or_default();
        self.pb_editor = Some(PlaybookEditor {
            editing,
            every,
            custom,
            error: None,
            permissions,
            declared,
            inputs: Vec::new(),
        });
    }

    /// Give each `{{key}}` in the editor's instructions a label and a default box.
    pub(crate) fn prepare_playbook_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.pb_editor.as_ref() else {
            return;
        };
        let prompt = self.auto_prompt.read(cx).value().to_string();
        let missing: Vec<crate::pbfile::Input> =
            crate::pbfile::inputs_of(&editor.declared, &prompt)
                .into_iter()
                .filter(|i| !editor.inputs.iter().any(|(k, _, _)| *k == i.key))
                .collect();
        if missing.is_empty() {
            return;
        }
        let rows: Vec<_> = missing
            .into_iter()
            .map(|input| {
                let title = input.title();
                let label = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(title)
                        .default_value(input.label.clone())
                });
                let default = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("No default")
                        .default_value(input.default.clone())
                });
                (input.key, label, default)
            })
            .collect();
        if let Some(editor) = self.pb_editor.as_mut() {
            editor.inputs.extend(rows);
        }
    }

    /// Save the editor: a new playbook, or changes to the one being edited.
    pub(crate) fn save_playbook_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.pb_editor.as_ref() else {
            return;
        };
        let permissions = editor.permissions.clone();
        let name = self.auto_name.read(cx).value().trim().to_string();
        let description = self.auto_desc.read(cx).value().trim().to_string();
        let prompt = self.auto_prompt.read(cx).value().trim().to_string();
        let keys = crate::pbfile::placeholders(&prompt);
        let inputs: Vec<crate::pbfile::Input> = editor
            .inputs
            .iter()
            .filter(|(key, _, _)| keys.contains(key))
            .map(|(key, label, default)| crate::pbfile::Input {
                key: key.clone(),
                label: label.read(cx).value().trim().to_string(),
                default: default.read(cx).value().trim().to_string(),
            })
            .filter(|i| !i.label.is_empty() || !i.default.is_empty())
            .collect();
        let every = if editor.custom {
            match self.auto_every.read(cx).value().trim().parse::<u64>() {
                Ok(m) if m < MIN_EVERY => {
                    self.playbook_error(&format!(
                        "Runs need at least {MIN_EVERY} minutes between them."
                    ));
                    return;
                }
                Ok(m) => m,
                Err(_) => {
                    self.playbook_error("Type the minutes between runs, such as 90.");
                    return;
                }
            }
        } else {
            editor.every
        };
        if prompt.is_empty() {
            self.playbook_error("Write what the playbook should do.");
            return;
        }
        let project_id = self.auto_project.unwrap_or(0);
        if every > 0 && project_id == 0 {
            self.playbook_error(
                "A schedule needs a project to run in. Pick one, or run it by hand.",
            );
            return;
        }
        let name = if name.is_empty() {
            prompt
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(40)
                .collect()
        } else {
            name
        };
        match editor.editing {
            Some(id) => {
                let provider = agent::PROVIDERS[self.auto_provider].id.to_string();
                if let Some(a) = self.state.automations.iter_mut().find(|a| a.id == id) {
                    if a.every_minutes != every {
                        // A new schedule counts from now.
                        a.last_run = a.last_run.max(unix_now());
                    }
                    a.name = name;
                    a.description = description;
                    a.prompt = prompt;
                    a.provider = provider;
                    a.project_id = project_id;
                    a.every_minutes = every;
                    a.permissions = permissions;
                    a.inputs = inputs;
                }
                self.save();
            }
            None => {
                self.push_playbook(name, prompt, description, project_id, every);
                if let Some(a) = self.state.automations.last_mut() {
                    a.permissions = permissions;
                    a.inputs = inputs;
                }
                self.save();
            }
        }
        self.pb_editor = None;
        cx.notify();
    }

    fn playbook_error(&mut self, why: &str) {
        if let Some(editor) = self.pb_editor.as_mut() {
            editor.error = Some(why.into());
        }
    }

    /// A small rounded label with an icon, for a card's details.
    pub(crate) fn pill(&self, glyph: Option<IconName>, text: String, color: u32) -> Div {
        div()
            .h(px(22.))
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .rounded_full()
            .border_1()
            .border_color(rgb(settings::mix(color, c_canvas(), 0.28)))
            .bg(rgb(settings::mix(
                color,
                c_canvas(),
                if is_dark() { 0.14 } else { 0.07 },
            )))
            .text_size(px(11.5))
            .text_color(rgb(if color == c_muted() {
                c_secondary()
            } else {
                color
            }))
            .when_some(glyph, |el, g| el.child(icon(g, 11., color)))
            .child(text)
    }

    /// A colored square with the playbook's icon.
    /// Pills for what a playbook declares: who made it, what it may do, what it asks for.
    fn playbook_badges(&self, auto: &store::Automation) -> Vec<Div> {
        let mut pills = Vec::new();
        let made = match (auto.author.is_empty(), auto.version.is_empty()) {
            (true, true) => None,
            (false, true) => Some(format!("by {}", auto.author)),
            (true, false) => Some(format!("v{}", auto.version.trim_start_matches('v'))),
            (false, false) => Some(format!(
                "by {} · v{}",
                auto.author,
                auto.version.trim_start_matches('v')
            )),
        };
        if let Some(made) = made {
            pills.push(self.pill(Some(IconName::PackageCheck), made, c_blue()));
        }
        if !auto.permissions.is_empty() {
            pills.push(match permission_summary(&auto.permissions) {
                None => self.pill(Some(IconName::Eye), "Read only".into(), c_green()),
                Some(text) => self.pill(Some(IconName::ShieldCheck), text, c_amber()),
            });
        }
        let inputs = auto.run_inputs().len();
        if inputs > 0 {
            pills.push(self.pill(
                Some(IconName::SquarePen),
                if inputs == 1 {
                    "1 input".into()
                } else {
                    format!("{inputs} inputs")
                },
                c_muted(),
            ));
        }
        pills
    }

    pub(crate) fn playbook_tile(glyph: IconName, color: u32, size: f32) -> Div {
        div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size * 0.28))
            .border_1()
            .border_color(rgb(settings::mix(color, c_canvas(), 0.3)))
            .bg(linear_gradient(
                135.,
                linear_color_stop(
                    rgb(settings::mix(
                        color,
                        c_canvas(),
                        if is_dark() { 0.32 } else { 0.2 },
                    )),
                    0.,
                ),
                linear_color_stop(
                    rgb(settings::mix(
                        color,
                        c_canvas(),
                        if is_dark() { 0.12 } else { 0.06 },
                    )),
                    1.,
                ),
            ))
            .child(icon(glyph, (size * 0.48).round(), color))
    }

    fn playbook_card(
        &self,
        i: usize,
        auto: &store::Automation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = unix_now();
        let id = auto.id;
        let provider = agent::provider(&auto.provider);
        let (glyph, color) = look(&auto.name, &auto.prompt);
        let project = (auto.project_id != 0).then(|| {
            self.state
                .projects
                .iter()
                .find(|p| p.id == auto.project_id)
                .cloned()
        });
        let here = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone());
        let run_in = match (&project, &here) {
            (Some(Some(p)), _) => Some(p.name.clone()),
            (Some(None), _) => None,
            (None, here) => here.clone(),
        };
        let schedule = if auto.scheduled() {
            if auto.enabled {
                self.pill(
                    Some(IconName::CalendarClock),
                    format!(
                        "{} · next {}",
                        every_label(auto.every_minutes),
                        until(auto.last_run + auto.every_minutes * 60, now)
                    ),
                    c_green(),
                )
            } else {
                self.pill(
                    Some(IconName::Pause),
                    format!("{} · paused", every_label(auto.every_minutes)),
                    c_amber(),
                )
            }
        } else {
            self.pill(Some(IconName::Hand), "Run by hand".into(), c_muted())
        };
        let where_ = match &project {
            Some(Some(p)) => self.pill(Some(projects::icon_of(p)), p.name.clone(), c_muted()),
            Some(None) => self.pill(
                Some(IconName::TriangleAlert),
                "Missing project".into(),
                c_red(),
            ),
            None => self.pill(Some(IconName::Layers), "Any project".into(), c_muted()),
        };
        let confirming = self.pb_confirm_delete == Some(id);
        let last = match (auto.runs.last(), auto.ran_at()) {
            (Some(run), _) if run.status == "failed" || run.status == "not started" => {
                format!("Last run failed {}", ago(run.at, now))
            }
            (_, 0) => "Never run".to_string(),
            (_, at) => format!("Last run {}", ago(at, now)),
        };
        let warning = crate::workspace::limits_warning(&auto.provider, &auto.permissions);
        let mut history = div().v_flex().gap(px(2.));
        for (j, run) in auto.runs.iter().rev().take(3).enumerate() {
            let (glyph, tone, word) = match run.status.as_str() {
                "done" => (IconName::CircleCheck, c_green(), "Done"),
                "running" => (IconName::LoaderCircle, c_blue(), "Running"),
                "skipped" => (IconName::SkipForward, c_muted(), "Skipped"),
                "not started" => (IconName::CircleX, c_red(), "Didn't start"),
                _ => (IconName::CircleX, c_red(), "Failed"),
            };
            let sid = run.session;
            let exists = sid != 0 && self.state.session(sid).is_some();
            history = history.child(
                div()
                    .id(("pb-run-row", i * 8 + j))
                    .h(px(24.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .text_size(px(12.))
                    .when(exists, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select(sid);
                                cx.notify();
                            }))
                    })
                    .child(icon(glyph, 12., tone))
                    .child(div().flex_none().text_color(rgb(c_text())).child(word))
                    .child(
                        div()
                            .flex_none()
                            .text_color(rgb(c_muted()))
                            .child(ago(run.at, now)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(c_secondary()))
                            .child(run.note.clone()),
                    )
                    .when(exists, |el| {
                        el.child(icon(IconName::ChevronRight, 12., c_muted()))
                    }),
            );
        }
        let card = div()
            .id(("pb-card", i))
            .group("pb-card")
            .p_4()
            .v_flex()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_raised()))
            .hover(|el| {
                el.border_color(rgb(settings::mix(color, c_border(), 0.45)))
                    .shadow_md()
            })
            .when(auto.scheduled() && !auto.enabled, |el| el.opacity(0.75))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(Self::playbook_tile(glyph, color, 40.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(auto.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(px(11.))
                                            .font_family(self.mono_font.clone())
                                            .text_color(rgb(c_muted()))
                                            .child(format!("/{}", auto.slug())),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .line_height(px(18.))
                                    .max_h(px(36.))
                                    .overflow_hidden()
                                    .text_color(rgb(c_secondary()))
                                    .child(auto.summary()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(2.))
                            .opacity(0.)
                            .group_hover("pb-card", |el| el.opacity(1.))
                            .when(!auto.source.is_empty(), |el| {
                                el.child(
                                    self.ghost_button_sized(
                                        ("pb-update", i),
                                        IconName::RefreshCw,
                                        "Check for an update",
                                        26.,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.check_playbook_update(id, window, cx);
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .child(
                                self.ghost_button_sized(
                                    ("pb-copy", i),
                                    IconName::Copy,
                                    "Copy as a playbook file",
                                    26.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.copy_playbook(id, cx);
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                self.ghost_button_sized(
                                    ("pb-export", i),
                                    IconName::Download,
                                    "Export to a file",
                                    26.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.export_playbook(id, cx);
                                    },
                                )),
                            )
                            .child(
                                self.ghost_button_sized(
                                    ("pb-edit", i),
                                    IconName::Pencil,
                                    "Edit",
                                    26.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.open_playbook_editor(Some(id), window, cx);
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                self.ghost_button_sized(
                                    ("pb-del", i),
                                    IconName::Trash,
                                    "Delete",
                                    26.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.pb_confirm_delete = Some(id);
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1p5()
                    .child(
                        div()
                            .h(px(22.))
                            .pl(px(3.))
                            .pr_2()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .text_size(px(11.5))
                            .text_color(rgb(c_secondary()))
                            .child(self.provider_badge(provider, 16.))
                            .child(provider.name),
                    )
                    .child(where_)
                    .child(schedule)
                    .children(self.playbook_badges(auto)),
            )
            .when_some(warning, |el, w| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(c_amber()))
                        .child(icon(IconName::ShieldAlert, 12., c_amber()))
                        .child(w),
                )
            })
            .when(!auto.runs.is_empty(), |el| el.child(history))
            .child(div().h(px(1.)).bg(rgb(c_border())))
            .child(if confirming {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.5))
                            .text_color(rgb(c_red()))
                            .child(format!("Delete “{}”?", auto.name)),
                    )
                    .child(
                        self.button(("pb-del-no", i), "Keep", false)
                            .h(px(28.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pb_confirm_delete = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(("pb-del-yes", i), "Delete", false)
                            .h(px(28.))
                            .border_color(rgb(c_red()))
                            .text_color(rgb(c_red()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state.automations.retain(|a| a.id != id);
                                this.pb_confirm_delete = None;
                                this.save();
                                cx.notify();
                            })),
                    )
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::Clock, 12., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child(last),
                    )
                    .when(auto.scheduled(), |el| {
                        el.child(
                            self.button(
                                ("pb-toggle", i),
                                if auto.enabled { "Pause" } else { "Resume" },
                                false,
                            )
                            .h(px(28.))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(a) =
                                        this.state.automations.iter_mut().find(|a| a.id == id)
                                    {
                                        a.enabled = !a.enabled;
                                        a.last_run = unix_now();
                                    }
                                    this.save();
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .child(
                        self.button(
                            ("pb-run", i),
                            match &run_in {
                                Some(p) => format!("Run in {p}"),
                                None => "Run".into(),
                            },
                            true,
                        )
                        .h(px(28.))
                        .child(icon(IconName::Play, 11., c_canvas()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let here = this.current_project();
                            this.run_playbook(id, here, cx);
                            cx.notify();
                        })),
                    )
            });
        stagger(card, ("pb-card-in", i), i)
    }

    pub(crate) fn automations_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let now = unix_now();
        let query = self.pb_query.read(cx).value().trim().to_lowercase();
        let all = &self.state.automations;
        let shown: Vec<&store::Automation> = all
            .iter()
            .filter(|a| {
                query.is_empty()
                    || a.name.to_lowercase().contains(&query)
                    || a.prompt.to_lowercase().contains(&query)
                    || a.description.to_lowercase().contains(&query)
            })
            .collect();
        let scheduled = all.iter().filter(|a| a.scheduled()).count();
        let paused = all.iter().filter(|a| a.scheduled() && !a.enabled).count();
        let next = all
            .iter()
            .filter(|a| a.scheduled() && a.enabled)
            .min_by_key(|a| a.last_run + a.every_minutes * 60);
        let latest = all
            .iter()
            .filter(|a| a.ran_at() > 0)
            .max_by_key(|a| a.ran_at());

        let stat = |glyph: IconName, color: u32, label: &str, value: String, sub: String| {
            div()
                .flex_1()
                .min_w_0()
                .p_3()
                .flex()
                .items_center()
                .gap_3()
                .rounded_xl()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(tint(c_raised()))
                .child(Self::playbook_tile(glyph, color, 34.))
                .child(
                    div()
                        .min_w_0()
                        .v_flex()
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(c_muted()))
                                .child(label.to_uppercase()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(value),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(sub),
                        ),
                )
        };
        let stats = div()
            .flex()
            .gap_3()
            .child(stat(
                IconName::BookOpen,
                c_blue(),
                "Playbooks",
                all.len().to_string(),
                format!(
                    "{} for any project",
                    all.iter().filter(|a| a.project_id == 0).count()
                ),
            ))
            .child(stat(
                IconName::CalendarClock,
                c_green(),
                "Scheduled",
                scheduled.to_string(),
                match next {
                    Some(a) => format!(
                        "{} runs {}",
                        a.name,
                        until(a.last_run + a.every_minutes * 60, now)
                    ),
                    None if paused > 0 => format!("{paused} paused"),
                    None => "None on a timer".into(),
                },
            ))
            .child(stat(
                IconName::Clock,
                0x8b5cf6,
                "Last run",
                latest
                    .map(|a| ago(a.ran_at(), now))
                    .unwrap_or_else(|| "—".into()),
                latest
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "Nothing has run yet".into()),
            ));

        let grid = columns(
            shown
                .iter()
                .enumerate()
                .map(|(i, auto)| self.playbook_card(i, auto, cx).into_any_element())
                .collect(),
            2,
        );
        let list = if all.is_empty() {
            div()
                .p_6()
                .rounded_xl()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(tint(c_raised()))
                .child(empty_hint(
                    "auto-empty",
                    IconName::BookOpen,
                    "No playbooks yet",
                    "A playbook is a task you run often: a bug hunt, a dependency check, a review. Add one from the templates below or write your own.",
                ))
                .into_any_element()
        } else if shown.is_empty() {
            empty_hint(
                "auto-none",
                IconName::Search,
                "No playbooks match",
                "Try another word, or clear the search.",
            )
        } else {
            grid.into_any_element()
        };

        // Templates: one click saves them as playbooks that run in any project.
        let mut templates = Vec::new();
        for (i, (name, prompt)) in PLAYBOOK_TEMPLATES.iter().enumerate() {
            let added = all.iter().any(|a| a.name == *name);
            let (glyph, color) = look(name, prompt);
            let summary = store::Automation {
                id: 0,
                name: name.to_string(),
                project_id: 0,
                provider: String::new(),
                prompt: prompt.to_string(),
                description: String::new(),
                every_minutes: 0,
                last_run: 0,
                enabled: true,
                ran: 0,
                ..Default::default()
            }
            .summary();
            templates.push(
                div()
                    .id(("auto-tpl", i))
                    .p_3()
                    .v_flex()
                    .gap_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .when(!added, |el| {
                        el.cursor_pointer()
                            .hover(|el| {
                                el.bg(tint(c_subtle())).border_color(rgb(settings::mix(
                                    color,
                                    c_border(),
                                    0.45,
                                )))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.push_playbook(
                                    name.to_string(),
                                    prompt.to_string(),
                                    String::new(),
                                    0,
                                    0,
                                );
                                cx.notify();
                            }))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Self::playbook_tile(glyph, color, 28.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(*name),
                            )
                            .child(if added {
                                self.pill(Some(IconName::Check), "Added".into(), c_green())
                            } else {
                                self.pill(Some(IconName::Plus), "Add".into(), c_blue())
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .max_h(px(34.))
                            .overflow_hidden()
                            .text_color(rgb(c_muted()))
                            .child(summary),
                    )
                    .into_any_element(),
            );
        }

        let header = div()
            .flex()
            .items_center()
            .gap_4()
            .child(Self::playbook_tile(IconName::BookOpen, c_blue(), 44.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Playbooks"),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(c_secondary()))
                            .child(format!(
                                "Saved tasks you run in any project: from here, by typing / in a message, from Quick open ({}K), or by right-clicking a project.",
                                MOD_KEY
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        self.button("pb-install", "Install", false)
                            .h(px(34.))
                            .px_3()
                            .child(icon(IconName::Download, 13., c_text()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_playbook_install(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                self.button("pb-new", "New playbook", true)
                    .h(px(34.))
                    .px_3()
                    .child(icon(IconName::Plus, 13., c_canvas()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_playbook_editor(None, window, cx);
                        cx.notify();
                    })),
                    ),
            );

        let section_head = |title: &str, aside: Option<AnyElement>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .children(aside)
        };
        let search = div()
            .w(px(240.))
            .child(
                Input::new(&self.pb_query)
                    .bordered(true)
                    .prefix(icon(IconName::Search, 13., c_muted()))
                    .h(px(30.)),
            )
            .into_any_element();

        let body = div().flex_1().min_w_0().overflow_y_scrollbar().child(
            div()
                .max_w(px(1040.))
                .mx_auto()
                .p_8()
                .v_flex()
                .gap_6()
                .child(header)
                .child(stats)
                .child(
                    div()
                        .v_flex()
                        .gap_3()
                        .child(section_head(
                            &format!("Your playbooks · {}", all.len()),
                            (!all.is_empty()).then_some(search),
                        ))
                        .child(list),
                )
                .child(
                    div()
                        .v_flex()
                        .gap_3()
                        .child(section_head("Start from a template", None))
                        .child(columns(templates, 3)),
                ),
        );
        div()
            .flex_1()
            .min_w_0()
            .relative()
            .flex()
            .child(body)
            .children(self.playbook_editor(cx))
            .children(self.playbook_install_modal(cx))
            .into_any_element()
    }

    /// The editor, as a dialog over the Playbooks page.
    fn playbook_editor(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.pb_editor.as_ref()?;
        let name = self.auto_name.read(cx).value().to_string();
        let prompt = self.auto_prompt.read(cx).value().to_string();
        let (glyph, color) = look(&name, &prompt);
        let label = |text: &'static str, hint: Option<&'static str>| {
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_text()))
                        .child(text),
                )
                .when_some(hint, |el, h| {
                    el.child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(h),
                    )
                })
        };
        let pick = |id: (&'static str, usize), on: bool| {
            div()
                .id(id)
                .h(px(30.))
                .px_2()
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded_lg()
                .border_1()
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |el| {
                    el.border_color(rgb(c_blue()))
                        .bg(tint(c_selected()))
                        .text_color(rgb(c_text()))
                })
                .when(!on, |el| {
                    el.border_color(rgb(c_border()))
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())))
                })
        };
        let mut agents = div().flex().flex_wrap().gap_1p5();
        for (i, p) in agent::PROVIDERS.iter().enumerate() {
            let installed = caps::get(p.id).installed;
            agents = agents.child(
                pick(("pb-agent", i), self.auto_provider == i)
                    .when(!installed, |el| el.opacity(0.55))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.auto_provider = i;
                        cx.notify();
                    }))
                    .child(self.provider_badge(p, 18.))
                    .child(p.name),
            );
        }
        let mut projects = div().flex().flex_wrap().gap_1p5().child(
            pick(("pb-project-any", 0), self.auto_project.is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.auto_project = None;
                    if let Some(e) = this.pb_editor.as_mut() {
                        e.error = None;
                    }
                    cx.notify();
                }))
                .child(icon(IconName::Layers, 13., c_muted()))
                .child("Any project"),
        );
        for (i, p) in self.state.projects.iter().enumerate() {
            let pid = p.id;
            projects = projects.child(
                pick(("pb-project", i), self.auto_project == Some(pid))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.auto_project = Some(pid);
                        if let Some(e) = this.pb_editor.as_mut() {
                            e.error = None;
                        }
                        cx.notify();
                    }))
                    .child(icon(
                        projects::icon_of(p),
                        13.,
                        projects::color_of(p).unwrap_or(c_muted()),
                    ))
                    .child(p.name.clone()),
            );
        }
        let mut schedule = div()
            .flex()
            .items_center()
            .p(px(3.))
            .gap(px(2.))
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_canvas()));
        let options = SCHEDULES
            .iter()
            .map(|(l, m)| (l.to_string(), Some(*m)))
            .chain(std::iter::once(("Custom".to_string(), None)));
        for (j, (text, minutes)) in options.enumerate() {
            let on = match minutes {
                Some(m) => !editor.custom && editor.every == m,
                None => editor.custom,
            };
            schedule = schedule.child(
                div()
                    .id(("pb-every", j))
                    .h(px(26.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |el| {
                        el.bg(tint(c_selected()))
                            .text_color(rgb(c_blue()))
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!on, |el| {
                        el.text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(e) = this.pb_editor.as_mut() {
                            match minutes {
                                Some(m) => {
                                    e.every = m;
                                    e.custom = false;
                                }
                                None => e.custom = true,
                            }
                            e.error = None;
                        }
                        cx.notify();
                    }))
                    .child(text),
            );
        }
        let found = crate::pbfile::placeholders(&prompt);
        let inputs = if found.is_empty() {
            div()
                .text_size(px(12.))
                .text_color(rgb(c_muted()))
                .child("None. It runs as written.")
        } else {
            let mut rows = div().v_flex().gap_1p5();
            for key in &found {
                let Some((_, label, default)) = editor.inputs.iter().find(|(k, _, _)| k == key)
                else {
                    continue;
                };
                rows = rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(110.))
                                .flex_none()
                                .truncate()
                                .text_size(px(12.))
                                .font_family(self.mono_font.clone())
                                .text_color(rgb(c_secondary()))
                                .child(format!("{{{{{key}}}}}")),
                        )
                        .child(div().flex_1().child(Input::new(label).bordered(true).h(px(28.))))
                        .child(div().flex_1().child(Input::new(default).bordered(true).h(px(28.)))),
                );
            }
            rows
        };
        let mut permissions = div().flex().flex_wrap().gap_1p5();
        for (j, (key, title, hint)) in crate::pbfile::PERMISSIONS.iter().enumerate() {
            let on = editor.permissions.iter().any(|p| p == key);
            let fixed = *key == "read";
            permissions = permissions.child(
                pick(("pb-perm", j), on)
                    .when(!fixed, |el| {
                        el.on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(e) = this.pb_editor.as_mut() {
                                if e.permissions.iter().any(|p| p == key) {
                                    e.permissions.retain(|p| p != key);
                                } else {
                                    e.permissions.push(key.to_string());
                                }
                                e.permissions.sort_by_key(|p| {
                                    crate::pbfile::PERMISSIONS.iter().position(|q| q.0 == p)
                                });
                            }
                            cx.notify();
                        }))
                    })
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(*hint).build(window, cx)
                    })
                    .child(icon(
                        if on {
                            IconName::SquareCheck
                        } else {
                            IconName::Square
                        },
                        13.,
                        if on { c_blue() } else { c_muted() },
                    ))
                    .child(*title),
            );
        }
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let editing = editor.editing.is_some();
        let chars = prompt.chars().count();
        let card = div()
            .id("pb-editor")
            .w(px(680.))
            .max_w(relative(0.94))
            .max_h(relative(0.92))
            .v_flex()
            .rounded_2xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .overflow_hidden()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_6()
                    .py_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(
                            rgb(settings::mix(color, c_canvas(), if is_dark() { 0.1 } else { 0.05 })),
                            0.,
                        ),
                        linear_color_stop(rgb(c_canvas()), 1.),
                    ))
                    .child(Self::playbook_tile(glyph, color, 40.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(if name.trim().is_empty() {
                                        if editing {
                                            "Edit playbook".to_string()
                                        } else {
                                            "New playbook".to_string()
                                        }
                                    } else {
                                        name.trim().to_string()
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .child("Runs as a new session, or in the current one when you type / in a message."),
                            ),
                    )
                    .child(
                        self.ghost_button_sized("pb-close", IconName::X, "Close", 28.)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pb_editor = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("pb-editor-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_6()
                    .py_5()
                    .v_flex()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .v_flex()
                                    .gap_1p5()
                                    .child(label("Name", None))
                                    .child(Input::new(&self.auto_name).bordered(true).h(px(34.))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .v_flex()
                                    .gap_1p5()
                                    .child(label("Description", Some("optional")))
                                    .child(Input::new(&self.auto_desc).bordered(true).h(px(34.))),
                            ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(label("Instructions", Some("sent to the agent as the task")))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(c_muted()))
                                            .child(format!("{chars} characters")),
                                    ),
                            )
                            .child(
                                div().text_size(px(13.)).child(
                                    Textarea::new(&self.auto_prompt).bordered(true).h(px(150.)),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(label("Inputs", Some("write {{name}} in the instructions to ask for a value on each run")))
                            .child(inputs),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(label("Permissions", Some("shown before anyone installs or runs it")))
                            .child(permissions),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(label("Agent", Some("faded ones aren't installed")))
                            .child(agents),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(label("Project", Some("Any project runs wherever you start it")))
                            .child(projects),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(label("Schedule", Some("runs while Runo is open, in the project above, on its default connection")))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(schedule)
                                    .when(editor.custom, |el| {
                                        el.child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div().w(px(90.)).child(
                                                        Input::new(&self.auto_every)
                                                            .bordered(true)
                                                            .h(px(30.)),
                                                    ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(12.))
                                                        .text_color(rgb(c_muted()))
                                                        .child("minutes"),
                                                ),
                                        )
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .px_6()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(c_border()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(rgb(if editor.error.is_some() { c_red() } else { c_muted() }))
                            .child(editor.error.clone().unwrap_or_else(|| {
                                format!(
                                    "Type /{} in a message to run it there.",
                                    store::Automation {
                                        id: 0,
                                        name: if name.trim().is_empty() { "playbook".into() } else { name.clone() },
                                        project_id: 0,
                                        provider: String::new(),
                                        prompt: String::new(),
                                        description: String::new(),
                                        every_minutes: 0,
                                        last_run: 0,
                                        enabled: true,
                                        ran: 0,
                                        ..Default::default()
                                    }
                                    .slug()
                                )
                            })),
                    )
                    .child(
                        self.button("pb-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pb_editor = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("pb-save", if editing { "Save changes" } else { "Save playbook" }, true)
                            .child(icon(IconName::Check, 12., c_canvas()))
                            .on_click(cx.listener(|this, _, _, cx| this.save_playbook_editor(cx))),
                    ),
            );
        Some(
            fade(
                div()
                    .id("pb-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(backdrop)
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pb_editor = None;
                        cx.notify();
                    }))
                    .child(rise(card, "pb-editor-rise", 300)),
                "pb-backdrop-fade",
                200,
            )
            .into_any_element(),
        )
    }
}

/// What a playbook may do beyond reading, as "Edits files · Runs commands"; None when
/// it only reads.
pub(crate) fn permission_summary(permissions: &[String]) -> Option<String> {
    let words: Vec<&str> = permissions
        .iter()
        .filter_map(|p| match p.as_str() {
            "edit" => Some("Edits files"),
            "shell" => Some("Runs commands"),
            "network" => Some("Uses the network"),
            _ => None,
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" · "))
}

/// Lay cards out in rows of `n` equal columns. Flex rows instead of a grid:
/// a grid sizes its tracks by measuring every card, which made each frame
/// (a blinking cursor in the editor above, say) slow on a long list.
fn columns(items: Vec<AnyElement>, n: usize) -> Div {
    let mut items = items.into_iter().peekable();
    let mut list = div().v_flex().gap_3();
    while items.peek().is_some() {
        let mut row = div().flex().gap_3();
        for _ in 0..n {
            row = row.child(match items.next() {
                Some(item) => div().flex_1().min_w_0().flex().flex_col().child(item),
                None => div().flex_1().min_w_0(),
            });
        }
        list = list.child(row);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn times_read_naturally() {
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 125 * 60), "2 h ago");
        assert_eq!(ago(0, 86_400), "1 day ago");
        assert_eq!(every_label(0), "By hand");
        assert_eq!(every_label(60), "Hourly");
        assert_eq!(every_label(360), "Every 6 h");
        assert_eq!(every_label(1440), "Daily");
        assert_eq!(every_label(90), "Every 90 min");
        assert_eq!(until(1000, 1000), "now");
    }

    #[test]
    fn templates_get_fitting_icons() {
        for (name, prompt) in PLAYBOOK_TEMPLATES {
            assert_ne!(look(name, prompt).0, IconName::Zap, "{name}");
        }
    }
}
