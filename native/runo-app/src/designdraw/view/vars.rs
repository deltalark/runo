//! Variables: the theme's custom properties listed one per row with their light and dark
//! values, as design tools list them. A value or name is edited in place; renaming one
//! renames it everywhere the design uses it.

use super::figma::{caption, mini};
use super::*;
use gpui_kit::component::Sizable;

/// Which part of a variable is being edited.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Name,
    Light,
    Dark,
}

/// Whether a value is a colour rather than a size or a font.
fn is_colour(v: &str) -> bool {
    let v = v.trim();
    v.starts_with('#')
        || ["rgb", "hsl", "color-mix", "oklch", "transparent"]
            .iter()
            .any(|p| v.starts_with(p))
}

/// Whether a variable is one of the theme's own, which new screens and the kit rely on, so
/// it keeps its name.
fn built_in(name: &str) -> bool {
    var(TOKENS_CSS, name).is_some()
}

impl DesignCanvas {
    pub(super) fn make_var_field(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let f = cx.new(|cx| InputState::new(window, cx));
        subs.push(cx.subscribe_in(&f, window, |this, state, ev, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let v = state.read(cx).value().trim().to_string();
                this.finish_var_edit(v, cx);
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    window.focus(&this.focus, cx);
                }
            }
        }));
        f
    }

    /// A variable's colour, following other variables it's set to.
    fn var_colour(&self, value: &str, depth: usize) -> Option<u32> {
        if let Some(c) = crate::archdraw::model::parse_hex(value.trim()) {
            return Some(c);
        }
        let name = value.trim().strip_prefix("var(")?.strip_suffix(')')?;
        let next = var(&self.tokens, name)?;
        (depth < 4)
            .then(|| self.var_colour(&next, depth + 1))
            .flatten()
    }

    /// The theme's colour variables that aren't among the usual tokens, for swatches.
    pub(super) fn own_colours(&self) -> Vec<String> {
        vars(&self.tokens)
            .into_iter()
            .filter(|v| !COLOR_TOKENS.contains(&v.name.as_str()))
            .filter(|v| crate::archdraw::model::parse_hex(&v.light).is_some())
            .map(|v| v.name)
            .collect()
    }

    /// Write the theme and redraw every screen with it.
    fn write_tokens(&mut self, out: String, cx: &mut Context<Self>) {
        if let Err(e) = seed(&self.root) {
            self.say(format!("Couldn't write the theme: {e}"));
            return;
        }
        let file = folder(&self.root).join(TOKENS);
        if let Err(e) = std::fs::write(&file, &out) {
            self.say(format!("Couldn't write the theme: {e}"));
            return;
        }
        self.tokens = out;
        self.note_writes();
        self.asked.forget();
        self.draw_all(cx);
        cx.notify();
    }

    fn start_var_edit(
        &mut self,
        name: String,
        part: Part,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let v = vars(&self.tokens).into_iter().find(|v| v.name == name);
        let now = match (part, v) {
            (Part::Name, _) => name.trim_start_matches("--").to_string(),
            (Part::Light, Some(v)) => v.light,
            (Part::Dark, Some(v)) => v.dark.unwrap_or_default(),
            _ => String::new(),
        };
        self.editing_var = Some((name, part));
        let input = self.f_var.clone();
        input.update(cx, |s, cx| s.set_value(now, window, cx));
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            })
        });
        cx.notify();
    }

    fn finish_var_edit(&mut self, typed: String, cx: &mut Context<Self>) {
        let Some((name, part)) = self.editing_var.take() else {
            return;
        };
        cx.notify();
        let css = self.tokens.clone();
        match part {
            Part::Light if !typed.is_empty() => {
                let out = set_var(&css, &name, &typed);
                if out != css {
                    self.write_tokens(out, cx);
                }
            }
            Part::Dark => {
                let out = set_dark_var(&css, &name, &typed);
                if out != css {
                    self.write_tokens(out, cx);
                }
            }
            Part::Name => self.rename_variable(&name, &typed, cx),
            _ => {}
        }
    }

    /// Rename a variable in the theme and everywhere the design uses it.
    fn rename_variable(&mut self, from: &str, typed: &str, cx: &mut Context<Self>) {
        let to = format!("--{}", slug(typed.trim_start_matches('-')));
        if to == "--" || to == from {
            return;
        }
        if vars(&self.tokens).iter().any(|v| v.name == to) {
            self.say(format!("There's already a variable called {to}."));
            return;
        }
        let dir = folder(&self.root);
        let mut files = vec![dir.join(TOKENS), dir.join("components.css")];
        for sub in ["screens", "components"] {
            for e in std::fs::read_dir(dir.join(sub))
                .into_iter()
                .flatten()
                .flatten()
            {
                if e.path().extension().is_some_and(|x| x == "html") {
                    files.push(e.path());
                }
            }
        }
        let mut changed = 0;
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let out = rename_var(&text, from, &to);
            if out != text {
                if let Err(e) = std::fs::write(&f, &out) {
                    self.say(format!("Couldn't rename it in {}: {e}", f.display()));
                    return;
                }
                changed += 1;
            }
        }
        self.tokens = std::fs::read_to_string(dir.join(TOKENS)).unwrap_or_default();
        self.note_writes();
        self.asked.forget();
        self.library = crate::designdraw::library::load(&self.root);
        self.draw_all(cx);
        self.say(format!(
            "Renamed {from} to {to} in {changed} file{}.",
            if changed == 1 { "" } else { "s" }
        ));
    }

    /// A new colour or number variable, named for the person to change.
    fn add_variable(&mut self, colour: bool, window: &mut Window, cx: &mut Context<Self>) {
        let have = vars(&self.tokens);
        let base = if colour { "--color" } else { "--size" };
        let name = (1..)
            .map(|n| format!("{base}-{n}"))
            .find(|n| !have.iter().any(|v| v.name == *n))
            .unwrap_or_default();
        let out = set_var(&self.tokens, &name, if colour { "#888888" } else { "16px" });
        self.write_tokens(out, cx);
        self.start_var_edit(name, Part::Name, window, cx);
    }

    fn delete_variable(&mut self, name: &str, cx: &mut Context<Self>) {
        let out = remove_var(&self.tokens, name);
        self.write_tokens(out, cx);
        self.say(format!(
            "Deleted {name}. Anything using it falls back to its default."
        ));
    }

    /// The Variables section of the Theme tab: each variable with its light and dark value.
    pub(super) fn variables_section(&self, cx: &mut Context<Self>) -> Div {
        let all = vars(&self.tokens);
        let (colours, other): (Vec<_>, Vec<_>) = all
            .into_iter()
            .partition(|v| is_colour(&v.light) || self.var_colour(&v.light, 0).is_some());
        let head = |a: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pt(px(6.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_muted()))
                .child(div().flex_1().child(a))
                .child(div().w(px(76.)).child("Light"))
                .child(div().w(px(76.)).child("Dark"))
                .child(div().w(px(20.)))
        };
        let mut out = div()
            .v_flex()
            .gap(px(2.))
            .child(section("Variables"))
            .child(caption(
                "Every screen uses these. Click a value to change it, or a name to rename it everywhere.",
            ))
            .child(head("Colours"));
        for (i, v) in colours.iter().enumerate() {
            out = out.child(self.var_row(i, v, true, cx));
        }
        out = out.child(head("Sizes"));
        for (i, v) in other.iter().enumerate() {
            out = out.child(self.var_row(1000 + i, v, false, cx));
        }
        out.child(
            div()
                .flex()
                .gap(px(6.))
                .pt(px(6.))
                .child(
                    text_btn("design-var-colour", "Colour", Some(IconName::Plus), false)
                        .tooltip(tip("Add a colour variable"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_variable(true, window, cx)),
                        ),
                )
                .child(
                    text_btn("design-var-number", "Number", Some(IconName::Plus), false)
                        .tooltip(tip("Add a size variable"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_variable(false, window, cx)),
                        ),
                ),
        )
    }

    fn var_row(&self, i: usize, v: &Var, colour: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let editing = |part: Part| {
            self.editing_var
                .as_ref()
                .is_some_and(|(n, p)| *n == v.name && *p == part)
        };
        let field = || {
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&self.f_var).small().h(px(22.)))
        };
        let cell = |id: (&'static str, usize),
                    part: Part,
                    value: Option<String>,
                    cx: &mut Context<Self>| {
            let name = v.name.clone();
            let swatch = colour
                .then(|| value.as_deref().and_then(|x| self.var_colour(x, 0)))
                .flatten();
            let shown = value.clone().unwrap_or_else(|| "Same".into());
            let shown = shown
                .strip_prefix("var(--")
                .and_then(|x| x.strip_suffix(')'))
                .map(str::to_string)
                .unwrap_or(shown);
            div()
                .id(id)
                .w(px(76.))
                .h(px(24.))
                .px(px(4.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|el| el.bg(tint(c_subtle())))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_var_edit(name.clone(), part, window, cx)
                }))
                .when(editing(part), |el| el.child(field()))
                .when(!editing(part), |el| {
                    el.when_some(swatch, |el, c| {
                        el.child(
                            div()
                                .size(px(12.))
                                .flex_none()
                                .rounded(px(3.))
                                .border_1()
                                .border_color(divider())
                                .bg(rgb(c)),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(if value.is_some() { c_text() } else { c_muted() }))
                            .child(shown),
                    )
                })
        };
        let name = v.name.clone();
        let gone = v.name.clone();
        div()
            .id(("design-var", i))
            .group("design-var")
            .flex()
            .items_center()
            .gap(px(6.))
            .child(if editing(Part::Name) {
                field().into_any_element()
            } else if built_in(&v.name) {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(rgb(c_text()))
                    .child(v.name.trim_start_matches("--").to_string())
                    .into_any_element()
            } else {
                div()
                    .id(("design-var-name", i))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(rgb(c_text()))
                    .tooltip(tip("Rename it everywhere"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_var_edit(name.clone(), Part::Name, window, cx)
                    }))
                    .child(v.name.trim_start_matches("--").to_string())
                    .into_any_element()
            })
            .child(cell(
                ("design-var-light", i),
                Part::Light,
                Some(v.light.clone()),
                cx,
            ))
            .child(cell(("design-var-dark", i), Part::Dark, v.dark.clone(), cx))
            .child(
                div()
                    .w(px(20.))
                    .invisible()
                    .group_hover("design-var", |el| el.visible())
                    .when(!built_in(&v.name), |el| {
                        el.child(
                            mini(
                                ("design-var-del", i),
                                IconName::Trash,
                                "Delete the variable",
                                false,
                            )
                            .size(px(20.))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.delete_variable(&gone, cx)),
                            ),
                        )
                    }),
            )
    }
}
