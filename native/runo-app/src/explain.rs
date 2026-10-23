//! Explain: git's errors and puzzling states in plain words, each with the step that
//! gets out of it, and the seatbelt's stop before a push sends something people regret.

use super::*;
use gitplace::SyncStep;
use seatbelt::Snag;

/// A way out, as a button on the card.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fix {
    /// Bring the remote's commits in, then send.
    Sync,
    /// Set uncommitted work aside.
    SetAside,
    /// Start a branch where the folder is.
    NewBranch,
    /// Switch to this branch.
    SwitchTo(String),
    /// Put the commits not sent yet on a new branch.
    MoveToBranch,
    /// Send after all, past the seatbelt.
    SendAnyway(SyncStep),
    /// Send this branch after all, past the seatbelt.
    SendBranchAnyway(String),
    /// Send this branch's stack and open its pull requests after all.
    OpenStackAnyway(String),
    /// Open Changes, to take something out.
    OpenChanges,
    /// Take the last commit back into the folder.
    Uncommit,
}

/// Something git said or did, in plain words.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Explained {
    pub title: String,
    pub words: String,
    pub fixes: Vec<(String, Fix)>,
    /// Amber: something to stop for, not just to understand.
    pub stop: bool,
}

/// What the card at the top of the Git place shows.
#[derive(Clone)]
pub(crate) struct Banner {
    pub explained: Explained,
    /// Git's own words, behind "What git said".
    pub raw: String,
    /// Things the seatbelt found, by file and line.
    pub found: Vec<(String, usize, Snag)>,
}

fn explained(title: &str, words: impl Into<String>, fixes: Vec<(String, Fix)>) -> Explained {
    Explained {
        title: title.into(),
        words: words.into(),
        fixes,
        stop: false,
    }
}

/// A git error in plain words, when it's one people run into.
pub(crate) fn explain_error(raw: &str, branch: &str, default: &str) -> Option<Explained> {
    let e = raw.to_lowercase();
    let has = |s: &str| e.contains(s);
    if has("gh013") || has("protected branch") || has("repository rule violations") {
        return Some(explained(
            "This branch only takes pull requests",
            format!("GitHub won't take commits sent straight to {branch}. Put them on a branch of their own and open a pull request."),
            vec![("Move my commits to a new branch".into(), Fix::MoveToBranch)],
        ));
    }
    if has("gh009") || has("push protection") || has("secret scanning") {
        return Some(Explained {
            stop: true,
            ..explained(
                "GitHub found a secret in what you sent",
                "One of the commits holds a password, token or key, so GitHub stopped the push. Take it out of the commit, then send again. If it's real, change it where it came from too.",
                vec![("Open Changes".into(), Fix::OpenChanges)],
            )
        });
    }
    if has("exceeds github's file size limit") || has("large files detected") || has("this exceeds")
    {
        return Some(explained(
            "A file is too big for GitHub",
            "GitHub takes files up to 100 MB. Take the file out of the commit (and add it to .gitignore), or store it with Git LFS.",
            vec![("Open Changes".into(), Fix::OpenChanges)],
        ));
    }
    if has("non-fast-forward") || has("fetch first") || (has("rejected") && has("behind")) {
        return Some(explained(
            "GitHub has commits you don't have yet",
            format!("Someone sent new commits to {branch} after you last synced. Bring them in first, then yours can go on top."),
            vec![("Sync now".into(), Fix::Sync)],
        ));
    }
    if has("would be overwritten")
        || has("please commit your changes or stash them")
        || has("your local changes")
    {
        return Some(explained(
            "Your uncommitted changes are in the way",
            "This step would write over files you're still changing. Set them aside for a moment (they come back with one click), or commit them first.",
            vec![("Set my changes aside".into(), Fix::SetAside)],
        ));
    }
    if has("terminal prompts disabled")
        || has("could not read username")
        || has("authentication failed")
        || has("permission denied (publickey")
        || has("invalid username or password")
    {
        return Some(explained(
            "Git couldn't sign in to GitHub",
            "Run `gh auth login` and then `gh auth setup-git` once in a terminal tab, so git can sign in without asking.",
            Vec::new(),
        ));
    }
    if has("could not resolve host")
        || has("network is unreachable")
        || has("connection timed out")
        || has("unable to access")
    {
        return Some(explained(
            "Can't reach GitHub",
            "This computer looks offline, or GitHub can't be reached right now. Your commits are safe here; send them when you're back online.",
            Vec::new(),
        ));
    }
    if has("index.lock") {
        return Some(explained(
            "Another git step is still running here",
            "git keeps a lock while it works, and one is still there: from an editor or terminal running git right now, or one that stopped halfway. Wait a moment and try again. If it stays, close other git tools; a leftover lock is the file .git/index.lock.",
            Vec::new(),
        ));
    }
    if has("not a git repository") {
        return Some(explained(
            "This folder isn't tracked by git",
            "There's no git history here yet, so there's nothing to commit or sync.",
            Vec::new(),
        ));
    }
    if has("no upstream") || has("has no upstream branch") {
        return Some(explained(
            "This branch isn't on GitHub yet",
            format!("{branch} only lives on this computer. Sync sends it up for the first time."),
            vec![("Send it now".into(), Fix::Sync)],
        ));
    }
    if has("refusing to merge unrelated histories") {
        return Some(explained(
            "These two have nothing in common",
            format!("The branches started separately, so git won't join them on its own. Usually the folder was linked to the wrong GitHub project. Check the remote, or start from a fresh copy of {default}."),
            Vec::new(),
        ));
    }
    None
}

/// A state that puzzles people, explained: on no branch, or out of step both ways.
pub(crate) fn explain_state(state: &git::RepoState, default: &str) -> Option<Explained> {
    // No branches at all is a folder with no commits yet, not a detached one.
    if state.branch.is_empty() && !state.branches.is_empty() {
        return Some(explained(
            "You're looking at an old commit, not a branch",
            "git calls this a detached HEAD. You can look around safely, but commits made here belong to no branch and are easy to lose. Start a branch to keep work, or go back.",
            vec![
                ("Start a branch here".into(), Fix::NewBranch),
                (format!("Back to {default}"), Fix::SwitchTo(default.to_string())),
            ],
        ));
    }
    if state.ahead > 0 && state.behind > 0 {
        return Some(explained(
            "You and GitHub both have new commits",
            format!(
                "{} has {} here that {} doesn't, and {} has {} you don't. Sync brings theirs in and puts yours on top, then sends them.",
                state.branch,
                count(state.ahead),
                state.host,
                state.host,
                count(state.behind)
            ),
            vec![("Sync now".into(), Fix::Sync)],
        ));
    }
    None
}

fn count(n: usize) -> String {
    match n {
        1 => "1 commit".into(),
        n => format!("{n} commits"),
    }
}

/// What the seatbelt says about a push, from what it found in the commits.
pub(crate) fn seatbelt_stop(found: &[(String, usize, Snag)], anyway: Fix) -> Explained {
    let n = found.len();
    Explained {
        title: match n {
            1 => "Wait: something in these commits shouldn't go out".into(),
            n => format!("Wait: {n} things in these commits shouldn't go out"),
        },
        words: "Once it's on GitHub, it stays in the history even if you delete it later. Take it out first: undo the commit, leave the line out in Changes, and commit again.".into(),
        fixes: match anyway {
            // Taking the last commit back only helps on the branch the folder is on.
            Fix::SendAnyway(_) => vec![
                ("Undo the last commit".into(), Fix::Uncommit),
                ("Send anyway".into(), anyway),
            ],
            _ => vec![("Send anyway".into(), anyway)],
        },
        stop: true,
    }
}

/// What the seatbelt says before sending straight to the main branch.
pub(crate) fn main_stop(branch: &str, ahead: usize, step: SyncStep) -> Explained {
    Explained {
        title: format!("Sending straight to {branch}?"),
        words: format!(
            "These {} will land on {branch} without a review. Teams usually put work on a branch and open a pull request.",
            count(ahead)
        ),
        fixes: vec![
            ("Move them to a new branch".into(), Fix::MoveToBranch),
            (format!("Send to {branch}"), Fix::SendAnyway(step)),
        ],
        stop: false,
    }
}

impl RunoApp {
    /// A git step failed: explain it when it's a known one, else show git's words.
    pub(crate) fn git_failed(&mut self, error: String) {
        let state = self
            .git_repo
            .as_ref()
            .map(|r| r.state.clone())
            .unwrap_or_default();
        let default = self.git_default_branch();
        match explain_error(&error, &state.branch, &default) {
            Some(explained) => {
                applog::warn("git", format!("{error} ({})", self.log_context()));
                self.git_banner = Some(Banner {
                    explained,
                    raw: error,
                    found: Vec::new(),
                });
                self.git_raw_open = false;
            }
            None => self.fail("git", error),
        }
    }

    /// The branch the remote treats as main.
    pub(crate) fn git_default_branch(&self) -> String {
        let branches = self
            .git_repo
            .as_ref()
            .map(|r| r.state.branches.clone())
            .unwrap_or_default();
        ["main", "master", "trunk"]
            .into_iter()
            .find(|b| branches.iter().any(|x| x == b))
            .unwrap_or("main")
            .to_string()
    }

    /// Run a fix from a card.
    fn run_fix(&mut self, fix: Fix, window: &mut Window, cx: &mut Context<Self>) {
        self.git_banner = None;
        match fix {
            Fix::Sync => self.git_sync(SyncStep::Both),
            Fix::SendAnyway(step) => {
                if let Some(dir) = self.git_repo.as_ref().map(|r| r.dir.clone()) {
                    self.git_main_ok.insert(dir);
                }
                self.git_belt_pass = true;
                self.git_sync(step);
            }
            Fix::SendBranchAnyway(_) | Fix::OpenStackAnyway(_) => {
                self.git_belt_pass = true;
                self.belt_resend(fix);
            }
            Fix::SetAside => {
                let branch = self
                    .git_repo
                    .as_ref()
                    .map(|r| r.state.branch.clone())
                    .unwrap_or_default();
                self.git_step("Setting aside…", move |dir| {
                    let when = chrono::Local::now().format("%b %-d, %H:%M");
                    git::stash_push(dir, &format!("Work on {branch}, {when}"))?;
                    Ok(Some((
                        "Set your changes aside. Try the step again; Undo brings them back.".into(),
                        Some(git::Undo::Unstash),
                    )))
                });
            }
            Fix::NewBranch => {
                self.open_branches();
                self.start_new_branch(window, cx);
            }
            Fix::SwitchTo(branch) => self.git_switch(branch),
            Fix::MoveToBranch => {
                self.git_step("Moving your commits…", |dir| {
                    let subject = git::last_commit(dir).map(|(s, _)| s).unwrap_or_default();
                    let wanted = match git::branch_slug(&subject) {
                        s if s.is_empty() => "my-changes".to_string(),
                        s => s.trim_start_matches("runo/").to_string(),
                    };
                    let name = git::unique_branch(&wanted, |n| git::has_branch(dir, n));
                    let undo = git::move_to_new_branch(dir, &name)?;
                    Ok(Some((
                        format!("Moved your commits to {name}. Sync sends it, then open a pull request."),
                        Some(undo),
                    )))
                });
            }
            Fix::Uncommit => {
                self.git_step("Taking the commit back…", |dir| {
                    let undo = git::uncommit(dir)?;
                    Ok(Some((
                        "Took the last commit back. Its changes are in Changes again.".into(),
                        Some(undo),
                    )))
                });
                self.open_review();
                self.git_folder_open = true;
            }
            Fix::OpenChanges => {
                self.open_review();
                self.git_folder_open = true;
                self.load_folder();
            }
        }
    }

    /// The card at the top of the Git place: an explained error or state, or the
    /// seatbelt's stop.
    pub(crate) fn git_banner_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let repo = self.git_repo.as_ref()?;
        // The clash page explains a stopped sync itself.
        if repo.clash.is_some() {
            return None;
        }
        let state = repo.state.clone();
        let (banner, closable) = match &self.git_banner {
            Some(b) => (b.clone(), true),
            None => {
                let explained = explain_state(&state, &self.git_default_branch())?;
                if self.git_state_seen.as_ref() == Some(&explained.title) {
                    return None;
                }
                (
                    Banner {
                        explained,
                        raw: String::new(),
                        found: Vec::new(),
                    },
                    false,
                )
            }
        };
        let ex = &banner.explained;
        let tone = if ex.stop { c_amber() } else { c_blue() };
        let title = ex.title.clone();
        let mut found = div().v_flex().gap(px(2.));
        for (path, line, snag) in banner.found.iter().take(8) {
            found = found.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .child(icon(
                        if snag.serious() {
                            IconName::ShieldAlert
                        } else {
                            IconName::TriangleAlert
                        },
                        12.,
                        c_amber(),
                    ))
                    .child(
                        div()
                            .flex_none()
                            .font_family("JetBrains Mono")
                            .text_size(px(11.5))
                            .child(match line {
                                0 => path.clone(),
                                l => format!("{path}:{l}"),
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(c_muted()))
                            .child(snag.words()),
                    ),
            );
        }
        let mut buttons = div().flex().flex_wrap().gap_2();
        for (i, (label, fix)) in ex.fixes.iter().enumerate() {
            let fix = fix.clone();
            let primary = i == 0;
            buttons = buttons.child(
                self.button(("git-fix", i), label.clone(), primary)
                    .h(px(28.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_fix(fix.clone(), window, cx);
                        cx.notify();
                    })),
            );
        }
        let has_raw = !banner.raw.is_empty();
        let raw_open = self.git_raw_open;
        Some(
            div()
                .absolute()
                .top(px(14.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("git-banner")
                        .occlude()
                        .w(px(560.))
                        .max_w_full()
                        .p_3()
                        .v_flex()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgba((tone << 8) | 0x66))
                        .bg(float(c_raised()))
                        .shadow_lg()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(icon(
                                    if ex.stop {
                                        IconName::ShieldAlert
                                    } else {
                                        IconName::Lightbulb
                                    },
                                    14.,
                                    tone,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(13.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(ex.title.clone()),
                                )
                                .child(
                                    div()
                                        .id("git-banner-close")
                                        .p(px(3.))
                                        .rounded_md()
                                        .cursor_pointer()
                                        .hover(|el| el.bg(tint(c_subtle())))
                                        .child(icon(IconName::X, 12., c_muted()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if closable {
                                                this.git_banner = None;
                                            } else {
                                                this.git_state_seen = Some(title.clone());
                                            }
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.5))
                                .line_height(px(18.))
                                .text_color(rgb(c_secondary()))
                                .child(ex.words.clone()),
                        )
                        .when(!banner.found.is_empty(), |el| el.child(found))
                        .when(!ex.fixes.is_empty(), |el| el.child(buttons))
                        .when(has_raw, |el| {
                            el.child(
                                div()
                                    .id("git-banner-raw")
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .cursor_pointer()
                                    .hover(|el| el.text_color(rgb(c_secondary())))
                                    .child(icon(
                                        if raw_open {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        },
                                        11.,
                                        c_muted(),
                                    ))
                                    .child("What git said")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.git_raw_open = !this.git_raw_open;
                                        cx.notify();
                                    })),
                            )
                            .when(raw_open, |el| {
                                el.child(
                                    div()
                                        .p_2()
                                        .rounded_md()
                                        .bg(tint(c_subtle()))
                                        .font_family("JetBrains Mono")
                                        .text_size(px(11.))
                                        .text_color(rgb(c_secondary()))
                                        .child(banner.raw.trim().to_string()),
                                )
                            })
                        }),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{explain_error, explain_state, Fix};

    #[test]
    fn git_errors_in_plain_words() {
        let rejected = "To github.com:a/b.git\n ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs";
        let ex = explain_error(rejected, "main", "main").unwrap();
        assert_eq!(ex.title, "GitHub has commits you don't have yet");
        assert_eq!(ex.fixes[0].1, Fix::Sync);
        let dirty = "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt";
        assert_eq!(
            explain_error(dirty, "main", "main").unwrap().fixes[0].1,
            Fix::SetAside
        );
        let rule = "remote: error: GH013: Repository rule violations found for refs/heads/main.";
        assert_eq!(
            explain_error(rule, "main", "main").unwrap().fixes[0].1,
            Fix::MoveToBranch
        );
        assert!(explain_error("fatal: something new", "main", "main").is_none());
        // No branches yet is a new folder, not a detached one.
        assert!(explain_state(&crate::git::RepoState::default(), "main").is_none());
        let detached = crate::git::RepoState {
            branches: vec!["main".into()],
            ..Default::default()
        };
        assert_eq!(
            explain_state(&detached, "main").unwrap().fixes[1].1,
            Fix::SwitchTo("main".into())
        );
        let diverged = crate::git::RepoState {
            branch: "main".into(),
            host: "GitHub".into(),
            ahead: 2,
            behind: 1,
            ..Default::default()
        };
        assert!(explain_state(&diverged, "main")
            .unwrap()
            .words
            .contains("2 commits here"));
    }
}
