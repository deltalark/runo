//! Components on the canvas: making one from the selection, its instances' panel (variants,
//! pushing changes to every instance, resetting and detaching) and the Assets list that
//! puts instances on a screen.

use super::*;
use crate::designdraw::library::{self, Component, DEFAULT};
use gpui_kit::component::Sizable;

/// The colour components and their instances are marked with, as design tools do.
pub(super) const PURPLE: u32 = 0x9747ff;

impl DesignCanvas {
    /// The text boxes for the selected instance's component name and variant name.
    pub(super) fn make_component_fields(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        let f_comp = cx.new(|cx| InputState::new(window, cx).placeholder("Component name"));
        subs.push(
            cx.subscribe_in(&f_comp, window, |this, state, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = state.read(cx).value().trim().to_string();
                    this.rename_component(v, cx);
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        window.focus(&this.focus, cx);
                    }
                }
            }),
        );
        let f_variant = cx.new(|cx| InputState::new(window, cx).placeholder("Variant name"));
        subs.push(
            cx.subscribe_in(&f_variant, window, |this, state, ev, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = state.read(cx).value().trim().to_string();
                    this.rename_variant(v, cx);
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        window.focus(&this.focus, cx);
                    }
                }
            }),
        );
        (f_comp, f_variant)
    }

    /// Fill the component and variant names for the selected instance.
    pub(super) fn load_component_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vals = match self.instance() {
            Some((c, v)) => (c.name.clone(), v),
            None => return,
        };
        if self.comp_vals.0 != vals.0 {
            self.f_comp
                .update(cx, |s, cx| s.set_value(vals.0.clone(), window, cx));
        }
        if self.comp_vals.1 != vals.1 {
            self.f_variant
                .update(cx, |s, cx| s.set_value(vals.1.clone(), window, cx));
        }
        self.comp_vals = vals;
    }

    /// The component the one selected element is an instance of, and its variant.
    fn instance(&self) -> Option<(&Component, String)> {
        let el = self.element.as_ref().filter(|_| self.also.is_empty())?;
        let c = self
            .library
            .iter()
            .find(|c| Some(&c.id) == el.component.as_ref())?;
        let v = el
            .variant
            .clone()
            .filter(|v| c.variant(v).is_some())
            .or_else(|| c.variants.first().map(|v| v.name.clone()))?;
        Some((c, v))
    }

    fn save_component(&mut self, c: &Component) {
        if let Err(e) = library::save(&self.root, c) {
            self.say(e);
        }
        match self.library.iter_mut().find(|have| have.id == c.id) {
            Some(have) => *have = c.clone(),
            None => {
                self.library.push(c.clone());
                self.library.sort_by_key(|c| c.name.to_lowercase());
            }
        }
        self.note_writes();
    }

    /// Mark the selected element as an instance of `id` (as `variant`), then hand its HTML,
    /// marked, to `done`.
    fn mark_then(
        &mut self,
        id: String,
        variant: String,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, String, &mut Context<Self>) + 'static,
    ) {
        let (Some(sid), Some(el)) = (self.screen.clone(), self.element.clone()) else {
            return;
        };
        let Some(device) = self.canvas.screen(&sid).map(|s| s.device) else {
            return;
        };
        let path = el.path.clone();
        let sid2 = sid.clone();
        self.run(
            cx,
            move |r| {
                let mark = dom::call("mark", &[json!(path), json!(id), json!(variant)]);
                let edited = r.edit(&sid, device, &mark)?;
                let html = r.eval(&sid, device, &dom::call("copy", &[json!([path])]))?;
                let html: Vec<String> = serde_json::from_value(html).unwrap_or_default();
                Ok::<_, String>((edited, html.into_iter().next()))
            },
            move |this, out, cx| match out {
                Ok((edited, Some(html))) => {
                    if let Some((before, shot)) = edited {
                        this.landed(sid2, before, shot, After::Same, cx);
                    }
                    done(this, html, cx);
                }
                Ok((_, None)) => this.say("That element is gone from the screen."),
                Err(e) => this.say(e),
            },
        );
    }

    /// Make the selected element a component (⌥⌘K): kept under Assets, and reused as
    /// instances that follow it.
    pub(super) fn create_component(&mut self, cx: &mut Context<Self>) {
        let Some(el) = self.element.clone() else {
            return;
        };
        if !self.also.is_empty() {
            self.say("Group the elements first (⌘G), then make the group a component.");
            return;
        }
        if el.component.is_some() && self.instance().is_some() {
            self.say("That's already an instance of a component.");
            return;
        }
        let name = library::name_for(&el.tag, &self.element_classes(), el.text.as_deref());
        let id = library::new_id(&self.library, &name);
        let id2 = id.clone();
        self.mark_then(id, DEFAULT.into(), cx, move |this, html, cx| {
            let c = Component {
                id: id2,
                name: name.clone(),
                variants: vec![library::Variant {
                    name: DEFAULT.into(),
                    html,
                }],
            };
            this.save_component(&c);
            this.say(format!(
                "Made {name} a component. Use it on any screen from Assets."
            ));
            cx.notify();
        });
    }

    fn element_classes(&self) -> Vec<String> {
        let Some(el) = self.element.as_ref() else {
            return Vec::new();
        };
        el.label.split('.').skip(1).map(String::from).collect()
    }

    /// Save the selected instance's look as its variant, and update every instance of the
    /// component on every screen. Each instance keeps its own text.
    fn push_component(&mut self, cx: &mut Context<Self>) {
        let Some((c, v)) = self.instance().map(|(c, v)| (c.clone(), v)) else {
            return;
        };
        let id = c.id.clone();
        self.mark_then(id, v.clone(), cx, move |this, html, cx| {
            let mut c = c;
            c.set(&v, &html);
            this.save_component(&c);
            let js = dom::call("sync", &[json!(c.id), serde_json::Value::Object(c.map())]);
            this.on_every_screen(js, format!("{} updated", c.name), false, cx);
        });
    }

    /// One more variant of the component: the selected instance as it looks now.
    fn add_variant(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.instance().map(|(c, _)| c.clone()) else {
            return;
        };
        let name = c.next_variant();
        self.add_variant_named(c, name, cx);
    }

    fn add_variant_named(&mut self, c: Component, name: String, cx: &mut Context<Self>) {
        self.mark_then(c.id.clone(), name.clone(), cx, move |this, html, cx| {
            let mut c = c;
            c.set(&name, &html);
            this.save_component(&c);
            this.comp_vals.1.clear();
            this.say(format!(
                "Added {name} to {}. Change this instance to make it look its part.",
                c.name
            ));
            cx.notify();
        });
    }

    /// Show the selected instance as another of its component's variants.
    fn swap_variant(&mut self, name: &str, own: bool, cx: &mut Context<Self>) {
        let Some((c, _)) = self.instance() else {
            return;
        };
        let (Some(v), Some(path)) = (
            c.variant(name),
            self.element.as_ref().map(|e| e.path.clone()),
        ) else {
            return;
        };
        let js = dom::call("swap", &[json!(path), json!(v.html), json!(own)]);
        self.edit(js, After::Path(path), cx);
    }

    /// Make the selected instance a plain element again, no longer following its component.
    fn detach_instance(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.element.as_ref().map(|e| e.path.clone()) else {
            return;
        };
        let js = dom::call(
            "mark",
            &[
                json!(path),
                serde_json::Value::Null,
                serde_json::Value::Null,
            ],
        );
        self.edit(js, After::Same, cx);
    }

    fn rename_component(&mut self, name: String, cx: &mut Context<Self>) {
        let Some((c, _)) = self.instance() else {
            return;
        };
        if name.is_empty() || name == c.name {
            return;
        }
        let mut c = c.clone();
        c.name = name.clone();
        self.save_component(&c);
        self.comp_vals.0 = name;
        cx.notify();
    }

    fn rename_variant(&mut self, to: String, cx: &mut Context<Self>) {
        let Some((c, from)) = self.instance().map(|(c, v)| (c.clone(), v)) else {
            return;
        };
        if to.is_empty() || to == from {
            return;
        }
        if c.variant(&to).is_some() {
            self.say(format!("{} already has a variant called {to}.", c.name));
            self.comp_vals.1.clear();
            cx.notify();
            return;
        }
        let mut c = c;
        if let Some(v) = c.variants.iter_mut().find(|v| v.name == from) {
            v.name = to.clone();
            v.html = v.html.replace(
                &format!("data-variant=\"{}\"", attr(&from)),
                &format!("data-variant=\"{}\"", attr(&to)),
            );
        }
        self.save_component(&c);
        self.comp_vals.1 = to.clone();
        let js = dom::call("renameVariant", &[json!(c.id), json!(from), json!(to)]);
        self.on_every_screen(js, format!("Renamed the variant to {to}"), false, cx);
    }

    pub(super) fn make_prop_field(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let f = cx.new(|cx| InputState::new(window, cx));
        subs.push(cx.subscribe_in(&f, window, |this, state, ev, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let v = state.read(cx).value().trim().to_string();
                this.finish_prop_rename(v, cx);
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    window.focus(&this.focus, cx);
                }
            }
        }));
        f
    }

    /// Rename the component's variants, and every instance on every screen with them.
    fn rename_variants(
        &mut self,
        c: Component,
        renames: Vec<(String, String)>,
        what: String,
        cx: &mut Context<Self>,
    ) {
        if renames.is_empty() {
            return;
        }
        if renames.iter().any(|(_, to)| {
            c.variants.iter().any(|v| v.name == *to) && !renames.iter().any(|(f, _)| f == to)
        }) {
            self.say(format!("{} already has a variant like that.", c.name));
            return;
        }
        let mut c = c;
        c.rename_all(&renames);
        self.save_component(&c);
        self.comp_vals.1.clear();
        let map: serde_json::Map<String, serde_json::Value> =
            renames.into_iter().map(|(f, t)| (f, t.into())).collect();
        let js = dom::call(
            "renameVariants",
            &[json!(c.id), serde_json::Value::Object(map)],
        );
        self.on_every_screen(js, what, false, cx);
    }

    /// Give every variant one more property, "Property N", set to Default. A component whose
    /// variants have plain names gets them as the values of a State property first.
    fn add_property(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.instance().map(|(c, _)| c.clone()) else {
            return;
        };
        let axes = c.axes();
        let renames = if axes.is_empty() {
            c.renames(|ps| ps)
        } else {
            let name = (1..)
                .map(|n| format!("Property {n}"))
                .find(|n| !axes.iter().any(|(k, _)| k == n))
                .unwrap_or_default();
            c.renames(move |mut ps| {
                ps.push((name.clone(), DEFAULT.to_string()));
                ps
            })
        };
        self.rename_variants(c, renames, "Added a property".into(), cx);
    }

    /// One more value of `prop`: the selected instance as it looks now, as a new variant.
    fn add_value(&mut self, prop: String, cx: &mut Context<Self>) {
        let Some((c, now)) = self.instance().map(|(c, v)| (c.clone(), v)) else {
            return;
        };
        let values: Vec<String> = c
            .axes()
            .into_iter()
            .find(|(k, _)| *k == prop)
            .map(|(_, v)| v)
            .unwrap_or_default();
        let value = (2..)
            .map(|n| format!("Value {n}"))
            .find(|v| !values.contains(v))
            .unwrap_or_default();
        let mut ps = library::props(&now);
        for (k, v) in &mut ps {
            if *k == prop {
                *v = value.clone();
            }
        }
        let name = library::name_of(&ps);
        self.add_variant_named(c, name, cx);
    }

    fn start_prop_rename(
        &mut self,
        prop: String,
        value: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = value.clone().unwrap_or_else(|| prop.clone());
        self.renaming_prop = Some((prop, value));
        let input = self.f_prop.clone();
        input.update(cx, |s, cx| s.set_value(now, window, cx));
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            })
        });
        cx.notify();
    }

    fn finish_prop_rename(&mut self, to: String, cx: &mut Context<Self>) {
        let Some((prop, value)) = self.renaming_prop.take() else {
            return;
        };
        cx.notify();
        let to = to.replace([',', '='], " ").trim().to_string();
        let Some(c) = self.instance().map(|(c, _)| c.clone()) else {
            return;
        };
        if to.is_empty() || Some(&to) == value.as_ref().or(Some(&prop)) {
            return;
        }
        let renames = c.renames(|ps| {
            ps.into_iter()
                .map(|(k, v)| match &value {
                    None if k == prop => (to.clone(), v),
                    Some(old) if k == prop && v == *old => (k, to.clone()),
                    _ => (k, v),
                })
                .collect()
        });
        let what = match &value {
            None => format!("Renamed {prop} to {to}"),
            Some(old) => format!("Renamed {old} to {to}"),
        };
        self.rename_variants(c, renames, what, cx);
    }

    /// The component's variant properties, a row of values each: picking a value shows the
    /// variant with it, picking the shown one again renames it.
    fn property_rows(&self, c: &Component, variant: &str, cx: &mut Context<Self>) -> Div {
        let now = library::props(variant);
        let mut out = div().v_flex().gap(px(6.));
        for (pi, (prop, values)) in c.axes().into_iter().enumerate() {
            let editing = |v: Option<&String>| {
                self.renaming_prop
                    .as_ref()
                    .is_some_and(|(p, x)| *p == prop && x.as_ref() == v)
            };
            let label: AnyElement = if editing(None) {
                div()
                    .w(px(72.))
                    .child(Input::new(&self.f_prop).small().h(px(24.)))
                    .into_any_element()
            } else {
                let p = prop.clone();
                div()
                    .id(("design-prop", pi))
                    .w(px(72.))
                    .flex_none()
                    .truncate()
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .tooltip(tip("Rename this property"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_prop_rename(p.clone(), None, window, cx)
                    }))
                    .child(prop.clone())
                    .into_any_element()
            };
            let mut chips = div().flex_1().min_w_0().flex().flex_wrap().gap(px(4.));
            for (vi, value) in values.iter().enumerate() {
                let on = now.iter().any(|(k, v)| *k == prop && v == value);
                if editing(Some(value)) {
                    chips = chips.child(
                        div()
                            .w(px(80.))
                            .child(Input::new(&self.f_prop).small().h(px(24.))),
                    );
                    continue;
                }
                let (p, v) = (prop.clone(), value.clone());
                chips = chips.child(
                    chip(("design-prop-value", pi * 100 + vi), value.clone(), on)
                        .tooltip(tip(if on {
                            "Rename this value"
                        } else {
                            "Show this value"
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if on {
                                this.start_prop_rename(p.clone(), Some(v.clone()), window, cx);
                            } else if let Some(name) =
                                this.instance().and_then(|(c, now)| c.pick(&now, &p, &v))
                            {
                                this.swap_variant(&name, true, cx);
                            }
                        })),
                );
            }
            let p = prop.clone();
            chips = chips.child(
                figma::mini(
                    ("design-prop-add", pi),
                    IconName::Plus,
                    "Add a value: this instance as it looks now",
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.add_value(p.clone(), cx))),
            );
            out = out.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .child(label)
                    .child(chips),
            );
        }
        out
    }

    /// Change every screen with `js`, each a step of undo, and say `what` happened.
    fn on_every_screen(&mut self, js: String, what: String, quiet: bool, cx: &mut Context<Self>) {
        let screens: Vec<(String, Device)> = self
            .canvas
            .screens
            .iter()
            .map(|s| (s.id.clone(), s.device))
            .collect();
        self.run(
            cx,
            move |r| {
                let mut out = Vec::new();
                for (id, device) in screens {
                    if let Ok(Some((before, shot))) = r.edit(&id, device, &js) {
                        out.push((id, before, shot));
                    }
                }
                out
            },
            move |this, out, cx| {
                let n = out.len();
                for (id, before, shot) in out {
                    this.landed(id, before, shot, After::Same, cx);
                }
                if quiet && n == 0 {
                    return;
                }
                this.say(match n {
                    0 => format!("{what}. Its instances already matched."),
                    1 => format!("{what} on 1 screen."),
                    n => format!("{what} on {n} screens."),
                });
            },
        );
    }

    /// Have AI add the states the selected instance's component needs, as variants.
    fn component_states_with_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((c, _)) = self.instance() else {
            return;
        };
        let have: Vec<String> = c.variants.iter().map(|v| v.name.clone()).collect();
        let task = brief::component_states_task(&c.id, &c.name, &have);
        self.ask(task, window, cx);
    }

    /// After an agent changed components' files, bring their instances on every screen up
    /// to date, each keeping its own text, so a component change never leaves screens
    /// behind.
    pub(super) fn sync_changed_components(
        &mut self,
        before: &HashMap<PathBuf, Vec<u8>>,
        cx: &mut Context<Self>,
    ) {
        self.library = library::load(&self.root);
        let dir = library::folder(&self.root);
        let changed: Vec<Component> = self
            .library
            .iter()
            .filter(|c| {
                let file = dir.join(format!("{}.html", c.id));
                before.get(&file) != std::fs::read(&file).ok().as_ref()
            })
            .cloned()
            .collect();
        for c in changed {
            let js = dom::call("sync", &[json!(c.id), serde_json::Value::Object(c.map())]);
            self.on_every_screen(
                js,
                format!("Brought {}'s instances up to date", c.name),
                true,
                cx,
            );
        }
    }

    /// Put an instance of a component's variant on the selected screen.
    fn insert_instance(&mut self, id: &str, variant: &str, cx: &mut Context<Self>) {
        if self.screen.is_none() {
            self.say("Pick a screen to put it on.");
            return;
        }
        let Some(html) = self
            .library
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.variant(variant))
            .map(|v| v.html.clone())
        else {
            return;
        };
        self.paste_html(vec![html], true, cx);
    }

    fn delete_component(&mut self, id: &str, cx: &mut Context<Self>) {
        let path = library::folder(&self.root).join(format!("{id}.html"));
        if let Err(e) = std::fs::remove_file(&path) {
            self.say(format!("Couldn't delete {}: {e}", path.display()));
            return;
        }
        let name = self
            .library
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        self.library.retain(|c| c.id != id);
        self.note_writes();
        self.say(format!(
            "Deleted {name}. Its instances stay on the screens as they are."
        ));
        cx.notify();
    }

    // ---- What's drawn

    /// The right panel's section for a selected instance.
    pub(super) fn component_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let el = self.element.as_ref().filter(|_| self.also.is_empty())?;
        el.component.as_ref()?;
        let Some((c, variant)) = self.instance() else {
            return Some(
                figma::sec("Instance", None)
                    .child(figma::caption(
                        "Its component was deleted. Detach it to keep it as it is.",
                    ))
                    .child(
                        chip("design-detach", "Detach instance", false)
                            .on_click(cx.listener(|this, _, _, cx| this.detach_instance(cx))),
                    ),
            );
        };
        let name_box = div()
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .pl(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(5.))
            .bg(tint(c_subtle()))
            .child(icon(component_icon(), 12., PURPLE))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.f_comp)
                        .appearance(false)
                        .small()
                        .h(px(26.)),
                ),
            );
        let axes = !c.axes().is_empty();
        let mut chips = div().flex().flex_wrap().gap(px(4.));
        for (i, v) in c.variants.iter().enumerate().filter(|_| !axes) {
            let name = v.name.clone();
            chips = chips.child(
                chip(("design-variant", i), v.name.clone(), v.name == variant).on_click(
                    cx.listener(move |this, _, _, cx| this.swap_variant(&name, true, cx)),
                ),
            );
        }
        if !axes {
            chips = chips.child(
                figma::mini("design-variant-add", IconName::Plus, "Add a variant", false)
                    .on_click(cx.listener(|this, _, _, cx| this.add_variant(cx))),
            );
        }
        let props = self.property_rows(c, &variant, cx);
        let n_screens = self.canvas.screens.len();
        Some(
            figma::sec(
                "Instance",
                Some(
                    figma::mini("design-detach", named("unlink"), "Detach instance", false)
                        .on_click(cx.listener(|this, _, _, cx| this.detach_instance(cx)))
                        .into_any_element(),
                ),
            )
            .child(name_box)
            .child(
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
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child("Variant"),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.f_variant)
                                .appearance(false)
                                .small()
                                .h(px(26.)),
                        ),
                    ),
            )
            .child(chips)
            .child(props)
            .child(
                text_btn(
                    "design-prop-new",
                    "Add property",
                    Some(IconName::Plus),
                    false,
                )
                .tooltip(tip(
                    "Give the variants another property, like Size or State, to pick by",
                ))
                .on_click(cx.listener(|this, _, _, cx| this.add_property(cx))),
            )
            .child(
                text_btn(
                    "design-component-states",
                    "States with AI",
                    Some(IconName::Sparkles),
                    false,
                )
                .tooltip(tip(
                    "AI adds the hover, pressed, disabled or other states it needs as variants",
                ))
                .on_click(
                    cx.listener(|this, _, window, cx| this.component_states_with_ai(window, cx)),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        button_like("design-push", "Push changes", true)
                            .tooltip(tip(
                                "Save this look as the variant and update every instance",
                            ))
                            .on_click(cx.listener(|this, _, _, cx| this.push_component(cx))),
                    )
                    .child(
                        button_like("design-reset", "Reset", false)
                            .tooltip(tip("Make it match its variant again"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some((_, v)) = this.instance() {
                                    this.swap_variant(&v, false, cx);
                                }
                            })),
                    ),
            )
            .child(figma::caption(format!(
                "Pushing updates its instances on all {n_screens} screens. Each keeps its own text."
            ))),
        )
    }

    /// The left panel's Assets tab: every component and its variants. Clicking one puts
    /// an instance on the selected screen.
    pub(super) fn assets_list(&self, cx: &mut Context<Self>) -> Div {
        let mut list = div().v_flex().px_1().pb_3();
        if self.library.is_empty() {
            return list.child(div().px_2().pt_2().child(figma::caption(
                "Components show here. Select an element and press ⌥⌘K, or Create component \
                 in the Design panel, to reuse it on any screen.",
            )));
        }
        for (ci, c) in self.library.iter().enumerate() {
            let (id, first) = (
                c.id.clone(),
                c.variants
                    .first()
                    .map(|v| v.name.clone())
                    .unwrap_or_default(),
            );
            let id2 = id.clone();
            list = list.child(
                div()
                    .id(("design-asset", ci))
                    .group("design-asset")
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
                    .hover(|el| el.bg(tint(c_subtle())))
                    .tooltip(tip("Put it on the selected screen"))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.insert_instance(&id, &first, cx)),
                    )
                    .child(icon(component_icon(), 12., PURPLE))
                    .child(div().flex_1().min_w_0().truncate().child(c.name.clone()))
                    .child(
                        div()
                            .flex_none()
                            .invisible()
                            .group_hover("design-asset", |el| el.visible())
                            .child(
                                figma::mini(
                                    ("design-asset-del", ci),
                                    IconName::Trash,
                                    "Delete the component",
                                    false,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.delete_component(&id2, cx);
                                    },
                                )),
                            ),
                    ),
            );
            if c.variants.len() < 2 {
                continue;
            }
            for (vi, v) in c.variants.iter().enumerate() {
                let (id, name) = (c.id.clone(), v.name.clone());
                list = list.child(
                    div()
                        .id(("design-asset-variant", ci * 100 + vi))
                        .h(px(26.))
                        .pl(px(30.))
                        .pr_2()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.insert_instance(&id, &name, cx)),
                        )
                        .child(icon(named("diamond"), 11., PURPLE))
                        .child(div().flex_1().min_w_0().truncate().child(v.name.clone())),
                );
            }
        }
        list
    }
}

pub(super) fn component_icon() -> IconName {
    crate::archdraw::view::icon_named("component").unwrap_or(IconName::Square)
}

fn named(name: &str) -> IconName {
    crate::archdraw::view::icon_named(name).unwrap_or(IconName::Square)
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

/// A small choice in a row, highlighted when it's the one in use.
fn chip(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(24.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(11.5))
        .border_1()
        .border_color(rgb(if on { PURPLE } else { c_border() }))
        .text_color(rgb(if on { c_text() } else { c_secondary() }))
        .when(on, |el| el.bg(col(PURPLE, 0.14)))
        .hover(|el| el.bg(tint(c_subtle())))
        .child(label.into())
}

/// A button filling its share of a row; `primary` is the one most likely wanted.
fn button_like(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .when(primary, |el| {
            el.bg(rgb(PURPLE))
                .text_color(rgb(0xffffff))
                .hover(|el| el.opacity(0.9))
        })
        .when(!primary, |el| {
            el.border_1()
                .border_color(rgb(c_border()))
                .text_color(rgb(c_text()))
                .hover(|el| el.bg(tint(c_subtle())))
        })
        .child(label)
}
