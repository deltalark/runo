//! Missions: one goal, planned into parts that agents from different vendors work on at
//! once, each in its own worktree. A part is done when its checks pass (and, when asked,
//! another vendor's agent has reviewed it); then it is merged into the mission branch,
//! and the parts waiting on it start. This file holds the data and the rules; the app
//! side is in `mission_ui.rs`.

use serde::{Deserialize, Serialize};

/// Parts a plan may have; more is cut off.
pub const MAX_PARTS: usize = 8;
/// Fix rounds a part gets when its checks fail, whatever the project's own setting.
pub const PART_FIX_ROUNDS: u32 = 2;
/// Times a part goes back to its agent to resolve conflicts with the mission branch.
pub const CONFLICT_ROUNDS: u32 = 2;
/// Entries a mission's log keeps; the oldest go first.
pub const LOG_LIMIT: usize = 200;
/// How long the planner may take.
pub const PLAN_LIMIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Where a mission stands.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// The planner is writing the plan.
    #[default]
    Planning,
    /// The plan waits for the person to start it.
    Plan,
    /// Parts are being worked on.
    Running,
    /// No new parts start; running ones finish.
    Paused,
    /// Every part is in: the checks run on the mission branch.
    Landing,
    /// The mission branch passed its checks and waits to be shipped.
    Ready,
    /// Planning failed; the note says why.
    Failed,
}

/// Where one part stands.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum PartState {
    /// Waits for the parts it depends on, or for a free agent.
    #[default]
    Waiting,
    /// Its agent is working on it.
    Working,
    /// The project's checks are running on it.
    Checking,
    /// Another vendor's agent is reviewing it.
    Reviewing,
    /// Proved, and waiting its turn to be merged.
    Proved,
    /// Being merged into the mission branch.
    Merging,
    Done,
    /// Stopped: checks still fail, the agent failed, or the merge did; the note says why.
    Stuck,
    /// Left out by the person.
    Skipped,
}

impl PartState {
    /// Finished one way or another: the parts after it may start.
    pub fn settled(self) -> bool {
        matches!(self, PartState::Done | PartState::Skipped)
    }

    /// An agent is busy with the part, so it counts against the parallel limit.
    pub fn busy(self) -> bool {
        matches!(
            self,
            PartState::Working | PartState::Checking | PartState::Reviewing
        )
    }

    /// How far along the part's pipeline it is: 0 working, 1 checks, 2 second opinion,
    /// 3 merging, 4 merged. Waiting, stuck and skipped parts have no step.
    pub fn step(self) -> Option<usize> {
        match self {
            PartState::Working => Some(0),
            PartState::Checking => Some(1),
            PartState::Reviewing => Some(2),
            PartState::Proved | PartState::Merging => Some(3),
            PartState::Done => Some(4),
            PartState::Waiting | PartState::Stuck | PartState::Skipped => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PartState::Waiting => "Waiting",
            PartState::Working => "Working",
            PartState::Checking => "Running checks",
            PartState::Reviewing => "Second opinion",
            PartState::Proved => "Waiting to merge",
            PartState::Merging => "Merging",
            PartState::Done => "Merged",
            PartState::Stuck => "Needs you",
            PartState::Skipped => "Skipped",
        }
    }
}

/// One part of a mission: a prompt for one agent.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Part {
    /// Short id the plan uses for dependencies, such as "api".
    pub key: String,
    pub title: String,
    pub prompt: String,
    /// Keys of the parts that must be merged first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    /// The agent that works on it, such as "claude".
    pub agent: String,
    /// Files the planner expects it to touch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default)]
    pub state: PartState,
    /// Its session, once started.
    #[serde(default)]
    pub session: u64,
    /// Why it is stuck, or what happened last.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Times it went back to its agent over conflicts with the mission branch.
    #[serde(default)]
    pub conflicts: u32,
    /// The second opinion's notes already went back once.
    #[serde(default)]
    pub reviewed: bool,
    /// Unix seconds it started and was merged, for how long it took.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub started: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub finished: u64,
}

/// A goal and its plan.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Mission {
    pub id: u64,
    pub project_id: u64,
    pub goal: String,
    /// The planner's one-line summary.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub stage: Stage,
    /// Agents the parts may use; the first one plans.
    pub agents: Vec<String>,
    /// Parts worked on at once.
    pub parallel: u32,
    /// Another vendor's agent reviews each part before it is merged.
    #[serde(default)]
    pub review: bool,
    /// The branch the parts are merged into, such as `mission/dark-mode`.
    pub branch: String,
    /// The branch the mission starts from and ships to.
    pub base: String,
    /// The mission's own session, on `branch`: its worktree takes the merges, and it is
    /// what gets shipped.
    #[serde(default)]
    pub lead: u64,
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Why planning failed, or what happened last.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Unix seconds it was created.
    #[serde(default)]
    pub created: u64,
    /// Unix seconds the parts started running, and the mission was ready.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub started: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub finished: u64,
    /// Unix seconds the planner last started.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub planned: u64,
    /// What happened, oldest first, for the activity feed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<Entry>,
}

/// What kind of thing happened on a mission, for the feed's icon and colour.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mark {
    #[default]
    Info,
    Start,
    Part,
    Passed,
    Fix,
    Review,
    Conflict,
    Merge,
    Stuck,
    Skip,
    Pause,
    Land,
    Ready,
}

/// One line of a mission's log.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Entry {
    pub at: u64,
    pub mark: Mark,
    pub text: String,
}

impl Mission {
    /// Add a line to the log, dropping the oldest past `LOG_LIMIT`.
    pub fn record(&mut self, at: u64, mark: Mark, text: impl Into<String>) {
        self.log.push(Entry {
            at,
            mark,
            text: text.into(),
        });
        let over = self.log.len().saturating_sub(LOG_LIMIT);
        self.log.drain(..over);
    }

    /// Parts settled, of all.
    pub fn progress(&self) -> (usize, usize) {
        (
            self.parts.iter().filter(|p| p.state.settled()).count(),
            self.parts.len(),
        )
    }

    /// Every part is merged or skipped.
    pub fn all_in(&self) -> bool {
        !self.parts.is_empty() && self.parts.iter().all(|p| p.state.settled())
    }

    /// Parts to start now, in plan order: waiting, every dependency settled, and room
    /// under the parallel limit.
    pub fn startable(&self) -> Vec<usize> {
        if self.stage != Stage::Running {
            return Vec::new();
        }
        let busy = self.parts.iter().filter(|p| p.state.busy()).count();
        let room = (self.parallel.max(1) as usize).saturating_sub(busy);
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.state == PartState::Waiting && self.deps_settled(p))
            .map(|(i, _)| i)
            .take(room)
            .collect()
    }

    fn deps_settled(&self, part: &Part) -> bool {
        part.deps.iter().all(|d| {
            self.parts
                .iter()
                .find(|p| &p.key == d)
                .is_none_or(|p| p.state.settled())
        })
    }

    /// The next proved part to merge, when none is merging.
    pub fn next_merge(&self) -> Option<usize> {
        if self.parts.iter().any(|p| p.state == PartState::Merging) {
            return None;
        }
        self.parts.iter().position(|p| p.state == PartState::Proved)
    }

    /// Nothing can move without the person: parts are stuck, and nothing else runs or
    /// can start.
    pub fn waiting_on_you(&self) -> bool {
        matches!(self.stage, Stage::Running | Stage::Paused)
            && self.parts.iter().any(|p| p.state == PartState::Stuck)
            && !self.parts.iter().any(|p| {
                p.state.busy() || matches!(p.state, PartState::Proved | PartState::Merging)
            })
            && self.startable().is_empty()
    }

    pub fn part_of(&self, session: u64) -> Option<usize> {
        self.parts
            .iter()
            .position(|p| p.session == session && session != 0)
    }

    /// The parts grouped into waves: the first wave waits on nothing, and each later one
    /// waits only on parts in the waves before it.
    pub fn waves(&self) -> Vec<Vec<usize>> {
        let mut depth: Vec<usize> = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            let d = part
                .deps
                .iter()
                .filter_map(|k| self.parts.iter().position(|p| &p.key == k))
                .filter(|i| *i < depth.len())
                .map(|i| depth[i] + 1)
                .max()
                .unwrap_or(0);
            depth.push(d);
        }
        let mut waves: Vec<Vec<usize>> = Vec::new();
        for (i, d) in depth.into_iter().enumerate() {
            if waves.len() <= d {
                waves.resize(d + 1, Vec::new());
            }
            waves[d].push(i);
        }
        waves
    }

    /// Titles of the parts a part waits on.
    pub fn dep_titles(&self, part: &Part) -> Vec<String> {
        part.deps
            .iter()
            .filter_map(|d| self.parts.iter().find(|p| &p.key == d))
            .map(|p| p.title.clone())
            .collect()
    }

    /// Remove a part from the plan, and from the parts that waited on it.
    pub fn remove_part(&mut self, index: usize) {
        if index >= self.parts.len() {
            return;
        }
        let key = self.parts.remove(index).key;
        for p in &mut self.parts {
            p.deps.retain(|d| *d != key);
        }
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// A branch name from the goal, such as `mission/add-dark-mode`.
pub fn branch_for(goal: &str) -> String {
    let words: Vec<String> = goal
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(5)
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let mut slug = words.join("-");
    slug.truncate(40);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "mission/work".into()
    } else {
        format!("mission/{slug}")
    }
}

/// What the planner is asked. `agents` are (id, name) of the ones it may assign;
/// `feedback` and `previous` come from "Plan again".
pub fn plan_prompt(
    goal: &str,
    agents: &[(&str, &str)],
    parallel: u32,
    feedback: &str,
    previous: &[Part],
) -> String {
    let roster = agents
        .iter()
        .map(|(id, name)| format!("- `{id}`: {name}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut prompt = format!(
        "You are planning a mission in this repository. Several coding agents will carry it out at the same time, each on one part in its own git worktree; each part is merged into one branch once its tests pass.\n\n\
## The goal\n{goal}\n\n\
## How to plan\n\
- Read the code first, so each part names the real files and functions to change.\n\
- Split the goal into 1 to {MAX_PARTS} parts. Fewer, larger parts beat many tiny ones; a small goal is one part.\n\
- Parts that touch different files can run at once ({parallel} at a time). When a part needs another part's work, list that part in `depends_on`; it then starts from the merged result. List parts in the order they can be done, so a part only depends on parts listed before it.\n\
- Each `prompt` is all its agent will know besides the goal: say what to change, where, and how to tell it is done (tests to add or run). Write it as an instruction to a developer.\n\
- Pick each part's `agent` from:\n{roster}\n\n\
## Answer\n\
Answer with only this JSON, in one ```json block, and nothing after it:\n\
```json\n\
{{\"summary\": \"one sentence on the approach\", \"tasks\": [{{\"id\": \"short-id\", \"title\": \"a few words\", \"prompt\": \"the full instruction\", \"depends_on\": [\"other-id\"], \"files\": [\"src/file.rs\"], \"agent\": \"{first}\"}}]}}\n\
```",
        first = agents.first().map(|a| a.0).unwrap_or("claude"),
    );
    if !previous.is_empty() || !feedback.trim().is_empty() {
        prompt.push_str("\n\n## Your previous plan\n");
        for p in previous {
            prompt.push_str(&format!(
                "- {} ({}): {}\n",
                p.title,
                p.agent,
                first_line(&p.prompt)
            ));
        }
        if !feedback.trim().is_empty() {
            prompt.push_str(&format!(
                "\nThe person's feedback on it, which the new plan must follow:\n{}\n",
                feedback.trim()
            ));
        }
    }
    prompt
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut out: String = line.chars().take(140).collect();
    if line.chars().count() > 140 {
        out.push('…');
    }
    out
}

/// The JSON object in a reply: a ```json block, else the outermost braces.
fn json_in(reply: &str) -> Option<serde_json::Value> {
    let mut candidates = Vec::new();
    let mut rest = reply;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let body_start = after.find('\n').map(|n| n + 1).unwrap_or(0);
        let Some(end) = after[body_start..].find("```") else {
            break;
        };
        candidates.push(after[body_start..body_start + end].to_string());
        rest = &after[body_start + end + 3..];
    }
    if let (Some(a), Some(b)) = (reply.find('{'), reply.rfind('}')) {
        if a < b {
            candidates.push(reply[a..=b].to_string());
        }
    }
    candidates
        .iter()
        .rev()
        .filter_map(|c| serde_json::from_str::<serde_json::Value>(c.trim()).ok())
        .find(|v| {
            v.get("tasks")
                .or_else(|| v.get("parts"))
                .is_some_and(|t| t.is_array())
        })
}

fn text_of(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn list_of(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|i| text_of(Some(i)))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    }
}

/// Read the planner's reply into a summary and parts. Dependencies only point back to
/// earlier parts, so the plan can always finish; agents outside `agents` are swapped for
/// ones in it.
pub fn parse_plan(reply: &str, agents: &[String]) -> Result<(String, Vec<Part>), String> {
    let Some(value) = json_in(reply) else {
        return Err("The planner's answer had no plan in it.".into());
    };
    let summary = text_of(value.get("summary"));
    let raw = value
        .get("tasks")
        .or_else(|| value.get("parts"))
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    let mut parts: Vec<Part> = Vec::new();
    // What each raw id was turned into, for dependencies given as ids or 1-based numbers.
    let mut ids: Vec<String> = Vec::new();
    for (i, item) in raw.iter().take(MAX_PARTS).enumerate() {
        let mut prompt = text_of(item.get("prompt").or_else(|| item.get("description")));
        let mut title = text_of(item.get("title").or_else(|| item.get("name")));
        if title.is_empty() {
            title = first_line(&prompt).chars().take(60).collect();
        }
        if prompt.is_empty() {
            prompt = title.clone();
        }
        if title.is_empty() {
            continue;
        }
        let raw_id = text_of(item.get("id").or_else(|| item.get("key")));
        let mut key: String = raw_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        if key.trim_matches('-').is_empty() {
            key = format!("part-{}", i + 1);
        }
        while parts.iter().any(|p| p.key == key) {
            key.push('2');
        }
        // Earlier parts only, by id or position.
        let deps = list_of(item.get("depends_on").or_else(|| item.get("deps")))
            .into_iter()
            .filter_map(|d| {
                ids.iter()
                    .position(|r| *r == d)
                    .or_else(|| {
                        d.parse::<usize>()
                            .ok()
                            .filter(|n| *n >= 1 && *n <= parts.len())
                            .map(|n| n - 1)
                    })
                    .map(|at| parts[at].key.clone())
            })
            .fold(Vec::new(), |mut acc, k| {
                if !acc.contains(&k) {
                    acc.push(k);
                }
                acc
            });
        let wanted = text_of(item.get("agent")).to_ascii_lowercase();
        let agent = if agents.contains(&wanted) {
            wanted
        } else if agents.is_empty() {
            "claude".to_string()
        } else {
            agents[parts.len() % agents.len()].clone()
        };
        ids.push(if raw_id.is_empty() {
            (i + 1).to_string()
        } else {
            raw_id
        });
        parts.push(Part {
            key,
            title: title.chars().take(80).collect(),
            prompt,
            deps,
            agent,
            files: list_of(item.get("files")),
            ..Default::default()
        });
    }
    if parts.is_empty() {
        return Err("The planner's plan had no parts.".into());
    }
    Ok((summary, parts))
}

/// The first prompt of a part's session. `names` gives an agent's display name.
pub fn part_prompt(mission: &Mission, index: usize, names: impl Fn(&str) -> String) -> String {
    let part = &mission.parts[index];
    let mut out = format!(
        "You are one of several agents working on a mission in this repository, each on its own part in its own worktree. When your part's checks pass, Runo commits it and merges it into the branch `{}`.\n\n## The mission\n{}\n\n## Your part: {}\n{}\n",
        mission.branch,
        mission.goal.trim(),
        part.title,
        part.prompt.trim()
    );
    if !part.files.is_empty() {
        out.push_str(&format!(
            "\nFiles it is expected to touch: {}\n",
            part.files.join(", ")
        ));
    }
    let others: Vec<String> = mission
        .parts
        .iter()
        .enumerate()
        .filter(|(i, p)| *i != index && p.state != PartState::Skipped)
        .map(|(_, p)| {
            let status = match p.state {
                PartState::Done => "already merged into your branch".to_string(),
                PartState::Waiting => "starts later".to_string(),
                _ => format!("in progress by {}", names(&p.agent)),
            };
            format!("- {} ({status})", p.title)
        })
        .collect();
    if !others.is_empty() {
        out.push_str("\n## The other parts, which are not yours to do\n");
        out.push_str(&others.join("\n"));
        out.push('\n');
    }
    out.push_str("\n## Rules\n- Do only your part. If it needs a change in another part's area, make the smallest change that works and say so in your last message.\n- Leave your work in this worktree; committing is fine. Don't push, and don't open a pull request.\n- Run the project's tests or build before you finish, when you can.\n- End with two or three sentences on what you changed.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(key: &str, deps: &[&str]) -> Part {
        Part {
            key: key.into(),
            title: key.to_uppercase(),
            prompt: format!("Do {key}"),
            deps: deps.iter().map(|d| d.to_string()).collect(),
            agent: "claude".into(),
            ..Default::default()
        }
    }

    fn mission(parts: Vec<Part>, parallel: u32) -> Mission {
        Mission {
            id: 1,
            goal: "Add dark mode".into(),
            stage: Stage::Running,
            agents: vec!["claude".into(), "codex".into()],
            parallel,
            branch: "mission/add-dark-mode".into(),
            parts,
            ..Default::default()
        }
    }

    #[test]
    fn branches_come_from_the_goal() {
        assert_eq!(
            branch_for("Add dark mode to the Settings page, please!"),
            "mission/add-dark-mode-to-the"
        );
        assert_eq!(branch_for("¿¡"), "mission/work");
    }

    #[test]
    fn parts_start_when_their_dependencies_are_in_and_there_is_room() {
        let mut m = mission(vec![part("a", &[]), part("b", &[]), part("c", &["a"])], 2);
        assert_eq!(m.startable(), vec![0, 1]);
        m.parts[0].state = PartState::Working;
        m.parts[1].state = PartState::Working;
        assert!(m.startable().is_empty());
        // A part waiting to merge leaves its agent free, but c still waits on a.
        m.parts[0].state = PartState::Proved;
        assert!(m.startable().is_empty());
        m.parts[0].state = PartState::Done;
        assert_eq!(m.startable(), vec![2]);
        // Skipping counts as settled; pausing starts nothing.
        m.parts[0].state = PartState::Skipped;
        assert_eq!(m.startable(), vec![2]);
        m.stage = Stage::Paused;
        assert!(m.startable().is_empty());
    }

    #[test]
    fn the_log_keeps_the_newest_lines() {
        let mut m = Mission::default();
        for i in 0..LOG_LIMIT + 5 {
            m.record(i as u64, Mark::Info, format!("line {i}"));
        }
        assert_eq!(m.log.len(), LOG_LIMIT);
        assert_eq!(m.log[0].text, "line 5");
        assert_eq!(m.log.last().map(|e| e.at), Some((LOG_LIMIT + 4) as u64));
    }

    #[test]
    fn parts_fall_into_waves_by_what_they_wait_on() {
        let m = mission(
            vec![
                part("a", &[]),
                part("b", &[]),
                part("c", &["a"]),
                part("d", &["c", "b"]),
            ],
            2,
        );
        assert_eq!(m.waves(), vec![vec![0, 1], vec![2], vec![3]]);
        assert_eq!(PartState::Reviewing.step(), Some(2));
        assert_eq!(PartState::Stuck.step(), None);
    }

    #[test]
    fn one_merge_at_a_time_in_plan_order() {
        let mut m = mission(vec![part("a", &[]), part("b", &[]), part("c", &[])], 3);
        m.parts[1].state = PartState::Proved;
        m.parts[2].state = PartState::Proved;
        assert_eq!(m.next_merge(), Some(1));
        m.parts[1].state = PartState::Merging;
        assert_eq!(m.next_merge(), None);
        m.parts[1].state = PartState::Done;
        assert_eq!(m.next_merge(), Some(2));
    }

    #[test]
    fn a_stuck_part_holds_only_what_waits_on_it() {
        let mut m = mission(vec![part("a", &[]), part("b", &["a"]), part("c", &[])], 2);
        m.parts[0].state = PartState::Stuck;
        assert_eq!(m.startable(), vec![2]);
        assert!(!m.waiting_on_you());
        m.parts[2].state = PartState::Done;
        assert!(m.waiting_on_you());
        assert_eq!(m.progress(), (1, 3));
        assert!(!m.all_in());
        m.remove_part(0);
        assert!(m.parts[0].deps.is_empty());
    }

    #[test]
    fn plans_are_read_from_messy_answers() {
        let reply = "I looked around.\n```json\n{\"summary\": \"Theme first, then pages\", \"tasks\": [\
            {\"id\": \"theme\", \"title\": \"Theme tokens\", \"prompt\": \"Add tokens\", \"files\": [\"src/theme.rs\"], \"agent\": \"codex\"},\
            {\"id\": \"pages\", \"title\": \"Pages\", \"prompt\": \"Use tokens\", \"depends_on\": [\"theme\", \"pages\", \"later\"], \"agent\": \"gemini\"},\
            {\"id\": \"theme\", \"prompt\": \"Write the docs\\nmore\", \"depends_on\": [2]}\
            ]}\n```\nDone.";
        let agents = vec!["claude".to_string(), "codex".to_string()];
        let (summary, parts) = parse_plan(reply, &agents).unwrap();
        assert_eq!(summary, "Theme first, then pages");
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].agent, "codex");
        assert_eq!(parts[0].files, vec!["src/theme.rs"]);
        // Only earlier parts count as dependencies; unknown agents get one from the pool.
        assert_eq!(parts[1].deps, vec!["theme"]);
        assert_eq!(parts[1].agent, "codex");
        // A repeated id gets its own key; a number is a position; a title comes from the prompt.
        assert_eq!(parts[2].key, "theme2");
        assert_eq!(parts[2].deps, vec!["pages"]);
        assert_eq!(parts[2].title, "Write the docs");
    }

    #[test]
    fn a_plan_without_json_or_parts_is_an_error() {
        let agents = vec!["claude".to_string()];
        assert!(parse_plan("I can't do that.", &agents).is_err());
        assert!(parse_plan("{\"summary\": \"x\", \"tasks\": []}", &agents).is_err());
        // Bare JSON with no fence is fine.
        let (_, parts) = parse_plan(
            "{\"tasks\": [{\"title\": \"One\", \"prompt\": \"Do it\"}]}",
            &agents,
        )
        .unwrap();
        assert_eq!(parts[0].key, "part-1");
    }

    #[test]
    fn a_part_knows_the_goal_and_what_the_others_do() {
        let mut m = mission(vec![part("a", &[]), part("b", &["a"]), part("c", &[])], 2);
        m.parts[0].state = PartState::Done;
        m.parts[2].state = PartState::Working;
        m.parts[2].agent = "codex".into();
        m.parts[1].files = vec!["src/b.rs".into()];
        let prompt = part_prompt(&m, 1, |id| {
            if id == "codex" {
                "Codex"
            } else {
                "Claude Code"
            }
            .to_string()
        });
        assert!(prompt.contains("Add dark mode"));
        assert!(prompt.contains("## Your part: B\nDo b"));
        assert!(prompt.contains("src/b.rs"));
        assert!(prompt.contains("- A (already merged into your branch)"));
        assert!(prompt.contains("- C (in progress by Codex)"));
        assert!(prompt.contains("mission/add-dark-mode"));
    }

    #[test]
    fn plan_again_carries_the_feedback() {
        let prompt = plan_prompt(
            "Add dark mode",
            &[("claude", "Claude Code"), ("codex", "Codex")],
            3,
            "Merge the two UI parts",
            &[part("a", &[])],
        );
        assert!(prompt.contains("- `codex`: Codex"));
        assert!(prompt.contains("(3 at a time)"));
        assert!(prompt.contains("Merge the two UI parts"));
        assert!(prompt.contains("- A (claude): Do a"));
    }
}
