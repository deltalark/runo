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
    pub issues: Vec<GhItem>,
    pub runs: Vec<GhItem>,
    pub loading: usize,
    pub error: Option<String>,
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
    fn gh_project(&self) -> Option<Project> {
        let id = if self.gh.project != 0 {
            self.gh.project
        } else {
            self.current()
                .map(|s| s.project_id)
                .or_else(|| self.state.projects.first().map(|p| p.id))?
        };
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
        self.gh.project = project.id;
        self.gh.error = None;
        self.gh.loading = 3;
        let jobs: [(&str, &[&str]); 3] = [
            (
                "gh-prs",
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
                "gh-issues",
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
                "gh-runs",
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
        for (key, args) in jobs {
            let mut command = std::process::Command::new("gh");
            command.args(args);
            agent::oneshot(key.to_string(), command, &project.path, self.tx.clone());
        }
    }

    /// Results of background jobs that are not agent turns.
    pub(crate) fn handle_job(&mut self, key: String, result: Result<String, String>) {
        if !key.starts_with("gh-") {
            return;
        }
        self.gh.loading = self.gh.loading.saturating_sub(1);
        match result {
            Ok(json) => match key.as_str() {
                "gh-prs" => self.gh.prs = parse_prs(&json),
                "gh-issues" => self.gh.issues = parse_issues(&json),
                "gh-runs" => self.gh.runs = parse_runs(&json),
                _ => {}
            },
            Err(error) => {
                let first = error.lines().next().unwrap_or("").to_string();
                self.gh.error = Some(if first.contains("could not start") {
                    "The GitHub CLI (gh) is not installed. Install it and run gh auth login.".into()
                } else {
                    first
                });
            }
        }
    }

    /// Start a session in a new worktree that works on an issue or reviews a PR.
    fn start_from_github(&mut self, task: String, cx: &mut Context<Self>) {
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
        match self.spawn_session(&project, provider, "", is_git, is_git, &task, cx) {
            Ok(id) => {
                if !self.expanded.contains(&project.id) {
                    self.expanded.push(project.id);
                }
                self.select(id);
                self.launch(id, task, cx);
                self.measure(id);
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
    ) -> impl IntoElement {
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
        div()
            .id(id)
            .group("gh-row")
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded_md()
            .hover(|el| el.bg(rgb(c_subtle())))
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
                        .h(px(26.))
                        .opacity(0.)
                        .group_hover("gh-row", |el| el.opacity(1.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_from_github(task.clone(), cx);
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
            })
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
                    this.gh.project = pid;
                    this.gh.prs.clear();
                    this.gh.issues.clear();
                    this.gh.runs.clear();
                    this.load_github();
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
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_muted()))
                .child(title)
                .child(count.to_string())
        };
        let mut prs = div().v_flex();
        for (i, pr) in self.gh.prs.iter().enumerate() {
            let task = format!(
                "Review pull request #{} ({}): {}. Read it with `gh pr view {} --comments` and `gh pr diff {}`, then summarise problems and suggested fixes.",
                pr.number, pr.url, pr.title, pr.number, pr.number
            );
            prs = prs.child(self.gh_row(
                ("pr", i),
                pr,
                IconName::GitPullRequest,
                if pr.state == "draft" {
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
        self.page(
            "GitHub",
            div()
                .v_flex()
                .gap_2()
                .child(chips)
                .when_some(self.gh.error.clone(), |el, error| {
                    el.child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(settings::mix(c_red(), c_canvas(), 0.08)))
                            .text_size(px(12.5))
                            .text_color(rgb(c_red()))
                            .child(error),
                    )
                })
                .when(self.gh.loading > 0, |el| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_muted()))
                            .child("Loading from GitHub…"),
                    )
                })
                .child(section("PULL REQUESTS", self.gh.prs.len()))
                .child(if self.gh.prs.is_empty() {
                    empty("No open pull requests.").into_any_element()
                } else {
                    prs.into_any_element()
                })
                .child(section("ISSUES", self.gh.issues.len()))
                .child(if self.gh.issues.is_empty() {
                    empty("No open issues.").into_any_element()
                } else {
                    issues.into_any_element()
                })
                .child(section("ACTIONS", self.gh.runs.len()))
                .child(if self.gh.runs.is_empty() {
                    empty("No workflow runs.").into_any_element()
                } else {
                    runs.into_any_element()
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
    }
}
