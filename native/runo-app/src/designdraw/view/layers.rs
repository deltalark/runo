//! The layers panel's own tools: naming a layer (double-click, ⌘R), locking it so the canvas
//! can't pick it (⇧⌘L), and finding layers by name. Text styles live here too: the
//! Typography section's style chips, and making a style from the picked text.

use super::figma::mini;
use super::*;
use crate::designdraw::styles::{self, TextStyle};
use gpui_kit::component::Sizable;

impl DesignCanvas {
    pub(super) fn make_layer_fields(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        let f_layer = cx.new(|cx| InputState::new(window, cx).placeholder("Layer name"));
        subs.push(
            cx.subscribe_in(&f_layer, window, |this, state, ev, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    let v = state.read(cx).value().trim().to_string();
                    this.finish_rename(Some(v), cx);
                    window.focus(&this.focus, cx);
                }
                InputEvent::Blur => {
                    let v = state.read(cx).value().trim().to_string();
                    this.finish_rename(Some(v), cx);
                }
                _ => {}
            }),
        );
        let f_find = cx.new(|cx| InputState::new(window, cx).placeholder("Find layers"));
        subs.push(cx.subscribe_in(&f_find, window, |this, state, ev, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.layer_query = state.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        }));
        (f_layer, f_find)
    }

    /// What the layers panel calls a layer: its name, else its text, else its classes,
    /// else its tag.
    pub(super) fn layer_title(l: &Layer) -> String {
        let (tag, classes) = l.label.split_once('.').unwrap_or((l.label.as_str(), ""));
        let glyph = classes.split('.').find_map(|c| c.strip_prefix("i-"));
        match (&l.name, &l.text) {
            (Some(n), _) if !n.is_empty() => n.clone(),
            (_, Some(t)) if !t.is_empty() => t.clone(),
            _ if glyph.is_some() => format!("{} icon", glyph.unwrap_or_default()),
            _ if !classes.is_empty() => classes.replace('.', " "),
            _ => tag.to_string(),
        }
    }

    /// The layers the find box lets through, by index: every one when it's empty, else
    /// those whose title, tag or classes have what was typed.
    pub(super) fn shown_layers(&self) -> Vec<usize> {
        let q = &self.layer_query;
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                q.is_empty()
                    || Self::layer_title(l).to_lowercase().contains(q.as_str())
                    || l.label.to_lowercase().contains(q.as_str())
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn find_box(&self) -> Div {
        div()
            .px_2()
            .pb_1()
            .child(
                Input::new(&self.f_find)
                    .small()
                    .prefix(icon(IconName::Search, 12., c_muted())),
            )
    }

    /// Start naming a layer in place.
    pub(super) fn start_rename(
        &mut self,
        path: dom::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = self
            .layers
            .iter()
            .find(|l| l.path == path)
            .map(Self::layer_title)
            .unwrap_or_default();
        self.renaming = Some(path);
        let input = self.f_layer.clone();
        input.update(cx, |s, cx| s.set_value(now, window, cx));
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            })
        });
        cx.notify();
    }

    /// Name the layer being renamed `to`; nothing, or what it's called anyway, clears the
    /// name. `None` stops without a change.
    pub(super) fn finish_rename(&mut self, to: Option<String>, cx: &mut Context<Self>) {
        let Some(path) = self.renaming.take() else {
            return;
        };
        cx.notify();
        let Some(to) = to else {
            return;
        };
        let Some(l) = self.layers.iter().find(|l| l.path == path) else {
            return;
        };
        if l.name.as_deref().unwrap_or("") == to || (l.name.is_none() && Self::layer_title(l) == to)
        {
            return;
        }
        let unnamed = Layer {
            name: None,
            ..l.clone()
        };
        let v = if to == Self::layer_title(&unnamed) {
            String::new()
        } else {
            to
        };
        self.edit(
            dom::op(&path, "attr", &[json!("data-name"), json!(v)]),
            After::Same,
            cx,
        );
    }

    /// ⌘R: name the picked layer.
    pub(super) fn rename_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.element.as_ref().map(|e| e.path.clone()) {
            self.assets = false;
            if self.layers_of != self.screen {
                self.load_layers(cx);
            }
            self.start_rename(path, window, cx);
        }
    }

    /// Lock or unlock a layer. A locked layer stays picked if it was, so it can be unlocked.
    pub(super) fn toggle_lock(&mut self, path: &dom::Path, cx: &mut Context<Self>) {
        self.edit(dom::op(path, "lock", &[]), After::Same, cx);
    }

    /// ⇧⌘L: lock or unlock what's picked.
    pub(super) fn lock_selected(&mut self, cx: &mut Context<Self>) {
        self.op("lock", &[], After::Same, cx);
    }

    // ---- Text styles

    pub(super) fn css(&self) -> String {
        std::fs::read_to_string(styles::css_path(&self.root)).unwrap_or_default()
    }

    /// The Typography section's text styles, the picked text's one marked, and a button
    /// to make one from it.
    pub(super) fn text_styles(&self, el: &Element, cx: &mut Context<Self>) -> Div {
        let all = styles::all(&self.css());
        let classes: Vec<&str> = el.label.split('.').skip(1).collect();
        let current = all
            .iter()
            .find(|s| !s.class.is_empty() && classes.contains(&s.class.as_str()))
            .map(|s| s.class.clone());
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, s) in all.iter().enumerate() {
            let on = match &current {
                Some(c) => *c == s.class,
                None => s.class.is_empty() && !el.tag.starts_with('h'),
            };
            let class = s.class.clone();
            let every: Vec<String> = all
                .iter()
                .filter(|s| !s.class.is_empty())
                .map(|s| s.class.clone())
                .collect();
            row = row.child(
                chip_btn(("design-text-style", i), s.name.clone(), on).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.op("textStyle", &[json!(class), json!(every)], After::Same, cx)
                    },
                )),
            );
        }
        row
    }

    /// Make a text style from the picked text's type, keep it in components.css, and give
    /// the text that style.
    pub(super) fn create_text_style(&mut self, cx: &mut Context<Self>) {
        let Some(el) = self.element.clone() else {
            return;
        };
        let css = self.css();
        let name = styles::name_for(&el.style("font-size"), &el.style("font-weight"));
        let taken = styles::all(&css).iter().any(|s| s.name == name);
        let name = if taken {
            (2..)
                .map(|n| format!("{name} {n}"))
                .find(|n| !styles::all(&css).iter().any(|s| &s.name == n))
                .unwrap_or(name)
        } else {
            name
        };
        let style = TextStyle {
            class: styles::class_for(&css, &name),
            name,
        };
        let decls: Vec<(&str, String)> = styles::PROPS
            .iter()
            .map(|k| (*k, el.style(k)))
            .filter(|(k, v)| !(*k == "letter-spacing" && v == "normal"))
            .collect();
        let path = styles::css_path(&self.root);
        if let Err(e) = std::fs::write(&path, styles::add(&css, styles::Kind::Text, &style, &decls))
        {
            self.error = Some(format!("Couldn't save {}: {e}", path.display()));
            cx.notify();
            return;
        }
        let every: Vec<String> = styles::all(&self.css())
            .into_iter()
            .filter(|s| !s.class.is_empty() && s.class != style.class)
            .map(|s| s.class)
            .collect();
        self.op(
            "textStyle",
            &[json!(style.class), json!(every)],
            After::Same,
            cx,
        );
        self.say(format!("Made the text style “{}”", style.name));
        cx.notify();
    }

    /// The lock button on a layer row: shown while locked, else on hover.
    pub(super) fn lock_button(&self, i: usize, l: &Layer, cx: &mut Context<Self>) -> AnyElement {
        let path = l.path.clone();
        div()
            .flex_none()
            .when(!l.locked, |el| {
                el.invisible()
                    .group_hover("design-layer", |el| el.visible())
            })
            .child(
                mini(
                    ("design-layer-lock", i),
                    if l.locked {
                        IconName::Lock
                    } else {
                        IconName::LockOpen
                    },
                    if l.locked {
                        "Unlock (⇧⌘L)"
                    } else {
                        "Lock (⇧⌘L)"
                    },
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_lock(&path, cx);
                })),
            )
            .into_any_element()
    }
}
