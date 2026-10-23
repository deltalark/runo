//! Agents using Design's own actions: a call to one of Runo's design tools lands here, on
//! the canvas open for the agent's project, and runs as the same operation a click runs.
//! It goes on the same undo history as the person's edits, marked as AI's, so any one
//! step can be undone, and it keeps ids and locked layers as a click does.

use super::*;
use crate::designdraw::agentops::{self, Action, Request};
use crate::designdraw::styles;

/// An action that changed its screen: the screen before, how it looks now, and how many
/// layers it changed, where they are now.
type Landed = (String, Shot, (u64, Vec<dom::Box>));

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
        // An ask about the selection changes only the selected layers.
        let scope = self
            .ai
            .as_ref()
            .filter(|ai| ai.busy())
            .and_then(|ai| ai.scope.clone());
        if let Some((screen, _)) = scope.as_ref().filter(|(s, _)| *s != call.screen) {
            return reply(
                false,
                format!(
                    "This ask is about the layers selected on the screen \"{screen}\", so other screens stay as they are."
                ),
            );
        }
        let library = self.library.clone();
        let (id, sel, action) = (call.screen.clone(), call.selector.clone(), call.action);
        let find = dom::call("find", &[json!(sel), json!(scope.map(|(_, p)| p))]);
        self.run(
            cx,
            move |r| -> Result<Option<Landed>, String> {
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
                Ok(r.edit_giving(&id, device, &js)?.map(|(before, shot, done)| {
                    let boxes: Vec<dom::Box> =
                        serde_json::from_value(done["boxes"].clone()).unwrap_or_default();
                    (before, shot, (done["n"].as_u64().unwrap_or(0), boxes))
                }))
            },
            move |this, out, cx| match out {
                Ok(Some((before, shot, (n, boxes)))) => {
                    // The same checks as for a change made by hand: what this one broke
                    // goes back to the agent at once, to fix before it finishes.
                    let broke = this.new_findings(&call.screen, &shot);
                    this.landed(call.screen.clone(), before, shot, After::Same, cx);
                    this.show_ai_touch(call.screen.clone(), boxes, call.what.clone(), cx);
                    let name = this
                        .canvas
                        .screen(&call.screen)
                        .map_or_else(|| call.screen.clone(), |s| s.name.clone());
                    if let Some(step) = this.undo.last_mut() {
                        step.by_ai = Some(call.what.clone());
                        step.lesson = Some(format!("{} on the screen {name}", call.detail));
                    }
                    if let Some(ai) = &mut this.ai {
                        ai.changed = true;
                    }
                    cx.notify();
                    let mut text = format!(
                        "Done: {} on {n} layer{} of {}. It's on the person's undo history.",
                        call.what,
                        if n == 1 { "" } else { "s" },
                        call.screen
                    );
                    if !broke.is_empty() {
                        let name = this
                            .canvas
                            .screen(&call.screen)
                            .map_or_else(|| call.screen.clone(), |s| s.name.clone());
                        text.push_str(
                            "\nDesign's checks find new problems after this change; fix them before you finish:",
                        );
                        text.push_str(&checks::listed(&[(call.screen.clone(), name, broke)]));
                    }
                    reply(true, text);
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

    /// After an ask about the selection, put back the other screens AI changed, and take
    /// away screens it added. Gives their names.
    pub(super) fn keep_scope(&mut self, before: &HashMap<PathBuf, Vec<u8>>) -> Vec<String> {
        let Some((scoped, _)) = self.ai.as_ref().and_then(|ai| ai.scope.clone()) else {
            return Vec::new();
        };
        let dir = folder(&self.root).join("screens");
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            let Some(id) = p
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|_| p.extension().is_some_and(|x| x == "html"))
            else {
                continue;
            };
            if id == scoped {
                continue;
            }
            let name = self
                .canvas
                .screen(id)
                .map_or_else(|| id.to_string(), |s| s.name.clone());
            let put = match before.get(&p) {
                Some(was) if std::fs::read(&p).ok().as_ref() != Some(was) => {
                    std::fs::write(&p, was)
                }
                Some(_) => continue,
                None => std::fs::remove_file(&p),
            };
            if put.is_ok() {
                out.push(name);
            }
        }
        out
    }

    /// Outline for a moment the layers AI just changed, so its work shows as it happens.
    fn show_ai_touch(
        &mut self,
        screen: String,
        boxes: Vec<dom::Box>,
        what: String,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .canvas
            .screen(&screen)
            .map_or_else(|| screen.clone(), |s| s.name.clone());
        self.say(format!("AI {what} on {name}."));
        self.ai_touch = Some((screen, boxes, Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AI_TOUCH).await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .ai_touch
                    .as_ref()
                    .is_some_and(|(_, _, at)| at.elapsed() >= AI_TOUCH)
                {
                    this.ai_touch = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Put one screen back as it was before AI's change, keeping the rest of the change.
    pub(super) fn reject_screen(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(ai) = &self.ai else {
            return;
        };
        let path = screen_path(&self.root, id);
        let name = self
            .canvas
            .screen(id)
            .map_or_else(|| id.to_string(), |s| s.name.clone());
        let put = match ai.before.get(&path) {
            Some(was) => std::fs::write(&path, was),
            None => std::fs::remove_file(&path),
        };
        if let Err(e) = put {
            self.say(format!("Couldn't put {name} back: {e}"));
            cx.notify();
            return;
        }
        // Its steps were made on the HTML that's gone.
        self.undo.retain(|s| s.id != id);
        self.redo.retain(|s| s.id != id);
        self.check_files(cx);
        self.say(format!(
            "Put {name} back as it was. Keep or Undo the rest of AI's change."
        ));
        cx.notify();
    }

    /// The screens AI's change touched, each to put back on its own, under Keep and Undo.
    pub(super) fn ai_screens_row(&self, cx: &mut Context<Self>) -> Option<Div> {
        let changed = self.ai_screens();
        if changed.is_empty() {
            return None;
        }
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .pt_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child("Changed:"),
            );
        for (i, (id, new)) in changed.into_iter().enumerate() {
            let name = self
                .canvas
                .screen(&id)
                .map_or_else(|| id.clone(), |s| s.name.clone());
            let label = if new { format!("{name} (new)") } else { name };
            // A screen that was there before opens before and after, to keep or put back;
            // a new one has no before, so it's taken away.
            let compare = !new && self.ai.as_ref().is_some_and(|ai| ai.version.is_some());
            row = row.child(
                crate::askbar::target_chip(("design-ai-screen", i), IconName::Smartphone, label)
                    .tooltip(tip(if new {
                        "Take this new screen away, keeping the rest of AI's change"
                    } else if compare {
                        "Compare before and after, and put this screen back if it's worse"
                    } else {
                        "Put this screen back, keeping the rest of AI's change"
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if compare {
                            this.compare_ai(id.clone(), cx)
                        } else {
                            this.reject_screen(&id, cx)
                        }
                    })),
            );
        }
        Some(row)
    }

    /// The serious problems Design's checks find on `screen` as `shot` shows it that they
    /// didn't find before.
    fn new_findings(&self, screen: &str, shot: &Shot) -> Vec<checks::Finding> {
        let Some(device) = self.canvas.screen(screen).map(|s| s.device) else {
            return Vec::new();
        };
        let ids: HashSet<String> = self.canvas.screens.iter().map(|s| s.id.clone()).collect();
        let found = |shot: Option<&Shot>| -> Vec<checks::Finding> {
            shot.and_then(|s| s.facts.as_ref())
                .map(|f| checks::review(f, device, &ids))
                .unwrap_or_default()
                .into_iter()
                .filter(|f| f.kind.serious())
                .collect()
        };
        let was = found(self.shots.get(screen));
        found(Some(shot))
            .into_iter()
            .filter(|f| !was.iter().any(|w| w.kind == f.kind && w.text == f.text))
            .collect()
    }

    /// Keep the AI change just undone as a design rule, which every later ask follows.
    pub(super) fn learn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(lesson) = self.lesson.take() else {
            return;
        };
        let file = crate::design::notes_path(&self.root);
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let text = learned(&text, &lesson);
        match crate::architecture::save_notes(&file, &text) {
            Ok(()) => {
                self.load_rules(window, cx);
                self.say("Added to the design rules. AI follows them from the next ask.");
            }
            Err(e) => self.say(format!("Couldn't save the design rules: {e}")),
        }
        cx.notify();
    }
}

const LEARNED: &str = "## Learned from undoing AI";

/// The rules with one more AI change not to repeat, under its own heading at the end.
fn learned(rules: &str, lesson: &str) -> String {
    let line = format!("- Don't repeat this change, which was undone: {lesson}.");
    let mut out = rules.trim_end().to_string();
    if !out.contains(LEARNED) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(LEARNED);
        out.push_str(
            "\nChanges AI made that the person undid. Treat each as a rule unless asked for it.",
        );
    }
    out.push('\n');
    out.push_str(&line);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn undone_changes_become_rules_under_one_heading() {
        let once = learned(
            "# Rules\nWarm and calm.\n",
            "set color: red on .price on the screen Home",
        );
        assert!(once.starts_with("# Rules\nWarm and calm.\n\n## Learned from undoing AI\n"));
        assert!(once.ends_with("set color: red on .price on the screen Home.\n"));
        let twice = learned(&once, "delete .hero on the screen Home");
        assert_eq!(twice.matches(LEARNED).count(), 1);
        assert!(twice.ends_with("delete .hero on the screen Home.\n"));
        assert!(learned("", "x").starts_with(LEARNED));
    }
}
