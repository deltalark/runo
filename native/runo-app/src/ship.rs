//! Landing a session's work: staging and commits, bringing the branch up to date with
//! its base and merging it back, pull requests with their checks and reviews, races,
//! and cleaning up worktrees and branches afterwards. Git runs off the UI thread.

use super::*;

/// Work finished on a background thread, applied on the UI thread.
pub(crate) type UiJob = Box<dyn FnOnce(&mut RunoApp, &mut Context<RunoApp>) + Send>;

/// How the selected session's branch stands against the branch it lands in.
#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct BranchInfo {
    /// The base branch, such as `main`.
    pub base: String,
    /// What it is compared with: `origin/main`, or `main` when that has unpushed work.
    pub base_ref: String,
    /// Commits the base has that the branch lacks, and the other way round.
    pub behind: usize,
    pub ahead: usize,
    /// A rebase or merge stopped on conflicts: "rebase" or "merge".
    pub in_progress: Option<&'static str>,
    pub conflicts: Vec<String>,
}

/// A git step waiting for the person to confirm it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Confirm {
    /// Keep this racer: stop the others and delete their worktrees and branches.
    Keep(u64),
    /// Merge the session's branch into its base branch.
    Merge(u64),
    /// Delete the session's worktree and branch, then archive it.
    CleanUp(u64),
    /// Merge the session's pull request on GitHub.
    MergePr(u64),
    /// Turn on auto-merge for the session's pull request, so GitHub merges it once its
    /// checks pass.
    AutoMergePr(u64),
    /// Post the review the session's agent wrote to its pull request.
    PostReview(u64),
    /// Push the session's branch although the seatbelt found something in it.
    PushAnyway(u64),
}

/// The Create pull request form.
pub(crate) struct PrForm {
    pub session: u64,
    /// The session works on the base branch itself, so it needs a branch of its own.
    pub needs_branch: bool,
    pub running: bool,
    pub error: Option<String>,
    /// The seatbelt has looked at what the pull request would send, or was told to let
    /// it through.
    pub belt_ok: bool,
}

/// A turn asked for while the session's worktree is still being made.
pub(crate) struct Deferred {
    pub prompt: String,
    pub images: Vec<String>,
    pub opts: AgentOptions,
    /// Playbook name and note to tag the message with once it is sent.
    pub tag: Option<(String, String)>,
}

/// What `refresh` reads from git for the selected session.
#[derive(Default)]
pub(crate) struct Scan {
    pub is_git: bool,
    pub changes: Vec<git::Change>,
    /// Files, added and removed lines not committed yet, for the sidebar.
    pub uncommitted: (usize, usize, usize),
    pub files: Vec<String>,
    pub sync: git::Sync,
    pub branch: BranchInfo,
    /// Each racer's work against the base, committed or not.
    pub race: HashMap<u64, (usize, usize, usize)>,
    /// What diffs compare with: HEAD, or where the branch left its base.
    pub against: String,
    /// The file whose diff was read, and the diff.
    pub diff: Option<(String, String)>,
    pub all_diffs: Vec<(String, String)>,
}

/// What the Changes panel should show.
pub(crate) struct ScanRequest {
    pub dir: String,
    pub base: String,
    /// Compare with the base branch instead of the last commit.
    pub since_base: bool,
    pub selected_file: Option<String>,
    pub whole: bool,
    pub all: bool,
    pub racers: Vec<(u64, String)>,
}

/// Read a session folder's git state. Slow on big repositories, so call it off the UI thread.
pub(crate) fn scan(req: ScanRequest) -> Scan {
    if !git::is_repo(&req.dir) {
        return Scan {
            files: plain_files(&req.dir),
            ..Default::default()
        };
    }
    let uncommitted_changes = git::changes(&req.dir);
    let mut out = Scan {
        is_git: true,
        uncommitted: git::totals(&uncommitted_changes),
        files: git::tracked_files(&req.dir),
        sync: git::sync_state(&req.dir),
        against: "HEAD".into(),
        ..Default::default()
    };
    let base = if req.base.is_empty() {
        git::default_branch(&req.dir)
    } else {
        req.base.clone()
    };
    if !base.is_empty() {
        let base_ref = git::base_ref(&req.dir, &base);
        out.branch = BranchInfo {
            behind: git::commits_ahead(&req.dir, &base_ref, "HEAD"),
            ahead: git::commits_ahead(&req.dir, "HEAD", &base_ref),
            base,
            base_ref,
            in_progress: git::in_progress(&req.dir),
            conflicts: git::conflicted(&req.dir),
        };
    }
    let since = req.since_base && !out.branch.base_ref.is_empty();
    out.changes = match since
        .then(|| git::merge_base(&req.dir, &out.branch.base_ref))
        .flatten()
    {
        Some(start) => {
            out.against = start;
            git::changes_since(&req.dir, &out.branch.base_ref)
        }
        None => uncommitted_changes,
    };
    for (id, dir) in req.racers {
        let base_ref = git::base_ref(&dir, &out.branch.base);
        out.race
            .insert(id, git::totals(&git::changes_since(&dir, &base_ref)));
    }
    let file = req
        .selected_file
        .filter(|f| out.changes.iter().any(|c| c.path == *f))
        .or_else(|| out.changes.first().map(|c| c.path.clone()));
    if let Some(file) = file {
        let diff = git::file_diff_context(&req.dir, &file, req.whole, &out.against);
        out.diff = Some((file, diff));
    }
    if req.all {
        out.all_diffs = out
            .changes
            .iter()
            .map(|c| {
                let diff = git::file_diff_context(&req.dir, &c.path, req.whole, &out.against);
                (c.path.clone(), diff)
            })
            .collect();
    }
    out
}

/// Make a session's worktree: fetch, branch from the up-to-date base, check out a pull
/// request when asked, and copy `.worktreeinclude` files. Returns the folder, the branch
/// and lines for the session's Terminal.
pub(crate) fn prepare_worktree(
    repo: &str,
    branch: &str,
    base: &str,
    pr: Option<u64>,
) -> Result<(String, String, Vec<String>), String> {
    let mut log = Vec::new();
    if let Err(error) = git::fetch(repo) {
        log.push(format!(
            "Could not fetch, so the branch starts from the local {base}: {}",
            error.lines().next().unwrap_or("")
        ));
    }
    let start = git::base_ref(repo, base);
    let (dir, branch) = match pr {
        Some(number) => {
            let dir = git::add_detached_worktree(repo, branch, &start)?;
            let checkout = |extra: &[&str]| {
                let mut command = std::process::Command::new("gh");
                command
                    .args(["pr", "checkout", &number.to_string()])
                    .args(extra)
                    .current_dir(&dir)
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .stdin(std::process::Stdio::null());
                match command.output() {
                    Ok(out) if out.status.success() => Ok(()),
                    Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                    Err(error) => Err(format!("could not run gh: {error}")),
                }
            };
            // The PR's own branch name, unless it is checked out somewhere already.
            if let Err(first) = checkout(&[]) {
                if let Err(error) = checkout(&["--branch", branch]) {
                    let _ = git::remove_worktree(repo, &dir);
                    return Err(format!(
                        "Could not check out pull request #{number}: {}",
                        if error.is_empty() { first } else { error }
                    ));
                }
            }
            log.push(format!("Checked out pull request #{number}."));
            let checked = git::current_branch(&dir);
            (dir, checked)
        }
        None => (
            git::add_worktree(repo, branch, Some(&start))?,
            branch.to_string(),
        ),
    };
    match git::copy_included(repo, &dir) {
        Ok(copied) if !copied.is_empty() => log.push(format!(
            "Copied {} file{} from .worktreeinclude: {}",
            copied.len(),
            if copied.len() == 1 { "" } else { "s" },
            copied.join(", ")
        )),
        Ok(_) => {}
        Err(error) => log.push(format!("Could not copy .worktreeinclude files: {error}")),
    }
    Ok((dir, branch, log))
}

/// What the seatbelt would stop in the commits `dir` hasn't sent yet.
fn belt_stops(dir: &str) -> Vec<(String, usize, seatbelt::Snag)> {
    seatbelt::scan_patch(&git::outgoing_patch(dir))
        .into_iter()
        .filter(|(_, _, s)| s.serious())
        .collect()
}

/// Push the branch, then open a pull request or find the one already open. Returns its
/// URL and whether it existed before.
fn open_pr(
    dir: &str,
    new_branch: Option<&str>,
    title: &str,
    body: &str,
    base: &str,
) -> Result<(String, bool), String> {
    if let Some(name) = new_branch {
        git::switch_new(dir, name)?;
    }
    let remote = git::remote_name(dir);
    git::push(dir, &remote)?;
    let gh = |args: &[&str]| -> Result<String, String> {
        let out = std::process::Command::new("gh")
            .args(args)
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("The GitHub CLI (gh) is needed for pull requests: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    let branch = git::current_branch(dir);
    if let Ok(json) = gh(&["pr", "view", &branch, "--json", "url,state"]) {
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
        if v["state"] == "OPEN" {
            if let Some(url) = v["url"].as_str() {
                return Ok((url.to_string(), true));
            }
        }
    }
    let mut args = vec!["pr", "create", "--head", &branch, "--title", title];
    args.extend(["--body", body]);
    if !base.is_empty() {
        args.extend(["--base", base]);
    }
    let out = gh(&args)?;
    let url = out
        .lines()
        .rev()
        .find(|l| l.starts_with("https://"))
        .unwrap_or(out.trim())
        .to_string();
    Ok((url, false))
}

/// The prompt that sends a stopped rebase or merge to the agent.
pub(crate) fn conflict_prompt(kind: &str, base: &str, files: &[String]) -> String {
    let finish = if kind == "rebase" {
        "`GIT_EDITOR=true git rebase --continue`, repeating for each commit until the rebase is done"
    } else {
        "`git commit --no-edit` to finish the merge"
    };
    format!(
        "Bringing this branch up to date with {base} stopped on conflicts ({kind}) in:\n{}\n\nResolve each conflict so both sides' intent survives, `git add` the files, then run {finish}. Don't abort. Run the build or tests afterwards if you can.",
        files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// The prompt that asks the agent to fix a pull request's failing checks.
pub(crate) fn ci_prompt(pr: &github::PrStatus) -> String {
    let logs = match pr.failed_run {
        Some(run) => format!("`gh run view {run} --log-failed`"),
        None => format!(
            "`gh pr checks {}` to find the failing run, then `gh run view <run id> --log-failed`",
            pr.number
        ),
    };
    format!(
        "CI is failing on pull request #{} ({}): {}. Read the failure logs with {logs}, fix the cause, run the same checks locally if you can, and commit the fix.",
        pr.number,
        pr.url,
        pr.failed.join(", ")
    )
}

/// The prompt that asks the agent to address review feedback.
pub(crate) fn review_prompt(pr: &github::PrStatus, feedback: &[String]) -> String {
    if feedback.is_empty() {
        return format!(
            "Address the review comments on pull request #{} ({}). Read them with `gh pr view {} --comments` and `gh api repos/{{owner}}/{{repo}}/pulls/{}/comments`, make the changes, and commit.",
            pr.number, pr.url, pr.number, pr.number
        );
    }
    let mut out = format!(
        "Address this review feedback on pull request #{} ({}). Make the changes, reply in your summary where you disagree, and commit.\n",
        pr.number, pr.url
    );
    for (i, item) in feedback.iter().enumerate() {
        out.push_str(&format!("\n{}. {item}", i + 1));
    }
    out
}

impl RunoApp {
    /// Run `work` on a background thread, then `done` with its result on the UI thread.
    pub(crate) fn bg<R: Send + 'static>(
        &self,
        work: impl FnOnce() -> R + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + Send + 'static,
    ) {
        let tx = self.ui_tx.clone();
        std::thread::spawn(move || {
            let result = work();
            let _ = tx.send(Box::new(
                move |this: &mut RunoApp, cx: &mut Context<RunoApp>| done(this, result, cx),
            ));
        });
    }

    /// Put a background refresh's results on screen.
    pub(crate) fn apply_scan(&mut self, id: u64, scan: Scan, asked: Option<String>) {
        self.dir_is_git = scan.is_git;
        self.stats.insert(id, scan.uncommitted);
        self.changes = scan.changes;
        self.files = scan.files;
        self.sync = scan.sync;
        self.branch_info = scan.branch;
        self.race_stats = scan.race;
        self.diff_against = scan.against;
        // A file picked while the scan ran wins over the scan's choice.
        if self.selected_file == asked {
            match scan.diff {
                Some((file, diff)) => {
                    self.selected_file = Some(file);
                    self.diff = diff;
                }
                None => {
                    self.selected_file = None;
                    self.diff.clear();
                }
            }
        }
        if self.diff_all {
            self.all_diffs = scan.all_diffs;
        }
        if let Some(session) = self.state.session_mut(id) {
            if session.base.is_empty() && !self.branch_info.base.is_empty() {
                session.base = self.branch_info.base.clone();
            }
        }
    }

    /// Whether the session's agent is running a turn now.
    pub(crate) fn turn_running(&self, id: u64) -> bool {
        self.handles.contains_key(&Target::Session(id))
    }

    /// Mark a git step as running for a session, or say why it can't start. Returns
    /// whether it may start.
    pub(crate) fn begin(&mut self, id: u64, label: &str) -> bool {
        if let Some(running) = self.git_busy.get(&id) {
            self.fail("git", format!("{running} Wait for it to finish."));
            return false;
        }
        self.git_busy.insert(id, label.to_string());
        true
    }

    /// Send a prompt to the session's agent, or queue it when a turn is running.
    pub(crate) fn send_or_queue(&mut self, id: u64, prompt: String, cx: &mut Context<Self>) {
        if self.turn_running(id) || self.preparing.contains_key(&id) {
            if let Some(s) = self.state.session_mut(id) {
                s.queue.push(crate::store::Queued::new(prompt, Vec::new()));
            }
            self.info = Some((
                "Queued for when the agent finishes this turn.".into(),
                std::time::Instant::now(),
            ));
            self.save();
            return;
        }
        let Some(provider) = self.state.session(id).map(|s| s.provider.clone()) else {
            return;
        };
        let opts = cx.global::<AppSettings>().agent(&provider);
        self.start_turn(id, prompt, Vec::new(), &opts);
    }

    /// Create an isolated session's worktree in the background. Turns asked for in the
    /// meantime wait for it.
    pub(crate) fn make_worktree(&mut self, id: u64, repo: String, branch: String, pr: Option<u64>) {
        let base = self
            .state
            .session(id)
            .map(|s| s.base.clone())
            .unwrap_or_default();
        self.preparing.insert(id, Vec::new());
        self.terminal.entry(id).or_default().push(format!(
            "Creating the worktree for {branch} from the latest {base}…"
        ));
        self.bg(
            move || prepare_worktree(&repo, &branch, &base, pr),
            move |this, result, cx| this.worktree_ready(id, result, cx),
        );
    }

    pub(crate) fn worktree_ready(
        &mut self,
        id: u64,
        result: Result<(String, String, Vec<String>), String>,
        cx: &mut Context<Self>,
    ) {
        let waiting = self.preparing.remove(&id).unwrap_or_default();
        match result {
            Ok((dir, branch, log)) => {
                self.terminal.entry(id).or_default().extend(log);
                if let Some(s) = self.state.session_mut(id) {
                    s.dir = dir;
                    s.branch = branch;
                }
                self.save();
                if let Some(task) = self.pending_start.remove(&id) {
                    self.launch(id, task, cx);
                }
                for turn in waiting {
                    let busy = self.turn_running(id) || self.pending_start.contains_key(&id);
                    if busy {
                        if let Some(s) = self.state.session_mut(id) {
                            s.queue
                                .push(crate::store::Queued::new(turn.prompt, turn.images));
                        }
                        continue;
                    }
                    self.start_turn(id, turn.prompt, turn.images, &turn.opts);
                    if let Some((name, note)) = turn.tag {
                        self.tag_playbook(id, &name, &note);
                    }
                }
                self.measure(id);
                if self.selected == Some(id) {
                    self.refresh();
                }
            }
            Err(error) => {
                self.pending_start.remove(&id);
                crate::applog::warn(
                    "git",
                    format!(
                        "{}: could not create the worktree: {}",
                        self.session_label(id),
                        crate::applog::gist(&error)
                    ),
                );
                if let Some(s) = self.state.session_mut(id) {
                    s.status = Status::Failed;
                    s.messages.push(Message::new(
                        Role::System,
                        format!("Could not create the worktree: {error}"),
                    ));
                }
                self.save();
            }
        }
    }

    /// Stage or unstage one changed file.
    pub(crate) fn toggle_stage(&mut self, path: &str, staged: bool) {
        let Some(dir) = self.current().map(|s| s.dir.clone()) else {
            return;
        };
        let path = path.to_string();
        self.bg(
            move || {
                if staged {
                    git::unstage(&dir, &path)
                } else {
                    git::stage(&dir, &path)
                }
            },
            |this, result, _| {
                if let Err(error) = result {
                    this.fail("git", format!("Could not change staging: {error}"));
                }
                this.refresh();
            },
        );
    }

    /// Commit the session folder's staged changes, or all of them. An empty message
    /// falls back to the session title.
    pub(crate) fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, dir, title)) = self
            .current()
            .map(|s| (s.id, s.dir.clone(), s.title.clone()))
        else {
            return;
        };
        if self.turn_running(id) {
            self.notice =
                Some("The agent is still working. Commit when its turn has finished.".into());
            return;
        }
        let typed = self.commit_msg.read(cx).value().trim().to_string();
        let mut message = if typed.is_empty() {
            title
        } else {
            typed.clone()
        };
        if cx.global::<AppSettings>().attribution {
            if let Some(name) = self.current().map(|s| agent::provider(&s.provider).name) {
                message.push_str(&format!("\n\nMade with {name} in Runo"));
            }
        }
        if !self.begin(id, "Committing.") {
            return;
        }
        self.commit_msg
            .update(cx, |input, cx| input.set_value("", window, cx));
        let text = message.clone();
        self.bg(
            move || git::commit(&dir, &text),
            move |this, result, _| {
                this.git_busy.remove(&id);
                match result {
                    Ok(hash) => {
                        this.notice = None;
                        let first = message.lines().next().unwrap_or_default().to_string();
                        this.info = Some((
                            format!("Committed {hash}: {first}"),
                            std::time::Instant::now(),
                        ));
                        this.state.log(id, format!("Committed {hash}: {first}"));
                        this.save();
                    }
                    Err(error) => {
                        this.fail("git", format!("Commit failed: {error}"));
                        // Put the message back so it isn't lost.
                        if !typed.is_empty() {
                            this.commit_suggestion = Some(typed);
                        }
                    }
                }
                this.refresh();
            },
        );
    }

    /// Whether the session works directly on its base branch (a project-folder session on
    /// main), where pushing would push main itself.
    pub(crate) fn on_base_branch(&self, session: &Session) -> bool {
        let base = if session.base.is_empty() {
            &self.branch_info.base
        } else {
            &session.base
        };
        !base.is_empty() && session.branch == *base && !session.isolated
    }

    /// Push the open session's branch to its remote.
    pub(crate) fn push(&mut self) {
        if let Some(id) = self.current().map(|s| s.id) {
            self.push_session(id);
        }
    }

    /// Push a session's branch, once the seatbelt has looked at the commits it sends.
    fn push_session(&mut self, id: u64) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if self.on_base_branch(session) {
            self.fail("git", format!(
                "This session works on {} itself. Use Create PR to move the work to a branch of its own first.",
                session.branch
            ));
            return;
        }
        let dir = session.dir.clone();
        let remote = self.sync.remote.clone();
        if remote.is_empty() {
            self.fail("git", "This repository has no remote to push to.".into());
            return;
        }
        if self.belt_pass.take() != Some(id) {
            let scan = dir.clone();
            self.bg(
                move || belt_stops(&scan),
                move |this, serious, cx| {
                    if serious.is_empty() {
                        this.belt_pass = Some(id);
                        this.push_session(id);
                    } else {
                        this.belt_found = serious;
                        this.confirm_git = Some(Confirm::PushAnyway(id));
                    }
                    cx.notify();
                },
            );
            return;
        }
        if !self.begin(id, "Pushing.") {
            return;
        }
        self.bg(
            move || git::push(&dir, &remote),
            move |this, result, _| {
                this.git_busy.remove(&id);
                match result {
                    Ok(_) => {
                        this.state.log(id, "Pushed the branch");
                        this.info = Some(("Pushed.".into(), std::time::Instant::now()));
                        this.save();
                    }
                    Err(error) => this.fail("git", format!("Push failed: {error}")),
                }
                this.refresh();
            },
        );
    }

    /// Open the Create pull request form, filled from the session and its commits.
    pub(crate) fn open_pr_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.current() else {
            return;
        };
        let id = session.id;
        let needs_branch = self.on_base_branch(session);
        let base = if session.base.is_empty() {
            self.branch_info.base.clone()
        } else {
            session.base.clone()
        };
        let range = format!("{}..HEAD", self.branch_info.base_ref);
        let log = std::process::Command::new("git")
            .args(["-C", &session.dir, "log", "--format=%s", &range])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let subjects: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
        let title = match subjects.as_slice() {
            [one] => one.to_string(),
            _ => session.title.clone(),
        };
        let body = if subjects.len() > 1 {
            subjects
                .iter()
                .rev()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        };
        // A reviewer's guide written for this branch leads the description.
        let body = match self.git_guides.get(&session.branch) {
            Some(Some(guide)) if body.is_empty() => guide.clone(),
            Some(Some(guide)) => format!("{guide}\n\n## Commits\n{body}"),
            _ => body,
        };
        let branch = format!("runo/session-{id}");
        self.pr_title
            .update(cx, |i, cx| i.set_value(title, window, cx));
        self.pr_body
            .update(cx, |i, cx| i.set_value(body, window, cx));
        self.pr_base
            .update(cx, |i, cx| i.set_value(base, window, cx));
        self.pr_branch
            .update(cx, |i, cx| i.set_value(branch, window, cx));
        self.pr_form = Some(PrForm {
            session: id,
            needs_branch,
            running: false,
            error: None,
            belt_ok: false,
        });
        self.right_tab = RightTab::Changes;
        self.show_panel = true;
    }

    fn submit_pr(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.pr_form.as_mut().filter(|f| !f.running) else {
            return;
        };
        let id = form.session;
        let title = self.pr_title.read(cx).value().trim().to_string();
        let body = self.pr_body.read(cx).value().to_string();
        let base = self.pr_base.read(cx).value().trim().to_string();
        let branch = git::sanitize_branch(&self.pr_branch.read(cx).value());
        if title.is_empty() {
            form.error = Some("Give the pull request a title.".into());
            return;
        }
        if form.needs_branch && (branch.is_empty() || branch == base) {
            form.error = Some(format!(
                "Name a new branch; {base} itself can't be pushed from here."
            ));
            return;
        }
        let new_branch = form.needs_branch.then_some(branch);
        form.running = true;
        form.error = None;
        let Some(dir) = self.state.session(id).map(|s| s.dir.clone()) else {
            return;
        };
        if !form.belt_ok {
            self.bg(
                move || belt_stops(&dir),
                move |this, serious, cx| {
                    let Some(form) = this.pr_form.as_mut().filter(|f| f.session == id) else {
                        return;
                    };
                    form.belt_ok = true;
                    if serious.is_empty() {
                        this.submit_pr(cx);
                    } else {
                        form.running = false;
                        form.error = Some(format!(
                            "Wait: {} Once it's on GitHub it stays in the history. Take it out and commit again, or press Create PR again to send it anyway.",
                            seatbelt::summary(&serious)
                        ));
                    }
                    cx.notify();
                },
            );
            return;
        }
        self.bg(
            move || {
                let result = open_pr(&dir, new_branch.as_deref(), &title, &body, &base);
                (result, git::current_branch(&dir))
            },
            move |this, (result, branch), _| {
                match result {
                    Ok((url, existed)) => {
                        this.pr_form = None;
                        if let Some(s) = this.state.session_mut(id) {
                            s.pr_url = url.clone();
                            s.branch = branch;
                        }
                        this.state.log(
                            id,
                            if existed {
                                format!("Pull request already open: {url}")
                            } else {
                                format!("Opened pull request {url}")
                            },
                        );
                        this.info = Some((
                            if existed {
                                "A pull request was already open for this branch.".into()
                            } else {
                                "Opened the pull request.".into()
                            },
                            std::time::Instant::now(),
                        ));
                        this.save();
                        this.load_pr_status(id);
                    }
                    Err(error) => {
                        crate::applog::warn(
                            "gh",
                            format!(
                                "{}: gh pr create failed: {}",
                                this.session_label(id),
                                crate::applog::gist(&error)
                            ),
                        );
                        if let Some(form) = this.pr_form.as_mut() {
                            form.running = false;
                            form.error = Some(error);
                        }
                        // A new branch may have been made before the push failed.
                        if let Some(s) = this.state.session_mut(id) {
                            s.branch = branch;
                        }
                    }
                }
                this.refresh();
            },
        );
    }

    /// Look up the pull request of a session's branch: state, checks and review.
    pub(crate) fn load_pr_status(&mut self, id: u64) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        if session.branch.is_empty() || session.branch == "local folder" {
            return;
        }
        let (dir, branch) = (session.dir.clone(), session.branch.clone());
        let known = session.pr_url.clone();
        if !std::path::Path::new(&dir).is_dir() || self.pr_loading.contains(&id) {
            return;
        }
        self.pr_loading.insert(id);
        self.bg(
            move || {
                let view = |which: &str| {
                    std::process::Command::new("gh")
                        .args([
                            "pr",
                            "view",
                            which,
                            "--json",
                            "number,url,title,state,isDraft,reviewDecision,statusCheckRollup,reviews,comments,mergeStateStatus,autoMergeRequest",
                        ])
                        .current_dir(&dir)
                        .stdin(std::process::Stdio::null())
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .and_then(|o| github::parse_pr_status(&String::from_utf8_lossy(&o.stdout)))
                };
                // A reviewed PR's branch may be checked out under another name.
                let mut status = view(&branch).or_else(|| {
                    (!known.is_empty()).then(|| view(&known)).flatten()
                })?;
                status.draft_review = std::fs::read_to_string(
                    std::path::Path::new(&dir).join(github::REVIEW_FILE),
                )
                .ok()
                .and_then(|json| github::parse_review_draft(&json));
                Some(status)
            },
            move |this, status, _| {
                this.pr_loading.remove(&id);
                match status {
                    Some(status) => {
                        if let Some(s) = this.state.session_mut(id) {
                            if s.pr_url != status.url {
                                s.pr_url = status.url.clone();
                            }
                        }
                        if status.state == "OPEN" {
                            this.load_merge_methods(id);
                        }
                        this.pr_status.insert(id, status);
                    }
                    None => {
                        this.pr_status.remove(&id);
                    }
                }
            },
        );
    }

    /// Read which merge methods the GitHub repository of the session's project allows,
    /// once per project for the app's run.
    pub(crate) fn load_merge_methods(&mut self, id: u64) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let (project, dir) = (session.project_id, session.dir.clone());
        if self.merge_methods.contains_key(&project) || !self.merge_methods_loading.insert(project)
        {
            return;
        }
        self.bg(
            move || {
                std::process::Command::new("gh")
                    .args([
                        "repo",
                        "view",
                        "--json",
                        "mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed",
                    ])
                    .current_dir(&dir)
                    .stdin(std::process::Stdio::null())
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .and_then(|o| github::parse_merge_methods(&String::from_utf8_lossy(&o.stdout)))
            },
            move |this, methods, _| {
                this.merge_methods_loading.remove(&project);
                if let Some(methods) = methods {
                    this.merge_methods.insert(project, methods);
                }
            },
        );
    }

    /// The merge methods the session's repository allows, once read, and the one to use:
    /// the project's remembered choice, else squash, else a merge commit, else rebase.
    pub(crate) fn merge_choice(
        &self,
        id: u64,
    ) -> Option<(github::MergeMethod, &[github::MergeMethod])> {
        let project_id = self.state.session(id)?.project_id;
        let allowed = self.merge_methods.get(&project_id)?;
        let remembered = self
            .state
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .map_or("", |p| p.merge_method.as_str());
        Some((github::default_merge_method(allowed, remembered)?, allowed))
    }

    /// Remember how the session's project merges pull requests.
    fn pick_merge_method(&mut self, id: u64, method: github::MergeMethod) {
        let Some(project_id) = self.state.session(id).map(|s| s.project_id) else {
            return;
        };
        if let Some(p) = self.state.projects.iter_mut().find(|p| p.id == project_id) {
            p.merge_method = method.key().to_string();
            self.save();
        }
    }

    /// Merge the session's pull request on GitHub with the picked method, or with `auto`
    /// turn on auto-merge so GitHub merges it once its checks pass.
    pub(crate) fn merge_pr(&mut self, id: u64, auto: bool) {
        let (Some(pr), Some(dir)) = (
            self.pr_status.get(&id).cloned(),
            self.state.session(id).map(|s| s.dir.clone()),
        ) else {
            return;
        };
        let Some((method, _)) = self.merge_choice(id) else {
            self.fail(
                "git",
                "Could not merge the pull request: Runo could not read which merge methods the repository allows (gh repo view).".into(),
            );
            self.load_merge_methods(id);
            return;
        };
        let doing = if auto {
            format!("Turning on auto-merge for pull request #{}.", pr.number)
        } else {
            format!("Merging pull request #{}.", pr.number)
        };
        if !self.begin(id, &doing) {
            return;
        }
        let number = pr.number.to_string();
        self.bg(
            move || {
                let mut args = vec!["pr", "merge", &number, method.flag()];
                if auto {
                    args.push("--auto");
                }
                let out = std::process::Command::new("gh")
                    .args(&args)
                    .current_dir(&dir)
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .stdin(std::process::Stdio::null())
                    .output()
                    .map_err(|e| format!("could not run gh: {e}"))?;
                if out.status.success() {
                    Ok(())
                } else {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    Err(if err.is_empty() {
                        String::from_utf8_lossy(&out.stdout).trim().to_string()
                    } else {
                        err
                    })
                }
            },
            move |this, result, _| {
                this.git_busy.remove(&id);
                match result {
                    Ok(()) if auto => {
                        this.state.log(
                            id,
                            format!("Turned on auto-merge for pull request #{}", pr.number),
                        );
                        this.info = Some((
                            format!(
                                "Auto-merge is on: GitHub merges #{} once its checks pass.",
                                pr.number
                            ),
                            std::time::Instant::now(),
                        ));
                        this.save();
                    }
                    Ok(()) => {
                        this.state
                            .log(id, format!("Merged pull request #{}", pr.number));
                        this.info = Some((
                            format!("Merged pull request #{}.", pr.number),
                            std::time::Instant::now(),
                        ));
                        if this.state.session(id).is_some_and(|s| s.isolated) {
                            this.confirm_git = Some(Confirm::CleanUp(id));
                        }
                        this.save();
                    }
                    Err(error) if auto => {
                        this.fail("git", format!("Could not turn on auto-merge: {error}"))
                    }
                    Err(error) => {
                        this.fail("git", format!("Could not merge the pull request: {error}"))
                    }
                }
                this.load_pr_status(id);
            },
        );
    }

    /// Ask the agent to fix the failing checks of the session's pull request.
    pub(crate) fn fix_ci(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(pr) = self.pr_status.get(&id).cloned() else {
            return;
        };
        self.state
            .log(id, format!("Asked the agent to fix CI on #{}", pr.number));
        self.send_or_queue(id, ci_prompt(&pr), cx);
    }

    /// Fetch the review comments of the session's pull request and send them to the agent.
    pub(crate) fn address_review(&mut self, id: u64) {
        let (Some(pr), Some(dir)) = (
            self.pr_status.get(&id).cloned(),
            self.state.session(id).map(|s| s.dir.clone()),
        ) else {
            return;
        };
        if !self.begin(id, "Reading the review comments.") {
            return;
        }
        let number = pr.number.to_string();
        self.bg(
            move || {
                let run = |args: &[&str]| {
                    std::process::Command::new("gh")
                        .args(args)
                        .current_dir(&dir)
                        .stdin(std::process::Stdio::null())
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                        .unwrap_or_default()
                };
                let view = run(&["pr", "view", &number, "--json", "reviews,comments"]);
                let lines = run(&[
                    "api",
                    &format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments"),
                ]);
                github::review_feedback(&view, &lines)
            },
            move |this, feedback, cx| {
                this.git_busy.remove(&id);
                this.state.log(
                    id,
                    format!("Asked the agent to address the review on #{}", pr.number),
                );
                this.send_or_queue(id, review_prompt(&pr, &feedback), cx);
            },
        );
    }

    /// Fetch, then rebase (or merge) the session branch onto the latest base branch.
    /// Conflicts leave the worktree mid-rebase, for the agent or the person to resolve.
    pub(crate) fn update_from_base(&mut self, rebase: bool) {
        let Some(session) = self.current() else {
            return;
        };
        let (id, dir) = (session.id, session.dir.clone());
        if self.turn_running(id) {
            self.fail(
                "git",
                "Wait for the agent to finish its turn before updating.".into(),
            );
            return;
        }
        let base = self.branch_info.base.clone();
        if base.is_empty() || !self.begin(id, &format!("Updating from {base}.")) {
            return;
        }
        self.bg(
            move || {
                let fetched = git::fetch(&dir);
                let onto = git::base_ref(&dir, &base);
                (fetched, git::update_from(&dir, &onto, rebase), base, onto)
            },
            move |this, (fetched, result, base, onto), _| {
                this.git_busy.remove(&id);
                let stale = match fetched {
                    Err(e) => format!(" (could not fetch, so this used the local {base}: {})", e.lines().next().unwrap_or("")),
                    Ok(()) => String::new(),
                };
                match result {
                    Ok(git::Update::UpToDate) => {
                        this.info = Some((
                            format!("Already up to date with {onto}{stale}."),
                            std::time::Instant::now(),
                        ))
                    }
                    Ok(git::Update::Updated) => {
                        this.state.log(id, format!("Updated from {onto}"));
                        this.info = Some((
                            format!("Updated from {onto}{stale}."),
                            std::time::Instant::now(),
                        ));
                    }
                    Ok(git::Update::Conflicts(files)) => {
                        this.state.log(id, format!("Update from {onto} stopped on conflicts"));
                        this.fail("git", format!(
                            "Updating stopped on conflicts in {} file{}. Resolve them, or send them to the agent.",
                            files.len(),
                            if files.len() == 1 { "" } else { "s" }
                        ));
                    }
                    Err(error) => this.fail("git", format!("Could not update: {error}")),
                }
                this.save();
                this.measure(id);
                this.refresh();
            },
        );
    }

    /// Ask the agent to finish a rebase or merge that stopped on conflicts.
    pub(crate) fn resolve_with_agent(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let info = &self.branch_info;
        let Some(kind) = info.in_progress else {
            return;
        };
        let prompt = conflict_prompt(kind, &info.base_ref, &info.conflicts);
        self.state.log(id, "Sent the conflicts to the agent");
        self.send_or_queue(id, prompt, cx);
    }

    /// Continue or abort a stopped rebase or merge.
    pub(crate) fn finish_update(&mut self, abort: bool) {
        let Some((id, dir)) = self.current().map(|s| (s.id, s.dir.clone())) else {
            return;
        };
        if !self.begin(id, if abort { "Aborting." } else { "Continuing." }) {
            return;
        }
        self.bg(
            move || {
                if abort {
                    git::abort_update(&dir).map(|_| git::Update::Updated)
                } else {
                    git::continue_update(&dir)
                }
            },
            move |this, result, _| {
                this.git_busy.remove(&id);
                match result {
                    Ok(git::Update::Conflicts(files)) => {
                        this.fail("git", format!("Still conflicted: {}", files.join(", ")))
                    }
                    Ok(_) => {
                        this.info = Some((
                            if abort {
                                "Back to where the branch was before updating.".into()
                            } else {
                                "Update finished.".into()
                            },
                            std::time::Instant::now(),
                        ))
                    }
                    Err(error) => this.fail("git", error),
                }
                this.refresh();
            },
        );
    }

    /// Merge the session branch into its base branch (after confirmation), then offer
    /// to clean up.
    pub(crate) fn merge_into_base(&mut self, id: u64) {
        let Some(session) = self.state.session(id) else {
            return;
        };
        let Some(project) = self.project_of(session) else {
            return;
        };
        let (repo, branch) = (project.path, session.branch.clone());
        let base = if session.base.is_empty() {
            self.branch_info.base.clone()
        } else {
            session.base.clone()
        };
        if self.turn_running(id) {
            self.fail(
                "git",
                "Wait for the agent to finish its turn before merging.".into(),
            );
            return;
        }
        let pending = self.stats.get(&id).map_or(0, |s| s.0);
        if pending > 0 {
            self.fail("git", format!(
                "Commit or discard the {pending} uncommitted change{} first; only commits are merged.",
                if pending == 1 { "" } else { "s" }
            ));
            return;
        }
        if !self.begin(id, &format!("Merging into {base}.")) {
            return;
        }
        self.bg(
            move || git::merge_into(&repo, &branch, &base).map(|hash| (hash, base, branch)),
            move |this, result, _| {
                this.git_busy.remove(&id);
                match result {
                    Ok((hash, base, branch)) => {
                        this.state
                            .log(id, format!("Merged {branch} into {base} ({hash})"));
                        this.info = Some((
                            format!("Merged into {base} ({hash}). Push {base} when you're ready."),
                            std::time::Instant::now(),
                        ));
                        if this.state.session(id).is_some_and(|s| s.isolated) {
                            this.confirm_git = Some(Confirm::CleanUp(id));
                        }
                        this.save();
                    }
                    Err(error) => this.fail("git", format!("Could not merge: {error}")),
                }
                this.refresh();
            },
        );
    }

    /// The project a session belongs to.
    pub(crate) fn project_of(&self, session: &Session) -> Option<Project> {
        self.state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .cloned()
    }

    /// Delete a session's worktree, branch and Undo checkpoints in the background.
    /// `force` deletes the branch even with commits the base lacks (a squash-merged PR,
    /// or a race the person decided against).
    pub(crate) fn remove_session_files(&mut self, session: &Session, force: bool) {
        let Some(project) = self.project_of(session) else {
            return;
        };
        let checkpoints: Vec<String> = session
            .messages
            .iter()
            .flat_map(|m| [m.checkpoint.clone(), m.redo.clone()])
            .chain([session.snapshot.clone()])
            .filter(|c| !c.is_empty())
            .collect();
        let isolated = session.isolated && remove_worktree();
        let (dir, branch, base) = (
            session.dir.clone(),
            session.branch.clone(),
            session.base.clone(),
        );
        if !git::is_repo(&project.path) {
            return;
        }
        let title = session.title.clone();
        self.bg(
            move || {
                let repo = project.path;
                git::drop_checkpoints(&repo, &checkpoints);
                if !isolated {
                    return Ok(None);
                }
                if std::path::Path::new(&dir).exists() {
                    git::remove_worktree(&repo, &dir)?;
                }
                let base = if base.is_empty() {
                    git::default_branch(&repo)
                } else {
                    base
                };
                if branch.is_empty() || branch == base {
                    return Ok(None);
                }
                let gone = if force {
                    git::delete_branch(&repo, &branch, true).is_ok()
                } else {
                    git::delete_branch_if_merged(&repo, &branch, &base)
                };
                Ok((!gone).then_some(branch))
            },
            move |this, result: Result<Option<String>, String>, _| match result {
                Ok(Some(kept)) => {
                    this.info = Some((
                        format!(
                            "Removed “{title}”. Kept branch {kept}: it has commits not merged yet."
                        ),
                        std::time::Instant::now(),
                    ))
                }
                Ok(None) => {}
                Err(error) => this.fail(
                    "git",
                    format!("Session removed; worktree cleanup failed: {error}"),
                ),
            },
        );
    }

    /// After a merge: delete the session's worktree and branch, and archive it.
    pub(crate) fn clean_up(&mut self, id: u64) {
        let Some(session) = self.state.session(id).cloned() else {
            return;
        };
        if self.turn_running(id) {
            self.fail("git", "Stop the agent first.".into());
            return;
        }
        let merged = self
            .pr_status
            .get(&id)
            .is_some_and(|pr| pr.state == "MERGED");
        self.remove_session_files(&session, merged);
        for shell in self.shells.iter().filter(|s| s.session == id) {
            shell.kill();
        }
        if let Some(s) = self.state.session_mut(id) {
            s.archived = true;
        }
        self.state.log(id, "Cleaned up the worktree and branch");
        self.info = Some((
            format!("Cleaned up “{}” and archived it.", session.title),
            std::time::Instant::now(),
        ));
        self.save();
    }

    /// Keep one racer: stop the others, delete their worktrees and branches, and offer
    /// to land the winner.
    pub(crate) fn keep_racer(&mut self, keep: u64) {
        let Some(race) = self.state.session(keep).map(|s| s.race).filter(|r| *r != 0) else {
            return;
        };
        let others: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.race == race && s.id != keep)
            .map(|s| s.id)
            .collect();
        for &id in &others {
            if let Some(handle) = self.handles.remove(&Target::Session(id)) {
                self.stopping.insert(id);
                handle.kill();
            }
            if let Some(handle) = self.handles.remove(&Target::Terminal(id)) {
                handle.kill();
            }
            self.started.remove(&id);
            self.pending_start.remove(&id);
            if let Some(session) = self.state.session(id).cloned() {
                self.remove_session_files(&session, true);
            }
            self.state.sessions.retain(|s| s.id != id);
            self.tabs.retain(|t| *t != id);
            self.stats.remove(&id);
            self.terminal.remove(&id);
        }
        self.prune_shells();
        if let Some(s) = self.state.session_mut(keep) {
            s.race = 0;
        }
        self.kept = Some(keep);
        self.state.log(keep, "Kept this racer's work");
        self.info = Some((
            format!(
                "Kept this one; stopped and removed {} other{}.",
                others.len(),
                if others.len() == 1 { "" } else { "s" }
            ),
            std::time::Instant::now(),
        ));
        self.keep_awake_update();
        self.save();
        self.refresh();
    }

    /// Carry out a confirmed git step.
    fn confirmed(&mut self, confirm: Confirm) {
        self.confirm_git = None;
        match confirm {
            Confirm::Keep(id) => self.keep_racer(id),
            Confirm::Merge(id) => self.merge_into_base(id),
            Confirm::CleanUp(id) => self.clean_up(id),
            Confirm::MergePr(id) => self.merge_pr(id, false),
            Confirm::AutoMergePr(id) => self.merge_pr(id, true),
            Confirm::PostReview(id) => self.post_review(id),
            Confirm::PushAnyway(id) => {
                self.belt_pass = Some(id);
                self.push_session(id);
            }
        }
    }

    /// A confirmation card for a git step.
    pub(crate) fn confirm_card(&self, id: u64, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.confirm_git?;
        let session = self.state.session(id)?;
        let base = if session.base.is_empty() {
            self.branch_info.base.clone()
        } else {
            session.base.clone()
        };
        let (title, body, action) = match confirm {
            Confirm::Keep(k) if k == id => {
                let others = self
                    .state
                    .sessions
                    .iter()
                    .filter(|s| s.race == session.race && s.id != id)
                    .count();
                (
                    format!("Keep {}'s work?", agent::provider(&session.provider).name),
                    format!(
                        "Stops the other {others} agent{} and deletes their worktrees and branches. Then you can merge this one into {base} or open a pull request.",
                        if others == 1 { "" } else { "s" }
                    ),
                    "Keep this one",
                )
            }
            Confirm::Merge(m) if m == id => (
                format!("Merge {} into {base}?", session.branch),
                format!(
                    "Merges the branch's commits into your local {base}, where it is checked out (or in a temporary worktree). Nothing is pushed."
                ),
                "Merge",
            ),
            Confirm::CleanUp(c) if c == id => (
                "Clean up this session?".to_string(),
                format!(
                    "Deletes the worktree at {} and the branch {}, then archives the session. Its conversation stays under All sessions.",
                    session.dir, session.branch
                ),
                "Clean up",
            ),
            Confirm::MergePr(m) if m == id => {
                let pr = self.pr_status.get(&id)?;
                let how = self
                    .merge_choice(id)
                    .map_or("with a method the repository allows", |(m, _)| m.describe());
                (
                    format!("Merge pull request #{}?", pr.number),
                    format!(
                        "Merges it on GitHub {how}{}. Then you can clean up the worktree and branch.",
                        if pr.failed.is_empty() {
                            ""
                        } else {
                            ", although checks are failing"
                        }
                    ),
                    "Merge PR",
                )
            }
            Confirm::AutoMergePr(m) if m == id => {
                let pr = self.pr_status.get(&id)?;
                let (method, _) = self.merge_choice(id)?;
                (
                    format!("Merge #{} when checks pass?", pr.number),
                    format!(
                        "Turns on auto-merge: GitHub merges it {} once the required checks pass and nothing else blocks it.",
                        method.describe()
                    ),
                    "Turn on auto-merge",
                )
            }
            Confirm::PushAnyway(p) if p == id => (
                "Push it anyway?".to_string(),
                format!(
                    "{} Once it's on the remote it stays in the history, even if you delete it later. Undo the commit and leave it out, or push it anyway.",
                    seatbelt::summary(&self.belt_found)
                ),
                "Push anyway",
            ),
            Confirm::PostReview(p) if p == id => {
                let pr = self.pr_status.get(&id)?;
                let draft = pr.draft_review.as_ref()?;
                (
                    format!("Post this review on #{} as you?", pr.number),
                    github::review_summary(draft),
                    "Post review",
                )
            }
            _ => return None,
        };
        Some(rise(
            div()
                .p_3()
                .v_flex()
                .gap_2()
                .rounded_xl()
                .border_1()
                .border_color(divider())
                .bg(tint(c_subtle()))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .child(body),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            self.button("git-confirm-no", "Cancel", false)
                                .h(px(28.))
                                .text_xs()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_git = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button("git-confirm-yes", action, true)
                                .h(px(28.))
                                .text_xs()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirmed(confirm);
                                    cx.notify();
                                })),
                        ),
                ),
            SharedString::from(format!("git-confirm-{confirm:?}")),
            260,
        ))
    }

    /// After keeping a racer: land its work.
    pub(crate) fn kept_card(&self, id: u64, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.kept != Some(id) || self.confirm_git.is_some() {
            return None;
        }
        let session = self.state.session(id)?;
        let base = if session.base.is_empty() {
            self.branch_info.base.clone()
        } else {
            session.base.clone()
        };
        Some(
            div()
                .p_3()
                .flex()
                .items_center()
                .gap_2()
                .rounded_xl()
                .border_1()
                .border_color(divider())
                .bg(tint(c_subtle()))
                .child(icon(IconName::Trophy, 14., c_blue()))
                .child(div().flex_1().text_sm().child(format!(
                    "Kept {}'s work. Land it:",
                    agent::provider(&session.provider).name
                )))
                .child(
                    self.button("kept-merge", format!("Merge into {base}"), false)
                        .h(px(28.))
                        .text_xs()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.kept = None;
                            this.confirm_git = Some(Confirm::Merge(id));
                            this.right_tab = RightTab::Changes;
                            this.show_panel = true;
                            cx.notify();
                        })),
                )
                .child(
                    self.button("kept-pr", "Create PR", true)
                        .h(px(28.))
                        .text_xs()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.kept = None;
                            this.open_pr_form(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    self.ghost_button_sized("kept-close", IconName::X, "Not now", 24.)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.kept = None;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    /// A small button for the ship bar.
    fn ship_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        name: IconName,
        enabled: bool,
    ) -> Stateful<Div> {
        self.button(id, label, false)
            .h(px(24.))
            .px_2()
            .text_size(px(12.))
            .when(!enabled, |el| el.opacity(0.45))
            .child(icon(name, 13., c_secondary()))
    }

    /// The Create pull request form.
    fn pr_form_view(&self, form: &PrForm, cx: &mut Context<Self>) -> AnyElement {
        let label = |text: &'static str| {
            div()
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child(text)
        };
        div()
            .p_3()
            .v_flex()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .bg(rgb(c_canvas()))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Create pull request"),
            )
            .when(form.needs_branch, |el| {
                el.child(label("This session works on the base branch itself. Its work moves to a new branch first:"))
                    .child(Input::new(&self.pr_branch).bordered(true).h(px(30.)))
            })
            .child(label("Title"))
            .child(Input::new(&self.pr_title).bordered(true).h(px(30.)))
            .child(label("Description"))
            .child(Textarea::new(&self.pr_body).bordered(true).h(px(90.)))
            .child(label("Merge into"))
            .child(Input::new(&self.pr_base).bordered(true).h(px(30.)))
            .when_some(form.error.clone(), |el, error| {
                el.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(c_red()))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("pr-cancel", "Cancel", false)
                            .h(px(28.))
                            .text_xs()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pr_form = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "pr-submit",
                            if form.running {
                                "Pushing…"
                            } else {
                                "Push and open PR"
                            },
                            true,
                        )
                        .h(px(28.))
                        .text_xs()
                        .when(form.running, |el| el.opacity(0.6))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.submit_pr(cx);
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element()
    }

    /// The pull request row: state, checks, review, and what to do about them.
    fn pr_row(&self, id: u64, pr: &github::PrStatus, cx: &mut Context<Self>) -> AnyElement {
        let (state, color) = match pr.state.as_str() {
            "MERGED" => ("Merged", c_blue()),
            "CLOSED" => ("Closed", c_muted()),
            _ if pr.draft => ("Draft", c_muted()),
            _ => ("Open", c_green()),
        };
        let checks = if !pr.failed.is_empty() {
            Some((format!("{} failing", pr.failed.len()), c_red()))
        } else if pr.pending > 0 {
            Some((format!("{} running", pr.pending), c_amber()))
        } else if pr.passed > 0 {
            Some(("checks passed".to_string(), c_green()))
        } else {
            None
        };
        let review = match pr.review.as_str() {
            "APPROVED" => Some(("approved", c_green())),
            "CHANGES_REQUESTED" => Some(("changes requested", c_amber())),
            _ => None,
        };
        let url = pr.url.clone();
        let open = pr.state == "OPEN";
        let choice = self
            .merge_choice(id)
            .map(|(method, allowed)| (method, allowed.to_vec()));
        div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .child(icon(IconName::GitPullRequest, 12., color))
                    .child(
                        div()
                            .id("pr-link")
                            .cursor_pointer()
                            .text_color(rgb(c_text()))
                            .hover(|el| el.underline())
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                            .child(format!("#{}", pr.number)),
                    )
                    .child(div().text_color(rgb(color)).child(state))
                    .when_some(checks, |el, (text, color)| {
                        el.child(div().text_color(rgb(color)).child(format!("· {text}")))
                    })
                    .when_some(review, |el, (text, color)| {
                        el.child(div().text_color(rgb(color)).child(format!("· {text}")))
                    })
                    .when(open && pr.auto_merge, |el| {
                        el.child(
                            div()
                                .text_color(rgb(c_green()))
                                .child("· auto-merge on"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .when(open && !pr.failed.is_empty(), |el| {
                        el.child(
                            self.ship_button("fix-ci", "Fix CI", IconName::Wrench, true)
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Ask the agent to read the failing logs (gh run view --log-failed) and fix them",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.fix_ci(id, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .when(open && (pr.comments > 0 || pr.review == "CHANGES_REQUESTED"), |el| {
                        el.child(
                            self.ship_button(
                                "address-review",
                                "Address review",
                                IconName::MessageSquareText,
                                true,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.address_review(id);
                                cx.notify();
                            })),
                        )
                    })
                    .when(open && pr.draft_review.is_some(), |el| {
                        el.child(
                            self.ship_button("post-review", "Post review", IconName::Send, true)
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Read the review the agent wrote, then post it to GitHub",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirm_git = Some(Confirm::PostReview(id));
                                    cx.notify();
                                })),
                        )
                    })
                    .when(open && !pr.draft, |el| {
                        el.child(
                            self.ship_button("merge-pr", "Merge PR", IconName::GitMerge, true)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirm_git = Some(Confirm::MergePr(id));
                                    cx.notify();
                                })),
                        )
                    })
                    .when_some(choice.filter(|_| open && !pr.draft), |el, (method, allowed)| {
                        el.when(allowed.len() > 1, |el| {
                            el.child(allowed.iter().enumerate().fold(
                                github::segmented().p(px(1.)),
                                |row, (i, &m)| {
                                    row.child(
                                        github::segment(("merge-method", i), m.label(), m == method)
                                            .h(px(20.))
                                            .px_2()
                                            .text_size(px(11.5))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.pick_merge_method(id, m);
                                                cx.notify();
                                            })),
                                    )
                                },
                            ))
                        })
                        .when(pr.waits_for_checks(), |el| {
                            el.child(
                                self.ship_button(
                                    "auto-merge-pr",
                                    "Merge when checks pass",
                                    IconName::Clock,
                                    true,
                                )
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Turn on auto-merge (gh pr merge --auto): GitHub merges it once the required checks pass",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirm_git = Some(Confirm::AutoMergePr(id));
                                    cx.notify();
                                })),
                            )
                        })
                    })
                    .when(pr.state == "MERGED", |el| {
                        el.child(
                            self.ship_button("pr-clean", "Clean up", IconName::Trash, true)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirm_git = Some(Confirm::CleanUp(id));
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Commit box, staging summary, and the branch's way to land: update from the base,
    /// merge into it, push and pull requests.
    pub(crate) fn ship_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session) = self.current() else {
            return div().into_any_element();
        };
        let id = session.id;
        let has_changes = !self.changes.is_empty() && !self.diff_base;
        let sync = &self.sync;
        let info = &self.branch_info;
        let busy = self.git_busy.get(&id).cloned();
        let running = self.turn_running(id);
        let on_base = self.on_base_branch(session);
        let status = if sync.remote.is_empty() {
            "No remote".to_string()
        } else if sync.ahead > 0 {
            format!(
                "{} unpushed commit{}",
                sync.ahead,
                if sync.ahead == 1 { "" } else { "s" }
            )
        } else if sync.upstream {
            format!("Up to date with {}", sync.remote)
        } else {
            "Not pushed yet".to_string()
        };
        let idle = busy.is_none();
        let can_push =
            !sync.remote.is_empty() && idle && !on_base && (sync.ahead > 0 || !sync.upstream);
        let can_pr = !sync.remote.is_empty() && idle && self.pr_form.is_none();
        let notes = session.notes.len();
        let staged = self.changes.iter().filter(|c| c.staged).count();
        let base = info.base.clone();
        let lands = !base.is_empty() && session.branch != base && session.branch != "local folder";
        let pr = self.pr_status.get(&id);
        div()
            .v_flex()
            .gap_2()
            .when(notes > 0, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_lg()
                        .bg(tint(c_selected()))
                        .child(icon(IconName::MessageSquareText, 14., c_blue()))
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(c_text()))
                                .child(format!(
                                    "{notes} review note{}",
                                    if notes == 1 { "" } else { "s" }
                                )),
                        )
                        .child(
                            self.button("clear-notes", "Clear", false)
                                .child(icon(IconName::Eraser, 12., c_secondary()))
                                .h(px(28.))
                                .text_xs()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.selected {
                                        if let Some(s) = this.state.session_mut(id) {
                                            s.notes.clear();
                                        }
                                        this.save();
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            self.button("send-notes", "Send to agent", true)
                                .child(icon(IconName::Send, 12., c_canvas()))
                                .h(px(28.))
                                .text_xs()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.send_notes(cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(info.in_progress, |el, kind| {
                let files = info.conflicts.len();
                el.child(
                    div()
                        .p_3()
                        .v_flex()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(c_amber()))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.5))
                                .child(icon(IconName::TriangleAlert, 13., c_amber()))
                                .child(if files > 0 {
                                    format!(
                                        "The {kind} onto {} stopped on conflicts in {files} file{}.",
                                        info.base_ref,
                                        if files == 1 { "" } else { "s" }
                                    )
                                } else {
                                    format!("A {kind} onto {} is waiting to be continued.", info.base_ref)
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .when(files > 0, |el| {
                                    el.child(
                                        self.ship_button(
                                            "conflict-agent",
                                            "Send to agent",
                                            IconName::Bot,
                                            idle,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.resolve_with_agent(cx);
                                            cx.notify();
                                        })),
                                    )
                                })
                                .child(
                                    self.ship_button("conflict-continue", "Continue", IconName::Play, idle)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.finish_update(false);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    self.ship_button("conflict-abort", "Abort", IconName::X, idle)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.finish_update(true);
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            })
            .when(has_changes, |el| {
                el.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div().flex_1().text_size(px(12.5)).child(
                                        Input::new(&self.commit_msg)
                                            .bordered(true)
                                            .bg(rgb(c_canvas()))
                                            .h(px(30.)),
                                    ),
                                )
                                .child(if self.writing_commit {
                                    div()
                                        .size(px(30.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            icon(IconName::Loader, 14., c_secondary()).animated(
                                                "commit-spin",
                                                Animation::new(Duration::from_millis(900)).repeat(),
                                                |i, d| i.rotate(percentage(d)),
                                            ),
                                        )
                                        .into_any_element()
                                } else {
                                    self.ghost_button_sized(
                                        "ai-commit",
                                        IconName::Sparkles,
                                        "Write the message with the agent",
                                        30.,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.suggest_commit(cx);
                                        cx.notify();
                                    }))
                                    .into_any_element()
                                }),
                        )
                        .child(
                            self.button(
                                "commit",
                                if staged > 0 {
                                    format!("Commit {staged} staged")
                                } else {
                                    "Commit".to_string()
                                },
                                true,
                            )
                            .w_full()
                            .justify_center()
                            .h(px(28.))
                            .when(running || !idle, |el| el.opacity(0.5))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(if running {
                                    "The agent is still working; commit when its turn ends."
                                } else if staged > 0 {
                                    "Commit the staged files. An empty message uses the session title."
                                } else {
                                    "Stage and commit every change. An empty message uses the session title."
                                })
                                .build(window, cx)
                            })
                            .child(icon(IconName::GitCommitHorizontal, 14., c_canvas()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.commit(window, cx);
                                cx.notify();
                            })),
                        ),
                )
            })
            .when(lands, |el| {
                let behind = info.behind;
                let ahead = info.ahead;
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(IconName::GitMerge, 12., c_muted()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .child(match (behind, ahead) {
                                    (0, 0) => format!("Even with {}", info.base_ref),
                                    (0, a) => format!("{a} ahead of {}", info.base_ref),
                                    (b, 0) => format!("{b} behind {}", info.base_ref),
                                    (b, a) => format!("{b} behind, {a} ahead of {}", info.base_ref),
                                }),
                        )
                        .child(
                            self.ship_button(
                                "update-base",
                                format!("Update from {base}"),
                                IconName::RefreshCw,
                                idle && info.in_progress.is_none(),
                            )
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Fetch, then rebase this branch onto the latest base. Hold Alt to merge instead. Conflicts stay for you or the agent to resolve.",
                                )
                                .build(window, cx)
                            })
                            .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                                this.update_from_base(!ev.modifiers().alt);
                                cx.notify();
                            })),
                        )
                        .when(ahead > 0, |el| {
                            el.child(
                                self.ship_button(
                                    "merge-base",
                                    format!("Merge into {base}"),
                                    IconName::GitMerge,
                                    idle,
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirm_git = Some(Confirm::Merge(id));
                                    cx.notify();
                                })),
                            )
                        }),
                )
            })
            .when(!lands && info.behind > 0 && !info.base_ref.is_empty(), |el| {
                // A session on the base branch itself: pull what others pushed.
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(IconName::CloudDownload, 12., c_muted()))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .child(format!("{} new on {}", info.behind, info.base_ref)),
                        )
                        .child(
                            self.ship_button("pull-base", "Pull", IconName::CloudDownload, idle)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.update_from_base(true);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::GitBranch, 12., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(c_secondary()))
                            .child(match &busy {
                                Some(label) => label.clone(),
                                None => status,
                            }),
                    )
                    .child(
                        self.ship_button("push", "Push", IconName::CloudUpload, can_push)
                            .when(on_base, |el| {
                                el.tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "This session is on the base branch; Create PR moves its work to a new branch first.",
                                    )
                                    .build(window, cx)
                                })
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if can_push {
                                    this.push();
                                }
                                cx.notify();
                            })),
                    )
                    .when(pr.is_none(), |el| {
                        el.child(
                            self.ship_button(
                                "create-pr",
                                "Create PR",
                                IconName::GitPullRequestCreate,
                                can_pr,
                            )
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(
                                    "Push the branch and open a pull request with the GitHub CLI (gh).",
                                )
                                .build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if can_pr {
                                    this.open_pr_form(window, cx);
                                }
                                cx.notify();
                            })),
                        )
                    }),
            )
            .when_some(pr, |el, pr| el.child(self.pr_row(id, pr, cx)))
            .when_some(
                self.pr_form.as_ref().filter(|f| f.session == id),
                |el, form| el.child(self.pr_form_view(form, cx)),
            )
            .children(
                self.confirm_card(id, cx)
                    .filter(|_| !matches!(self.confirm_git, Some(Confirm::Keep(_)))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn prompts_name_the_commands_and_files() {
        let conflict = conflict_prompt("rebase", "origin/main", &["a.rs".into(), "b.rs".into()]);
        assert!(conflict.contains("- a.rs\n- b.rs"));
        assert!(conflict.contains("git rebase --continue"));
        assert!(conflict_prompt("merge", "main", &[]).contains("git commit --no-edit"));
        let pr = github::PrStatus {
            number: 7,
            url: "https://github.com/o/r/pull/7".into(),
            failed: vec!["build".into(), "lint".into()],
            failed_run: Some(99),
            ..Default::default()
        };
        let ci = ci_prompt(&pr);
        assert!(ci.contains("`gh run view 99 --log-failed`"));
        assert!(ci.contains("build, lint"));
        let unknown = ci_prompt(&github::PrStatus {
            failed_run: None,
            ..pr.clone()
        });
        assert!(unknown.contains("gh pr checks 7"));
        let review = review_prompt(&pr, &["ann on `src/a.rs:3`: rename this".into()]);
        assert!(review.contains("1. ann on `src/a.rs:3`: rename this"));
        assert!(review_prompt(&pr, &[]).contains("pulls/7/comments"));
    }

    #[test]
    fn scans_count_commits_against_the_base() {
        let root = std::env::temp_dir().join(format!("runo-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sh = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        sh(&["init", "-q", "-b", "main"]);
        sh(&["config", "user.email", "t@t"]);
        sh(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        sh(&["add", "."]);
        sh(&["commit", "-qm", "init"]);
        sh(&["checkout", "-q", "-b", "work"]);
        std::fs::write(root.join("b.txt"), "b\nb\n").unwrap();
        sh(&["add", "."]);
        sh(&["commit", "-qm", "work"]);
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        let dir = root.to_string_lossy().into_owned();
        let request = |since_base| ScanRequest {
            dir: dir.clone(),
            base: String::new(),
            since_base,
            selected_file: None,
            whole: false,
            all: true,
            racers: vec![(5, dir.clone())],
        };
        let scan_now = scan(request(false));
        assert!(scan_now.is_git);
        assert_eq!(scan_now.branch.base, "main");
        assert_eq!((scan_now.branch.ahead, scan_now.branch.behind), (1, 0));
        assert_eq!(scan_now.uncommitted, (1, 1, 0));
        assert_eq!(scan_now.race[&5], (2, 3, 0), "racers count commits too");
        assert_eq!(scan_now.diff.as_ref().unwrap().0, "c.txt");
        let since = scan(request(true));
        assert_eq!(since.changes.len(), 2);
        assert_eq!(since.all_diffs.len(), 2);
        assert_ne!(since.against, "HEAD");
        let _ = std::fs::remove_dir_all(&root);
    }
}
