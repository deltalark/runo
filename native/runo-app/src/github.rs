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

#[derive(Default)]
pub struct GhData {
    pub project: u64,
    pub prs: Vec<GhItem>,
    /// Pull requests that ask for the person's review.
    pub review_requested: Vec<u64>,
    pub issues: Vec<GhItem>,
    pub runs: Vec<GhItem>,
    pub loading: usize,
    /// Why the whole page is unavailable (no gh, not signed in, no GitHub remote).
    pub error: Option<String>,
    /// Errors of single lists, by list ("prs", "issues", "runs"); the others still show.
    pub errors: HashMap<&'static str, String>,
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

/// Parse `gh pr list --json number,title,url,headRefName,author,isDraft`.
pub fn parse_prs(json: &str) -> Vec<GhItem> {
    items(json)
        .iter()
        .map(|v| GhItem {
            number: v["number"].as_u64().unwrap_or(0),
            title: s(&v["title"]),
            url: s(&v["url"]),
            detail: s(&v["headRefName"]),
            author: s(&v["author"]["login"]),
            state: if v["isDraft"].as_bool() == Some(true) {
                "draft".into()
            } else {
                "open".into()
            },
        })
        .collect()
}

/// Parse `gh issue list --json number,title,url,author,labels`.
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
            state: "open".into(),
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

/// Parse `gh pr list --json number`: just the numbers.
pub fn parse_numbers(json: &str) -> Vec<u64> {
    items(json)
        .iter()
        .filter_map(|v| v["number"].as_u64())
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
        self.prs
            .iter()
            .filter(|p| self.review_requested.contains(&p.number))
            .collect()
    }

    /// What in Reviews needs the person: review requests and failed checks.
    pub fn needs_you(&self) -> usize {
        self.requested().len() + latest_failures(&self.runs).len()
    }
}

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

    /// Fetch PRs, issues and runs for the chosen project in the background.
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
        let Some(project) = self.gh_project() else {
            return;
        };
        if self.gh.project != project.id {
            self.gh.prs.clear();
            self.gh.review_requested.clear();
            self.gh.issues.clear();
            self.gh.runs.clear();
        }
        self.gh.project = project.id;
        self.gh.error = None;
        self.gh.errors.clear();
        self.gh.loading = 4;
        let jobs: [(&str, &[&str]); 4] = [
            (
                "review",
                &[
                    "pr",
                    "list",
                    "--search",
                    "review-requested:@me",
                    "--json",
                    "number",
                ],
            ),
            (
                "prs",
                &[
                    "pr",
                    "list",
                    "--limit",
                    "30",
                    "--json",
                    "number,title,url,headRefName,author,isDraft",
                ],
            ),
            (
                "issues",
                &[
                    "issue",
                    "list",
                    "--limit",
                    "30",
                    "--json",
                    "number,title,url,author,labels",
                ],
            ),
            (
                "runs",
                &[
                    "run",
                    "list",
                    "--limit",
                    "12",
                    "--json",
                    "databaseId,displayTitle,workflowName,status,conclusion,url,headBranch",
                ],
            ),
        ];
        // Each list is keyed by project, so a late answer for another project is dropped.
        for (key, args) in jobs {
            let mut command = std::process::Command::new("gh");
            command.args(args);
            agent::oneshot(
                format!("gh-{key}:{}", project.id),
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
                "review" => self.gh.review_requested = parse_numbers(&json),
                "issues" => self.gh.issues = parse_issues(&json),
                _ => self.gh.runs = parse_runs(&json),
            },
            Err(error) => {
                let first = error.lines().next().unwrap_or("").to_string();
                // These stop every list; anything else stays with its own list.
                let whole = if first.contains("could not start") {
                    Some(
                        "The GitHub CLI (gh) is not installed. Install it, then run gh auth login.",
                    )
                } else if first.contains("none of the git remotes")
                    || first.contains("no git remotes")
                {
                    Some("This project has no GitHub remote. Push it to GitHub to see its pull requests, issues and runs here.")
                } else if first.contains("auth login") || first.contains("not logged") {
                    Some("Sign in to GitHub first: run gh auth login in the Terminal.")
                } else {
                    None
                };
                match whole {
                    Some(why) => self.gh.error = Some(why.into()),
                    None => {
                        self.gh.errors.insert(list, first);
                    }
                }
            }
        }
    }

    /// Start a session in a new worktree that works on an issue, or reviews a PR with
    /// the PR's own branch checked out.
    fn start_from_github(&mut self, task: String, pr: Option<u64>, cx: &mut Context<Self>) {
        if self.demo_guard(cx) {
            return;
        }
        let Some(project) = self.gh_project() else {
            return;
        };
        let provider = agent::PROVIDERS
            .iter()
            .position(|p| p.id == cx.global::<AppSettings>().default_provider)
            .unwrap_or(0);
        let is_git = git::is_repo(&project.path);
        let branch = pr.map(|n| format!("runo/pr-{n}")).unwrap_or_default();
        match self.spawn_session_for(&project, provider, &branch, is_git, is_git, &task, pr, cx) {
            Ok(id) => {
                if !self.expanded.contains(&project.id) {
                    self.expanded.push(project.id);
                }
                self.select(id);
                self.launch(id, task, cx);
                self.save();
            }
            Err(error) => self.notice = Some(error),
        }
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
        let pr = (id.0 == "pr").then_some(item.number);
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
        if item.state == "draft" {
            meta.push("draft".into());
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
                            this.start_from_github(task.clone(), pr, cx);
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
            let task = format!(
                "Review pull request #{} ({}): {}. Its branch is checked out here. Read the discussion with `gh pr view {} --comments` and the changes with `gh pr diff {}` or git, then summarise problems and suggested fixes.",
                pr.number, pr.url, pr.title, pr.number, pr.number
            );
            let requested = self.gh.review_requested.contains(&pr.number);
            let marked;
            let pr = if requested {
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
                if requested {
                    c_amber()
                } else if pr.state == "draft" {
                    c_muted()
                } else {
                    c_green()
                },
                Some(("Review", task)),
                cx,
            ));
        }
        let mut issues = div().v_flex();
        for (i, issue) in self.gh.issues.iter().enumerate() {
            let task = format!(
                "Fix issue #{} ({}): {}. Read the details with `gh issue view {}`.",
                issue.number, issue.url, issue.title, issue.number
            );
            issues = issues.child(self.gh_row(
                ("issue", i),
                issue,
                IconName::CircleDot,
                c_green(),
                Some(("Start session", task)),
                cx,
            ));
        }
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
        self.page(
            "Reviews",
            div()
                .v_flex()
                .gap_2()
                .child(chips)
                .when_some(self.gh.error.clone(), |el, error| {
                    el.child(empty_hint(
                        "gh-error",
                        IconName::GitPullRequest,
                        "GitHub is not available for this project",
                        &error,
                    ))
                })
                .when(self.gh.loading > 0, |el| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child("Loading from GitHub…"),
                    )
                })
                .when(self.gh.error.is_none(), |el| {
                    el.child(section("PULL REQUESTS", self.gh.prs.len()))
                        .child(failed("prs").unwrap_or_else(|| {
                            if self.gh.prs.is_empty() {
                                empty("No open pull requests.").into_any_element()
                            } else {
                                prs.into_any_element()
                            }
                        }))
                        .child(section("ISSUES", self.gh.issues.len()))
                        .child(failed("issues").unwrap_or_else(|| {
                            if self.gh.issues.is_empty() {
                                empty("No open issues.").into_any_element()
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
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

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
        assert_eq!(parse_numbers(r#"[{"number":4},{"number":9}]"#), vec![4, 9]);
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
            review_requested: vec![5, 77],
            runs,
            ..GhData::default()
        };
        assert_eq!(data.requested().len(), 1);
        assert_eq!(data.needs_you(), 2);
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
}
