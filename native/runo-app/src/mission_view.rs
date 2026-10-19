//! The Missions page: the list of missions, the New mission composer, planning, the
//! plan laid out in waves, the live board and the ready-to-ship result.

use super::*;
use crate::mission::{Mission, Part, PartState, Stage};

/// Width of the list of missions.
const LIST_WIDTH: f32 = 280.;
/// Parallel parts offered on the form.
const PARALLEL: [u32; 4] = [1, 2, 3, 4];
/// Goals offered to start from on the New mission page.
const EXAMPLES: [&str; 3] = [
    "Add a dark theme with a switch in Settings, and remember the choice after a restart",
    "Add input validation and clear error messages to every form",
    "Write tests for the modules that have none, and fix the bugs they find",
];
/// The steps each part goes through, as the board's pipeline shows them.
const STEPS: [&str; 4] = ["Work", "Checks", "Review", "Merge"];

fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Planning => "Planning",
        Stage::Plan => "Check the plan",
        Stage::Running => "Running",
        Stage::Paused => "Paused",
        Stage::Landing => "Landing",
        Stage::Ready => "Ready to ship",
        Stage::Failed => "Planning failed",
    }
}

/// "45s", "4m 12s" or "1h 3m".
fn took(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// The text cut at a word before `max` characters, with an ellipsis.
fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let at = cut.rfind(' ').filter(|i| *i > max / 2).unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end_matches([',', '.', ';', ':']))
}

fn since(start: u64, end: u64) -> u64 {
    if start == 0 {
        return 0;
    }
    let end = if end == 0 {
        crate::workspace::unix_now()
    } else {
        end
    };
    end.saturating_sub(start)
}

/// A soft fill of an agent's colour, for stripes and badges.
fn shade(color: u32, amount: f32) -> u32 {
    settings::mix(color, c_canvas(), amount)
}

fn spin(id: impl Into<ElementId>, size: f32, color: u32) -> AnyElement {
    icon(IconName::LoaderCircle, size, color).animated(
        id,
        Animation::new(Duration::from_millis(900)).repeat(),
        |i, d| i.rotate(percentage(d)),
    )
}

/// A dot that breathes, for the step a part is on now.
fn breathing(id: impl Into<ElementId>, size: f32, color: u32) -> AnyElement {
    div().size(px(size)).rounded_full().bg(rgb(color)).animated(
        id,
        Animation::new(Duration::from_millis(1400)).repeat(),
        |el, d| el.opacity(0.35 + 0.65 * crate::askbar::pulse(d)),
    )
}

fn small_caps(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_muted()))
        .child(text.into())
}

/// A number with its label under it, for the mission's stats.
fn stat(value: String, label: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex_1()
        .min_w(px(96.))
        .px_3()
        .py_2()
        .v_flex()
        .gap(px(2.))
        .rounded_lg()
        .border_1()
        .border_color(rgb(c_border()))
        .bg(rgb(c_raised()))
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(color))
                .child(value),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child(label.into()),
        )
}

fn part_tone(state: PartState) -> u32 {
    match state {
        PartState::Done => c_green(),
        PartState::Stuck => c_amber(),
        PartState::Waiting | PartState::Skipped => c_muted(),
        _ => c_blue(),
    }
}

impl RunoApp {
    pub(crate) fn missions_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut missions: Vec<&Mission> = self.state.missions.iter().collect();
        missions.sort_by_key(|m| std::cmp::Reverse(m.id));
        let rows: Vec<AnyElement> = missions.iter().map(|m| self.mission_row(m, cx)).collect();
        let none = missions.is_empty();
        let on_form = self.mission_open.is_none();
        let side = div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .border_r_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .child(
                div()
                    .h(px(46.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(icon(IconName::Rocket, 14., c_text()))
                    .child("Missions")
                    .child(div().flex_1())
                    .child(
                        self.button("mission-new", "New", false)
                            .h(px(26.))
                            .px_2()
                            .text_size(px(12.))
                            .when(on_form, |el| el.border_color(rgb(c_blue())))
                            .child(icon(IconName::Plus, 12., c_secondary()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.mission_open = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("mission-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .v_flex()
                    .gap_1()
                    .py_1()
                    .children(rows)
                    .when(none, |el| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(12.5))
                                .text_color(rgb(c_muted()))
                                .child("Your missions show here, with how far along each one is."),
                        )
                    }),
            );
        let main = match self.mission_open.and_then(|id| self.mission(id)) {
            Some(m) => self.mission_page(m, cx),
            None => self.mission_form_view(cx),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(side)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .bg(glass_fill(c_canvas()))
                    .child(main),
            )
            .into_any_element()
    }

    /// The agents of a mission's parts, as overlapping badges.
    fn agent_stack(&self, agents: &[String], size: f32) -> Div {
        let mut seen: Vec<&str> = Vec::new();
        for a in agents {
            if !seen.contains(&a.as_str()) {
                seen.push(a);
            }
        }
        div()
            .flex()
            .children(seen.into_iter().enumerate().map(|(i, a)| {
                div()
                    .when(i > 0, |el| el.ml(px(-size * 0.3)))
                    .rounded(px((size * 0.3).min(6.)))
                    .border_1()
                    .border_color(rgb(c_sidebar()))
                    .child(self.provider_badge(agent::provider(a), size))
            }))
    }

    fn mission_row(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let id = m.id;
        let on = self.mission_open == Some(id);
        let (done, total) = m.progress();
        let stuck = m.parts.iter().any(|p| p.state == PartState::Stuck);
        let tone = match m.stage {
            Stage::Ready => c_green(),
            Stage::Failed => c_red(),
            Stage::Plan => c_amber(),
            _ if stuck => c_amber(),
            Stage::Paused => c_muted(),
            _ => c_blue(),
        };
        let label = if stuck && matches!(m.stage, Stage::Running | Stage::Paused) {
            "Needs you"
        } else {
            stage_label(m.stage)
        };
        let agents: Vec<String> = if m.parts.is_empty() {
            m.agents.clone()
        } else {
            m.parts.iter().map(|p| p.agent.clone()).collect()
        };
        let live = matches!(m.stage, Stage::Planning | Stage::Running | Stage::Landing);
        div()
            .id(("mission-row", id as usize))
            .mx_1()
            .px_2p5()
            .py_2()
            .v_flex()
            .gap_1p5()
            .rounded_lg()
            .cursor_pointer()
            .border_1()
            .when(on, |el| {
                el.bg(tint(c_active())).border_color(rgb(c_border()))
            })
            .when(!on, |el| {
                el.border_color(rgba(0)).hover(|el| el.bg(tint(c_subtle())))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.mission_open = Some(id);
                cx.notify();
            }))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(short(m.goal.lines().next().unwrap_or(""), 64)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(11.5))
                    .child(if live {
                        spin(("mission-row-spin", id as usize), 11., tone)
                    } else {
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(rgb(tone))
                            .into_any_element()
                    })
                    .child(div().text_color(rgb(tone)).child(label))
                    .when(total > 0 && m.stage != Stage::Plan, |el| {
                        el.child(
                            div()
                                .text_color(rgb(c_muted()))
                                .child(format!("·  {done}/{total}")),
                        )
                    })
                    .child(div().flex_1())
                    .child(self.agent_stack(&agents, 16.)),
            )
            .when(total > 0 && m.stage != Stage::Plan, |el| {
                el.child(self.part_segments(m, 4.))
            })
            .into_any_element()
    }

    /// One segment per part, coloured by where it stands.
    fn part_segments(&self, m: &Mission, height: f32) -> Div {
        div().flex().gap(px(3.)).children(m.parts.iter().map(|p| {
            let fill = match p.state {
                PartState::Done => c_green(),
                PartState::Stuck => c_amber(),
                PartState::Waiting | PartState::Skipped => c_border(),
                _ => shade(c_blue(), 0.75),
            };
            div().flex_1().h(px(height)).rounded_full().bg(rgb(fill))
        }))
    }

    // ---------- new mission ----------

    fn mission_form_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let form = &self.mission_form;
        let installed = self.installed_agents(cx);
        let project = Some(form.project)
            .filter(|p| *p != 0)
            .or_else(|| self.current_project())
            .or_else(|| self.state.projects.first().map(|p| p.id));
        let project_name = project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "No project".into());
        let projects: Vec<u64> = self.state.projects.iter().map(|p| p.id).collect();
        let parallel = form.parallel;
        let review = form.review;

        // Agents that may work on it: badges that light up when picked.
        let agent_pills: Vec<AnyElement> = installed
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let id = *id;
                let p = agent::provider(id);
                let order = form.agents.iter().position(|a| a == id);
                let on = order.is_some();
                div()
                    .id(("mission-agent", i))
                    .h(px(30.))
                    .pl_1()
                    .pr_2p5()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .rounded_full()
                    .border_1()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |el| {
                        el.border_color(rgb(shade(p.color, 0.7)))
                            .bg(rgb(shade(p.color, if is_dark() { 0.16 } else { 0.08 })))
                            .text_color(rgb(c_text()))
                    })
                    .when(!on, |el| {
                        el.border_color(rgb(c_border()))
                            .text_color(rgb(c_muted()))
                            .opacity(0.75)
                            .hover(|el| el.opacity(1.))
                    })
                    .child(self.provider_badge(p, 22.))
                    .child(p.name)
                    .when(order == Some(0), |el| {
                        el.child(
                            div()
                                .px_1p5()
                                .rounded_md()
                                .bg(tint(c_subtle()))
                                .text_size(px(10.5))
                                .text_color(rgb(c_secondary()))
                                .child("plans"),
                        )
                    })
                    .when(on, |el| el.child(icon(IconName::Check, 12., p.color)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let agents = &mut this.mission_form.agents;
                        match agents.iter().position(|a| a == id) {
                            Some(at) => {
                                agents.remove(at);
                            }
                            None => agents.push(id.to_string()),
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();

        let segment = div()
            .flex()
            .items_center()
            .p(px(2.))
            .rounded_md()
            .bg(tint(c_subtle()))
            .children(PARALLEL.iter().enumerate().map(|(i, n)| {
                let n = *n;
                let on = parallel == n;
                div()
                    .id(("mission-parallel", i))
                    .w(px(26.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .when(on, |el| {
                        el.bg(rgb(c_raised()))
                            .text_color(rgb(c_text()))
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!on, |el| el.text_color(rgb(c_muted())))
                    .child(n.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mission_form.parallel = n;
                        cx.notify();
                    }))
            }));
        let switch = div()
            .id("mission-review")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(rgb(c_secondary()))
            .child(
                div()
                    .w(px(28.))
                    .h(px(16.))
                    .p(px(2.))
                    .flex()
                    .rounded_full()
                    .when(review, |el| el.bg(rgb(c_blue())).justify_end())
                    .when(!review, |el| el.bg(rgb(c_border())))
                    .child(div().size(px(12.)).rounded_full().bg(rgb(0xffffff))),
            )
            .child("Second opinion")
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(
                    "Another vendor's agent reviews each part before it is merged",
                )
                .build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.mission_form.review = !review;
                cx.notify();
            }));

        let composer = div()
            .w_full()
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()))
            .shadow_lg()
            .child(
                div().px_2().pt_2().child(
                    Textarea::new(&self.mission_goal)
                        .bordered(false)
                        .h(px(128.)),
                ),
            )
            .child(
                div()
                    .px_3()
                    .pb_3()
                    .pt_1()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(small_caps("TEAM").mr_1())
                    .children(agent_pills)
                    .when(installed.is_empty(), |el| {
                        el.child(
                            div().text_size(px(12.)).text_color(rgb(c_amber())).child(
                                "No agent CLIs are installed. Add one in Settings → Providers.",
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .border_t_1()
                    .border_color(divider())
                    .child(
                        div()
                            .id("mission-project")
                            .h(px(26.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .rounded_md()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
                            .child(icon(IconName::Folder, 12., c_muted()))
                            .child(project_name)
                            .when(projects.len() > 1, |el| {
                                el.child(icon(IconName::ChevronsUpDown, 11., c_muted()))
                            })
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("The project it runs in")
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if projects.is_empty() {
                                    return;
                                }
                                let at = project
                                    .and_then(|p| projects.iter().position(|x| *x == p))
                                    .map(|i| (i + 1) % projects.len())
                                    .unwrap_or(0);
                                this.mission_form.project = projects[at];
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .child("At once")
                            .child(segment),
                    )
                    .child(switch)
                    .child(div().flex_1())
                    .child(
                        self.button("mission-plan", "Plan mission", true)
                            .h(px(32.))
                            .px_4()
                            .child(icon(IconName::Sparkles, 13., c_canvas()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.mission_form_defaults(cx);
                                this.plan_mission(window, cx);
                                cx.notify();
                            })),
                    ),
            );

        let examples = div()
            .w_full()
            .v_flex()
            .gap_2()
            .child(small_caps("OR START FROM"))
            .children(EXAMPLES.iter().enumerate().map(|(i, text)| {
                let text = *text;
                div()
                    .id(("mission-example", i))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                    .child(icon(IconName::Lightbulb, 13., c_amber()))
                    .child(div().flex_1().child(text))
                    .child(icon(IconName::ArrowUpRight, 12., c_muted()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.mission_goal
                            .update(cx, |i, cx| i.set_value(text, window, cx));
                        cx.notify();
                    }))
            }));

        let how = [
            (
                IconName::Sparkles,
                "Plan",
                "An agent reads the code and splits the goal into parts",
            ),
            (
                IconName::GitBranch,
                "Run at once",
                "Each part gets its own agent and worktree",
            ),
            (
                IconName::ListChecks,
                "Prove",
                "Checks and a second opinion before anything merges",
            ),
            (
                IconName::GitMerge,
                "Ship one branch",
                "Parts merge in order; conflicts go back to the agent",
            ),
        ];
        let steps = div()
            .w_full()
            .flex()
            .gap_2()
            .children(
                how.into_iter()
                    .enumerate()
                    .map(|(i, (glyph, title, text))| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .p_3()
                            .v_flex()
                            .gap_1p5()
                            .rounded_lg()
                            .bg(tint(c_subtle()))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(24.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_md()
                                            .bg(rgb(shade(c_blue(), 0.18)))
                                            .child(icon(glyph, 13., c_blue())),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(rgb(c_muted()))
                                            .child(format!("{}", i + 1)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(text),
                            )
                    }),
            );

        div()
            .id("mission-form")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(760.))
                    .mx_auto()
                    .px_6()
                    .pt(px(56.))
                    .pb_8()
                    .v_flex()
                    .items_center()
                    .gap_5()
                    .child(
                        div()
                            .size(px(48.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_xl()
                            .border_1()
                            .border_color(rgb(shade(c_blue(), 0.4)))
                            .bg(linear_gradient(
                                135.,
                                linear_color_stop(rgb(shade(c_blue(), 0.35)), 0.),
                                linear_color_stop(rgb(shade(c_blue(), 0.08)), 1.),
                            ))
                            .child(icon(IconName::Rocket, 22., c_blue())),
                    )
                    .child(
                        div()
                            .v_flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_size(px(26.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("What should your agents build?"),
                            )
                            .child(
                                div()
                                    .max_w(px(560.))
                                    .text_center()
                                    .text_size(px(13.5))
                                    .text_color(rgb(c_secondary()))
                                    .child("Describe one goal. Runo plans it, runs the parts on several agents at once, proves each one, and hands you a single branch to ship."),
                            ),
                    )
                    .child(composer)
                    .when_some(form.error.clone(), |el, error| {
                        el.child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(12.5))
                                .text_color(rgb(c_red()))
                                .child(icon(IconName::CircleAlert, 13., c_red()))
                                .child(error),
                        )
                    })
                    .child(examples)
                    .child(steps),
            )
            .into_any_element()
    }

    // ---------- a mission ----------

    fn mission_page(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let mid = m.id;
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == m.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let stuck = m.parts.iter().any(|p| p.state == PartState::Stuck);
        let tone = match m.stage {
            Stage::Ready => c_green(),
            Stage::Failed => c_red(),
            _ if stuck || m.stage == Stage::Plan => c_amber(),
            Stage::Paused => c_muted(),
            _ => c_blue(),
        };
        let lead = m.lead;
        let small = |this: &Self, key: &'static str, label: &'static str, primary: bool| {
            this.button(key, label, primary)
                .h(px(28.))
                .px_2p5()
                .text_size(px(12.))
        };
        let header = div()
            .flex_none()
            .px_5()
            .pt_4()
            .pb_3()
            .v_flex()
            .gap_2()
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_clamp(2)
                            .child(m.goal.trim().to_string()),
                    )
                    .when(m.stage == Stage::Running, |el| {
                        el.child(
                            small(self, "mission-pause", "Pause", false)
                                .child(icon(IconName::Pause, 12., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(m) = this.mission_mut(mid) {
                                        m.stage = Stage::Paused;
                                    }
                                    this.save();
                                    cx.notify();
                                })),
                        )
                    })
                    .when(m.stage == Stage::Paused, |el| {
                        el.child(
                            small(self, "mission-resume", "Resume", true)
                                .child(icon(IconName::Play, 12., c_canvas()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(m) = this.mission_mut(mid) {
                                        m.stage = Stage::Running;
                                    }
                                    this.mission_tick(mid, cx);
                                    this.save();
                                    cx.notify();
                                })),
                        )
                    })
                    .when(lead != 0 && self.state.session(lead).is_some(), |el| {
                        el.child(
                            small(self, "mission-lead", "Mission session", false)
                                .child(icon(IconName::MessageSquare, 12., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select(lead);
                                    this.view = View::Session;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        self.ghost_button_sized(
                            ("mission-delete", mid as usize),
                            IconName::Trash,
                            "Delete the mission (its sessions stay)",
                            28.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_mission(mid);
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .child(
                        div()
                            .px_2()
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .rounded_full()
                            .bg(rgb(shade(tone, 0.16)))
                            .text_color(rgb(tone))
                            .font_weight(FontWeight::MEDIUM)
                            .when(
                                matches!(
                                    m.stage,
                                    Stage::Planning | Stage::Running | Stage::Landing
                                ),
                                |el| el.child(breathing("mission-stage-dot", 6., tone)),
                            )
                            .child(
                                if stuck && matches!(m.stage, Stage::Running | Stage::Paused) {
                                    "Needs you"
                                } else {
                                    stage_label(m.stage)
                                },
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon(IconName::Folder, 11., c_muted()))
                            .child(project),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .font_family("monospace")
                            .child(icon(IconName::GitBranch, 11., c_muted()))
                            .child(format!("{} ← {}", m.branch, m.base)),
                    )
                    .when(!m.parts.is_empty(), |el| {
                        let agents: Vec<String> = m.parts.iter().map(|p| p.agent.clone()).collect();
                        el.child(self.agent_stack(&agents, 18.))
                    }),
            )
            .when(!m.summary.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child(m.summary.clone()),
                )
            });
        let body = match m.stage {
            Stage::Planning => self.planning_view(m),
            Stage::Plan | Stage::Failed => self.plan_review(m, cx),
            _ => self.mission_board(m, cx),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// While the planner reads: what it's doing, for how long, and the plan taking shape.
    fn planning_view(&self, m: &Mission) -> AnyElement {
        let planner = agent::provider(m.agents.first().map(String::as_str).unwrap_or("claude"));
        let secs = since(m.created, 0);
        let ghost = |i: usize, w: f32| {
            div()
                .w_full()
                .p_3()
                .v_flex()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_border()))
                .child(
                    div()
                        .h(px(10.))
                        .w(relative(w))
                        .rounded_full()
                        .bg(tint(c_subtle())),
                )
                .child(
                    div()
                        .h(px(8.))
                        .w(relative(w * 0.8))
                        .rounded_full()
                        .bg(tint(c_subtle())),
                )
                .child(
                    div()
                        .h(px(8.))
                        .w(relative(w * 0.6))
                        .rounded_full()
                        .bg(tint(c_subtle())),
                )
                .animated(
                    ("mission-ghost", i),
                    Animation::new(Duration::from_millis(1600)).repeat(),
                    move |el, d| {
                        el.opacity(0.4 + 0.6 * crate::askbar::pulse((d + i as f32 * 0.25) % 1.))
                    },
                )
        };
        div()
            .id("mission-planning")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(560.))
                    .mx_auto()
                    .pt(px(64.))
                    .px_6()
                    .v_flex()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .size(px(56.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(shade(planner.color, 0.14)))
                            .child(self.provider_badge(planner, 30.)),
                    )
                    .child(
                        div()
                            .v_flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(spin("mission-planning-spin", 14., c_blue()))
                                    .child(format!("{} is reading the project", planner.name)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(rgb(c_muted()))
                                    .child(format!(
                                        "Planning for {}. It only reads; nothing changes until you start.",
                                        took(secs)
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .v_flex()
                            .gap_2()
                            .child(ghost(0, 0.7))
                            .child(ghost(1, 0.55))
                            .child(ghost(2, 0.8)),
                    ),
            )
            .into_any_element()
    }

    /// The plan before it starts, in waves: what starts at once, and what waits.
    fn plan_review(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let mid = m.id;
        let has_parts = !m.parts.is_empty();
        let waves = m.waves();
        let width = waves.iter().map(|w| w.len()).max().unwrap_or(0);
        let mut agents: Vec<&str> = Vec::new();
        for p in &m.parts {
            if !agents.contains(&p.agent.as_str()) {
                agents.push(&p.agent);
            }
        }
        let wave_views: Vec<AnyElement> = waves
            .iter()
            .enumerate()
            .map(|(w, parts)| {
                let after: Vec<String> = if w == 0 {
                    Vec::new()
                } else {
                    waves[..w]
                        .iter()
                        .flatten()
                        .filter(|i| {
                            parts
                                .iter()
                                .any(|p| m.parts[*p].deps.contains(&m.parts[**i].key))
                        })
                        .map(|i| m.parts[*i].title.clone())
                        .collect()
                };
                let last = w + 1 == waves.len();
                let note = if w == 0 {
                    match parts.len() {
                        1 => "Starts right away".to_string(),
                        n => format!("{n} parts start at once"),
                    }
                } else {
                    format!("After {}", after.join(", "))
                };
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .w(px(24.))
                            .flex_none()
                            .v_flex()
                            .items_center()
                            .child(
                                div()
                                    .size(px(24.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(shade(c_blue(), 0.6)))
                                    .bg(rgb(shade(c_blue(), 0.16)))
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(c_blue()))
                                    .child((w + 1).to_string()),
                            )
                            .when(!last, |el| {
                                el.child(
                                    div()
                                        .flex_1()
                                        .w(px(2.))
                                        .my_1()
                                        .rounded_full()
                                        .bg(rgb(c_border())),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap_2()
                            .pb_5()
                            .child(
                                div()
                                    .h(px(24.))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!("Wave {}", w + 1)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(c_muted()))
                                            .child(note),
                                    ),
                            )
                            .child(
                                div().flex().flex_wrap().gap_2().children(
                                    parts
                                        .iter()
                                        .map(|i| self.mission_plan_card(m, *i, &m.parts[*i], cx)),
                                ),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let summary = format!(
            "{} part{}  ·  {} wave{}  ·  up to {} at once",
            m.parts.len(),
            if m.parts.len() == 1 { "" } else { "s" },
            waves.len(),
            if waves.len() == 1 { "" } else { "s" },
            width.min(m.parallel.max(1) as usize)
        );
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(
                div()
                    .id("mission-plan")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_5()
                    .py_4()
                    .v_flex()
                    .gap_4()
                    .when(m.stage == Stage::Failed, |el| {
                        el.child(
                            div()
                                .p_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(shade(c_red(), 0.5)))
                                .bg(rgb(shade(c_red(), 0.1)))
                                .text_size(px(12.5))
                                .child(icon(IconName::CircleAlert, 14., c_red()))
                                .child(div().flex_1().child(m.note.clone()))
                                .child(div().text_color(rgb(c_muted())).child("Plan again below.")),
                        )
                    })
                    .when(has_parts, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .child(icon(IconName::ListTree, 13., c_muted()))
                                .child(summary),
                        )
                    })
                    .children(wave_views),
            )
            .child(
                div()
                    .flex_none()
                    .px_5()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(divider())
                    .bg(pane(c_sidebar()))
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.mission_feedback).bordered(true).h(px(34.))),
                    )
                    .child(
                        self.button("mission-replan", "Plan again", !has_parts)
                            .h(px(34.))
                            .child(icon(
                                IconName::RotateCw,
                                12.,
                                if has_parts { c_secondary() } else { c_canvas() },
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let feedback =
                                    this.mission_feedback.read(cx).value().trim().to_string();
                                this.mission_feedback
                                    .update(cx, |i, cx| i.set_value("", window, cx));
                                this.run_planner(mid, feedback, cx);
                                this.save();
                                cx.notify();
                            })),
                    )
                    .when(has_parts, |el| {
                        el.child(
                            self.button("mission-start", "Start mission", true)
                                .h(px(34.))
                                .px_4()
                                .child(icon(IconName::Rocket, 13., c_canvas()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.start_mission(mid, cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn mission_plan_card(
        &self,
        m: &Mission,
        index: usize,
        part: &Part,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mid = m.id;
        let provider = agent::provider(&part.agent);
        let pool = m.agents.clone();
        let mut preview: String = part.prompt.chars().take(300).collect();
        if part.prompt.chars().count() > 300 {
            preview.push('…');
        }
        let deps = m.dep_titles(part);
        div()
            .w(px(400.))
            .flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()))
            .overflow_hidden()
            .child(div().w(px(3.)).flex_none().bg(rgb(provider.color)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .p_3()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(20.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(tint(c_subtle()))
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child((index + 1).to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_clamp(2)
                                    .child(part.title.clone()),
                            )
                            .child(
                                self.ghost_button_sized(
                                    ("mission-part-remove", index),
                                    IconName::X,
                                    "Leave this part out",
                                    22.,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if let Some(m) = this.mission_mut(mid) {
                                            m.remove_part(index);
                                        }
                                        this.save();
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(rgb(c_secondary()))
                            .line_clamp(5)
                            .child(preview),
                    )
                    .when(!part.files.is_empty(), |el| {
                        el.child(div().flex().flex_wrap().gap_1().children(
                            part.files.iter().take(6).map(|f| {
                                div()
                                    .px_1p5()
                                    .h(px(18.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(4.))
                                    .bg(tint(c_subtle()))
                                    .font_family("monospace")
                                    .text_size(px(10.5))
                                    .text_color(rgb(c_secondary()))
                                    .child(f.clone())
                            }),
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id(("mission-part-agent", index))
                                    .h(px(24.))
                                    .pl_0p5()
                                    .pr_2()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(shade(provider.color, 0.6)))
                                    .cursor_pointer()
                                    .hover(|el| el.bg(tint(c_subtle())))
                                    .text_size(px(11.5))
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            "Switch the agent for this part",
                                        )
                                        .build(window, cx)
                                    })
                                    .child(self.provider_badge(provider, 18.))
                                    .child(provider.name)
                                    .when(pool.len() > 1, |el| {
                                        el.child(icon(IconName::ChevronsUpDown, 10., c_muted()))
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Some(p) = this
                                            .mission_mut(mid)
                                            .and_then(|m| m.parts.get_mut(index))
                                        else {
                                            return;
                                        };
                                        let at = pool
                                            .iter()
                                            .position(|a| *a == p.agent)
                                            .map(|i| i + 1)
                                            .unwrap_or(0);
                                        if let Some(next) = pool.get(at % pool.len().max(1)) {
                                            p.agent = next.clone();
                                        }
                                        this.save();
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .when(!deps.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_size(px(11.))
                                        .text_color(rgb(c_muted()))
                                        .child(icon(IconName::CornerDownRight, 11., c_muted()))
                                        .child(format!("after {}", deps.join(", "))),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    // ---------- the board ----------

    fn mission_board(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let (done, total) = m.progress();
        let working = m.parts.iter().filter(|p| p.state.busy()).count();
        let conflicts: u32 = m.parts.iter().map(|p| p.conflicts).sum();
        let cents: f64 = m
            .parts
            .iter()
            .map(|p| p.session)
            .chain(std::iter::once(m.lead))
            .filter_map(|id| self.state.session(id))
            .map(crate::health::session_cost)
            .sum();
        let elapsed = since(m.started, m.finished);
        let ready = m.stage == Stage::Ready;
        let stats = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(stat(
                format!("{done}/{total}"),
                "parts merged",
                if done == total && total > 0 {
                    c_green()
                } else {
                    c_text()
                },
            ))
            .child(stat(
                working.to_string(),
                if working == 1 {
                    "agent working"
                } else {
                    "agents working"
                },
                if working > 0 { c_blue() } else { c_text() },
            ))
            .child(stat(
                conflicts.to_string(),
                if conflicts == 1 {
                    "conflict resolved"
                } else {
                    "conflicts resolved"
                },
                c_text(),
            ))
            .child(stat(
                took(elapsed),
                if ready {
                    "start to ready"
                } else {
                    "running for"
                },
                c_text(),
            ))
            .child(stat(
                if cents > 0. {
                    format!("${:.2}", cents / 100.)
                } else {
                    "—".into()
                },
                "agent cost",
                c_text(),
            ));
        let banner = self.mission_banner(m, cx);
        let titles = [
            (
                "Waiting",
                c_muted(),
                "Parts waiting on others, or on a free agent",
            ),
            ("Working", c_blue(), "Agents writing code"),
            (
                "Checking",
                c_purple_or_blue(),
                "Checks, second opinion and merge",
            ),
            ("Done", c_green(), "Merged into the mission branch"),
        ];
        let mut columns: Vec<Vec<AnyElement>> =
            vec![Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (i, p) in m.parts.iter().enumerate() {
            columns[p.state.column()].push(self.part_card(m, i, p, cx));
        }
        let board =
            div()
                .flex()
                .items_start()
                .gap_3()
                .children(columns.into_iter().enumerate().map(|(c, cards)| {
                    let (title, color, empty) = titles[c];
                    let count = cards.len();
                    div()
                        .flex_1()
                        .min_w_0()
                        .v_flex()
                        .gap_2()
                        .child(
                            div()
                                .px_1()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(div().size(px(7.)).rounded_full().bg(rgb(color)))
                                .child(small_caps(title.to_uppercase()))
                                .child(
                                    div()
                                        .px_1p5()
                                        .rounded_full()
                                        .bg(tint(c_subtle()))
                                        .text_size(px(10.5))
                                        .text_color(rgb(c_secondary()))
                                        .child(count.to_string()),
                                ),
                        )
                        .child(
                            div()
                                .id(("mission-column", c))
                                .flex_1()
                                .v_flex()
                                .gap_2()
                                .p_1p5()
                                .rounded_xl()
                                .bg(tint(c_subtle()))
                                .children(cards)
                                .when(count == 0, |el| {
                                    el.child(
                                        div()
                                            .py_6()
                                            .px_3()
                                            .text_center()
                                            .text_size(px(11.5))
                                            .text_color(rgb(c_muted()))
                                            .child(empty),
                                    )
                                }),
                        )
                }));
        div()
            .id("mission-board")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .v_flex()
            .gap_4()
            .children(banner)
            .child(stats)
            .when(total > 0, |el| el.child(self.part_segments(m, 6.)))
            .child(board)
            .into_any_element()
    }

    /// What the mission needs from you, or what it's doing as a whole.
    fn mission_banner(&self, m: &Mission, cx: &mut Context<Self>) -> Option<AnyElement> {
        let lead = m.lead;
        let card = |tone: u32| {
            div()
                .p_4()
                .flex()
                .items_center()
                .gap_3()
                .rounded_xl()
                .border_1()
                .border_color(rgb(shade(tone, 0.5)))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(shade(tone, 0.16)), 0.),
                    linear_color_stop(rgb(shade(tone, 0.04)), 1.),
                ))
        };
        let words = |title: String, text: String| {
            div()
                .flex_1()
                .min_w_0()
                .v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child(text),
                )
        };
        let badge = |glyph: IconName, tone: u32| {
            div()
                .size(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(shade(tone, 0.22)))
                .child(icon(glyph, 20., tone))
        };
        match m.stage {
            Stage::Ready => Some(
                card(c_green())
                    .child(badge(IconName::PartyPopper, c_green()))
                    .child(words("Ready to ship".into(), m.note.clone()))
                    .child(
                        self.button("mission-ship", "Review and ship", true)
                            .h(px(34.))
                            .px_4()
                            .child(icon(IconName::GitCompare, 13., c_canvas()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.review(lead);
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            ),
            Stage::Landing => Some(
                card(c_blue())
                    .child(if self.proving.contains(&lead) || self.turn_running(lead) {
                        div()
                            .size(px(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(spin("mission-landing-spin", 22., c_blue()))
                            .into_any_element()
                    } else {
                        badge(IconName::GitMerge, c_blue()).into_any_element()
                    })
                    .child(words("Landing".into(), m.note.clone()))
                    .into_any_element(),
            ),
            _ if m.waiting_on_you() => Some(
                card(c_amber())
                    .child(badge(IconName::TriangleAlert, c_amber()))
                    .child(words(
                        "Waiting on you".into(),
                        "Nothing else can move until you look at the parts that need you, below."
                            .into(),
                    ))
                    .into_any_element(),
            ),
            Stage::Paused => Some(
                card(c_muted())
                    .child(badge(IconName::Pause, c_muted()))
                    .child(words(
                        "Paused".into(),
                        "No new parts start. Parts already running finish their turn.".into(),
                    ))
                    .into_any_element(),
            ),
            _ => None,
        }
    }

    /// What a part's agent is doing now: its latest step, or its latest words.
    fn part_activity(&self, session: u64) -> Option<String> {
        let s = self.state.session(session)?;
        let last = s.messages.iter().rev().find(|m| m.role == Role::Agent)?;
        let line = last.steps.last().cloned().or_else(|| {
            last.text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(str::to_string)
        })?;
        let line = line.trim().to_string();
        (!line.is_empty()).then_some(line)
    }

    fn part_card(
        &self,
        m: &Mission,
        index: usize,
        part: &Part,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mid = m.id;
        let provider = agent::provider(&part.agent);
        let session = part.session;
        let state = part.state;
        let stuck = state == PartState::Stuck;
        let tone = part_tone(state);
        let asking = session != 0 && self.approval_for(session).is_some();
        let deps: Vec<String> = m
            .parts
            .iter()
            .filter(|d| part.deps.contains(&d.key) && !d.state.settled())
            .map(|d| d.title.clone())
            .collect();
        let activity = (state == PartState::Working && session != 0)
            .then(|| self.part_activity(session))
            .flatten();
        let checks = self
            .state
            .session(session)
            .and_then(|s| s.proof.as_ref())
            .filter(|p| !p.results.is_empty());
        let elapsed = since(part.started, part.finished);
        let step = state.step();

        // Work → Checks → Review → Merge, with the current step breathing.
        let pipeline = (step.is_some() || stuck).then(|| {
            let at = step.unwrap_or(0);
            let review = m.review;
            let shown: Vec<(usize, &str)> = STEPS
                .iter()
                .enumerate()
                .filter(|(i, _)| review || *i != 2)
                .map(|(i, l)| (i, *l))
                .collect();
            let last = shown.last().map(|(i, _)| *i).unwrap_or(0);
            let mut row = div().flex().items_center().gap_1();
            for (i, label) in shown {
                let done = at > i;
                let now = at == i && !stuck;
                let color = if done {
                    c_green()
                } else if now {
                    c_blue()
                } else if stuck && i == 0 {
                    c_amber()
                } else {
                    c_border()
                };
                row = row
                    .child(if now {
                        breathing(("mission-step", index * 8 + i), 7., color)
                    } else {
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(color))
                            .into_any_element()
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.5))
                            .text_color(rgb(if done || now {
                                c_secondary()
                            } else {
                                c_muted()
                            }))
                            .child(label),
                    );
                if i != last {
                    row = row.child(div().flex_1().min_w(px(6.)).h(px(1.)).bg(rgb(if done {
                        shade(c_green(), 0.6)
                    } else {
                        c_border()
                    })));
                }
            }
            row
        });

        let small = |this: &Self, key: &'static str, label: &'static str, primary: bool| {
            this.button((key, index), label, primary)
                .h(px(24.))
                .px_2()
                .text_size(px(11.5))
        };
        div()
            .id(("mission-part", index))
            .flex()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if stuck || asking {
                c_amber()
            } else {
                c_border()
            }))
            .bg(rgb(c_raised()))
            .overflow_hidden()
            .when(state == PartState::Skipped, |el| el.opacity(0.55))
            .when(session != 0, |el| {
                el.cursor_pointer()
                    .hover(|el| el.border_color(rgb(shade(c_blue(), 0.7))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(session);
                        this.view = View::Session;
                        cx.notify();
                    }))
            })
            .child(div().w(px(3.)).flex_none().bg(rgb(provider.color)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .p_2p5()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .line_clamp(2)
                                    .child(part.title.clone()),
                            )
                            .child(self.provider_badge(provider, 18.)),
                    )
                    .children(pipeline)
                    .when_some(activity, |el, line| {
                        el.child(
                            div()
                                .px_2()
                                .py_1()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .rounded_md()
                                .bg(tint(c_subtle()))
                                .child(spin(("mission-part-spin", index), 10., c_blue()))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family("monospace")
                                        .text_size(px(10.5))
                                        .text_color(rgb(c_secondary()))
                                        .child(line),
                                ),
                        )
                    })
                    .when(asking, |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(11.5))
                                .text_color(rgb(c_amber()))
                                .child(icon(IconName::ShieldAlert, 11., c_amber()))
                                .child("Asks to run a command. Open it to answer."),
                        )
                    })
                    .when(state == PartState::Waiting && !deps.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(icon(IconName::CornerDownRight, 11., c_muted()))
                                .child(format!("after {}", deps.join(", "))),
                        )
                    })
                    .when(stuck && !part.note.is_empty(), |el| {
                        el.child(
                            div()
                                .p_2()
                                .rounded_md()
                                .bg(rgb(shade(c_amber(), 0.12)))
                                .text_size(px(11.5))
                                .text_color(rgb(c_text()))
                                .child(part.note.clone()),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(div().text_color(rgb(tone)).child(state.label()))
                            .when_some(checks, |el, p| {
                                let failed = p.failed().len();
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_0p5()
                                        .text_color(rgb(if failed == 0 {
                                            c_green()
                                        } else {
                                            c_red()
                                        }))
                                        .child(icon(
                                            if failed == 0 {
                                                IconName::CircleCheck
                                            } else {
                                                IconName::CircleX
                                            },
                                            10.,
                                            if failed == 0 { c_green() } else { c_red() },
                                        ))
                                        .child(if failed == 0 {
                                            format!(
                                                "{} check{}",
                                                p.results.len(),
                                                if p.results.len() == 1 { "" } else { "s" }
                                            )
                                        } else {
                                            format!("{failed} failing")
                                        }),
                                )
                            })
                            .when(part.conflicts > 0, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_0p5()
                                        .child(icon(IconName::GitMerge, 10., c_muted()))
                                        .child(format!(
                                            "{} conflict{}",
                                            part.conflicts,
                                            if part.conflicts == 1 { "" } else { "s" }
                                        )),
                                )
                            })
                            .child(div().flex_1())
                            .when(elapsed > 0, |el| el.child(took(elapsed))),
                    )
                    .when(stuck, |el| {
                        el.child(
                            div()
                                .flex()
                                .gap_1p5()
                                .child(
                                    small(self, "mission-part-retry", "Try again", true).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.retry_part(mid, index, cx);
                                        }),
                                    ),
                                )
                                .child(small(self, "mission-part-skip", "Skip", false).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.skip_part(mid, index, cx);
                                    }),
                                )),
                        )
                    }),
            )
            .into_any_element()
    }
}

/// The colour of the Checking column.
fn c_purple_or_blue() -> u32 {
    settings::mix(c_blue(), c_red(), 0.6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn long_goals_are_cut_at_a_word() {
        assert_eq!(short("Add dark mode", 64), "Add dark mode");
        assert_eq!(
            short("Add two commands to notes.py: a delete", 30),
            "Add two commands to notes.py…"
        );
        assert_eq!(took(45), "45s");
        assert_eq!(took(252), "4m 12s");
        assert_eq!(took(3780), "1h 3m");
    }
}
