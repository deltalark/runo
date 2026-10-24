//! Crew in the app: each Crew project's chat (a read-only session, see `crew::CANVAS`),
//! where a message goes, the actions Crew ends its replies with, the threads it starts,
//! and what threads report back. The rules and the data are in `crew.rs`; the cards and
//! the Threads panel in `crew_view.rs`.

use super::*;
use crate::crew::{self, Action, Card, Chip, Origin, Route, ThreadInfo};
use std::collections::HashSet;

/// What the app keeps for Crew while it runs; nothing here is saved.
#[derive(Default)]
pub(crate) struct CrewState {
    /// Blocks being taken out of replies as they stream in, by session.
    pub filters: HashMap<u64, crew::TagFilter>,
    /// Events already applied, so one read twice is applied once.
    pub seen: crew::Seen,
    /// The routing chip on each project's Crew composer.
    pub chips: HashMap<u64, Chip>,
    /// Crew chats whose next turn may only answer, and those whose running turn may.
    pub answer_next: HashSet<u64>,
    pub answer_now: HashSet<u64>,
    /// What goes to Crew with its next turn, by Crew chat: messages from threads and the
    /// threads it asked to read.
    pub inbox: HashMap<u64, Vec<String>>,
    /// Turns Crew took on its own since the person last wrote to it, by Crew chat.
    pub wakes: HashMap<u64, u32>,
}

/// How a thread stands, in a word, for Crew's board and the cards.
pub(crate) fn thread_status(working: bool, s: &Session) -> &'static str {
    let info = s.crew.as_ref();
    if working {
        "working"
    } else if info.is_some_and(|i| i.decision.is_some()) {
        "needs you"
    } else if info.is_some_and(|i| i.resolved) {
        "resolved"
    } else {
        match s.status {
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Stopped => "stopped",
            _ => "not started",
        }
    }
}

impl RunoApp {
    /// Agents whose command line is installed here.
    pub(crate) fn installed_agents(&self, cx: &App) -> Vec<&'static str> {
        let settings = cx.global::<AppSettings>();
        agent::PROVIDERS
            .iter()
            .filter(|p| agent::available(p.bin, &settings.agent(p.id).binary))
            .map(|p| p.id)
            .collect()
    }

    /// Project `pid` works through Crew.
    pub(crate) fn crew_mode(&self, pid: u64) -> bool {
        self.state
            .projects
            .iter()
            .any(|p| p.id == pid && crew::is_crew(p))
    }

    /// Project `pid`'s Crew chat, once it has one.
    pub(crate) fn crew_chat(&self, pid: u64) -> Option<&Session> {
        self.state
            .sessions
            .iter()
            .find(|s| s.project_id == pid && s.canvas == crew::CANVAS && !s.archived)
    }

    /// Project `pid`'s threads, newest first; resolved ones too when `all`.
    pub(crate) fn crew_threads(&self, pid: u64, all: bool) -> Vec<&Session> {
        let mut out: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| {
                s.project_id == pid
                    && !s.archived
                    && s.crew.as_ref().is_some_and(|i| all || !i.resolved)
            })
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.id));
        out
    }

    /// Switch a project between Crew and Direct. Nothing else changes: threads carry on
    /// as ordinary sessions, and Crew's chat waits for the switch back.
    pub(crate) fn set_project_mode(&mut self, pid: u64, on: bool) {
        if let Some(p) = self.state.projects.iter_mut().find(|p| p.id == pid) {
            p.mode = if on {
                crew::MODE_CREW.into()
            } else {
                String::new()
            };
        }
        if !on {
            // Its tab closes; the chat itself stays.
            if let Some(id) = self.crew_chat(pid).map(|s| s.id) {
                self.tabs.retain(|t| *t != id);
                if self.selected == Some(id) {
                    self.selected = self.tabs.last().copied();
                }
            }
        }
        self.save();
    }

    /// The agent Crew runs on, and threads use when Crew names none: the default agent
    /// when it is installed, otherwise the first one that is.
    fn crew_agent(&self, cx: &App) -> usize {
        let installed = self.installed_agents(cx);
        let default = cx.global::<AppSettings>().default_provider.clone();
        let id = installed
            .iter()
            .find(|a| **a == default)
            .or(installed.first())
            .copied()
            .unwrap_or("claude");
        agent::PROVIDERS
            .iter()
            .position(|p| p.id == id)
            .unwrap_or(0)
    }

    /// Open project `pid`'s Crew chat as a tab, making it the first time: a session in the
    /// project folder that can read but never change anything.
    pub(crate) fn open_crew(&mut self, pid: u64, cx: &mut Context<Self>) -> Option<u64> {
        if let Some(id) = self.crew_chat(pid).map(|s| s.id) {
            self.select(id);
            return Some(id);
        }
        let project = self.state.projects.iter().find(|p| p.id == pid).cloned()?;
        let provider = self.crew_agent(cx);
        let id = match self
            .spawn_session_from(&project, provider, "", false, false, "Crew", None, None, cx)
        {
            Ok(id) => id,
            Err(error) => {
                self.fail("crew", error);
                return None;
            }
        };
        let name = agent::PROVIDERS[provider].name;
        if let Some(s) = self.state.session_mut(id) {
            s.canvas = crew::CANVAS.into();
            s.title = "Crew".into();
            s.limits = vec!["read".into()];
            s.branch = git::current_branch(&project.path);
            // A note, not the red card that system messages get for errors.
            let mut welcome = Message::new(
                Role::System,
                format!(
                    "Crew runs {} for you on {name}. Ask it anything about the project, or describe work: it answers what it can and starts threads for the rest. It reads the code but never changes it.",
                    project.name
                ),
            );
            welcome.meta = "note".into();
            s.messages.push(welcome);
        }
        crate::applog::info(
            "crew",
            format!("project {:?}: Crew chat #{id} made", project.name),
        );
        self.select(id);
        Some(id)
    }

    /// Context that goes ahead of a turn's prompt, unseen in the chat: Crew's rules and
    /// what changed in its threads, or a thread's rules on its first turn.
    pub(crate) fn crew_preface(&mut self, id: u64) -> Option<String> {
        let s = self.state.session(id)?;
        if s.canvas == crew::CANVAS {
            return self.chat_preface(id);
        }
        if s.crew.is_none() || s.has_conversation() {
            return None;
        }
        let tools = settings::memory_server() && s.connection == connections::LOCAL;
        Some(crew::thread_rules(tools))
    }

    fn chat_preface(&mut self, id: u64) -> Option<String> {
        let s = self.state.session(id)?;
        let pid = s.project_id;
        let first = !s.has_conversation();
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == pid)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let board: Vec<String> = self
            .crew_threads(pid, false)
            .iter()
            .rev()
            .map(|t| {
                let working = self.handles.contains_key(&Target::Session(t.id));
                crew::board_line(
                    t.id,
                    &t.title,
                    thread_status(working, t),
                    t.crew.as_ref().unwrap_or(&ThreadInfo::default()),
                )
            })
            .collect();
        let before = if first {
            String::new()
        } else {
            s.canvas_context.clone()
        };
        let changes = crew::board_changes(&before, &board);
        let mut parts = Vec::new();
        if first {
            let roster: Vec<(&str, &str)> =
                agent::PROVIDERS.iter().map(|p| (p.id, p.name)).collect();
            let default = agent::provider(&s.provider).name;
            let models: Vec<String> = crate::caps::get(&s.provider)
                .models
                .iter()
                .take(8)
                .map(|m| m.choice.value.clone())
                .filter(|v| !v.is_empty())
                .collect();
            parts.push(crew::rules(&project, &roster, default, &models));
        }
        if !changes.is_empty() {
            parts.push(format!("## Threads\n{}", changes.trim_end()));
        }
        let (running, waiting) = self.thread_slots(pid);
        let cap = settings::crew_running();
        if cap > 0 && (waiting > 0 || running >= cap) {
            parts.push(format!(
                "## Slots\nThreads run {cap} at a time: {running} running, {waiting} waiting. Waiting threads start on their own as others finish."
            ));
        }
        if let Some(inbox) = self.crew.inbox.remove(&id).filter(|i| !i.is_empty()) {
            parts.push(format!("## From threads\n{}", inbox.join("\n\n")));
        }
        let answer = self.crew.answer_next.remove(&id);
        if answer {
            self.crew.answer_now.insert(id);
            parts.push(
                "## This message\nJust answer. Don't start or message threads this time.".into(),
            );
        } else {
            self.crew.answer_now.remove(&id);
        }
        if let Some(s) = self.state.session_mut(id) {
            s.canvas_context = board.join("\n");
        }
        (!parts.is_empty()).then(|| format!("{}\n\n## The person says", parts.join("\n\n")))
    }

    /// A message typed in a Crew chat: an @thread goes straight to that thread and the
    /// New thread chip starts one, neither asking Crew. True when it was handled here;
    /// otherwise it goes to Crew as an ordinary turn.
    pub(crate) fn crew_route_submit(
        &mut self,
        id: u64,
        text: &str,
        images: &[String],
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(s) = self.state.session(id).filter(|s| s.canvas == crew::CANVAS) else {
            return false;
        };
        let pid = s.project_id;
        let chip = self.crew.chips.get(&pid).copied().unwrap_or_default();
        let threads: Vec<(u64, String)> = self
            .crew_threads(pid, true)
            .iter()
            .map(|t| (t.id, t.title.clone()))
            .collect();
        // The person wrote: Crew may take turns on its own again.
        self.crew.wakes.remove(&id);
        match crew::route(text, chip, &threads) {
            Route::Crew { answer_only } => {
                if answer_only {
                    self.crew.answer_next.insert(id);
                }
                false
            }
            Route::Thread {
                id: thread,
                text: body,
            } => {
                let title = self
                    .state
                    .session(thread)
                    .map(|t| t.title.clone())
                    .unwrap_or_default();
                self.crew_note(id, Role::User, text.to_string(), None);
                self.message_thread(thread, body, images.to_vec(), cx);
                self.crew_note(
                    id,
                    Role::System,
                    format!("Sent to {title}, without Crew."),
                    Some(Card::News { thread }),
                );
                true
            }
            Route::NewThread => {
                self.crew_note(id, Role::User, text.to_string(), None);
                let title = crew::title_from(text);
                match self.start_thread(
                    pid,
                    &title,
                    text,
                    "",
                    None,
                    crew::ThreadPick::default(),
                    Origin::Person,
                    cx,
                ) {
                    Ok(thread) => self.crew_note(
                        id,
                        Role::System,
                        "Started a thread, without Crew.".into(),
                        Some(Card::Started {
                            threads: vec![thread],
                        }),
                    ),
                    Err(error) => self.crew_note(
                        id,
                        Role::System,
                        format!("Couldn't start the thread: {error}"),
                        None,
                    ),
                }
                self.crew.chips.insert(pid, Chip::Auto);
                true
            }
        }
    }

    /// Add a line to a Crew chat without a turn: what the person sent elsewhere, or news.
    pub(crate) fn crew_note(&mut self, chat: u64, role: Role, text: String, card: Option<Card>) {
        if let Some(s) = self.state.session_mut(chat) {
            let mut m = Message::now(role, text);
            m.crew = card;
            s.messages.push(m);
        }
        self.save();
    }

    /// Start a thread in project `pid` that works on `brief`, in the background.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_thread(
        &mut self,
        pid: u64,
        title: &str,
        brief: &str,
        agent_id: &str,
        isolated: Option<bool>,
        pick: crew::ThreadPick,
        origin: Origin,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == pid)
            .cloned()
            .ok_or("The project is gone.")?;
        let installed = self.installed_agents(cx);
        let provider = agent::PROVIDERS
            .iter()
            .position(|p| !agent_id.is_empty() && p.id == agent_id && installed.contains(&p.id))
            .unwrap_or_else(|| self.crew_agent(cx));
        let is_git = git::is_repo(&project.path);
        // Threads run side by side, so each gets its own worktree unless Crew says not to.
        let isolated = isolated.unwrap_or(true) && is_git;
        let model = thread_model(agent::PROVIDERS[provider].id, &pick.model);
        let id = self.spawn_session_from(
            &project, provider, "", isolated, is_git, title, None, None, cx,
        )?;
        let connection = self.project_connection(pid);
        if let Some(s) = self.state.session_mut(id) {
            s.title = title.to_string();
            s.connection = connection;
            if let Some(model) = model {
                s.set_model(model);
            }
            if !pick.effort.is_empty() {
                s.opts.insert("effort".into(), pick.effort.clone());
            }
            s.crew = Some(ThreadInfo {
                origin,
                brief: brief.to_string(),
                budget: (pick.budget_usd > 0.0).then_some(crew::Budget {
                    dollars: pick.budget_usd,
                    turns: 0,
                }),
                ..Default::default()
            });
        }
        if !self.expanded.contains(&pid) {
            self.expanded.push(pid);
        }
        crate::applog::info(
            "crew",
            format!("project {:?}: thread #{id} started: {title}", project.name),
        );
        self.launch(id, brief.to_string(), cx);
        self.save();
        Ok(id)
    }

    /// Send a thread `text` as its next prompt; it waits in the queue while the thread works.
    pub(crate) fn message_thread(
        &mut self,
        thread: u64,
        text: String,
        images: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.state.session(thread).map(|s| s.provider.clone()) else {
            return;
        };
        if let Some(info) = self.state.session_mut(thread).and_then(|s| s.crew.as_mut()) {
            info.resolved = false;
        }
        let opts = cx.global::<AppSettings>().agent(&provider);
        self.start_turn(thread, text, images, &opts);
    }

    /// The person picked an answer to a thread's decision: it goes to the thread as its
    /// next prompt, and the decision closes.
    pub(crate) fn answer_decision(&mut self, thread: u64, choice: usize, cx: &mut Context<Self>) {
        let Some(decision) = self
            .state
            .session_mut(thread)
            .and_then(|s| s.crew.as_mut())
            .and_then(|i| {
                i.attention = None;
                i.decision.take()
            })
        else {
            return;
        };
        let Some(option) = decision.options.get(choice) else {
            return;
        };
        let text = format!("On \"{}\" I choose: {}.", decision.question, option.label);
        self.message_thread(thread, text, Vec::new(), cx);
        self.save();
    }

    /// Text of a reply as it streams in, with Runo's blocks taken out, for Crew's chat and
    /// its threads: `Some(None)` when it is all held back, `None` for other sessions.
    /// `plain` lines come without their newline.
    pub(crate) fn crew_text(&mut self, id: u64, text: &str, plain: bool) -> Option<Option<String>> {
        if !self.crew.filters.contains_key(&id) {
            let s = self.state.session(id)?;
            if s.canvas != crew::CANVAS && s.crew.is_none() {
                return None;
            }
            self.crew.filters.insert(id, crew::TagFilter::default());
        }
        let filter = self.crew.filters.get_mut(&id)?;
        let shown = if plain {
            filter.push(&format!("{text}\n"))
        } else {
            filter.push(text)
        };
        if shown.is_empty() {
            return Some(None);
        }
        Some(Some(if plain {
            shown.strip_suffix('\n').unwrap_or(&shown).to_string()
        } else {
            shown
        }))
    }

    /// A turn of a Crew chat or a thread ended: the blocks in its reply take effect.
    pub(crate) fn crew_turn_ended(
        &mut self,
        id: u64,
        ok: bool,
        stopped: bool,
        cx: &mut Context<Self>,
    ) {
        let blocks =
            match self.crew.filters.remove(&id) {
                Some(filter) => {
                    let (rest, blocks) = filter.finish();
                    if !rest.trim().is_empty() {
                        if let Some(m) = self.state.session_mut(id).and_then(|s| {
                            s.messages.iter_mut().rev().find(|m| m.role == Role::Agent)
                        }) {
                            m.text.push_str(&rest);
                        }
                    }
                    blocks
                }
                None => Vec::new(),
            };
        let Some(s) = self.state.session(id) else {
            return;
        };
        if s.canvas == crew::CANVAS {
            let answer_only = self.crew.answer_now.remove(&id);
            self.chat_turn_ended(id, blocks, ok && !stopped && !answer_only, answer_only, cx);
        } else if s.crew.is_some() {
            let reports = s.crew.as_ref().map_or(0, |i| i.reports.len());
            // What the thread sent through Runo's tools during the turn lands first, so
            // its last checklist and report count, not one read a tick later.
            self.crew_tick(cx);
            self.thread_turn_ended(id, blocks, ok, stopped, reports);
        }
    }

    fn chat_turn_ended(
        &mut self,
        chat: u64,
        blocks: Vec<crew::Tagged>,
        act: bool,
        answer_only: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(pid) = self.state.session(chat).map(|s| s.project_id) else {
            return;
        };
        let mut actions = Vec::new();
        let mut problems = Vec::new();
        for block in blocks.iter().filter(|b| b.kind == "actions") {
            let (found, wrong) = crew::parse_actions(&block.body);
            actions.extend(found);
            problems.extend(wrong);
        }
        if actions.is_empty() && problems.is_empty() {
            return;
        }
        if !act {
            if answer_only {
                self.crew_note(chat, Role::System, "Crew suggested actions, but this message was set to Just answer, so nothing ran.".into(), None);
            }
            return;
        }
        let threads: Vec<u64> = self.crew_threads(pid, true).iter().map(|t| t.id).collect();
        let (run, held, more) = crew::within_limits(actions, &threads);
        problems.extend(more);
        let mut started = Vec::new();
        let mut notes = Vec::new();
        let message = self
            .state
            .session(chat)
            .and_then(|s| s.messages.iter().rposition(|m| m.role == Role::User))
            .unwrap_or(0) as u64;
        for action in run {
            match action {
                Action::StartThread {
                    ref title,
                    ref brief,
                    ref agent,
                    isolated,
                    ..
                } => {
                    match self.start_thread(
                        pid,
                        title,
                        brief,
                        agent,
                        isolated,
                        action.pick(),
                        Origin::Crew { message },
                        cx,
                    ) {
                        Ok(id) => started.push(id),
                        Err(error) => problems.push(format!("Couldn't start \"{title}\": {error}")),
                    }
                }
                Action::MessageThread { thread, text } => {
                    let title = self
                        .state
                        .session(thread)
                        .map(|t| t.title.clone())
                        .unwrap_or_default();
                    self.message_thread(thread, text, Vec::new(), cx);
                    notes.push((format!("Sent a message to {title}."), thread));
                }
                Action::ReadThread { thread } => {
                    let reply = self.state.session(thread).map(|t| {
                        let last: Vec<String> = t
                            .messages
                            .iter()
                            .filter(|m| m.role == Role::Agent && !m.text.trim().is_empty())
                            .rev()
                            .take(2)
                            .map(|m| m.text.chars().take(crew::MAX_TEXT).collect())
                            .collect();
                        format!(
                            "Thread #{thread} ({}), latest replies, newest first:\n{}",
                            t.title,
                            last.join("\n---\n")
                        )
                    });
                    if let Some(reply) = reply {
                        self.crew.inbox.entry(chat).or_default().push(reply);
                    }
                }
                Action::ResolveThread { thread } => {
                    if let Some(info) = self.state.session_mut(thread).and_then(|s| s.crew.as_mut())
                    {
                        info.resolved = true;
                        info.attention = None;
                    }
                    self.queue_build_clean(thread);
                }
            }
        }
        if !started.is_empty() {
            let n = started.len();
            self.crew_note(
                chat,
                Role::System,
                format!("Started {n} thread{}.", if n == 1 { "" } else { "s" }),
                Some(Card::Started { threads: started }),
            );
        }
        for (text, thread) in notes {
            self.crew_note(chat, Role::System, text, Some(Card::News { thread }));
        }
        if !held.is_empty() {
            let n = held.len();
            self.crew_note(
                chat,
                Role::System,
                format!(
                    "Crew wants {n} more thread{} than it may start at once.",
                    if n == 1 { "" } else { "s" }
                ),
                Some(Card::Held {
                    actions: held,
                    answered: false,
                }),
            );
        }
        if !problems.is_empty() {
            let mut m = Message::now(Role::System, problems.join("\n"));
            m.meta = "warning".into();
            if let Some(s) = self.state.session_mut(chat) {
                s.messages.push(m);
            }
            // Crew hears what went wrong with its next turn.
            self.crew.inbox.entry(chat).or_default().push(format!(
                "Runo couldn't do everything you asked last turn:\n{}",
                problems.join("\n")
            ));
        }
        self.save();
    }

    /// Start the threads Crew asked for over its limit, from the card in message `index`.
    pub(crate) fn start_held(&mut self, chat: u64, index: usize, cx: &mut Context<Self>) {
        let Some((pid, actions)) = self.state.session_mut(chat).and_then(|s| {
            let pid = s.project_id;
            match s.messages.get_mut(index).and_then(|m| m.crew.as_mut()) {
                Some(Card::Held { actions, answered }) if !*answered => {
                    *answered = true;
                    Some((pid, actions.clone()))
                }
                _ => None,
            }
        }) else {
            return;
        };
        let mut started = Vec::new();
        for action in actions {
            let pick = action.pick();
            if let Action::StartThread {
                title,
                brief,
                agent,
                isolated,
                ..
            } = action
            {
                if let Ok(id) = self.start_thread(
                    pid,
                    &title,
                    &brief,
                    &agent,
                    isolated,
                    pick,
                    Origin::Person,
                    cx,
                ) {
                    started.push(id);
                }
            }
        }
        if !started.is_empty() {
            self.crew_note(
                chat,
                Role::System,
                "Started the rest.".into(),
                Some(Card::Started { threads: started }),
            );
        }
        self.save();
    }

    /// Leave the held threads unstarted.
    pub(crate) fn drop_held(&mut self, chat: u64, index: usize) {
        if let Some(Card::Held { answered, .. }) = self
            .state
            .session_mut(chat)
            .and_then(|s| s.messages.get_mut(index))
            .and_then(|m| m.crew.as_mut())
        {
            *answered = true;
        }
        self.save();
    }

    /// `reports` is how many reports the thread had before its turn's last ones were read.
    fn thread_turn_ended(
        &mut self,
        id: u64,
        blocks: Vec<crew::Tagged>,
        ok: bool,
        stopped: bool,
        reports: usize,
    ) {
        let at = crate::workspace::unix_now();
        let mut reported = None;
        if let Some(s) = self.state.session_mut(id).filter(|s| s.crew.is_some()) {
            let info = s.crew.as_mut().expect("a thread");
            let mut name = None;
            for block in &blocks {
                if let Some(kind) = crew::block_event(block) {
                    name = crew::self_title(info, &kind).or(name);
                    crew::apply(info, &kind, at);
                }
            }
            if let Some(name) = name {
                s.title = name;
            }
            let info = s.crew.as_mut().expect("a thread");
            if info.decision.is_none() {
                info.attention = ok.then_some(crew::Attention::Review);
            }
            if ok && !stopped {
                crew::settle(info);
            }
            reported = info
                .reports
                .last()
                .filter(|r| info.reports.len() > reports || r.at >= at.saturating_sub(1))
                .map(|r| r.summary.clone());
        }
        let Some(s) = self.state.session(id) else {
            return;
        };
        let title = s.title.clone();
        let pid = s.project_id;
        let decision = s
            .crew
            .as_ref()
            .and_then(|i| i.decision.as_ref())
            .map(|d| d.question.clone());
        let text = match (decision, ok, stopped) {
            (Some(q), _, _) => format!("{title} needs you: {q}"),
            (None, true, _) => match reported {
                Some(r) => format!("{title} finished. {r}"),
                None => format!("{title} finished."),
            },
            (None, false, true) => format!("{title} was stopped."),
            (None, false, false) => format!("{title} failed."),
        };
        if let Some(chat) = self.crew_chat(pid).map(|c| c.id) {
            self.crew_note(chat, Role::System, text, Some(Card::News { thread: id }));
        }
    }

    /// Apply what threads reported through Runo's tools since the last look.
    pub(crate) fn crew_tick(&mut self, cx: &mut Context<Self>) {
        if !self.state.sessions.iter().any(|s| s.crew.is_some()) {
            return;
        }
        let events = crew::take_events(&crew::events_dir(), &mut self.crew.seen);
        if events.is_empty() {
            return;
        }
        for event in events {
            let Some(s) = self.state.session_mut(event.thread) else {
                continue;
            };
            let Some(info) = s.crew.as_mut() else {
                continue;
            };
            let name = crew::self_title(info, &event.kind);
            let changed = crew::apply(info, &event.kind, event.at);
            if let Some(name) = name {
                s.title = name;
            }
            let (pid, title) = (s.project_id, s.title.clone());
            let chat = self.crew_chat(pid).map(|c| c.id);
            match (&event.kind, chat) {
                (crew::EventKind::Message { text }, Some(chat)) => {
                    let text = format!("Thread #{} ({title}) says: {text}", event.thread);
                    self.wake_crew(chat, &title, text, cx);
                }
                (crew::EventKind::Decision(d), Some(chat)) if changed => {
                    self.crew_note(
                        chat,
                        Role::System,
                        format!("{title} needs you: {}", d.question),
                        Some(Card::News {
                            thread: event.thread,
                        }),
                    );
                }
                (crew::EventKind::Report { summary, .. }, Some(chat)) if changed => {
                    let working = self.handles.contains_key(&Target::Session(event.thread));
                    // A report at the end of a turn is shown with the turn's end.
                    if working {
                        self.crew_note(
                            chat,
                            Role::System,
                            format!("{title}: {summary}"),
                            Some(Card::News {
                                thread: event.thread,
                            }),
                        );
                    }
                }
                _ => {}
            }
        }
        self.save();
        cx.notify();
    }

    /// Give Crew a turn of its own to answer a thread, up to `crew::MAX_WAKES` between two
    /// messages from the person; past that the message waits for the person's next one.
    fn wake_crew(&mut self, chat: u64, from: &str, text: String, cx: &mut Context<Self>) {
        let wakes = self.crew.wakes.entry(chat).or_default();
        if *wakes >= crew::MAX_WAKES {
            self.crew.inbox.entry(chat).or_default().push(text);
            return;
        }
        *wakes += 1;
        let Some(provider) = self.state.session(chat).map(|s| s.provider.clone()) else {
            return;
        };
        let opts = cx.global::<AppSettings>().agent(&provider);
        let before = self.state.session(chat).map_or(0, |s| s.messages.len());
        self.start_turn(chat, text, Vec::new(), &opts);
        if self.state.session(chat).map_or(0, |s| s.messages.len()) > before {
            self.tag_playbook(chat, &format!("From {from}"), "");
        }
    }
}

/// The model a thread Crew started runs on: the one Crew picked, as its agent's CLI
/// names it, when the CLI lists it. `None` keeps the agent's default.
fn thread_model(provider: &str, picked: &str) -> Option<String> {
    if picked.is_empty() {
        return None;
    }
    let caps = crate::caps::get(provider);
    let known: Vec<String> = crate::caps::all_models(&caps)
        .map(|m| m.choice.value.clone())
        .filter(|v| !v.is_empty())
        .collect();
    if known.is_empty() {
        return Some(picked.to_string());
    }
    let lower = picked.to_lowercase();
    let found = known
        .iter()
        .find(|v| v.to_lowercase() == lower)
        .or_else(|| known.iter().find(|v| v.to_lowercase().contains(&lower)))
        .cloned();
    if found.is_none() {
        crate::applog::info(
            "crew",
            format!("thread model {picked:?} isn't one {provider} lists; using its default"),
        );
    }
    found
}
