//! GitHub pull requests, issues and Actions runs for a project, through the `gh` CLI.

use super::*;
use serde_json::Value;

#[derive(Clone, Default)]
pub struct GhItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// PRs: head branch. Runs: workflow name.
    pub detail: String,
    pub author: String,
    /// PRs: "draft"/"open". Runs: conclusion or status.
    pub state: String,
}

/// Why GitHub can't be shown for a project at all, each with its own way out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GhBlock {
    /// The project folder is not a git repository.
    NoGit,
    /// A git repository with no GitHub remote.
    NoRemote,
    /// The GitHub CLI is not installed.
    NoCli,
    /// The GitHub CLI is not signed in.
    SignedOut,
}

impl GhBlock {
    /// The block a failed `gh` call means, when it stops every list rather than one.
    pub fn from_error(first: &str) -> Option<Self> {
        if first.contains("could not start") {
            Some(Self::NoCli)
        } else if first.contains("not a git repository") {
            Some(Self::NoGit)
        } else if first.contains("none of the git remotes") || first.contains("no git remotes") {
            Some(Self::NoRemote)
        } else if first.contains("auth login") || first.contains("not logged") {
            Some(Self::SignedOut)
        } else {
            None
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::NoGit => "This project isn't a git repository yet",
            Self::NoRemote => "This project isn't on GitHub yet",
            Self::NoCli => "Install the GitHub CLI",
            Self::SignedOut => "Sign in to GitHub",
        }
    }

    fn body(self) -> &'static str {
        match self {
            Self::NoGit => "Pull requests, issues and checks show up here once the project is in git and on GitHub. An agent can set up both, and asks you before it publishes anything.",
            Self::NoRemote => "Publish it to GitHub to see its pull requests, issues and checks here. An agent can create the repository and push, and asks you first whether it should be private.",
            Self::NoCli => "Runo reads GitHub through gh. Install it, run gh auth login in the Terminal, then refresh.",
            Self::SignedOut => "Run gh auth login in the Terminal, then refresh.",
        }
    }
}

/// What an agent is asked to do to put a project on GitHub.
const SETUP_TASK: &str = "Put this project on GitHub. If the folder is not a git repository, run git init, add a .gitignore that fits the project, and make a first commit. Then ask me whether the GitHub repository should be private or public and what to name it, and only after I answer, create it with gh repo create --source . --push. Tell me the repository's URL when it is done.";

#[derive(Default)]
pub struct GhData {
    pub project: u64,
    pub prs: Vec<GhItem>,
    /// Open pull requests that ask for the person's review, whatever the filters show.
    pub review_requested: Vec<GhItem>,
    pub issues: Vec<GhItem>,
    pub runs: Vec<GhItem>,
    pub loading: usize,
    /// Why the whole page is unavailable (no git, no GitHub remote, no gh, signed out).
    pub error: Option<GhBlock>,
    /// Errors of single lists, by list ("prs", "issues", "runs"); the others still show.
    pub errors: HashMap<&'static str, String>,
    pub filter: GhFilter,
    /// The agent for sessions started here; empty uses the default agent.
    pub provider: String,
    /// The author filter's box.
    pub author: Option<Entity<InputState>>,
}

/// What the pull request and issue lists show.
#[derive(Clone, Debug, PartialEq)]
pub struct GhFilter {
    /// "open", "merged", "closed" or "all".
    pub state: &'static str,
    /// A login, or `@me`; empty for anyone.
    pub author: String,
    /// Only pull requests that ask for the person's review.
    pub review_requested: bool,
    pub prs_limit: usize,
    pub issues_limit: usize,
}

/// How many items each list loads at a time.
pub const PAGE: usize = 30;

/// How often Reviews refreshes in the background, so its badge stays current.
pub const REFRESH_SECS: u64 = 300;

impl Default for GhFilter {
    fn default() -> Self {
        Self {
            state: "open",
            author: String::new(),
            review_requested: false,
            prs_limit: PAGE,
            issues_limit: PAGE,
        }
    }
}

/// `gh pr list` arguments for the filter.
pub fn pr_list_args(f: &GhFilter) -> Vec<String> {
    let mut args: Vec<String> = ["pr", "list", "--state", f.state, "--limit"]
        .map(String::from)
        .to_vec();
    args.push(f.prs_limit.to_string());
    if !f.author.trim().is_empty() {
        args.extend(["--author".into(), f.author.trim().to_string()]);
    }
    if f.review_requested {
        args.extend(["--search".into(), "review-requested:@me".into()]);
    }
    args.extend([
        "--json".into(),
        "number,title,url,headRefName,author,isDraft,state".into(),
    ]);
    args
}

/// `gh issue list` arguments for the filter. Issues are never merged, so that shows closed ones.
pub fn issue_list_args(f: &GhFilter) -> Vec<String> {
    let state = if f.state == "merged" {
        "closed"
    } else {
        f.state
    };
    let mut args: Vec<String> = ["issue", "list", "--state", state, "--limit"]
        .map(String::from)
        .to_vec();
    args.push(f.issues_limit.to_string());
    if !f.author.trim().is_empty() {
        args.extend(["--author".into(), f.author.trim().to_string()]);
    }
    args.extend([
        "--json".into(),
        "number,title,url,author,labels,state".into(),
    ]);
    args
}

/// Where a "Review this PR" session leaves the review it wrote, in its worktree.
pub const REVIEW_FILE: &str = ".runo/review.json";

/// A review an agent wrote, waiting for the person to post it.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct ReviewDraft {
    /// COMMENT, APPROVE or REQUEST_CHANGES.
    pub event: String,
    pub body: String,
    pub comments: Vec<DraftComment>,
}

/// A comment on one line of the new version of a file.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct DraftComment {
    pub path: String,
    pub line: u64,
    pub body: String,
}

/// Read a review draft. Unknown fields are ignored and comments without a file, line
/// or text are dropped; a draft with nothing to say is None.
pub fn parse_review_draft(json: &str) -> Option<ReviewDraft> {
    let v: Value = serde_json::from_str(json).ok()?;
    let event = match s(&v["event"])
        .to_ascii_uppercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "APPROVE" | "APPROVED" => "APPROVE",
        "REQUEST_CHANGES" | "CHANGES_REQUESTED" => "REQUEST_CHANGES",
        _ => "COMMENT",
    };
    let text = |c: &Value| {
        c["body"]
            .as_str()
            .or(c["comment"].as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let comments: Vec<DraftComment> = v["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let path = c["path"].as_str().or(c["file"].as_str())?.trim();
            let line = c["line"]
                .as_u64()
                .or_else(|| c["line"].as_str().and_then(|l| l.trim().parse().ok()))?;
            let body = text(c);
            (!path.is_empty() && line > 0 && !body.is_empty()).then(|| DraftComment {
                path: path.trim_start_matches("./").to_string(),
                line,
                body,
            })
        })
        .collect();
    let body = text(&v);
    if body.is_empty() && comments.is_empty() {
        return None;
    }
    Some(ReviewDraft {
        event: event.into(),
        body,
        comments,
    })
}

impl ReviewDraft {
    /// "comment", "approval" or "request for changes", for the confirmation.
    pub fn kind(&self) -> &'static str {
        match self.event.as_str() {
            "APPROVE" => "approval",
            "REQUEST_CHANGES" => "request for changes",
            _ => "comment",
        }
    }

    /// The body for `POST /repos/{owner}/{repo}/pulls/N/reviews`. Without `inline`, line
    /// comments go into the review's text instead, for when GitHub refuses their lines.
    pub fn payload(&self, inline: bool) -> String {
        let mut body = self.body.clone();
        if !inline && !self.comments.is_empty() {
            for c in &self.comments {
                if !body.is_empty() {
                    body.push_str("\n\n");
                }
                body.push_str(&format!("`{}:{}`: {}", c.path, c.line, c.body));
            }
        }
        let mut out = serde_json::json!({ "event": self.event, "body": body });
        if inline && !self.comments.is_empty() {
            out["comments"] = self
                .comments
                .iter()
                .map(|c| serde_json::json!({ "path": c.path, "line": c.line, "side": "RIGHT", "body": c.body }))
                .collect();
        }
        out.to_string()
    }
}

/// What a draft says, for the confirmation before posting it.
pub fn review_summary(draft: &ReviewDraft) -> String {
    let n = draft.comments.len();
    let mut out = format!(
        "A {} with {n} line comment{}.",
        draft.kind(),
        if n == 1 { "" } else { "s" }
    );
    if !draft.body.is_empty() {
        let mut body: String = draft.body.chars().take(400).collect();
        if draft.body.chars().count() > 400 {
            body.push('…');
        }
        out.push_str(&format!("\n\n{body}"));
    }
    for c in draft.comments.iter().take(5) {
        let text: String = c.body.chars().take(120).collect();
        out.push_str(&format!("\n• {}:{}: {text}", c.path, c.line));
    }
    if n > 5 {
        out.push_str(&format!("\n…and {} more", n - 5));
    }
    out
}

/// The task of a "Review this PR" session.
pub fn review_task(pr: &GhItem) -> String {
    format!(
        "Review pull request #{n} ({url}): {title}. Its branch is checked out here. Read the discussion with `gh pr view {n} --comments` and the changes with `gh pr diff {n}` or git, then summarise problems and suggested fixes.\n\nThen write your review to `{REVIEW_FILE}` in this folder, so Runo can offer to post it to GitHub after the person reads it. Don't post it yourself. Use this JSON: {{\"event\": \"COMMENT\" or \"REQUEST_CHANGES\" or \"APPROVE\", \"body\": \"the overall review\", \"comments\": [{{\"path\": \"src/file.rs\", \"line\": 42, \"body\": \"what to change and why\"}}]}}. Each line is a line number in the pull request's version of the file, on a line the diff changes or shows.",
        n = pr.number,
        url = pr.url,
        title = pr.title,
    )
}

/// A session branch's pull request: state, checks and review.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct PrStatus {
    pub number: u64,
    pub url: String,
    pub title: String,
    /// OPEN, MERGED or CLOSED.
    pub state: String,
    pub draft: bool,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED or empty.
    pub review: String,
    pub passed: usize,
    pub pending: usize,
    /// Names of the checks that failed.
    pub failed: Vec<String>,
    /// The Actions run of the first failed check, for `gh run view --log-failed`.
    pub failed_run: Option<u64>,
    /// Reviews with something to say, plus conversation comments.
    pub comments: usize,
    /// A review the session's agent wrote and nobody posted yet.
    pub draft_review: Option<ReviewDraft>,
}

/// Parse `gh pr view --json number,url,title,state,isDraft,reviewDecision,statusCheckRollup,reviews,comments`.
pub fn parse_pr_status(json: &str) -> Option<PrStatus> {
    let v: Value = serde_json::from_str(json).ok()?;
    let number = v["number"].as_u64()?;
    let mut status = PrStatus {
        number,
        url: s(&v["url"]),
        title: s(&v["title"]),
        state: s(&v["state"]),
        draft: v["isDraft"].as_bool() == Some(true),
        review: s(&v["reviewDecision"]),
        ..Default::default()
    };
    for check in v["statusCheckRollup"].as_array().into_iter().flatten() {
        // Check runs have status + conclusion; commit statuses have a state.
        let outcome = check["conclusion"]
            .as_str()
            .filter(|c| !c.is_empty())
            .or(check["state"].as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let done = check["status"].as_str().is_none_or(|st| st == "COMPLETED");
        let name = check["name"]
            .as_str()
            .or(check["context"].as_str())
            .unwrap_or("check")
            .to_string();
        match outcome.as_str() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" if done => status.passed += 1,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE"
                if done =>
            {
                if status.failed_run.is_none() {
                    status.failed_run = check["detailsUrl"]
                        .as_str()
                        .or(check["targetUrl"].as_str())
                        .and_then(run_id);
                }
                status.failed.push(name);
            }
            _ => status.pending += 1,
        }
    }
    status.comments = v["reviews"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["state"] != "APPROVED" && !s(&r["body"]).trim().is_empty())
        .count()
        + v["comments"].as_array().map_or(0, Vec::len);
    Some(status)
}

/// The Actions run id in a check's link, such as `.../actions/runs/123/job/456`.
pub fn run_id(url: &str) -> Option<u64> {
    let rest = url.split("/actions/runs/").nth(1)?;
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Review feedback on a pull request, as a list for the agent: reviews and conversation
/// comments from `gh pr view --json reviews,comments`, and line comments from
/// `gh api repos/{owner}/{repo}/pulls/N/comments`.
pub fn review_feedback(view: &str, line_comments: &str) -> Vec<String> {
    let mut out = Vec::new();
    let v: Value = serde_json::from_str(view).unwrap_or_default();
    for r in v["reviews"].as_array().into_iter().flatten() {
        let body = s(&r["body"]);
        if r["state"] != "APPROVED" && !body.trim().is_empty() {
            out.push(format!(
                "{} ({}): {}",
                s(&r["author"]["login"]),
                s(&r["state"]).to_lowercase().replace('_', " "),
                body.trim()
            ));
        }
    }
    for c in v["comments"].as_array().into_iter().flatten() {
        let body = s(&c["body"]);
        if !body.trim().is_empty() {
            out.push(format!("{}: {}", s(&c["author"]["login"]), body.trim()));
        }
    }
    for c in items(line_comments) {
        let body = s(&c["body"]);
        if body.trim().is_empty() {
            continue;
        }
        let line = c["line"].as_u64().or(c["original_line"].as_u64());
        let place = match line {
            Some(n) => format!("{}:{n}", s(&c["path"])),
            None => s(&c["path"]),
        };
        out.push(format!(
            "{} on `{place}`: {}",
            s(&c["user"]["login"]),
            body.trim()
        ));
    }
    out
}

/// Parse `gh pr list --json number,title,url,headRefName,author,isDraft,state`.
pub fn parse_prs(json: &str) -> Vec<GhItem> {
    items(json)
        .iter()
        .map(|v| GhItem {
            number: v["number"].as_u64().unwrap_or(0),
            title: s(&v["title"]),
            url: s(&v["url"]),
            detail: s(&v["headRefName"]),
            author: s(&v["author"]["login"]),
            state: match v["state"].as_str() {
                Some("MERGED") => "merged".into(),
                Some("CLOSED") => "closed".into(),
                _ if v["isDraft"].as_bool() == Some(true) => "draft".into(),
                _ => "open".into(),
            },
        })
        .collect()
}

/// Parse `gh issue list --json number,title,url,author,labels,state`.
pub fn parse_issues(json: &str) -> Vec<GhItem> {
    items(json)
        .iter()
        .map(|v| GhItem {
            number: v["number"].as_u64().unwrap_or(0),
            title: s(&v["title"]),
            url: s(&v["url"]),
            detail: v["labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| l["name"].as_str())
                .collect::<Vec<_>>()
                .join(", "),
            author: s(&v["author"]["login"]),
            state: if v["state"] == "CLOSED" {
                "closed".into()
            } else {
                "open".into()
            },
        })
        .collect()
}

/// Parse `gh run list --json databaseId,displayTitle,workflowName,status,conclusion,url,headBranch`.
pub fn parse_runs(json: &str) -> Vec<GhItem> {
    items(json)
        .iter()
        .map(|v| {
            let conclusion = s(&v["conclusion"]);
            GhItem {
                number: v["databaseId"].as_u64().unwrap_or(0),
                title: s(&v["displayTitle"]),
                url: s(&v["url"]),
                detail: format!("{} · {}", s(&v["workflowName"]), s(&v["headBranch"])),
                author: String::new(),
                state: if conclusion.is_empty() {
                    s(&v["status"])
                } else {
                    conclusion
                },
            }
        })
        .collect()
}

/// The newest run of each workflow and branch, when it failed. Older failures that a later
/// run fixed don't need anyone.
pub fn latest_failures(runs: &[GhItem]) -> Vec<&GhItem> {
    let mut seen = std::collections::HashSet::new();
    runs.iter()
        .filter(|r| seen.insert(r.detail.clone()))
        .filter(|r| {
            matches!(
                r.state.as_str(),
                "failure" | "timed_out" | "startup_failure"
            )
        })
        .collect()
}

impl GhData {
    /// Open pull requests that ask for the person's review.
    pub fn requested(&self) -> Vec<&GhItem> {
        self.review_requested.iter().collect()
    }

    /// Whether pull request `number` asks for the person's review.
    pub fn is_requested(&self, number: u64) -> bool {
        self.review_requested.iter().any(|p| p.number == number)
    }

    /// What in Reviews needs the person: review requests and failed checks.
    pub fn needs_you(&self) -> usize {
        self.requested().len() + latest_failures(&self.runs).len()
    }
}

/// Every list on the page.
const ALL_LISTS: [&str; 4] = ["review", "prs", "issues", "runs"];

fn items(json: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(json)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

impl RunoApp {
    /// Reviews follows the project in the top bar.
    fn gh_project(&self) -> Option<Project> {
        let id = self.current_project()?;
        self.state.projects.iter().find(|p| p.id == id).cloned()
    }

    /// Make the author filter's box and start refreshing Reviews in the background.
    pub(crate) fn init_github(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let author = cx.new(|cx| InputState::new(window, cx).placeholder("Author, such as @me"));
        let sub = cx.subscribe_in(&author, window, |this, input, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let value = input.read(cx).value().trim().to_string();
                if value != this.gh.filter.author {
                    this.gh.filter.author = value;
                    this.gh.filter.prs_limit = PAGE;
                    this.gh.filter.issues_limit = PAGE;
                    this.fetch_github(&["prs", "issues"]);
                    cx.notify();
                }
            }
        });
        self._subscriptions.push(sub);
        self.gh.author = Some(author);
        // Soon after launch, then every few minutes, so the badge counts review requests
        // and failed runs without visiting Reviews.
        cx.spawn(async move |this, cx| {
            let mut wait = 5;
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(wait))
                    .await;
                wait = REFRESH_SECS;
                if this
                    .update(cx, |this, cx| {
                        if this.gh.loading == 0 {
                            this.fetch_github(&ALL_LISTS);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }

    /// Show Reviews and fetch PRs, issues and runs for the chosen project.
    pub(crate) fn load_github(&mut self) {
        self.view = View::GitHub;
        if self.demo {
            self.gh.prs = vec![GhItem {
                number: 42,
                title: "Refine sidebar navigation".into(),
                url: "https://github.com/example/runo/pull/42".into(),
                detail: "feature/navigation".into(),
                author: "you".into(),
                state: "open".into(),
            }];
            self.gh.issues = vec![GhItem {
                number: 17,
                title: "Search should match branch names".into(),
                url: "https://github.com/example/runo/issues/17".into(),
                detail: "bug".into(),
                author: "teammate".into(),
                state: "open".into(),
            }];
            self.gh.runs = vec![GhItem {
                number: 1,
                title: "Refine sidebar navigation".into(),
                url: String::new(),
                detail: "CI · feature/navigation".into(),
                author: String::new(),
                state: "success".into(),
            }];
            return;
        }
        self.fetch_github(&ALL_LISTS);
    }

    /// Fetch some of the lists for the chosen project in the background.
    pub(crate) fn fetch_github(&mut self, lists: &[&'static str]) {
        if self.demo {
            return;
        }
        let Some(project) = self.gh_project() else {
            return;
        };
        if self.gh.project != project.id {
            self.gh.prs.clear();
            self.gh.review_requested.clear();
            self.gh.issues.clear();
            self.gh.runs.clear();
            self.gh.errors.clear();
            // Answers still on their way for the other project are dropped uncounted.
            self.gh.loading = 0;
            self.gh.filter.prs_limit = PAGE;
            self.gh.filter.issues_limit = PAGE;
        }
        self.gh.project = project.id;
        self.gh.error = None;
        for list in lists {
            self.gh.errors.remove(list);
        }
        // Without git, gh can only fail, once per list; say it once instead.
        if !git::is_repo(&project.path) {
            self.gh.error = Some(GhBlock::NoGit);
            self.gh.loading = 0;
            return;
        }
        self.gh.loading += lists.len();
        // Each list is keyed by project, so a late answer for another project is dropped.
        for &list in lists {
            let args: Vec<String> = match list {
                "review" => [
                    "pr",
                    "list",
                    "--search",
                    "review-requested:@me",
                    "--limit",
                    "100",
                    "--json",
                    "number,title,url,headRefName,author,isDraft,state",
                ]
                .map(String::from)
                .to_vec(),
                "prs" => pr_list_args(&self.gh.filter),
                "issues" => issue_list_args(&self.gh.filter),
                _ => [
                    "run",
                    "list",
                    "--limit",
                    "12",
                    "--json",
                    "databaseId,displayTitle,workflowName,status,conclusion,url,headBranch",
                ]
                .map(String::from)
                .to_vec(),
            };
            let mut command = std::process::Command::new("gh");
            command.args(args);
            agent::oneshot(
                format!("gh-{list}:{}", project.id),
                command,
                &project.path,
                self.tx.clone(),
            );
        }
    }

    /// Results of background jobs that are not agent turns.
    pub(crate) fn handle_job(&mut self, key: String, result: Result<String, String>) {
        let Some((list, project)) = key
            .strip_prefix("gh-")
            .and_then(|k| k.split_once(':'))
            .and_then(|(list, p)| Some((list, p.parse::<u64>().ok()?)))
        else {
            return;
        };
        if project != self.gh.project {
            return;
        }
        let list: &'static str = match list {
            "prs" => "prs",
            "review" => "review",
            "issues" => "issues",
            _ => "runs",
        };
        self.gh.loading = self.gh.loading.saturating_sub(1);
        match result {
            Ok(json) => match list {
                "prs" => self.gh.prs = parse_prs(&json),
                "review" => self.gh.review_requested = parse_prs(&json),
                "issues" => self.gh.issues = parse_issues(&json),
                _ => self.gh.runs = parse_runs(&json),
            },
            Err(error) => {
                let first = error.lines().next().unwrap_or("").to_string();
                // Lists refresh every few minutes; the same failure is written once.
                crate::applog::warn_once(
                    "gh",
                    format!(
                        "project #{project}: gh {list} list failed: {}",
                        crate::applog::gist(&error)
                    ),
                );
                // These stop every list; anything else stays with its own list.
                let whole = GhBlock::from_error(&first);
                match whole {
                    Some(block) => self.gh.error = Some(block),
                    None => {
                        self.gh.errors.insert(list, first);
                    }
                }
            }
        }
    }

    /// Start a session in a new worktree that works on an issue, or reviews a PR with
    /// the PR's own branch checked out. It runs with the agent picked on the page.
    fn start_from_github(&mut self, task: String, pr: Option<&GhItem>, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some(project) = self.gh_project() else {
            return;
        };
        let chosen = if self.gh.provider.is_empty() {
            cx.global::<AppSettings>().default_provider.clone()
        } else {
            self.gh.provider.clone()
        };
        let provider = agent::PROVIDERS
            .iter()
            .position(|p| p.id == chosen)
            .unwrap_or(0);
        let is_git = git::is_repo(&project.path);
        let branch = pr
            .map(|p| format!("runo/pr-{}", p.number))
            .unwrap_or_default();
        if pr.is_some() && is_git {
            // The review draft stays out of the PR's changes, in every worktree.
            git::exclude(&project.path, "/.runo/");
        }
        let number = pr.map(|p| p.number);
        match self.spawn_session_for(
            &project, provider, &branch, is_git, is_git, &task, number, cx,
        ) {
            Ok(id) => {
                // The PR is known even when its branch is checked out under another name.
                if let (Some(pr), Some(s)) = (pr, self.state.session_mut(id)) {
                    s.pr_url = pr.url.clone();
                }
                if !self.expanded.contains(&project.id) {
                    self.expanded.push(project.id);
                }
                self.select(id);
                self.launch(id, task, cx);
                self.save();
            }
            Err(error) => self.fail("gh", error),
        }
    }

    /// Post the review a session's agent wrote, after the person confirmed it. When
    /// GitHub refuses the line comments (a line outside the diff), they go into the
    /// review's text instead.
    pub(crate) fn post_review(&mut self, id: u64) {
        let (Some(pr), Some(dir)) = (
            self.pr_status.get(&id).cloned(),
            self.state.session(id).map(|s| s.dir.clone()),
        ) else {
            return;
        };
        let Some(draft) = pr.draft_review.clone() else {
            return;
        };
        if !self.begin(id, &format!("Posting your review on #{}.", pr.number)) {
            return;
        }
        let number = pr.number;
        self.bg(
            move || {
                let post = |payload: String| -> Result<(), String> {
                    use std::io::Write;
                    let mut child = std::process::Command::new("gh")
                        .args([
                            "api",
                            "-X",
                            "POST",
                            &format!("repos/{{owner}}/{{repo}}/pulls/{number}/reviews"),
                            "--input",
                            "-",
                        ])
                        .current_dir(&dir)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|e| format!("could not run gh: {e}"))?;
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(payload.as_bytes());
                    }
                    let out = child.wait_with_output().map_err(|e| e.to_string())?;
                    if out.status.success() {
                        Ok(())
                    } else {
                        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        let body = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        Err(if err.is_empty() { body } else { err })
                    }
                };
                let folded = match post(draft.payload(true)) {
                    Ok(()) => false,
                    Err(_) if !draft.comments.is_empty() => {
                        post(draft.payload(false))?;
                        true
                    }
                    Err(error) => return Err(error),
                };
                // Posted drafts are kept beside it, so the button goes away.
                let file = std::path::Path::new(&dir).join(REVIEW_FILE);
                let _ = std::fs::rename(
                    &file,
                    file.with_file_name(format!("review-posted-{}.json", workspace::unix_now())),
                );
                Ok(folded)
            },
            move |this, result, _| {
                this.git_busy.remove(&id);
                match result {
                    Ok(folded) => {
                        this.state
                            .log(id, format!("Posted a review on pull request #{number}"));
                        this.info = Some((
                            if folded {
                                format!("Posted your review on #{number}. GitHub refused the line comments' places, so they are in the review's text.")
                            } else {
                                format!("Posted your review on #{number}.")
                            },
                            std::time::Instant::now(),
                        ));
                        this.save();
                    }
                    Err(error) => {
                        this.fail("gh", format!(
                            "Could not post the review: {}",
                            error.lines().next().unwrap_or("")
                        ))
                    }
                }
                this.load_pr_status(id);
            },
        );
    }

    fn gh_row(
        &self,
        id: (&'static str, usize),
        item: &GhItem,
        mark: IconName,
        color: u32,
        action: Option<(&'static str, String)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pr = (id.0 == "pr").then(|| item.clone());
        let url = item.url.clone();
        let mut meta = Vec::new();
        if item.number > 0 && !id.0.starts_with("run") {
            meta.push(format!("#{}", item.number));
        }
        for part in [&item.detail, &item.author] {
            if !part.is_empty() {
                meta.push(part.clone());
            }
        }
        if matches!(item.state.as_str(), "draft" | "merged" | "closed") && !id.0.starts_with("run")
        {
            meta.push(item.state.clone());
        }
        let row = div()
            .id(id)
            .group("gh-row")
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded_md()
            .hover(|el| el.bg(tint(c_subtle())))
            .child(icon(mark, 14., color))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(meta.join(" · ")),
                    ),
            )
            .when_some(action, |el, (label, task)| {
                el.child(
                    self.button((label, id.1), label, false)
                        .child(icon(
                            if label == "Review" {
                                IconName::GitCompare
                            } else {
                                IconName::Play
                            },
                            12.,
                            c_secondary(),
                        ))
                        .h(px(26.))
                        .opacity(0.)
                        .group_hover("gh-row", |el| el.opacity(1.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_from_github(task.clone(), pr.as_ref(), cx);
                            cx.notify();
                        })),
                )
            })
            .when(!url.is_empty(), |el| {
                el.child(
                    self.ghost_button_sized(
                        (
                            if id.0 == "pr" {
                                "pr-open"
                            } else if id.0 == "issue" {
                                "issue-open"
                            } else {
                                "run-open"
                            },
                            id.1,
                        ),
                        IconName::ExternalLink,
                        "Open on GitHub",
                        26.,
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                )
            });
        stagger(
            row,
            SharedString::from(format!("{}-in-{}", id.0, id.1)),
            id.1,
        )
    }

    pub(crate) fn github_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.gh_project().map(|p| p.id);
        let mut chips = div().flex().flex_wrap().items_center().gap_2();
        for (i, project) in self.state.projects.iter().enumerate() {
            let pid = project.id;
            chips = chips.child(
                self.chip(
                    ("gh-project", i),
                    project.name.clone(),
                    current == Some(pid),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    // Picking a project here switches the top bar too.
                    this.project = Some(pid);
                    this.new_project = Some(pid);
                    this.load_github();
                    this.save();
                    cx.notify();
                })),
            );
        }
        chips = chips.child(
            self.ghost_button_sized("gh-refresh", IconName::RefreshCw, "Refresh", 28.)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.load_github();
                    cx.notify();
                })),
        );
        let filter = &self.gh.filter;
        let mut states = segmented();
        for (i, (label, state)) in [
            ("Open", "open"),
            ("Merged", "merged"),
            ("Closed", "closed"),
            ("All", "all"),
        ]
        .into_iter()
        .enumerate()
        {
            states = states.child(
                segment(("gh-state", i), label, filter.state == state).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.gh.filter.state = state;
                        this.gh.filter.prs_limit = PAGE;
                        this.gh.filter.issues_limit = PAGE;
                        this.fetch_github(&["prs", "issues"]);
                        cx.notify();
                    },
                )),
            );
        }
        let mut filters = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(states)
            .child(
                self.chip(
                    ("gh-requested", 0),
                    "Review requested".into(),
                    filter.review_requested,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.gh.filter.review_requested = !this.gh.filter.review_requested;
                    this.gh.filter.prs_limit = PAGE;
                    this.fetch_github(&["prs"]);
                    cx.notify();
                })),
            )
            .child(div().flex_1());
        if let Some(author) = &self.gh.author {
            filters = filters.child(
                div()
                    .w(px(180.))
                    .child(Input::new(author).bordered(true).h(px(30.))),
            );
        }
        // Agents still being detected stay listed until Runo knows they're missing.
        let chosen = if self.gh.provider.is_empty() {
            cx.global::<AppSettings>().default_provider.clone()
        } else {
            self.gh.provider.clone()
        };
        let mut picker = segmented();
        for (i, p) in agent::PROVIDERS.iter().enumerate() {
            let caps = caps::get(p.id);
            if p.id != chosen && caps.detected && !caps.installed {
                continue;
            }
            let id = p.id;
            picker = picker.child(segment(("gh-agent", i), p.name, p.id == chosen).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.gh.provider = id.to_string();
                    cx.notify();
                }),
            ));
        }
        let agents = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child("Review and Start session use"),
            )
            .child(picker);
        let more = |list: &'static str, shown: usize, limit: usize, cx: &mut Context<Self>| {
            (shown >= limit).then(|| {
                self.button(("gh-more", (list == "issues") as usize), "Load more", false)
                    .mt_1()
                    .h(px(26.))
                    .text_xs()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if list == "prs" {
                            this.gh.filter.prs_limit += PAGE;
                        } else {
                            this.gh.filter.issues_limit += PAGE;
                        }
                        this.fetch_github(&[list]);
                        cx.notify();
                    }))
            })
        };
        let section = |title: &'static str, count: usize| {
            div()
                .mt_2()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(c_muted()))
                .child(title)
                .child(count.to_string())
        };
        let mut prs = div().v_flex();
        for (i, pr) in self.gh.prs.iter().enumerate() {
            let task = review_task(pr);
            let requested = self.gh.is_requested(pr.number);
            let open = matches!(pr.state.as_str(), "open" | "draft");
            let marked;
            let pr = if requested && open {
                marked = GhItem {
                    detail: format!("Review requested  ·  {}", pr.detail),
                    ..pr.clone()
                };
                &marked
            } else {
                pr
            };
            prs = prs.child(self.gh_row(
                ("pr", i),
                pr,
                IconName::GitPullRequest,
                match pr.state.as_str() {
                    "merged" => c_blue(),
                    "closed" | "draft" => c_muted(),
                    _ if requested => c_amber(),
                    _ => c_green(),
                },
                open.then_some(("Review", task)),
                cx,
            ));
        }
        prs = prs.children(more("prs", self.gh.prs.len(), filter.prs_limit, cx));
        let mut issues = div().v_flex();
        for (i, issue) in self.gh.issues.iter().enumerate() {
            let task = format!(
                "Fix issue #{} ({}): {}. Read the details with `gh issue view {}`.",
                issue.number, issue.url, issue.title, issue.number
            );
            let open = issue.state == "open";
            issues = issues.child(self.gh_row(
                ("issue", i),
                issue,
                IconName::CircleDot,
                if open { c_green() } else { c_muted() },
                open.then_some(("Start session", task)),
                cx,
            ));
        }
        issues = issues.children(more(
            "issues",
            self.gh.issues.len(),
            filter.issues_limit,
            cx,
        ));
        let mut runs = div().v_flex();
        for (i, run) in self.gh.runs.iter().enumerate() {
            let (mark, color) = match run.state.as_str() {
                "success" => (IconName::CircleCheck, c_green()),
                "failure" | "cancelled" | "timed_out" => (IconName::CircleX, c_red()),
                _ => (IconName::Loader, c_amber()),
            };
            runs = runs.child(self.gh_row(("run", i), run, mark, color, None, cx));
        }
        let empty = |text: &'static str| {
            div()
                .px_3()
                .py_2()
                .text_size(px(12.5))
                .text_color(rgb(c_muted()))
                .child(text)
        };
        let failed = |list: &'static str| {
            self.gh.errors.get(list).map(|why| {
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_red()))
                    .child(icon(IconName::CircleAlert, 13., c_red()))
                    .child(format!("Could not load: {why}"))
                    .into_any_element()
            })
        };
        let blocked = self.gh.error.map(|block| self.gh_blocked(block, cx));
        self.page(
            "GitHub",
            div().v_flex().gap_3().child(chips).children(blocked).when(
                self.gh.error.is_none(),
                |el| {
                    el.child(filters)
                        .child(agents)
                        .when(self.gh.loading > 0, |el| {
                            el.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .child("Loading from GitHub…"),
                            )
                        })
                        .child(section("PULL REQUESTS", self.gh.prs.len()))
                        .child(failed("prs").unwrap_or_else(|| {
                            if self.gh.prs.is_empty() {
                                empty("No pull requests match.").into_any_element()
                            } else {
                                prs.into_any_element()
                            }
                        }))
                        .child(section("ISSUES", self.gh.issues.len()))
                        .child(failed("issues").unwrap_or_else(|| {
                            if self.gh.issues.is_empty() {
                                empty("No issues match.").into_any_element()
                            } else {
                                issues.into_any_element()
                            }
                        }))
                        .child(section("ACTIONS", self.gh.runs.len()))
                        .child(failed("runs").unwrap_or_else(|| {
                            if self.gh.runs.is_empty() {
                                empty("No workflow runs.").into_any_element()
                            } else {
                                runs.into_any_element()
                            }
                        }))
                },
            ),
        )
    }

    /// The page when GitHub can't be shown for the project: what is missing and the
    /// way to fix it, in place of the lists and their filters.
    fn gh_blocked(&self, block: GhBlock, cx: &mut Context<Self>) -> AnyElement {
        let refresh = self
            .button("gh-blocked-refresh", "Refresh", false)
            .child(icon(IconName::RefreshCw, 12., c_secondary()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.load_github();
                cx.notify();
            }));
        let actions = div().flex().items_center().justify_center().gap_2();
        let actions = match block {
            GhBlock::NoGit | GhBlock::NoRemote => actions
                .child(
                    self.button(
                        "gh-setup",
                        if block == GhBlock::NoGit {
                            "Set it up with an agent"
                        } else {
                            "Publish with an agent"
                        },
                        true,
                    )
                    .child(icon(IconName::Sparkles, 12., c_canvas()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.start_from_github(SETUP_TASK.to_string(), None, cx);
                        cx.notify();
                    })),
                )
                .when(block == GhBlock::NoGit, |el| {
                    el.child(
                        self.button("gh-git-init", "Only run git init", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(project) = this.gh_project() {
                                    let done = std::process::Command::new("git")
                                        .arg("init")
                                        .current_dir(&project.path)
                                        .output();
                                    if !done.is_ok_and(|o| o.status.success()) {
                                        this.fail(
                                            "gh",
                                            "Runo couldn't run git init in this folder.".into(),
                                        );
                                    }
                                }
                                this.load_github();
                                cx.notify();
                            })),
                    )
                }),
            GhBlock::NoCli => actions
                .child(
                    self.button("gh-get-cli", "Get the GitHub CLI", true)
                        .child(icon(IconName::ExternalLink, 12., c_canvas()))
                        .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://cli.github.com"))),
                )
                .child(refresh),
            GhBlock::SignedOut => actions.child(refresh),
        };
        div()
            .mt_2()
            .pb_8()
            .v_flex()
            .items_center()
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .child(empty_hint(
                "gh-blocked",
                IconName::GitPullRequest,
                block.title(),
                block.body(),
            ))
            // The hint keeps generous space below its text; pull the buttons up to it.
            .child(actions.mt(px(-20.)))
            .into_any_element()
    }
}

/// A row of connected choices, one of them on, like Home's project scope.
fn segmented() -> Div {
    div()
        .flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .rounded_lg()
        .border_1()
        .border_color(rgb(c_border()))
}

fn segment(id: (&'static str, usize), label: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .px_3()
        .flex()
        .items_center()
        .rounded_md()
        .cursor_pointer()
        .text_size(px(12.5))
        .when(on, |el| el.bg(tint(c_active())).text_color(rgb(c_text())))
        .when(!on, |el| {
            el.text_color(rgb(c_secondary()))
                .hover(|el| el.bg(tint(c_subtle())))
        })
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn whole_page_errors_name_their_fix() {
        let no_git = "failed to run git: fatal: not a git repository (or any of the parent directories): .git";
        assert_eq!(GhBlock::from_error(no_git), Some(GhBlock::NoGit));
        let base = "failed to determine base repo: failed to run git: fatal: not a git repository";
        assert_eq!(GhBlock::from_error(base), Some(GhBlock::NoGit));
        let remote =
            "none of the git remotes configured for this repository point to a known GitHub host";
        assert_eq!(GhBlock::from_error(remote), Some(GhBlock::NoRemote));
        assert_eq!(
            GhBlock::from_error("To get started with GitHub CLI, please run:  gh auth login"),
            Some(GhBlock::SignedOut)
        );
        assert_eq!(GhBlock::from_error("HTTP 502: Bad Gateway"), None);
    }

    #[test]
    fn parses_gh_json() {
        let prs = parse_prs(
            r#"[{"number":4,"title":"Add x","url":"u","headRefName":"feat/x","author":{"login":"ann"},"isDraft":true}]"#,
        );
        assert_eq!(
            (
                prs[0].number,
                prs[0].detail.as_str(),
                prs[0].author.as_str(),
                prs[0].state.as_str()
            ),
            (4, "feat/x", "ann", "draft")
        );
        let issues = parse_issues(
            r#"[{"number":9,"title":"Bug","url":"u","author":{"login":"bo"},"labels":[{"name":"bug"},{"name":"p1"}]}]"#,
        );
        assert_eq!(issues[0].detail, "bug, p1");
        let runs = parse_runs(
            r#"[{"databaseId":7,"displayTitle":"CI","workflowName":"Build","status":"completed","conclusion":"failure","url":"u","headBranch":"main"},{"databaseId":8,"displayTitle":"CI","workflowName":"Build","status":"in_progress","conclusion":"","url":"u","headBranch":"main"}]"#,
        );
        assert_eq!(
            (runs[0].state.as_str(), runs[1].state.as_str()),
            ("failure", "in_progress")
        );
        assert_eq!(runs[0].detail, "Build · main");
        assert!(parse_prs("not json").is_empty());
        let merged = parse_prs(r#"[{"number":5,"isDraft":true,"state":"MERGED"}]"#);
        assert_eq!(merged[0].state, "merged");
        let closed = parse_issues(r#"[{"number":9,"state":"CLOSED"}]"#);
        assert_eq!(closed[0].state, "closed");
    }

    #[test]
    fn only_the_newest_run_of_each_workflow_counts() {
        let run = |detail: &str, state: &str| GhItem {
            detail: detail.into(),
            state: state.into(),
            ..GhItem::default()
        };
        // Newest first, as gh lists them: CI on main failed before and passes now.
        let runs = vec![
            run("CI · main", "success"),
            run("Lint · main", "failure"),
            run("CI · main", "failure"),
            run("CI · feat", "in_progress"),
        ];
        let failed = latest_failures(&runs);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].detail, "Lint · main");
        let data = GhData {
            prs: vec![
                GhItem {
                    number: 4,
                    ..GhItem::default()
                },
                GhItem {
                    number: 5,
                    ..GhItem::default()
                },
            ],
            review_requested: vec![
                GhItem {
                    number: 5,
                    ..GhItem::default()
                },
                GhItem {
                    number: 77,
                    ..GhItem::default()
                },
            ],
            runs,
            ..GhData::default()
        };
        // A review request counts even when the list's filters hide it.
        assert_eq!(data.requested().len(), 2);
        assert!(data.is_requested(77) && !data.is_requested(4));
        assert_eq!(data.needs_you(), 3);
    }

    #[test]
    fn parses_a_session_pr_with_checks_and_reviews() {
        let json = r#"{"number":12,"url":"https://github.com/o/r/pull/12","title":"Add x","state":"OPEN","isDraft":false,"reviewDecision":"CHANGES_REQUESTED",
            "statusCheckRollup":[
              {"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://github.com/o/r/actions/runs/5/job/1"},
              {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://github.com/o/r/actions/runs/777/job/9"},
              {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":"","detailsUrl":""},
              {"__typename":"StatusContext","context":"ci/legacy","state":"ERROR","targetUrl":"https://ci.example/1"}],
            "reviews":[{"author":{"login":"ann"},"state":"CHANGES_REQUESTED","body":"Please rename"},{"author":{"login":"bo"},"state":"APPROVED","body":"ok"}],
            "comments":[{"author":{"login":"cy"},"body":"Also docs?"}]}"#;
        let pr = parse_pr_status(json).unwrap();
        assert_eq!(
            (pr.number, pr.state.as_str(), pr.draft),
            (12, "OPEN", false)
        );
        assert_eq!(pr.review, "CHANGES_REQUESTED");
        assert_eq!((pr.passed, pr.pending), (1, 1));
        assert_eq!(pr.failed, ["test", "ci/legacy"]);
        assert_eq!(pr.failed_run, Some(777));
        assert_eq!(pr.comments, 2);
        assert!(parse_pr_status("no pull requests found").is_none());
        assert_eq!(run_id("https://github.com/o/r/actions/runs/42"), Some(42));
        assert_eq!(run_id("https://ci.example/1"), None);

        let lines = r#"[{"path":"src/a.rs","line":3,"user":{"login":"ann"},"body":"Use a match"},{"path":"b.rs","line":null,"original_line":7,"user":{"login":"bo"},"body":" "}]"#;
        let feedback = review_feedback(json, lines);
        assert_eq!(
            feedback,
            [
                "ann (changes requested): Please rename",
                "cy: Also docs?",
                "ann on `src/a.rs:3`: Use a match",
            ]
        );
    }

    #[test]
    fn filters_become_gh_arguments() {
        let f = GhFilter::default();
        assert_eq!(
            pr_list_args(&f).join(" "),
            "pr list --state open --limit 30 --json number,title,url,headRefName,author,isDraft,state"
        );
        let f = GhFilter {
            state: "merged",
            author: " @me ".into(),
            review_requested: true,
            prs_limit: 60,
            issues_limit: 90,
        };
        let prs = pr_list_args(&f).join(" ");
        assert!(prs.starts_with(
            "pr list --state merged --limit 60 --author @me --search review-requested:@me"
        ));
        // Issues are never merged: that filter shows closed ones.
        assert!(issue_list_args(&f)
            .join(" ")
            .starts_with("issue list --state closed --limit 90 --author @me --json"));
    }

    #[test]
    fn reads_review_drafts_defensively() {
        let draft = parse_review_draft(
            r#"{"event":"request changes","body":" Needs work ","comments":[
                {"path":"./src/a.rs","line":12,"body":"Handle None"},
                {"file":"b.rs","line":"7","comment":"Typo"},
                {"path":"c.rs","body":"no line"},
                {"path":"d.rs","line":3,"body":"  "}],"extra":1}"#,
        )
        .unwrap();
        assert_eq!(draft.event, "REQUEST_CHANGES");
        assert_eq!(draft.body, "Needs work");
        assert_eq!(
            draft.comments,
            [
                DraftComment {
                    path: "src/a.rs".into(),
                    line: 12,
                    body: "Handle None".into()
                },
                DraftComment {
                    path: "b.rs".into(),
                    line: 7,
                    body: "Typo".into()
                }
            ]
        );
        assert_eq!(
            parse_review_draft(r#"{"event":"lgtm","body":"ok"}"#)
                .unwrap()
                .event,
            "COMMENT"
        );
        assert!(parse_review_draft(r#"{"event":"APPROVE","body":""}"#).is_none());
        assert!(parse_review_draft("not json").is_none());

        let inline: Value = serde_json::from_str(&draft.payload(true)).unwrap();
        assert_eq!(inline["event"], "REQUEST_CHANGES");
        assert_eq!(inline["comments"][0]["path"], "src/a.rs");
        assert_eq!(inline["comments"][0]["side"], "RIGHT");
        assert_eq!(inline["comments"][1]["line"], 7);
        let folded: Value = serde_json::from_str(&draft.payload(false)).unwrap();
        assert!(folded.get("comments").is_none());
        assert_eq!(
            folded["body"],
            "Needs work\n\n`src/a.rs:12`: Handle None\n\n`b.rs:7`: Typo"
        );
        let summary = review_summary(&draft);
        assert!(summary.starts_with("A request for changes with 2 line comments."));
        assert!(summary.contains("src/a.rs:12: Handle None"));
        assert!(review_task(&GhItem {
            number: 3,
            ..GhItem::default()
        })
        .contains(REVIEW_FILE));
    }
}
