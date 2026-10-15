//! Which agent an ask bar on a canvas hands its task to, shown on the bar with a menu to
//! change it, so it's clear before sending who will do the work and how.

use crate::archdraw::view::tip;
use crate::*;
use std::collections::HashMap;

pub(crate) struct AgentPick {
    /// Index into `agent::PROVIDERS`.
    pub agent: usize,
    /// The model picked for it; empty for the agent's default.
    pub model: String,
    pub open: bool,
    /// The agent whose models the menu shows, while hovering one.
    shown: Option<usize>,
}

impl AgentPick {
    /// Starts on the agent picked in Settings.
    pub(crate) fn new(cx: &App) -> Self {
        let id = cx.global::<AppSettings>().default_provider.clone();
        Self {
            agent: agent::PROVIDERS
                .iter()
                .position(|p| p.id == id)
                .unwrap_or(0),
            model: String::new(),
            open: false,
            shown: None,
        }
    }

    /// A model's name as the menu and chip show it.
    fn model_label(agent: usize, model: &str) -> Option<String> {
        if model.is_empty() {
            return None;
        }
        let caps = caps::get(agent::PROVIDERS[agent].id);
        Some(
            caps.models
                .iter()
                .find(|m| m.choice.value == model)
                .map(|m| m.choice.label.clone())
                .unwrap_or_else(|| model.to_string()),
        )
    }

    /// The model choices an agent offers, its default first.
    fn models(agent: usize) -> Vec<caps::Choice> {
        let p = &agent::PROVIDERS[agent];
        caps::controls(p.id, &HashMap::new())
            .into_iter()
            .find(|c| c.id == "model")
            .map(|c| c.choices)
            .unwrap_or_else(|| {
                vec![caps::Choice {
                    value: String::new(),
                    label: "Default".into(),
                    desc: if caps::get(p.id).detected {
                        String::new()
                    } else {
                        "Detecting models…".into()
                    },
                }]
            })
    }

    /// The agents to offer: the installed ones, and the current one even if it isn't.
    fn choices(&self, cx: &App) -> Vec<usize> {
        let settings = cx.global::<AppSettings>();
        (0..agent::PROVIDERS.len())
            .filter(|i| {
                let p = &agent::PROVIDERS[*i];
                *i == self.agent || agent::available(p.bin, &settings.agent(p.id).binary)
            })
            .collect()
    }

    /// The agent's mark and name on the bar; a click opens the menu.
    pub(crate) fn chip<T: 'static>(
        &self,
        key: &'static str,
        cx: &mut Context<T>,
        get: fn(&mut T) -> &mut AgentPick,
    ) -> AnyElement {
        let p = &agent::PROVIDERS[self.agent];
        div()
            .id(SharedString::from(format!("{key}-agent")))
            .flex_none()
            .h(px(24.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(11.5))
            .text_color(rgb(c_secondary()))
            .hover(|el| el.bg(tint(c_active())))
            .when(self.open, |el| el.bg(tint(c_active())))
            .child(div().text_color(rgb(p.color)).child(p.mark))
            .child(p.name)
            .children(
                Self::model_label(self.agent, &self.model)
                    .map(|m| div().text_color(rgb(c_muted())).child(m)),
            )
            .child(icon(IconName::ChevronDown, 11., c_muted()))
            .tooltip(tip("Pick the agent and model that do this"))
            .on_click(cx.listener(move |this, _, _, cx| {
                let pick = get(this);
                pick.open = !pick.open;
                cx.notify();
            }))
            .into_any_element()
    }

    /// The menu of agents, with a line on how the ask runs. `how` says what the agent
    /// changes, such as "the diagram's files in architecture/".
    pub(crate) fn menu<T: 'static>(
        &self,
        key: &'static str,
        how: &str,
        cx: &mut Context<T>,
        get: fn(&mut T) -> &mut AgentPick,
    ) -> Option<AnyElement> {
        if !self.open {
            return None;
        }
        let shown = self.shown.unwrap_or(self.agent);
        let mut agents = div().p_1().w(px(190.)).v_flex();
        for i in self.choices(cx) {
            let p = &agent::PROVIDERS[i];
            agents = agents.child(
                menu_row(
                    (SharedString::from(format!("{key}-agent-pick")), i),
                    p.name.to_string(),
                    String::new(),
                    i == self.agent,
                    Some(
                        div()
                            .w(px(16.))
                            .text_color(rgb(p.color))
                            .child(p.mark)
                            .into_any_element(),
                    ),
                )
                .when(i == shown, |el| el.bg(tint(c_subtle())))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    let pick = get(this);
                    if *hovered && pick.shown != Some(i) {
                        pick.shown = Some(i);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let pick = get(this);
                    if pick.agent != i {
                        pick.model.clear();
                    }
                    pick.agent = i;
                    pick.open = false;
                    pick.shown = None;
                    cx.notify();
                }))
                .child(icon(IconName::ChevronRight, 12., c_muted())),
            );
        }
        // The models column keeps the height of the longest list, so hovering an agent
        // with fewer models doesn't resize the menu under the pointer.
        let rows = |agent: usize| -> f32 {
            Self::models(agent)
                .iter()
                .map(|c| if c.desc.is_empty() { 28. } else { 44. })
                .sum::<f32>()
        };
        let tallest = self.choices(cx).into_iter().map(rows).fold(0., f32::max);
        let mut models = div()
            .id(SharedString::from(format!("{key}-models")))
            .p_1()
            .w(px(240.))
            .h(px((tallest + 34.).clamp(150., 330.)))
            .overflow_y_scroll()
            .v_flex()
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb(px(4.))
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(format!("{} models", agent::PROVIDERS[shown].name)),
            );
        for (j, choice) in Self::models(shown).into_iter().enumerate() {
            let value = choice.value.clone();
            models = models.child(
                menu_row(
                    SharedString::from(format!("{key}-model-{shown}-{j}")),
                    choice.label,
                    choice.desc,
                    shown == self.agent && choice.value == self.model,
                    None,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let pick = get(this);
                    pick.agent = shown;
                    pick.model = value.clone();
                    pick.open = false;
                    pick.shown = None;
                    cx.notify();
                })),
            );
        }
        let list = div()
            .v_flex()
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .w(px(430.))
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .whitespace_normal()
                    .child(format!(
                        "The agent runs in your project folder and edits {how} in place. You review its change here before keeping it. Each ask carries on this canvas's chat, so the agent remembers the earlier ones; picking another agent hands it that chat."
                    )),
            )
            .child(
                div()
                    .flex()
                    .child(agents)
                    .child(div().w(px(1.)).bg(rgb(c_border())))
                    .child(models),
            );
        Some(
            menu_box(key, list)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .into_any_element(),
        )
    }
}
