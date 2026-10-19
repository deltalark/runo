//! Missions in the app: planning with an agent, starting each part as its own session
//! when the parts before it are in, proving and merging each one into the mission
//! branch, landing the mission, and the Missions page with its board.

use super::*;
use crate::mission::{self, Mission, Part, PartState, Stage};
use crate::proof;

/// Width of the list of missions.
const LIST_WIDTH: f32 = 280.;
/// Parallel parts offered on the form.
const PARALLEL: [u32; 4] = [1, 2, 3, 4];

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

fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Planning => "Planning",
        Stage::Plan => "Check the plan",
        Stage::Running => "Running",
        Stage::Paused => "Paused",
        Stage::Landing => "Landing",
        Stage::Ready => "Ready to ship",
        Stage::Failed => "Planning failed",
    }
}

impl RunoApp {
    fn mission(&self, mid: u64) -> Option<&Mission> {
        self.state.missions.iter().find(|m| m.id == mid)
    }

    fn mission_mut(&mut self, mid: u64) -> Option<&mut Mission> {
        self.state.missions.iter_mut().find(|m| m.id == mid)
    }

    fn installed_agents(&self, cx: &App) -> Vec<&'static str> {
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
    fn mission_form_defaults(&mut self, cx: &App) {
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
    fn plan_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    fn run_planner(&mut self, mid: u64, feedback: String, cx: &mut Context<Self>) {
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
        let prompt = mission::plan_prompt(&m.goal, &roster, m.parallel, &feedback, &m.parts);
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
            m.note.clear();
        }
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
    fn start_mission(&mut self, mid: u64, cx: &mut Context<Self>) {
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
                    m.note.clear();
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
                    p.note.clear();
                }
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
        p.note = note.clone();
        let title = p.title.clone();
        let session = p.session;
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
        if let Some(p) = self.mission_mut(mid).and_then(|m| m.parts.get_mut(index)) {
            p.state = state;
            p.note = note.to_string();
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
    fn retry_part(&mut self, mid: u64, index: usize, cx: &mut Context<Self>) {
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

    fn skip_part(&mut self, mid: u64, index: usize, cx: &mut Context<Self>) {
        self.set_part(mid, index, PartState::Skipped, "Skipped");
        self.save();
        self.mission_tick(mid, cx);
        cx.notify();
    }

    fn delete_mission(&mut self, mid: u64) {
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

    // ---------- the page ----------

    pub(crate) fn missions_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut missions: Vec<&Mission> = self.state.missions.iter().collect();
        missions.sort_by_key(|m| std::cmp::Reverse(m.id));
        let rows: Vec<AnyElement> = missions.iter().map(|m| self.mission_row(m, cx)).collect();
        let none = missions.is_empty();
        let side = div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .border_r_1()
            .border_color(divider())
            .bg(pane(c_sidebar()))
            .child(
                div()
                    .h(px(46.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Missions")
                    .child(div().flex_1())
                    .child(
                        self.button("mission-new", "New", self.mission_open.is_some())
                            .h(px(26.))
                            .px_2()
                            .text_size(px(12.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.mission_open = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("mission-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .v_flex()
                    .gap(px(2.))
                    .children(rows)
                    .when(none, |el| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(12.5))
                                .text_color(rgb(c_muted()))
                                .child("No missions yet. Give Runo a goal, and it plans the work, runs the parts on several agents at once, checks each one and merges them into one branch."),
                        )
                    }),
            );
        let main = match self.mission_open.and_then(|id| self.mission(id)) {
            Some(m) => self.mission_page(m, cx),
            None => self.mission_form_view(cx),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(side)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .bg(glass_fill(c_canvas()))
                    .child(main),
            )
            .into_any_element()
    }

    fn mission_row(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let id = m.id;
        let on = self.mission_open == Some(id);
        let (done, total) = m.progress();
        let stuck = m.parts.iter().any(|p| p.state == PartState::Stuck);
        let tone = match m.stage {
            Stage::Ready => c_green(),
            Stage::Failed => c_red(),
            Stage::Plan => c_amber(),
            _ if stuck => c_amber(),
            Stage::Paused => c_muted(),
            _ => c_blue(),
        };
        let line = match m.stage {
            Stage::Running | Stage::Paused | Stage::Landing if total > 0 => {
                format!(
                    "{done} of {total} parts  ·  {}",
                    if stuck {
                        "Needs you"
                    } else {
                        stage_label(m.stage)
                    }
                )
            }
            stage => stage_label(stage).to_string(),
        };
        let fraction = if total == 0 {
            0.
        } else {
            done as f32 / total as f32
        };
        div()
            .id(("mission-row", id as usize))
            .mx_1()
            .px_2()
            .py(px(7.))
            .v_flex()
            .gap_1()
            .rounded_md()
            .cursor_pointer()
            .when(on, |el| el.bg(tint(c_active())))
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.mission_open = Some(id);
                cx.notify();
            }))
            .child(
                div()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(m.goal.lines().next().unwrap_or("").to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(div().size(px(6.)).rounded_full().bg(rgb(tone)))
                    .child(line),
            )
            .when(total > 0 && m.stage != Stage::Plan, |el| {
                el.child(
                    div()
                        .h(px(3.))
                        .w_full()
                        .rounded_full()
                        .bg(tint(c_subtle()))
                        .child(
                            div()
                                .h_full()
                                .w(relative(fraction))
                                .rounded_full()
                                .bg(rgb(tone)),
                        ),
                )
            })
            .into_any_element()
    }

    fn section_label(&self, text: &'static str) -> Div {
        div()
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(c_secondary()))
            .child(text)
    }

    fn mission_form_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let form = &self.mission_form;
        let installed = self.installed_agents(cx);
        let project = Some(form.project)
            .filter(|p| *p != 0)
            .or_else(|| self.current_project())
            .or_else(|| self.state.projects.first().map(|p| p.id));
        let agent_chips: Vec<AnyElement> = installed
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let id = *id;
                let p = agent::provider(id);
                let order = form.agents.iter().position(|a| a == id);
                let label = match order {
                    Some(0) => format!("{}  ·  plans", p.name),
                    _ => p.name.to_string(),
                };
                self.chip(("mission-agent", i), label, order.is_some())
                    .gap_1p5()
                    .child(self.provider_mark(p, 12.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let agents = &mut this.mission_form.agents;
                        match agents.iter().position(|a| a == id) {
                            Some(at) => {
                                agents.remove(at);
                            }
                            None => agents.push(id.to_string()),
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let project_chips: Vec<AnyElement> = self
            .state
            .projects
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let pid = p.id;
                self.chip(("mission-project", i), p.name.clone(), project == Some(pid))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mission_form.project = pid;
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let parallel = form.parallel;
        let review = form.review;
        div()
            .id("mission-form")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(680.))
                    .mx_auto()
                    .px_6()
                    .py_8()
                    .v_flex()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon(IconName::Rocket, 20., c_text()))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("New mission"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(c_secondary()))
                            .child("Give Runo one goal. A planner splits it into parts, agents from different vendors work on them at once in their own worktrees, and each part is merged into one branch only when its checks pass."),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(self.section_label("Goal"))
                            .child(Textarea::new(&self.mission_goal).bordered(true).h(px(120.))),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(self.section_label("Project"))
                            .child(div().flex().flex_wrap().gap_1p5().children(project_chips)),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1p5()
                            .child(self.section_label("Agents that may work on it"))
                            .child(div().flex().flex_wrap().gap_1p5().children(agent_chips))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .child(if installed.is_empty() {
                                        "No agent CLIs are installed. Add one in Settings → Providers."
                                    } else {
                                        "The first one you pick writes the plan. The planner picks an agent for each part, and you can change it before starting."
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1p5()
                            .child(self.section_label("Parts at once").mr_1())
                            .children(PARALLEL.iter().enumerate().map(|(i, n)| {
                                let n = *n;
                                self.chip(("mission-parallel", i), n.to_string(), parallel == n)
                                    .h(px(26.))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.mission_form.parallel = n;
                                        cx.notify();
                                    }))
                            }))
                            .child(div().w(px(16.)))
                            .child(self.section_label("Second opinion on each part").mr_1())
                            .children([(0usize, "On", true), (1, "Off", false)].map(
                                |(i, label, on)| {
                                    self.chip(("mission-review", i), label.into(), review == on)
                                        .h(px(26.))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.mission_form.review = on;
                                            cx.notify();
                                        }))
                                },
                            )),
                    )
                    .when_some(form.error.clone(), |el, error| {
                        el.child(
                            div()
                                .text_size(px(12.5))
                                .text_color(rgb(c_red()))
                                .child(error),
                        )
                    })
                    .child(
                        div().flex().child(
                            self.button("mission-plan", "Plan mission", true)
                                .h(px(34.))
                                .px_4()
                                .child(icon(IconName::Sparkles, 13., c_canvas()))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.mission_form_defaults(cx);
                                    this.plan_mission(window, cx);
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn mission_page(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let mid = m.id;
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == m.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let stuck = m.parts.iter().any(|p| p.state == PartState::Stuck);
        let tone = match m.stage {
            Stage::Ready => c_green(),
            Stage::Failed => c_red(),
            _ if stuck || m.stage == Stage::Plan => c_amber(),
            _ => c_blue(),
        };
        let lead = m.lead;
        let header = div()
            .min_h(px(46.))
            .flex_none()
            .px_4()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(divider())
            .child(icon(IconName::Rocket, 15., c_text()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(m.goal.lines().next().unwrap_or("").to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .h(px(22.))
                    .rounded_md()
                    .bg(tint(c_subtle()))
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .child(icon(IconName::GitBranch, 11., c_muted()))
                    .child(format!("{project}  ·  {}", m.branch)),
            )
            .child(
                div()
                    .flex_none()
                    .px_2()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(tone))
                    .text_size(px(11.5))
                    .text_color(rgb(tone))
                    .child(
                        if stuck && matches!(m.stage, Stage::Running | Stage::Paused) {
                            "Needs you"
                        } else {
                            stage_label(m.stage)
                        },
                    ),
            )
            .when(m.stage == Stage::Running, |el| {
                el.child(
                    self.button("mission-pause", "Pause", false)
                        .h(px(26.))
                        .px_2()
                        .text_size(px(12.))
                        .child(icon(IconName::Pause, 12., c_secondary()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(m) = this.mission_mut(mid) {
                                m.stage = Stage::Paused;
                            }
                            this.save();
                            cx.notify();
                        })),
                )
            })
            .when(m.stage == Stage::Paused, |el| {
                el.child(
                    self.button("mission-resume", "Resume", true)
                        .h(px(26.))
                        .px_2()
                        .text_size(px(12.))
                        .child(icon(IconName::Play, 12., c_canvas()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(m) = this.mission_mut(mid) {
                                m.stage = Stage::Running;
                            }
                            this.mission_tick(mid, cx);
                            this.save();
                            cx.notify();
                        })),
                )
            })
            .when(lead != 0 && self.state.session(lead).is_some(), |el| {
                el.child(
                    self.button("mission-lead", "Mission session", false)
                        .h(px(26.))
                        .px_2()
                        .text_size(px(12.))
                        .child(icon(IconName::MessageSquare, 12., c_secondary()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select(lead);
                            this.view = View::Session;
                            cx.notify();
                        })),
                )
            })
            .when(m.stage == Stage::Ready, |el| {
                el.child(
                    self.button("mission-ship", "Review and ship", true)
                        .h(px(26.))
                        .px_2()
                        .text_size(px(12.))
                        .child(icon(IconName::GitCompare, 12., c_canvas()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.review(lead);
                            cx.notify();
                        })),
                )
            })
            .child(
                self.ghost_button_sized(
                    ("mission-delete", mid as usize),
                    IconName::Trash,
                    "Delete the mission (its sessions stay)",
                    26.,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.delete_mission(mid);
                    cx.notify();
                })),
            );
        let mut about = div().px_4().pt_3().v_flex().gap_1();
        if !m.summary.is_empty() {
            about = about.child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(c_secondary()))
                    .child(m.summary.clone()),
            );
        }
        if !m.note.is_empty() {
            about = about.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(12.5))
                    .text_color(rgb(match m.stage {
                        Stage::Failed => c_red(),
                        Stage::Ready => c_green(),
                        _ => c_secondary(),
                    }))
                    .child(icon(
                        match m.stage {
                            Stage::Ready => IconName::CircleCheck,
                            Stage::Failed => IconName::CircleAlert,
                            Stage::Landing if self.proving.contains(&lead) => {
                                IconName::LoaderCircle
                            }
                            _ => IconName::Info,
                        },
                        13.,
                        tone,
                    ))
                    .child(m.note.clone()),
            );
        }
        if m.waiting_on_you() {
            about = about.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(12.5))
                    .text_color(rgb(c_amber()))
                    .child(icon(IconName::TriangleAlert, 13., c_amber()))
                    .child("Nothing else can move until you look at the parts that need you."),
            );
        }
        let body = match m.stage {
            Stage::Planning => div()
                .flex_1()
                .v_flex()
                .items_center()
                .justify_center()
                .gap_2()
                .child(icon(IconName::LoaderCircle, 22., c_muted()))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!(
                            "{} is reading the project and planning…",
                            agent::provider(
                                m.agents.first().map(String::as_str).unwrap_or("claude")
                            )
                            .name
                        )),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_muted()))
                        .child("It only reads; nothing changes until you start the mission."),
                )
                .into_any_element(),
            Stage::Plan | Stage::Failed => self.plan_review(m, cx),
            _ => self.mission_board(m, cx),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(header)
            .child(about)
            .child(body)
            .into_any_element()
    }

    /// The plan before it starts: each part, its agent and order, and Plan again.
    fn plan_review(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let mid = m.id;
        let cards: Vec<AnyElement> = m
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| self.mission_plan_card(m, i, p, cx))
            .collect();
        let has_parts = !m.parts.is_empty();
        div()
            .id("mission-plan")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .v_flex()
            .gap_2()
            .children(cards)
            .child(
                div()
                    .mt_2()
                    .v_flex()
                    .gap_1p5()
                    .child(self.section_label(if has_parts {
                        "Not quite right? Say what to change"
                    } else {
                        "Anything the planner should know?"
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().child(
                                Input::new(&self.mission_feedback).bordered(true).h(px(32.)),
                            ))
                            .child(
                                self.button("mission-replan", "Plan again", !has_parts)
                                    .child(icon(
                                        IconName::RotateCw,
                                        12.,
                                        if has_parts { c_secondary() } else { c_canvas() },
                                    ))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let feedback = this
                                            .mission_feedback
                                            .read(cx)
                                            .value()
                                            .trim()
                                            .to_string();
                                        this.mission_feedback
                                            .update(cx, |i, cx| i.set_value("", window, cx));
                                        this.run_planner(mid, feedback, cx);
                                        this.save();
                                        cx.notify();
                                    })),
                            )
                            .when(has_parts, |el| {
                                el.child(
                                    self.button("mission-start", "Start mission", true)
                                        .child(icon(IconName::Play, 12., c_canvas()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.start_mission(mid, cx);
                                        })),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn mission_plan_card(
        &self,
        m: &Mission,
        index: usize,
        part: &Part,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mid = m.id;
        let provider = agent::provider(&part.agent);
        let deps = m.dep_titles(part);
        let pool = m.agents.clone();
        let mut preview: String = part.prompt.chars().take(360).collect();
        if part.prompt.chars().count() > 360 {
            preview.push('…');
        }
        div()
            .p_3()
            .v_flex()
            .gap_1p5()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_raised()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(20.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(tint(c_subtle()))
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child((index + 1).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(part.title.clone()),
                    )
                    .child(
                        div()
                            .id(("mission-part-agent", index))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .h(px(24.))
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(c_border()))
                            .cursor_pointer()
                            .hover(|el| el.bg(tint(c_subtle())))
                            .text_size(px(12.))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Switch the agent for this part",
                                )
                                .build(window, cx)
                            })
                            .child(self.provider_mark(provider, 12.))
                            .child(provider.name)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let Some(p) =
                                    this.mission_mut(mid).and_then(|m| m.parts.get_mut(index))
                                else {
                                    return;
                                };
                                let at = pool
                                    .iter()
                                    .position(|a| *a == p.agent)
                                    .map(|i| i + 1)
                                    .unwrap_or(0);
                                if let Some(next) = pool.get(at % pool.len().max(1)) {
                                    p.agent = next.clone();
                                }
                                this.save();
                                cx.notify();
                            })),
                    )
                    .child(
                        self.ghost_button_sized(
                            ("mission-part-remove", index),
                            IconName::X,
                            "Leave this part out",
                            24.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(m) = this.mission_mut(mid) {
                                m.remove_part(index);
                            }
                            this.save();
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child(preview),
            )
            .when(!deps.is_empty() || !part.files.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x_3()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .when(!deps.is_empty(), |el| {
                            el.child(format!("After: {}", deps.join(", ")))
                        })
                        .when(!part.files.is_empty(), |el| {
                            el.child(format!("Files: {}", part.files.join(", ")))
                        }),
                )
            })
            .into_any_element()
    }

    /// Waiting, Working, Checking and Done, one card per part.
    fn mission_board(&self, m: &Mission, cx: &mut Context<Self>) -> AnyElement {
        let titles = ["Waiting", "Working", "Checking", "Done"];
        let mut columns: Vec<Vec<AnyElement>> =
            vec![Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (i, p) in m.parts.iter().enumerate() {
            columns[p.state.column()].push(self.part_card(m, i, p, cx));
        }
        div()
            .flex_1()
            .min_h_0()
            .px_4()
            .py_3()
            .flex()
            .gap_3()
            .children(columns.into_iter().enumerate().map(|(c, cards)| {
                let count = cards.len();
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
                            .child(titles[c].to_uppercase())
                            .child(count.to_string()),
                    )
                    .child(
                        div()
                            .id(("mission-column", c))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .v_flex()
                            .gap_2()
                            .p_1()
                            .rounded_lg()
                            .bg(tint(c_subtle()))
                            .children(cards),
                    )
            }))
            .into_any_element()
    }

    fn part_card(
        &self,
        m: &Mission,
        index: usize,
        part: &Part,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mid = m.id;
        let provider = agent::provider(&part.agent);
        let session = part.session;
        let stuck = part.state == PartState::Stuck;
        let tone = match part.state {
            PartState::Done => c_green(),
            PartState::Stuck => c_amber(),
            PartState::Waiting | PartState::Skipped => c_muted(),
            _ => c_blue(),
        };
        let deps: Vec<String> = m
            .parts
            .iter()
            .filter(|d| part.deps.contains(&d.key) && !d.state.settled())
            .map(|d| d.title.clone())
            .collect();
        let asking = session != 0 && self.approval_for(session).is_some();
        let live = matches!(
            part.state,
            PartState::Working | PartState::Checking | PartState::Reviewing | PartState::Merging
        );
        let small = |this: &Self, key: &'static str, label: &'static str, primary: bool| {
            this.button((key, index), label, primary)
                .h(px(24.))
                .px_2()
                .text_size(px(12.))
        };
        div()
            .id(("mission-part", index))
            .p_2p5()
            .v_flex()
            .gap_1p5()
            .rounded_md()
            .border_1()
            .border_color(rgb(if stuck { c_amber() } else { c_border() }))
            .bg(rgb(c_raised()))
            .when(session != 0, |el| {
                el.cursor_pointer()
                    .hover(|el| el.border_color(rgb(c_blue())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(session);
                        this.view = View::Session;
                        cx.notify();
                    }))
            })
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(part.title.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(11.5))
                    .child(self.provider_mark(provider, 12.))
                    .child(div().text_color(rgb(c_secondary())).child(provider.name))
                    .child(div().flex_1())
                    .when(live, |el| el.child(icon(IconName::LoaderCircle, 11., tone)))
                    .child(div().text_color(rgb(tone)).child(part.state.label())),
            )
            .when(asking, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(11.5))
                        .text_color(rgb(c_amber()))
                        .child(icon(IconName::ShieldAlert, 11., c_amber()))
                        .child("Asks to run a command. Open it to answer."),
                )
            })
            .when(part.state == PartState::Waiting && !deps.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(format!("After {}", deps.join(", "))),
                )
            })
            .when(
                !part.note.is_empty() && part.state != PartState::Skipped,
                |el| {
                    el.child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(if stuck { c_text() } else { c_muted() }))
                            .child(part.note.clone()),
                    )
                },
            )
            .when(stuck, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1p5()
                        .child(
                            small(self, "mission-part-retry", "Try again", true).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.retry_part(mid, index, cx);
                                }),
                            ),
                        )
                        .child(small(self, "mission-part-skip", "Skip", false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.skip_part(mid, index, cx);
                            }),
                        )),
                )
            })
            .into_any_element()
    }
}
