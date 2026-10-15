//! Which agent an ask bar on a canvas hands its task to, shown on the bar with a menu to
//! change it, so it's clear before sending who will do the work and how.

use crate::archdraw::view::tip;
use crate::*;

pub(crate) struct AgentPick {
    /// Index into `agent::PROVIDERS`.
    pub agent: usize,
    pub open: bool,
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
            open: false,
        }
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
            .child(icon(IconName::ChevronDown, 11., c_muted()))
            .tooltip(tip("Pick the agent that does this"))
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
        let mut list = div().p_1().w(px(300.)).v_flex().child(
            div()
                .px_2()
                .pt_1()
                .pb_2()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .whitespace_normal()
                .child(format!(
                    "The agent runs in your project folder and edits {how} in place. You review its change here before keeping it."
                )),
        );
        for i in self.choices(cx) {
            let p = &agent::PROVIDERS[i];
            list = list.child(
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    let pick = get(this);
                    pick.agent = i;
                    pick.open = false;
                    cx.notify();
                })),
            );
        }
        Some(
            menu_box(key, list)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .into_any_element(),
        )
    }
}
