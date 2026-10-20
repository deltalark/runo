//! Missions in the app: planning with an agent, starting each part as its own session
//! when the parts before it are in, proving and merging each one into the mission
//! branch, and landing the mission. The page is in `mission_view.rs`.

use super::*;
use crate::mission::{self, Mark, Mission, PartState, Stage};
use crate::proof;

/// The New mission form's choices; the goal is in its own text box.
#[derive(Default)]
pub(crate) struct MissionForm {
    /// Agents parts may use; the first one plans.
    pub agents: Vec<String>,
    pub parallel: u32,
    pub review: bool,
    pub project: u64,
    /// Filled with defaults once.
    pub ready: bool,
    pub error: Option<String>,
}

/// Index of the session's last message from the person.
fn last_ask(session: &Session) -> Option<usize> {
    session.messages.iter().rposition(|m| m.role == Role::User)
}

impl RunoApp {
    /// Add a line to a mission's activity log.
    pub(crate) fn mission_log(&mut self, mid: u64, mark: Mark, text: impl Into<String>) {
        if let Some(m) = self.mission_mut(mid) {
            m.record(crate::workspace::unix_now(), mark, text);
        }
    }

    pub(crate) fn mission(&self, mid: u64) -> Option<&Mission> {
        self.state.missions.iter().find(|m| m.id == mid)
    }

    pub(crate) fn mission_mut(&mut self, mid: u64) -> Option<&mut Mission> {
        self.state.missions.iter_mut().find(|m| m.id == mid)
    }

    pub(crate) fn installed_agents(&self, cx: &App) -> Vec<&'static str> {
        let settings = cx.global::<AppSettings>();
        agent::PROVIDERS
            .iter()
            .filter(|p| agent::available(p.bin, &settings.agent(p.id).binary))
            .map(|p| p.id)
            .collect()
    }

    pub(crate) fn open_missions(&mut self, cx: &App) {
        self.view = View::Missions;
        self.mission_form_defaults(cx);
        if self
            .mission_open
            .is_some_and(|id| self.mission(id).is_none())
        {
            self.mission_open = None;
        }
    }

    /// The mission's own session is ready to be shipped from Review.
    pub(crate) fn mission_ready(&self, session: u64) -> bool {
        self.state
            .missions
            .iter()
            .any(|m| m.lead == session && session != 0 && m.stage == Stage::Ready)
    }

    /// The rail's dot: amber when a mission needs you, blue while one runs.
    pub(crate) fn missions_badge(&self) -> Option<u32> {
        let missions = &self.state.missions;
        if missions.iter().any(|m| {
            m.stage == Stage::Plan
                || m.stage == Stage::Failed
                || m.parts.iter().any(|p| p.state == PartState::Stuck)
        }) {
            Some(c_amber())
        } else if missions
            .iter()
            .any(|m| matches!(m.stage, Stage::Planning | Stage::Running | Stage::Landing))
        {
            Some(c_blue())
        } else {
            None
        }
    }

    /// Fill the form's choices the first time it shows: the agents that are installed,
    /// two at a time, and a second opinion when there is a second vendor.
    pub(crate) fn mission_form_defaults(&mut self, cx: &App) {
        if self.mission_form.ready {
            return;
        }
        let installed = self.installed_agents(cx);
        self.mission_form.agents = installed.iter().take(3).map(|a| a.to_string()).collect();
        self.mission_form.parallel = 2;
        self.mission_form.review = installed.len() > 1;
        self.mission_form.ready = true;
    }

    // ---------- planning ----------

    /// Create a mission from the form and ask the planner for a plan.
    pub(crate) fn plan_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let goal = self.mission_goal.read(cx).value().trim().to_string();
        let form = &mut self.mission_form;
        form.error = None;
        if goal.is_empty() {
            form.error = Some("Describe the goal first.".into());
            return;
        }
        if form.agents.is_empty() {
            form.error = Some("Pick at least one agent.".into());
            return;
        }
        let pid = Some(form.project)
            .filter(|p| *p != 0)
            .or_else(|| self.current_project())
            .or_else(|| self.state.projects.first().map(|p| p.id));
        let Some(project) = pid
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            self.mission_form.error = Some("Add a project to run the mission in.".into());
            return;
        };
        if !git::is_repo(&project.path) || !git::has_commits(&project.path) {
            self.mission_form.error = Some(format!(
                "{} needs to be a git repository with at least one commit: each part gets its own worktree.",
                project.name
            ));
            return;
        }
        let taken: Vec<String> = self
            .state
            .missions
            .iter()
            .map(|m| m.branch.clone())
            .collect();
        let path = project.path.clone();
        let branch = git::unique_branch(&mission::branch_for(&goal), |b| {
            taken.iter().any(|t| t == b)
                || git::has_branch(&path, b)
                || git::worktree_path(&path, b).exists()
        });
        let id = self.state.next_id();
        self.state.missions.push(Mission {
            id,
            project_id: project.id,
            goal,
            stage: Stage::Planning,
            agents: self.mission_form.agents.clone(),
            parallel: self.mission_form.parallel.max(1),
            review: self.mission_form.review,
            branch,
            base: git::default_branch(&project.path),
            created: crate::workspace::unix_now(),
            ..Default::default()
        });
        self.mission_open = Some(id);
        self.mission_goal
            .update(cx, |input, cx| input.set_value("", window, cx));
        crate::applog::info("mission", format!("mission #{id}: planning"));
        self.run_planner(id, String::new(), cx);
        self.save();
    }

    /// Ask the mission's first agent for a plan, read-only, in the project folder.
    pub(crate) fn run_planner(&mut self, mid: u64, feedback: String, cx: &mut Context<Self>) {
        let Some(m) = self.mission(mid) else {
            return;
        };
        let Some(dir) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == m.project_id)
            .map(|p| p.path.clone())
        else {
            return;
        };
        let planner = agent::provider(m.agents.first().map(String::as_str).unwrap_or("claude"));
        let roster: Vec<(&str, &str)> = m
            .agents
            .iter()
            .map(|a| {
                let p = agent::provider(a);
                (p.id, p.name)
            })
            .collect();
        let records = mission::records(&self.state.missions);
        let prompt =
            mission::plan_prompt(&m.goal, &roster, &records, m.parallel, &feedback, &m.parts);
        let binary = cx.global::<AppSettings>().agent(planner.id).binary;
        let command = agent::ask_command(planner.id, &binary, &prompt);
        agent::oneshot_within(
            format!("mission-plan:{mid}"),
            command,
            &dir,
            mission::PLAN_LIMIT,
            self.tx.clone(),
        );
        if let Some(m) = self.mission_mut(mid) {
            m.stage = Stage::Planning;
            m.planned = crate::workspace::unix_now();
            m.note.clear();
        }
        cx.notify();
    }

    /// Plan again with what the person typed under the plan.
    pub(crate) fn replan(&mut self, mid: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .mission(mid)
            .is_some_and(|m| matches!(m.stage, Stage::Plan | Stage::Failed))
        {
            return;
        }
        let feedback = self.mission_feedback.read(cx).value().trim().to_string();
        self.mission_feedback
            .update(cx, |i, cx| i.set_value("", window, cx));
        self.run_planner(mid, feedback, cx);
        self.save();
        cx.notify();
    }

    /// The planner answered. True when `key` was a plan.
    pub(crate) fn handle_mission_output(
        &mut self,
        key: &str,
        result: &Result<String, String>,
    ) -> bool {
        let Some(mid) = key
            .strip_prefix("mission-plan:")
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return false;
        };
        let Some(m) = self.mission_mut(mid) else {
            return true;
        };
        if m.stage != Stage::Planning {
            return true;
        }
        let planned = match result {
            Ok(reply) => mission::parse_plan(reply, &m.agents),
            Err(error) => Err(format!(
                "The planner stopped: {}",
                error.lines().next().unwrap_or("")
            )),
        };
        match planned {
            Ok((summary, parts)) => {
                m.summary = summary;
                m.parts = parts;
                m.stage = Stage::Plan;
                m.note.clear();
            }
            Err(error) => {
                crate::applog::warn("mission", format!("mission #{mid}: {error}"));
                m.stage = Stage::Failed;
                m.note = error;
            }
        }
        self.save();
        true
    }

    // ---------- running ----------

    /// Start the planned mission: its own session on the mission branch takes the
    /// merges, and the first parts start once its worktree is ready.
    pub(crate) fn start_mission(&mut self, mid: u64, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some(m) = self.mission(mid).cloned() else {
            return;
        };
        if m.parts.is_empty() {
            return;
        }
        let Some(project) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == m.project_id)
            .cloned()
        else {
            return;
        };
        let lead_agent = m.agents.first().cloned().unwrap_or_else(|| "claude".into());
        let provider = agent::PROVIDERS
            .iter()
            .position(|p| p.id == lead_agent)
            .unwrap_or(0);
        let title = format!("Mission: {}", m.goal.lines().next().unwrap_or(""));
        match self.spawn_session_from(
            &project,
            provider,
            &m.branch,
            true,
            true,
            &title,
            Some(m.base.clone()),
            None,
            cx,
        ) {
            Ok(lead) => {
                let plan = m
                    .parts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        format!(
                            "{}. {} ({})",
                            i + 1,
                            p.title,
                            agent::provider(&p.agent).name
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(s) = self.state.session_mut(lead) {
                    s.mission = mid;
                    s.messages.push(Message::new(
                        Role::System,
                        format!(
                            "This is the mission's own session. Each part is merged into {} when its checks pass; then the checks run here once more and it is ready to ship.\n\n{plan}",
                            m.branch
                        ),
                    ));
                }
                if let Some(m) = self.mission_mut(mid) {
                    m.lead = lead;
                    m.stage = Stage::Running;
                    m.started = crate::workspace::unix_now();
                    m.note.clear();
                    let n = m.parts.len();
                    m.record(
                        m.started,
                        Mark::Start,
                        format!(
                            "Mission started with {n} part{}",
                            if n == 1 { "" } else { "s" }
                        ),
                    );
                }
                if !self.expanded.contains(&project.id) {
                    self.expanded.push(project.id);
                }
                self.state.log(lead, format!("Mission started: {}", m.goal));
                self.mission_tick(mid, cx);
            }
            Err(error) => {
                if let Some(m) = self.mission_mut(mid) {
                    m.note = format!("Could not start: {error}");
                }
            }
        }
        self.save();
        cx.notify();
    }

    /// Move every mission along; the first call after launch also picks up missions
    /// that were running at the last quit.
    pub(crate) fn missions_tick_all(&mut self, cx: &mut Context<Self>) {
        if self.demo {
            return;
        }
        if !self.missions_resumed {
            self.missions_resumed = true;
            let mut look_again = Vec::new();
            for m in &mut self.state.missions {
                if m.stage == Stage::Planning {
                    m.stage = Stage::Failed;
                    m.note = "Runo was closed while planning. Plan again.".into();
                }
                for p in &mut m.parts {
                    match p.state {
                        PartState::Checking | PartState::Reviewing | PartState::Working => {
                            p.state = PartState::Working;
                            look_again.push(p.session);
                        }
                        PartState::Merging => p.state = PartState::Proved,
                        _ => {}
                    }
                }
                if m.stage == Stage::Landing {
                    look_again.push(m.lead);
                }
            }
            for id in look_again.into_iter().filter(|id| *id != 0) {
                self.mission_check(id, cx);
            }
        }
        let live: Vec<u64> = self
            .state
            .missions
            .iter()
            .filter(|m| m.stage == Stage::Running)
            .map(|m| m.id)
            .collect();
        for mid in live {
            self.mission_tick(mid, cx);
        }
    }

    /// Merge the next proved part, start the parts that may start, and land the mission
    /// when every part is in.
    pub(crate) fn mission_tick(&mut self, mid: u64, cx: &mut Context<Self>) {
        let Some(m) = self.mission(mid) else {
            return;
        };
        if m.stage != Stage::Running || self.preparing.contains_key(&m.lead) {
            return;
        }
        if let Some(i) = m.next_merge() {
            self.merge_part(mid, i);
        }
        let Some(m) = self.mission(mid) else {
            return;
        };
        for i in m.startable() {
            self.start_part(mid, i, cx);
        }
        let Some(m) = self.mission(mid) else {
            return;
        };
        if m.all_in() {
            self.land(mid, cx);
        }
    }

    /// Start one part as its own session, branched from the mission branch.
    fn start_part(&mut self, mid: u64, index: usize, cx: &mut Context<Self>) {
        let Some(m) = self.mission(mid).cloned() else {
            return;
        };
        let Some(part) = m.parts.get(index).cloned() else {
            return;
        };
        let Some(project) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == m.project_id)
            .cloned()
        else {
            return;
        };
        let installed = {
            let settings = cx.global::<AppSettings>();
            let p = agent::provider(&part.agent);
            agent::available(p.bin, &settings.agent(p.id).binary)
        };
        if !installed {
            self.stuck(
                mid,
                index,
                format!(
                    "{} isn't installed here. Pick another agent with Try again, or Skip.",
                    agent::provider(&part.agent).name
                ),
            );
            return;
        }
        let provider = agent::PROVIDERS
            .iter()
            .position(|p| p.id == part.agent)
            .unwrap_or(0);
        let path = project.path.clone();
        let branch = git::unique_branch(&format!("{}-{}", m.branch, part.key), |b| {
            git::has_branch(&path, b) || git::worktree_path(&path, b).exists()
        });
        match self.spawn_session_from(
            &project,
            provider,
            &branch,
            true,
            true,
            &part.title,
            Some(m.branch.clone()),
            None,
            cx,
        ) {
            Ok(id) => {
                let names = |a: &str| agent::provider(a).name.to_string();
                let prompt = mission::part_prompt(&m, index, names);
                if let Some(s) = self.state.session_mut(id) {
                    s.mission = mid;
                }
                if let Some(p) = self.mission_mut(mid).and_then(|m| m.parts.get_mut(index)) {
                    p.session = id;
                    p.state = PartState::Working;
                    p.started = crate::workspace::unix_now();
                    p.note.clear();
                }
                self.mission_log(
                    mid,
                    Mark::Part,
                    format!(
                        "{} started {}",
                        agent::provider(&part.agent).name,
                        part.title
                    ),
                );
                self.state.log(id, format!("Mission part: {}", part.title));
                crate::applog::info(
                    "mission",
                    format!(
                        "mission #{mid}: started part {:?} as session #{id}",
                        part.key
                    ),
                );
                self.launch(id, prompt, cx);
            }
            Err(error) => self.stuck(mid, index, format!("Could not start: {error}")),
        }
        self.save();
    }

    fn stuck(&mut self, mid: u64, index: usize, note: String) {
        let goal = self
            .mission(mid)
            .map(|m| m.goal.clone())
            .unwrap_or_default();
        let Some(p) = self.mission_mut(mid).and_then(|m| m.parts.get_mut(index)) else {
            return;
        };
        p.state = PartState::Stuck;
        p.needed_you = true;
        p.note = note.clone();
        let title = p.title.clone();
        let session = p.session;
        self.mission_log(mid, Mark::Stuck, format!("{title} needs you"));
        crate::applog::warn("mission", format!("mission #{mid}: {title}: {note}"));
        if session != 0 {
            self.state
                .log(session, format!("Mission part needs you: {title}"));
            if let Some(s) = self.state.session_mut(session) {
                s.unread = true;
            }
        }
        self.info = Some((
            format!("Mission “{goal}”: {title} needs you."),
            std::time::Instant::now(),
        ));
        self.save();
    }

    fn set_part(&mut self, mid: u64, index: usize, state: PartState, note: &str) {
        let Some(m) = self.mission_mut(mid) else {
            return;
        };
        let Some(p) = m.parts.get_mut(index) else {
            return;
        };
        let was = p.state;
        p.state = state;
        if was == PartState::Checking && state == PartState::Working {
            p.fixes += 1;
        }
        p.note = note.to_string();
        let (title, name) = (p.title.clone(), agent::provider(&p.agent).name);
        let line = match (was, state) {
            (PartState::Checking, PartState::Working) => Some((
                Mark::Fix,
                format!("Checks failed on {title}. {name} is fixing them"),
            )),
            (PartState::Checking, PartState::Reviewing) => Some((
                Mark::Passed,
                format!("Checks passed on {title}. A second agent is reviewing it"),
            )),
            (PartState::Checking, PartState::Proved) => {
                Some((Mark::Passed, format!("Checks passed on {title}")))
            }
            (PartState::Reviewing, PartState::Proved) => Some((
                Mark::Review,
                format!("The second opinion found nothing to change in {title}"),
            )),
            (PartState::Reviewing, PartState::Working) => Some((
                Mark::Review,
                format!("The second opinion left notes on {title}. {name} is on them"),
            )),
            (PartState::Working, PartState::Reviewing) => {
                Some((Mark::Review, format!("A second agent is reviewing {title}")))
            }
            _ => None,
        };
        if let Some((mark, text)) = line {
            m.record(crate::workspace::unix_now(), mark, text);
        }
    }

    /// Sessions looked at again after an event that had no context to act in.
    pub(crate) fn mission_wakes(&mut self, cx: &mut Context<Self>) {
        for id in std::mem::take(&mut self.mission_wake) {
            self.mission_check(id, cx);
        }
    }

    /// A mission's session finished something (a turn, its checks, a review): decide
    /// what its part does next.
    pub(crate) fn mission_check(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let mid = session.mission;
        let Some(m) = self.mission(mid) else {
            return;
        };
        if m.lead == id {
            self.lead_check(mid, cx);
            return;
        }
        let Some(index) = m.part_of(id) else {
            return;
        };
        let part = m.parts[index].clone();
        let review = m.review;
        if !matches!(
            part.state,
            PartState::Working | PartState::Checking | PartState::Reviewing | PartState::Stuck
        ) {
            return;
        }
        let busy = self.turn_running(id)
            || self.preparing.contains_key(&id)
            || self.pending_start.contains_key(&id);
        if busy {
            self.set_part(mid, index, PartState::Working, "");
            self.save();
            return;
        }
        // Stuck parts wait for the person, unless they talked to the agent since.
        if part.state == PartState::Stuck {
            return;
        }
        if self.proving.contains(&id) {
            self.set_part(mid, index, PartState::Checking, "");
            return;
        }
        if self.reviewing.contains_key(&id) {
            self.set_part(mid, index, PartState::Reviewing, "");
            return;
        }
        let Some(session) = self.state.session(id) else {
            return;
        };
        if session.status == Status::Failed {
            self.stuck(
                mid,
                index,
                "The agent stopped with an error. Open the part to see why, then Try again.".into(),
            );
            return;
        }
        let has_checks = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .is_some_and(|p| !proof::commands(&p.checks).is_empty());
        if has_checks && session.connection == 0 {
            let turn = last_ask(session);
            let verdict = session
                .proof
                .as_ref()
                .filter(|p| Some(p.turn) == turn)
                .cloned();
            match verdict {
                None => {
                    self.prove_turn(id, true, cx);
                    if self.proving.contains(&id) {
                        self.set_part(mid, index, PartState::Checking, "");
                        self.save();
                        return;
                    }
                }
                Some(p) if p.skipped.is_empty() && !p.passed() => {
                    let failed: Vec<String> = p
                        .failed()
                        .iter()
                        .map(|r| format!("`{}`", r.command))
                        .collect();
                    self.stuck(
                        mid,
                        index,
                        format!(
                            "Checks still fail after {} fix round{}: {}. Open it, or Try again for more rounds.",
                            p.rounds,
                            if p.rounds == 1 { "" } else { "s" },
                            failed.join(", ")
                        ),
                    );
                    return;
                }
                Some(_) => {}
            }
        }
        // The second opinion: its notes go back to the part's agent once.
        if part.state == PartState::Reviewing {
            let notes = self
                .state
                .session_mut(id)
                .map(|s| std::mem::take(&mut s.notes))
                .unwrap_or_default();
            if !notes.is_empty() {
                let prompt = store::notes_prompt(&notes);
                let provider = self
                    .state
                    .session(id)
                    .map(|s| s.provider.clone())
                    .unwrap_or_default();
                let opts = cx.global::<AppSettings>().agent(&provider);
                self.set_part(
                    mid,
                    index,
                    PartState::Working,
                    "Fixing the second opinion's notes",
                );
                self.start_turn(id, prompt, Vec::new(), &opts);
                self.save();
                return;
            }
        } else if review && !part.reviewed {
            if let Some(p) = self.mission_mut(mid).and_then(|m| m.parts.get_mut(index)) {
                p.reviewed = true;
            }
            self.second_opinion(id, cx);
            if self.reviewing.contains_key(&id) {
                self.set_part(mid, index, PartState::Reviewing, "");
                self.save();
                return;
            }
        }
        self.set_part(mid, index, PartState::Proved, "");
        self.save();
        self.mission_tick(mid, cx);
    }

    /// Commit a proved part in its worktree and merge it into the mission branch.
    fn merge_part(&mut self, mid: u64, index: usize) {
        let Some(m) = self.mission(mid) else {
            return;
        };
        let Some(part) = m.parts.get(index) else {
            return;
        };
        let Some(session) = self.state.session(part.session) else {
            return;
        };
        let Some(repo) = self.project_of(session).map(|p| p.path) else {
            return;
        };
        let (dir, branch) = (session.dir.clone(), session.branch.clone());
        let target = m.branch.clone();
        let message = part.title.clone();
        let key = part.key.clone();
        self.set_part(mid, index, PartState::Merging, "");
        self.bg(
            move || {
                if let Some(kind) = git::in_progress(&dir) {
                    return Err(format!(
                        "A {kind} is half done in the part's worktree. Finish or abort it there."
                    ));
                }
                if git::has_uncommitted(&dir) == Some(true) {
                    git::commit(&dir, &message)?;
                }
                git::merge_into(&repo, &branch, &target)
            },
            move |this, result, cx| this.part_merged(mid, &key, result, cx),
        );
    }

    fn part_merged(
        &mut self,
        mid: u64,
        key: &str,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(m) = self.mission(mid) else {
            return;
        };
        let Some(index) = m.parts.iter().position(|p| p.key == key) else {
            return;
        };
        let part = m.parts[index].clone();
        let target = m.branch.clone();
        let lead = m.lead;
        match result {
            Ok(commit) => {
                self.set_part(mid, index, PartState::Done, &format!("Merged as {commit}"));
                if let Some(p) = self.mission_mut(mid).and_then(|m| m.parts.get_mut(index)) {
                    p.finished = crate::workspace::unix_now();
                }
                self.mission_log(
                    mid,
                    Mark::Merge,
                    format!("{} merged into the mission branch", part.title),
                );
                self.state.log(
                    part.session,
                    format!("Merged into {target}: {}", part.title),
                );
                self.measure(lead);
                self.save();
                self.mission_tick(mid, cx);
            }
            Err(error) if error.contains("conflicts") => {
                if part.conflicts >= mission::CONFLICT_ROUNDS {
                    self.stuck(
                        mid,
                        index,
                        format!(
                            "Still conflicts with {target} after {} tries. Open it to resolve them, then Try again.",
                            part.conflicts
                        ),
                    );
                    return;
                }
                if let Some(p) = self.mission_mut(mid).and_then(|m| m.parts.get_mut(index)) {
                    p.conflicts += 1;
                    p.state = PartState::Working;
                    p.note = format!("Bringing in {target} to resolve conflicts");
                }
                self.mission_log(
                    mid,
                    Mark::Conflict,
                    format!(
                        "{} conflicts with {target}. {} is resolving it",
                        part.title,
                        agent::provider(&part.agent).name
                    ),
                );
                self.save();
                let id = part.session;
                let Some(dir) = self.state.session(id).map(|s| s.dir.clone()) else {
                    return;
                };
                let onto = target.clone();
                self.bg(
                    move || git::update_from(&dir, &onto, false),
                    move |this, update, cx| match update {
                        Ok(git::Update::Conflicts(files)) => {
                            let prompt = ship::conflict_prompt("merge", &target, &files);
                            let provider = this
                                .state
                                .session(id)
                                .map(|s| s.provider.clone())
                                .unwrap_or_default();
                            let opts = cx.global::<AppSettings>().agent(&provider);
                            this.start_turn(id, prompt, Vec::new(), &opts);
                            cx.notify();
                        }
                        Ok(_) => {
                            // Merged cleanly in the part's worktree: try landing again.
                            this.set_part(mid, index, PartState::Proved, "");
                            this.save();
                            this.mission_tick(mid, cx);
                        }
                        Err(error) => this.stuck(
                            mid,
                            index,
                            format!(
                                "Could not bring in {target}: {}",
                                error.lines().next().unwrap_or("")
                            ),
                        ),
                    },
                );
            }
            Err(error) => self.stuck(
                mid,
                index,
                format!("Could not merge: {}", error.lines().next().unwrap_or("")),
            ),
        }
    }

    /// Every part is in: run the checks on the mission branch, in its own session.
    fn land(&mut self, mid: u64, cx: &mut Context<Self>) {
        let Some(m) = self.mission_mut(mid) else {
            return;
        };
        m.stage = Stage::Landing;
        m.note = "Every part is merged. Running the checks on the mission branch.".into();
        m.record(
            crate::workspace::unix_now(),
            Mark::Land,
            "Every part is in. Running the checks on the mission branch",
        );
        let lead = m.lead;
        let project = m.project_id;
        let checks = self
            .state
            .projects
            .iter()
            .find(|p| p.id == project)
            .map(|p| proof::commands(&p.checks))
            .unwrap_or_default();
        let Some(dir) = self.state.session(lead).map(|s| s.dir.clone()) else {
            return;
        };
        self.save();
        if checks.is_empty() {
            self.mission_shippable(mid, "Every part is merged.", cx);
            return;
        }
        self.proving.insert(lead);
        self.bg(
            move || proof::run_all(&dir, &checks, proof::CHECK_LIMIT),
            move |this, results, cx| {
                this.proving.remove(&lead);
                let passed = !results.is_empty() && results.iter().all(|r| r.ok);
                if passed {
                    let n = results.len();
                    this.mission_shippable(
                        mid,
                        &format!(
                            "Every part is merged and {} on the mission branch.",
                            if n == 1 {
                                "the check passes".to_string()
                            } else {
                                format!("all {n} checks pass")
                            }
                        ),
                        cx,
                    );
                    return;
                }
                // The mission's own agent fixes it, and its turn is proved like any other.
                let prompt = proof::fix_prompt(&results);
                let provider = this
                    .state
                    .session(lead)
                    .map(|s| s.provider.clone())
                    .unwrap_or_default();
                if let Some(m) = this.mission_mut(mid) {
                    m.note = format!(
                        "The parts work alone but not together. {} is fixing the mission branch.",
                        agent::provider(&provider).name
                    );
                    let note = m.note.trim_end_matches('.').to_string();
                    m.record(crate::workspace::unix_now(), Mark::Fix, note);
                }
                let opts = cx.global::<AppSettings>().agent(&provider);
                this.start_turn(lead, prompt, Vec::new(), &opts);
                this.save();
                cx.notify();
            },
        );
    }

    /// The mission's own session finished a turn or its checks while landing.
    fn lead_check(&mut self, mid: u64, cx: &mut Context<Self>) {
        let Some(m) = self.mission(mid) else {
            return;
        };
        let lead = m.lead;
        if m.stage != Stage::Landing || self.turn_running(lead) || self.proving.contains(&lead) {
            return;
        }
        let Some(session) = self.state.session(lead) else {
            return;
        };
        let turn = last_ask(session);
        let verdict = session
            .proof
            .as_ref()
            .filter(|p| Some(p.turn) == turn)
            .cloned();
        match verdict {
            Some(p) if p.passed() || !p.skipped.is_empty() => {
                self.mission_shippable(mid, "Every part is merged and the checks pass.", cx)
            }
            Some(_) => {
                if let Some(m) = self.mission_mut(mid) {
                    m.note = "The checks still fail on the mission branch. Open its session to fix them, or ship it as it is from Review.".into();
                }
                self.save();
            }
            None if session.status == Status::Failed => {
                if let Some(m) = self.mission_mut(mid) {
                    m.note =
                        "Fixing the mission branch failed. Open its session to see why.".into();
                }
                self.save();
            }
            None => self.prove_turn(lead, true, cx),
        }
    }

    fn mission_shippable(&mut self, mid: u64, note: &str, cx: &mut Context<Self>) {
        let Some(m) = self.mission_mut(mid) else {
            return;
        };
        m.stage = Stage::Ready;
        m.finished = crate::workspace::unix_now();
        m.record(m.finished, Mark::Ready, "Ready to ship");
        m.note = format!("{note} Review the mission branch, then open a pull request or merge it.");
        let (lead, goal) = (m.lead, m.goal.clone());
        if let Some(s) = self.state.session_mut(lead) {
            s.unread = true;
        }
        self.state
            .log(lead, format!("Mission ready to ship: {goal}"));
        crate::applog::info("mission", format!("mission #{mid}: ready to ship"));
        crate::desktop_notify::send(cx, Some(lead), "Mission ready to ship", &goal);
        self.measure(lead);
        self.save();
        cx.notify();
    }

    /// Try a stuck part again: more fix rounds, the agent picks up after an error, or the
    /// merge runs again.
    pub(crate) fn retry_part(&mut self, mid: u64, index: usize, cx: &mut Context<Self>) {
        let Some(part) = self.mission(mid).and_then(|m| m.parts.get(index)).cloned() else {
            return;
        };
        if part.session == 0 {
            self.set_part(mid, index, PartState::Waiting, "");
            self.save();
            self.mission_tick(mid, cx);
            return;
        }
        let Some(session) = self.state.session(part.session) else {
            return;
        };
        let failed_checks = session.proof.as_ref().is_some_and(|p| {
            p.turn == last_ask(session).unwrap_or(usize::MAX) && !p.passed() && p.skipped.is_empty()
        });
        let provider = session.provider.clone();
        let errored = session.status == Status::Failed;
        let id = part.session;
        if let Some(p) = self.mission_mut(mid).and_then(|m| m.parts.get_mut(index)) {
            p.conflicts = 0;
        }
        self.mission_log(mid, Mark::Part, format!("Trying {} again", part.title));
        if errored || failed_checks {
            if let Some(s) = self.state.session_mut(id) {
                if let Some(p) = s.proof.as_mut() {
                    p.rounds = 0;
                    p.last_failure.clear();
                }
            }
            self.set_part(mid, index, PartState::Working, "");
            if failed_checks {
                self.fix_checks(id, cx);
            } else {
                let opts = cx.global::<AppSettings>().agent(&provider);
                self.start_turn(
                    id,
                    "Your last turn stopped with an error. Pick up your part where it left off and finish it.".into(),
                    Vec::new(),
                    &opts,
                );
            }
        } else {
            self.set_part(mid, index, PartState::Proved, "");
            self.mission_tick(mid, cx);
        }
        self.save();
        cx.notify();
    }

    pub(crate) fn skip_part(&mut self, mid: u64, index: usize, cx: &mut Context<Self>) {
        self.set_part(mid, index, PartState::Skipped, "Skipped");
        if let Some(title) = self
            .mission(mid)
            .and_then(|m| m.parts.get(index))
            .map(|p| p.title.clone())
        {
            self.mission_log(mid, Mark::Skip, format!("{title} skipped"));
        }
        self.save();
        self.mission_tick(mid, cx);
        cx.notify();
    }

    pub(crate) fn delete_mission(&mut self, mid: u64) {
        self.state.missions.retain(|m| m.id != mid);
        for s in &mut self.state.sessions {
            if s.mission == mid {
                s.mission = 0;
            }
        }
        if self.mission_open == Some(mid) {
            self.mission_open = None;
        }
        self.save();
    }
}
