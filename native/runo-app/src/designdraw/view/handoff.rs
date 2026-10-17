//! The Code tab: what a developer needs to build the picked layer. Its size and place, the
//! variables it uses, the CSS written for it and its markup, each ready to copy.

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

impl DesignCanvas {
    pub(super) fn code_panel(&self, cx: &mut Context<Self>) -> Div {
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
                                .border_color(rgb(c_border()))
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
        .border_color(rgb(c_border()))
        .font_family("monospace")
        .text_size(px(11.))
        .text_color(rgb(c_text()))
        .child(text)
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
