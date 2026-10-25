//! The Runo assistant: a mark in the corner of the window that answers questions about
//! Runo and what is happening in it. It reads a short guide built into the app
//! (`docs/guide`, one article per feature), a reference read from the app itself (menus,
//! commands, shortcuts and settings, see `assistant_ref`) and a snapshot of the app's
//! state, and the person's default agent on a fast model writes the answer. The answer can
//! offer actions (start or race agents, run a playbook, answer a permission request,
//! message or stop a session, go somewhere, run any command, change any setting), each a
//! card the person runs with a click (see `assistant_act`); nothing changes until they do.
//! With Guide only, it answers from the guide alone. It is off until turned on in Settings > General.

use super::*;
use crate::agent_pick::AgentPick;
use crate::assistant_act::{Action, Outcome, MAX_ACTIONS};
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
    ("data", include_str!("../../../docs/guide/data.md")),
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
    ("crew", include_str!("../../../docs/guide/crew.md")),
    ("git", include_str!("../../../docs/guide/git.md")),
    (
        "git-safety",
        include_str!("../../../docs/guide/git-safety.md"),
    ),
    ("cove", include_str!("../../../docs/guide/cove.md")),
    ("tour", include_str!("../../../docs/guide/tour.md")),
    ("usage", include_str!("../../../docs/guide/usage.md")),
    ("phone", include_str!("../../../docs/guide/phone.md")),
    (
        "assistant",
        include_str!("../../../docs/guide/assistant.md"),
    ),
    ("menu-bar", include_str!("../../../docs/guide/menu-bar.md")),
    (
        "quit-update",
        include_str!("../../../docs/guide/quit-update.md"),
    ),
    ("settings", include_str!("../../../docs/guide/settings.md")),
    ("projects", include_str!("../../../docs/guide/projects.md")),
    ("design", include_str!("../../../docs/guide/design.md")),
    (
        "architecture",
        include_str!("../../../docs/guide/architecture.md"),
    ),
    ("timeline", include_str!("../../../docs/guide/timeline.md")),
    ("notes", include_str!("../../../docs/guide/notes.md")),
    ("arcade", include_str!("../../../docs/guide/arcade.md")),
    ("inbox", include_str!("../../../docs/guide/inbox.md")),
    ("home", include_str!("../../../docs/guide/home.md")),
];

/// Questions offered when the panel opens with nothing asked yet.
pub(crate) const STARTERS: [&str; 4] = [
    "How do I race Claude Code and Codex on one task?",
    "How do I keep an agent running after I close my laptop?",
    "How do I share a playbook?",
    "How do I undo what the agent just did?",
];

/// Offered instead when an agent answers, which also knows what is happening in the app
/// and can do things in it.
pub(crate) const AGENT_STARTERS: [&str; 4] = [
    "What needs me right now?",
    "How are my running agents doing?",
    "Race Claude Code and Codex on a task",
    "How do I share a playbook?",
];

/// How long an answer may take before it is stopped.
const ANSWER_LIMIT: Duration = Duration::from_secs(120);

/// How many earlier messages go to the agent with each question.
const HISTORY_SENT: usize = 6;

/// How many articles go to the agent with each question.
const ARTICLES_SENT: usize = 4;

/// How many pages of Settings go to the agent with each question, besides the overview.
const PAGES_SENT: usize = 2;

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

/// How well a text matches the words asked: a keyword phrase in the question counts
/// most, then words of the title, the keywords and the text.
fn score(lower: &str, asked: &[String], title: &str, keywords: &[String], body: &str) -> u32 {
    let mut score = 0;
    for k in keywords {
        if k.contains(' ') && lower.contains(k.as_str()) {
            score += 6;
        }
    }
    let title = words(title);
    let keys: Vec<String> = keywords.iter().flat_map(|k| words(k)).collect();
    let body = words(body);
    for w in asked {
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
    score
}

/// Articles that match `query`, best first.
pub(crate) fn search(query: &str) -> Vec<&'static Article> {
    let lower = query.to_lowercase();
    let asked = words(query);
    let mut scored: Vec<(u32, &'static Article)> = guide()
        .iter()
        .map(|a| (score(&lower, &asked, &a.title, &a.keywords, &a.body), a))
        .filter(|(score, _)| *score > 1)
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, a)| a).collect()
}

/// The pages of Settings that match `query` best, at most `max`, best first. A page
/// counts only when several words of the question are on it.
fn search_pages(
    query: &str,
    pages: Vec<crate::assistant_ref::RefPage>,
    max: usize,
) -> Vec<crate::assistant_ref::RefPage> {
    let lower = query.to_lowercase();
    let asked = words(query);
    let mut scored: Vec<(u32, crate::assistant_ref::RefPage)> = pages
        .into_iter()
        .map(|p| (score(&lower, &asked, &p.title, &[], &p.body), p))
        .filter(|(score, _)| *score >= 2)
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().take(max).map(|(_, p)| p).collect()
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

/// The places the assistant can take the person, by the names guide articles' `open:`
/// lines and the agent's `open` actions use, with their button labels. Settings pages
/// are `settings:<page>` besides these.
const PLACES: &[(&str, &str, Jump)] = &[
    ("new-session", "Open New session", Jump::Action),
    ("home", "Go to Home", Jump::View(View::Home)),
    ("inbox", "Go to Inbox", Jump::View(View::Inbox)),
    ("sessions", "Go to Sessions", Jump::View(View::AllSessions)),
    (
        "isolated",
        "Go to Isolated sessions",
        Jump::View(View::Worktrees),
    ),
    ("review", "Go to Git", Jump::View(View::Review)),
    (
        "git-history",
        "Go to Git: History",
        Jump::View(View::GitHistory),
    ),
    (
        "branches",
        "Go to Git: Branches",
        Jump::View(View::Branches),
    ),
    ("github", "Go to GitHub", Jump::View(View::GitHub)),
    (
        "playbooks",
        "Go to Playbooks",
        Jump::View(View::Automations),
    ),
    ("history", "Go to History", Jump::View(View::History)),
    ("timeline", "Go to Timeline", Jump::View(View::Activity)),
    ("notes", "Go to Project notes", Jump::View(View::Notes)),
    ("design", "Go to Design", Jump::View(View::Design)),
    (
        "architecture",
        "Go to Architecture",
        Jump::View(View::Architecture),
    ),
    ("data", "Go to Data", Jump::View(View::Data)),
    (
        "settings",
        "Open Settings",
        Jump::Settings(SettingsPage::Overview),
    ),
    ("tour", "Start the tour", Jump::Tour),
    ("arcade", "Open the Arcade", Jump::Arcade),
    ("add-project", "Add a project", Jump::AddProject),
];

/// Where an article's `open:` value leads.
pub(crate) fn jump_for(open: &str) -> Option<Jump> {
    if let Some(page) = open.strip_prefix("settings:") {
        return SettingsPage::pages()
            .find(|p| p.label().eq_ignore_ascii_case(page))
            .map(Jump::Settings);
    }
    PLACES
        .iter()
        .find(|(id, _, _)| *id == open)
        .map(|(_, _, jump)| jump.clone())
}

/// The label of an article's "Take me there" button.
pub(crate) fn open_label(open: &str) -> String {
    if let Some(page) = open.strip_prefix("settings:") {
        let name = SettingsPage::pages()
            .find(|p| p.label().eq_ignore_ascii_case(page))
            .map(|p| p.label())
            .unwrap_or("Settings");
        return format!("Open Settings → {name}");
    }
    PLACES
        .iter()
        .find(|(id, _, _)| *id == open)
        .map(|(_, label, _)| label.to_string())
        .unwrap_or_else(|| "Go to Home".into())
}

/// The guide article about each place, so every place in Runo has one: a new place
/// doesn't build until it is given an article, and the tests check the article exists.
pub(crate) fn view_article(view: View) -> &'static str {
    match view {
        View::Home => "home",
        View::Design => "design",
        View::Architecture => "architecture",
        View::Data => "data",
        View::Session => "composer",
        View::NewSession => "new-session",
        View::Inbox => "inbox",
        View::AllSessions => "navigation",
        View::History => "history",
        View::Worktrees => "worktrees",
        View::WorktreeSpace => "worktrees",
        View::Settings => "settings",
        View::Notes => "notes",
        View::Activity => "timeline",
        View::Automations => "playbooks",
        View::GitHub => "github",
        View::Editor => "files-terminal",
        View::Review => "git",
        View::Branches => "git",
        View::GitHistory => "git",
    }
}

pub(crate) struct Msg {
    pub you: bool,
    /// What the answer offers to do, and what became of each.
    pub(crate) actions: Vec<(Action, Outcome)>,
    pub(crate) text: String,
    /// The guide articles the answer came from.
    pub(crate) sources: Vec<&'static str>,
    /// Where "Take me there" goes.
    pub(crate) open: Option<String>,
}

impl Msg {
    /// A message Runo writes itself, or the person's question to it.
    pub(crate) fn new(you: bool, text: String, actions: Vec<(Action, Outcome)>) -> Self {
        Self {
            you,
            actions,
            text,
            sources: Vec::new(),
            open: None,
        }
    }
}

pub(crate) struct Assistant {
    pub open: bool,
    input: Entity<TextareaState>,
    pub(crate) chat: Vec<Msg>,
    /// The question being answered by the agent, by its run key.
    pub(crate) asking: Option<String>,
    asked: u64,
    /// The agent answering the question being asked.
    answerer: &'static str,
    pub(crate) scroll: ScrollHandle,
    /// Nudges waved off or followed, so each shows once.
    pub(crate) nudged: HashSet<String>,
    /// Whether the pointer is over the mark in the corner, and since when.
    hover: Option<(bool, std::time::Instant)>,
    /// The answer so far, while the agent writes it.
    pub(crate) streamed: String,
    /// Stops the agent writing the answer.
    stopper: Option<agent::Stopper>,
    /// Chats and notes kept between runs.
    pub(crate) kept: crate::assistant_mem::Kept,
    /// Which kept chat the open one is.
    pub(crate) chat_id: u64,
    /// Whether the list of earlier chats and notes shows instead of the chat.
    pub(crate) chats_open: bool,
    /// Whether the panel is the big command deck in the middle of the window.
    pub(crate) deck: bool,
    /// Answers that came in while the panel was closed or the window was in the
    /// background, not read yet: the count on the corner mark.
    pub(crate) unread: usize,
}

impl Assistant {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<RunoApp>) -> (Self, Vec<Subscription>) {
        // A long question wraps and the box grows with it; Shift+Enter starts a new line.
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask how to do something in Runo…")
                .submit_on_enter(true)
                .auto_grow(1, 6)
        });
        let subs = vec![
            cx.subscribe_in(&input, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    let question = input.read(cx).value().trim().to_string();
                    if !question.is_empty() {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                        this.ask_runo(question, cx);
                    }
                }
                cx.notify();
            }),
        ];
        // Today's chat reopens as it was; an older one waits under Chats.
        let kept = crate::assistant_mem::load();
        let (chat, chat_id) = match kept.chats.first() {
            Some(c)
                if crate::assistant_mem::same_day(c.updated, chrono::Local::now().timestamp()) =>
            {
                (c.msgs.iter().map(crate::assistant_mem::msg).collect(), c.id)
            }
            _ => (
                Vec::new(),
                kept.chats.iter().map(|c| c.id).max().unwrap_or(0) + 1,
            ),
        };
        (
            Self {
                open: false,
                input,
                chat,
                kept,
                chat_id,
                chats_open: false,
                deck: false,
                unread: 0,
                asking: None,
                asked: 0,
                answerer: "",
                scroll: ScrollHandle::new(),
                hover: None,
                streamed: String::new(),
                stopper: None,
                nudged: HashSet::new(),
            },
            subs,
        )
    }
}

/// How the answering agent offers to do something, and what it can offer.
fn actions_help() -> String {
    let places: Vec<&str> = PLACES.iter().map(|(id, _, _)| *id).collect();
    format!(
        "You can also act in Runo. To offer an action, add a line that starts with ACTION: \
followed by one JSON object on the same line. The person sees each action as a card and runs it \
with a click, so also say in your text what you're offering. Offer at most 3, and only when the \
person asks for something to be done or one clearly helps; when they ask you to do something \
Runo can do, offer it rather than only explaining. Use only the session ids, request ids, agent \
ids, project and playbook names in the state above, and only command ids and setting names from \
the reference. The actions:\n\
ACTION: {{\"do\":\"start\",\"task\":\"...\",\"agent\":\"codex\",\"isolated\":true,\"project\":\"...\"}} \
start one agent on a task; agent, isolated (own worktree, the default) and project are optional\n\
ACTION: {{\"do\":\"race\",\"task\":\"...\",\"agents\":[\"claude\",\"codex\"]}} several agents on \
the same task, each in its own worktree\n\
ACTION: {{\"do\":\"playbook\",\"name\":\"...\"}} run a playbook\n\
ACTION: {{\"do\":\"send\",\"session\":12,\"text\":\"...\"}} send a message to a session's agent\n\
ACTION: {{\"do\":\"stop\",\"session\":12}} stop a running agent\n\
ACTION: {{\"do\":\"approve\",\"request\":\"id\"}} or {{\"do\":\"deny\",\"request\":\"id\"}} answer a \
permission request once\n\
ACTION: {{\"do\":\"session\",\"id\":12}} open a session\n\
ACTION: {{\"do\":\"project\",\"name\":\"...\"}} switch project\n\
ACTION: {{\"do\":\"command\",\"id\":\"quit\"}} run any command from the reference's Quick open \
list by its id, such as quit (quits Runo for real, after the usual warning if agents are \
running), arcade, toggle-terminal or go-design\n\
ACTION: {{\"do\":\"open\",\"place\":\"inbox\"}} go to {} or settings:<page>\n\
ACTION: {{\"do\":\"setting\",\"page\":\"Appearance\",\"name\":\"Theme\",\"value\":\"Dark\"}} \
change any setting the reference lists, by its page and its name exactly as listed: true or false \
for a switch, one of the listed choices for the rest. For a button, leave out value to press it, \
such as {{\"do\":\"setting\",\"page\":\"General\",\"name\":\"Check for updates\"}}. Text \
settings can't be changed this way.\n\
ACTION: {{\"do\":\"remember\",\"note\":\"...\"}} keep a preference or fact the person states \
for later chats in this project, like how they like work started; offer it when they say \
\"always\", \"never\" or \"remember\"\n",
        places.join(", ")
    )
}

/// The prompt the answering agent gets: what is happening in the app, what is on screen,
/// the guide articles that match, how to answer, and what it can offer to do.
#[allow(clippy::too_many_arguments)]
fn prompt(
    question: &str,
    articles: &[&Article],
    reference: &str,
    context: &str,
    state: &str,
    history: &str,
    notes: &str,
    cx: &App,
) -> String {
    let titles: Vec<&str> = guide().iter().map(|a| a.title.as_str()).collect();
    let mut text = String::from(
        "You are Runo's assistant, inside Runo, a desktop app that runs coding agents side by \
         side. You know all of Runo: its reference below is read from the app itself (its menus, \
         every command, every shortcut and every setting with its value now), and the guide \
         articles below explain how its features work. You answer what is happening in the app \
         from the state below, and you can offer to do things in it. Reply in a few short \
         sentences or numbered steps, plain Markdown, no headings. Name menus, buttons, commands, \
         shortcuts and settings exactly as the reference and the guide do, with the keys the \
         reference gives, and sessions by their title. Answer from the reference and the guide; \
         only when neither covers something, say so in one sentence rather than guessing. Don't \
         run tools or look at files. On the last line, write OPEN: and the id of the one guide \
         article whose place in the app would help most, or OPEN: none.\n\n",
    );
    text.push_str(&format!("What is on screen now: {context}\n\n"));
    text.push_str(&format!("State of the app now:\n{state}\n"));
    text.push_str(&format!("Runo's reference:\n{reference}\n"));
    text.push_str(&actions_help());
    text.push('\n');
    if !notes.is_empty() {
        text.push_str(&format!("{notes}\n"));
    }
    if !history.is_empty() {
        text.push_str(&format!("The conversation so far:\n{history}\n"));
    }
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

/// The agent's reply without its `OPEN:` and `ACTION:` lines, the article id the first
/// named, and the actions the others offer.
fn split_reply(reply: &str) -> (String, Option<String>, Vec<Action>) {
    let mut open = None;
    let mut actions = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    for line in reply.trim().lines() {
        if line.trim().starts_with("ACTION:") {
            if let Some(a) = assistant_act::parse(line) {
                if actions.len() < MAX_ACTIONS && !actions.contains(&a) {
                    actions.push(a);
                }
            }
        } else if let Some(id) = line.trim().strip_prefix("OPEN:") {
            let id = id.trim().trim_matches('`');
            if id != "none" && !id.is_empty() {
                open = Some(id.to_string());
            }
        } else {
            lines.push(line);
        }
    }
    (lines.join("\n").trim().to_string(), open, actions)
}

/// The agent that answers: the one chosen in Settings, or the default agent.
pub(crate) fn assistant_provider(st: &AppSettings) -> usize {
    let id = if st.assistant_agent.is_empty() {
        &st.default_provider
    } else {
        &st.assistant_agent
    };
    agent::PROVIDERS
        .iter()
        .position(|p| p.id == id)
        .unwrap_or(0)
}

/// The model it answers on: the one chosen in Settings, or a small, fast one where Runo
/// knows one.
pub(crate) fn assistant_model(st: &AppSettings, provider: &str) -> String {
    match st.assistant_model.as_str() {
        "" if provider == "claude" => "haiku".to_string(),
        m => m.to_string(),
    }
}

impl RunoApp {
    /// Whether an agent answers, so the assistant knows the app's state and can act.
    pub(crate) fn assistant_acts(&self, cx: &App) -> bool {
        let st = cx.global::<AppSettings>();
        let p = &agent::PROVIDERS[assistant_provider(st)];
        st.assistant_answers == "agent" && agent::available(p.bin, &st.agent(p.id).binary)
    }

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
            self.assistant.unread = 0;
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

    /// Runo's reference sent with a question: always the overview, then the pages of
    /// Settings that match the question best.
    fn assistant_reference(&self, question: &str, cx: &App) -> String {
        let data = self.discovered.clone().unwrap_or_default();
        let mut out = crate::assistant_ref::overview(cx);
        let pages = crate::assistant_ref::pages(&data, cx);
        for page in search_pages(question, pages, PAGES_SENT) {
            out.push_str(&format!("\n## {}\n{}", page.title, page.body));
        }
        out
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
            View::WorktreeSpace => "a project's worktrees and their disk space",
            View::Settings => "Settings",
            View::Notes => "Project notes",
            View::Activity => "Timeline",
            View::Automations => "Playbooks",
            View::GitHub => "GitHub",
            View::Editor => "a file",
            View::Review => "Review",
            View::Branches => "Git branches",
            View::GitHistory => "Git history",
            View::Design => "Design",
            View::Architecture => "Architecture",
            View::Data => "Data",
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
        parts.push(format!("{} agents running", self.turns.len()));
        parts.join("; ")
    }

    /// The last few exchanges before the question just asked, so a follow-up like "do
    /// that" or "and the other one?" makes sense.
    fn assistant_history(&self) -> String {
        let chat = &self.assistant.chat;
        let earlier = &chat[..chat.len().saturating_sub(1)];
        earlier
            .iter()
            .rev()
            .take(HISTORY_SENT)
            .rev()
            .map(|m| {
                let text: String = m.text.chars().take(600).collect();
                let mut line = format!("{}: {}", if m.you { "Person" } else { "You" }, text);
                // What it offered, and what the person did with it.
                for (action, outcome) in &m.actions {
                    let what = match outcome {
                        Outcome::Ready => "not run yet".to_string(),
                        Outcome::Done(_) => "the person ran it".to_string(),
                        Outcome::Failed(why) => format!("it failed: {why}"),
                        Outcome::Dismissed => "the person dismissed it".to_string(),
                    };
                    let json = serde_json::to_string(action).unwrap_or_default();
                    line.push_str(&format!("\n  (offered ACTION: {json}; {what})"));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Answer `question`: from the agent when one is set to answer, otherwise from the guide.
    pub(crate) fn ask_runo(&mut self, question: String, cx: &mut Context<Self>) {
        if self.assistant.asking.is_some() {
            return;
        }
        self.assistant.chats_open = false;
        if self.assistant_command(&question, cx) {
            return;
        }
        let found = search(&question);
        self.assistant.chat.push(Msg {
            you: true,
            actions: Vec::new(),
            text: question.clone(),
            sources: Vec::new(),
            open: None,
        });
        self.assistant.scroll.scroll_to_bottom();
        if !self.assistant_acts(cx) {
            self.answer_from_guide(&found, None);
            return;
        }
        let st = cx.global::<AppSettings>();
        let provider = &agent::PROVIDERS[assistant_provider(st)];
        let model = assistant_model(st, provider.id);
        self.assistant.answerer = provider.name;
        // With nothing matching, the article about the place on screen.
        let articles: Vec<&Article> = if found.is_empty() {
            article(view_article(self.view)).into_iter().collect()
        } else {
            found.iter().take(ARTICLES_SENT).copied().collect()
        };
        let text = prompt(
            &question,
            &articles,
            &self.assistant_reference(&question, cx),
            &self.assistant_context(),
            &format!(
                "{}{}",
                self.assistant_snapshot(cx),
                Some(self.assistant_mentioned(&question))
                    .filter(|m| !m.is_empty())
                    .map(|m| format!("Named in the question with @:\n{m}"))
                    .unwrap_or_default()
            ),
            &self.assistant_history(),
            &self.assistant_notes(),
            cx,
        );
        let configured = st.agent(provider.id).binary;
        let dir = store::data_dir().join("assistant");
        let _ = std::fs::create_dir_all(&dir);
        let dir = dir.to_string_lossy().to_string();
        let (command, parser) = if provider.id == "claude" {
            // Its reply streams in as it is written.
            let bin = if configured.trim().is_empty() {
                provider.bin
            } else {
                configured.trim()
            };
            let mut command = std::process::Command::new(bin);
            command
                .args(["--model", &model, "-p", "--output-format", "stream-json"])
                .args(["--verbose", "--include-partial-messages"])
                .args(["--permission-mode", "plan", "--", &text]);
            let parser = stream::Parser::Claude(stream::ClaudeStream::new(&dir));
            (command, Some(parser))
        } else {
            (
                agent::ask_command_on(provider.id, &configured, &text, &model),
                None,
            )
        };
        self.assistant.asked += 1;
        let key = format!("assistant:{}", self.assistant.asked);
        self.assistant.asking = Some(key.clone());
        self.assistant.streamed.clear();
        self.assistant.stopper = Some(agent::oneshot_streamed(
            key,
            command,
            &dir,
            parser,
            ANSWER_LIMIT,
            self.tx.clone(),
        ));
    }

    /// More of the answer being written arrived.
    pub(crate) fn assistant_partial(&mut self, key: String, text: String) {
        if self.assistant.asking.as_ref() == Some(&key) {
            self.assistant.streamed = text;
            // Follow the answer only while the panel is at the bottom, so reading back
            // isn't interrupted.
            if crate::chat_follow::at_bottom(&self.assistant.scroll) {
                self.assistant.scroll.scroll_to_bottom();
            }
        }
    }

    /// Stop the agent writing the answer; what it wrote so far stays as the answer.
    pub(crate) fn assistant_stop(&mut self) {
        if let Some(stopper) = self.assistant.stopper.take() {
            stopper.stop();
        }
    }

    /// The best matching article as the answer. Its `{shortcut}` tags are filled in when
    /// it is drawn, so they follow the person's keys.
    fn answer_from_guide(&mut self, found: &[&'static Article], note: Option<String>) {
        let msg = match found.first() {
            Some(a) => Msg {
                you: false,
                actions: Vec::new(),
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
                actions: Vec::new(),
                text: "I couldn't find that in the guide. Try asking about sessions, worktrees, \
                       racing agents, reviewing changes, playbooks or connections."
                    .into(),
                sources: Vec::new(),
                open: None,
            },
        };
        self.assistant.chat.push(msg);
        self.assistant.scroll.scroll_to_bottom();
        self.assistant_keep();
    }

    /// The answering agent finished.
    pub(crate) fn assistant_reply(&mut self, key: String, result: Result<String, String>) {
        if self.assistant.asking.as_ref() != Some(&key) {
            return;
        }
        self.assistant.asking = None;
        self.assistant.stopper = None;
        self.assistant.streamed.clear();
        if !self.assistant.open || !self.window_active {
            self.assistant.unread += 1;
        }
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
                let (text, open_id, offered) = split_reply(&reply);
                let opened = open_id.as_deref().and_then(article);
                let mut sources: Vec<&'static str> = found.iter().take(2).map(|a| a.id).collect();
                if let Some(a) = opened {
                    if !sources.contains(&a.id) {
                        sources.insert(0, a.id);
                    }
                }
                // A card that can't be done says why, rather than vanishing.
                let actions = offered.into_iter().map(|a| (a, Outcome::Ready)).collect();
                self.assistant.chat.push(Msg {
                    you: false,
                    actions,
                    text,
                    sources,
                    open: opened
                        .or(found.first().copied())
                        .and_then(|a| a.open.clone()),
                });
                self.assistant.scroll.scroll_to_bottom();
                self.assistant_keep();
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

    /// Run answer `i`'s action `j`, once.
    fn assistant_run(&mut self, i: usize, j: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self
            .assistant
            .chat
            .get(i)
            .and_then(|m| m.actions.get(j))
            .filter(|(_, o)| *o == Outcome::Ready)
            .map(|(a, _)| a.clone())
        else {
            return;
        };
        let outcome = match self.run_action(&action, window, cx) {
            Ok(done) => Outcome::Done(done),
            Err(why) => Outcome::Failed(why),
        };
        if let Some(slot) = self
            .assistant
            .chat
            .get_mut(i)
            .and_then(|m| m.actions.get_mut(j))
        {
            slot.1 = outcome;
        }
        self.assistant_keep();
        cx.notify();
    }

    /// One thing an answer offers to do, as a card: what it does, written from Runo's own
    /// state, and a button to do it. Once run, it says how that went.
    fn assistant_action(
        &self,
        i: usize,
        j: usize,
        action: &Action,
        outcome: &Outcome,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = i * MAX_ACTIONS + j;
        let card = self.action_card(action, cx);
        let (icon_name, title, detail, changes) = match &card {
            Ok(c) => (c.icon, c.title.clone(), c.detail.clone(), c.changes),
            Err(why) => (
                IconName::CircleAlert,
                "Can't do this now".into(),
                why.clone(),
                false,
            ),
        };
        let tone = match outcome {
            Outcome::Done(_) => c_green(),
            Outcome::Failed(_) => c_red(),
            _ if card.is_err() => c_muted(),
            _ => c_blue(),
        };
        let end: AnyElement = match outcome {
            Outcome::Ready if card.is_ok() => div()
                .flex()
                .gap_1()
                .child(
                    self.ghost_button_sized(("assistant-dismiss", id), IconName::X, "Dismiss", 28.)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(slot) = this
                                .assistant
                                .chat
                                .get_mut(i)
                                .and_then(|m| m.actions.get_mut(j))
                            {
                                slot.1 = Outcome::Dismissed;
                            }
                            this.assistant_keep();
                            cx.notify();
                        })),
                )
                .child(
                    self.button(
                        ("assistant-run", id),
                        if changes { "Run" } else { "Go" },
                        true,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.assistant_run(i, j, window, cx);
                    })),
                )
                .into_any_element(),
            Outcome::Done(_) => icon(IconName::CircleCheck, 15., c_green()).into_any_element(),
            Outcome::Failed(_) => icon(IconName::CircleX, 15., c_red()).into_any_element(),
            Outcome::Dismissed => div()
                .text_size(px(11.))
                .text_color(rgb(c_muted()))
                .child("Dismissed")
                .into_any_element(),
            Outcome::Ready => div().into_any_element(),
        };
        let note = match outcome {
            Outcome::Failed(why) => Some(why.clone()),
            _ => None,
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(10.))
            .py(px(8.))
            .rounded_lg()
            .border_1()
            .border_color(if matches!(outcome, Outcome::Ready) && card.is_ok() {
                glow(tone, 0.45)
            } else {
                divider()
            })
            .bg(rgb(c_raised()))
            .when(
                matches!(outcome, Outcome::Dismissed) || card.is_err(),
                |el| el.opacity(0.6),
            )
            .child(
                div()
                    .size(px(26.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .bg(glow(tone, 0.12))
                    .child(icon(icon_name, 13., tone)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(!detail.is_empty(), |el| {
                        el.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(detail),
                        )
                    })
                    .when_some(note, |el, why| {
                        el.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_red()))
                                .child(why),
                        )
                    }),
            )
            .child(end)
            .into_any_element()
    }

    /// Commands after `/` and names after `@`, over the input; a click fills it in.
    fn assistant_suggest_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let typed = self.assistant.input.read(cx).value().to_string();
        let found = self.assistant_suggestions(&typed);
        if found.is_empty() {
            return None;
        }
        let mut list = div().p_1().v_flex();
        for (i, s) in found.into_iter().enumerate() {
            let fill = s.fill.clone();
            list = list.child(
                menu_row(("assistant-suggest", i), s.label, s.detail, false, None).on_click(
                    cx.listener(move |this, _, window, cx| {
                        let fill = fill.clone();
                        this.assistant.input.update(cx, |input, cx| {
                            input.set_value(fill, window, cx);
                            input.focus(window, cx);
                        });
                        cx.notify();
                    }),
                ),
            );
        }
        Some(
            div()
                .absolute()
                .bottom(relative(1.))
                .mb_1()
                .left_0()
                .right_0()
                .child(menu_box("assistant-suggest", list))
                .into_any_element(),
        )
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
    pub(crate) fn assistant_view(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.assistant_shown(cx) {
            return None;
        }
        // Only the assistant's own answers, not yet read; everything else that needs the
        // person is counted on the Dock icon.
        let unread = self.assistant.unread;
        let state = if self.assistant.asking.is_some() {
            Mood::Thinking
        } else if self.assistant.open {
            Mood::Listening
        } else if unread > 0 {
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
            .tooltip(move |window, cx| {
                let keys = keymap::label(cx, "ask-runo");
                gpui_kit::component::tooltip::Tooltip::new(format!("{}  {keys}", mark_tip(unread)))
                    .build(window, cx)
            })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_assistant(window, cx)))
            .child(live_mark(
                state,
                (unread > 0).then_some(unread),
                self.assistant.hover,
            ));
        let mut layer = div().absolute().top_0().left_0().size_full().child(mark);
        if self.assistant.open {
            layer = layer.child(self.assistant_panel(window, cx));
        } else if let Some(nudge) = self.assistant_nudge_view(cx) {
            layer = layer.child(nudge);
        }
        Some(layer.into_any_element())
    }

    /// The chat and the answer being written, for this panel and the menu bar's.
    pub(crate) fn assistant_messages(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let mut out = Vec::new();
        let chat = &self.assistant.chat;
        for (i, m) in chat.iter().enumerate() {
            if m.you {
                out.push(
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
            for (j, (action, outcome)) in m.actions.iter().enumerate() {
                answer = answer.child(self.assistant_action(i, j, action, outcome, cx));
            }
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
            // Cards say where they go themselves; "Take me there" is for answers without.
            if let Some(open) = m
                .open
                .clone()
                .filter(|o| m.actions.is_empty() && jump_for(o).is_some())
            {
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
            out.push(answer);
        }
        if self.assistant.asking.is_some() && !self.assistant.chats_open {
            let (so_far, _, _) = split_reply(&self.assistant.streamed);
            if !so_far.is_empty() {
                out.push(
                    div().text_size(px(12.5)).child(
                        TextView::markdown("assistant-streaming", with_live_keys(&so_far, cx))
                            .selectable(true),
                    ),
                );
            }
            out.push(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(thinking_line(if so_far.is_empty() {
                        "Looking it up"
                    } else {
                        "Writing"
                    }))
                    .child(
                        div()
                            .id("assistant-stop")
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py(px(3.))
                            .rounded_md()
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                            .child(icon(IconName::CircleStop, 12., c_muted()))
                            .child("Stop")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.assistant_stop();
                                cx.notify();
                            })),
                    ),
            );
        }

        out
    }

    fn assistant_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let thinking = self.assistant.asking.is_some();
        let mood = if thinking {
            Mood::Thinking
        } else {
            Mood::Listening
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(header_mark(mood))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Runo"))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(if thinking { pose(mood).1 } else { c_muted() }))
                    .child(if thinking {
                        "Thinking…"
                    } else if self.assistant_acts(cx) {
                        "Ask, or tell me what to do"
                    } else {
                        "Ask how to do anything in Runo"
                    }),
            )
            .child(div().flex_1())
            .child(
                self.ghost_button(
                    "assistant-chats",
                    IconName::RotateCcwClock,
                    "Chats and what I remember",
                )
                .when(self.assistant.chats_open, |el| el.bg(tint(c_subtle())))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.assistant.chats_open = !this.assistant.chats_open;
                    cx.notify();
                })),
            )
            .child(
                self.ghost_button(
                    "assistant-deck",
                    if self.assistant.deck {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    },
                    if self.assistant.deck {
                        "Back to the corner"
                    } else {
                        "Command deck"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.assistant.deck = !this.assistant.deck;
                    cx.notify();
                })),
            )
            .when(!self.assistant.chat.is_empty(), |el| {
                el.child(
                    self.ghost_button("assistant-clear", IconName::MessageSquarePlus, "New chat")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.assistant_new_chat();
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
        let header = div()
            .px_4()
            .pt_3()
            .pb(px(10.))
            .v_flex()
            .gap(px(10.))
            .border_b_1()
            .border_color(divider())
            .child(header)
            .child(self.assistant_telemetry());

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
        let acts = self.assistant_acts(cx);
        let deck = self.assistant.deck;
        if self.assistant.chats_open {
            body = body.child(self.assistant_chats_view(cx));
        } else if self.assistant.chat.is_empty() {
            body = body
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(crate::home::greeting()),
                )
                .child(self.assistant_briefing(if deck { 3 } else { 2 }, cx))
                .child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(rgb(c_secondary()))
                        .child(if acts {
                            "I know Runo's features and what's happening in it. Ask how to do something, what needs you, or tell me what to do; I'll ask before changing anything."
                        } else {
                            "I know Runo's features from its built-in guide. Ask how to do something and I'll explain it and take you there."
                        }),
                )
                .child(
                    div()
                        .pt_1()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child("TRY ASKING"),
                );
            let starters: &'static [&str] = if acts { &AGENT_STARTERS } else { &STARTERS };
            for (i, q) in starters.iter().enumerate() {
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
        if !self.assistant.chats_open {
            body = body.children(self.assistant_messages(cx));
        }

        // Which agent answers, on which model, as chosen in Settings; or the guide.
        let answers = {
            let st = cx.global::<AppSettings>();
            if st.assistant_answers == "agent" {
                let i = assistant_provider(st);
                let name = agent::PROVIDERS[i].name;
                if acts {
                    let model = assistant_model(st, agent::PROVIDERS[i].id);
                    let label = if model == "haiku" {
                        Some("Haiku".to_string())
                    } else {
                        AgentPick::model_label(i, &model)
                    };
                    match label {
                        Some(label) => format!("Answers with {name} · {label}"),
                        None => format!("Answers with {name}"),
                    }
                } else {
                    format!("{name} isn't installed; answering from the guide")
                }
            } else {
                "Answers from the guide".to_string()
            }
        };
        // The input sits in a box like the session composer's: one soft rounded frame on
        // the raised surface, its edge taking the accent, with a faint ring, while focused.
        let focused = self
            .assistant
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let accent = c_blue();
        let footer = div()
            .px_4()
            .pt_1()
            .pb_3()
            .v_flex()
            .gap(px(6.))
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .px_3()
                            .py(px(8.))
                            .rounded(px(12.))
                            .border_1()
                            .border_color(if focused {
                                glow(accent, 0.6)
                            } else {
                                divider()
                            })
                            .bg(raised())
                            .when(focused, |el| {
                                el.shadow(vec![halo(
                                    accent,
                                    if is_dark() { 0.22 } else { 0.14 },
                                    0.,
                                    3.,
                                    0.,
                                )])
                            })
                            .text_size(px(13.))
                            .line_height(px(19.))
                            .child(
                                Textarea::new(&self.assistant.input)
                                    .appearance(false)
                                    .bordered(false),
                            ),
                    )
                    .children(self.assistant_suggest_view(cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    // In line with the text in the box above.
                    .px(px(13.))
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(answers)
                    .child(if acts {
                        "Asks before changing anything"
                    } else {
                        "Never changes anything"
                    }),
            );

        // The ring Home's prompt wears: a gradient a hair larger than the panel, the
        // accent turning violet, with a soft glow of the accent under it.
        let accent = c_blue();
        let ring_to = settings::mix(RING_VIOLET, accent, 0.6);
        let panel = div()
            .id("assistant-panel")
            .when(!deck, |el| el.absolute().right(px(18.)).bottom(px(96.)))
            .w(px(if deck { 780. } else { 400. }))
            .max_w(relative(0.9))
            .h(px(if deck { 640. } else { 520. }))
            .max_h(relative(if deck { 0.86 } else { 0.75 }))
            .p(px(1.5))
            .rounded(px(13.5))
            .bg(linear_gradient(
                160.,
                linear_color_stop(glow(accent, 0.85), 0.),
                linear_color_stop(glow(ring_to, 0.75), 1.),
            ))
            .shadow(vec![
                halo(0x000000, if is_dark() { 0.45 } else { 0.16 }, 48., 0., 18.),
                halo(accent, if is_dark() { 0.3 } else { 0.2 }, 40., 0., 8.),
            ])
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .size_full()
                    .v_flex()
                    .rounded(px(12.))
                    .overflow_hidden()
                    .bg(float(c_canvas()))
                    .child(header)
                    .child(body)
                    .child(footer),
            );
        if deck {
            // The deck sits in the middle over a dimmed window; a click outside it puts it
            // back in the corner.
            return div()
                .id("assistant-deck")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::hsla(0., 0., 0., if is_dark() { 0.45 } else { 0.25 }))
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.assistant.deck = false;
                    cx.notify();
                }))
                .child(panel.with_animation(
                    "assistant-deck-in",
                    Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                    |el, d| el.opacity(d).mt(px(12. * (1. - d))),
                ))
                .into_any_element();
        }
        // It opens up out of the mark: fading in while rising the last few pixels.
        panel
            .with_animation(
                "assistant-panel-in",
                Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                |el, d| el.opacity(d).bottom(px(96. - 10. * (1. - d))),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Mood {
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
    let alive = mood != Mood::Idle;
    let animation = if alive {
        Animation::new(Duration::from_millis(1400)).repeat()
    } else {
        Animation::new(Duration::from_millis(MORPH_MS.max(LIFT_MS) as u64))
    };
    let key = format!("runo-mark-{mood:?}-{since:?}-{:?}", hover);
    let el = div().relative().size(px(46.)).looping(
        ElementId::Name(key.into()),
        animation,
        0.,
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
            // Something waiting makes the glow breathe.
            let swell = if mood == Mood::NeedsYou {
                (d * std::f32::consts::TAU).sin() * 0.5 + 0.5
            } else {
                0.
            };
            el.child(div().absolute().top(px(-2. * lift)).child(lit_mark(
                mood,
                46.,
                bars,
                alive.then(|| turn(mood)),
                swell,
            )))
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

/// The corner mark's tooltip: what it opens, and how many answers wait in it.
fn mark_tip(unread: usize) -> String {
    match unread {
        0 => "Ask Runo".into(),
        1 => "Runo answered".into(),
        n => format!("{n} answers from Runo"),
    }
}

/// The count of the assistant's unread answers, on the mark's corner.
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
        .child(crate::dock::label(n).unwrap_or_default())
}

/// Runo's icon, three staggered bars on a rounded square, posed for `mood`. At `size` 46 it
/// is the mark in the corner; smaller in the panel header.
fn mark(mood: Mood, size: f32, badge: Option<usize>) -> AnyElement {
    mark_with(mood, size, pose(mood).0, badge, false)
}

/// The icon with its bars at `bars`, as (left, width) at size 46.
/// While a light runs round its edge (`running`), the edge itself steps back so the light
/// stands out on it.
fn mark_with(mood: Mood, size: f32, bars: Bars, badge: Option<usize>, running: bool) -> AnyElement {
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
            _ if running => settings::mix(c_border(), middle, 0.55),
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
/// The violet Home's prompt ring turns into.
pub(crate) const RING_VIOLET: u32 = 0x8b5cf6;
/// How long the light takes to run once round the mark's edge; thinking runs it three
/// times as fast.
const RUN_MS: f32 = 4200.;

/// `color` at opacity `a`.
pub(crate) fn glow(color: u32, a: f32) -> Rgba {
    rgba((color << 8) | (a.clamp(0., 1.) * 255.).round() as u32)
}

/// A soft shadow of `color`, `lift` pixels below what casts it.
pub(crate) fn halo(color: u32, a: f32, blur: f32, spread: f32, lift: f32) -> BoxShadow {
    BoxShadow {
        color: glow(color, a).into(),
        offset: point(px(0.), px(lift)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

/// How far round the mark's edge the light has run, as a fraction of a lap. Read from
/// the clock, so it never jumps back when the mark is drawn again.
fn turn(mood: Mood) -> f32 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let ms = ms_since(*START.get_or_init(std::time::Instant::now));
    let lap = if mood == Mood::Thinking {
        RUN_MS / 3.
    } else {
        RUN_MS
    };
    (ms / lap).fract()
}

/// The point `t` of the way round a `w` square with corners of radius `r`, clockwise from
/// the middle of its top edge.
fn round_square_at(w: f32, r: f32, t: f32) -> (f32, f32) {
    let side = w - 2. * r;
    let bend = std::f32::consts::FRAC_PI_2 * r;
    let lap = 4. * (side + bend);
    let mut d = (t.rem_euclid(1.) * lap + side / 2.).rem_euclid(lap);
    // Each side, then the corner after it, starting along the top.
    let corners = [(w - r, r), (w - r, w - r), (r, w - r), (r, r)];
    let starts = [(r, 0.), (w, r), (w - r, w), (0., w - r)];
    let dirs = [(1., 0.), (0., 1.), (-1., 0.), (0., -1.)];
    for i in 0..4 {
        if d <= side {
            return (starts[i].0 + dirs[i].0 * d, starts[i].1 + dirs[i].1 * d);
        }
        d -= side;
        if d <= bend {
            let a = (i as f32 - 1.) * std::f32::consts::FRAC_PI_2 + d / r;
            return (corners[i].0 + r * a.cos(), corners[i].1 + r * a.sin());
        }
        d -= bend;
    }
    (w / 2., 0.)
}

/// The mark, lit: a glow of its mood's colour under it and, while it is alive, a light
/// running round its edge with a tail that fades behind it. `run` is how far round the
/// light is; `swell` how far the glow has breathed out, 0 to 1.
fn lit_mark(mood: Mood, size: f32, bars: Bars, run: Option<f32>, swell: f32) -> AnyElement {
    let s = size / 46.;
    let light = match mood {
        Mood::Idle => return mark_with(mood, size, bars, None, false),
        Mood::Listening => c_blue(),
        Mood::Thinking => VIOLET,
        Mood::NeedsYou => c_amber(),
    };
    let strength = if is_dark() { 0.55 } else { 0.32 };
    // The light's head: the mood's colour lifted towards white on dark themes.
    let head = if is_dark() {
        settings::mix(0xffffff, light, 0.35)
    } else {
        light
    };
    let mut layer = div()
        .relative()
        .size(px(size))
        .rounded(px(14. * s))
        .shadow(vec![halo(
            light,
            strength + 0.18 * swell,
            20. * s,
            5. * s * swell,
            1. * s,
        )])
        .child(mark_with(mood, size, bars, None, run.is_some()));
    if let Some(run) = run {
        layer = layer.child(
            div().absolute().inset_0().child(
                canvas(
                    |_, _, _| (),
                    move |b, _, window, _| {
                        let (w, r) = (size - 1., 14. * s - 0.5);
                        let at = |t: f32| {
                            let (x, y) = round_square_at(w, r, t);
                            point(b.origin.x + px(x + 0.5), b.origin.y + px(y + 0.5))
                        };
                        // The head, bright, then two lengths of tail, each fainter.
                        for (from, to, color, a) in [
                            (0.2, 0.3, head, 1.),
                            (0.1, 0.2, light, 0.8),
                            (0., 0.1, light, 0.35),
                        ] {
                            let mut f = PathBuilder::stroke(px(2.4 * s));
                            let steps = 16;
                            for i in 0..=steps {
                                let t = run + from + (to - from) * i as f32 / steps as f32;
                                if i == 0 {
                                    f.move_to(at(t));
                                } else {
                                    f.line_to(at(t));
                                }
                            }
                            if let Ok(path) = f.build() {
                                window.paint_path(path, Hsla::from(glow(color, a)));
                            }
                        }
                    },
                )
                .size_full(),
            ),
        );
        // A spark at the head, glowing.
        let (x, y) = round_square_at(size - 1., 14. * s - 0.5, run + 0.3);
        let d = 4. * s;
        layer = layer.child(
            div()
                .absolute()
                .left(px(x + 0.5 - d / 2.))
                .top(px(y + 0.5 - d / 2.))
                .size(px(d))
                .rounded_full()
                .bg(rgb(head))
                .shadow(vec![halo(light, 0.9, 8. * s, 1. * s, 0.)]),
        );
    }
    layer.into_any_element()
}

/// The mark in the panel's header, with the light running round it while the panel is
/// open.
fn header_mark(mood: Mood) -> AnyElement {
    live_mark_sized(mood, 24.)
}

/// Runo's mark at `size`, its light running round while `mood` is lively.
pub(crate) fn live_mark_sized(mood: Mood, size: f32) -> AnyElement {
    if !anim_on() {
        return lit_mark(mood, size, pose(mood).0, None, 0.);
    }
    div()
        .size(px(size))
        .with_animation(
            ElementId::Name(format!("assistant-header-mark-{mood:?}-{size}").into()),
            Animation::new(Duration::from_millis(1400)).repeat(),
            move |el, _| el.child(lit_mark(mood, size, pose(mood).0, Some(turn(mood)), 0.)),
        )
        .into_any_element()
}

/// While the agent works on an answer: the line it always showed, and a caret in the
/// accent that blinks after it.
fn thinking_line(label: &'static str) -> AnyElement {
    let caret = div()
        .w(px(6.))
        .h(px(13.))
        .rounded(px(1.))
        .bg(rgb(c_blue()))
        .shadow(vec![halo(c_blue(), 0.6, 8., 0., 0.)]);
    let caret = if anim_on() {
        caret
            .with_animation(
                "assistant-caret",
                Animation::new(Duration::from_millis(1000)).repeat(),
                |el, d| el.opacity(if d < 0.5 { 1. } else { 0.15 }),
            )
            .into_any_element()
    } else {
        caret.into_any_element()
    };
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(12.5))
        .text_color(rgb(c_muted()))
        .child(label)
        .child(caret)
        .into_any_element()
}

/// One reading on the panel's status row, in the look of the chips under an answer: a
/// lit dot when `lit`, the value, and what it counts.
fn readout(lit: Option<u32>, value: String, what: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(7.))
        .py(px(2.))
        .rounded_md()
        .border_1()
        .border_color(divider())
        .text_size(px(11.))
        .text_color(rgb(c_muted()))
        .when_some(lit, |el, c| {
            el.child(
                div()
                    .size(px(6.))
                    .rounded_full()
                    .bg(rgb(c))
                    .shadow(vec![halo(c, 0.7, 6., 0., 0.)]),
            )
        })
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(c_text()))
                .child(value),
        )
        .child(what)
}

impl RunoApp {
    /// The status row under the panel's header: agents running, what waits on the
    /// person, the session's branch and the clashes the radar found. Only what is already
    /// in memory, so drawing it never runs git.
    fn assistant_telemetry(&self) -> Div {
        let running = self.turns.len();
        let waiting = self.approvals.len() + self.waiting_sessions().len();
        let mut row = div().flex().flex_wrap().gap_1().child(readout(
            (running > 0).then_some(c_green()),
            running.to_string(),
            "running",
        ));
        if waiting > 0 {
            row = row.child(readout(Some(c_amber()), waiting.to_string(), "waiting"));
        }
        if let Some(session) = self.current() {
            row = row.child(readout(None, session.branch.clone(), "branch"));
        }
        let clashes = self.git_radar.as_ref().map_or(0, |hits| hits.len());
        if clashes > 0 {
            row = row.child(readout(
                Some(c_red()),
                clashes.to_string(),
                if clashes == 1 { "clash" } else { "clashes" },
            ));
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn mark_tip_counts_unread_answers() {
        assert_eq!(mark_tip(0), "Ask Runo");
        assert_eq!(mark_tip(1), "Runo answered");
        assert_eq!(mark_tip(3), "3 answers from Runo");
    }

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
    fn every_open_leads_somewhere() {
        for a in guide() {
            if let Some(open) = &a.open {
                assert!(
                    jump_for(open).is_some(),
                    "{} opens {open}, which leads nowhere",
                    a.id
                );
            }
        }
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
        assert!(search("zebra quantum lasagna").is_empty());
    }

    #[test]
    fn reply_open_line_is_taken_off() {
        let (text, open, _) = split_reply("Use a race.\n\nOPEN: race-agents");
        assert_eq!(text, "Use a race.");
        assert_eq!(open.as_deref(), Some("race-agents"));
        let (_, none, _) = split_reply("Not covered.\nOPEN: none");
        assert!(none.is_none());
    }

    #[test]
    fn the_light_runs_round_the_edge() {
        let (w, r) = (46., 14.);
        // It starts in the middle of the top edge and comes back to it after a lap.
        assert_eq!(round_square_at(w, r, 0.), (23., 0.));
        let back = round_square_at(w, r, 1.);
        assert!((back.0 - 23.).abs() < 0.01 && back.1.abs() < 0.01);
        // A quarter of the way round it is halfway down the right edge.
        let q = round_square_at(w, r, 0.25);
        assert!((q.0 - w).abs() < 0.01 && (q.1 - 23.).abs() < 0.01);
        // Small steps make small moves: no jumps where a side meets a corner.
        let mut last = round_square_at(w, r, 0.);
        for i in 1..=400 {
            let at = round_square_at(w, r, i as f32 / 400.);
            let step = ((at.0 - last.0).powi(2) + (at.1 - last.1).powi(2)).sqrt();
            assert!(step < 0.6, "jumped {step} at {i}");
            assert!((0. ..=w).contains(&at.0) && (0. ..=w).contains(&at.1));
            last = at;
        }
    }

    #[test]
    fn reply_actions_are_taken_off_and_capped() {
        let reply = "Racing them now.\n\
            ACTION: {\"do\":\"race\",\"task\":\"fix\",\"agents\":[\"claude\",\"codex\"]}\n\
            ACTION: {\"do\":\"race\",\"task\":\"fix\",\"agents\":[\"claude\",\"codex\"]}\n\
            ACTION: {\"do\":\"stop\",\"session\":1}\n\
            ACTION: {\"do\":\"stop\",\"session\":2}\n\
            ACTION: {\"do\":\"stop\",\"session\":3}\n\
            ACTION: {\"do\":\"rm -rf\"}\n\
            OPEN: race-agents";
        let (text, open, actions) = split_reply(reply);
        assert_eq!(text, "Racing them now.");
        assert_eq!(open.as_deref(), Some("race-agents"));
        // The repeat is dropped, the unknown one ignored, and no more than three kept.
        assert_eq!(actions.len(), MAX_ACTIONS);
        assert!(matches!(actions[0], Action::Race { .. }));
        assert_eq!(actions[2], Action::Stop { session: 2 });
    }

    #[test]
    fn every_place_the_agent_is_told_of_exists() {
        let help = actions_help();
        for (place, _, _) in PLACES {
            assert!(help.contains(place), "the agent isn't told of {place}");
            assert!(jump_for(place).is_some(), "{place} leads nowhere");
        }
        assert!(jump_for("settings:general").is_some());
    }

    #[test]
    fn every_place_in_runo_has_an_article() {
        for view in [
            View::Home,
            View::Design,
            View::Architecture,
            View::Data,
            View::Session,
            View::NewSession,
            View::Inbox,
            View::AllSessions,
            View::History,
            View::Worktrees,
            View::Settings,
            View::Notes,
            View::Activity,
            View::Automations,
            View::GitHub,
            View::Editor,
            View::Review,
            View::Branches,
            View::GitHistory,
        ] {
            let id = view_article(view);
            assert!(article(id).is_some(), "there's no guide article {id}");
        }
    }

    #[test]
    fn every_command_has_an_article() {
        let mut ids = Vec::new();
        for c in crate::commands::COMMANDS {
            assert!(
                article(c.guide).is_some(),
                "{} points at guide article {}, which doesn't exist",
                c.id,
                c.guide
            );
            assert!(!ids.contains(&c.id), "two commands are called {}", c.id);
            ids.push(c.id);
            assert_eq!(crate::commands::find(c.id).map(|f| f.id), Some(c.id));
            assert_eq!(crate::commands::find(c.label).map(|f| f.id), Some(c.id));
        }
    }

    #[test]
    fn questions_about_every_part_find_it() {
        let top = |q: &str| search(q).first().map(|a| a.id);
        assert_eq!(top("exit runo"), Some("quit-update"));
        assert_eq!(top("how do I quit"), Some("quit-update"));
        assert_eq!(top("Is there a new version of Runo?"), Some("quit-update"));
        assert_eq!(
            top("how do I draw an architecture diagram"),
            Some("architecture")
        );
        assert_eq!(
            top("how do I design a screen and hand it off"),
            Some("design")
        );
        assert_eq!(top("where are the games"), Some("arcade"));
        assert_eq!(top("how do I add a project folder"), Some("projects"));
        assert_eq!(top("where are my project notes"), Some("notes"));
        assert_eq!(top("show me the timeline"), Some("timeline"));
        assert_eq!(top("how do I search settings"), Some("settings"));
    }

    #[test]
    fn agent_starters_are_questions_it_can_take() {
        assert!(!search(AGENT_STARTERS[3]).is_empty());
    }

    #[test]
    fn starters_have_answers() {
        for q in STARTERS {
            assert!(!search(q).is_empty(), "nothing in the guide for {q}");
        }
    }
}
