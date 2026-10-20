//! The Missions page: the list of missions, the New mission composer, planning, the
//! plan laid out in waves, the live board and the ready-to-ship result.

use super::*;
use crate::mission::{Mark, Mission, Part, PartState, Stage};

/// Width of the list of missions.
const LIST_WIDTH: f32 = 280.;
/// Width of the live feed beside the board.
const FEED_WIDTH: f32 = 300.;
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

/// The colour a mission is shown in: where it stands as a whole.
fn mission_tone(m: &Mission) -> u32 {
    let stuck = m.parts.iter().any(|p| p.state == PartState::Stuck);
    match m.stage {
        Stage::Ready => c_green(),
        Stage::Failed => c_red(),
        Stage::Plan => c_amber(),
        _ if stuck => c_amber(),
        Stage::Paused => c_muted(),
        _ => c_blue(),
    }
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
        let rows: Vec<AnyElement> = missions
            .iter()
            .enumerate()
            .map(|(i, m)| {
                shell::cascade(self.mission_row(m, cx), format!("mission-in-{}", m.id), i)
            })
            .collect();
        let none = missions.is_empty();
        let on_form = self.mission_open.is_none();
        let side = div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .relative()
            .v_flex()
            .bg(pane(c_sheet()))
            .child(shell::sheet_corner())
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
                    // Room for the selected row's curves at the list's ends.
                    .py(px(12.))
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
                    .border_color(rgb(c_sheet()))
                    .child(self.provider_badge(agent::provider(a), size))
            }))
    }

    fn mission_row(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let id = m.id;
        let on = self.mission_open == Some(id);
        let (done, total) = m.progress();
        let stuck = m.parts.iter().any(|p| p.state == PartState::Stuck);
        let tone = mission_tone(m);
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
        let counted = total > 0 && !matches!(m.stage, Stage::Plan | Stage::Planning);
        let fraction = if counted {
            done as f32 / total as f32
        } else {
            0.
        };
        let center: AnyElement = match m.stage {
            Stage::Ready => icon(IconName::Check, 13., tone).into_any_element(),
            Stage::Planning => spin(("mission-row-spin", id as usize), 12., tone),
            Stage::Plan | Stage::Failed => icon(IconName::ListTree, 12., tone).into_any_element(),
            Stage::Paused => icon(IconName::Pause, 11., tone).into_any_element(),
            _ if stuck => icon(IconName::TriangleAlert, 11., tone).into_any_element(),
            _ => div()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{done}/{total}"))
                .into_any_element(),
        };
        div()
            .id(("mission-row", id as usize))
            .relative()
            .mx_2()
            .p_2p5()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded_xl()
            .cursor_pointer()
            .border_1()
            .border_color(rgba(0))
            .when(on, |el| {
                el.child(shell::row_fuse(format!("mission-fuse-{id}"), 8.))
            })
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.mission_open = Some(id);
                cx.notify();
            }))
            .child(
                div()
                    .relative()
                    .size(px(36.))
                    .flex_none()
                    .child(ring(36., 3.5, fraction, tone))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(center),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .line_height(px(17.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(short(m.goal.lines().next().unwrap_or(""), 60)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(11.))
                            .when(live, |el| {
                                el.child(breathing(("mission-row-dot", id as usize), 5., tone))
                            })
                            .child(div().text_color(rgb(tone)).child(label))
                            .child(div().flex_1())
                            .child(self.agent_stack(&agents, 16.)),
                    ),
            )
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

        // Agents that may work on it: badges that light up when picked, with how often
        // each got its parts through first time on earlier missions.
        let records = crate::mission::records(&self.state.missions);
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
                    .when_some(records.get(id).and_then(|r| r.first_try()), |el, rate| {
                        el.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(c_muted()))
                                .child(format!("{rate}%")),
                        )
                    })
                    .when_some(records.get(id).and_then(|r| r.line()), |el, line| {
                        let tip: SharedString = format!("On earlier missions: {line}").into();
                        el.tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                .build(window, cx)
                        })
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
        let tone = mission_tone(m);
        let lead = m.lead;
        let live = matches!(m.stage, Stage::Planning | Stage::Running | Stage::Landing);
        let on_board = !matches!(m.stage, Stage::Planning | Stage::Plan | Stage::Failed);
        let (done, total) = m.progress();
        let small = |this: &Self, key: &'static str, label: &'static str, primary: bool| {
            this.button(key, label, primary)
                .h(px(30.))
                .px_3()
                .text_size(px(12.))
        };

        // The ring: parts merged on the board, what's happening before it.
        let center: AnyElement = match m.stage {
            Stage::Planning => spin("mission-hero-spin", 18., tone),
            Stage::Plan | Stage::Failed => icon(IconName::ListTree, 18., tone).into_any_element(),
            Stage::Ready => icon(IconName::Check, 20., tone).into_any_element(),
            _ => div()
                .v_flex()
                .items_center()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BOLD)
                        .line_height(px(16.))
                        .child(format!("{done}/{total}")),
                )
                .into_any_element(),
        };
        let fraction = if total == 0 {
            0.
        } else {
            done as f32 / total as f32
        };
        let dial = div()
            .relative()
            .size(px(60.))
            .flex_none()
            .child(ring(60., 5., if on_board { fraction } else { 0. }, tone))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(center),
            );

        let pill = div()
            .px_2()
            .h(px(22.))
            .flex()
            .items_center()
            .gap_1p5()
            .rounded_full()
            .border_1()
            .border_color(rgb(shade(tone, 0.45)))
            .bg(rgb(shade(tone, 0.14)))
            .text_color(rgb(tone))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .when(live, |el| {
                el.child(breathing("mission-stage-dot", 6., tone))
            })
            .child(
                if stuck && matches!(m.stage, Stage::Running | Stage::Paused) {
                    "Needs you"
                } else {
                    stage_label(m.stage)
                },
            );
        let meta = |glyph: IconName, text: String, mono: bool| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(mono, |el| el.font_family("monospace"))
                .child(icon(glyph, 11., c_muted()))
                .child(text)
        };

        let actions = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1p5()
            .when(m.stage == Stage::Running, |el| {
                el.child(
                    small(self, "mission-pause", "Pause", false)
                        .child(icon(IconName::Pause, 12., c_secondary()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(m) = this.mission_mut(mid) {
                                m.stage = Stage::Paused;
                            }
                            this.mission_log(mid, Mark::Pause, "Paused");
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
                            this.mission_log(mid, Mark::Start, "Resumed");
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
            .when(on_board, |el| {
                let shown = !self.mission_feed_hidden;
                el.child(
                    self.ghost_button_sized(
                        "mission-feed-toggle",
                        IconName::Activity,
                        if shown {
                            "Hide the activity"
                        } else {
                            "Show the activity"
                        },
                        30.,
                    )
                    .when(shown, |el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mission_feed_hidden = !this.mission_feed_hidden;
                        cx.notify();
                    })),
                )
            })
            .map(|el| {
                if self.mission_deleting == Some(mid) {
                    el.child(
                        self.button("mission-delete-yes", "Delete mission", false)
                            .h(px(30.))
                            .px_3()
                            .text_size(px(12.))
                            .border_color(rgb(shade(c_red(), 0.6)))
                            .bg(rgb(shade(c_red(), 0.14)))
                            .text_color(rgb(c_red()))
                            .child(icon(IconName::Trash, 12., c_red()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.mission_deleting = None;
                                this.delete_mission(mid);
                                cx.notify();
                            })),
                    )
                    .child(
                        small(self, "mission-delete-no", "Keep", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.mission_deleting = None;
                                cx.notify();
                            },
                        )),
                    )
                } else {
                    el.child(
                        self.ghost_button_sized(
                            ("mission-delete", mid as usize),
                            IconName::Trash,
                            "Delete the mission (its sessions and branches stay)",
                            30.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mission_deleting = Some(mid);
                            cx.notify();
                        })),
                    )
                }
            });

        let header = div()
            .flex_none()
            .px_5()
            .pt_5()
            .pb_4()
            .v_flex()
            .gap_4()
            .border_b_1()
            .border_color(divider())
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgba((tone << 8) | 0x16), 0.),
                linear_color_stop(rgba(tone << 8), 1.),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(dial)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_3()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_secondary()))
                                    .child(pill)
                                    .child(meta(IconName::Folder, project, false))
                                    .child(meta(
                                        IconName::GitBranch,
                                        format!("{} ← {}", m.branch, m.base),
                                        true,
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .line_height(px(24.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_clamp(2)
                                    .text_ellipsis()
                                    .child(short(m.goal.trim(), 170)),
                            ),
                    )
                    .child(actions),
            )
            .when(!m.summary.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(rgb(c_muted()))
                        .line_clamp(2)
                        .child(m.summary.clone()),
                )
            })
            .when(on_board, |el| el.child(self.mission_metrics(m)));
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

    /// The mission's numbers in one row: parts, agents, time and cost.
    fn mission_metrics(&self, m: &Mission) -> Div {
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
        let agents: Vec<String> = m.parts.iter().map(|p| p.agent.clone()).collect();
        let metric = |glyph: IconName, value: String, label: &'static str, color: u32| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_lg()
                        .bg(tint(c_subtle()))
                        .child(icon(glyph, 13., color)),
                )
                .child(
                    div()
                        .v_flex()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(c_text()))
                                .child(value),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(c_muted()))
                                .child(label),
                        ),
                )
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_6()
            .gap_y_3()
            .child(metric(
                IconName::GitMerge,
                format!("{done} of {total}"),
                "parts merged",
                if done == total && total > 0 {
                    c_green()
                } else {
                    c_secondary()
                },
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.agent_stack(&agents, 28.))
                    .child(
                        div()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(working.to_string()),
                            )
                            .child(div().text_size(px(10.5)).text_color(rgb(c_muted())).child(
                                if working == 1 {
                                    "agent working"
                                } else {
                                    "agents working"
                                },
                            )),
                    ),
            )
            .child(metric(
                IconName::Timer,
                took(elapsed),
                if ready {
                    "start to ready"
                } else {
                    "running for"
                },
                c_secondary(),
            ))
            .child(metric(
                IconName::Coins,
                if cents > 0. {
                    format!("${:.2}", cents / 100.)
                } else {
                    "—".into()
                },
                "agent cost",
                c_secondary(),
            ))
            .when(conflicts > 0, |el| {
                el.child(metric(
                    IconName::GitCompare,
                    conflicts.to_string(),
                    if conflicts == 1 {
                        "conflict resolved"
                    } else {
                        "conflicts resolved"
                    },
                    c_secondary(),
                ))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .child(self.part_segments(m, 6.)),
            )
    }

    /// While the planner reads: what it's doing, for how long, and the plan taking shape.
    fn planning_view(&self, m: &Mission) -> AnyElement {
        let planner = agent::provider(m.agents.first().map(String::as_str).unwrap_or("claude"));
        let secs = since(if m.planned > 0 { m.planned } else { m.created }, 0);
        let ghost = |i: usize, w: f32| {
            div()
                .flex_1()
                .h(px(132.))
                .p_3()
                .v_flex()
                .gap_2p5()
                .rounded_xl()
                .bg(rgb(c_raised()))
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
                    .max_w(px(800.))
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
                            .w(px(760.))
                            .flex()
                            .gap_3()
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
        let mut wave_views: Vec<AnyElement> = Vec::new();
        for (w, parts) in waves.iter().enumerate() {
            if w > 0 {
                wave_views.push(wave_link(w, m, &waves));
            }
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
                    .map(|i| short(&m.parts[*i].title, 28))
                    .collect()
            };
            let note = if w == 0 {
                "Starts right away".to_string()
            } else {
                format!("After {}", after.join(", "))
            };
            wave_views.push(wave_column(
                waves.len() == 1,
                wave_head(w, parts.len(), note, c_blue(), false),
                parts
                    .iter()
                    .map(|i| self.mission_plan_card(m, *i, &m.parts[*i], cx))
                    .collect(),
            ));
        }
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
                    .child(
                        div()
                            .id("mission-plan-waves")
                            .w_full()
                            .overflow_x_scroll()
                            .child(div().flex().items_start().gap_1().children(wave_views)),
                    ),
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
                                this.replan(mid, window, cx);
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
        let record = crate::mission::records(&self.state.missions)
            .get(&part.agent)
            .and_then(|r| r.first_try());
        div()
            .w_full()
            .flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()))
            .overflow_hidden()
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
                            .when_some(record, |el, rate| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_size(px(11.))
                                        .text_color(rgb(c_muted()))
                                        .child(icon(IconName::ShieldCheck, 11., c_muted()))
                                        .child(format!("{rate}% first time")),
                                )
                            })
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

    /// The running mission: the parts flowing wave by wave on the left, and what is
    /// happening right now on the right.
    fn mission_board(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let banner = self.mission_banner(m, cx);
        let waves = m.waves();
        let count = waves.len();
        let mut columns: Vec<AnyElement> = Vec::new();
        for (w, parts) in waves.iter().enumerate() {
            if w > 0 {
                columns.push(wave_link(w, m, &waves));
            }
            let settled = parts.iter().all(|i| m.parts[*i].state.settled());
            let busy = parts.iter().any(|i| m.parts[*i].state.busy());
            let (status, tone) = if settled {
                ("Done", c_green())
            } else if busy {
                ("Running", c_blue())
            } else if parts.iter().any(|i| m.parts[*i].state == PartState::Stuck) {
                ("Needs you", c_amber())
            } else {
                ("Up next", c_muted())
            };
            columns.push(wave_column(
                waves.len() == 1,
                wave_head(w, parts.len(), status, tone, busy),
                parts
                    .iter()
                    .map(|i| self.part_card(m, *i, &m.parts[*i], cx))
                    .collect(),
            ));
        }
        let flow = div()
            .id("mission-flow")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .v_flex()
            .gap_4()
            .children(banner)
            .child(
                div()
                    .id("mission-waves")
                    .w_full()
                    .overflow_x_scroll()
                    .child(div().flex().items_start().gap_1().children(columns)),
            )
            .when(count == 0, |el| {
                el.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("This mission has no parts."),
                )
            });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(flow)
            .when(!self.mission_feed_hidden, |el| {
                el.child(self.mission_feed(m))
            })
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
                .border_color(rgb(shade(tone, 0.45)))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(shade(tone, 0.2)), 0.),
                    linear_color_stop(rgb(shade(tone, 0.03)), 1.),
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
                .size(px(42.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_xl()
                .border_1()
                .border_color(rgb(shade(tone, 0.5)))
                .bg(rgb(shade(tone, 0.24)))
                .child(icon(glyph, 20., tone))
        };
        match m.stage {
            Stage::Ready => Some(
                card(c_green())
                    .child(badge(IconName::PartyPopper, c_green()))
                    .child(words("Ready to ship".into(), m.note.clone()))
                    .child(
                        self.button("mission-ship", "Review and ship", true)
                            .h(px(36.))
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
                            .size(px(42.))
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

    /// What a part's agent did last: its latest steps, oldest first, or its latest words.
    fn part_activity(&self, session: u64, lines: usize) -> Vec<String> {
        let Some(s) = self.state.session(session) else {
            return Vec::new();
        };
        let Some(last) = s.messages.iter().rev().find(|m| m.role == Role::Agent) else {
            return Vec::new();
        };
        let mut out: Vec<String> = last
            .steps
            .iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if let Some(words) = last
            .text
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
        {
            out.push(words.to_string());
        }
        let skip = out.len().saturating_sub(lines);
        out.split_off(skip)
    }

    /// The live line for a part that isn't writing code: what Runo is doing with it.
    fn part_phase(&self, m: &Mission, part: &Part) -> Option<String> {
        match part.state {
            PartState::Checking => Some("Running the project's checks".into()),
            PartState::Reviewing => {
                let other = m
                    .agents
                    .iter()
                    .find(|a| **a != part.agent)
                    .map(|a| agent::provider(a).name)
                    .unwrap_or("A second agent");
                Some(format!("{other} is reviewing the changes"))
            }
            PartState::Proved | PartState::Merging => Some(format!("Merging into {}", m.branch)),
            _ => None,
        }
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
        let done = state == PartState::Done;
        let busy = state.busy();
        let tone = part_tone(state);
        let asking = session != 0 && self.approval_for(session).is_some();
        let deps: Vec<String> = m
            .parts
            .iter()
            .filter(|d| part.deps.contains(&d.key) && !d.state.settled())
            .map(|d| d.title.clone())
            .collect();
        let mut console: Vec<String> = if state == PartState::Working && session != 0 {
            self.part_activity(session, 3)
        } else {
            Vec::new()
        };
        if let Some(line) = self.part_phase(m, part) {
            console.push(line);
        }
        if state == PartState::Working && console.is_empty() {
            console.push(format!("{} is writing code", provider.name));
        }
        let checks = self
            .state
            .session(session)
            .and_then(|s| s.proof.as_ref())
            .filter(|p| !p.results.is_empty());
        let elapsed = since(part.started, part.finished);
        let step = state.step();
        let accent = if stuck || asking {
            c_amber()
        } else if busy {
            c_blue()
        } else if done {
            c_green()
        } else {
            c_border()
        };

        // Work → Checks → Review → Merge as a segmented bar, the current step breathing.
        let steps = (!done && (step.is_some() || stuck)).then(|| {
            let at = step.unwrap_or(0);
            let review = m.review;
            div().flex().gap_1().children(
                STEPS
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| review || *i != 2)
                    .map(|(i, label)| {
                        let past = at > i;
                        let now = at == i;
                        let color = if past {
                            c_green()
                        } else if now && stuck {
                            c_amber()
                        } else if now {
                            c_blue()
                        } else {
                            c_border()
                        };
                        let bar = div().h(px(3.)).w_full().rounded_full().bg(rgb(color));
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .gap_1()
                            .child(if now && !stuck {
                                bar.animated(
                                    ("mission-step", index * 8 + i),
                                    Animation::new(Duration::from_millis(1400)).repeat(),
                                    |el, d| el.opacity(0.35 + 0.65 * crate::askbar::pulse(d)),
                                )
                            } else {
                                bar.into_any_element()
                            })
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(if past || now {
                                        c_secondary()
                                    } else {
                                        c_muted()
                                    }))
                                    .child(*label),
                            )
                    }),
            )
        });

        let chip = div()
            .flex_none()
            .h(px(20.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded_full()
            .bg(rgb(shade(tone, 0.16)))
            .text_size(px(10.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(tone))
            .when(busy, |el| {
                el.child(breathing(("mission-part-dot", index), 5., tone))
            })
            .when(done, |el| el.child(icon(IconName::Check, 10., tone)))
            .child(state.label());

        let small = |this: &Self, key: &'static str, label: &'static str, primary: bool| {
            this.button((key, index), label, primary)
                .h(px(26.))
                .px_2p5()
                .text_size(px(11.5))
        };
        let footer = div()
            .flex()
            .items_center()
            .gap_2p5()
            .text_size(px(11.))
            .text_color(rgb(c_muted()))
            .when_some(checks, |el, p| {
                let failed = p.failed().len();
                let color = if failed == 0 { c_green() } else { c_red() };
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_color(rgb(color))
                        .child(icon(
                            if failed == 0 {
                                IconName::ShieldCheck
                            } else {
                                IconName::CircleX
                            },
                            11.,
                            color,
                        ))
                        .child(if failed == 0 {
                            format!(
                                "{} check{} passed",
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
                        .gap_1()
                        .child(icon(IconName::GitMerge, 11., c_muted()))
                        .child(format!(
                            "{} conflict{}",
                            part.conflicts,
                            if part.conflicts == 1 { "" } else { "s" }
                        )),
                )
            })
            .when(!done && !part.files.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(icon(IconName::FileCode, 11., c_muted()))
                        .child(format!(
                            "{} file{}",
                            part.files.len(),
                            if part.files.len() == 1 { "" } else { "s" }
                        )),
                )
            })
            .child(div().flex_1())
            .when(elapsed > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(icon(IconName::Timer, 11., c_muted()))
                        .child(took(elapsed)),
                )
            });

        let console_box = (!console.is_empty()).then(|| {
            let last = console.len() - 1;
            div()
                .px_2p5()
                .py_2()
                .v_flex()
                .gap_1()
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_border()))
                .bg(rgb(settings::mix(0x000000, c_canvas(), 0.2)))
                .font_family("monospace")
                .text_size(px(10.5))
                .children(console.into_iter().enumerate().map(|(i, line)| {
                    let now = i == last;
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_color(rgb(if now { c_text() } else { c_muted() }))
                        .child(if now {
                            spin(("mission-part-spin", index), 10., c_blue())
                        } else {
                            div()
                                .w(px(10.))
                                .flex_none()
                                .text_color(rgb(c_muted()))
                                .child("›")
                                .into_any_element()
                        })
                        .child(div().flex_1().min_w_0().truncate().child(line))
                }))
        });

        div()
            .id(("mission-part", index))
            .w_full()
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(rgb(if busy || stuck || asking {
                shade(accent, 0.55)
            } else {
                c_border()
            }))
            .bg(if busy {
                linear_gradient(
                    180.,
                    linear_color_stop(rgb(shade(c_blue(), 0.1)), 0.),
                    linear_color_stop(rgb(c_raised()), 0.6),
                )
            } else {
                rgb(c_raised()).into()
            })
            .overflow_hidden()
            .when(state == PartState::Waiting, |el| el.opacity(0.85))
            .when(state == PartState::Skipped, |el| el.opacity(0.5))
            .when(session != 0, |el| {
                el.cursor_pointer()
                    .hover(|el| el.border_color(rgb(shade(c_blue(), 0.7))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(session);
                        this.view = View::Session;
                        cx.notify();
                    }))
            })
            .when(done, |el| {
                el.child(div().h(px(2.)).w_full().bg(rgb(shade(c_green(), 0.7))))
            })
            .child(
                div()
                    .p_3()
                    .v_flex()
                    .gap_2p5()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2p5()
                            .child(self.provider_badge(provider, 28.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .v_flex()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .line_clamp(2)
                                            .child(part.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(c_muted()))
                                            .child(provider.name),
                                    ),
                            )
                            .child(chip),
                    )
                    .children(steps)
                    .children(console_box)
                    .when(asking, |el| {
                        el.child(
                            div()
                                .px_2p5()
                                .py_1p5()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .rounded_lg()
                                .bg(rgb(shade(c_amber(), 0.12)))
                                .text_size(px(11.5))
                                .text_color(rgb(c_amber()))
                                .child(icon(IconName::ShieldAlert, 12., c_amber()))
                                .child("Asks to run a command. Open it to answer."),
                        )
                    })
                    .when(state == PartState::Waiting && !deps.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(icon(IconName::Hourglass, 11., c_muted()))
                                .child(format!("Starts after {}", deps.join(", "))),
                        )
                    })
                    .when(state == PartState::Waiting && deps.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(icon(IconName::Hourglass, 11., c_muted()))
                                .child(if m.stage == Stage::Paused {
                                    "Starts when you resume"
                                } else {
                                    "Starts when an agent is free"
                                }),
                        )
                    })
                    .when(stuck && !part.note.is_empty(), |el| {
                        el.child(
                            div()
                                .p_2p5()
                                .rounded_lg()
                                .bg(rgb(shade(c_amber(), 0.12)))
                                .text_size(px(11.5))
                                .text_color(rgb(c_text()))
                                .child(part.note.clone()),
                        )
                    })
                    .child(footer)
                    .when(stuck, |el| {
                        el.child(
                            div()
                                .flex()
                                .gap_1p5()
                                .child(
                                    small(self, "mission-part-retry", "Try again", true)
                                        .child(icon(IconName::RotateCw, 11., c_canvas()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.retry_part(mid, index, cx);
                                        })),
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

    /// Everything that happened on the mission, newest first, with what agents are doing
    /// this moment on top.
    fn mission_feed(&self, m: &Mission) -> AnyElement {
        let now = crate::workspace::unix_now();
        let live = matches!(m.stage, Stage::Running | Stage::Landing);
        // (when, icon, tone, text)
        let mut events: Vec<(u64, IconName, u32, String)> = m
            .log
            .iter()
            .map(|e| {
                let (glyph, tone) = mark_look(e.mark);
                (e.at, glyph, tone, e.text.clone())
            })
            .collect();
        // Missions from before the log: what their parts' times tell.
        if m.log.is_empty() && m.started > 0 {
            events.push((
                m.started,
                IconName::Rocket,
                c_blue(),
                format!(
                    "Mission started with {} part{}",
                    m.parts.len(),
                    if m.parts.len() == 1 { "" } else { "s" }
                ),
            ));
        }
        for p in m.parts.iter().filter(|_| m.log.is_empty()) {
            let name = agent::provider(&p.agent).name;
            if p.started > 0 {
                events.push((
                    p.started,
                    IconName::Play,
                    c_blue(),
                    format!("{name} started {}", p.title),
                ));
            }
            if p.finished > 0 && p.state == PartState::Done {
                events.push((
                    p.finished,
                    IconName::GitMerge,
                    c_green(),
                    format!("{} merged", p.title),
                ));
            }
            if p.state == PartState::Skipped && p.finished > 0 {
                events.push((
                    p.finished,
                    IconName::SkipForward,
                    c_muted(),
                    format!("{} skipped", p.title),
                ));
            }
        }
        if m.log.is_empty() && m.finished > 0 && m.stage == Stage::Ready {
            events.push((
                m.finished,
                IconName::PartyPopper,
                c_green(),
                "Ready to ship".into(),
            ));
        }
        // Newest first; lines logged in the same second keep their order.
        events.reverse();
        events.sort_by_key(|e| std::cmp::Reverse(e.0));

        let mut now_rows: Vec<AnyElement> = Vec::new();
        for (i, p) in m.parts.iter().enumerate() {
            let provider = agent::provider(&p.agent);
            let line = if p.state == PartState::Stuck {
                Some((c_amber(), "Needs you".to_string()))
            } else if p.state.busy() {
                let last = if p.state == PartState::Working {
                    self.part_activity(p.session, 1).pop()
                } else {
                    self.part_phase(m, p)
                };
                Some((
                    c_blue(),
                    last.unwrap_or_else(|| format!("{} is writing code", provider.name)),
                ))
            } else {
                None
            };
            let Some((tone, line)) = line else {
                continue;
            };
            now_rows.push(
                div()
                    .p_2p5()
                    .v_flex()
                    .gap_1p5()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(shade(tone, 0.4)))
                    .bg(rgb(shade(tone, 0.07)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(self.provider_badge(provider, 16.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(p.title.clone()),
                            )
                            .child(if tone == c_amber() {
                                icon(IconName::TriangleAlert, 11., tone).into_any_element()
                            } else {
                                spin(("mission-feed-spin", i), 11., tone)
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_family("monospace")
                            .text_size(px(10.5))
                            .text_color(rgb(c_secondary()))
                            .child(line),
                    )
                    .into_any_element(),
            );
        }

        let total = events.len();
        let timeline = events
            .into_iter()
            .enumerate()
            .map(|(i, (at, glyph, tone, text))| {
                div()
                    .flex()
                    .gap_2p5()
                    .child(
                        div()
                            .w(px(22.))
                            .flex_none()
                            .v_flex()
                            .items_center()
                            .child(
                                div()
                                    .size(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(rgb(shade(tone, 0.18)))
                                    .child(icon(glyph, 11., tone)),
                            )
                            .when(i + 1 < total, |el| {
                                el.child(div().flex_1().w(px(1.)).bg(rgb(c_border())))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pb_3()
                            .v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .pt(px(3.))
                                    .text_size(px(12.))
                                    .text_color(rgb(c_text()))
                                    .child(text),
                            )
                            .child(div().text_size(px(10.5)).text_color(rgb(c_muted())).child(
                                match now.saturating_sub(at) {
                                    0..=4 => "just now".to_string(),
                                    s => format!("{} ago", took(s)),
                                },
                            )),
                    )
            });

        div()
            .id("mission-feed")
            .w(px(FEED_WIDTH))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .px_4()
            .py_4()
            .v_flex()
            .gap_4()
            .when(!now_rows.is_empty(), |el| {
                el.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(breathing("mission-feed-live", 6., c_red()))
                                .child(small_caps("LIVE")),
                        )
                        .children(now_rows),
                )
            })
            .children(self.mission_team(m))
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(small_caps(if live { "ACTIVITY" } else { "HISTORY" }))
                    .when(total == 0, |el| {
                        el.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_muted()))
                                .child("What the agents do shows here."),
                        )
                    })
                    .child(div().v_flex().children(timeline)),
            )
            .into_any_element()
    }
}

impl RunoApp {
    /// Each agent on the mission: its parts, how many went back for fixes, and its cost.
    fn mission_team(&self, m: &Mission) -> Option<AnyElement> {
        let mut agents: Vec<&str> = Vec::new();
        for p in &m.parts {
            if !agents.contains(&p.agent.as_str()) {
                agents.push(&p.agent);
            }
        }
        if agents.is_empty() || m.started == 0 {
            return None;
        }
        let rows = agents.into_iter().map(|a| {
            let provider = agent::provider(a);
            let parts: Vec<&Part> = m.parts.iter().filter(|p| p.agent == a).collect();
            let done = parts.iter().filter(|p| p.state == PartState::Done).count();
            let fixes: u32 = parts.iter().map(|p| p.fixes).sum();
            let secs: u64 = parts.iter().map(|p| since(p.started, p.finished)).sum();
            let cents: f64 = parts
                .iter()
                .filter_map(|p| self.state.session(p.session))
                .map(crate::health::session_cost)
                .sum();
            let mut facts = vec![format!("{done} of {} merged", parts.len())];
            if fixes > 0 {
                facts.push(format!(
                    "{fixes} fix round{}",
                    if fixes == 1 { "" } else { "s" }
                ));
            }
            if secs > 0 {
                facts.push(took(secs));
            }
            if cents > 0. {
                facts.push(format!("${:.2}", cents / 100.));
            }
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(self.provider_badge(provider, 22.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .v_flex()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(provider.name),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(10.5))
                                .text_color(rgb(c_muted()))
                                .child(facts.join(" · ")),
                        ),
                )
        });
        Some(
            div()
                .v_flex()
                .gap_2()
                .child(small_caps("TEAM"))
                .children(rows)
                .into_any_element(),
        )
    }
}

/// The icon and colour of a line in the mission's log.
fn mark_look(mark: Mark) -> (IconName, u32) {
    match mark {
        Mark::Info => (IconName::Activity, c_muted()),
        Mark::Start => (IconName::Rocket, c_blue()),
        Mark::Part => (IconName::Play, c_blue()),
        Mark::Passed => (IconName::ShieldCheck, c_green()),
        Mark::Fix => (IconName::RotateCw, c_amber()),
        Mark::Review => (IconName::MessageSquare, c_blue()),
        Mark::Conflict => (IconName::GitCompare, c_amber()),
        Mark::Merge => (IconName::GitMerge, c_green()),
        Mark::Stuck => (IconName::TriangleAlert, c_amber()),
        Mark::Skip => (IconName::SkipForward, c_muted()),
        Mark::Pause => (IconName::Pause, c_muted()),
        Mark::Land => (IconName::GitBranch, c_blue()),
        Mark::Ready => (IconName::PartyPopper, c_green()),
    }
}

/// A wave's title over its column: its number, how many parts and where it stands.
fn wave_head(
    w: usize,
    parts: usize,
    status: impl Into<SharedString>,
    tone: u32,
    busy: bool,
) -> Div {
    div()
        .h(px(28.))
        .px_1()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(rgb(shade(tone, 0.55)))
                .bg(rgb(shade(tone, 0.16)))
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(tone))
                .child((w + 1).to_string()),
        )
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Wave {}", w + 1)),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child(format!("{parts} part{}", if parts == 1 { "" } else { "s" })),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(11.))
                .text_color(rgb(tone))
                .when(busy, |el| {
                    el.child(breathing(("mission-wave-dot", w), 5., tone))
                })
                .child(div().max_w(px(180.)).truncate().child(status.into())),
        )
}

/// One wave's column. A mission of one wave lays its parts side by side instead.
fn wave_column(single: bool, head: Div, cards: Vec<AnyElement>) -> AnyElement {
    if single {
        return div()
            .flex_1()
            .v_flex()
            .gap_2p5()
            .child(head.max_w(px(380.)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_2p5()
                    .children(cards.into_iter().map(|c| div().w(px(380.)).child(c))),
            )
            .into_any_element();
    }
    div()
        .flex_1()
        .min_w(px(290.))
        .max_w(px(440.))
        .v_flex()
        .gap_2p5()
        .child(head)
        .children(cards)
        .into_any_element()
}

/// The arrow between two waves, lit once the wave before it is done.
fn wave_link(w: usize, m: &Mission, waves: &[Vec<usize>]) -> AnyElement {
    let lit = waves[w - 1].iter().all(|i| m.parts[*i].state.settled());
    let color = if lit { c_green() } else { c_border() };
    div()
        .w(px(28.))
        .flex_none()
        .pt(px(64.))
        .flex()
        .items_center()
        .child(div().flex_1().h(px(1.5)).bg(rgb(color)))
        .child(icon(IconName::ChevronRight, 13., color))
        .into_any_element()
}

/// A progress ring: a track and an arc for `fraction` of it, from twelve o'clock.
fn ring(size: f32, stroke: f32, fraction: f32, color: u32) -> impl IntoElement {
    let track = c_border();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let r = (size - stroke) / 2.;
            let cx = bounds.origin.x + px(size / 2.);
            let cy = bounds.origin.y + px(size / 2.);
            let at = move |a: f32| point(cx + px(r * a.cos()), cy + px(r * a.sin()));
            let mut arc = |from: f32, to: f32, color: u32| {
                let mut b = PathBuilder::stroke(px(stroke));
                b.move_to(at(from));
                let mut a = from;
                while a < to - 1e-4 {
                    let next = (a + std::f32::consts::PI * 0.9).min(to);
                    b.arc_to(point(px(r), px(r)), px(0.), false, true, at(next));
                    a = next;
                }
                if let Ok(p) = b.build() {
                    window.paint_path(p, rgb(color));
                }
            };
            let top = -std::f32::consts::FRAC_PI_2;
            arc(top, top + std::f32::consts::TAU, track);
            let f = fraction.clamp(0., 1.);
            if f > 0. {
                arc(top, top + std::f32::consts::TAU * f, color);
            }
        },
    )
    .size(px(size))
    .flex_none()
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
