//! The watcher for CI after a push to the default branch, a merge or a tag push. The
//! pull request watcher (`prwatch.rs`) stops once a pull request is merged, but a thread
//! that merged or pushed to main, or pushed a release tag, often has more to do once the
//! runs on that commit or tag finish ("I'll push the release once main's CI is green").
//! Such a thread ends its turn; Runo reads the GitHub Actions runs for what it pushed,
//! on the pull request watcher's pace, and wakes it when they have all finished: "Main CI
//! passed for 1a2b3c4" or "Release workflow failed: build (macos)".
//!
//! What to watch comes from the turn's own commands (`detect`): `git push` to the default
//! branch or of a tag, `gh pr merge`, `gh release create`, or a line the thread writes,
//! such as `runo-watch: main`, `runo-watch: tag v1.2.0` or `runo-watch: run 123`.
//! A watched thread isn't running, so it holds no slot. What the watcher decides is in
//! `decide` and `read_failed`, pure functions over the saved `RunWatch` and one read.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::*;
use crate::prwatch::{interval, MAX_FAILED_READS, POLL};

/// Reads in a row that find no runs at all before the watcher gives up: the push started
/// no workflow, or it was for something GitHub doesn't run Actions on.
pub const MAX_EMPTY_READS: u32 = 6;
/// How long one watch may last, however slow the runs.
pub const MAX_WATCH: Duration = Duration::from_secs(3 * 60 * 60);
/// Watches one turn can start.
const MAX_TARGETS: usize = 3;

/// What a thread waits on.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// The runs on the latest commit of a branch, the default branch in practice. Empty
    /// for the folder's current branch, watched only when it turns out to be the default.
    Branch(String),
    /// The runs for a pushed tag, such as a Release workflow. Empty for the newest tag.
    Tag(String),
    /// One run, by its id.
    Run(u64),
}

/// A thread's watched runs, saved with the thread.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct RunWatch {
    pub target: Target,
    /// For a branch: the commit whose runs are watched, found on the first read.
    #[serde(default)]
    pub sha: String,
    /// When the watch started, in seconds since the epoch.
    #[serde(default)]
    pub since: u64,
    #[serde(default)]
    pub failed_reads: u32,
    #[serde(default)]
    pub empty_reads: u32,
    /// A run to open from the card.
    #[serde(default)]
    pub url: String,
}

impl RunWatch {
    pub fn new(target: Target, now: u64) -> RunWatch {
        RunWatch {
            target,
            sha: String::new(),
            since: now,
            failed_reads: 0,
            empty_reads: 0,
            url: String::new(),
        }
    }
}

/// One GitHub Actions run.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Run {
    pub id: u64,
    pub workflow: String,
    /// queued, in_progress, completed and the like.
    pub status: String,
    /// success, failure, cancelled, skipped and the like, once completed.
    pub conclusion: String,
    pub url: String,
    /// The jobs that failed, read only for a completed run that failed.
    pub failed_jobs: Vec<String>,
}

impl Run {
    fn done(&self) -> bool {
        self.status == "completed"
    }

    fn failed(&self) -> bool {
        self.done() && !matches!(self.conclusion.as_str(), "success" | "skipped" | "neutral")
    }
}

/// One read: the target as resolved (a branch's name, a tag's name), the commit for a
/// branch, and the runs found.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Read {
    pub target: Option<Target>,
    pub sha: String,
    pub runs: Vec<Run>,
}

/// What to do after one read.
#[derive(Clone, PartialEq, Debug)]
pub enum Step {
    Wait,
    /// Every run finished and passed: wake the thread with this news.
    Passed(String),
    /// Every run finished and some failed: wake the thread with this news.
    Failed(String),
    /// Stop watching, for this reason, without a wake.
    Stop(String),
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// "Main CI" for a branch named main.
fn branch_ci(branch: &str) -> String {
    let mut c = branch.chars();
    match c.next() {
        Some(f) => format!("{}{} CI", f.to_uppercase(), c.as_str()),
        None => "CI".into(),
    }
}

/// What the card says while the runs are watched.
pub fn label(w: &RunWatch) -> String {
    match &w.target {
        Target::Branch(b) if b.is_empty() => "Watching CI".into(),
        Target::Branch(b) => format!("Watching {b} CI"),
        Target::Tag(_) => "Watching release".into(),
        Target::Run(n) => format!("Watching run {n}"),
    }
}

/// What failed, run by run: the failed jobs, else the workflow.
fn failures(runs: &[Run]) -> String {
    let failed: Vec<&Run> = runs.iter().filter(|r| r.failed()).collect();
    let many = failed.len() > 1;
    failed
        .iter()
        .map(|r| {
            let what = if r.failed_jobs.is_empty() {
                r.workflow.clone()
            } else {
                r.failed_jobs.join(", ")
            };
            if many && !r.failed_jobs.is_empty() {
                format!("{}: {what}", r.workflow)
            } else {
                what
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The news for runs that have all finished: "Main CI passed for 1a2b3c4", "Release
/// workflow failed: build (macos)".
pub fn news(w: &RunWatch, runs: &[Run]) -> Step {
    let failed = runs.iter().any(Run::failed);
    let what = match &w.target {
        Target::Branch(b) => {
            let ci = branch_ci(b);
            let sha = short(&w.sha);
            return if failed {
                Step::Failed(format!("{ci} failed for {sha}: {}", failures(runs)))
            } else {
                Step::Passed(format!("{ci} passed for {sha}"))
            };
        }
        Target::Tag(tag) => {
            // The release workflow by name when there is one, so a tag's other runs read
            // plainly.
            let release = runs
                .iter()
                .any(|r| r.workflow.to_lowercase().contains("release"));
            let name = if release { "Release workflow" } else { "CI" };
            if failed {
                return Step::Failed(format!("{name} failed: {}", failures(runs)));
            }
            format!("{name} passed for {tag}")
        }
        Target::Run(n) => {
            let name = runs.first().map(|r| r.workflow.as_str()).unwrap_or("");
            let name = if name.is_empty() {
                format!("Run {n}")
            } else {
                format!("Run {n} ({name})")
            };
            if failed {
                return Step::Failed(format!("{name} failed: {}", failures(runs)));
            }
            format!("{name} passed")
        }
    };
    Step::Passed(what)
}

/// Decide what one successful read means. `busy` is true while the thread is running,
/// queued or waiting for a slot: finished runs wait for it to be free. `now` is in
/// seconds since the epoch.
pub fn decide(w: &mut RunWatch, read: &Read, busy: bool, now: u64) -> Step {
    w.failed_reads = 0;
    if let Some(t) = &read.target {
        w.target = t.clone();
    }
    if w.sha.is_empty() {
        w.sha = read.sha.clone();
    }
    if now.saturating_sub(w.since) > MAX_WATCH.as_secs() {
        return Step::Stop(format!(
            "{} hours on {}",
            MAX_WATCH.as_secs() / 3600,
            label(w).to_lowercase()
        ));
    }
    if read.runs.is_empty() {
        w.empty_reads += 1;
        return if w.empty_reads >= MAX_EMPTY_READS {
            Step::Stop(format!("no workflow runs for {}", what(w)))
        } else {
            Step::Wait
        };
    }
    w.empty_reads = 0;
    if let Some(r) = read.runs.iter().find(|r| r.failed()).or(read.runs.first()) {
        w.url = r.url.clone();
    }
    if busy || !read.runs.iter().all(Run::done) {
        return Step::Wait;
    }
    news(w, &read.runs)
}

/// What is watched, for the log: "main at 1a2b3c4", "tag v1.2.0", "run 123".
fn what(w: &RunWatch) -> String {
    match &w.target {
        Target::Branch(b) => format!("{b} at {}", short(&w.sha)),
        Target::Tag(t) => format!("tag {t}"),
        Target::Run(n) => format!("run {n}"),
    }
}

/// Count a read that failed: gh or git couldn't run or answer.
pub fn read_failed(w: &mut RunWatch) -> Option<Step> {
    w.failed_reads += 1;
    (w.failed_reads >= MAX_FAILED_READS).then(|| {
        Step::Stop(format!(
            "{MAX_FAILED_READS} reads of {} failed in a row",
            what(w)
        ))
    })
}

/// The branches that count as the default for `base`.
fn is_default(name: &str, base: &str) -> bool {
    !name.is_empty() && (name == base || name == "main" || name == "master")
}

fn looks_like_tag(name: &str, tags: &[String]) -> bool {
    tags.iter().any(|t| t == name)
        || name
            .strip_prefix('v')
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// The targets one shell command names. `tags` are the tags the turn made so far, which
/// a later `git push --tags` pushes; `branch` is the folder's branch ("local folder" or
/// empty when it isn't known).
fn from_command(command: &str, branch: &str, base: &str, tags: &mut Vec<String>) -> Vec<Target> {
    let mut out = Vec::new();
    // A label cut short ends in "…": its last word is only part of one.
    let cut = command.ends_with('…');
    let command = command.trim_end_matches('…');
    let segments: Vec<&str> = command
        .split(['\n', ';', '|', '&'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let last = segments.len().saturating_sub(1);
    for (i, segment) in segments.iter().enumerate() {
        let mut words: Vec<&str> = segment
            .split_whitespace()
            // Quotes around words matter little here.
            .map(|w| w.trim_matches(|c| c == '"' || c == '\''))
            .collect();
        if cut && i == last {
            words.pop();
        }
        // Variables set for the command, such as GIT_TERMINAL_PROMPT=0.
        let start = words
            .iter()
            .position(|w| !w.contains('=') || w.starts_with('-'))
            .unwrap_or(words.len());
        let words = &words[start..];
        match words.first().copied() {
            Some("git") => {
                // Options before the subcommand, some with a value.
                let mut rest = &words[1..];
                while let Some(w) = rest.first() {
                    if matches!(*w, "-C" | "-c" | "--git-dir" | "--work-tree") {
                        rest = rest.get(2..).unwrap_or(&[]);
                    } else if w.starts_with('-') {
                        rest = &rest[1..];
                    } else {
                        break;
                    }
                }
                match rest.first().copied() {
                    Some("tag") => {
                        let args = &rest[1..];
                        if args.iter().any(|a| {
                            matches!(*a, "-d" | "--delete" | "-l" | "--list" | "-v" | "--verify")
                        }) {
                            continue;
                        }
                        let mut skip = false;
                        for a in args {
                            if skip {
                                skip = false;
                            } else if matches!(*a, "-m" | "-F" | "-u" | "--message" | "--file") {
                                skip = true;
                            } else if !a.starts_with('-') {
                                tags.push(a.to_string());
                                break;
                            }
                        }
                    }
                    Some("push") => {
                        let args = &rest[1..];
                        if args
                            .iter()
                            .any(|a| matches!(*a, "-d" | "--delete" | "--dry-run" | "-n"))
                        {
                            continue;
                        }
                        let all_tags = args
                            .iter()
                            .any(|a| matches!(*a, "--tags" | "--follow-tags"));
                        let mut positional = Vec::new();
                        let mut skip = false;
                        for a in args {
                            if skip {
                                skip = false;
                            } else if matches!(*a, "-o" | "--push-option" | "--repo") {
                                skip = true;
                            } else if !a.starts_with('-') {
                                positional.push(*a);
                            }
                        }
                        let refspecs = positional.get(1..).unwrap_or(&[]);
                        for spec in refspecs {
                            let spec = spec.trim_start_matches('+');
                            // ":name" deletes it.
                            if spec.starts_with(':') {
                                continue;
                            }
                            let dst = spec.rsplit(':').next().unwrap_or(spec);
                            if let Some(tag) = dst.strip_prefix("refs/tags/") {
                                out.push(Target::Tag(tag.to_string()));
                                continue;
                            }
                            let dst = dst.strip_prefix("refs/heads/").unwrap_or(dst);
                            let dst = if dst == "HEAD" { branch } else { dst };
                            if is_default(dst, base) {
                                out.push(Target::Branch(dst.to_string()));
                            } else if looks_like_tag(dst, tags) {
                                out.push(Target::Tag(dst.to_string()));
                            }
                        }
                        if all_tags {
                            out.push(Target::Tag(tags.last().cloned().unwrap_or_default()));
                        }
                        // A plain `git push` pushes the folder's branch.
                        if refspecs.is_empty() && !all_tags {
                            if is_default(branch, base) {
                                out.push(Target::Branch(branch.to_string()));
                            } else if branch.is_empty() || branch == "local folder" {
                                out.push(Target::Branch(String::new()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("gh") => match (words.get(1).copied(), words.get(2).copied()) {
                (Some("pr"), Some("merge")) => {
                    // Auto-merge happens later, when the PR watcher still has it.
                    if !words
                        .iter()
                        .any(|w| w.starts_with("--auto") || *w == "--disable-auto")
                    {
                        let base = if base.is_empty() { "main" } else { base };
                        out.push(Target::Branch(base.to_string()));
                    }
                }
                (Some("release"), Some("create")) => {
                    if let Some(tag) = words[3..].iter().find(|w| !w.starts_with('-')) {
                        out.push(Target::Tag(tag.to_string()));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

/// A target the thread named in its reply, on a line of its own: `runo-watch: main`,
/// `runo-watch: tag v1.2.0`, `runo-watch: run 123`, or plainly "watch run 123" or
/// "wait for main CI".
fn from_line(line: &str, base: &str) -> Option<Target> {
    let line = line
        .trim()
        .trim_start_matches(['-', '*', '>'])
        .trim()
        .trim_matches('`')
        .trim()
        .trim_end_matches('.');
    let lower = line.to_lowercase();
    let main = || Target::Branch(if base.is_empty() { "main" } else { base }.to_string());
    if let Some(rest) = lower.strip_prefix("runo-watch:") {
        let words: Vec<&str> = rest.split_whitespace().collect();
        // Names keep their case from the original line.
        let original: Vec<&str> = line["runo-watch:".len()..].split_whitespace().collect();
        return match words.as_slice() {
            ["run", n] => n.trim_start_matches('#').parse().ok().map(Target::Run),
            ["tag" | "release", _] => Some(Target::Tag(original[1].to_string())),
            ["tag" | "release"] => Some(Target::Tag(String::new())),
            ["main"] | ["main", "ci"] => Some(main()),
            [_] => Some(Target::Branch(original[0].to_string())),
            _ => None,
        };
    }
    if let Some(n) = lower.strip_prefix("watch run ") {
        return n
            .trim()
            .trim_start_matches('#')
            .parse()
            .ok()
            .map(Target::Run);
    }
    (lower == "wait for main ci").then(main)
}

/// What a turn asks to be watched, from its shell commands, in order, and its reply text.
/// `branch` is the folder's branch, `base` the default branch ("main" when empty).
pub fn detect(commands: &[String], text: &str, branch: &str, base: &str) -> Vec<Target> {
    let base = if base.is_empty() { "main" } else { base };
    let mut tags = Vec::new();
    let mut found: Vec<Target> = commands
        .iter()
        .flat_map(|c| from_command(c, branch, base, &mut tags))
        .collect();
    found.extend(text.lines().filter_map(|l| from_line(l, base)));
    // A named branch or tag says more than an unnamed one.
    let named_branch = found
        .iter()
        .any(|t| matches!(t, Target::Branch(b) if !b.is_empty()));
    let named_tag = found
        .iter()
        .any(|t| matches!(t, Target::Tag(t) if !t.is_empty()));
    let mut out: Vec<Target> = Vec::new();
    for t in found {
        let vague = match &t {
            Target::Branch(b) => b.is_empty() && named_branch,
            Target::Tag(n) => n.is_empty() && named_tag,
            Target::Run(_) => false,
        };
        if !vague && !out.contains(&t) {
            out.push(t);
        }
    }
    out.truncate(MAX_TARGETS);
    out
}

/// The card's chip text and link for a thread's first watch.
pub fn chip(watches: &[RunWatch]) -> Option<(String, String)> {
    let w = watches.first()?;
    let more = watches.len() - 1;
    let text = if more > 0 {
        format!("{} +{more}", label(w))
    } else {
        label(w)
    };
    Some((text, w.url.clone()))
}

/// Said after the news when the thread is woken.
const CARRY_ON: &str = "Carry on with what you were waiting for. If you have to wait for CI again, end your turn instead of running `gh run watch` or a sleep loop: Runo watches the runs and wakes you when they finish.";

pub fn wake_prompt(news: &str, url: &str) -> String {
    let link = if url.is_empty() {
        String::new()
    } else {
        format!(" ({url})")
    };
    format!("{news}{link}.\n\n{CARRY_ON}")
}

// ---- Reading GitHub ----

fn run_in(dir: &str, program: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

const RUN_FIELDS: &str = "databaseId,workflowName,status,conclusion,url";

fn parse_run(v: &serde_json::Value) -> Run {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    Run {
        id: v["databaseId"].as_u64().unwrap_or(0),
        workflow: s("workflowName"),
        status: s("status").to_lowercase(),
        conclusion: s("conclusion").to_lowercase(),
        url: s("url"),
        failed_jobs: Vec::new(),
    }
}

/// The runs in `gh run list --json` output.
pub fn parse_runs(json: &str) -> Vec<Run> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.as_array().map(|a| a.iter().map(parse_run).collect()))
        .unwrap_or_default()
}

/// The failed jobs in `gh run view --json jobs` output.
pub fn parse_failed_jobs(json: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    v["jobs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|j| {
            matches!(
                j["conclusion"].as_str().unwrap_or(""),
                "failure" | "timed_out" | "cancelled" | "startup_failure"
            )
        })
        .filter_map(|j| j["name"].as_str().map(String::from))
        .collect()
}

/// One read of what `w` watches, run off the UI thread in the thread's folder.
fn read(dir: &str, w: &RunWatch) -> Result<Read, String> {
    let mut read = Read::default();
    match &w.target {
        Target::Branch(b) => {
            let mut branch = b.clone();
            if branch.is_empty() {
                // The folder's branch, watched only when it is the default one.
                let current = run_in(dir, "git", &["rev-parse", "--abbrev-ref", "HEAD"])?;
                let current = current.trim().to_string();
                let default = run_in(
                    dir,
                    "git",
                    &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
                )
                .map(|d| d.trim().trim_start_matches("origin/").to_string())
                .unwrap_or_else(|_| "main".into());
                if current != default {
                    return Ok(read);
                }
                branch = current;
                read.target = Some(Target::Branch(branch.clone()));
            }
            read.sha = if w.sha.is_empty() {
                let refs = format!("refs/heads/{branch}");
                let out = run_in(dir, "git", &["ls-remote", "origin", &refs])?;
                out.split_whitespace().next().unwrap_or("").to_string()
            } else {
                w.sha.clone()
            };
            if read.sha.is_empty() {
                return Err(format!("origin has no branch {branch}"));
            }
            let out = run_in(
                dir,
                "gh",
                &[
                    "run", "list", "--commit", &read.sha, "--limit", "30", "--json", RUN_FIELDS,
                ],
            )?;
            read.runs = parse_runs(&out);
        }
        Target::Tag(t) => {
            let mut tag = t.clone();
            if tag.is_empty() {
                let out = run_in(
                    dir,
                    "git",
                    &[
                        "for-each-ref",
                        "--sort=-creatordate",
                        "--count=1",
                        "--format=%(refname:short)",
                        "refs/tags",
                    ],
                )?;
                tag = out.trim().to_string();
                if tag.is_empty() {
                    return Err("no tags".into());
                }
                read.target = Some(Target::Tag(tag.clone()));
            }
            let out = run_in(
                dir,
                "gh",
                &[
                    "run", "list", "--branch", &tag, "--limit", "20", "--json", RUN_FIELDS,
                ],
            )?;
            read.runs = parse_runs(&out);
        }
        Target::Run(n) => {
            let out = run_in(
                dir,
                "gh",
                &["run", "view", &n.to_string(), "--json", RUN_FIELDS],
            )?;
            let v: serde_json::Value =
                serde_json::from_str(&out).map_err(|e| format!("gh run view: {e}"))?;
            read.runs = vec![parse_run(&v)];
        }
    }
    // Which jobs failed, once everything has finished.
    if read.runs.iter().all(Run::done) {
        for r in read.runs.iter_mut().filter(|r| r.failed()) {
            if let Ok(out) = run_in(
                dir,
                "gh",
                &["run", "view", &r.id.to_string(), "--json", "jobs"],
            ) {
                r.failed_jobs = parse_failed_jobs(&out);
            }
        }
    }
    Ok(read)
}

/// What the app keeps for the watcher while it runs; the watches are saved with their
/// threads.
#[derive(Default)]
pub(crate) struct Polls {
    /// When each watched thread's runs are read next.
    next: HashMap<u64, Instant>,
    /// Threads with a read under way.
    loading: HashSet<u64>,
}

/// The shell commands of the latest turn in `messages`, oldest first, and its reply text.
pub(crate) fn turn_commands(messages: &[Message]) -> (Vec<String>, String) {
    use crate::harness::event::ToolKind;
    let turn = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .map_or(messages, |i| &messages[i + 1..]);
    let replies = turn.iter().filter(|m| m.role == Role::Agent);
    let commands = replies
        .clone()
        .flat_map(|m| &m.steps)
        .filter(|s| s.kind == ToolKind::Shell)
        .filter_map(|s| s.label.strip_prefix("Ran ").map(String::from))
        .collect();
    let text = replies
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (commands, text)
}

impl RunoApp {
    /// A thread's turn ended well: watch the runs for what it pushed or merged to the
    /// default branch, or the tag it pushed, or what it asked to be watched.
    pub(crate) fn run_watch_turn_ended(&mut self, id: u64) {
        let Some(s) = self.state.session(id) else {
            return;
        };
        if s.crew.is_none() {
            return;
        }
        let (commands, text) = turn_commands(&s.messages);
        let targets = detect(&commands, &text, &s.branch, &s.base);
        if targets.is_empty() {
            return;
        }
        let now = crate::workspace::unix_now();
        let mut started = Vec::new();
        if let Some(info) = self.state.session_mut(id).and_then(|s| s.crew.as_mut()) {
            for t in targets {
                // A target already watched keeps its watch, and its commit.
                if !info.runs.iter().any(|w| w.target == t) {
                    started.push(label(&RunWatch::new(t.clone(), now)));
                    info.runs.push(RunWatch::new(t, now));
                }
            }
        }
        for what in started {
            self.state.log(id, what);
        }
        // Read soon, once the push has reached GitHub and its runs have started.
        self.run_watch
            .next
            .insert(id, Instant::now() + Duration::from_secs(30));
        self.save();
    }

    /// Read the watched runs that are due, on the pull request watcher's pace.
    pub(crate) fn run_watch_tick(&mut self) {
        let now = Instant::now();
        let due: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| !s.archived)
            .filter(|s| {
                s.crew
                    .as_ref()
                    .is_some_and(|i| !i.runs.is_empty() && !i.resolved)
            })
            .map(|s| s.id)
            .collect();
        self.run_watch.next.retain(|id, _| due.contains(id));
        for (n, id) in due.into_iter().enumerate() {
            let at = *self
                .run_watch
                .next
                .entry(id)
                .or_insert(now + Duration::from_secs(15 + 3 * n as u64));
            if at <= now && !self.run_watch.loading.contains(&id) {
                self.run_watch.next.insert(id, now + POLL);
                self.load_runs(id);
            }
        }
    }

    /// Read each of a thread's watches in the background.
    fn load_runs(&mut self, id: u64) {
        let Some(s) = self.state.session(id) else {
            return;
        };
        let Some(watches) = s.crew.as_ref().map(|i| i.runs.clone()) else {
            return;
        };
        // A merged thread's worktree may be gone; its project's folder is the same repo.
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == s.project_id)
            .map(|p| p.path.clone());
        let Some(dir) = std::iter::once(s.dir.clone())
            .chain(project)
            .find(|d| std::path::Path::new(d).is_dir())
        else {
            if let Some(info) = self.state.session_mut(id).and_then(|s| s.crew.as_mut()) {
                info.runs.clear();
            }
            self.state.log(
                id,
                "Stopped watching runs: the thread's folder is gone".to_string(),
            );
            return;
        };
        self.run_watch.loading.insert(id);
        self.bg(
            move || {
                watches
                    .iter()
                    .map(|w| (w.target.clone(), read(&dir, w)))
                    .collect::<Vec<_>>()
            },
            move |this, reads, cx| {
                this.run_watch.loading.remove(&id);
                this.runs_read(id, reads, cx);
            },
        );
    }

    /// The reads of a thread's watches came back.
    fn runs_read(
        &mut self,
        id: u64,
        reads: Vec<(Target, Result<Read, String>)>,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.state.session(id) else {
            return;
        };
        let busy = self.is_running(s)
            || !s.queue.is_empty()
            || self.waiting_for_slot(id)
            || self.preparing.contains_key(&id)
            || self.git_busy.contains_key(&id);
        let Some(mut watches) = s.crew.as_ref().map(|i| i.runs.clone()) else {
            return;
        };
        let before = watches.clone();
        let now = crate::workspace::unix_now();
        let mut wakes = Vec::new();
        let mut logs = Vec::new();
        watches.retain_mut(|w| {
            let Some((_, read)) = reads.iter().find(|(t, _)| *t == w.target) else {
                return true;
            };
            let step = match read {
                // The folder's branch turned out not to be the default: nothing to watch.
                Ok(r)
                    if matches!(&w.target, Target::Branch(b) if b.is_empty())
                        && r.target.is_none() =>
                {
                    Step::Stop("the folder isn't on the default branch".into())
                }
                Ok(r) => decide(w, r, busy, now),
                Err(_) => read_failed(w).unwrap_or(Step::Wait),
            };
            match step {
                Step::Wait => true,
                Step::Passed(news) | Step::Failed(news) => {
                    logs.push(news.clone());
                    wakes.push(wake_prompt(&news, &w.url));
                    false
                }
                Step::Stop(why) => {
                    logs.push(format!("Stopped watching runs: {why}"));
                    false
                }
            }
        });
        let backoff = watches.iter().map(|w| w.failed_reads).max().unwrap_or(0);
        self.run_watch
            .next
            .insert(id, Instant::now() + interval(backoff));
        if watches == before {
            return;
        }
        if let Some(info) = self.state.session_mut(id).and_then(|s| s.crew.as_mut()) {
            info.runs = watches;
        }
        for line in logs {
            self.state.log(id, line);
        }
        if !wakes.is_empty() {
            let news = wakes.join("\n\n");
            self.watch_news(
                id,
                format!("was woken: {}", news.lines().next().unwrap_or("")),
            );
            self.send_or_queue(id, news, cx);
        }
        self.save();
    }

    /// The card's chip for a thread whose runs are watched: "Watching main CI",
    /// "Watching release". Clicking it opens a run once one is known.
    pub(crate) fn run_chip(
        &self,
        s: &Session,
        key: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.is_running(s) {
            return None;
        }
        let (text, url) = chip(&s.crew.as_ref()?.runs)?;
        let tone = c_blue();
        Some(
            div()
                .id(SharedString::from(format!("runs-{key}-{}", s.id)))
                .flex_none()
                .px(px(6.))
                .rounded_full()
                .bg(rgba((tone << 8) | 0x22))
                .text_size(px(10.5))
                .text_color(rgb(tone))
                .whitespace_nowrap()
                .when(!url.is_empty(), |el| {
                    el.cursor_pointer()
                        .hover(|el| el.underline())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.open_url(&url);
                        }))
                })
                .child(text)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    fn cmds(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    fn main() -> Target {
        Target::Branch("main".into())
    }

    #[test]
    fn a_push_to_main_is_watched() {
        let found = |c: &str| detect(&cmds(&[c]), "", "runo/x", "main");
        assert_eq!(found("git push origin main"), [main()]);
        assert_eq!(found("cd /repo && git push origin HEAD:main"), [main()]);
        assert_eq!(
            found("git -C /repo push -u origin refs/heads/main"),
            [main()]
        );
        assert_eq!(
            found("GIT_TERMINAL_PROMPT=0 git push origin +main"),
            [main()]
        );
        // The thread's own branch is the pull request watcher's.
        assert!(found("git push -u origin runo/x").is_empty());
        assert!(found("git push --force-with-lease").is_empty());
        // Deleting, or a dry run, isn't a push.
        assert!(found("git push origin --delete main").is_empty());
        assert!(found("git push -n origin main").is_empty());
        // A plain push from the project folder: its branch is found when read.
        assert_eq!(
            detect(&cmds(&["git push"]), "", "local folder", "main"),
            [Target::Branch(String::new())]
        );
        // A plain push on main itself.
        assert_eq!(detect(&cmds(&["git push"]), "", "main", "main"), [main()]);
    }

    #[test]
    fn a_merge_is_watched_on_the_base() {
        let found = |c: &str| detect(&cmds(&[c]), "", "runo/x", "");
        assert_eq!(found("gh pr merge 230 --merge --delete-branch"), [main()]);
        assert_eq!(
            detect(&cmds(&["gh pr merge 7 --squash"]), "", "x", "develop"),
            [Target::Branch("develop".into())]
        );
        // Auto-merge lands later; the pull request watcher still has it.
        assert!(found("gh pr merge 7 --auto --squash").is_empty());
    }

    #[test]
    fn a_tag_push_is_watched() {
        let found = |list: &[&str]| detect(&cmds(list), "", "local folder", "main");
        assert_eq!(
            found(&["git push origin v0.5.11"]),
            [Target::Tag("v0.5.11".into())]
        );
        assert_eq!(
            found(&["git push origin refs/tags/release-1"]),
            [Target::Tag("release-1".into())]
        );
        // A tag made earlier in the turn, pushed with --tags or by name.
        assert_eq!(
            found(&[
                "git tag -a rel-2 -m \"Release\"",
                "git push --follow-tags origin main"
            ]),
            [main(), Target::Tag("rel-2".into())]
        );
        assert_eq!(
            found(&["git tag rel-3 && git push origin rel-3"]),
            [Target::Tag("rel-3".into())]
        );
        assert_eq!(
            found(&["gh release create v1.0.0 --notes x"]),
            [Target::Tag("v1.0.0".into())]
        );
        // Listing or deleting tags isn't a release.
        assert!(found(&["git tag -l", "git push origin --delete v1"]).is_empty());
    }

    #[test]
    fn a_label_cut_short_drops_its_partial_word() {
        // "Ran git push origin ma…": the branch's name was cut off.
        assert!(detect(&cmds(&["git push origin ma…"]), "", "runo/x", "main").is_empty());
        assert_eq!(
            detect(
                &cmds(&["git push origin main && echo done and so…"]),
                "",
                "runo/x",
                "main"
            ),
            [main()]
        );
    }

    #[test]
    fn the_thread_can_name_what_to_watch() {
        let found = |text: &str| detect(&[], text, "runo/x", "main");
        assert_eq!(found("Merged.\n\nruno-watch: main"), [main()]);
        assert_eq!(
            found("`runo-watch: tag v0.5.11`"),
            [Target::Tag("v0.5.11".into())]
        );
        assert_eq!(found("runo-watch: run 123"), [Target::Run(123)]);
        assert_eq!(found("Watch run #456."), [Target::Run(456)]);
        assert_eq!(found("Wait for main CI"), [main()]);
        // Only on a line of its own: prose about waiting isn't a request.
        assert!(found("I'll wait for main CI to finish before tagging.").is_empty());
        // Said twice, watched once; a named tag beats an unnamed one.
        assert_eq!(
            detect(
                &cmds(&["git push origin main", "git push --tags"]),
                "runo-watch: main\nruno-watch: tag v2",
                "x",
                "main"
            ),
            [main(), Target::Tag("v2".into())]
        );
    }

    fn run(workflow: &str, status: &str, conclusion: &str) -> Run {
        Run {
            id: 1,
            workflow: workflow.into(),
            status: status.into(),
            conclusion: conclusion.into(),
            url: "https://github.com/o/r/actions/runs/1".into(),
            failed_jobs: Vec::new(),
        }
    }

    fn reading(runs: Vec<Run>) -> Read {
        Read {
            target: None,
            sha: "1a2b3c4d5e".into(),
            runs,
        }
    }

    #[test]
    fn finished_runs_wake_with_the_news() {
        let mut w = RunWatch::new(main(), 100);
        let mut read = reading(vec![
            run("CI", "in_progress", ""),
            run("Lint", "completed", "success"),
        ]);
        assert_eq!(decide(&mut w, &read, false, 200), Step::Wait);
        assert_eq!(w.sha, "1a2b3c4d5e");
        read.runs[0] = run("CI", "completed", "success");
        assert_eq!(
            decide(&mut w, &read, false, 300),
            Step::Passed("Main CI passed for 1a2b3c4".into())
        );
        // A failure names its jobs.
        read.runs[0] = Run {
            failed_jobs: vec!["test (ubuntu)".into()],
            ..run("CI", "completed", "failure")
        };
        assert_eq!(
            decide(&mut w, &read, false, 300),
            Step::Failed("Main CI failed for 1a2b3c4: test (ubuntu)".into())
        );
        // Skipped runs pass.
        read.runs[0] = run("Docs", "completed", "skipped");
        assert!(matches!(decide(&mut w, &read, false, 300), Step::Passed(_)));
    }

    #[test]
    fn the_release_workflow_is_named() {
        let mut w = RunWatch::new(Target::Tag("v0.5.11".into()), 0);
        let failed = Run {
            failed_jobs: vec!["build (macos)".into()],
            ..run("Release", "completed", "failure")
        };
        assert_eq!(
            decide(&mut w, &reading(vec![failed]), false, 10),
            Step::Failed("Release workflow failed: build (macos)".into())
        );
        assert_eq!(
            decide(
                &mut w,
                &reading(vec![run("Release", "completed", "success")]),
                false,
                10
            ),
            Step::Passed("Release workflow passed for v0.5.11".into())
        );
        assert_eq!(label(&w), "Watching release");
        assert_eq!(label(&RunWatch::new(main(), 0)), "Watching main CI");
    }

    #[test]
    fn a_busy_thread_waits_for_its_news() {
        let mut w = RunWatch::new(main(), 0);
        let read = reading(vec![run("CI", "completed", "success")]);
        assert_eq!(decide(&mut w, &read, true, 10), Step::Wait);
        assert!(matches!(decide(&mut w, &read, false, 10), Step::Passed(_)));
    }

    #[test]
    fn no_runs_or_failed_reads_or_a_long_wait_stop_the_watch() {
        let mut w = RunWatch::new(main(), 0);
        for _ in 1..MAX_EMPTY_READS {
            assert_eq!(decide(&mut w, &reading(vec![]), false, 10), Step::Wait);
        }
        assert!(matches!(
            decide(&mut w, &reading(vec![]), false, 10),
            Step::Stop(_)
        ));
        let mut w = RunWatch::new(main(), 0);
        for _ in 1..MAX_FAILED_READS {
            assert_eq!(read_failed(&mut w), None);
        }
        assert!(matches!(read_failed(&mut w), Some(Step::Stop(_))));
        // A good read clears the run of failures.
        decide(&mut w, &reading(vec![run("CI", "queued", "")]), false, 10);
        assert_eq!(w.failed_reads, 0);
        let late = MAX_WATCH.as_secs() + 1;
        let mut w = RunWatch::new(main(), 0);
        assert!(matches!(
            decide(&mut w, &reading(vec![run("CI", "queued", "")]), false, late),
            Step::Stop(_)
        ));
    }

    #[test]
    fn runs_and_jobs_parse_from_gh() {
        let runs = parse_runs(
            r#"[{"databaseId":9,"workflowName":"CI","status":"completed","conclusion":"failure","url":"u"}]"#,
        );
        assert_eq!(runs.len(), 1);
        assert!(runs[0].failed());
        let jobs = parse_failed_jobs(
            r#"{"jobs":[{"name":"lint","conclusion":"success"},{"name":"test (macos)","conclusion":"failure"}]}"#,
        );
        assert_eq!(jobs, ["test (macos)"]);
        assert!(wake_prompt("Main CI passed for 1a2b3c4", "u")
            .starts_with("Main CI passed for 1a2b3c4 (u)."));
    }
}
