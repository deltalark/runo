//! Each agent's plan usage in the status bar, and the Usage popover it opens.

use super::*;
use crate::usage::{self, Reading, Usage, Window};

/// Colour for a share used: amber from 75%, red from 90%.
fn level_color(used: f32) -> u32 {
    if used >= 90. {
        c_red()
    } else if used >= 75. {
        c_amber()
    } else {
        c_secondary()
    }
}

/// A thin meter filled to `used` percent.
fn meter(used: f32) -> Div {
    div()
        .flex_1()
        .h(px(4.))
        .rounded_full()
        .bg(tint(c_active()))
        .child(
            div()
                .h_full()
                .rounded_full()
                .w(relative((used / 100.).clamp(0., 1.)))
                .bg(rgb(level_color(used))),
        )
}

impl RunoApp {
    /// Installed agents, in Runo's order.
    fn usage_agents(&self) -> Vec<&'static agent::Provider> {
        agent::PROVIDERS
            .iter()
            .filter(|p| caps::get(p.id).installed)
            .collect()
    }

    /// The status bar's usage chips: one per agent with limits to show or a sign-in to
    /// fix. Clicking any of them opens the Usage popover.
    pub(crate) fn usage_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let now = usage::now_secs();
        let chips: Vec<AnyElement> = self
            .usage_agents()
            .into_iter()
            .filter_map(|p| Some((p, usage::get(p.id)?)))
            .filter_map(|(p, u)| {
                let chip = div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(self.provider_mark(p, 11.));
                match &u.reading {
                    Reading::Limits(_) => {
                        let w = u.headline()?;
                        let reset = w.resets_at.map(|r| usage::left(r, now));
                        Some(
                            chip.child(
                                div()
                                    .text_color(rgb(level_color(w.used)))
                                    .child(format!("{:.0}% used", w.used)),
                            )
                            .when_some(reset, |el, r| el.child(r))
                            .into_any_element(),
                        )
                    }
                    Reading::Problem { short, .. } => Some(
                        chip.child(icon(IconName::TriangleAlert, 11., c_amber()))
                            .child(short.clone())
                            .into_any_element(),
                    ),
                    Reading::Note(_) => None,
                }
            })
            .collect();
        if chips.is_empty() {
            return None;
        }
        let open = self.menu.as_deref() == Some("usage");
        Some(
            div()
                .id("usage-status")
                .relative()
                .h(px(20.))
                .px_1()
                .flex()
                .items_center()
                .gap_3()
                .rounded_sm()
                .cursor_pointer()
                .hover(|el| el.bg(tint(c_subtle())))
                .when(open, |el| el.bg(tint(c_active())))
                .when(!open, |el| {
                    el.tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new("Plan usage for each agent")
                            .build(window, cx)
                    })
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.menu.as_deref() == Some("usage") {
                        this.menu = None;
                    } else {
                        this.menu = Some("usage".into());
                        usage::refresh(false);
                    }
                    cx.notify();
                }))
                .children(chips)
                .when(open, |el| el.child(self.usage_menu(cx)))
                .into_any_element(),
        )
    }

    /// The popover over the status bar: every installed agent's usage, compact or with
    /// each limit window spelled out.
    fn usage_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let detailed = cx.global::<AppSettings>().usage_detailed;
        let now = usage::now_secs();
        let busy = usage::busy();
        let header = div()
            .px_3()
            .pt_3()
            .pb_2()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child("Usage"),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(if busy { "Checking…" } else { "all agents" }),
            )
            .child(
                self.ghost_button_sized("usage-refresh", IconName::RefreshCw, "Check again", 22.)
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.stop_propagation();
                        usage::refresh(true);
                        cx.notify();
                    })),
            );
        let toggle = div().px_3().pb_2().child(
            github::segmented()
                .w_full()
                .child(
                    github::segment(("usage-view", 0), "Detailed", detailed)
                        .flex_1()
                        .justify_center()
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                            settings::update(cx, |s| s.usage_detailed = true);
                        })),
                )
                .child(
                    github::segment(("usage-view", 1), "Compact", !detailed)
                        .flex_1()
                        .justify_center()
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                            settings::update(cx, |s| s.usage_detailed = false);
                        })),
                ),
        );
        let line = || div().mx_2().my_1().h(px(1.)).bg(divider());
        let mut rows = div().v_flex().px_1();
        let agents = self.usage_agents();
        if agents.is_empty() {
            rows = rows.child(
                div()
                    .px_2()
                    .py_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child("No agents are installed yet."),
            );
        }
        for (i, p) in agents.into_iter().enumerate() {
            rows = rows.child(self.usage_row(i, p, usage::get(p.id), detailed, now, cx));
        }
        let manage = div()
            .id("usage-manage")
            .mx_1()
            .mb_1()
            .h(px(32.))
            .px_2()
            .flex()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(rgb(c_text()))
            .hover(|el| el.bg(tint(c_active())))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.menu = None;
                this.open_settings();
                this.settings_page = SettingsPage::Providers;
                cx.notify();
            }))
            .child(div().flex_1().child("Manage agents and sign-ins…"))
            .child(icon(IconName::ChevronRight, 12., c_muted()));
        let content = div()
            .id("usage-menu")
            .w(px(340.))
            .max_h(px(520.))
            .overflow_y_scroll()
            .rounded_lg()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_raised()))
            .shadow_lg()
            .occlude()
            .cursor_default()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(header)
            .child(toggle)
            .child(line())
            .child(rows)
            .child(line())
            .child(manage);
        div()
            .absolute()
            .top_0()
            .right_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomRight)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .pb(px(6.))
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    if this.menu.as_deref() == Some("usage") {
                                        this.menu = None;
                                        cx.notify();
                                    }
                                }))
                                .child(content)
                                .animated(
                                    "usage-menu-in",
                                    Animation::new(Duration::from_millis(180))
                                        .with_easing(ease_out_quint()),
                                    |el, d| el.opacity(d).top(px(6. * (1. - d))),
                                ),
                        ),
                )
                .with_priority(2),
            )
            .into_any_element()
    }

    /// One agent in the popover. Clicking it opens the agent's settings.
    fn usage_row(
        &self,
        index: usize,
        p: &'static agent::Provider,
        u: Option<Usage>,
        detailed: bool,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = p.id;
        let muted = |text: String| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child(text)
        };
        let mut head = div()
            .h(px(32.))
            .flex()
            .items_center()
            .gap_2()
            .child(self.provider_mark(p, 13.))
            .child(
                div()
                    .flex_none()
                    .text_size(px(13.))
                    .text_color(rgb(c_text()))
                    .child(p.name),
            );
        let mut body = None;
        match u.as_ref().map(|u| &u.reading) {
            None => head = head.child(muted("Checking…".into())),
            Some(Reading::Note(text)) => head = head.child(muted(text.clone())),
            Some(Reading::Problem { short, long }) => {
                head = head
                    .child(icon(IconName::TriangleAlert, 11., c_amber()))
                    .child(muted(short.clone()));
                if detailed {
                    body = Some(
                        div()
                            .pl(px(23.))
                            .pb_2()
                            .text_size(px(11.5))
                            .text_color(rgb(c_secondary()))
                            .child(long.clone()),
                    );
                }
            }
            Some(Reading::Limits(windows)) => {
                let u = u.as_ref().expect("a reading");
                head = head.child(div().flex_1());
                if detailed {
                    let mut list = div().pl(px(23.)).pb_2().v_flex().gap(px(6.));
                    for w in windows {
                        list = list.child(self.usage_window(w, now));
                    }
                    if p.id == "codex" {
                        list = list.child(muted(format!(
                            "As of Codex's last turn, {}",
                            usage::ago(u.at, now)
                        )));
                    }
                    // Runo runs Claude Code in print mode, which Anthropic meters as SDK
                    // use; people measure it taking more of the window for the same work.
                    if p.id == "claude" {
                        list = list.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child("Chat runs Claude Code in print mode, which can use more of these windows than the same work typed into claude in a terminal. For long hands-on work, the CLI view runs claude itself on the same conversation."),
                        );
                    }
                    body = Some(list);
                } else if let Some(w) = u.headline() {
                    head = head
                        .when_some(w.resets_at, |el, r| {
                            el.child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(usage::left(r, now)),
                            )
                        })
                        .child(
                            div()
                                .w(px(36.))
                                .flex()
                                .justify_end()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(level_color(w.used)))
                                .child(format!("{:.0}%", w.used)),
                        );
                }
            }
        }
        head = head.child(icon(IconName::ChevronRight, 12., c_muted()));
        // Agents that report a cost: what they put on today. On a plan it's what the
        // tokens would cost at API prices, not what the person is billed.
        let on_plan = matches!(u.as_ref().map(|u| &u.reading), Some(Reading::Limits(_)));
        let spend = agent::reports_cost(id).then(|| {
            let cents = self.state.agent_spend_on(&store::today(), id);
            div()
                .pl(px(23.))
                .pb_2()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child(div().flex_1().min_w_0().truncate().child(if on_plan {
                    "Est. today at API prices, not billed on your plan"
                } else {
                    "Est. today, as the agent reports it"
                }))
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(c_secondary()))
                        .child(format!("${:.2}", cents / 100.)),
                )
        });
        div()
            .id(("usage-row", index))
            .px_2()
            .v_flex()
            .rounded_md()
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_active())))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.menu = None;
                this.open_settings();
                this.settings_page = SettingsPage::Agent(id);
                cx.notify();
            }))
            .child(head)
            .children(body)
            .children(spend)
            .into_any_element()
    }

    /// One limit window in the detailed view: its name, a meter and when it resets.
    fn usage_window(&self, w: &Window, now: i64) -> Div {
        let reset = match w.resets_at {
            Some(r) => format!("Resets in {}", usage::left(r, now)),
            None => "Reset time unknown".into(),
        };
        div()
            .v_flex()
            .gap(px(3.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.5))
                    .child(
                        div()
                            .w(px(92.))
                            .flex_none()
                            .text_color(rgb(c_secondary()))
                            .child(w.label),
                    )
                    .child(meter(w.used))
                    .child(
                        div()
                            .w(px(34.))
                            .flex()
                            .justify_end()
                            .text_color(rgb(level_color(w.used)))
                            .child(format!("{:.0}%", w.used)),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(reset),
            )
    }
}
