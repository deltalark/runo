//! Switching a session to another agent: a confirm dialog that says what carries over
//! and what it costs, and a compact handoff brief so the new agent picks up the work.

use super::*;

/// Characters kept from each part of the conversation in the brief.
const TASK_CHARS: usize = 2000;
const ASK_CHARS: usize = 600;
const REPLY_CHARS: usize = 900;
const PLAN_CHARS: usize = 3000;
/// Exchanges quoted in full; older requests are listed one line each.
const RECENT_EXCHANGES: usize = 5;

/// A switch waiting for the person to confirm it.
pub(crate) struct SwitchPlan {
    pub session: u64,
    pub to: usize,
    pub brief: String,
    pub files: Vec<git::Change>,
    pub exchanges: usize,
}

/// What the brief is built from.
pub(crate) struct BriefInput<'a> {
    pub from: &'a str,
    pub dir: &'a str,
    pub branch: &'a str,
    pub messages: &'a [Message],
    pub files: &'a [git::Change],
    pub commits: &'a str,
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str(" […]");
    out
}

fn one_line(text: &str, max: usize) -> String {
    clip(text.lines().next().unwrap_or(""), max)
}

/// Rough token count of a text (about four characters a token).
pub(crate) fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Pairs of (request, reply) in order, skipping notices and empty turns.
fn exchanges(messages: &[Message]) -> Vec<(&Message, Option<&Message>)> {
    let mut out: Vec<(&Message, Option<&Message>)> = Vec::new();
    for m in messages {
        match m.role {
            Role::User if !m.text.trim().is_empty() => out.push((m, None)),
            Role::Agent => {
                if let Some(last) = out.last_mut().filter(|(_, r)| r.is_none()) {
                    last.1 = Some(m);
                }
            }
            _ => {}
        }
    }
    out
}

/// The handoff the new agent reads before its first instruction.
pub(crate) fn brief(input: &BriefInput) -> String {
    let all = exchanges(input.messages);
    let mut out = format!(
        "You are taking over a coding task from another agent ({}). It worked in this same folder, so its changes are already on disk.\n",
        input.from
    );
    out.push_str(&format!("Folder: {}\n", input.dir));
    if !input.branch.is_empty() {
        out.push_str(&format!("Branch: {}\n", input.branch));
    }
    if let Some((task, _)) = all.first() {
        out.push_str(&format!(
            "\n## The task\n{}\n",
            clip(&task.text, TASK_CHARS)
        ));
    }
    let split = all.len().saturating_sub(RECENT_EXCHANGES).max(1);
    if all.len() > 1 && split > 1 {
        out.push_str("\n## Earlier requests\n");
        for (ask, _) in &all[1..split] {
            out.push_str(&format!("- {}\n", one_line(&ask.text, 160)));
        }
    }
    let recent: Vec<_> = all
        .iter()
        .enumerate()
        .filter(|(i, _)| *i >= split || *i == 0)
        .collect();
    if !recent.is_empty() {
        out.push_str("\n## Recent conversation\n");
        for (i, (ask, reply)) in recent {
            if i > 0 {
                out.push_str(&format!("\nMe: {}\n", clip(&ask.text, ASK_CHARS)));
            }
            if let Some(reply) = reply {
                if !reply.steps.is_empty() {
                    out.push_str(&format!("{} did: {}\n", input.from, reply.steps.join("; ")));
                }
                if !reply.text.trim().is_empty() {
                    out.push_str(&format!(
                        "{}: {}\n",
                        input.from,
                        clip(&reply.text, REPLY_CHARS)
                    ));
                }
            }
        }
    }
    let last_agent = input.messages.iter().rev().find(|m| m.role == Role::Agent);
    if let Some(plan) = input
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Agent && !m.plan.trim().is_empty())
        .map(|m| m.plan.as_str())
    {
        out.push_str(&format!("\n## Latest plan\n{}\n", clip(plan, PLAN_CHARS)));
    }
    if let Some(last) = last_agent {
        let mut open = Vec::new();
        if !last.denied.is_empty() {
            open.push(format!(
                "The previous agent was not allowed to use: {}.",
                last.denied.join(", ")
            ));
        }
        for q in &last.questions {
            open.push(format!(
                "It asked, still unanswered: {} (options: {})",
                q.question,
                q.options.join(" / ")
            ));
        }
        if !open.is_empty() {
            out.push_str("\n## Open items\n");
            for item in open {
                out.push_str(&format!("- {item}\n"));
            }
        }
    }
    out.push_str("\n## Code state\n");
    if input.files.is_empty() {
        out.push_str("No uncommitted changes.\n");
    } else {
        out.push_str("Uncommitted changes:\n");
        for f in input.files.iter().take(40) {
            out.push_str(&format!(
                "- {} {} (+{} −{})\n",
                f.status, f.path, f.added, f.removed
            ));
        }
        if input.files.len() > 40 {
            out.push_str(&format!("- …and {} more\n", input.files.len() - 40));
        }
    }
    if !input.commits.trim().is_empty() {
        out.push_str(&format!("Recent commits:\n{}\n", input.commits.trim()));
    }
    out.push_str(
        "\nThe files on disk are the source of truth: read the ones above before editing, keep what is finished, and do not redo work.",
    );
    out
}

/// The agent that wrote message `index`: the one before the next switch, if any.
pub(crate) fn provider_at(session: &Session, index: usize) -> &'static agent::Provider {
    let from = session.messages[index..]
        .iter()
        .filter(|m| m.role == Role::System)
        .find_map(|m| parse_divider(&m.meta).map(|(from, ..)| from));
    agent::provider(from.unwrap_or(&session.provider))
}

/// Where Runo keeps a session's memory, inside its folder.
pub(crate) const MEMORY_FILE: &str = ".runo/memory.md";

/// The memory file's text: the same summary as a handoff, for any agent to read.
pub(crate) fn memory_text(brief: &str) -> String {
    let body = brief
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(brief);
    format!(
        "# Session memory\nRuno updates this file after every turn. It summarizes the conversation so far and the state of the code.\n\n{}\n",
        body.trim()
    )
}

/// Rendered in the chat where a session switched agents.
pub(crate) fn divider_meta(from: &str, to: &str, carried: bool, tokens: usize) -> String {
    format!(
        "switch:{from}:{to}:{}:{tokens}",
        if carried { "carry" } else { "fresh" }
    )
}

pub(crate) fn parse_divider(meta: &str) -> Option<(&str, &str, bool, usize)> {
    let mut parts = meta.strip_prefix("switch:")?.split(':');
    Some((
        parts.next()?,
        parts.next()?,
        parts.next()? == "carry",
        parts.next()?.parse().ok()?,
    ))
}

impl RunoApp {
    /// Picking another agent in the composer: switch at once in an empty session,
    /// otherwise ask first.
    pub(crate) fn request_switch(&mut self, id: u64, to: usize) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if session.provider == agent::PROVIDERS[to].id {
            return;
        }
        if self.handles.contains_key(&Target::Session(id)) {
            self.notice = Some(
                "The agent is still working. Stop it or let it finish before switching agents."
                    .into(),
            );
            return;
        }
        if exchanges(&session.messages).is_empty() {
            if let Some(s) = self.state.session_mut(id) {
                s.switch_provider(agent::PROVIDERS[to].id);
            }
            self.save();
            return;
        }
        let from = agent::provider(&session.provider).name;
        let (files, commits) = if !self.demo && git::is_repo(&session.dir) {
            (
                git::changes(&session.dir),
                git::recent_commits(&session.dir, 5),
            )
        } else {
            (Vec::new(), String::new())
        };
        let brief = brief(&BriefInput {
            from,
            dir: &session.dir,
            branch: &session.branch,
            messages: &session.messages,
            files: &files,
            commits: &commits,
        });
        self.switch_preview = false;
        self.switch_plan = Some(SwitchPlan {
            session: id,
            to,
            brief,
            files,
            exchanges: exchanges(&session.messages).len(),
        });
    }

    /// Write the session's memory file after a turn, so any agent can catch up from it
    /// instead of replaying the conversation.
    pub(crate) fn write_memory(&self, id: u64) {
        if self.demo {
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        if exchanges(&session.messages).is_empty() || !git::is_repo(&session.dir) {
            return;
        }
        let files = git::changes(&session.dir);
        let files: Vec<git::Change> = files
            .into_iter()
            .filter(|f| !f.path.starts_with(".runo/"))
            .collect();
        let text = memory_text(&brief(&BriefInput {
            from: agent::provider(&session.provider).name,
            dir: &session.dir,
            branch: &session.branch,
            messages: &session.messages,
            files: &files,
            commits: &git::recent_commits(&session.dir, 5),
        }));
        let path = std::path::Path::new(&session.dir).join(MEMORY_FILE);
        git::exclude(&session.dir, "/.runo/");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }

    pub(crate) fn confirm_switch(&mut self, carry: bool) {
        let Some(plan) = self.switch_plan.take() else {
            return;
        };
        let to = &agent::PROVIDERS[plan.to];
        let Some(session) = self.state.session_mut(plan.session) else {
            return;
        };
        let from = session.provider.clone();
        session.switch_provider(to.id);
        let cost = if carry { tokens(&plan.brief) } else { 0 };
        if carry {
            session.handoff = plan.brief.clone();
            session.carried = plan.brief;
        } else {
            session.handoff.clear();
            session.carried.clear();
        }
        let mut divider = Message::now(Role::System, "");
        divider.meta = divider_meta(&from, to.id, carry, cost);
        session.messages.push(divider);
        session.conversation_start = session.messages.len();
        let id = plan.session;
        self.state.log(
            id,
            format!(
                "Switched to {}{}",
                to.name,
                if carry {
                    " with context"
                } else {
                    ", fresh start"
                }
            ),
        );
        self.save();
        self.info = Some((
            if carry {
                format!(
                    "{} takes over. Your next message carries the context.",
                    to.name
                )
            } else {
                format!("{} starts fresh in the same folder.", to.name)
            },
            std::time::Instant::now(),
        ));
    }

    /// The divider in the chat where the agent changed.
    pub(crate) fn switch_divider(&self, index: usize, meta: &str) -> Option<AnyElement> {
        let (from, to, carried, cost) = parse_divider(meta)?;
        let (from, to) = (agent::provider(from), agent::provider(to));
        let line = || div().flex_1().h(px(1.)).bg(rgb(c_border()));
        let label = if carried {
            format!("context carried over · ~{} tokens", short_count(cost))
        } else {
            "fresh start, same folder".to_string()
        };
        Some(
            rise(
                div()
                    .id(("switch-div", index))
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_1()
                    .child(line())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py(px(5.))
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .bg(rgb(c_raised()))
                            .text_size(px(12.))
                            .child(self.provider_badge(from, 16.).opacity(0.7))
                            .child(icon(IconName::ArrowRight, 12., c_muted()))
                            .child(self.provider_badge(to, 16.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("Switched to {}", to.name)),
                            )
                            .child(div().text_color(rgb(c_muted())).child(label)),
                    )
                    .child(line()),
                SharedString::from(format!("switch-div-rise-{index}")),
                420,
            )
            .into_any_element(),
        )
    }

    /// Confirm dialog for switching agents.
    pub(crate) fn switch_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let plan = self.switch_plan.as_ref()?;
        let session = self.state.session(plan.session)?;
        let from = agent::provider(&session.provider);
        let to = &agent::PROVIDERS[plan.to];
        let cost = tokens(&plan.brief);
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let row = |glyph: IconName, color: u32, text: String| {
            div()
                .flex()
                .items_start()
                .gap_2()
                .text_size(px(12.5))
                .child(div().pt(px(2.)).child(icon(glyph, 13., color)))
                .child(div().flex_1().min_w_0().child(text))
        };
        let files = match plan.files.len() {
            0 => "No uncommitted changes".to_string(),
            1 => "1 changed file, untouched".to_string(),
            n => format!("{n} changed files, untouched"),
        };
        let place = if session.branch.is_empty() {
            "The same folder".to_string()
        } else {
            format!("The same folder and branch ({})", session.branch)
        };
        let badge = |p: &agent::Provider, key: &'static str, dx: f32| {
            div()
                .v_flex()
                .items_center()
                .gap_1()
                .child(self.provider_badge(p, 44.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(p.name),
                )
                .with_animation(
                    key,
                    Animation::new(Duration::from_millis(420)).with_easing(ease_out_quint()),
                    move |el, d| el.opacity(d).left(px(dx * (1. - d))),
                )
        };
        let arrow = div().flex().gap_1().children((0..3i32).map(|i| {
            div()
                .size(px(5.))
                .rounded_full()
                .bg(rgb(c_blue()))
                .with_animation(
                    ("switch-dot", i as usize),
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    move |el, d| {
                        let phase = (d * 3. - i as f32).rem_euclid(3.);
                        let on = if phase < 1. {
                            1. - (phase - 0.5).abs() * 2.
                        } else {
                            0.
                        };
                        el.opacity(0.25 + 0.75 * on.max(0.))
                    },
                )
        }));
        let card = div()
            .id("switch-card")
            .w(px(520.))
            .max_w(relative(0.94))
            .v_flex()
            .gap_4()
            .p_6()
            .rounded_2xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_6()
                    .py_2()
                    .child(badge(from, "switch-from", 24.))
                    .child(arrow)
                    .child(badge(to, "switch-to", -24.)),
            )
            .child(
                div()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child(format!("Switch this session to {}?", to.name)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child("Nothing is sent until your next message."),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_xl()
                    .bg(rgb(c_subtle()))
                    .child(row(IconName::Check, c_green(), {
                        let mut parts = vec![format!(
                            "a summary of {} exchange{}",
                            plan.exchanges,
                            if plan.exchanges == 1 { "" } else { "s" }
                        )];
                        if plan.brief.contains("## Latest plan") {
                            parts.push("the latest plan".into());
                        }
                        if plan.brief.contains("## Open items") {
                            parts.push("open questions and blocked tools".into());
                        }
                        format!("Carries over: {}", alerts::and_list(&parts))
                    }))
                    .child(row(IconName::Check, c_green(), format!("{place}, {files}")))
                    .child(row(
                        IconName::RotateCcw,
                        c_amber(),
                        format!(
                            "Starts fresh: {}'s own memory, and the model and mode settings",
                            from.name
                        ),
                    ))
                    .child(row(
                        IconName::Coins,
                        c_secondary(),
                        format!(
                            "Cost: about {} input tokens, once, on your next message",
                            short_count(cost)
                        ),
                    )),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .id("switch-preview")
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(c_secondary()))
                            .hover(|el| el.text_color(rgb(c_text())))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switch_preview = !this.switch_preview;
                                cx.notify();
                            }))
                            .child(icon(
                                if self.switch_preview {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                },
                                12.,
                                c_secondary(),
                            ))
                            .child("See exactly what is sent"),
                    )
                    .when(self.switch_preview, |el| {
                        el.child(rise(
                            div()
                                .id("switch-brief")
                                .max_h(px(220.))
                                .overflow_y_scroll()
                                .p_3()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(c_border()))
                                .bg(rgb(c_term_bg()))
                                .font_family(self.mono_font.clone())
                                .text_size(px(11.5))
                                .text_color(rgb(c_secondary()))
                                .whitespace_normal()
                                .child(plan.brief.clone()),
                            "switch-brief-rise",
                            200,
                        ))
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("switch-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switch_plan = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("switch-fresh", "Start fresh", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_switch(false);
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("switch-carry", "Switch and carry over", true)
                            .child(icon(IconName::ArrowRight, 13., c_canvas()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_switch(true);
                                cx.notify();
                            })),
                    ),
            );
        Some(
            fade(
                div()
                    .id("switch-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(backdrop)
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.switch_plan = None;
                        cx.notify();
                    }))
                    .child(rise(card, "switch-card-rise", 320)),
                "switch-backdrop-fade",
                200,
            )
            .into_any_element(),
        )
    }
}

/// 1234 → "1.2k".
pub(crate) fn short_count(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn briefs_carry_the_task_recent_turns_plan_and_code_state() {
        let mut messages = vec![Message::new(Role::User, "Add a login page")];
        let mut reply = Message::new(Role::Agent, "Added the form.");
        reply.steps = vec!["Edited 2 files".into()];
        messages.push(reply);
        for i in 0..7 {
            messages.push(Message::new(Role::User, format!("Follow-up {i}")));
            messages.push(Message::new(Role::Agent, format!("Done {i}")));
        }
        let mut last = Message::new(Role::Agent, "");
        last.plan = "1. Add tests".into();
        last.denied = vec!["Bash".into()];
        messages.push(Message::new(Role::User, "Plan the tests"));
        messages.push(last);
        let files = vec![git::Change {
            path: "src/login.rs".into(),
            status: "M".into(),
            added: 10,
            removed: 2,
        }];
        let text = brief(&BriefInput {
            from: "Claude Code",
            dir: "/repo",
            branch: "runo/session-1",
            messages: &messages,
            files: &files,
            commits: "abc123 Add form",
        });
        assert!(text.contains("## The task\nAdd a login page"));
        assert!(text.contains("## Earlier requests\n- Follow-up 0"));
        assert!(text.contains("Me: Follow-up 6"));
        assert!(text.contains("Claude Code did: Edited 2 files"));
        assert!(text.contains("## Latest plan\n1. Add tests"));
        assert!(text.contains("not allowed to use: Bash"));
        assert!(text.contains("- M src/login.rs (+10 −2)"));
        assert!(text.contains("abc123 Add form"));
        assert!(tokens(&text) < 1000);
        let meta = divider_meta("claude", "codex", true, 812);
        assert_eq!(parse_divider(&meta), Some(("claude", "codex", true, 812)));
        assert_eq!(short_count(1234), "1.2k");
        let memory = memory_text(&text);
        assert!(memory.starts_with("# Session memory\n"));
        assert!(!memory.contains("You are taking over"));
        assert!(memory.contains("## The task"));
    }
}
