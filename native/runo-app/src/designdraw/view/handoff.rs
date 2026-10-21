//! The Code tab: what a developer needs to build the picked layer. Its size and place, the
//! variables it uses, the CSS written for it and its markup, each ready to copy; or the
//! layer (or the whole screen) written as React or SwiftUI.

use super::figma::{caption, sec};
use super::*;

/// The declarations as a CSS rule for the element's own class, or its tag.
pub(crate) fn css_text(el: &dom::Element, code: &dom::Code) -> String {
    let selector = match el.classes.first() {
        Some(c) => format!(".{c}"),
        None => el.tag.clone(),
    };
    let mut out = format!("{selector} {{\n");
    for d in &code.css {
        out.push_str(&format!("  {}: {};\n", d.k, d.v));
    }
    out.push('}');
    out
}

/// The variables as a `:root` block, so a developer can bring them along.
pub(crate) fn vars_text(code: &dom::Code) -> String {
    let mut out = String::from(":root {\n");
    for d in &code.vars {
        out.push_str(&format!("  {}: {};\n", d.k, d.v));
    }
    out.push('}');
    out
}

/// What the Code tab writes the picked layer as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Lang {
    #[default]
    Css,
    React,
    Swift,
}

impl Lang {
    const ALL: [(Lang, &'static str); 3] = [
        (Lang::Css, "CSS"),
        (Lang::React, "React"),
        (Lang::Swift, "SwiftUI"),
    ];
}

/// Code written for a layer: the code itself, and for SwiftUI the theme it uses.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Written {
    /// What it's called: the component, or the view without "View".
    pub name: String,
    pub main: String,
    pub theme: Option<String>,
}

impl DesignCanvas {
    pub(super) fn code_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut langs = div().flex().gap(px(4.)).pt_3().pb_1();
        for (i, (lang, label)) in Lang::ALL.into_iter().enumerate() {
            langs = langs.child(
                chip_btn(("design-code-lang", i), label, self.code_lang == lang).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.code_lang = lang;
                        cx.notify();
                    }),
                ),
            );
        }
        let body = if self.code_lang == Lang::Css {
            self.css_panel(cx)
        } else {
            self.app_code_panel(cx)
        };
        div()
            .v_flex()
            .child(self.handoff_section(cx))
            .child(langs)
            .child(body)
    }

    /// The picked layer, or the screen, as React or SwiftUI.
    fn app_code_panel(&self, cx: &mut Context<Self>) -> Div {
        let Some(s) = self.screen.as_ref().and_then(|id| self.canvas.screen(id)) else {
            return div().v_flex().child(sec("Code", None).child(caption(
                "Pick a screen or a layer to write it as React or SwiftUI.",
            )));
        };
        let path = self
            .element
            .as_ref()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        let key = (s.id.clone(), path.clone(), self.undo.len());
        let lang = self.code_lang;
        let written = match &self.code_gen {
            Some((k, l, w)) if *k == key && *l == lang => Some(w.clone()),
            _ => {
                if self.gen_asked.borrow().as_ref() != Some(&(key.clone(), lang)) {
                    *self.gen_asked.borrow_mut() = Some((key.clone(), lang));
                    let me = cx.entity().downgrade();
                    let id = s.id.clone();
                    cx.defer(move |cx| {
                        let _ = me.update(cx, |this, cx| {
                            this.write_code(
                                id,
                                path,
                                lang,
                                move |this, w, cx| {
                                    this.code_gen = Some((key, lang, w));
                                    cx.notify();
                                },
                                cx,
                            )
                        });
                    });
                }
                None
            }
        };
        let what = match &self.element {
            Some(e) => e.name.clone().unwrap_or_else(|| e.label.clone()),
            None => s.name.clone(),
        };
        let (title, note) = if lang == Lang::React {
            (
                "React",
                "Keeps the design's classes: it needs design/tokens.css and components.css, which \
                 Export writes next to it. Components become React components, their variant \
                 properties props.",
            )
        } else {
            (
                "SwiftUI",
                "Stacks, text and shapes as the screen lays them out. Colours come from the \
                 design's tokens in Theme.swift, which Export writes next to it.",
            )
        };
        let Some(w) = written else {
            return div()
                .v_flex()
                .child(sec(what, None).child(caption(format!("Writing {title}…"))));
        };
        let mut out = div().v_flex().child(
            sec(
                format!("{what} in {title}"),
                Some(copy_btn("design-code-app", w.main.clone(), cx)),
            )
            .child(caption(note))
            .child(app_block("design-code-app-block", w.main.clone())),
        );
        if let Some(theme) = w.theme.clone() {
            out = out.child(
                sec(
                    "Theme.swift",
                    Some(copy_btn("design-code-theme", theme.clone(), cx)),
                )
                .child(app_block("design-code-theme-block", theme)),
            );
        }
        out
    }

    /// Write the layer at `path` on screen `id` (or the whole screen, with no path) as
    /// `lang`, then `done` with the code.
    pub(super) fn write_code(
        &mut self,
        id: String,
        path: dom::Path,
        lang: Lang,
        done: impl FnOnce(&mut Self, Written, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.canvas.screen(&id) else {
            return;
        };
        let device = s.device;
        let what = match self
            .element
            .as_ref()
            .filter(|e| e.path == path && !path.is_empty())
        {
            Some(e) => e.name.clone().unwrap_or_else(|| e.label.clone()),
            None => s.name.clone(),
        };
        let comps: serde_json::Map<String, serde_json::Value> = self
            .library
            .iter()
            .map(|c| (c.id.clone(), serde_json::Value::Object(c.map())))
            .collect();
        let js = dom::call("codeTree", &[json!(path), serde_json::Value::Object(comps)]);
        let lib = self.library.clone();
        let tokens =
            std::fs::read_to_string(self.root.join(FOLDER).join("tokens.css")).unwrap_or_default();
        let map = crate::designdraw::codemap::CodeMap::load(&self.root);
        let swift_app = crate::designdraw::spec::settings(&self.root).mobile
            == crate::designdraw::spec::Mobile::SwiftUi;
        self.run(
            cx,
            move |r| r.eval(&id, device, &js),
            move |this, out, cx| {
                let tree: Option<crate::designdraw::codegen::Tree> =
                    out.ok().and_then(|v| serde_json::from_value(v).ok());
                let Some(tree) = tree else {
                    this.say("Couldn't read the screen to write its code.");
                    cx.notify();
                    return;
                };
                let name = crate::designdraw::codegen::type_name(&what);
                let written = match lang {
                    Lang::Swift => {
                        let (main, theme) = crate::designdraw::codegen::swiftui(
                            &tree,
                            &format!("{name}View"),
                            &tokens,
                            &lib,
                            swift_app.then_some(&map),
                        );
                        Written {
                            name,
                            main,
                            theme: Some(theme),
                        }
                    }
                    _ => Written {
                        main: crate::designdraw::codegen::react(&tree, &name, &lib, &map),
                        name,
                        theme: None,
                    },
                };
                done(this, written, cx);
            },
        );
    }

    /// Save code written for Export, with the styles or theme it needs beside it.
    pub(super) fn save_code_export(&mut self, lang: Lang, code: Written, cx: &mut Context<Self>) {
        let name = &match lang {
            Lang::Swift => format!("{}View.swift", code.name),
            _ => format!("{}.tsx", code.name),
        };
        let dir = crate::designdraw::export::folder(&self.root);
        let design = self.root.join(FOLDER);
        let mut extra: Vec<(String, String)> = Vec::new();
        if lang == Lang::React {
            for sheet in ["tokens.css", "components.css"] {
                if let Ok(css) = std::fs::read_to_string(design.join(sheet)) {
                    extra.push((sheet.to_string(), css));
                }
            }
        }
        if let Some(theme) = code.theme {
            extra.push(("Theme.swift".into(), theme));
        }
        let _ = std::fs::create_dir_all(&dir);
        for (file, text) in &extra {
            if let Err(e) = std::fs::write(dir.join(file), text) {
                self.say(format!("Couldn't save {file}: {e}"));
                return;
            }
        }
        self.save_export(name, code.main.into_bytes(), cx);
        if !extra.is_empty() {
            let names: Vec<&str> = extra.iter().map(|(f, _)| f.as_str()).collect();
            self.say(format!(
                "Exported {FOLDER}/exports/{name}, with {}",
                names.join(" and ")
            ));
        }
    }

    fn css_panel(&self, cx: &mut Context<Self>) -> Div {
        let Some(s) = self.screen.as_ref().and_then(|id| self.canvas.screen(id)) else {
            return div().v_flex().child(sec("Code", None).child(caption(
                "Pick a layer to see its size, the variables it uses, its CSS and its HTML.",
            )));
        };
        let Some(el) = &self.element else {
            let file = format!("{FOLDER}/screens/{}.html", s.id);
            let path = self.root.join(&file);
            return div().v_flex().child(
                sec("Screen", None)
                    .child(caption(format!(
                        "{} is a plain HTML page at {file}, styled by {FOLDER}/tokens.css and \
                         {FOLDER}/components.css. Pick a layer to see its code.",
                        s.name
                    )))
                    .child(
                        text_btn(
                            "design-code-page",
                            "Copy screen HTML",
                            Some(IconName::Copy),
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            match std::fs::read_to_string(&path) {
                                Ok(html) => {
                                    cx.write_to_clipboard(ClipboardItem::new_string(html));
                                    this.say("Copied the screen's HTML.");
                                }
                                Err(e) => {
                                    this.say(format!("Couldn't read {}: {e}", path.display()))
                                }
                            }
                            cx.notify();
                        })),
                    ),
            );
        };
        // The code is read from the page when it's first shown, and again after an edit.
        let key = (s.id.clone(), el.path.clone(), self.undo.len());
        let code = match &self.code {
            Some((k, code)) if *k == key => code.clone(),
            _ => {
                if self.code_asked.borrow().as_ref() != Some(&key) {
                    *self.code_asked.borrow_mut() = Some(key.clone());
                    let me = cx.entity().downgrade();
                    cx.defer(move |cx| {
                        let _ = me.update(cx, |this, cx| this.load_code(key, cx));
                    });
                }
                dom::Code::default()
            }
        };
        let r = &el.rect;
        let name = el.name.clone().unwrap_or_else(|| el.label.clone());
        let mut size = div().v_flex().gap(px(2.));
        for (k, v) in [("Width", r.w), ("Height", r.h), ("Left", r.x), ("Top", r.y)] {
            size = size.child(row(k, format!("{}px", (v * 10.).round() / 10.)));
        }
        let mut out = div().v_flex().child(sec(name, None).child(size));
        if !code.vars.is_empty() {
            let mut list = div().v_flex().gap(px(4.));
            for d in &code.vars {
                let swatch =
                    crate::archdraw::model::parse_hex(&d.v).or_else(|| self.token_color(&d.k));
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.5))
                        .child(
                            div()
                                .size(px(12.))
                                .flex_none()
                                .rounded(px(3.))
                                .border_1()
                                .border_color(divider())
                                .when_some(swatch, |el, c| el.bg(rgb(c))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family("monospace")
                                .text_color(rgb(c_text()))
                                .child(d.k.clone()),
                        )
                        .child(
                            div()
                                .max_w(px(110.))
                                .truncate()
                                .text_color(rgb(c_muted()))
                                .child(d.v.clone()),
                        ),
                );
            }
            out = out.child(
                sec(
                    "Variables",
                    Some(copy_btn("design-code-vars", vars_text(&code), cx)),
                )
                .child(list),
            );
        }
        out.child(
            sec(
                "CSS",
                Some(copy_btn("design-code-css", css_text(el, &code), cx)),
            )
            .child(code_block(css_text(el, &code))),
        )
        .child(
            sec(
                "HTML",
                Some(copy_btn("design-code-html", code.html.clone(), cx)),
            )
            .child(code_block(code.html.clone())),
        )
    }
}

impl DesignCanvas {
    /// Read the code for the layer at `key` from its screen.
    fn load_code(&mut self, key: CodeKey, cx: &mut Context<Self>) {
        let Some(device) = self.canvas.screen(&key.0).map(|s| s.device) else {
            return;
        };
        let dir = self.root.join(FOLDER);
        let text: String = ["tokens.css", "components.css"]
            .iter()
            .filter_map(|f| std::fs::read_to_string(dir.join(f)).ok())
            .collect::<Vec<_>>()
            .join("\n");
        let js = dom::call("code", &[json!(key.1), json!(text)]);
        let id = key.0.clone();
        self.run(
            cx,
            move |r| r.eval(&id, device, &js),
            move |this, out, cx| {
                if let Some(code) = out.ok().and_then(|v| serde_json::from_value(v).ok()) {
                    this.code = Some((key, code));
                    cx.notify();
                }
            },
        );
    }
}

/// Which screen, which layer and which edit some code was read at.
pub(super) type CodeKey = (String, dom::Path, usize);

fn row(k: &'static str, v: String) -> Div {
    div()
        .flex()
        .justify_between()
        .text_size(px(11.5))
        .child(div().text_color(rgb(c_muted())).child(k))
        .child(div().text_color(rgb(c_text())).child(v))
}

fn code_block(text: String) -> Div {
    div()
        .p_2()
        .rounded(px(6.))
        .bg(rgb(c_subtle()))
        .border_1()
        .border_color(divider())
        .font_family("monospace")
        .text_size(px(11.))
        .text_color(rgb(c_text()))
        .child(text)
}

/// Code for an app, its lines kept whole and scrolled across.
fn app_block(id: &'static str, text: String) -> Stateful<Div> {
    div()
        .id(id)
        .max_h(px(520.))
        .overflow_scroll()
        .p_2()
        .rounded(px(6.))
        .bg(rgb(c_subtle()))
        .border_1()
        .border_color(divider())
        .font_family("monospace")
        .text_size(px(10.5))
        .text_color(rgb(c_text()))
        .child(div().whitespace_nowrap().child(text))
}

fn copy_btn(id: &'static str, text: String, cx: &mut Context<DesignCanvas>) -> AnyElement {
    super::figma::mini(id, IconName::Copy, "Copy", false)
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            this.say("Copied.");
            cx.notify();
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{css_text, vars_text};
    use crate::designdraw::dom::{Code, Decl, Element};

    #[test]
    fn code_reads_as_css_a_developer_can_paste() {
        let d = |k: &str, v: &str| Decl {
            k: k.into(),
            v: v.into(),
        };
        let el: Element = serde_json::from_value(serde_json::json!({
            "path": [0], "label": "button.btn", "tag": "button", "text": "Go",
            "rect": {"x": 0, "y": 0, "w": 10, "h": 10}, "classes": ["btn", "primary"]
        }))
        .unwrap();
        let code = Code {
            css: vec![d("background", "var(--primary)"), d("padding", "12px 16px")],
            vars: vec![d("--primary", "#2563eb")],
            html: String::new(),
        };
        assert_eq!(
            css_text(&el, &code),
            ".btn {\n  background: var(--primary);\n  padding: 12px 16px;\n}"
        );
        assert_eq!(vars_text(&code), ":root {\n  --primary: #2563eb;\n}");
    }
}
