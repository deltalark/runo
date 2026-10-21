//! The Runo assistant: a mark in the corner of the window that answers questions about
//! Runo. It reads a short guide built into the app (`docs/guide`, one article per feature)
//! and the person's default agent on a fast model writes the answer from it. It never
//! changes anything; an answer can only offer to take the person to the right place.
//! It is off until turned on in Settings > General.

use super::*;
use std::sync::OnceLock;

/// The guide, embedded so it always matches this build and works offline.
const SOURCES: &[(&str, &str)] = &[
    (
        "new-session",
        include_str!("../../../docs/guide/new-session.md"),
    ),
    (
        "worktrees",
        include_str!("../../../docs/guide/worktrees.md"),
    ),
    (
        "race-agents",
        include_str!("../../../docs/guide/race-agents.md"),
    ),
    ("tabs", include_str!("../../../docs/guide/tabs.md")),
    ("history", include_str!("../../../docs/guide/history.md")),
    ("composer", include_str!("../../../docs/guide/composer.md")),
    ("chat-cli", include_str!("../../../docs/guide/chat-cli.md")),
    (
        "approvals",
        include_str!("../../../docs/guide/approvals.md"),
    ),
    (
        "undo-turn",
        include_str!("../../../docs/guide/undo-turn.md"),
    ),
    (
        "proof-of-done",
        include_str!("../../../docs/guide/proof-of-done.md"),
    ),
    ("missions", include_str!("../../../docs/guide/missions.md")),
    (
        "switch-agent",
        include_str!("../../../docs/guide/switch-agent.md"),
    ),
    (
        "changes-review",
        include_str!("../../../docs/guide/changes-review.md"),
    ),
    (
        "diff-notes",
        include_str!("../../../docs/guide/diff-notes.md"),
    ),
    (
        "files-terminal",
        include_str!("../../../docs/guide/files-terminal.md"),
    ),
    ("browser", include_str!("../../../docs/guide/browser.md")),
    ("github", include_str!("../../../docs/guide/github.md")),
    (
        "playbooks",
        include_str!("../../../docs/guide/playbooks.md"),
    ),
    (
        "share-playbooks",
        include_str!("../../../docs/guide/share-playbooks.md"),
    ),
    (
        "schedules",
        include_str!("../../../docs/guide/schedules.md"),
    ),
    (
        "connections",
        include_str!("../../../docs/guide/connections.md"),
    ),
    (
        "memory-mcp",
        include_str!("../../../docs/guide/memory-mcp.md"),
    ),
    (
        "agents-setup",
        include_str!("../../../docs/guide/agents-setup.md"),
    ),
    (
        "quick-open",
        include_str!("../../../docs/guide/quick-open.md"),
    ),
    (
        "navigation",
        include_str!("../../../docs/guide/navigation.md"),
    ),
    (
        "appearance",
        include_str!("../../../docs/guide/appearance.md"),
    ),
    (
        "keybindings",
        include_str!("../../../docs/guide/keybindings.md"),
    ),
    ("tour", include_str!("../../../docs/guide/tour.md")),
    ("usage", include_str!("../../../docs/guide/usage.md")),
    ("phone", include_str!("../../../docs/guide/phone.md")),
    (
        "assistant",
        include_str!("../../../docs/guide/assistant.md"),
    ),
];

/// Questions offered when the panel opens with nothing asked yet.
const STARTERS: [&str; 4] = [
    "How do I race Claude Code and Codex on one task?",
    "How do I keep an agent running after I close my laptop?",
    "How do I share a playbook?",
    "How do I undo what the agent just did?",
];

/// How many articles go to the agent with each question.
const ARTICLES_SENT: usize = 4;

/// One guide article: a short header (title, keywords, shortcuts, where it opens) and text.
pub(crate) struct Article {
    pub id: &'static str,
    pub title: String,
    keywords: Vec<String>,
    /// Shortcut ids from `keymap::SHORTCUTS` the article mentions.
    pub shortcuts: Vec<String>,
    /// Where "Take me there" goes, as `jump_for` reads it.
    pub open: Option<String>,
    pub body: String,
}

fn parse(id: &'static str, text: &str) -> Article {
    let mut article = Article {
        id,
        title: id.to_string(),
        keywords: Vec::new(),
        shortcuts: Vec::new(),
        open: None,
        body: text.trim().to_string(),
    };
    let Some(rest) = text.trim_start().strip_prefix("---") else {
        return article;
    };
    let Some((header, body)) = rest.split_once("\n---") else {
        return article;
    };
    let list = |v: &str| -> Vec<String> {
        v.split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    };
    for line in header.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "title" => article.title = value.to_string(),
            "keywords" => article.keywords = list(value),
            "shortcuts" => article.shortcuts = list(value),
            "open" if !value.is_empty() => article.open = Some(value.to_string()),
            _ => {}
        }
    }
    article.body = body.trim().to_string();
    article
}

pub(crate) fn guide() -> &'static [Article] {
    static GUIDE: OnceLock<Vec<Article>> = OnceLock::new();
    GUIDE.get_or_init(|| SOURCES.iter().map(|(id, text)| parse(id, text)).collect())
}

pub(crate) fn article(id: &str) -> Option<&'static Article> {
    guide().iter().find(|a| a.id == id)
}

const STOP_WORDS: [&str; 24] = [
    "how", "do", "does", "i", "a", "an", "the", "to", "in", "on", "of", "is", "it", "can", "my",
    "me", "what", "where", "and", "or", "with", "for", "this", "runo",
];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1 && !STOP_WORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// Articles that match `query`, best first: a keyword phrase in the question counts most,
/// then words of the title, the keywords and the text.
pub(crate) fn search(query: &str) -> Vec<&'static Article> {
    let lower = query.to_lowercase();
    let asked = words(query);
    let mut scored: Vec<(u32, &'static Article)> = guide()
        .iter()
        .map(|a| {
            let mut score = 0;
            for k in &a.keywords {
                if k.contains(' ') && lower.contains(k.as_str()) {
                    score += 6;
                }
            }
            let title = words(&a.title);
            let keys: Vec<String> = a.keywords.iter().flat_map(|k| words(k)).collect();
            let body = words(&a.body);
            for w in &asked {
                let stem = |x: &String| x == w || (w.len() > 4 && x.starts_with(&w[..w.len() - 1]));
                if title.iter().any(stem) {
                    score += 3;
                }
                if keys.iter().any(stem) {
                    score += 2;
                }
                if body.iter().any(stem) {
                    score += 1;
                }
            }
            (score, a)
        })
        .filter(|(score, _)| *score > 1)
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, a)| a).collect()
}

/// An article's text with `{shortcut-id}` filled in with the keys it uses now.
fn with_live_keys(text: &str, cx: &App) -> String {
    let mut out = text.to_string();
    for s in keymap::SHORTCUTS.iter() {
        let tag = format!("{{{}}}", s.id);
        if out.contains(&tag) {
            out = out.replace(&tag, &format!("`{}`", keymap::label(cx, s.id)));
        }
    }
    out
}

/// Where an article's `open:` value leads.
fn jump_for(open: &str) -> Option<Jump> {
    if let Some(page) = open.strip_prefix("settings:") {
        return SettingsPage::pages()
            .find(|p| p.label().eq_ignore_ascii_case(page))
            .map(Jump::Settings);
    }
    Some(match open {
        "new-session" => Jump::Action,
        "home" => Jump::View(View::Home),
        "inbox" => Jump::View(View::Inbox),
        "sessions" => Jump::View(View::AllSessions),
        "review" => Jump::View(View::Review),
        "missions" => Jump::View(View::Missions),
        "github" => Jump::View(View::GitHub),
        "playbooks" => Jump::View(View::Automations),
        "history" => Jump::View(View::History),
        "tour" => Jump::Tour,
        _ => return None,
    })
}

/// The label of an article's "Take me there" button.
fn open_label(open: &str) -> String {
    if let Some(page) = open.strip_prefix("settings:") {
        let name = SettingsPage::pages()
            .find(|p| p.label().eq_ignore_ascii_case(page))
            .map(|p| p.label())
            .unwrap_or("Settings");
        return format!("Open Settings → {name}");
    }
    match open {
        "new-session" => "Open New session".into(),
        "tour" => "Start the tour".into(),
        "playbooks" => "Go to Playbooks".into(),
        "github" => "Go to GitHub".into(),
        "history" => "Go to History".into(),
        "review" => "Go to Review".into(),
        "missions" => "Go to Missions".into(),
        "inbox" => "Go to Inbox".into(),
        "sessions" => "Go to Sessions".into(),
        _ => "Go to Home".into(),
    }
}

pub(crate) struct Msg {
    you: bool,
    text: String,
    /// The guide articles the answer came from.
    sources: Vec<&'static str>,
    /// Where "Take me there" goes.
    open: Option<String>,
}

pub(crate) struct Assistant {
    pub open: bool,
    input: Entity<InputState>,
    chat: Vec<Msg>,
    /// The question being answered by the agent, by its run key.
    asking: Option<String>,
    asked: u64,
    /// The agent answering the question being asked.
    answerer: &'static str,
    scroll: ScrollHandle,
    /// Whether the pointer is over the mark in the corner, and since when.
    hover: Option<(bool, std::time::Instant)>,
}

impl Assistant {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<RunoApp>) -> (Self, Vec<Subscription>) {
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Ask how to do something in Runo…"));
        let subs = vec![
            cx.subscribe_in(&input, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let question = input.read(cx).value().trim().to_string();
                    if !question.is_empty() {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                        this.ask_runo(question, cx);
                    }
                }
                cx.notify();
            }),
        ];
        (
            Self {
                open: false,
                input,
                chat: Vec::new(),
                asking: None,
                asked: 0,
                answerer: "",
                scroll: ScrollHandle::new(),
                hover: None,
            },
            subs,
        )
    }
}

/// The prompt the answering agent gets: the guide articles that match, what is on screen,
/// and how to answer.
fn prompt(question: &str, articles: &[&Article], context: &str, cx: &App) -> String {
    let titles: Vec<&str> = guide().iter().map(|a| a.title.as_str()).collect();
    let mut text = String::from(
        "You are the help assistant inside Runo, a desktop app that runs coding agents side by \
         side. Answer the person's question about using Runo from the guide below only. Reply \
         in a few short sentences or numbered steps, plain Markdown, no headings. Name buttons \
         and settings exactly as the guide does. If the guide doesn't cover it, say so in one \
         sentence and suggest the closest article. Don't run tools or look at files. On the last \
         line, write OPEN: and the id of the one article whose place in the app would help \
         most, or OPEN: none.\n\n",
    );
    text.push_str(&format!("What is on screen now: {context}\n\n"));
    for a in articles {
        text.push_str(&format!(
            "## {} (id: {})\n{}\n\n",
            a.title,
            a.id,
            with_live_keys(&a.body, cx)
        ));
    }
    text.push_str(&format!(
        "Other articles in the guide: {}\n\nQuestion: {question}\n",
        titles.join("; ")
    ));
    text
}

/// The agent's reply without its `OPEN:` line, and the article id that line named.
fn split_reply(reply: &str) -> (String, Option<String>) {
    let mut open = None;
    let mut lines: Vec<&str> = Vec::new();
    for line in reply.trim().lines() {
        if let Some(id) = line.trim().strip_prefix("OPEN:") {
            let id = id.trim().trim_matches('`');
            if id != "none" && !id.is_empty() {
                open = Some(id.to_string());
            }
        } else {
            lines.push(line);
        }
    }
    (lines.join("\n").trim().to_string(), open)
}

impl RunoApp {
    pub(crate) fn assistant_enabled(cx: &App) -> bool {
        cx.global::<AppSettings>().assistant
    }

    /// The mark is on screen: turned on, and not covered by the tour or Settings.
    pub(crate) fn assistant_shown(&self, cx: &App) -> bool {
        Self::assistant_enabled(cx) && self.tour.is_none() && self.view != View::Settings
    }

    pub(crate) fn toggle_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !Self::assistant_enabled(cx) {
            self.info = Some((
                "Turn on the Runo assistant in Settings → General.".into(),
                std::time::Instant::now(),
            ));
            return;
        }
        self.assistant.open = !self.assistant.open;
        if self.assistant.open {
            if self.view == View::Settings {
                self.view = View::Home;
            }
            self.assistant.input.update(cx, |i, cx| i.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant.open = false;
        window.focus(&self.focus, cx);
    }

    /// A short description of what is on screen, sent with each question. No file
    /// contents and no conversations.
    fn assistant_context(&self) -> String {
        let view = match self.view {
            View::Home => "Home",
            View::Session => "a session",
            View::NewSession => "the New session form",
            View::Inbox => "Inbox",
            View::AllSessions => "Sessions",
            View::History => "History",
            View::Worktrees => "Isolated sessions",
            View::Settings => "Settings",
            View::Notes => "Project notes",
            View::Activity => "Timeline",
            View::Automations => "Playbooks",
            View::GitHub => "GitHub",
            View::Editor => "a file",
            View::Review => "Review",
            View::Branches => "Git branches",
            View::Missions => "Missions",
            View::Design => "Design",
            View::Architecture => "Architecture",
        };
        let mut parts = vec![format!("the {view} view")];
        if let Some(project) = self
            .current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
        {
            parts.push(format!("project {}", project.name));
        }
        if let Some(session) = self.current() {
            parts.push(format!(
                "session \"{}\" with {} on branch {}, {}",
                session.title,
                agent::provider(&session.provider).name,
                session.branch,
                Self::status_label(session.status).to_lowercase()
            ));
        }
        parts.push(format!("{} agents running", self.started.len()));
        parts.join("; ")
    }

    /// Answer `question`: from the agent when one is set to answer, otherwise from the guide.
    pub(crate) fn ask_runo(&mut self, question: String, cx: &mut Context<Self>) {
        if self.assistant.asking.is_some() {
            return;
        }
        let found = search(&question);
        self.assistant.chat.push(Msg {
            you: true,
            text: question.clone(),
            sources: Vec::new(),
            open: None,
        });
        self.assistant.scroll.scroll_to_bottom();
        let st = cx.global::<AppSettings>();
        let provider = agent::provider(&st.default_provider);
        let use_agent = st.assistant_answers == "agent" && agent::installed(provider.bin);
        if !use_agent {
            self.answer_from_guide(&found, None);
            return;
        }
        self.assistant.answerer = provider.name;
        let articles: Vec<&Article> = if found.is_empty() {
            guide().iter().take(ARTICLES_SENT).collect()
        } else {
            found.iter().take(ARTICLES_SENT).copied().collect()
        };
        let text = prompt(&question, &articles, &self.assistant_context(), cx);
        let configured = st.agent(provider.id).binary;
        let mut command = agent::ask_command(provider.id, &configured, &text);
        // A small, fast model is plenty for a how-to answer.
        if provider.id == "claude" {
            let args: Vec<_> = command.get_args().map(|a| a.to_owned()).collect();
            command = std::process::Command::new(command.get_program());
            command.args(["--model", "haiku"]).args(args);
        }
        let dir = store::data_dir().join("assistant");
        let _ = std::fs::create_dir_all(&dir);
        self.assistant.asked += 1;
        let key = format!("assistant:{}", self.assistant.asked);
        self.assistant.asking = Some(key.clone());
        agent::oneshot(key, command, &dir.to_string_lossy(), self.tx.clone());
    }

    /// The best matching article as the answer. Its `{shortcut}` tags are filled in when
    /// it is drawn, so they follow the person's keys.
    fn answer_from_guide(&mut self, found: &[&'static Article], note: Option<String>) {
        let msg = match found.first() {
            Some(a) => Msg {
                you: false,
                text: format!(
                    "{}**{}**\n\n{}",
                    note.map(|n| format!("{n}\n\n")).unwrap_or_default(),
                    a.title,
                    a.body
                ),
                sources: found.iter().take(3).map(|a| a.id).collect(),
                open: a.open.clone(),
            },
            None => Msg {
                you: false,
                text: "I couldn't find that in the guide. Try asking about sessions, worktrees, \
                       racing agents, reviewing changes, playbooks or connections."
                    .into(),
                sources: Vec::new(),
                open: None,
            },
        };
        self.assistant.chat.push(msg);
        self.assistant.scroll.scroll_to_bottom();
    }

    /// The answering agent finished.
    pub(crate) fn assistant_reply(&mut self, key: String, result: Result<String, String>) {
        if self.assistant.asking.as_ref() != Some(&key) {
            return;
        }
        self.assistant.asking = None;
        let question = self
            .assistant
            .chat
            .iter()
            .rev()
            .find(|m| m.you)
            .map(|m| m.text.clone())
            .unwrap_or_default();
        let found = search(&question);
        match result {
            Ok(reply) if !reply.trim().is_empty() => {
                let (text, open_id) = split_reply(&reply);
                let opened = open_id.as_deref().and_then(article);
                let mut sources: Vec<&'static str> = found.iter().take(2).map(|a| a.id).collect();
                if let Some(a) = opened {
                    if !sources.contains(&a.id) {
                        sources.insert(0, a.id);
                    }
                }
                self.assistant.chat.push(Msg {
                    you: false,
                    text,
                    sources,
                    open: opened
                        .or(found.first().copied())
                        .and_then(|a| a.open.clone()),
                });
                self.assistant.scroll.scroll_to_bottom();
            }
            _ => {
                let name = self.assistant.answerer;
                self.answer_from_guide(
                    &found,
                    Some(format!(
                        "{name} didn't answer, so here is what the guide says."
                    )),
                );
            }
        }
    }

    fn assistant_follow(&mut self, open: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(jump) = jump_for(open) else { return };
        self.assistant.open = false;
        if open == "new-session" {
            window.dispatch_action(Box::new(NewSession), cx);
        } else {
            self.palette_jump(jump, window, cx);
        }
    }

    /// The mark in the corner, and the panel above it while it is open.
    pub(crate) fn assistant_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.assistant_shown(cx) {
            return None;
        }
        let waiting = self.approvals.len() + self.waiting_sessions().len();
        let state = if self.assistant.asking.is_some() {
            Mood::Thinking
        } else if self.assistant.open {
            Mood::Listening
        } else if waiting > 0 {
            Mood::NeedsYou
        } else {
            Mood::Idle
        };
        let mark = div()
            .id("runo-assistant")
            .absolute()
            .right(px(18.))
            .bottom(px(36.))
            .occlude()
            .cursor_pointer()
            .active(|el| el.opacity(0.85))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.assistant.hover = Some((*hovered, std::time::Instant::now()));
                cx.notify();
            }))
            .tooltip(|window, cx| {
                let keys = keymap::label(cx, "ask-runo");
                gpui_kit::component::tooltip::Tooltip::new(format!("Ask Runo  {keys}"))
                    .build(window, cx)
            })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_assistant(window, cx)))
            .child(live_mark(
                state,
                (waiting > 0).then_some(waiting),
                self.assistant.hover,
            ));
        let mut layer = div().absolute().top_0().left_0().size_full().child(mark);
        if self.assistant.open {
            layer = layer.child(self.assistant_panel(cx));
        }
        Some(layer.into_any_element())
    }

    fn assistant_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(divider())
            .child(mark(Mood::Idle, 24., None))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Runo"))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("Ask how to do anything in Runo"),
            )
            .child(div().flex_1())
            .when(!self.assistant.chat.is_empty(), |el| {
                el.child(
                    self.ghost_button("assistant-clear", IconName::RotateCcw, "New chat")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.assistant.chat.clear();
                            this.assistant.asking = None;
                            cx.notify();
                        })),
                )
            })
            .child(
                self.ghost_button("assistant-close", IconName::X, "Close")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_assistant(window, cx);
                        cx.notify();
                    })),
            );

        let mut body = div()
            .id("assistant-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.assistant.scroll)
            .px_4()
            .py_3()
            .v_flex()
            .gap_3();
        if self.assistant.chat.is_empty() {
            body = body
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("What do you need?"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(rgb(c_secondary()))
                        .child("I know Runo's features from its built-in guide. Ask how to do something and I'll explain it and take you there."),
                )
                .child(
                    div()
                        .pt_1()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child("TRY ASKING"),
                );
            for (i, q) in STARTERS.iter().enumerate() {
                body = body.child(
                    div()
                        .id(("assistant-starter", i))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(divider())
                        .bg(rgb(c_raised()))
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .text_size(px(12.5))
                        .child(icon(IconName::CircleQuestionMark, 13., c_muted()))
                        .child(*q)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_runo(q.to_string(), cx);
                            cx.notify();
                        })),
                );
            }
        }
        for (i, m) in self.assistant.chat.iter().enumerate() {
            if m.you {
                body = body.child(
                    div().flex().justify_end().child(
                        div()
                            .max_w(px(300.))
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(c_active()))
                            .text_size(px(12.5))
                            .line_height(px(18.))
                            .child(m.text.clone()),
                    ),
                );
                continue;
            }
            let mut answer = div().v_flex().gap_2().child(
                div().text_size(px(12.5)).child(
                    TextView::markdown(
                        SharedString::from(format!("assistant-answer-{i}")),
                        with_live_keys(&m.text, cx),
                    )
                    .selectable(true),
                ),
            );
            if !m.sources.is_empty() {
                let mut chips = div().flex().flex_wrap().gap_1();
                for id in &m.sources {
                    if let Some(a) = article(id) {
                        chips = chips.child(
                            div()
                                .px(px(7.))
                                .py(px(2.))
                                .rounded_md()
                                .border_1()
                                .border_color(divider())
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .child(format!("Guide: {}", a.title)),
                        );
                    }
                }
                answer = answer.child(chips);
            }
            if let Some(open) = m.open.clone().filter(|o| jump_for(o).is_some()) {
                let label = open_label(&open);
                answer = answer.child(
                    div()
                        .flex()
                        .child(self.button(("assistant-open", i), label, true).on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.assistant_follow(&open, window, cx);
                                cx.notify();
                            }),
                        )),
                );
            }
            body = body.child(answer);
        }
        if self.assistant.asking.is_some() {
            body = body.child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child("Looking it up…"),
            );
        }

        let st = cx.global::<AppSettings>();
        let answers = if st.assistant_answers == "agent" {
            let provider = agent::provider(&st.default_provider);
            if agent::installed(provider.bin) {
                format!("Answers with {}", provider.name)
            } else {
                "Answers from the guide".to_string()
            }
        } else {
            "Answers from the guide".to_string()
        };
        let footer = div()
            .px_4()
            .pb_3()
            .v_flex()
            .gap_2()
            .child(Input::new(&self.assistant.input).bordered(true).h(px(36.)))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(answers)
                    .child("Never changes anything"),
            );

        div()
            .id("assistant-panel")
            .absolute()
            .right(px(18.))
            .bottom(px(96.))
            .w(px(400.))
            .max_w(relative(0.9))
            .h(px(520.))
            .max_h(relative(0.75))
            .v_flex()
            .rounded_xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(header)
            .child(body)
            .child(footer)
            // It opens up out of the mark: fading in while rising the last few pixels.
            .animated(
                "assistant-panel-in",
                Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d).bottom(px(96. - 10. * (1. - d))),
            )
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mood {
    Idle,
    Listening,
    Thinking,
    NeedsYou,
}

/// Where Runo's three bars sit for `mood`, as (left, width) at size 46, and the middle
/// bar's colour.
fn pose(mood: Mood) -> (Bars, u32) {
    match mood {
        Mood::Idle => ([(10., 17.), (17., 19.), (12., 13.)], c_text()),
        Mood::Listening => ([(12., 22.), (9., 28.), (15., 16.)], c_blue()),
        Mood::Thinking => ([(18., 17.), (9., 19.), (21., 13.)], VIOLET),
        Mood::NeedsYou => ([(10., 17.), (17., 19.), (12., 13.)], c_amber()),
    }
}

/// Each of Runo's bars, as (left, width) at size 46.
type Bars = [(f32, f32); 3];

thread_local! {
    /// The corner mark's mood, when it took it, and where its bars were then.
    static MARK: std::cell::Cell<Option<(Mood, std::time::Instant, Bars)>> =
        const { std::cell::Cell::new(None) };
}

/// How long the bars take to move into a new mood's pose.
const MORPH_MS: f32 = 260.;
/// How long the mark takes to lift under the pointer, or settle back.
const LIFT_MS: f32 = 140.;

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

fn ms_since(at: std::time::Instant) -> f32 {
    at.elapsed().as_secs_f32() * 1000.
}

/// Where the bars are `since` a change from `from` to `to`.
fn bars_between(from: Bars, to: Bars, t: f32) -> Bars {
    let k = ease_out(t);
    std::array::from_fn(|i| {
        (
            from[i].0 + (to[i].0 - from[i].0) * k,
            from[i].1 + (to[i].1 - from[i].1) * k,
        )
    })
}

/// The mark in the corner, alive: its bars glide into each new mood's pose, drift to and
/// fro while Runo thinks, and the mark lifts a little, its middle bar reaching out, while
/// the pointer is over it. Still when animations are off.
fn live_mark(
    mood: Mood,
    badge: Option<usize>,
    hover: Option<(bool, std::time::Instant)>,
) -> AnyElement {
    let target = pose(mood).0;
    let now = std::time::Instant::now();
    let (since, from) = match MARK.get() {
        Some((was, since, from)) if was == mood => (since, from),
        Some((was, since, from)) => {
            // Start from wherever the bars are now, even mid-glide.
            let at = bars_between(from, pose(was).0, ms_since(since) / MORPH_MS);
            (now, at)
        }
        None => (now, target),
    };
    MARK.set(Some((mood, since, from)));
    if !anim_on() {
        return mark(mood, 46., badge);
    }
    let lift_at = move || match hover {
        Some((on, at)) => {
            let t = ease_out(ms_since(at) / LIFT_MS);
            if on {
                t
            } else {
                1. - t
            }
        }
        None => 0.,
    };
    let thinking = mood == Mood::Thinking;
    let animation = if thinking {
        Animation::new(Duration::from_millis(1400)).repeat()
    } else {
        Animation::new(Duration::from_millis(MORPH_MS.max(LIFT_MS) as u64))
    };
    let key = format!("runo-mark-{mood:?}-{since:?}-{:?}", hover);
    let el = div().relative().size(px(46.)).with_animation(
        ElementId::Name(key.into()),
        animation,
        move |el, d| {
            let mut bars = bars_between(from, target, ms_since(since) / MORPH_MS);
            if thinking {
                // Each bar drifts a little out of step with the next.
                for (i, bar) in bars.iter_mut().enumerate() {
                    let phase = (d + i as f32 / 3.) * std::f32::consts::TAU;
                    bar.0 += 3. * phase.sin();
                }
            }
            let lift = lift_at();
            bars[1].1 += 5. * lift;
            el.child(
                div()
                    .absolute()
                    .top(px(-2. * lift))
                    .child(mark_with(mood, 46., bars, None)),
            )
        },
    );
    let mut layer = div().relative().child(el);
    if let Some(n) = badge {
        layer = layer.child(badge_dot(n).with_animation(
            ElementId::Name(format!("runo-badge-{n}").into()),
            Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
            |el, d| el.opacity(d).top(px(-6. + 4. * (1. - d))),
        ));
    }
    layer.into_any_element()
}

/// The count of things waiting, on the mark's corner.
fn badge_dot(n: usize) -> Div {
    div()
        .absolute()
        .top(px(-6.))
        .right(px(-6.))
        .min_w(px(18.))
        .h(px(18.))
        .px(px(4.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(c_amber()))
        .text_color(rgb(0x1a1205))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(n.to_string())
}

/// Runo's icon, three staggered bars on a rounded square, posed for `mood`. At `size` 46 it
/// is the mark in the corner; smaller in the panel header.
fn mark(mood: Mood, size: f32, badge: Option<usize>) -> AnyElement {
    mark_with(mood, size, pose(mood).0, badge)
}

/// The icon with its bars at `bars`, as (left, width) at size 46.
fn mark_with(mood: Mood, size: f32, bars: Bars, badge: Option<usize>) -> AnyElement {
    let s = size / 46.;
    let middle = pose(mood).1;
    let tops = [13., 20.5, 28.];
    let shades = [
        settings::mix(c_muted(), c_raised(), 0.85),
        middle,
        settings::mix(c_secondary(), c_raised(), 0.8),
    ];
    let mut square = div()
        .relative()
        .size(px(size))
        .rounded(px(14. * s))
        .border_1()
        .border_color(rgb(match mood {
            Mood::Listening => c_blue(),
            Mood::NeedsYou => c_amber(),
            _ => settings::mix(c_border(), c_text(), 0.88),
        }))
        .bg(rgb(c_raised()))
        .shadow_lg();
    for i in 0..3 {
        square = square.child(
            div()
                .absolute()
                .left(px(bars[i].0 * s))
                .top(px(tops[i] * s))
                .w(px(bars[i].1 * s))
                .h(px((5. * s).max(2.)))
                .rounded_full()
                .bg(rgb(shades[i])),
        );
    }
    if let Some(n) = badge {
        square = square.child(badge_dot(n));
    }
    square.into_any_element()
}

const VIOLET: u32 = 0xa78bfa;

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn every_guide_file_is_built_in() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guide");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".md"))
            .map(|n| n.trim_end_matches(".md").to_string())
            .collect();
        files.sort();
        let mut built: Vec<String> = SOURCES.iter().map(|(id, _)| id.to_string()).collect();
        built.sort();
        assert_eq!(
            files, built,
            "docs/guide and SOURCES list different articles"
        );
    }

    #[test]
    fn articles_have_a_title_and_text() {
        for a in guide() {
            assert!(
                a.title != a.id && !a.title.is_empty(),
                "{} has no title",
                a.id
            );
            assert!(a.body.len() > 40, "{} has almost no text", a.id);
            assert!(!a.body.starts_with("---"), "{} header was not read", a.id);
        }
    }

    #[test]
    fn every_shortcut_is_in_the_guide() {
        for s in keymap::SHORTCUTS.iter() {
            assert!(
                guide()
                    .iter()
                    .any(|a| a.shortcuts.iter().any(|k| k == s.id)),
                "no guide article lists the {} shortcut",
                s.id
            );
        }
    }

    #[test]
    fn shortcuts_and_places_the_guide_names_exist() {
        for a in guide() {
            for k in &a.shortcuts {
                assert!(
                    keymap::SHORTCUTS.iter().any(|s| s.id == k),
                    "{} lists unknown shortcut {k}",
                    a.id
                );
            }
            for (start, _) in a.body.match_indices('{') {
                let rest = &a.body[start + 1..];
                if rest.starts_with('{') || a.body[..start].ends_with('{') {
                    continue;
                }
                let id = rest.split('}').next().unwrap_or("");
                assert!(
                    a.shortcuts.iter().any(|k| k == id),
                    "{} uses {{{id}}} without listing it under shortcuts",
                    a.id
                );
            }
            if let Some(open) = &a.open {
                assert!(
                    jump_for(open).is_some(),
                    "{} opens unknown place {open}",
                    a.id
                );
            }
        }
    }

    #[test]
    fn questions_find_the_right_article() {
        let top = |q: &str| search(q).first().map(|a| a.id);
        assert_eq!(
            top("How do I race Claude Code and Codex on one task?"),
            Some("race-agents")
        );
        assert_eq!(top("How do I share a playbook?"), Some("share-playbooks"));
        assert_eq!(
            top("How do I keep an agent running after I close my laptop?"),
            Some("connections")
        );
        assert_eq!(
            top("How do I undo what the agent just did?"),
            Some("undo-turn")
        );
        assert_eq!(
            top("How do I make Runo run my tests after each turn?"),
            Some("proof-of-done")
        );
        assert_eq!(
            top("Can several agents split up one big goal and work on it at once?"),
            Some("missions")
        );
        assert!(search("zebra quantum lasagna").is_empty());
    }

    #[test]
    fn reply_open_line_is_taken_off() {
        let (text, open) = split_reply("Use a race.\n\nOPEN: race-agents");
        assert_eq!(text, "Use a race.");
        assert_eq!(open.as_deref(), Some("race-agents"));
        let (_, none) = split_reply("Not covered.\nOPEN: none");
        assert!(none.is_none());
    }

    #[test]
    fn starters_have_answers() {
        for q in STARTERS {
            assert!(!search(q).is_empty(), "nothing in the guide for {q}");
        }
    }
}
