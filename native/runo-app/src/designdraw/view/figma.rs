//! The editor's frame, laid out the way design tools are: tools along the top, layers on
//! the left, and on the right a Design tab of typed fields grouped into sections, a
//! Prototype tab, the theme and the checks.

use super::*;
use crate::designdraw::fields::{self, FIELDS, SHADOWS};
use gpui_kit::component::Sizable;

/// The left and right panels' widths.
pub(super) const LEFT_W: f32 = 236.;
pub(super) const RIGHT_W: f32 = 268.;

/// Tags whose type settings are worth showing even when they hold more than text.
const TEXT_TAGS: [&str; 13] = [
    "h1", "h2", "h3", "h4", "h5", "h6", "p", "span", "a", "button", "label", "strong", "li",
];

/// A section of the right panel: a title, an optional button at its right, then its rows.
fn sec(title: impl Into<SharedString>, action: Option<AnyElement>) -> Div {
    div()
        .v_flex()
        .gap(px(6.))
        .px_3()
        .pt(px(10.))
        .pb(px(12.))
        .border_b_1()
        .border_color(rgb(c_border()))
        .child(
            div()
                .h(px(22.))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_text()))
                        .child(title.into()),
                )
                .children(action),
        )
}

/// A small square button with an icon, for a section's add and remove.
fn mini(
    id: impl Into<ElementId>,
    name: IconName,
    tooltip: &'static str,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .when(on, |el| el.bg(tint(c_selected())))
        .hover(|el| el.bg(tint(c_subtle())))
        .tooltip(tip(tooltip))
        .child(icon(name, 13., if on { c_text() } else { c_secondary() }))
}

/// A field's leading mark: a letter or two, muted.
fn mark(text: &'static str) -> AnyElement {
    div()
        .flex_none()
        .min_w(px(12.))
        .text_size(px(11.))
        .text_color(rgb(c_muted()))
        .child(text)
        .into_any_element()
}

fn mark_icon(name: IconName) -> AnyElement {
    div()
        .flex_none()
        .child(icon(name, 12., c_muted()))
        .into_any_element()
}

/// A row of fields sharing the width.
fn pair(a: impl IntoElement, b: impl IntoElement) -> Div {
    div().flex().gap(px(6.)).child(a).child(b)
}

fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(c_muted()))
        .child(text.into())
}

fn color_of(hex: &str) -> Option<u32> {
    u32::from_str_radix(hex.trim_start_matches('#').get(..6)?, 16).ok()
}

impl DesignCanvas {
    /// The inspector's fields, one text box per entry of `fields::FIELDS`, each setting
    /// its styles on the selection when Enter is pressed or it loses focus.
    pub(super) fn make_fields(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Vec<(&'static str, Entity<InputState>)> {
        let mut out = Vec::new();
        for f in FIELDS.iter() {
            let id = f.id;
            let state = cx.new(|cx| InputState::new(window, cx));
            subs.push(
                cx.subscribe_in(&state, window, move |this, state, ev, window, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let v = state.read(cx).value().to_string();
                        this.apply_field(id, &v, cx);
                        if matches!(ev, InputEvent::PressEnter { .. }) {
                            window.focus(&this.focus, cx);
                        }
                    }
                }),
            );
            out.push((id, state));
        }
        out
    }

    fn field_state(&self, id: &str) -> Option<&Entity<InputState>> {
        self.fields.iter().find(|(f, _)| *f == id).map(|(_, s)| s)
    }

    /// Fill the fields from the selected element, leaving alone any that already show it,
    /// so what's being typed isn't replaced.
    pub(super) fn load_typed_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(el) = self.element.clone() else {
            return;
        };
        for (id, state) in self.fields.clone() {
            let Some(f) = fields::field(id) else {
                continue;
            };
            let v = fields::display(f, &el);
            if self.field_vals.get(id) != Some(&v) {
                state.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
                self.field_vals.insert(id, v);
            }
        }
    }

    /// Set what was typed in a field on the selection.
    fn apply_field(&mut self, id: &'static str, typed: &str, cx: &mut Context<Self>) {
        let (Some(f), Some(el)) = (fields::field(id), self.element.as_ref()) else {
            return;
        };
        if self.field_vals.get(id).map(String::as_str) == Some(typed.trim()) {
            return;
        }
        let Some(value) = fields::parse(f, typed) else {
            self.say(format!(
                "\"{}\" isn't a value this field takes.",
                typed.trim()
            ));
            self.field_vals.remove(id);
            cx.notify();
            return;
        };
        let mut map = serde_json::Map::new();
        for (k, v) in fields::styles(f, &value, el) {
            map.insert(k.into(), json!(v));
        }
        self.field_vals.insert(id, typed.trim().to_string());
        self.op("styles", &[serde_json::Value::Object(map)], After::Same, cx);
    }

    fn set_styles(&mut self, pairs: &[(&str, &str)], cx: &mut Context<Self>) {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), json!(v));
        }
        self.op("styles", &[serde_json::Value::Object(map)], After::Same, cx);
    }

    /// A typed field with its mark in front.
    fn fld(&self, id: &'static str, mark: AnyElement) -> Div {
        let input = self
            .field_state(id)
            .map(|s| Input::new(s).appearance(false).small().h(px(26.)));
        div()
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .pl(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(5.))
            .bg(tint(c_subtle()))
            .border_1()
            .border_color(tint(c_subtle()))
            .hover(|el| el.border_color(rgb(c_border())))
            .child(mark)
            .child(div().flex_1().min_w_0().children(input))
    }

    /// A colour field: a swatch of the colour, then its hex or token name.
    fn color_fld(&self, id: &'static str, el: &Element) -> Div {
        let shown = fields::field(id)
            .map(|f| fields::display(f, el))
            .unwrap_or_default();
        let c = color_of(&shown)
            .or_else(|| self.token_color(&format!("--{shown}")))
            .unwrap_or(0xffffff);
        self.fld(
            id,
            div()
                .size(px(14.))
                .flex_none()
                .rounded(px(3.))
                .border_1()
                .border_color(rgb(c_border()))
                .bg(rgb(c))
                .into_any_element(),
        )
    }

    // ---- The top bar

    pub(super) fn fig_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let sep = || div().w(px(1.)).h(px(18.)).mx_1().bg(rgb(c_border()));
        let tool = |id: &'static str, name: IconName, tip_text: &'static str, on: bool| {
            div()
                .id(id)
                .size(px(32.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(on, |el| el.bg(rgb(c_blue())))
                .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                .tooltip(tip(tip_text))
                .child(icon(name, 16., if on { 0xffffff } else { c_text() }))
        };
        let title = self
            .screen
            .as_ref()
            .and_then(|id| self.canvas.screen(id))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Design".into());
        let hand = self.hand;
        div()
            .h(px(44.))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_canvas()))
            .child(
                tool(
                    "design-tool-move",
                    IconName::MousePointer2,
                    "Move (V)",
                    !hand,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.hand = false;
                    cx.notify();
                })),
            )
            .child(
                tool(
                    "design-tool-frame",
                    IconName::Frame,
                    "New screen (F)",
                    self.new_open,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.new_open = !this.new_open;
                    cx.notify();
                })),
            )
            .child(
                tool("design-tool-text", IconName::Type, "Text (T)", false)
                    .on_click(cx.listener(|this, _, _, cx| this.insert("<p>Text</p>", cx))),
            )
            .child(
                tool("design-tool-image", IconName::Image, "Image", false).on_click(
                    cx.listener(|this, _, _, cx| this.insert("<div class=\"image\"></div>", cx)),
                ),
            )
            .child(
                tool("design-tool-hand", IconName::Hand, "Hand (H)", hand).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.hand = !this.hand;
                        cx.notify();
                    },
                )),
            )
            .child(sep())
            .child(
                tool("design-tool-ai", IconName::Sparkles, "Ask AI", false)
                    .on_click(cx.listener(|this, _, window, cx| this.focus_ask(window, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_center()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_text()))
                    .child(div().truncate().child(title)),
            )
            .child(
                icon_btn("design-undo", IconName::Undo2, "Undo (⌘Z)", false)
                    .when(
                        self.undo.is_empty() && !self.ai.as_ref().is_some_and(|ai| ai.changed),
                        |el| el.opacity(0.4),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
            )
            .child(
                icon_btn("design-redo", IconName::Redo2, "Redo (⇧⌘Z)", false)
                    .when(self.redo.is_empty(), |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
            )
            .child(sep())
            .child(
                icon_btn(
                    "design-arrange",
                    IconName::Workflow,
                    "Arrange along the flow",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.arrange(cx))),
            )
            .child(
                icon_btn(
                    "design-hotspots",
                    IconName::MousePointerClick,
                    "Show links between screens",
                    self.hotspots,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.hotspots = !this.hotspots;
                    cx.notify();
                })),
            )
            .child(
                icon_btn(
                    "design-dark",
                    if self.dark {
                        IconName::Moon
                    } else {
                        IconName::Sun
                    },
                    "Light or dark",
                    self.dark,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let d = !this.dark;
                    this.set_dark(d, cx)
                })),
            )
            .child(sep())
            .child(
                div()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .child(
                        icon_btn("design-zoom-out", IconName::Minus, "Zoom out (⌘-)", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.zoom_step(1. / 1.25);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("design-zoom")
                            .w(px(44.))
                            .text_center()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(rgb(c_text()))
                            .tooltip(tip("Zoom to fit (⇧1)"))
                            .child(format!("{}%", (self.zoom * 100.).round()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.fit();
                                cx.notify();
                            })),
                    )
                    .child(
                        icon_btn("design-zoom-in", IconName::Plus, "Zoom in (⌘=)", false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.zoom_step(1.25);
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(sep())
            .child(if self.playing.is_some() {
                text_btn("design-play", "Stop", Some(IconName::Square), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.playing = None;
                        cx.notify();
                    },
                ))
            } else {
                text_btn("design-play", "Present", Some(IconName::Play), true)
                    .tooltip(tip("Click through the prototype"))
                    .on_click(cx.listener(|this, _, _, cx| this.play(cx)))
            })
            .into_any_element()
    }

    // ---- The layers panel on the left

    pub(super) fn layers_side(&self, cx: &mut Context<Self>) -> AnyElement {
        let picked: Vec<dom::Path> = self
            .element
            .iter()
            .chain(&self.also)
            .map(|e| e.path.clone())
            .collect();
        let mut list = div().v_flex().px_1().pb_3();
        if self.canvas.screens.is_empty() {
            list = list.child(div().px_2().pt_2().child(caption(
                "Screens and what's on them show here. Add one with the frame tool.",
            )));
        }
        for (si, s) in self.canvas.screens.iter().enumerate() {
            let on = self.screen.as_ref() == Some(&s.id) && self.element.is_none();
            let open = self.screen.as_ref() == Some(&s.id);
            let id = s.id.clone();
            list = list.child(
                div()
                    .id(("design-frame-row", si))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_text()))
                    .when(on, |el| el.bg(tint(c_selected())))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_screen(Some(id.clone()));
                        this.load_layers(cx);
                        cx.notify();
                    }))
                    .child(icon(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        11.,
                        c_muted(),
                    ))
                    .child(icon(IconName::Hash, 12., c_secondary()))
                    .child(div().flex_1().min_w_0().truncate().child(s.name.clone()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(c_muted()))
                            .child(s.device.label()),
                    ),
            );
            if !open || self.layers_of.as_ref() != Some(&s.id) {
                continue;
            }
            for (i, l) in self.layers.iter().enumerate() {
                list = list.child(self.layer_row(i, l, picked.contains(&l.path), cx));
            }
        }
        div()
            .w(px(LEFT_W))
            .flex_none()
            .h_full()
            .v_flex()
            .border_r_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_canvas()))
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child("Layers")
                    .child(
                        mini("design-layers-add", IconName::Plus, "New screen (F)", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.new_open = !this.new_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("design-layers-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
            .into_any_element()
    }

    fn layer_row(&self, i: usize, l: &Layer, on: bool, cx: &mut Context<Self>) -> AnyElement {
        let (tag, classes) = l.label.split_once('.').unwrap_or((l.label.as_str(), ""));
        // What a person would call it: its text, else its class, else its tag.
        let title: SharedString = match &l.text {
            Some(t) if !t.is_empty() => t.clone().into(),
            _ if !classes.is_empty() => classes.replace('.', " ").into(),
            _ => tag.to_string().into(),
        };
        let glyph = layer_icon(tag);
        let (p1, p2, p3) = (l.path.clone(), l.path.clone(), l.path.clone());
        let dragged = DraggedLayer {
            path: l.path.clone(),
            label: l.label.clone().into(),
        };
        div()
            .id(("design-layer", i))
            .group("design-layer")
            .h(px(26.))
            .pl(px(22. + l.depth.min(8) as f32 * 12.))
            .pr_1()
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_size(px(12.))
            .when(on, |el| el.bg(tint(c_selected())))
            .hover(|el| el.bg(tint(c_subtle())))
            .when(l.hidden, |el| el.opacity(0.5))
            .text_color(rgb(c_text()))
            .on_drag(dragged, |d, _, _, cx| cx.new(|_| d.clone()))
            .drag_over::<DraggedLayer>(|el, _, _, _| el.border_t_2().border_color(rgb(c_blue())))
            .on_drop(cx.listener(move |this, d: &DraggedLayer, _, cx| {
                let target = p3.clone();
                if d.path != target {
                    let js = dom::op(&d.path, "moveTo", &[json!(target), json!(false)]);
                    this.edit(js, After::Clear, cx);
                }
            }))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                let id = this.screen.clone().unwrap_or_default();
                if e.modifiers().shift {
                    this.toggle_pick(&id, p1.clone(), cx);
                } else {
                    this.select_path(&id, p1.clone(), cx);
                }
            }))
            .child(icon(glyph, 12., if on { c_blue() } else { c_muted() }))
            .child(div().flex_1().min_w_0().truncate().child(title))
            .child(
                div()
                    .flex_none()
                    .when(!l.hidden, |el| {
                        el.invisible()
                            .group_hover("design-layer", |el| el.visible())
                    })
                    .child(
                        mini(
                            ("design-layer-eye", i),
                            if l.hidden {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            },
                            "Hide or show",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.edit(dom::op(&p2, "hide", &[]), After::Same, cx);
                        })),
                    ),
            )
            .into_any_element()
    }

    // ---- The right panel

    pub(super) fn right_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = |id: &'static str,
                   label: &'static str,
                   count: usize,
                   p: Panel,
                   cx: &mut Context<Self>| {
            let on = self.panel == p;
            div()
                .id(id)
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(if on {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(rgb(if on { c_text() } else { c_muted() }))
                .when(on, |el| el.bg(tint(c_subtle())))
                .hover(|el| el.text_color(rgb(c_text())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.panel = p;
                    cx.notify();
                }))
                .child(label)
                .when(count > 0, |el| {
                    el.child(
                        div()
                            .px(px(4.))
                            .rounded_full()
                            .bg(col(c_amber(), 0.2))
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_amber()))
                            .child(if count > 99 {
                                "99+".to_string()
                            } else {
                                count.to_string()
                            }),
                    )
                })
        };
        let body = match self.panel {
            Panel::Design => self.design_panel(cx),
            Panel::Prototype => self.prototype_panel(cx),
            Panel::Theme => div().px_3().child(self.theme_panel(cx)),
            Panel::Checks => div().px_3().child(self.checks_panel(cx)),
        };
        let issues: usize = self.findings().iter().map(|f| f.2.len()).sum();
        div()
            .w(px(RIGHT_W))
            .flex_none()
            .h_full()
            .v_flex()
            .border_l_1()
            .border_color(rgb(c_border()))
            .bg(tint(c_canvas()))
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .border_b_1()
                    .border_color(rgb(c_border()))
                    .child(tab("design-tab-design", "Design", 0, Panel::Design, cx))
                    .child(tab(
                        "design-tab-proto",
                        "Prototype",
                        0,
                        Panel::Prototype,
                        cx,
                    ))
                    .child(tab("design-tab-theme", "Theme", 0, Panel::Theme, cx))
                    .child(tab(
                        "design-tab-checks",
                        "Checks",
                        issues,
                        Panel::Checks,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("design-side")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb_4()
                    .child(body),
            )
            .into_any_element()
    }

    fn design_panel(&self, cx: &mut Context<Self>) -> Div {
        let Some(s) = self.screen.as_ref().and_then(|id| self.canvas.screen(id)) else {
            return div().px_3().child(self.add_screen_panel(cx));
        };
        match &self.element {
            None => self.frame_panel(s, cx),
            Some(el) => self.element_panel(el, cx),
        }
    }

    /// A screen with nothing on it picked: its name, its device and what to add.
    fn frame_panel(&self, s: &Screen, cx: &mut Context<Self>) -> Div {
        let mut devices = div().flex().gap_1();
        for (i, d) in Device::ALL.iter().enumerate() {
            let d = *d;
            devices = devices.child(
                chip_btn(("design-dev", i), d.label(), s.device == d)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_device(d, cx))),
            );
        }
        let (w, h) = self
            .shots
            .get(&s.id)
            .map(|shot| shot.size)
            .unwrap_or(s.device.size());
        let readout = |m: &'static str, v: f32| {
            div()
                .flex_1()
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(5.))
                .bg(tint(c_subtle()))
                .text_size(px(12.))
                .text_color(rgb(c_secondary()))
                .child(mark(m))
                .child(format!("{}", v.round()))
        };
        let mut actions = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(
                text_btn(
                    "design-dup-screen",
                    "Duplicate",
                    Some(IconName::Copy),
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.duplicate_screen(cx))),
            )
            .child(
                text_btn(
                    "design-variants",
                    "Variants with AI",
                    Some(IconName::Sparkles),
                    false,
                )
                .on_click(cx.listener(|this, _, window, cx| this.variants_with_ai(window, cx))),
            );
        if variant_of(&s.id).is_none() && brief::state_of(&s.id).is_none() {
            actions = actions.child(
                text_btn(
                    "design-states",
                    "States with AI",
                    Some(IconName::Layers),
                    false,
                )
                .tooltip(tip(
                    "AI designs this screen empty, loading and with an error, beside it",
                ))
                .on_click(cx.listener(|this, _, window, cx| this.states_with_ai(window, cx))),
            );
        }
        if variant_of(&s.id).is_some_and(|b| self.canvas.screen(&b).is_some()) {
            let id = s.id.clone();
            actions = actions.child(
                text_btn(
                    "design-use-variant",
                    "Use this variant",
                    Some(IconName::Check),
                    true,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.use_variant(&id, cx))),
            );
        }
        actions = actions.child(
            text_btn("design-del-screen", "Delete", Some(IconName::Trash), false)
                .on_click(cx.listener(|this, _, _, cx| this.delete_screen(cx))),
        );
        div()
            .v_flex()
            .child(
                sec("Frame", None)
                    .child(Input::new(&self.f_name).small())
                    .child(devices)
                    .child(pair(readout("W", w), readout("H", h)))
                    .child(caption(format!("{FOLDER}/screens/{}.html", s.id))),
            )
            .child(sec("Actions", None).child(actions))
            .child(sec("Insert", None).child(self.blocks(cx)))
    }

    /// An element, or several, picked on a screen: its fields, section by section.
    fn element_panel(&self, el: &Element, cx: &mut Context<Self>) -> Div {
        let multi = !self.also.is_empty();
        let mut out = div().v_flex();
        // The header: what's picked, and what can be done to it.
        let title: SharedString = if multi {
            format!("{} elements", self.also.len() + 1).into()
        } else {
            el.label.clone().into()
        };
        let mut tools = div().flex().items_center().gap(px(2.));
        if multi {
            tools = tools
                .child(
                    mini("design-group", IconName::Group, "Group (⌘G)", false)
                        .on_click(cx.listener(|this, _, _, cx| this.group(false, cx))),
                )
                .child(
                    mini("design-ungroup", IconName::Ungroup, "Ungroup (⇧⌘G)", false)
                        .on_click(cx.listener(|this, _, _, cx| this.ungroup(cx))),
                );
        } else {
            tools = tools
                .child(
                    mini("design-up", IconName::ArrowUp, "Move up (⌥↑)", false)
                        .on_click(cx.listener(|this, _, _, cx| this.nudge(-1, cx))),
                )
                .child(
                    mini("design-down", IconName::ArrowDown, "Move down (⌥↓)", false)
                        .on_click(cx.listener(|this, _, _, cx| this.nudge(1, cx))),
                )
                .child(
                    mini(
                        "design-parent",
                        IconName::ArrowUpLeft,
                        "Select its parent (Esc)",
                        false,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.escape(window, cx))),
                );
        }
        tools = tools
            .child(
                mini("design-dup", IconName::Copy, "Duplicate (⌘D)", false)
                    .on_click(cx.listener(|this, _, _, cx| this.duplicate_element(cx))),
            )
            .child(
                mini("design-del", IconName::Trash, "Delete (⌫)", false)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_element(cx))),
            );
        out = out.child(
            div()
                .px_3()
                .h(px(40.))
                .flex()
                .items_center()
                .gap_1()
                .border_b_1()
                .border_color(rgb(c_border()))
                .child(icon(
                    if multi {
                        IconName::Group
                    } else {
                        layer_icon(&el.tag)
                    },
                    13.,
                    c_secondary(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_text()))
                        .child(title),
                )
                .child(tools),
        );

        // Layout: size and padding.
        out = out.child(
            sec("Layout", None)
                .child(pair(self.fld("w", mark("W")), self.fld("h", mark("H"))))
                .child(label_row(
                    "Width",
                    self.choice_row(
                        "width",
                        el,
                        &[("Hug", "fit-content"), ("Fill", "100%"), ("Auto", "")],
                        cx,
                    ),
                ))
                .child(pair(
                    self.fld("pad-x", mark_icon(IconName::MoveHorizontal)),
                    self.fld("pad-y", mark_icon(IconName::MoveVertical)),
                )),
        );

        // Auto layout.
        if el.is_flex() {
            let across = fields::row(el);
            let (cell_c, cell_r) = fields::cell(el);
            let mut grid = div()
                .size(px(66.))
                .flex_none()
                .p(px(3.))
                .grid()
                .grid_cols(3)
                .rounded(px(5.))
                .bg(tint(c_subtle()));
            for r in 0..3 {
                for c in 0..3 {
                    let on = (c, r) == (cell_c, cell_r);
                    let (j, a) = fields::align_for(c, r, across);
                    grid = grid.child(
                        div()
                            .id(("design-align", r * 3 + c))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .rounded(px(3.))
                            .hover(|el| el.bg(tint(c_active())))
                            .child(
                                div()
                                    .when(on, |el| {
                                        el.w(px(if across { 4. } else { 12. }))
                                            .h(px(if across { 12. } else { 4. }))
                                            .bg(rgb(c_blue()))
                                    })
                                    .when(!on, |el| el.size(px(3.)).bg(rgb(c_muted())))
                                    .rounded_full(),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_styles(&[("justify-content", j), ("align-items", a)], cx)
                            })),
                    );
                }
            }
            let spread = el.style("justify-content") == "space-between";
            let wraps = el.style("flex-wrap") == "wrap";
            out = out.child(
                sec(
                    "Auto layout",
                    Some(
                        mini(
                            "design-al-off",
                            IconName::Minus,
                            "Remove auto layout",
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_styles(
                                &[
                                    ("display", "block"),
                                    ("flex-direction", ""),
                                    ("gap", ""),
                                    ("justify-content", ""),
                                    ("align-items", ""),
                                ],
                                cx,
                            )
                        }))
                        .into_any_element(),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex()
                                .gap(px(2.))
                                .p(px(2.))
                                .rounded(px(5.))
                                .bg(tint(c_subtle()))
                                .child(
                                    mini(
                                        "design-al-down",
                                        IconName::ArrowDown,
                                        "Vertical layout",
                                        !across,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.set_style("flex-direction", "column", cx)
                                        },
                                    )),
                                )
                                .child(
                                    mini(
                                        "design-al-across",
                                        IconName::ArrowRight,
                                        "Horizontal layout",
                                        across,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.set_style("flex-direction", "row", cx)
                                        },
                                    )),
                                )
                                .child(
                                    mini(
                                        "design-al-wrap",
                                        IconName::Grid3x3,
                                        "Wrap onto more lines",
                                        wraps,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.set_style(
                                                "flex-wrap",
                                                if wraps { "" } else { "wrap" },
                                                cx,
                                            )
                                        },
                                    )),
                                ),
                        )
                        .child(self.fld(
                            "gap",
                            mark_icon(if across {
                                IconName::AlignHorizontalSpaceBetween
                            } else {
                                IconName::AlignVerticalSpaceBetween
                            }),
                        )),
                )
                .child(
                    div().flex().gap(px(8.)).child(grid).child(
                        div()
                            .v_flex()
                            .gap(px(4.))
                            .child(caption("Where the children sit"))
                            .child(
                                chip_btn("design-al-spread", "Space between", spread).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.set_style(
                                            "justify-content",
                                            if spread { "" } else { "space-between" },
                                            cx,
                                        )
                                    }),
                                ),
                            ),
                    ),
                ),
            );
        } else {
            out = out.child(
                sec(
                    "Auto layout",
                    Some(
                        mini(
                            "design-al-on",
                            IconName::Plus,
                            "Add auto layout (⇧A)",
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.auto_layout(cx)))
                        .into_any_element(),
                    ),
                )
                .child(caption(
                    "Lay children out in a row or column, with even gaps.",
                )),
            );
        }

        // Appearance.
        out = out.child(
            sec(
                "Appearance",
                Some(
                    mini(
                        "design-hide",
                        if el.hidden {
                            IconName::EyeOff
                        } else {
                            IconName::Eye
                        },
                        "Hide or show",
                        el.hidden,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.op("hide", &[], After::Same, cx)))
                    .into_any_element(),
                ),
            )
            .child(pair(
                self.fld("opacity", mark_icon(IconName::Blend)),
                self.fld("radius", mark_icon(IconName::SquareRoundCorner)),
            )),
        );

        // Typography.
        if el.text.is_some() || TEXT_TAGS.contains(&el.tag.as_str()) {
            let align = el.style("text-align");
            let mut aligns = div()
                .flex()
                .gap(px(2.))
                .p(px(2.))
                .rounded(px(5.))
                .bg(tint(c_subtle()));
            for (i, (name, v, t)) in [
                (IconName::TextAlignStart, "left", "Align left"),
                (IconName::TextAlignCenter, "center", "Align centre"),
                (IconName::TextAlignEnd, "right", "Align right"),
            ]
            .into_iter()
            .enumerate()
            {
                let on = align == v || (v == "left" && (align == "start" || align.is_empty()));
                aligns = aligns.child(mini(("design-talign", i), name, t, on).on_click(
                    cx.listener(move |this, _, _, cx| this.set_style("text-align", v, cx)),
                ));
            }
            let mut text = sec("Typography", None);
            if el.text.is_some() && !multi {
                text = text.child(Input::new(&self.f_text).small());
            }
            text = text
                .child(pair(
                    self.fld("font-size", mark_icon(IconName::CaseSensitive)),
                    self.color_fld("color", el),
                ))
                .child(self.choice_row(
                    "font-weight",
                    el,
                    &[
                        ("Regular", "400"),
                        ("Medium", "500"),
                        ("Semibold", "600"),
                        ("Bold", "700"),
                    ],
                    cx,
                ))
                .child(pair(
                    self.fld("line-height", mark_icon(IconName::Baseline)),
                    self.fld("letter", mark("|A|")),
                ))
                .child(aligns);
            out = out.child(text);
        }

        // Fill.
        let fill = fields::field("fill")
            .map(|f| fields::display(f, el))
            .unwrap_or_default();
        out = out.child(if fill.is_empty() {
            sec(
                "Fill",
                Some(
                    mini("design-fill-add", IconName::Plus, "Add a fill", false)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_style("background-color", "var(--surface)", cx)
                        }))
                        .into_any_element(),
                ),
            )
        } else {
            sec(
                "Fill",
                Some(
                    mini("design-fill-off", IconName::Minus, "Remove the fill", false)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_style("background-color", "transparent", cx)
                        }))
                        .into_any_element(),
                ),
            )
            .child(self.color_fld("fill", el))
            .child(self.token_swatches("background-color", cx))
        });

        // Stroke.
        let stroked = el.style("border-top-style") != "none"
            && fields::field("stroke-w")
                .map(|f| fields::display(f, el))
                .is_some_and(|w| w != "0" && w != "Auto");
        out = out.child(if stroked {
            sec(
                "Stroke",
                Some(
                    mini(
                        "design-stroke-off",
                        IconName::Minus,
                        "Remove the stroke",
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_styles(
                            &[
                                ("border-style", "none"),
                                ("border-width", ""),
                                ("border-color", ""),
                            ],
                            cx,
                        )
                    }))
                    .into_any_element(),
                ),
            )
            .child(pair(
                self.color_fld("stroke", el),
                div()
                    .w(px(84.))
                    .flex_none()
                    .flex()
                    .child(self.fld("stroke-w", mark_icon(IconName::Minus))),
            ))
        } else {
            sec(
                "Stroke",
                Some(
                    mini("design-stroke-add", IconName::Plus, "Add a stroke", false)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_styles(
                                &[
                                    ("border-style", "solid"),
                                    ("border-width", "1px"),
                                    ("border-color", "var(--border)"),
                                ],
                                cx,
                            )
                        }))
                        .into_any_element(),
                ),
            )
        });

        // Effects.
        out = out.child(sec("Effects", None).child(label_row(
            "Drop shadow",
            self.choice_row("box-shadow", el, &SHADOWS, cx),
        )));

        out.child(
            sec(
                if el.is_container() && !multi {
                    "Insert inside"
                } else {
                    "Insert after"
                },
                None,
            )
            .child(self.blocks(cx)),
        )
    }

    /// The theme's colours as swatches that set a style to the token.
    fn token_swatches(&self, key: &'static str, cx: &mut Context<Self>) -> Div {
        let now = self
            .element
            .as_ref()
            .map(|e| e.style(key))
            .unwrap_or_default();
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, t) in COLOR_TOKENS.iter().enumerate() {
            let t: &'static str = t;
            let value = format!("var({t})");
            let on = now.trim() == value;
            let c = self.token_color(t).unwrap_or(0x888888);
            row = row.child(
                div()
                    .id((key, i))
                    .size(px(18.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(if on { c_blue() } else { c_border() }))
                    .when(on, |el| el.border_2())
                    .bg(rgb(c))
                    .cursor_pointer()
                    .tooltip(tip(token_name(t)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_style(key, &value, cx))),
            );
        }
        row
    }

    // ---- Prototype

    fn prototype_panel(&self, cx: &mut Context<Self>) -> Div {
        let Some(s) = self.screen.as_ref().and_then(|id| self.canvas.screen(id)) else {
            return div().v_flex().child(
                sec("Prototype", None).child(caption(
                    "Pick an element and choose the screen it opens, or drag its blue dot onto that screen. Present clicks through it.",
                )),
            );
        };
        let Some(el) = &self.element else {
            let mut platforms = div().flex().gap_1();
            for (i, (label, p)) in [("iOS", "ios"), ("Android", "android")]
                .into_iter()
                .enumerate()
            {
                let on =
                    self.canvas.platform == p || (p == "ios" && self.canvas.platform.is_empty());
                platforms = platforms.child(
                    chip_btn(("design-platform", i), label, on)
                        .on_click(cx.listener(move |this, _, _, cx| this.set_platform(p, cx))),
                );
            }
            return div()
                .v_flex()
                .child(
                    sec("Flow", None)
                        .child(caption(format!(
                            "Present starts from {} and follows its links.",
                            s.name
                        )))
                        .child(
                            text_btn(
                                "design-play-here",
                                "Present from here",
                                Some(IconName::Play),
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                        ),
                )
                .child(sec("Moves like", None).child(platforms));
        };
        let mut opens = div().flex().flex_wrap().gap(px(4.));
        opens = opens.child(
            chip_btn(("design-goto", 0usize), "Nowhere", el.goto.is_none()).on_click(cx.listener(
                |this, _, _, cx| this.op("attr", &[json!("data-goto"), json!("")], After::Same, cx),
            )),
        );
        for (i, t) in self.canvas.screens.iter().enumerate() {
            if Some(&t.id) == self.screen.as_ref() {
                continue;
            }
            let id = t.id.clone();
            opens = opens.child(
                chip_btn(
                    ("design-goto", i + 1),
                    t.name.clone(),
                    el.goto.as_deref() == Some(t.id.as_str()),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.op("attr", &[json!("data-goto"), json!(id)], After::Same, cx)
                })),
            );
        }
        let mut out = div().v_flex().child(
            sec("Interaction", None)
                .child(caption("On click, navigate to"))
                .child(opens),
        );
        if el.goto.is_some() {
            let mut moves = div().flex().flex_wrap().gap(px(4.));
            let now = el.transition.clone().unwrap_or_default();
            for (i, (label, value)) in MOVES.iter().enumerate() {
                let value: &'static str = value;
                moves = moves.child(chip_btn(("design-move", i), *label, now == value).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.op(
                            "attr",
                            &[json!("data-transition"), json!(value)],
                            After::Same,
                            cx,
                        )
                    }),
                ));
            }
            let mut speeds = div().flex().flex_wrap().gap(px(4.));
            let now = el.speed.clone().unwrap_or_default();
            for (i, (label, value)) in SPEEDS.iter().enumerate() {
                let value: &'static str = value;
                speeds = speeds.child(
                    chip_btn(("design-speed", i), *label, now == value).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.op(
                                "attr",
                                &[json!("data-speed"), json!(value)],
                                After::Same,
                                cx,
                            )
                        },
                    )),
                );
            }
            out = out
                .child(sec("Animation", None).child(moves))
                .child(sec("Speed", None).child(speeds));
        }
        out
    }
}
