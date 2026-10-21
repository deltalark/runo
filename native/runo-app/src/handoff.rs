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
    pub recap: Recap,
    /// A model picked for the new agent from the composer's agent menu.
    pub model: Option<String>,
}

/// The outgoing agent's own recap of the session, asked for when the switch is requested.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Recap {
    /// Not asked: the agent cannot be asked, or did no work worth recapping.
    Skipped,
    Asking,
    Added,
    Failed,
}

/// How long the outgoing agent gets to write its recap.
const RECAP_LIMIT: std::time::Duration = std::time::Duration::from_secs(45);
/// Characters of the recap kept in the brief.
const RECAP_CHARS: usize = 1500;
const RECAP_HEADING: &str = "## Recap from ";

/// What the outgoing agent is asked, in its own conversation, when the session switches away.
pub(crate) fn recap_prompt(to: &str) -> String {
    format!(
        "The person is switching this session to another coding agent ({to}). Write a short recap of \
         this conversation so it can continue the work. Under 150 words, plain markdown, no greeting \
         and no title. Use only what happened in this conversation: do not run commands, read files \
         or call tools. Include only what applies: what was done and why, decisions and dead ends \
         worth knowing, files you edited, and the next step you would take."
    )
}

/// Whether the outgoing agent did work worth asking it to recap: it ran tools in its
/// current conversation, and its CLI can answer a read-only follow-up there.
pub(crate) fn wants_recap(session: &Session) -> bool {
    let askable = match session.provider.as_str() {
        "claude" | "qwen" | "opencode" | "droid" => !session.agent_session.is_empty(),
        "codex" => true,
        _ => false,
    };
    askable
        && session.messages[session.conversation_start.min(session.messages.len())..]
            .iter()
            .any(|m| m.role == Role::Agent && !m.steps.is_empty())
}

/// The brief with the outgoing agent's recap added before the code state.
pub(crate) fn with_recap(brief: &str, from: &str, recap: &str) -> String {
    let recap = recap.trim();
    if recap.is_empty() || brief.contains(RECAP_HEADING) {
        return brief.to_string();
    }
    let section = format!("\n{RECAP_HEADING}{from}\n{}\n", clip(recap, RECAP_CHARS));
    match brief.find("\n## Code state") {
        Some(at) => format!("{}{section}{}", &brief[..at], &brief[at..]),
        None => format!("{brief}\n{section}"),
    }
}

/// What the brief is built from.
pub(crate) struct BriefInput<'a> {
    pub from: &'a str,
    pub dir: &'a str,
    pub branch: &'a str,
    pub messages: &'a [Message],
    pub files: &'a [git::Change],
    pub commits: &'a str,
    /// Notes agents saved through the memory server.
    pub notes: &'a str,
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
type Exchange<'a> = (&'a Message, Option<(usize, &'a Message)>);

fn exchanges(messages: &[Message]) -> Vec<Exchange<'_>> {
    let mut out: Vec<Exchange> = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        match m.role {
            Role::User if !m.text.trim().is_empty() => out.push((m, None)),
            Role::Agent => {
                if let Some(last) = out.last_mut().filter(|(_, r)| r.is_none()) {
                    last.1 = Some((i, m));
                }
            }
            _ => {}
        }
    }
    out
}

/// Which agent wrote message `index`: the one a later switch moved away from, else `current`.
fn author<'a>(messages: &[Message], index: usize, current: &'a str) -> &'a str {
    messages[index..]
        .iter()
        .filter(|m| m.role == Role::System)
        .find_map(|m| parse_divider(&m.meta).map(|(from, ..)| agent::provider(from).name))
        .unwrap_or(current)
}

/// Older requests listed by name; the rest are left to search_history.
const EARLIER_LISTED: usize = 12;
/// Tool steps quoted per reply.
const STEPS_LISTED: usize = 12;

/// The handoff the new agent reads before its first instruction.
pub(crate) fn brief(input: &BriefInput) -> String {
    let all = exchanges(input.messages);
    let mut out = format!(
        "You are continuing an existing session, handed over from another agent ({}). This is not a new session: do not say you have no prior context. It worked in this same folder, so its changes are already on disk.\n",
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
        let earlier = &all[1..split];
        let skip = earlier.len().saturating_sub(EARLIER_LISTED);
        if skip > 0 {
            out.push_str(&format!(
                "- ({skip} older request{} not listed)\n",
                if skip == 1 { "" } else { "s" }
            ));
        }
        for (ask, _) in &earlier[skip..] {
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
            if let Some((at, reply)) = reply {
                let who = author(input.messages, *at, input.from);
                if !reply.steps.is_empty() {
                    let skip = reply.steps.len().saturating_sub(STEPS_LISTED);
                    let steps: Vec<String> = reply.steps[skip..]
                        .iter()
                        .map(|s| one_line(s, 80))
                        .collect();
                    out.push_str(&format!(
                        "{who} did: {}{}\n",
                        if skip > 0 { "…; " } else { "" },
                        steps.join("; ")
                    ));
                }
                if !reply.text.trim().is_empty() {
                    out.push_str(&format!("{who}: {}\n", clip(&reply.text, REPLY_CHARS)));
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
    if !input.notes.trim().is_empty() {
        out.push_str(&format!("\n## Notes\n{}\n", input.notes.trim()));
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

/// Exchanges listed in a digest.
const DIGEST_EXCHANGES: usize = 12;

/// A compact timeline of the whole session, credited per agent: what agents that cannot
/// resume their own conversation get every turn, with the full memory file one read away.
pub(crate) fn digest(messages: &[Message], current: &str) -> String {
    let all = exchanges(messages);
    let Some((task, _)) = all.first() else {
        return String::new();
    };
    let mut out = format!(
        "Session memory: {MEMORY_FILE} has the full history, plan and code state; read it before answering questions about earlier work.\n\nTask: {}\n",
        clip(&task.text, 400)
    );
    out.push_str("Timeline (oldest first):\n");
    let skip = all.len().saturating_sub(DIGEST_EXCHANGES);
    if skip > 0 {
        out.push_str(&format!(
            "- ({skip} earlier exchanges in the memory file)\n"
        ));
    }
    for (ask, reply) in &all[skip..] {
        let did = match reply {
            Some((at, r)) => {
                let who = author(messages, *at, current);
                let what = if r.text.trim().is_empty() {
                    r.steps.last().map(|s| one_line(s, 120)).unwrap_or_default()
                } else {
                    one_line(&r.text, 160)
                };
                format!("{who}: {what}")
            }
            None => "no reply".into(),
        };
        out.push_str(&format!("- Asked: {} → {did}\n", one_line(&ask.text, 120)));
    }
    out
}

/// Where Runo keeps a session's memory, inside its folder.
pub(crate) const MEMORY_FILE: &str = ".runo/memory.md";
/// Notes agents saved through the memory server.
pub(crate) const NOTES_FILE: &str = ".runo/notes.md";

pub(crate) fn read_notes(dir: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(dir).join(NOTES_FILE)).unwrap_or_default()
}

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
    pub(crate) fn request_switch(&mut self, id: u64, to: usize, from_bin: &str) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if session.provider == agent::PROVIDERS[to].id {
            return;
        }
        // Straight back to the agent just left, with nothing sent: resume its own conversation.
        if session
            .undoable_switch()
            .is_some_and(|f| f.provider == agent::PROVIDERS[to].id)
        {
            if let Some(s) = self.state.session_mut(id) {
                s.switch_back();
            }
            let name = agent::PROVIDERS[to].name;
            self.state.log(id, format!("Switched back to {name}"));
            self.save();
            self.info = Some((
                format!("Back to {name}, in its own conversation. Nothing was sent."),
                std::time::Instant::now(),
            ));
            return;
        }
        if self.handles.contains_key(&Target::Session(id)) {
            self.fail(
                "handoff",
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
            notes: &read_notes(&session.dir),
        });
        let recap = if self.demo || !wants_recap(session) {
            None
        } else {
            agent::recap_command(
                &session.provider,
                from_bin,
                &session.agent_session,
                &recap_prompt(agent::PROVIDERS[to].name),
            )
        };
        let dir = session.dir.clone();
        let exchanges = exchanges(&session.messages).len();
        self.switch_preview = false;
        self.switch_plan = Some(SwitchPlan {
            session: id,
            to,
            brief,
            files,
            exchanges,
            recap: if recap.is_some() {
                Recap::Asking
            } else {
                Recap::Skipped
            },
            model: None,
        });
        if let Some(command) = recap {
            agent::oneshot_within(
                format!("handoff-recap:{id}:{to}"),
                command,
                &dir,
                RECAP_LIMIT,
                self.tx.clone(),
            );
        }
    }

    /// The outgoing agent's recap arrived: add it to the brief, whether the switch is still
    /// waiting to be confirmed or was confirmed and nothing has been sent yet.
    pub(crate) fn handle_recap(&mut self, key: &str, result: Result<String, String>) -> bool {
        let Some(rest) = key.strip_prefix("handoff-recap:") else {
            return false;
        };
        let mut parts = rest.split(':');
        let (Some(id), Some(to)) = (
            parts.next().and_then(|v| v.parse::<u64>().ok()),
            parts.next().and_then(|v| v.parse::<usize>().ok()),
        ) else {
            return true;
        };
        let text = result
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        if let Some(plan) = self
            .switch_plan
            .as_mut()
            .filter(|p| p.session == id && p.to == to && p.recap == Recap::Asking)
        {
            let from = self
                .state
                .session(id)
                .map(|s| agent::provider(&s.provider).name)
                .unwrap_or("the previous agent");
            match text {
                Some(text) => {
                    plan.brief = with_recap(&plan.brief, from, &text);
                    plan.recap = Recap::Added;
                }
                None => plan.recap = Recap::Failed,
            }
            return true;
        }
        let Some(text) = text else {
            return true;
        };
        let Some(session) = self.state.session_mut(id) else {
            return true;
        };
        // Confirmed already: only a handoff still waiting for the next message takes it.
        let Some(from) = session
            .switched_from
            .as_ref()
            .filter(|_| session.provider == agent::PROVIDERS[to].id && !session.handoff.is_empty())
            .map(|f| agent::provider(&f.provider).name)
        else {
            return true;
        };
        session.handoff = with_recap(&session.handoff, from, &text);
        session.carried = session.handoff.clone();
        self.save();
        true
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
        // What was cleared or compacted away stays out of it.
        let messages = &session.messages[crate::chatctl::context_from(&session.messages)..];
        if exchanges(messages).is_empty() || !git::is_repo(&session.dir) {
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
            messages,
            files: &files,
            commits: &git::recent_commits(&session.dir, 5),
            notes: &read_notes(&session.dir),
        }));
        let path = std::path::Path::new(&session.dir).join(MEMORY_FILE);
        git::exclude(&session.dir, "/.runo/");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }

    /// Hand a session to another agent at once, with a brief of the work so far for its
    /// next message. Asks from a canvas switch agents this way, without the dialog.
    pub(crate) fn carry_to(&mut self, id: u64, to: usize) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let to = &agent::PROVIDERS[to];
        if session.provider == to.id {
            return;
        }
        if exchanges(&session.messages).is_empty() {
            if let Some(s) = self.state.session_mut(id) {
                s.switch_provider(to.id);
            }
            self.save();
            return;
        }
        let from = session.provider.clone();
        let (files, commits) = if !self.demo && git::is_repo(&session.dir) {
            (
                git::changes(&session.dir),
                git::recent_commits(&session.dir, 5),
            )
        } else {
            (Vec::new(), String::new())
        };
        let brief = brief(&BriefInput {
            from: agent::provider(&from).name,
            dir: &session.dir,
            branch: &session.branch,
            messages: &session.messages,
            files: &files,
            commits: &commits,
            notes: &read_notes(&session.dir),
        });
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        session.switch_provider_keeping(to.id);
        let mut divider = Message::now(Role::System, "");
        divider.meta = divider_meta(&from, to.id, true, tokens(&brief));
        session.messages.push(divider);
        session.conversation_start = session.messages.len();
        session.handoff = brief.clone();
        session.carried = brief;
        self.state
            .log(id, format!("Switched to {} with context", to.name));
        self.save();
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
        session.switch_provider_keeping(to.id);
        if let Some(model) = plan.model {
            session.set_model(model);
        }
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
        let line = || div().flex_1().h(px(1.)).bg(divider());
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
                            .border_color(divider())
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
                .w(px(120.))
                .v_flex()
                .items_center()
                .gap_2()
                .child(self.provider_badge(p, 48.))
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .truncate()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child(p.name),
                )
                .with_animation(
                    key,
                    Animation::new(Duration::from_millis(420)).with_easing(ease_out_quint()),
                    move |el, d| el.opacity(d).left(px(dx * (1. - d))),
                )
        };
        // A track between the two badges, centred on them, with pulses flowing to the new agent.
        const TRACK: f32 = 84.;
        let arrow = div()
            .h(px(48.))
            .w(px(TRACK + 14.))
            .relative()
            .flex()
            .items_center()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(23.))
                    .w(px(TRACK))
                    .h(px(2.))
                    .rounded_full()
                    .bg(divider()),
            )
            .children((0..3i32).map(|i| {
                div()
                    .absolute()
                    .top(px(20.))
                    .size(px(8.))
                    .rounded_full()
                    .bg(rgb(c_blue()))
                    .shadow_md()
                    .with_animation(
                        ("switch-pulse", i as usize),
                        Animation::new(Duration::from_millis(1500)).repeat(),
                        move |el, d| {
                            let t = (d + i as f32 / 3.).fract();
                            let fade = (t * std::f32::consts::PI).sin();
                            el.left(px(t * (TRACK - 8.))).opacity(fade)
                        },
                    )
            }))
            .child(
                div()
                    .absolute()
                    .left(px(TRACK - 2.))
                    .top(px(17.))
                    .child(icon(IconName::ChevronRight, 14., c_blue())),
            );
        let card = div()
            .id("switch-card")
            .w(px(520.))
            .max_w(relative(0.94))
            .v_flex()
            .gap_4()
            .p_6()
            .rounded(px(crate::shell::ISLAND_R))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_center()
                    .gap_3()
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
                    .text_center()
                    .child(
                        div()
                            .w_full()
                            .text_center()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Switch this session to {}?", to.name)),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_center()
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
                    .bg(tint(c_subtle()))
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
                        if plan.recap == Recap::Added {
                            parts.push(format!("{}'s own recap", from.name));
                        }
                        format!("Carries over: {}", alerts::and_list(&parts))
                    }))
                    .when(plan.recap == Recap::Asking, |el| {
                        el.child(row(
                            IconName::Sparkles,
                            c_blue(),
                            format!(
                                "Asking {} for a short recap of its work. It is added when it arrives.",
                                from.name
                            ),
                        ))
                    })
                    .child(row(IconName::Check, c_green(), format!("{place}, {files}")))
                    .child(row(
                        IconName::RotateCcw,
                        c_amber(),
                        format!(
                            "Starts fresh: {to}'s model and mode settings. Switch back before \
                             sending to resume {from}'s own conversation as it was.",
                            to = to.name,
                            from = from.name
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
                                .border_color(divider())
                                .bg(term_fill())
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
            ..Default::default()
        }];
        let text = brief(&BriefInput {
            from: "Claude Code",
            dir: "/repo",
            branch: "runo/session-1",
            messages: &messages,
            files: &files,
            commits: "abc123 Add form",
            notes: "- Use bcrypt",
        });
        assert!(text.contains("## Notes\n- Use bcrypt"));
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
        assert!(!memory.contains("You are continuing"));
        assert!(memory.contains("## The task"));
    }

    fn input<'a>(messages: &'a [Message], files: &'a [git::Change]) -> BriefInput<'a> {
        BriefInput {
            from: "Codex",
            dir: "/repo",
            branch: "",
            messages,
            files,
            commits: "",
            notes: "",
        }
    }

    #[test]
    fn digests_credit_each_agent_and_stay_short() {
        let mut d = Message::new(Role::System, "");
        d.meta = divider_meta("claude", "gemini", true, 10);
        let mut messages = vec![
            Message::new(Role::User, "Create add()"),
            Message::new(Role::Agent, "Created add in calc.py.\nDetails…"),
            d,
        ];
        for i in 0..20 {
            messages.push(Message::new(Role::User, format!("Step {i}")));
            messages.push(Message::new(Role::Agent, format!("Did {i}")));
        }
        let text = digest(&messages, "Gemini");
        assert!(text.contains("Task: Create add()"));
        assert!(text.contains("(9 earlier exchanges in the memory file)"));
        assert!(text.contains("- Asked: Step 19 → Gemini: Did 19"));
        assert!(tokens(&text) < 600);
        let short = digest(&messages[..3], "Gemini");
        assert!(short.contains("- Asked: Create add() → Claude Code: Created add in calc.py."));
        assert!(digest(&[], "Gemini").is_empty());
    }

    #[test]
    fn briefs_survive_empty_and_odd_sessions() {
        // Nothing yet.
        let text = brief(&input(&[], &[]));
        assert!(!text.contains("## The task"));
        assert!(text.contains("No uncommitted changes."));
        // A reply with no request, notices and dividers only.
        let mut divider = Message::new(Role::System, "");
        divider.meta = divider_meta("claude", "codex", true, 10);
        let odd = vec![
            Message::new(Role::Agent, "stray reply"),
            Message::new(Role::System, "The agent stopped with an error."),
            divider,
            Message::new(Role::User, "   "),
        ];
        let text = brief(&input(&odd, &[]));
        assert!(!text.contains("stray reply"));
        assert!(!text.contains("stopped with an error"));
        assert!(!text.contains("Branch:"));
        // A folder that is not a repository.
        assert!(brief(&input(&odd, &[])).contains("source of truth"));
    }

    #[test]
    fn briefs_stay_small_for_huge_sessions() {
        let big = "lörem ïpsum 🚀 ".repeat(2000);
        let mut messages = Vec::new();
        for i in 0..300 {
            messages.push(Message::new(Role::User, format!("Request {i}: {big}")));
            let mut reply = Message::new(Role::Agent, big.clone());
            reply.steps = (0..50).map(|n| format!("Step {n} {big}")).collect();
            messages.push(reply);
        }
        let files: Vec<git::Change> = (0..500)
            .map(|n| git::Change {
                path: format!("src/f{n}.rs"),
                status: "M".into(),
                added: n,
                removed: 0,
                ..Default::default()
            })
            .collect();
        let text = brief(&input(&messages, &files));
        assert!(tokens(&text) < 6000, "{} tokens", tokens(&text));
        assert!(text.contains("(282 older requests not listed)"));
        assert!(text.contains("Me: Request 299"));
        assert!(text.contains("- …and 460 more"));
        assert!(text.contains("did: …; Step 38"));
    }

    #[test]
    fn briefs_say_the_session_continues() {
        let text = brief(&input(&[], &[]));
        assert!(text.contains("This is not a new session"));
    }

    #[test]
    fn recaps_go_before_the_code_state_once() {
        let messages = vec![
            Message::new(Role::User, "Fix the build"),
            Message::new(Role::Agent, "Fixed it."),
        ];
        let text = brief(&input(&messages, &[]));
        let with = with_recap(&text, "Codex", "- Bumped Rust to 1.99\n- Next: wire SQLite");
        let recap = with.find("## Recap from Codex\n- Bumped Rust").unwrap();
        assert!(recap < with.find("## Code state").unwrap());
        assert_eq!(with_recap(&with, "Codex", "again"), with);
        assert_eq!(with_recap(&text, "Codex", "  "), text);
        let long = with_recap(&text, "Codex", &"x".repeat(5000));
        assert!(long.contains("x […]"));
        assert_eq!(
            with_recap("no state", "Codex", "r"),
            "no state\n\n## Recap from Codex\nr\n"
        );
        assert!(recap_prompt("Claude Code").contains("(Claude Code)"));
    }

    #[test]
    fn recaps_are_asked_only_after_real_work() {
        let mut worked = Message::new(Role::Agent, "Done");
        worked.steps = vec!["Edited src/a.rs".into()];
        let mut session = Session {
            provider: "claude".into(),
            agent_session: "abc".into(),
            messages: vec![
                Message::new(Role::User, "Hi"),
                Message::new(Role::Agent, "Hello"),
            ],
            ..Session::default()
        };
        assert!(!wants_recap(&session), "a chat with no tool steps");
        session.messages.push(Message::new(Role::User, "Edit a.rs"));
        session.messages.push(worked);
        assert!(wants_recap(&session));
        session.agent_session.clear();
        assert!(!wants_recap(&session), "no Claude conversation to fork");
        session.provider = "codex".into();
        assert!(wants_recap(&session));
        session.provider = "gemini".into();
        assert!(!wants_recap(&session));
        session.provider = "opencode".into();
        assert!(!wants_recap(&session), "no OpenCode session to fork");
        session.agent_session = "ses_1".into();
        assert!(wants_recap(&session));
        session.provider = "codex".into();
        session.conversation_start = session.messages.len();
        assert!(!wants_recap(&session), "work from before an earlier switch");
    }

    #[test]
    fn switching_straight_back_restores_the_conversation() {
        let mut session = Session {
            provider: "codex".into(),
            agent_session: "thread-1".into(),
            carried: "older brief".into(),
            messages: vec![
                Message::new(Role::User, "Go"),
                Message::new(Role::Agent, "Done"),
            ],
            ..Session::default()
        };
        session.opts.insert("mode".into(), "plan".into());
        session.opts.insert("model".into(), "gpt-5".into());
        session.switch_provider_keeping("claude");
        session.handoff = "brief".into();
        let mut divider = Message::new(Role::System, "");
        divider.meta = divider_meta("codex", "claude", true, 10);
        session.messages.push(divider);
        session.conversation_start = session.messages.len();
        assert_eq!(
            session.undoable_switch().map(|f| f.provider.as_str()),
            Some("codex")
        );
        assert!(session.switch_back());
        assert_eq!(session.provider, "codex");
        assert_eq!(session.agent_session, "thread-1");
        assert_eq!(session.opts.get("model").map(String::as_str), Some("gpt-5"));
        assert_eq!(session.opts.get("mode").map(String::as_str), Some("plan"));
        assert_eq!(session.conversation_start, 0);
        assert_eq!(session.messages.len(), 2, "the divider is dropped");
        assert!(session.handoff.is_empty());
        assert_eq!(session.carried, "older brief");
        assert!(session.has_conversation());
        assert!(!session.switch_back(), "nothing left to undo");

        // After a message goes to the new agent the switch is final.
        session.switch_provider_keeping("claude");
        session.messages.push(Message::new(Role::User, "Next"));
        assert!(session.undoable_switch().is_none());
        assert!(!session.switch_back());
        assert_eq!(session.provider, "claude");
    }

    #[test]
    fn replies_are_credited_to_the_agent_that_wrote_them() {
        let mut d1 = Message::new(Role::System, "");
        d1.meta = divider_meta("claude", "gemini", true, 10);
        let mut d2 = Message::new(Role::System, "");
        d2.meta = divider_meta("gemini", "codex", false, 0);
        let messages = vec![
            Message::new(Role::User, "Start"),
            Message::new(Role::Agent, "Claude reply"),
            d1,
            Message::new(Role::User, "Next"),
            Message::new(Role::Agent, "Gemini reply"),
            d2,
            Message::new(Role::User, "Last"),
            Message::new(Role::Agent, "Codex reply"),
        ];
        let text = brief(&input(&messages, &[]));
        assert!(text.contains("Claude Code: Claude reply"), "{text}");
        assert!(text.contains("Gemini: Gemini reply"));
        assert!(text.contains("Codex: Codex reply"));
        let session = Session {
            provider: "codex".into(),
            messages,
            ..Session::default()
        };
        assert_eq!(provider_at(&session, 1).id, "claude");
        assert_eq!(provider_at(&session, 4).id, "gemini");
        assert_eq!(provider_at(&session, 7).id, "codex");
    }
}
