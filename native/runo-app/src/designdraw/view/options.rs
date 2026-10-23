//! Try options: three takes on the selected layer, from one call to the agent, drawn in
//! place on the screen and shown side by side under the layer. Clicking one makes it the
//! layer, as one edit with one undo step; the others go.

use super::figma::caption;
use super::*;
use crate::designdraw::dom::Code;

/// How many takes are asked for.
const TAKES: usize = 3;
/// The widest a take is shown, in canvas pixels.
const CARD_W: f32 = 240.;

/// Takes on a layer, on their way or waiting for one to be picked.
pub(super) struct Options {
    pub screen: String,
    pub path: dom::Path,
    /// What they were asked to try, if anything was typed.
    pub asked: String,
    /// Each take's markup, and the screen drawn with it once ready.
    pub takes: Vec<(String, Option<Shot>)>,
    /// Still waiting for the agent.
    pub working: bool,
}

/// What the agent is asked: takes on the layer, each in its own block.
pub(super) fn prompt(asked: &str, code: &Code, tokens: &str) -> String {
    let css: String = code
        .css
        .iter()
        .map(|d| format!("  {}: {};\n", d.k, d.v))
        .collect();
    let toward = if asked.trim().is_empty() {
        String::new()
    } else {
        format!(" Each should answer this: {}.", asked.trim())
    };
    format!(
        "You are exploring designs for one layer of an HTML screen in a design tool. Make \
         {TAKES} clearly different takes on it.{toward} Vary layout, emphasis, type and \
         colour between them, not just one value. Answer with {TAKES} ```html blocks, one \
         take each, and nothing else.\n\n\
         Rules for every take:\n\
         - One element, the whole layer, the same tag unless a take needs another.\n\
         - Keep every data-* attribute it has, and its text, unless rewording is the point.\n\
         - Use the theme's variables, var(--name), and its component classes before any raw \
         value. Put new styles in a style attribute.\n\
         - Fit the space the layer has now.\n\n\
         The layer:\n```html\n{html}\n```\n\n\
         Styles that apply to it now:\n```css\n{css}```\n\n\
         The theme's tokens:\n```css\n{tokens}\n```\n",
        html = code.html.trim(),
        tokens = tokens.trim(),
    )
}

/// The takes in an answer: each ```html block that holds an element.
pub(super) fn takes(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("```") {
        let after = &rest[i + 3..];
        let body = after.trim_start_matches(|c: char| c.is_ascii_alphabetic());
        let Some(end) = body.find("```") else {
            break;
        };
        let html = body[..end].trim();
        if html.starts_with('<') && html.ends_with('>') && !out.iter().any(|t| t == html) {
            out.push(html.to_string());
        }
        rest = &body[end + 3..];
    }
    out.truncate(TAKES);
    out
}

impl DesignCanvas {
    /// Ask for takes on the one selected layer, toward what's typed in the ask bar.
    pub(super) fn try_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.options.is_some() || self.quick.is_some() || self.ai.is_some() {
            return;
        }
        let (Some(id), Some(el)) = (self.screen.clone(), self.element.as_ref()) else {
            return;
        };
        if !self.also.is_empty() {
            self.say("Try options works on one layer. Pick just one.");
            cx.notify();
            return;
        }
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let provider = agent::PROVIDERS
            .get(self.pick.agent)
            .unwrap_or(&agent::PROVIDERS[0]);
        let configured = crate::settings::AppSettings::load()
            .agent(provider.id)
            .binary;
        if !agent::available(provider.bin, &configured) {
            self.say(format!(
                "Try options needs {} on this computer.",
                provider.name
            ));
            cx.notify();
            return;
        }
        let asked = self.f_ask.read(cx).value().trim().to_string();
        self.f_ask.update(cx, |s, cx| s.set_value("", window, cx));
        let path = el.path.clone();
        let dir = folder(&self.root);
        let tokens = std::fs::read_to_string(dir.join("tokens.css")).unwrap_or_default();
        let sheets = format!(
            "{tokens}\n{}",
            std::fs::read_to_string(dir.join(COMPONENTS)).unwrap_or_default()
        );
        let (pid, model) = (provider.id, self.pick.model.clone());
        self.options = Some(Options {
            screen: id.clone(),
            path: path.clone(),
            asked: asked.clone(),
            takes: Vec::new(),
            working: true,
        });
        let js = dom::call("code", &[json!(path), json!(sheets)]);
        let root = self.root.to_string_lossy().into_owned();
        self.run(
            cx,
            move |r| r.eval(&id, device, &js),
            move |this, out, cx| {
                let code: Option<Code> = out.ok().and_then(|v| serde_json::from_value(v).ok());
                let Some(code) = code.filter(quick::fits) else {
                    this.options = None;
                    this.say("This layer is too big to try options on. Pick a smaller one.");
                    return;
                };
                let text = prompt(&asked, &code, &tokens);
                let mut command = agent::ask_command_on(pid, &configured, &text, &model);
                command
                    .current_dir(&root)
                    .stdin(std::process::Stdio::null());
                cx.spawn(async move |this, cx| {
                    let out = cx.background_spawn(async move { command.output() }).await;
                    let answer = match out {
                        Ok(o) if o.status.success() => {
                            Ok(String::from_utf8_lossy(&o.stdout).to_string())
                        }
                        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                    let _ = this.update(cx, |this, cx| this.options_answered(answer, cx));
                })
                .detach();
            },
        );
        cx.notify();
    }

    /// The agent answered: draw each take on the screen, unsaved.
    fn options_answered(&mut self, answer: Result<String, String>, cx: &mut Context<Self>) {
        let Some(o) = self.options.as_mut() else {
            return;
        };
        let found = answer.as_deref().map(takes).unwrap_or_default();
        if found.is_empty() {
            self.options = None;
            self.say(match answer {
                Err(e) if !e.is_empty() => format!("Try options didn't work: {e}"),
                _ => "The options came back empty. Try again.".into(),
            });
            cx.notify();
            return;
        }
        o.working = false;
        o.takes = found.iter().map(|h| (h.clone(), None)).collect();
        let (id, path) = (o.screen.clone(), o.path.clone());
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        for (i, html) in found.into_iter().enumerate() {
            let (id, path) = (id.clone(), path.clone());
            let js = dom::op(&path, "swapTo", &[json!(html)]);
            self.run(
                cx,
                move |r| r.shoot_trying(&id, device, &js),
                move |this, out, cx| match this.options.as_mut() {
                    Some(o) if o.path == path && i < o.takes.len() => {
                        drop_shot(std::mem::replace(&mut o.takes[i].1, out.ok()), cx);
                    }
                    _ => drop_shot(out.ok(), cx),
                },
            );
        }
        cx.notify();
    }

    /// Make take `i` the layer.
    fn pick_option(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(o) = self.options.take() else {
            return;
        };
        let html = o.takes.get(i).map(|t| t.0.clone());
        let (screen, path) = (o.screen.clone(), o.path.clone());
        drop_options(o, cx);
        let Some(html) = html else {
            return;
        };
        if self.screen.as_ref() != Some(&screen) {
            self.select_screen(Some(screen));
        }
        self.edit(
            dom::op(&path, "swapTo", &[json!(html)]),
            After::Path(path),
            cx,
        );
        cx.notify();
    }

    pub(super) fn close_options(&mut self, cx: &mut Context<Self>) {
        if let Some(o) = self.options.take() {
            drop_options(o, cx);
        }
        cx.notify();
    }

    /// The takes, side by side under the layer, each cut to the layer as it draws there.
    pub(super) fn options_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let o = self.options.as_ref()?;
        let s = self.canvas.screen(&o.screen)?;
        let el = self.element.as_ref().filter(|e| e.path == o.path)?;
        let r = &el.rect;
        let (_, y) = self.screen_pt((s.x + r.x, s.y + r.y + r.h));
        let (lx, _) = self.screen_pt((s.x + r.x + r.w / 2., 0.));
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(12.))
            .text_color(rgb(c_text()))
            .child(icon(IconName::Shuffle, 13., c_blue()))
            .child(div().flex_1().child(if o.working {
                "Making options…".to_string()
            } else if o.asked.is_empty() {
                "Options for this layer. Click one to use it.".to_string()
            } else {
                format!("Options: {}. Click one to use it.", summary(&o.asked))
            }))
            .child(
                icon_btn(
                    "design-options-close",
                    IconName::Close,
                    "Keep it as is (Esc)",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.close_options(cx))),
            );
        let mut row = div().flex().gap_2();
        if o.working {
            for i in 0..TAKES {
                row = row.child(
                    div()
                        .w(px(CARD_W * 0.6))
                        .h(px(80.))
                        .rounded_md()
                        .bg(tint(c_subtle()))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(caption(format!("Option {}", i + 1))),
                );
            }
        }
        for (i, (_, shot)) in o.takes.iter().enumerate() {
            row = row.child(self.option_card(i, shot.as_ref(), &o.path, cx));
        }
        let width = 24. + (o.takes.len().max(TAKES) as f32) * (CARD_W + 8.);
        let (vw, _) = self.view_size();
        let left = (lx - width / 2.).clamp(8., (vw - width - 8.).max(8.));
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(y + 12.))
                .p_3()
                .v_flex()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(float_edge())
                .bg(float(c_raised()))
                .shadow_lg()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(header)
                .child(row)
                .into_any_element(),
        )
    }

    /// One take: the layer cut from the screen drawn with it.
    fn option_card(
        &self,
        i: usize,
        shot: Option<&Shot>,
        path: &dom::Path,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let picture = shot.and_then(|shot| {
            let b = shot.boxes.iter().find(|b| &b.path == path)?;
            let scale = (CARD_W / b.rect.w.max(1.)).min(1.6);
            let (w, h) = (b.rect.w * scale, (b.rect.h * scale).min(220.));
            Some(
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .overflow_hidden()
                    .rounded(px(6.))
                    .bg(rgb(0xffffff))
                    .child(
                        img(ImageSource::Render(shot.image.clone()))
                            .absolute()
                            .left(px(-b.rect.x * scale))
                            .top(px(-b.rect.y * scale))
                            .w(px(shot.size.0 * scale))
                            .h(px(shot.size.1 * scale)),
                    ),
            )
        });
        div()
            .id(("design-option", i))
            .p(px(6.))
            .v_flex()
            .gap_1()
            .rounded_md()
            .border_1()
            .border_color(divider())
            .cursor_pointer()
            .hover(|d| d.border_color(rgb(c_blue())).bg(tint(c_subtle())))
            .child(caption(format!("Option {}", i + 1)))
            .child(picture.unwrap_or_else(|| {
                div()
                    .w(px(CARD_W * 0.6))
                    .h(px(60.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(caption("Drawing…".to_string()))
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.pick_option(i, cx)))
            .into_any_element()
    }
}

fn drop_options(o: Options, cx: &mut App) {
    for (_, shot) in o.takes {
        drop_shot(shot, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn takes_are_read_from_each_block() {
        let text = "Here:\n```html\n<a class=\"x\">1</a>\n```\n```html\n<a>2</a>\n```\nAnd\n```\n<a>3</a>\n```\n```html\n<a>4</a>\n```";
        assert_eq!(
            takes(text),
            vec!["<a class=\"x\">1</a>", "<a>2</a>", "<a>3</a>"]
        );
        assert_eq!(
            takes("```html\n<a>1</a>\n```\n```html\n<a>1</a>\n```"),
            vec!["<a>1</a>"]
        );
        assert!(takes("No.").is_empty());
    }

    #[test]
    fn the_prompt_asks_toward_what_was_typed() {
        let code = Code {
            html: "<button>Go</button>".into(),
            ..Default::default()
        };
        let p = prompt("warmer", &code, ":root { --primary: red; }");
        assert!(p.contains("Each should answer this: warmer."));
        assert!(p.contains("<button>Go</button>"));
        assert!(!prompt(" ", &code, "").contains("Each should answer"));
    }
}
