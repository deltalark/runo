//! Quick edits: an ask about one selected layer goes straight to the agent as a single
//! call that returns the layer's new markup, and lands as one edit with one undo step,
//! instead of starting a whole session. Asks that need more than the layer go on to a
//! session as before.

use super::*;
use crate::designdraw::dom::Code;

/// The longest markup a quick edit sends; a bigger layer goes to a session.
const MOST: usize = 3_500;

/// A quick edit on its way.
pub(super) struct Quick {
    /// The ask as typed, and shortened for the pill.
    pub asked: String,
    pub task: String,
    pub screen: String,
    pub path: dom::Path,
}

/// What the agent answered.
#[derive(Debug, PartialEq)]
pub(super) enum Reply {
    /// The layer's new markup.
    Html(String),
    /// The ask needs more than this layer, and why.
    Session(String),
}

/// What the agent is asked: the layer as code, the theme's tokens, and how to answer.
pub(super) fn prompt(task: &str, code: &Code, tokens: &str) -> String {
    let css: String = code
        .css
        .iter()
        .map(|d| format!("  {}: {};\n", d.k, d.v))
        .collect();
    let vars: String = code
        .vars
        .iter()
        .map(|d| format!("  {}: {};\n", d.k, d.v))
        .collect();
    format!(
        "You are editing one layer of an HTML screen in a design tool. Change it as asked and \
         answer with only its new HTML in one ```html block, nothing else.\n\n\
         Rules:\n\
         - Answer with the whole layer: one element, the same tag unless the ask needs another.\n\
         - Keep every data-* attribute and class it has unless the ask is to change them.\n\
         - Use the theme's variables, var(--name), for colours, sizes, radii and type, and \
         its component classes, before any raw value. Put new styles in a style attribute.\n\
         - Keep text and children the ask doesn't mention.\n\
         - If the ask needs changes outside this layer (other layers, other screens, new \
         screens), answer only `SESSION: ` and one line saying what it needs.\n\n\
         The ask: {task}\n\n\
         The layer:\n```html\n{html}\n```\n\n\
         Styles that apply to it now:\n```css\n{css}```\n\n\
         Variables it uses:\n```css\n{vars}```\n\n\
         The theme's tokens:\n```css\n{tokens}\n```\n",
        html = code.html.trim(),
        tokens = tokens.trim(),
    )
}

/// Whether a layer is small enough for a quick edit.
pub(super) fn fits(code: &Code) -> bool {
    !code.html.is_empty() && code.html.len() <= MOST && !code.html.ends_with('…')
}

/// Read the agent's answer: the markup in its ```html block, or the first element in it.
pub(super) fn reply(text: &str) -> Option<Reply> {
    let t = text.trim();
    if let Some(i) = t.find("SESSION:") {
        let why = t[i + "SESSION:".len()..]
            .lines()
            .next()
            .unwrap_or("")
            .trim();
        return Some(Reply::Session(why.to_string()));
    }
    let body = match t.find("```html") {
        Some(i) => {
            let rest = &t[i + "```html".len()..];
            &rest[..rest.find("```").unwrap_or(rest.len())]
        }
        None => match t.find("```") {
            Some(i) => {
                let rest = &t[i + 3..];
                let rest = rest.trim_start_matches(|c: char| c.is_ascii_alphabetic());
                &rest[..rest.find("```").unwrap_or(rest.len())]
            }
            None => t,
        },
    };
    let body = body.trim();
    (body.starts_with('<') && body.ends_with('>')).then(|| Reply::Html(body.to_string()))
}

impl DesignCanvas {
    /// Send an ask about the one selected layer as a quick edit. False when it can't go
    /// that way, so it goes to a session.
    pub(super) fn quick_edit(&mut self, task: &str, cx: &mut Context<Self>) -> bool {
        if self.quick.is_some() || !self.also.is_empty() || self.ai.is_some() {
            return false;
        }
        let (Some(id), Some(el)) = (self.screen.clone(), self.element.as_ref()) else {
            return false;
        };
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return false;
        };
        let path = el.path.clone();
        let dir = folder(&self.root);
        let tokens = std::fs::read_to_string(dir.join("tokens.css")).unwrap_or_default();
        let sheets = format!(
            "{tokens}\n{}",
            std::fs::read_to_string(dir.join(COMPONENTS)).unwrap_or_default()
        );
        let provider = agent::PROVIDERS
            .get(self.pick.agent)
            .unwrap_or(&agent::PROVIDERS[0]);
        let configured = crate::settings::AppSettings::load()
            .agent(provider.id)
            .binary;
        if !agent::available(provider.bin, &configured) {
            return false;
        }
        let (pid, model) = (provider.id, self.pick.model.clone());
        self.quick = Some(Quick {
            asked: task.to_string(),
            task: summary(task),
            screen: id.clone(),
            path: path.clone(),
        });
        let task = task.to_string();
        let js = dom::call("code", &[json!(path), json!(sheets)]);
        let root = self.root.to_string_lossy().into_owned();
        self.run(
            cx,
            move |r| r.eval(&id, device, &js),
            move |this, out, cx| {
                let code: Option<Code> = out.ok().and_then(|v| serde_json::from_value(v).ok());
                let Some(code) = code.filter(fits) else {
                    this.quick_to_session(None, cx);
                    return;
                };
                let text = prompt(&task, &code, &tokens);
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
                    let _ = this.update(cx, |this, cx| this.quick_answered(answer, cx));
                })
                .detach();
            },
        );
        cx.notify();
        true
    }

    fn quick_answered(&mut self, answer: Result<String, String>, cx: &mut Context<Self>) {
        let Some(q) = self.quick.as_ref() else {
            return;
        };
        match answer.as_deref().ok().and_then(reply) {
            Some(Reply::Html(html)) => {
                let (screen, path) = (q.screen.clone(), q.path.clone());
                self.quick = None;
                if self.screen.as_ref() != Some(&screen) {
                    self.select_screen(Some(screen));
                }
                self.edit(
                    dom::op(&path, "swapTo", &[json!(html)]),
                    After::Path(path),
                    cx,
                );
            }
            Some(Reply::Session(why)) => self.quick_to_session(Some(why), cx),
            None => {
                self.quick = None;
                self.say(match answer {
                    Err(e) if !e.is_empty() => format!("The quick edit didn't work: {e}"),
                    _ => {
                        "The quick edit came back without a layer. Try again, or ask in more words."
                            .into()
                    }
                });
            }
        }
        cx.notify();
    }

    /// Hand a quick edit on to a session, saying why when the agent said.
    fn quick_to_session(&mut self, why: Option<String>, cx: &mut Context<Self>) {
        let Some(q) = self.quick.take() else {
            return;
        };
        if let Some(why) = why.filter(|w| !w.is_empty()) {
            self.say(format!(
                "This needs more than the layer ({why}), so it's going to a session."
            ));
        }
        self.pending_session = Some(q.asked);
        cx.notify();
    }

    /// The working pill over the layer a quick edit is changing.
    pub(super) fn quick_mark(&self) -> Option<AnyElement> {
        let q = self.quick.as_ref()?;
        let s = self.canvas.screen(&q.screen)?;
        let el = self.element.as_ref().filter(|e| e.path == q.path)?;
        let (x, y) = self.screen_pt((s.x + el.rect.x, s.y + el.rect.y));
        Some(
            div()
                .absolute()
                .left(px(x))
                .top(px(y - 28.))
                .h(px(22.))
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .rounded_full()
                .bg(rgb(c_blue()))
                .shadow_md()
                .text_size(px(11.))
                .text_color(rgb(0xffffff))
                .child(icon(IconName::Sparkles, 11., 0xffffff))
                .child(format!("{}…", q.task))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn replies_are_read_from_their_html_block() {
        assert_eq!(
            reply("Sure:\n```html\n<button class=\"btn\">Go</button>\n```\nDone."),
            Some(Reply::Html("<button class=\"btn\">Go</button>".into()))
        );
        assert_eq!(
            reply("<div>bare</div>"),
            Some(Reply::Html("<div>bare</div>".into()))
        );
        assert_eq!(
            reply("```\n<p>x</p>\n```"),
            Some(Reply::Html("<p>x</p>".into()))
        );
        assert_eq!(
            reply("SESSION: it needs a new screen\nmore"),
            Some(Reply::Session("it needs a new screen".into()))
        );
        assert_eq!(reply("I can't do that."), None);
    }

    #[test]
    fn big_or_cut_layers_go_to_a_session() {
        let code = |html: &str| Code {
            html: html.into(),
            ..Default::default()
        };
        assert!(fits(&code("<a>x</a>")));
        assert!(!fits(&code("")));
        assert!(!fits(&code("<div>…")));
        assert!(!fits(&code(&"x".repeat(MOST + 1))));
    }
}
