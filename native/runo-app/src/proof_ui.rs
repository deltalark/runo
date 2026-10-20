//! Proof of done in the app: checks after each turn, Fix it, fixing until green, the
//! second opinion from another vendor's agent, and the card that shows the verdict.

use super::*;
use crate::proof::{self, Proof};

/// Rounds offered for fixing failed checks on their own; 0 asks first.
const FIX_ROUNDS: [u32; 5] = [0, 1, 2, 3, 5];

/// Index of the session's last message from the person: the turn being proved.
fn last_turn(session: &Session) -> Option<usize> {
    session.messages.iter().rposition(|m| m.role == Role::User)
}

impl RunoApp {
    /// Run the project's checks on the session's last turn. `auto` is set when a turn
    /// just finished, so failures may go back to the agent on their own.
    pub(crate) fn prove_turn(&mut self, id: u64, auto: bool, cx: &mut Context<Self>) {
        if self.demo || self.proving.contains(&id) {
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let Some(turn) = last_turn(session) else {
            return;
        };
        let checks = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .map(|p| proof::commands(&p.checks))
            .unwrap_or_default();
        if checks.is_empty() {
            return;
        }
        let dir = session.dir.clone();
        let remote = session.connection != 0;
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        // Earlier rounds count only while the same turn's fixes go on.
        let (rounds, last_failure) = match session.proof.take() {
            Some(p) if auto => (p.rounds, p.last_failure),
            _ => (0, String::new()),
        };
        if remote {
            session.proof = Some(Proof {
                turn,
                rounds,
                last_failure,
                skipped: "Checks run on this computer, and this session runs on a connection."
                    .into(),
                ..Default::default()
            });
            self.save();
            return;
        }
        session.proof = Some(Proof {
            turn,
            rounds,
            last_failure,
            ..Default::default()
        });
        self.proving.insert(id);
        crate::applog::info(
            "proof",
            format!("session #{id}: running {} check(s)", checks.len()),
        );
        self.bg(
            move || proof::run_all(&dir, &checks, proof::CHECK_LIMIT),
            move |this, results, cx| this.proof_done(id, turn, auto, results, cx),
        );
        cx.notify();
    }

    fn proof_done(
        &mut self,
        id: u64,
        turn: usize,
        auto: bool,
        results: Vec<proof::CheckRun>,
        cx: &mut Context<Self>,
    ) {
        self.proving.remove(&id);
        let busy = self.handles.contains_key(&Target::Session(id));
        let rounds_allowed = self
            .state
            .session(id)
            .and_then(|s| self.state.projects.iter().find(|p| p.id == s.project_id))
            .map(|p| p.fix_rounds)
            .unwrap_or(0);
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        // A newer turn started meanwhile: these results are about older code.
        if last_turn(session) != Some(turn) {
            return;
        }
        let mut verdict = session.proof.take().unwrap_or_default();
        verdict.turn = turn;
        verdict.results = results;
        let key = proof::failure_key(&verdict.results);
        let passed = verdict.passed();
        let failed = verdict.failed().len();
        let mut fix = None;
        if passed {
            verdict.rounds = 0;
            verdict.last_failure.clear();
        } else if auto
            && !busy
            && session.queue.is_empty()
            && verdict.rounds < rounds_allowed
            && key != verdict.last_failure
        {
            verdict.rounds += 1;
            fix = Some(proof::fix_prompt(&verdict.results));
        }
        if !passed {
            verdict.last_failure = key;
        }
        let round = verdict.rounds;
        let provider = session.provider.clone();
        let title = session.title.clone();
        session.proof = Some(verdict);
        self.state.log(
            id,
            if passed {
                format!("Checks passed: {title}")
            } else {
                format!("Checks failed ({failed}): {title}")
            },
        );
        crate::applog::info(
            "proof",
            format!(
                "session #{id}: checks {}",
                if passed { "passed" } else { "failed" }
            ),
        );
        self.save();
        if let Some(prompt) = fix {
            self.info = Some((
                format!("Checks failed. Sent them back to the agent (round {round})."),
                std::time::Instant::now(),
            ));
            let opts = cx.global::<AppSettings>().agent(&provider);
            self.start_turn(id, prompt, Vec::new(), &opts);
        }
        cx.notify();
    }

    /// Send the failing checks to the agent as the next prompt.
    pub(crate) fn fix_checks(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let Some(p) = session.proof.as_ref().filter(|p| !p.passed()) else {
            return;
        };
        let prompt = proof::fix_prompt(&p.results);
        let opts = cx.global::<AppSettings>().agent(&session.provider);
        self.start_turn(id, prompt, Vec::new(), &opts);
        cx.notify();
    }

    /// Ask an agent from another vendor to review the last turn's change, read-only.
    /// Its findings land as diff notes, ready for Send notes.
    pub(crate) fn second_opinion(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.reviewing.contains_key(&id) || self.demo_guard(cx) {
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        let settings = cx.global::<AppSettings>();
        let Some(reviewer) = proof::pick_reviewer(&session.provider, |p| {
            agent::available(agent::provider(p).bin, &settings.agent(p).binary)
        }) else {
            self.fail(
                "proof",
                "A second opinion needs another agent installed, such as Codex or Claude Code. Add one in Settings → Providers."
                    .into(),
            );
            return;
        };
        let turn = last_turn(session).unwrap_or(0);
        let task = session
            .messages
            .get(turn)
            .map(|m| m.text.clone())
            .unwrap_or_default();
        let checkpoint = session
            .messages
            .get(turn)
            .map(|m| m.checkpoint.clone())
            .unwrap_or_default();
        let dir = session.dir.clone();
        let binary = settings.agent(reviewer).binary;
        self.reviewing.insert(id, reviewer);
        self.bg(
            move || {
                if checkpoint.is_empty() {
                    git::patch_for_message(&dir, 40_000)
                } else {
                    git::patch_since(&dir, &checkpoint, 40_000)
                }
            },
            move |this, patch, cx| {
                let Some(dir) = this.state.session(id).map(|s| s.dir.clone()) else {
                    this.reviewing.remove(&id);
                    return;
                };
                if patch.trim().is_empty() {
                    this.reviewing.remove(&id);
                    this.fail(
                        "proof",
                        "There are no changes from this turn to review.".into(),
                    );
                    cx.notify();
                    return;
                }
                let prompt = proof::review_prompt(&task, &patch);
                let command = agent::ask_command(reviewer, &binary, &prompt);
                agent::oneshot_within(
                    format!("proof-review:{id}:{turn}"),
                    command,
                    &dir,
                    proof::REVIEW_LIMIT,
                    this.tx.clone(),
                );
                cx.notify();
            },
        );
        cx.notify();
    }

    /// A second opinion came back. True when `key` was one.
    pub(crate) fn handle_review(&mut self, key: &str, result: Result<String, String>) -> bool {
        let Some(rest) = key.strip_prefix("proof-review:") else {
            return false;
        };
        let mut parts = rest.split(':');
        let id: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let turn: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let reviewer = self.reviewing.remove(&id).unwrap_or("codex");
        let name = agent::provider(reviewer).name;
        let Some(session) = self.state.session_mut(id) else {
            return true;
        };
        let summary = match result {
            Ok(reply) => {
                let findings = proof::parse_review(&reply);
                let said_none = findings.is_empty() && reply.contains("NO NOTES");
                let count = findings.len();
                for f in findings {
                    // The note shows the line as it reads now, like a note left by hand.
                    let code =
                        std::fs::read_to_string(std::path::Path::new(&session.dir).join(&f.path))
                            .ok()
                            .and_then(|text| {
                                text.lines()
                                    .nth(f.line.saturating_sub(1))
                                    .map(|l| format!(" {l}"))
                            })
                            .unwrap_or_default();
                    session.notes.push(Note {
                        path: f.path,
                        line: f.line,
                        code,
                        text: format!("{name}: {}", f.text),
                    });
                }
                if count == 0 && !said_none {
                    crate::applog::warn(
                        "proof",
                        format!(
                            "session #{id}: {name}'s review had no notes in the asked form: {}",
                            crate::applog::gist(&reply)
                        ),
                    );
                }
                proof::review_summary(name, count, said_none)
            }
            Err(error) => {
                let first = error.lines().next().unwrap_or("").to_string();
                crate::applog::warn(
                    "proof",
                    format!("session #{id}: {name}'s review failed: {first}"),
                );
                format!("{name} couldn't give a second opinion: {first}")
            }
        };
        let p = session.proof.get_or_insert_with(|| Proof {
            turn,
            ..Default::default()
        });
        p.review = summary;
        self.save();
        true
    }

    /// Open the project's popup at its checks, filled with suggestions when it has none.
    pub(crate) fn set_up_checks(&mut self, pid: u64, cx: &mut Context<Self>) {
        self.edit_project(pid);
        let path = self
            .state
            .projects
            .iter()
            .find(|p| p.id == pid)
            .filter(|p| p.checks.trim().is_empty())
            .map(|p| std::path::PathBuf::from(&p.path));
        if let (Some(path), Some(form)) = (path, self.project_form.as_mut()) {
            let found = proof::detect(&path);
            if !found.is_empty() {
                form.sync_checks = Some(found.join("\n"));
            }
        }
        cx.notify();
    }

    /// The checks field of the project popup, with suggestions and the fix rounds.
    pub(crate) fn checks_field(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = self.project_form.as_ref() else {
            return div().into_any_element();
        };
        let rounds = form.fix_rounds;
        let path = form.path.clone();
        div()
            .v_flex()
            .gap_1p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_secondary()))
                            .child("Checks after each turn"),
                    )
                    .when_some(path, |el, path| {
                        el.child(
                            self.button("project-checks-suggest", "Suggest", false)
                                .h(px(22.))
                                .px_2()
                                .text_size(px(11.5))
                                .child(icon(IconName::Sparkles, 11., c_secondary()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let found = proof::detect(&path);
                                    if let Some(form) = this.project_form.as_mut() {
                                        form.sync_checks = Some(if found.is_empty() {
                                            String::new()
                                        } else {
                                            found.join("\n")
                                        });
                                    }
                                    if found.is_empty() {
                                        this.info = Some((
                                            "Runo found no test, lint or build commands to suggest."
                                                .into(),
                                            std::time::Instant::now(),
                                        ));
                                    }
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(Textarea::new(&self.project_checks).bordered(true).h(px(72.)))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("One command per line, such as tests, lint and build. They run in the session's folder after every turn that changes files, and a failure can go straight back to the agent."),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_secondary()))
                            .mr_1()
                            .child("Fix failures on their own"),
                    )
                    .children(FIX_ROUNDS.iter().enumerate().map(|(i, n)| {
                        let n = *n;
                        let label = match n {
                            0 => "Off".to_string(),
                            1 => "Once".to_string(),
                            n => format!("{n}×"),
                        };
                        self.chip(("project-fix-rounds", i), label, rounds == n)
                            .h(px(24.))
                            .text_size(px(11.5))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(form) = this.project_form.as_mut() {
                                    form.fix_rounds = n;
                                }
                                cx.notify();
                            }))
                    })),
            )
            .into_any_element()
    }

    /// The verdict on the last turn, under the agent's reply: checks, Fix it, and the
    /// second opinion. Offers to set up checks when a turn changed files without any.
    pub(crate) fn proof_card(&self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.current()?;
        if !session.canvas.is_empty() {
            return None;
        }
        let id = session.id;
        let turn = last_turn(session)?;
        if index < turn {
            return None;
        }
        let proving = self.proving.contains(&id);
        let reviewer = self.reviewing.get(&id).copied();
        let verdict = session.proof.as_ref().filter(|p| p.turn == turn);
        let changed = self.turn_files.get(&id).copied().unwrap_or(0) > 0;
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)?;
        let has_checks = !proof::commands(&project.checks).is_empty();
        if verdict.is_none() && !proving && reviewer.is_none() && !changed {
            return None;
        }
        let pid = project.id;

        let small = |this: &Self, key: &'static str, label: &'static str, primary: bool| {
            this.button((key, index), label, primary)
                .h(px(24.))
                .px_2()
                .text_size(px(12.))
        };

        let (tone, glyph, headline) = match verdict {
            _ if proving => (
                c_secondary(),
                IconName::LoaderCircle,
                "Running the checks…".to_string(),
            ),
            Some(v) if !v.skipped.is_empty() => (c_muted(), IconName::Info, v.skipped.clone()),
            Some(v) if v.passed() => (
                c_green(),
                IconName::CircleCheck,
                match v.results.len() {
                    1 => "The check passed".to_string(),
                    n => format!("All {n} checks passed"),
                },
            ),
            Some(v) if !v.results.is_empty() => {
                let failed = v.failed().len();
                (
                    c_red(),
                    IconName::CircleX,
                    if v.results.len() == 1 {
                        "The check failed".to_string()
                    } else {
                        format!("{failed} of {} checks failed", v.results.len())
                    },
                )
            }
            _ if has_checks => (
                c_muted(),
                IconName::ListChecks,
                "Checks haven't run on this turn".to_string(),
            ),
            _ => (
                c_muted(),
                IconName::ListChecks,
                "Prove this turn: set up checks, or ask another agent".to_string(),
            ),
        };
        let red = verdict.is_some_and(|v| !v.results.is_empty() && !v.passed());
        let rounds = verdict.map(|v| v.rounds).unwrap_or(0);
        let border = if red {
            c_red()
        } else if tone == c_green() {
            c_green()
        } else {
            c_border()
        };

        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(icon(glyph, 14., tone))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(rgb(if red { c_text() } else { c_secondary() }))
                    .child(headline)
                    .when(red && rounds > 0, |el| {
                        el.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(format!(
                                    "Stopped fixing on its own after {rounds} round{}.",
                                    if rounds == 1 { "" } else { "s" }
                                )),
                        )
                    }),
            )
            .when(!has_checks && !proving, |el| {
                el.child(
                    small(self, "proof-setup", "Set up checks", false)
                        .child(icon(IconName::Settings, 12., c_secondary()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_up_checks(pid, cx);
                        })),
                )
            })
            .when(has_checks && !proving, |el| {
                el.child(
                    small(self, "proof-rerun", "Run again", false)
                        .child(icon(IconName::RotateCw, 12., c_secondary()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.prove_turn(id, false, cx);
                        })),
                )
            })
            .when(reviewer.is_none(), |el| {
                el.child(
                    small(self, "proof-second", "Second opinion", false)
                        .child(icon(IconName::MessagesSquare, 12., c_secondary()))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Another vendor's agent reviews this turn's diff without editing, and its comments land as diff notes",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.second_opinion(id, cx);
                        })),
                )
            })
            .when(red && !proving, |el| {
                el.child(
                    small(self, "proof-fix", "Fix it", true)
                        .child(icon(IconName::Wrench, 12., c_canvas()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.fix_checks(id, cx);
                        })),
                )
            });

        let rows = verdict
            .filter(|_| !proving)
            .map(|v| v.results.clone())
            .unwrap_or_default();
        let list = div()
            .v_flex()
            .gap_1()
            .children(rows.into_iter().enumerate().map(|(i, r)| {
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.))
                            .child(icon(
                                if r.ok { IconName::Check } else { IconName::X },
                                12.,
                                if r.ok { c_green() } else { c_red() },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(self.mono_font.clone())
                                    .text_color(rgb(c_secondary()))
                                    .child(r.command.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(c_muted()))
                                    .child(format!("{}s", r.secs)),
                            ),
                    )
                    .when(!r.ok && !r.tail.is_empty(), |el| {
                        let shown: Vec<&str> = r.tail.lines().collect();
                        let start = shown.len().saturating_sub(12);
                        el.child(
                            div()
                                .id(("proof-tail", i))
                                .ml(px(20.))
                                .max_h(px(180.))
                                .overflow_y_scroll()
                                .p_2()
                                .rounded_md()
                                .bg(tint(c_subtle()))
                                .font_family(self.mono_font.clone())
                                .text_size(px(11.5))
                                .text_color(rgb(c_secondary()))
                                .child(shown[start..].join("\n")),
                        )
                    })
            }));

        let review = match (reviewer, verdict.map(|v| v.review.clone())) {
            (Some(r), _) => Some(format!(
                "Asking {} for a second opinion…",
                agent::provider(r).name
            )),
            (None, Some(text)) if !text.is_empty() => Some(text),
            _ => None,
        };
        let notes = session.notes.len();

        Some(
            div()
                .px_3()
                .py_2()
                .v_flex()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(rgb(settings::mix(border, c_canvas(), 0.45)))
                .bg(rgb(settings::mix(border, c_canvas(), 0.06)))
                .text_size(px(12.5))
                .child(header)
                .child(list)
                .when_some(review, |el, text| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .child(icon(
                                if reviewer.is_some() {
                                    IconName::LoaderCircle
                                } else {
                                    IconName::MessagesSquare
                                },
                                12.,
                                c_secondary(),
                            ))
                            .child(div().flex_1().min_w_0().child(text))
                            .when(reviewer.is_none() && notes > 0, |el| {
                                el.child(
                                    small(self, "proof-notes", "See notes", false)
                                        .child(icon(IconName::GitCompare, 12., c_secondary()))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.right_tab = RightTab::Changes;
                                            this.show_panel = true;
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                })
                .into_any_element(),
        )
    }
}
