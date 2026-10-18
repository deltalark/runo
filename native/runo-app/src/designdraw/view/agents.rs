//! Agents using Design's own actions: a call to one of Runo's design tools lands here, on
//! the canvas open for the agent's project, and runs as the same operation a click runs.
//! It goes on the same undo history as the person's edits, marked as AI's, so any one
//! step can be undone, and it keeps ids and locked layers as a click does.

use super::*;
use crate::designdraw::agentops::{self, Action, Request};
use crate::designdraw::styles;

impl DesignCanvas {
    /// Take agents' calls every moment, whether or not the canvas is on screen: the agent
    /// waits for each answer.
    pub(super) fn serve_agents(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            if this
                .update(cx, |this, cx| this.take_agent_calls(cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    fn take_agent_calls(&mut self, cx: &mut Context<Self>) {
        for r in agentops::take(&agentops::folder(), &self.root) {
            self.agent_call(r, cx);
        }
    }

    fn agent_call(&mut self, req: Request, cx: &mut Context<Self>) {
        let base = agentops::folder();
        let reply = move |ok: bool, text: String| agentops::answer(&base, &req.id, ok, text);
        let text_styles: Vec<String> = styles::all(&self.css())
            .into_iter()
            .filter(|s| !s.class.is_empty())
            .map(|s| s.class)
            .collect();
        let call = match agentops::parse(&req.tool, &req.input, &text_styles) {
            Ok(c) => c,
            Err(e) => return reply(false, e),
        };
        if call.action == Action::Selection {
            return self.answer_selection(reply, cx);
        }
        let Some(device) = self.canvas.screen(&call.screen).map(|s| s.device) else {
            let ids: Vec<&str> = self.canvas.screens.iter().map(|s| s.id.as_str()).collect();
            return reply(
                false,
                format!(
                    "There's no screen \"{}\". The screens are: {}.",
                    call.screen,
                    ids.join(", ")
                ),
            );
        };
        let library = self.library.clone();
        let (id, sel, action) = (call.screen.clone(), call.selector.clone(), call.action);
        let find = dom::call("find", &[json!(sel)]);
        self.run(
            cx,
            move |r| -> Result<Option<(String, Shot, u64)>, String> {
                let found = r.eval(&id, device, &find)?;
                if let Some(e) = found["error"].as_str() {
                    return Err(e.to_string());
                }
                let js = match &action {
                    Action::Op(name, args) => {
                        dom::call("agent", &[json!(sel), json!(name), json!(args)])
                    }
                    Action::Swap(variant) => {
                        let htmls: Vec<Option<String>> = found["components"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|c| {
                                let c = library.iter().find(|l| Some(l.id.as_str()) == c[0].as_str())?;
                                c.variant(variant).map(|v| v.html.clone())
                            })
                            .collect();
                        if htmls.iter().all(Option::is_none) {
                            return Err(format!(
                                "None of the layers matching \"{sel}\" is an instance of a component with a variant \"{variant}\"."
                            ));
                        }
                        dom::call("agentSwap", &[json!(sel), json!(htmls)])
                    }
                    Action::Selection => unreachable!(),
                };
                Ok(r.edit_giving(&id, device, &js)?
                    .map(|(before, shot, n)| (before, shot, n.as_u64().unwrap_or(0))))
            },
            move |this, out, cx| match out {
                Ok(Some((before, shot, n))) => {
                    this.landed(call.screen.clone(), before, shot, After::Same, cx);
                    if let Some(step) = this.undo.last_mut() {
                        step.by_ai = Some(call.what.clone());
                    }
                    if let Some(ai) = &mut this.ai {
                        ai.changed = true;
                    }
                    cx.notify();
                    reply(
                        true,
                        format!(
                            "Done: {} on {n} layer{} of {}. It's on the person's undo history.",
                            call.what,
                            if n == 1 { "" } else { "s" },
                            call.screen
                        ),
                    );
                }
                Ok(None) => reply(true, "Nothing changed: the layers already were that way.".into()),
                Err(e) => reply(false, e),
            },
        );
    }

    /// Say what the person has selected, with a selector for each layer.
    fn answer_selection(
        &mut self,
        reply: impl FnOnce(bool, String) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.screen.clone() else {
            return reply(true, "Nothing is selected in Design.".into());
        };
        let Some(s) = self.canvas.screen(&id) else {
            return reply(true, "Nothing is selected in Design.".into());
        };
        let (name, device) = (s.name.clone(), s.device);
        let els: Vec<Element> = self.element.iter().chain(&self.also).cloned().collect();
        if els.is_empty() {
            return reply(
                true,
                format!("The screen \"{name}\" (id {id}) is selected, with no layer in it."),
            );
        }
        let paths: Vec<dom::Path> = els.iter().map(|e| e.path.clone()).collect();
        let js = dom::call("selectors", &[json!(paths)]);
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.eval(&id2, device, &js),
            move |_, out, _| {
                let sels: Vec<String> = out
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let mut text =
                    format!("On the screen \"{name}\" (id {id}), the person has selected:\n");
                for (e, sel) in els.iter().zip(sels) {
                    let what = match &e.text {
                        Some(t) if !t.is_empty() => {
                            format!("{} \"{}\"", e.label, t.chars().take(60).collect::<String>())
                        }
                        _ => e.label.clone(),
                    };
                    text.push_str(&format!("- {what} <{}>: {sel}\n", e.tag));
                }
                reply(true, text);
            },
        );
    }
}
