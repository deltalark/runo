//! Styles: a look (fill, stroke, corners, shadow, padding, layout) made once from an element,
//! named, and given to others in a click. Changing the style changes every element that has
//! it, on every screen. Fills beyond a colour (gradients, pictures) live here too.

use super::figma::{caption, mini, sec};
use super::insert::PictureFor;
use super::*;
use crate::designdraw::styles::{self, Kind, TextStyle};
use gpui_kit::component::Sizable;

/// What stays on an element when its look becomes a style: where it is and how big. The
/// page's own `PLACE` says the same.
fn places(k: &str) -> bool {
    [
        "width",
        "height",
        "min-",
        "max-",
        "margin",
        "top",
        "left",
        "right",
        "bottom",
        "inset",
        "position",
        "z-index",
        "flex-grow",
        "flex-shrink",
        "flex-basis",
        "align-self",
        "order",
        "grid-column",
        "grid-row",
    ]
    .iter()
    .any(|p| k.starts_with(p))
        || k == "flex"
}

/// The look an element has of its own, which a style can be made from or added to.
fn own_look(el: &Element) -> Vec<&str> {
    let mut out: Vec<&str> = el
        .inline
        .iter()
        .filter(|(k, v)| !places(k) && !v.trim().is_empty())
        .map(|(k, _)| k.as_str())
        .collect();
    out.sort();
    out
}

/// What a property is called in the inspector, so a list of changes reads as words.
fn group(k: &str) -> &'static str {
    const GROUPS: [(&str, &str); 19] = [
        ("border-radius", "corners"),
        ("border-top-left-radius", "corners"),
        ("border-top-right-radius", "corners"),
        ("border-bottom-left-radius", "corners"),
        ("border-bottom-right-radius", "corners"),
        ("background", "fill"),
        ("border", "stroke"),
        ("outline", "stroke"),
        ("box-shadow", "shadow"),
        ("padding", "padding"),
        ("gap", "gap"),
        ("backdrop-filter", "background blur"),
        ("filter", "blur"),
        ("opacity", "opacity"),
        ("color", "text colour"),
        ("font", "type"),
        ("letter-spacing", "type"),
        ("line-height", "type"),
        ("text-", "type"),
    ];
    GROUPS
        .iter()
        .find(|(p, _)| k.starts_with(p))
        .map(|(_, g)| *g)
        .unwrap_or("layout")
}

/// The changes an element has over its style, each named once.
fn changes(own: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for k in own {
        let g = group(k);
        if !out.contains(&g) {
            out.push(g);
        }
    }
    out.join(", ")
}

/// The angles a gradient can run at, as the chips show them.
const ANGLES: [(&str, &str); 4] = [
    ("↓", "180deg"),
    ("→", "90deg"),
    ("↘", "135deg"),
    ("↗", "45deg"),
];

/// A gradient's angle and two stops, from `linear-gradient(135deg, a, b)`.
pub(super) fn gradient_parts(v: &str) -> Option<(String, String, String)> {
    let body = v
        .trim()
        .strip_prefix("linear-gradient(")?
        .strip_suffix(')')?;
    let mut parts = Vec::new();
    let (mut depth, mut cur) = (0, String::new());
    for c in body.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur).trim().to_string());
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    parts.push(cur.trim().to_string());
    match parts.as_slice() {
        [a, from, to] if a.ends_with("deg") || a.starts_with("to ") => {
            Some((a.clone(), from.clone(), to.clone()))
        }
        [from, to] => Some(("180deg".into(), from.clone(), to.clone())),
        _ => None,
    }
}

fn gradient(angle: &str, from: &str, to: &str) -> String {
    format!("linear-gradient({angle}, {from}, {to})")
}

impl DesignCanvas {
    pub(super) fn make_look_field(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let f = cx.new(|cx| InputState::new(window, cx).placeholder("Style name"));
        subs.push(
            cx.subscribe_in(&f, window, |this, state, ev, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    let v = state.read(cx).value().trim().to_string();
                    this.finish_style_rename(Some(v), cx);
                    window.focus(&this.focus, cx);
                }
                InputEvent::Blur => {
                    let v = state.read(cx).value().trim().to_string();
                    this.finish_style_rename(Some(v), cx);
                }
                _ => {}
            }),
        );
        f
    }

    /// The styles made in Design.
    fn looks(&self) -> Vec<TextStyle> {
        styles::own(&self.css(), Kind::Look)
    }

    /// The style an element has, if any.
    fn look_of(&self, el: &Element, looks: &[TextStyle]) -> Option<TextStyle> {
        looks
            .iter()
            .find(|s| el.classes.contains(&s.class))
            .cloned()
    }

    fn write_css(&mut self, css: &str, cx: &mut Context<Self>) -> bool {
        let path = styles::css_path(&self.root);
        if let Err(e) = std::fs::write(&path, css) {
            self.error = Some(format!("Couldn't save {}: {e}", path.display()));
            cx.notify();
            return false;
        }
        self.note_writes();
        true
    }

    /// Move the picked element's look into the style `class`, then do `done` with what
    /// moved, once the screen is saved.
    fn take_look(
        &mut self,
        class: String,
        done: impl FnOnce(&mut Self, Vec<(String, String)>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (Some(id), Some(el)) = (self.screen.clone(), self.element.clone()) else {
            return;
        };
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let js = dom::call("takeLook", &[json!(el.path), json!(class)]);
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.edit_giving(&id, device, &js),
            move |this, out, cx| match out {
                Ok(Some((before, shot, moved))) => {
                    let moved: Vec<(String, String)> =
                        serde_json::from_value(moved).unwrap_or_default();
                    done(this, moved, cx);
                    this.landed(id2.clone(), before, shot, After::Same, cx);
                    // The styles changed under every screen: draw them all again, then
                    // read what's picked once its screen has the new styles.
                    this.draw_all(cx);
                    let paths = this.selected_paths();
                    if this.screen.as_ref() == Some(&id2) && !paths.is_empty() {
                        this.select_many(&id2, paths, cx);
                    }
                }
                Ok(None) => {}
                Err(e) => this.say(e),
            },
        );
    }

    /// Make a style from the picked element's look, and give it the style.
    fn make_style(&mut self, cx: &mut Context<Self>) {
        let Some(el) = self.element.clone() else {
            return;
        };
        if own_look(&el).is_empty() {
            self.say("Give it a fill, stroke, corners or padding first, then make a style.");
            cx.notify();
            return;
        }
        let css = self.css();
        let taken = |n: &str| styles::own(&css, Kind::Look).iter().any(|s| s.name == n);
        let base = el
            .name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Style".into());
        let name = if taken(&base) || base == "Style" {
            (if base == "Style" { 1 } else { 2 }..)
                .map(|n| format!("{base} {n}"))
                .find(|n| !taken(n))
                .unwrap_or(base)
        } else {
            base
        };
        let style = TextStyle {
            class: styles::class_for(&css, &name),
            name,
        };
        let class = style.class.clone();
        self.take_look(
            class,
            move |this, moved, cx| {
                let decls: Vec<(&str, String)> =
                    moved.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                let css = styles::add(&this.css(), Kind::Look, &style, &decls);
                if this.write_css(&css, cx) {
                    this.say(format!("Made the style “{}”", style.name));
                }
            },
            cx,
        );
    }

    /// Give what's picked the style `class`, in place of any other style.
    fn apply_style(&mut self, class: String, cx: &mut Context<Self>) {
        let css = self.css();
        let props: Vec<String> = styles::decls(&css, &class)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let drop: Vec<String> = styles::own(&css, Kind::Look)
            .into_iter()
            .map(|s| s.class)
            .collect();
        self.op(
            "look",
            &[json!(class), json!(props), json!(drop)],
            After::Same,
            cx,
        );
    }

    /// Put the picked element's own changes into its style, so every element with the
    /// style changes too.
    fn update_style(&mut self, cx: &mut Context<Self>) {
        let Some(el) = self.element.clone() else {
            return;
        };
        let Some(style) = self.look_of(&el, &self.looks()) else {
            return;
        };
        let class = style.class.clone();
        self.take_look(
            class.clone(),
            move |this, moved, cx| {
                let css = styles::merge(&this.css(), &class, &moved);
                if this.write_css(&css, cx) {
                    this.say(format!("Updated the style “{}” everywhere", style.name));
                }
            },
            cx,
        );
    }

    /// Take what's picked out of its style, keeping how it looks.
    fn detach_style(&mut self, cx: &mut Context<Self>) {
        let Some(el) = self.element.clone() else {
            return;
        };
        let Some(style) = self.look_of(&el, &self.looks()) else {
            return;
        };
        let decls: Vec<[String; 2]> = styles::decls(&self.css(), &style.class)
            .into_iter()
            .map(|(k, v)| [k, v])
            .collect();
        self.op(
            "detach",
            &[json!(style.class), json!(decls)],
            After::Same,
            cx,
        );
    }

    fn start_style_rename(&mut self, class: String, window: &mut Window, cx: &mut Context<Self>) {
        let now = self
            .looks()
            .into_iter()
            .find(|s| s.class == class)
            .map(|s| s.name)
            .unwrap_or_default();
        self.renaming_style = Some(class);
        let input = self.f_look.clone();
        input.update(cx, |s, cx| s.set_value(now, window, cx));
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            })
        });
        cx.notify();
    }

    fn finish_style_rename(&mut self, to: Option<String>, cx: &mut Context<Self>) {
        let Some(class) = self.renaming_style.take() else {
            return;
        };
        cx.notify();
        let Some(to) = to.filter(|t| !t.is_empty()) else {
            return;
        };
        let css = self.css();
        let out = styles::rename(&css, &class, &to);
        if out != css {
            self.write_css(&out, cx);
        }
    }

    /// The Style section: the styles made in Design to give what's picked, and its own
    /// style to rename, update everywhere or let go of.
    pub(super) fn style_section(&self, el: &Element, cx: &mut Context<Self>) -> Div {
        let looks = self.looks();
        let now = self.look_of(el, &looks);
        let own = own_look(el);
        let add = (now.is_none() && !own.is_empty()).then(|| {
            mini(
                "design-style-add",
                IconName::Plus,
                "Make a style from this look",
                false,
            )
            .on_click(cx.listener(|this, _, _, cx| this.make_style(cx)))
            .into_any_element()
        });
        let mut out = sec("Style", add);
        if let Some(s) = &now {
            let class = s.class.clone();
            let name: AnyElement = if self.renaming_style.as_deref() == Some(&s.class) {
                div()
                    .flex_1()
                    .child(Input::new(&self.f_look).small())
                    .into_any_element()
            } else {
                div()
                    .id("design-style-name")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .tooltip(tip("Rename the style"))
                    .child(
                        div()
                            .size(px(14.))
                            .flex_none()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(rgb(components::PURPLE)),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_text()))
                            .child(s.name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_style_rename(class.clone(), window, cx)
                    }))
                    .into_any_element()
            };
            let mut row = div().flex().items_center().gap(px(2.)).child(name);
            if !own.is_empty() {
                row = row.child(
                    mini(
                        "design-style-update",
                        IconName::RefreshCw,
                        "Update the style everywhere with this element's changes",
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.update_style(cx))),
                );
            }
            row = row.child(
                mini(
                    "design-style-detach",
                    IconName::Unlink,
                    "Detach from the style",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.detach_style(cx))),
            );
            out = out.child(row);
            if !own.is_empty() {
                out = out.child(caption(format!(
                    "Changed here: {}. Update the style to change it everywhere.",
                    changes(&own)
                )));
            }
        }
        if looks.is_empty() {
            return out.child(caption(
                "Make a style from this look, then give it to anything in a click. Change the style and everything with it changes.",
            ));
        }
        let mut chips = div().flex().flex_wrap().gap(px(4.));
        for (i, s) in looks.iter().enumerate() {
            let on = now.as_ref().is_some_and(|n| n.class == s.class);
            let class = s.class.clone();
            chips = chips.child(chip_btn(("design-style", i), s.name.clone(), on).on_click(
                cx.listener(move |this, _, _, cx| {
                    if !on {
                        this.apply_style(class.clone(), cx)
                    }
                }),
            ));
        }
        out.child(chips)
    }

    // ---- Fills

    /// How the picked element is filled: a colour, a gradient or a picture.
    pub(super) fn fill_kind(el: &Element) -> &'static str {
        let img = el
            .inline
            .get("background-image")
            .or_else(|| el.computed.get("background-image"))
            .map(String::as_str)
            .unwrap_or("");
        if img.contains("gradient(") {
            "gradient"
        } else if img.contains("url(") {
            "picture"
        } else {
            "solid"
        }
    }

    /// Chips to fill the picked element with a colour, a gradient or a picture.
    pub(super) fn fill_kinds(&self, el: &Element, cx: &mut Context<Self>) -> Div {
        let kind = Self::fill_kind(el);
        div()
            .flex()
            .gap(px(4.))
            .child(
                chip_btn("design-fill-solid", "Solid", kind == "solid").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.set_styles(
                            &[
                                ("background-image", ""),
                                ("background-size", ""),
                                ("background-position", ""),
                                ("background", ""),
                            ],
                            cx,
                        )
                    },
                )),
            )
            .child(
                chip_btn("design-fill-gradient", "Gradient", kind == "gradient").on_click(
                    cx.listener(move |this, _, _, cx| {
                        if kind != "gradient" {
                            this.set_styles(
                                &[(
                                    "background-image",
                                    &gradient("135deg", "var(--primary)", "var(--surface)"),
                                )],
                                cx,
                            )
                        }
                    }),
                ),
            )
            .child(
                chip_btn("design-fill-picture", "Picture", kind == "picture").on_click(
                    cx.listener(|this, _, _, cx| this.choose_picture(PictureFor::Picked, cx)),
                ),
            )
    }

    /// The gradient's controls: which way it runs, and the colour at each end.
    pub(super) fn gradient_editor(&self, el: &Element, cx: &mut Context<Self>) -> Div {
        let v = el
            .inline
            .get("background-image")
            .or_else(|| el.computed.get("background-image"))
            .cloned()
            .unwrap_or_default();
        let (angle, from, to) = gradient_parts(&v).unwrap_or_else(|| {
            (
                "135deg".into(),
                "var(--primary)".into(),
                "var(--surface)".into(),
            )
        });
        let mut angles = div().flex().gap(px(4.));
        for (i, (label, a)) in ANGLES.iter().enumerate() {
            let a: &'static str = a;
            let (f, t) = (from.clone(), to.clone());
            angles = angles.child(
                chip_btn(("design-grad-angle", i), *label, angle == a).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.set_style("background-image", &gradient(a, &f, &t), cx)
                    },
                )),
            );
        }
        let stops = |end: usize| {
            let mut row = div().flex().flex_wrap().gap(px(4.));
            for (i, t) in COLOR_TOKENS.iter().enumerate() {
                let value = format!("var({t})");
                let on = if end == 0 { from == value } else { to == value };
                let c = self.token_color(t).unwrap_or(0x888888);
                let (a, f, to2) = (angle.clone(), from.clone(), to.clone());
                row = row.child(
                    div()
                        .id(("design-grad-stop", end * 100 + i))
                        .size(px(18.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(if on { c_blue() } else { c_border() }))
                        .when(on, |el| el.border_2())
                        .bg(rgb(c))
                        .cursor_pointer()
                        .tooltip(tip(token_name(t)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let g = if end == 0 {
                                gradient(&a, &value, &to2)
                            } else {
                                gradient(&a, &f, &value)
                            };
                            this.set_style("background-image", &g, cx)
                        })),
                );
            }
            row
        };
        div()
            .v_flex()
            .gap(px(6.))
            .child(label_row("Direction", angles))
            .child(label_row("From", stops(0)))
            .child(label_row("To", stops(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::{changes, gradient, gradient_parts, places};

    #[test]
    fn gradients_read_back_their_parts() {
        assert_eq!(
            gradient_parts(&gradient(
                "135deg",
                "var(--primary)",
                "color-mix(in srgb, red, blue)"
            )),
            Some((
                "135deg".into(),
                "var(--primary)".into(),
                "color-mix(in srgb, red, blue)".into()
            ))
        );
        assert_eq!(
            gradient_parts("linear-gradient(red, blue)"),
            Some(("180deg".into(), "red".into(), "blue".into()))
        );
        assert_eq!(gradient_parts("url(a.png)"), None);
        assert!(places("width") && places("margin-top") && places("flex"));
        assert_eq!(
            changes(&[
                "border-bottom-left-radius",
                "border-top-left-radius",
                "background-image",
                "letter-spacing"
            ]),
            "corners, fill, type"
        );
        assert!(!places("padding-left") && !places("background") && !places("flex-direction"));
    }
}
