//! The "In code" section for a selected instance: which component in the app's code its
//! component is, for the stack the screen is built with. Code written from the design then
//! uses that component instead of writing it out again.

use super::figma::{caption, sec};
use super::*;
use crate::designdraw::codemap::{CodeMap, Target};
use crate::designdraw::spec;
use gpui_kit::component::Sizable;

impl DesignCanvas {
    pub(super) fn make_code_link_fields(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        let make = |hint: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(hint))
        };
        let name = make("Component, like Button", window, cx);
        let import = make("Import, like @/components/ui/button", window, cx);
        for f in [&name, &import] {
            subs.push(cx.subscribe_in(f, window, |this, _, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.save_code_link(cx);
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        window.focus(&this.focus, cx);
                    }
                }
            }));
        }
        (name, import)
    }

    /// What the selected screen is built for.
    fn code_target(&self) -> Target {
        self.screen
            .as_ref()
            .and_then(|id| self.canvas.screen(id))
            .map_or(Target::Mobile, |s| Target::of(s.device))
    }

    /// The selected instance's component, and the stack it's mapped for.
    fn code_link_key(&self) -> Option<(String, Target)> {
        let el = self.element.as_ref().filter(|_| self.also.is_empty())?;
        Some((el.component.clone()?, self.code_target()))
    }

    /// Fill the boxes with the selected instance's mapping, when the selection changed.
    pub(super) fn load_code_link_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, t)) = self.code_link_key() else {
            return;
        };
        let key = format!("{id}:{t:?}");
        if self.code_link_for == key {
            return;
        }
        let (name, import) = self
            .code_map
            .get(&id, t)
            .map(|l| (l.name.clone(), l.import.clone()))
            .unwrap_or_default();
        self.f_link_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.f_link_import
            .update(cx, |s, cx| s.set_value(import, window, cx));
        self.code_link_for = key;
    }

    fn save_code_link(&mut self, cx: &mut Context<Self>) {
        let Some((id, t)) = self.code_link_key() else {
            return;
        };
        if self.code_link_for != format!("{id}:{t:?}") {
            return;
        }
        let name = self.f_link_name.read(cx).value().trim().to_string();
        let import = self.f_link_import.read(cx).value().trim().to_string();
        let had = self.code_map.get(&id, t).cloned();
        if had.as_ref().map(|l| (l.name.as_str(), l.import.as_str()))
            == Some((name.as_str(), import.as_str()))
            || had.is_none() && name.is_empty()
        {
            return;
        }
        self.code_map = CodeMap::load(&self.root);
        self.code_map.set(&id, t, &import, &name);
        match self.code_map.save(&self.root) {
            Ok(()) if name.is_empty() => {
                self.say("Unmapped. Code written from the design writes this component out again.")
            }
            Ok(()) => self.say(format!(
                "Mapped to {name}. Code written from the design uses it."
            )),
            Err(e) => self.say(format!("Couldn't save the code map: {e}")),
        }
        self.note_writes();
        cx.notify();
    }

    /// The selected instance's mapping to a component in the app's code.
    pub(super) fn code_link_section(&self) -> Option<Div> {
        let (id, t) = self.code_link_key()?;
        let (c, variant) = self.instance()?;
        let stack = match t {
            Target::Web => spec::WEB_LABEL,
            Target::Mobile => self.handoff_settings.mobile.label(),
        };
        let field = |label: &'static str, f: &Entity<InputState>| {
            div()
                .h(px(28.))
                .pl(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(5.))
                .bg(tint(c_subtle()))
                .child(
                    div()
                        .w(px(44.))
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(f).appearance(false).small().h(px(26.))),
                )
        };
        let swift = t == Target::Mobile && self.handoff_settings.mobile == spec::Mobile::SwiftUi;
        let note = match self.code_map.get(&id, t) {
            Some(link) => {
                let args = link.args(Some(c), &variant, swift);
                let used = if swift {
                    let a: Vec<String> = args.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                    format!("{}({})", link.name, a.join(", "))
                } else {
                    let a: String = args.iter().map(|(k, v)| format!(" {k}=\"{v}\"")).collect();
                    format!("<{}{a} />", link.name)
                };
                format!("This instance is written as {used}. Its variant's properties become props.")
            }
            None => format!(
                "Not mapped: code for {stack} writes {} out. Name the app's own component to use it instead.",
                c.name
            ),
        };
        Some(
            sec(format!("In code · {stack}"), None)
                .child(field("Name", &self.f_link_name))
                .child(field("Import", &self.f_link_import))
                .child(caption(note)),
        )
    }
}
