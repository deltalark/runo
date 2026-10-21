//! Thin wrappers over the `git` CLI for worktrees, status and diffs.

use std::path::Path;
use std::process::Command;

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    pub status: String,
    pub added: usize,
    pub removed: usize,
    /// Some of the change is in the index.
    pub staged: bool,
    /// Some of the change is only in the working tree.
    pub unstaged: bool,
    /// Git treats the file as binary, so there are no line counts.
    pub binary: bool,
}

/// Run git in `dir` with extra environment and optional stdin.
fn git_with(
    dir: &str,
    args: &[&str],
    env: &[(&str, &str)],
    input: Option<&[u8]>,
) -> Result<String, String> {
    use std::io::Write;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        // Never wait on a password prompt nobody can see.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        command.env(k, v);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not run git: {error}"))?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(bytes);
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            err
        })
    }
}

fn git(dir: &str, args: &[&str]) -> Result<String, String> {
    git_with(dir, args, &[], None)
}

/// Whether `rev` names an object in the repository.
fn exists(dir: &str, rev: &str) -> bool {
    git(
        dir,
        &["rev-parse", "-q", "--verify", &format!("{rev}^{{commit}}")],
    )
    .is_ok()
}

/// Where the folder was before the last rebase, merge or reset.
pub fn orig_head(dir: &str) -> Option<String> {
    rev(dir, "ORIG_HEAD")
}

/// The commit the folder is on, if there is one yet.
pub fn head(dir: &str) -> Option<String> {
    rev(dir, "HEAD")
}

pub fn rev(dir: &str, rev: &str) -> Option<String> {
    git(
        dir,
        &["rev-parse", "-q", "--verify", &format!("{rev}^{{commit}}")],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// Whether `a` is `b` or one of its ancestors.
pub fn is_ancestor(dir: &str, a: &str, b: &str) -> bool {
    git(dir, &["merge-base", "--is-ancestor", a, b]).is_ok()
}

fn count(dir: &str, args: &[&str]) -> Option<usize> {
    git(dir, args).ok().and_then(|s| s.trim().parse().ok())
}

/// Keep `pattern` out of `git status` for this repository and all its worktrees,
/// without touching any tracked file (.git/info/exclude).
pub fn exclude(dir: &str, pattern: &str) {
    let Ok(common) = git(dir, &["rev-parse", "--git-common-dir"]) else {
        return;
    };
    let common = Path::new(dir).join(common.trim());
    let file = common.join("info").join("exclude");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    if current.lines().any(|l| l.trim() == pattern) {
        return;
    }
    let _ = std::fs::create_dir_all(common.join("info"));
    let sep = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let _ = std::fs::write(&file, format!("{current}{sep}{pattern}\n"));
}

/// Whether git tracks `path` (relative to `dir`).
pub fn is_tracked(dir: &str, path: &str) -> bool {
    git(dir, &["ls-files", "--error-unmatch", "--", path]).is_ok()
}

/// Keep one file in `dir` (given relative to it) out of `git status`.
pub fn exclude_path(dir: &str, path: &str) {
    let Ok(prefix) = git(dir, &["rev-parse", "--show-prefix"]) else {
        return;
    };
    exclude(dir, &format!("/{}{path}", prefix.trim()));
}

/// The last `n` commits, one line each.
pub fn recent_commits(dir: &str, n: usize) -> String {
    git(
        dir,
        &["log", &format!("-{n}"), "--oneline", "--no-decorate"],
    )
    .unwrap_or_default()
}

pub fn is_repo(dir: &str) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"]).is_ok()
}

pub fn current_branch(dir: &str) -> String {
    git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn sanitize_branch(input: &str) -> String {
    let cleaned: String = input
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    cleaned
        .trim_matches(|c| c == '-' || c == '/' || c == '.')
        .to_string()
}

/// Where an isolated session's worktree lives: `<repo-parent>/.runo-worktrees/<repo>/<branch>`.
/// A branch with slashes gets nested folders, so `a/b` and `a-b` never share one.
pub fn worktree_path(repo: &str, branch: &str) -> std::path::PathBuf {
    let repo_path = Path::new(repo);
    let name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let mut path = repo_path
        .parent()
        .unwrap_or(repo_path)
        .join(".runo-worktrees")
        .join(name);
    for part in branch.split('/').filter(|p| !p.is_empty() && *p != "..") {
        path.push(part);
    }
    path
}

/// A branch name drawn from a task, such as `runo/fix-login-redirect` for "Fix the login
/// redirect". Uses the first line's first few words; empty when it has no usable words.
pub fn branch_slug(task: &str) -> String {
    const SKIP: &[&str] = &[
        "a", "an", "the", "please", "can", "could", "you", "to", "of", "for", "and", "in", "on",
        "with",
    ];
    let line = task.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut slug = String::new();
    for word in line
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !SKIP.contains(&w.as_str()))
        .take(5)
    {
        if !slug.is_empty() && slug.len() + 1 + word.len() > 40 {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&word[..word.len().min(40)]);
    }
    if slug.is_empty() {
        slug
    } else {
        format!("runo/{slug}")
    }
}

/// `wanted`, or `wanted-2`, `wanted-3`… when `taken` says the name is in use.
pub fn unique_branch(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(wanted) {
        return wanted.to_string();
    }
    (2..)
        .map(|n| format!("{wanted}-{n}"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

/// Whether the repository has a local branch called `name`.
pub fn has_branch(repo: &str, name: &str) -> bool {
    exists(repo, &format!("refs/heads/{name}"))
}

/// Branches a new session can start from, most recently committed first: local ones, then
/// the remote's that have no local copy (`remote` set). Names carry no remote prefix.
pub fn branches(dir: &str) -> Vec<(String, bool)> {
    let remote = remote_name(dir);
    let remote_refs = format!("refs/remotes/{remote}");
    let mut args = vec![
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname)",
        "refs/heads",
    ];
    if !remote.is_empty() {
        args.push(&remote_refs);
    }
    let out = git(dir, &args).unwrap_or_default();
    let mut local = Vec::new();
    let mut remote_only = Vec::new();
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("refs/heads/") {
            local.push(name.to_string());
        } else if let Some(name) = line
            .strip_prefix(remote_refs.as_str())
            .and_then(|n| n.strip_prefix('/'))
        {
            if name != "HEAD" {
                remote_only.push(name.to_string());
            }
        }
    }
    remote_only.retain(|r| !local.contains(r));
    local
        .into_iter()
        .map(|n| (n, false))
        .chain(remote_only.into_iter().map(|n| (n, true)))
        .collect()
}

/// The folder `git clone` makes for `url`: its last path part without `.git`.
pub fn clone_dir_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\', ':']).next().unwrap_or(trimmed);
    last.strip_suffix(".git").unwrap_or(last).to_string()
}

/// Clone `url` into `dest`, handing each progress line git prints to `progress`.
pub fn clone(url: &str, dest: &Path, mut progress: impl FnMut(&str)) -> Result<(), String> {
    use std::io::Read;
    let occupied = std::fs::read_dir(dest).is_ok_and(|mut d| d.next().is_some());
    if occupied || dest.is_file() {
        return Err(format!("{} already exists", dest.display()));
    }
    let mut child = Command::new("git")
        .arg("clone")
        .arg("--progress")
        .arg("--")
        .arg(url.trim())
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run git: {error}"))?;
    // Progress lines end in \r while they update and \n once done.
    let mut all = String::new();
    let mut line = Vec::new();
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &byte in &buf[..n] {
                if byte != b'\r' && byte != b'\n' {
                    line.push(byte);
                    continue;
                }
                let text = String::from_utf8_lossy(&line).trim().to_string();
                if !text.is_empty() {
                    progress(&text);
                    all.push_str(&text);
                    all.push('\n');
                }
                line.clear();
            }
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("could not run git: {error}"))?;
    if status.success() {
        return Ok(());
    }
    all.push_str(String::from_utf8_lossy(&line).trim());
    let errors: Vec<&str> = all
        .lines()
        .filter(|l| l.starts_with("fatal:") || l.starts_with("error:") || l.starts_with("ERROR"))
        .collect();
    let why = if errors.is_empty() {
        all.lines()
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("git clone failed")
            .to_string()
    } else {
        errors.join("\n")
    };
    Err(auth_hint(&why))
}

/// The remote to fetch from and push to: `origin`, else the first one, else empty.
pub fn remote_name(dir: &str) -> String {
    let remotes = git(dir, &["remote"]).unwrap_or_default();
    if remotes.lines().any(|r| r == "origin") {
        "origin".to_string()
    } else {
        remotes.lines().next().unwrap_or("").to_string()
    }
}

/// The repository's main branch: what the remote's HEAD points at, else `main` or
/// `master`, else the branch checked out now.
pub fn default_branch(dir: &str) -> String {
    let remote = remote_name(dir);
    if !remote.is_empty() {
        if let Ok(head) = git(
            dir,
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        ) {
            if let Some(branch) = head.trim().strip_prefix(&format!("{remote}/")) {
                return branch.to_string();
            }
        }
    }
    for name in ["main", "master", "trunk", "develop"] {
        if exists(dir, &format!("refs/heads/{name}"))
            || (!remote.is_empty() && exists(dir, &format!("refs/remotes/{remote}/{name}")))
        {
            return name.to_string();
        }
    }
    let current = current_branch(dir);
    if current == "HEAD" {
        String::new()
    } else {
        current
    }
}

/// Bring the remote's branches up to date. Fine when there is no remote.
pub fn fetch(dir: &str) -> Result<(), String> {
    let remote = remote_name(dir);
    if remote.is_empty() {
        return Ok(());
    }
    git(dir, &["fetch", "--quiet", "--prune", &remote])
        .map(|_| ())
        .map_err(|e| auth_hint(&e))
}

/// Git's error, with a pointer to the fix when it failed for want of credentials.
pub fn auth_hint(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("terminal prompts disabled")
        || lower.contains("could not read username")
        || lower.contains("authentication failed")
        || lower.contains("permission denied (publickey")
    {
        format!(
            "{}\nGit needs credentials. Run `gh auth setup-git` (or push once in a shell tab) so it can sign in without asking.",
            error.lines().next().unwrap_or(error)
        )
    } else {
        error.to_string()
    }
}

/// What the session branch is compared and merged against: the remote's copy of
/// `base` when it exists, otherwise the local branch.
pub fn base_ref(dir: &str, base: &str) -> String {
    let remote = remote_name(dir);
    let remote_ref = format!("{remote}/{base}");
    let has_local = exists(dir, &format!("refs/heads/{base}"));
    let has_remote = !remote.is_empty() && exists(dir, &format!("refs/remotes/{remote_ref}"));
    match (has_local, has_remote) {
        // The local branch may hold commits not pushed yet; use it then.
        (true, true) if !is_ancestor(dir, base, &remote_ref) => base.to_string(),
        (_, true) => remote_ref,
        (true, false) => base.to_string(),
        _ => "HEAD".to_string(),
    }
}

/// Whether the repository has a commit checked out; a fresh `git init` has none.
pub fn has_commits(dir: &str) -> bool {
    exists(dir, "HEAD")
}

/// Make an empty first commit, so branches and worktrees have somewhere to start.
pub fn initial_commit(dir: &str) -> Result<(), String> {
    git(
        dir,
        &["commit", "--allow-empty", "--quiet", "-m", "Initial commit"],
    )
    .map(|_| ())
}

/// Create the session worktree on a new (or existing) branch. A new branch starts at
/// `start` (a commit-ish such as `origin/main`), or at HEAD when `None`.
pub fn add_worktree(repo: &str, branch: &str, start: Option<&str>) -> Result<String, String> {
    if !has_commits(repo) {
        return Err(
            "the repository has no commits yet. Make a first commit, then start the session again."
                .into(),
        );
    }
    let dest = worktree_path(repo, branch);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    let dest_str = dest.to_string_lossy().into_owned();
    if exists(repo, &format!("refs/heads/{branch}")) {
        git(repo, &["worktree", "add", &dest_str, branch])?;
    } else {
        let mut args = vec!["worktree", "add", "--no-track", "-b", branch, &dest_str];
        if let Some(start) = start {
            args.push(start);
        }
        git(repo, &args)?;
    }
    Ok(dest_str)
}

/// A new worktree on `branch` holding the folder as checkpoint `cp` saw it: the branch
/// starts at the commit it was on, and uncommitted work comes along.
pub fn fork_worktree(repo: &str, branch: &str, cp: &str) -> Result<String, String> {
    let (head, _) = checkpoint_parts(repo, cp);
    let dir = add_worktree(repo, branch, head.as_deref())?;
    if let Err(error) = restore(&dir, cp, None) {
        let _ = remove_worktree(repo, &dir);
        return Err(error);
    }
    Ok(dir)
}

/// A worktree at `start` with no branch checked out, for a branch made inside it later
/// (such as by `gh pr checkout`). Lives where a branch called `name` would.
pub fn add_detached_worktree(repo: &str, name: &str, start: &str) -> Result<String, String> {
    let dest = worktree_path(repo, name);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    let dest_str = dest.to_string_lossy().into_owned();
    git(repo, &["worktree", "add", "--detach", &dest_str, start])?;
    Ok(dest_str)
}

/// Move the work in `dir` onto a new branch, keeping uncommitted changes.
pub fn switch_new(dir: &str, branch: &str) -> Result<(), String> {
    git(dir, &["switch", "-c", branch]).map(|_| ())
}

/// Push the checked-out branch and make it track the remote branch of the same name.
pub fn push(dir: &str, remote: &str) -> Result<String, String> {
    git(dir, &["push", "-u", remote, "HEAD"]).map_err(|e| auth_hint(&e))
}

pub fn remove_worktree(repo: &str, dir: &str) -> Result<(), String> {
    git(repo, &["worktree", "remove", "--force", dir]).map(|_| ())
}

/// `git worktree list --porcelain`: one block per worktree, the repository's own first.
pub fn worktree_list(repo: &str) -> Result<String, String> {
    git(repo, &["worktree", "list", "--porcelain"])
}

/// Forget worktrees whose folder is gone.
pub fn prune_worktrees(repo: &str) -> Result<(), String> {
    git(repo, &["worktree", "prune"]).map(|_| ())
}

/// Local branches under `prefix`, such as `runo/`.
pub fn local_branches(repo: &str, prefix: &str) -> Vec<String> {
    let pattern = format!("refs/heads/{}", prefix.trim_end_matches('/'));
    git(
        repo,
        &["for-each-ref", "--format=%(refname:short)", &pattern],
    )
    .map(|out| {
        out.lines()
            .map(str::trim)
            .filter(|l| l.starts_with(prefix))
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}

/// Whether the worktree has uncommitted changes, untracked files included. None when
/// git can't tell, such as for a folder that is not a worktree.
pub fn has_uncommitted(dir: &str) -> Option<bool> {
    git(dir, &["status", "--porcelain"])
        .ok()
        .map(|s| !s.trim().is_empty())
}

/// Delete a branch. Without `force` git refuses when it holds unmerged work.
pub fn delete_branch(repo: &str, branch: &str, force: bool) -> Result<(), String> {
    git(repo, &["branch", if force { "-D" } else { "-d" }, branch]).map(|_| ())
}

/// Commits on `branch` that `base` does not have.
pub fn commits_ahead(dir: &str, branch: &str, base: &str) -> usize {
    count(dir, &["rev-list", "--count", &format!("{base}..{branch}")]).unwrap_or(0)
}

/// Delete a session's branch unless that would lose work: it goes when `base` already
/// has all of its commits. Returns whether it was deleted.
pub fn delete_branch_if_merged(repo: &str, branch: &str, base: &str) -> bool {
    if branch.is_empty() || branch == base || !exists(repo, &format!("refs/heads/{branch}")) {
        return false;
    }
    let base = base_ref(repo, base);
    commits_ahead(repo, branch, &base) == 0 && delete_branch(repo, branch, true).is_ok()
}

/// Remove the refs that keep Undo checkpoints alive, so git can clean them up.
pub fn drop_checkpoints(repo: &str, ids: &[String]) {
    for id in ids.iter().filter(|id| !id.is_empty()) {
        let _ = git(
            repo,
            &["update-ref", "-d", &format!("refs/runo/checkpoints/{id}")],
        );
    }
}

/// How bringing a branch up to date went.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    UpToDate,
    Updated,
    /// Stopped on conflicts in these files; the worktree is left mid-rebase or mid-merge.
    Conflicts(Vec<String>),
}

/// Files with unresolved conflicts.
pub fn conflicted(dir: &str) -> Vec<String> {
    git(dir, &["diff", "--name-only", "-z", "--diff-filter=U"])
        .map(|s| split_z(&s))
        .unwrap_or_default()
}

fn split_z(s: &str) -> Vec<String> {
    s.split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Rebase (or merge) the checked-out branch onto `onto`, stashing uncommitted work
/// around it. Conflicts leave the worktree as git left it, for someone to resolve.
pub fn update_from(dir: &str, onto: &str, rebase: bool) -> Result<Update, String> {
    if count(dir, &["rev-list", "--count", &format!("HEAD..{onto}")]) == Some(0) {
        return Ok(Update::UpToDate);
    }
    let editor = [("GIT_EDITOR", "true")];
    let result = if rebase {
        git_with(dir, &["rebase", "--autostash", onto], &editor, None)
    } else {
        git_with(
            dir,
            &["merge", "--autostash", "--no-edit", onto],
            &editor,
            None,
        )
    };
    match result {
        Ok(_) => Ok(Update::Updated),
        Err(error) => {
            let files = conflicted(dir);
            if files.is_empty() {
                Err(error)
            } else {
                Ok(Update::Conflicts(files))
            }
        }
    }
}

/// A rebase or merge stopped half way: "rebase" or "merge".
pub fn in_progress(dir: &str) -> Option<&'static str> {
    let path = |name: &str| {
        git(dir, &["rev-parse", "--git-path", name])
            .map(|p| Path::new(dir).join(p.trim()))
            .ok()
    };
    if ["rebase-merge", "rebase-apply"]
        .iter()
        .any(|n| path(n).is_some_and(|p| p.exists()))
    {
        Some("rebase")
    } else if path("MERGE_HEAD").is_some_and(|p| p.exists()) {
        Some("merge")
    } else {
        None
    }
}

/// One stretch of a clashing file: lines both sides agree on, or a spot where each side
/// changed the same lines its own way. `ours` is what git calls ours: the branch being
/// rebased onto in a rebase, the checked-out branch in a merge.
#[derive(Clone, Debug, PartialEq)]
pub enum Stretch {
    Same(String),
    Spot { ours: String, theirs: String },
}

/// A file with clashing lines; `stretches` is None when it has no lines to pick from
/// (binary, or deleted on one side), so one side's whole file has to win.
#[derive(Clone, Debug, PartialEq)]
pub struct ClashFile {
    pub path: String,
    pub stretches: Option<Vec<Stretch>>,
    /// The file as it is, with git's markers, for an agent to read.
    pub text: String,
}

/// A rebase or merge stopped on clashing lines in the folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    /// "rebase" or "merge".
    pub kind: &'static str,
    /// The person's own branch, and what is coming in.
    pub yours: String,
    pub theirs: String,
    /// Whether the person's own side is git's "ours".
    pub yours_is_ours: bool,
    /// Files still clashing, and files that came together on their own.
    pub files: Vec<ClashFile>,
    pub merged: Vec<String>,
}

impl ClashFile {
    pub fn spots(&self) -> usize {
        self.stretches.as_ref().map_or(0, |s| {
            s.iter()
                .filter(|s| matches!(s, Stretch::Spot { .. }))
                .count()
        })
    }
}

/// Split a file git left with clash markers into its stretches.
pub fn parse_clash(text: &str) -> Option<Vec<Stretch>> {
    let mut out = Vec::new();
    let mut same = String::new();
    let (mut ours, mut theirs) = (String::new(), String::new());
    // 0: outside a spot, 1: our lines, 2: the common ancestor's (skipped), 3: theirs.
    let mut at = 0;
    let mut any = false;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        match at {
            0 if bare.starts_with("<<<<<<<") => {
                if !same.is_empty() {
                    out.push(Stretch::Same(std::mem::take(&mut same)));
                }
                at = 1;
                any = true;
            }
            1 if bare.starts_with("|||||||") => at = 2,
            1 | 2 if bare == "=======" => at = 3,
            3 if bare.starts_with(">>>>>>>") => {
                out.push(Stretch::Spot {
                    ours: std::mem::take(&mut ours),
                    theirs: std::mem::take(&mut theirs),
                });
                at = 0;
            }
            0 => same.push_str(line),
            1 => ours.push_str(line),
            2 => {}
            _ => theirs.push_str(line),
        }
    }
    if !any || at != 0 {
        return None;
    }
    if !same.is_empty() {
        out.push(Stretch::Same(same));
    }
    Some(out)
}

/// What a commit or branch is called, for people: a branch name where one points at it.
fn name_of(dir: &str, sha: &str) -> String {
    git(
        dir,
        &[
            "name-rev",
            "--name-only",
            "--no-undefined",
            "--refs=refs/heads/*",
            "--refs=refs/remotes/*",
            "--exclude=*/HEAD",
            sha,
        ],
    )
    .ok()
    .map(|n| n.trim().trim_start_matches("remotes/").to_string())
    .filter(|n| !n.is_empty() && !n.contains('~') && !n.contains('^'))
    .unwrap_or_else(|| sha.chars().take(7).collect())
}

/// The rebase or merge stopped in the folder, if one is.
pub fn clash(dir: &str) -> Option<Clash> {
    let kind = in_progress(dir)?;
    let path = |name: &str| {
        git(dir, &["rev-parse", "--git-path", name])
            .map(|p| Path::new(dir).join(p.trim()))
            .ok()
    };
    let read = |name: &str| {
        path(name)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let (yours, theirs, yours_is_ours) = if kind == "rebase" {
        let state = if path("rebase-merge").is_some_and(|p| p.exists()) {
            "rebase-merge"
        } else {
            "rebase-apply"
        };
        let head = read(&format!("{state}/head-name"));
        let onto = read(&format!("{state}/onto"));
        (
            head.trim_start_matches("refs/heads/").to_string(),
            name_of(dir, &onto),
            false,
        )
    } else {
        let merging = read("MERGE_HEAD");
        let merging = merging.lines().next().unwrap_or("");
        (current_branch(dir), name_of(dir, merging), true)
    };
    let left = conflicted(dir);
    let files = left
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(Path::new(dir).join(p)).unwrap_or_default();
            ClashFile {
                path: p.clone(),
                stretches: parse_clash(&text),
                text,
            }
        })
        .collect();
    let merged = git(dir, &["diff", "--cached", "--name-only", "-z"])
        .map(|s| split_z(&s))
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !left.contains(p))
        .collect();
    Some(Clash {
        kind,
        yours,
        theirs,
        yours_is_ours,
        files,
        merged,
    })
}

/// Settle a clashing file with this text.
pub fn settle(dir: &str, path: &str, text: &str) -> Result<(), String> {
    std::fs::write(Path::new(dir).join(path), text).map_err(|e| e.to_string())?;
    git(dir, &["add", "--", path]).map(|_| ())
}

/// Settle a clashing file with one side's whole version, or its deletion where that side
/// deleted it.
pub fn settle_side(dir: &str, path: &str, ours: bool) -> Result<(), String> {
    let side = if ours { "--ours" } else { "--theirs" };
    match git(dir, &["checkout", side, "--", path]) {
        Ok(_) => git(dir, &["add", "--", path]).map(|_| ()),
        Err(_) => git(dir, &["rm", "-q", "--", path]).map(|_| ()),
    }
}

/// A file two lines of work both change the same lines of, and where: the first line of
/// each clashing spot in the file as the two would merge.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlap {
    pub path: String,
    pub lines: Vec<usize>,
}

/// Merge `a` and `b` in memory, touching no folder, and say where they clash. Empty when
/// they come together cleanly.
pub fn test_merge(dir: &str, a: &str, b: &str) -> Result<Vec<Overlap>, String> {
    let out = match git(dir, &["merge-tree", "--write-tree", "--no-messages", a, b]) {
        Ok(_) => return Ok(Vec::new()),
        // With clashes git exits 1 and prints the merged tree, then the clashing files.
        Err(out) => out,
    };
    let mut lines = out.lines();
    let tree = lines.next().unwrap_or("").trim().to_string();
    if tree.len() < 40 || !tree.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(out);
    }
    let mut paths: Vec<String> = lines
        .filter_map(|l| l.split_once('\t').map(|(_, p)| p.to_string()))
        .collect();
    paths.dedup();
    Ok(paths
        .into_iter()
        .map(|path| {
            let merged =
                git(dir, &["cat-file", "-p", &format!("{tree}:{path}")]).unwrap_or_default();
            let lines = merged
                .lines()
                .enumerate()
                .filter(|(_, l)| l.starts_with("<<<<<<<"))
                .map(|(i, _)| i + 1)
                .collect();
            Overlap { path, lines }
        })
        .collect())
}

/// The tip of a folder's work: its uncommitted changes as a commit nothing points at,
/// or its HEAD when there are none.
pub fn work_tip(dir: &str) -> Option<String> {
    let wip = git(dir, &["stash", "create"]).unwrap_or_default();
    let wip = wip.trim();
    if wip.is_empty() {
        rev(dir, "HEAD")
    } else {
        Some(wip.to_string())
    }
}

/// Give up on a stopped rebase or merge, back to where it started.
pub fn abort_update(dir: &str) -> Result<(), String> {
    match in_progress(dir) {
        Some("rebase") => git(dir, &["rebase", "--abort"]).map(|_| ()),
        Some(_) => git(dir, &["merge", "--abort"]).map(|_| ()),
        None => Ok(()),
    }
}

/// Finish a rebase or merge once its conflicts are resolved and staged.
pub fn continue_update(dir: &str) -> Result<Update, String> {
    let left = conflicted(dir);
    if !left.is_empty() {
        return Ok(Update::Conflicts(left));
    }
    let editor = [("GIT_EDITOR", "true")];
    let result = match in_progress(dir) {
        Some("rebase") => {
            git(dir, &["add", "-u"])?;
            git_with(dir, &["rebase", "--continue"], &editor, None)
        }
        Some(_) => {
            git(dir, &["add", "-u"])?;
            git_with(dir, &["commit", "--no-edit"], &editor, None)
        }
        None => return Ok(Update::Updated),
    };
    match result {
        Ok(_) => Ok(Update::Updated),
        Err(error) => {
            let files = conflicted(dir);
            if files.is_empty() {
                Err(error)
            } else {
                Ok(Update::Conflicts(files))
            }
        }
    }
}

/// The worktree that has `branch` checked out, if any.
pub fn checked_out_at(repo: &str, branch: &str) -> Option<String> {
    let list = git(repo, &["worktree", "list", "--porcelain", "-z"]).ok()?;
    let mut path = None;
    for field in list.split('\0') {
        if let Some(p) = field.strip_prefix("worktree ") {
            path = Some(p.to_string());
        } else if field == format!("branch refs/heads/{branch}") {
            return path;
        }
    }
    None
}

/// Merge `branch` into `base` without touching the session's worktree. Uses the
/// worktree where `base` is checked out (it must have no uncommitted changes), or a
/// temporary one. Conflicts abort the merge. Returns the new short hash of `base`.
pub fn merge_into(repo: &str, branch: &str, base: &str) -> Result<String, String> {
    if !exists(repo, &format!("refs/heads/{base}")) {
        // Only the remote has it: start a local branch there first.
        let remote = remote_name(repo);
        git(
            repo,
            &["branch", "--track", base, &format!("{remote}/{base}")],
        )?;
    }
    let editor = [("GIT_EDITOR", "true")];
    let (dir, temporary) = match checked_out_at(repo, base) {
        Some(dir) => {
            let dirty = git(&dir, &["status", "--porcelain", "--untracked-files=no"])?;
            if !dirty.trim().is_empty() {
                return Err(format!(
                    "{dir} has uncommitted changes on {base}. Commit or stash them first."
                ));
            }
            (dir, false)
        }
        None => {
            let common = git(repo, &["rev-parse", "--git-common-dir"])?;
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = Path::new(repo)
                .join(common.trim())
                .join(format!("runo-merge-{nanos}"))
                .to_string_lossy()
                .into_owned();
            git(repo, &["worktree", "add", "--quiet", &dir, base])?;
            (dir, true)
        }
    };
    let result = git_with(&dir, &["merge", "--no-edit", branch], &editor, None);
    let outcome = match result {
        Ok(_) => git(&dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string()),
        Err(error) => {
            let files = conflicted(&dir);
            let _ = git(&dir, &["merge", "--abort"]);
            Err(if files.is_empty() {
                error
            } else {
                format!(
                    "Merging stopped on conflicts in {}. Update the branch from {base} first so they can be resolved there.",
                    files.join(", ")
                )
            })
        }
    };
    if temporary {
        let _ = remove_worktree(repo, &dir);
    }
    outcome
}

/// The commit where the branch left `base`.
pub fn merge_base(dir: &str, base: &str) -> Option<String> {
    git(dir, &["merge-base", "HEAD", base])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Added and removed line counts per path from `--numstat -z` output; binary files
/// report `-` for both.
fn numstat(out: &str) -> std::collections::HashMap<String, (usize, usize, bool)> {
    let mut map = std::collections::HashMap::new();
    for entry in out.split('\0') {
        let mut parts = entry.trim_start_matches('\n').splitn(3, '\t');
        if let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) {
            map.insert(
                path.to_string(),
                (a.parse().unwrap_or(0), r.parse().unwrap_or(0), a == "-"),
            );
        }
    }
    map
}

/// Line count of a new file, or `None` when it looks binary.
fn new_file_lines(path: &Path) -> Option<usize> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 8_000_000 {
        return Some(0);
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return None;
    }
    Some(bytes.split(|b| *b == b'\n').count() - usize::from(bytes.ends_with(b"\n")))
}

/// Uncommitted changes: the working tree and index against HEAD.
pub fn changes(dir: &str) -> Vec<Change> {
    let stats = numstat(
        &git(dir, &["diff", "HEAD", "--numstat", "-z", "--no-renames"]).unwrap_or_default(),
    );
    let mut result = Vec::new();
    let status = git(
        dir,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )
    .unwrap_or_default();
    let mut entries = status.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 || !entry.is_char_boundary(3) {
            continue;
        }
        let bytes = entry.as_bytes();
        let (x, y) = (bytes[0], bytes[1]);
        let code = entry[..2].trim().to_string();
        let path = entry[3..].to_string();
        // Renames and copies carry the old path as the next entry.
        if matches!(x, b'R' | b'C') {
            entries.next();
        }
        let untracked = code == "??";
        let (added, removed, binary) = if untracked {
            match new_file_lines(&Path::new(dir).join(&path)) {
                Some(lines) => (lines, 0, false),
                None => (0, 0, true),
            }
        } else {
            stats.get(&path).copied().unwrap_or((0, 0, false))
        };
        result.push(Change {
            path,
            status: if untracked { "A".into() } else { code },
            added,
            removed,
            staged: !matches!(x, b' ' | b'?'),
            unstaged: y != b' ',
            binary,
        });
    }
    result
}

/// Everything the branch changed since it left `base`: commits plus uncommitted work,
/// so committed work counts too.
pub fn changes_since(dir: &str, base: &str) -> Vec<Change> {
    let Some(start) = merge_base(dir, base) else {
        return changes(dir);
    };
    let stats = numstat(
        &git(dir, &["diff", &start, "--numstat", "-z", "--no-renames"]).unwrap_or_default(),
    );
    let names = git(
        dir,
        &["diff", &start, "--name-status", "-z", "--no-renames"],
    )
    .unwrap_or_default();
    let mut result = Vec::new();
    let mut fields = names.split('\0');
    while let (Some(code), Some(path)) = (fields.next(), fields.next()) {
        let (added, removed, binary) = stats.get(path).copied().unwrap_or((0, 0, false));
        result.push(Change {
            path: path.to_string(),
            status: code.trim().to_string(),
            added,
            removed,
            binary,
            ..Default::default()
        });
    }
    let untracked =
        git(dir, &["ls-files", "--others", "--exclude-standard", "-z"]).unwrap_or_default();
    for path in split_z(&untracked) {
        let lines = new_file_lines(&Path::new(dir).join(&path));
        result.push(Change {
            path,
            status: "A".into(),
            added: lines.unwrap_or(0),
            binary: lines.is_none(),
            unstaged: true,
            ..Default::default()
        });
    }
    result
}

/// Files, lines added and lines removed in a list of changes.
pub fn totals(changes: &[Change]) -> (usize, usize, usize) {
    let (a, r) = changes
        .iter()
        .fold((0, 0), |a, c| (a.0 + c.added, a.1 + c.removed));
    (changes.len(), a, r)
}

#[cfg(test)]
pub fn file_diff(dir: &str, path: &str) -> String {
    file_diff_context(dir, path, false, "HEAD")
}

/// The diff of one file against `against` (HEAD, or where the branch started); with
/// `whole` every unchanged line is included as context.
pub fn file_diff_context(dir: &str, path: &str, whole: bool, against: &str) -> String {
    let tracked = git(dir, &["ls-files", "--error-unmatch", "--", path]).is_ok()
        || git(dir, &["cat-file", "-e", &format!("{against}:{path}")]).is_ok();
    let out = if tracked {
        let context = if whole { "-U100000" } else { "-U3" };
        git(dir, &["diff", context, against, "--", path])
    } else {
        // `--no-index` exits 1 when files differ, which `git()` reports as an error.
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .map_err(|e| e.to_string())
    };
    out.unwrap_or_else(|e| e)
}

/// One line of a part: ' ' unchanged, '+' added, '-' removed, '\\' git's note that the
/// line before has no newline at the end.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct PartLine {
    pub kind: char,
    pub text: String,
    /// Line numbers before and after, 0 where the line isn't on that side.
    pub old: usize,
    pub new: usize,
}

/// One part of a file's change: a hunk.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Part {
    /// What git shows after the @@, often the function the part is in.
    pub title: String,
    pub old_start: usize,
    pub new_start: usize,
    pub lines: Vec<PartLine>,
}

/// A changed file split into its parts.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct FileParts {
    pub change: Change,
    /// The diff's header, everything before the first part.
    pub head: String,
    pub parts: Vec<Part>,
    /// The whole diff, to tell when the file changed again.
    pub diff: String,
}

impl FileParts {
    /// Only whole: binary, deleted or renamed files, which can't be split into lines.
    pub fn whole_only(&self) -> bool {
        self.change.binary || self.parts.is_empty() || self.change.status == "D"
    }
}

fn hunk_start(text: &str, sign: char) -> usize {
    text.split_whitespace()
        .find_map(|w| w.strip_prefix(sign))
        .and_then(|w| w.split(',').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Split a unified diff of one file into its header and parts.
pub fn parse_parts(diff: &str) -> (String, Vec<Part>) {
    let mut head = String::new();
    let mut parts: Vec<Part> = Vec::new();
    let (mut old, mut new) = (0, 0);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            let (range, title) = rest.split_once("@@").unwrap_or((rest, ""));
            let (old_start, new_start) = (hunk_start(range, '-'), hunk_start(range, '+'));
            old = old_start;
            new = new_start;
            parts.push(Part {
                title: title.trim().to_string(),
                old_start,
                new_start,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(part) = parts.last_mut() else {
            head.push_str(line);
            head.push('\n');
            continue;
        };
        let kind = line.chars().next().unwrap_or(' ');
        let text = line.get(1..).unwrap_or("").to_string();
        let (o, n) = match kind {
            '+' => {
                new += 1;
                (0, new - 1)
            }
            '-' => {
                old += 1;
                (old - 1, 0)
            }
            '\\' => (0, 0),
            _ => {
                old += 1;
                new += 1;
                (old - 1, new - 1)
            }
        };
        part.lines.push(PartLine {
            kind,
            text,
            old: o,
            new: n,
        });
    }
    (head, parts)
}

/// The folder's changed files against HEAD, each split into parts.
pub fn file_parts(dir: &str) -> Vec<FileParts> {
    changes(dir)
        .into_iter()
        .map(|change| {
            let diff = if change.binary {
                String::new()
            } else {
                file_diff_context(dir, &change.path, false, "HEAD")
            };
            let (head, parts) = parse_parts(&diff);
            FileParts {
                change,
                head,
                parts,
                diff,
            }
        })
        .collect()
}

/// A patch with only the picked lines of a file's change. Unpicked removals stay as
/// unchanged lines and unpicked additions are left out, so the patch applies to HEAD's
/// version. None when nothing is picked.
pub fn picked_patch(file: &FileParts, picked: impl Fn(usize, usize) -> bool) -> Option<String> {
    let mut out = String::new();
    let mut shift: isize = 0;
    for (p, part) in file.parts.iter().enumerate() {
        let mut body = String::new();
        let (mut olds, mut news, mut any) = (0usize, 0usize, false);
        let mut kept_last = true;
        for (l, line) in part.lines.iter().enumerate() {
            let kind = match line.kind {
                '+' if picked(p, l) => '+',
                '+' => {
                    kept_last = false;
                    continue;
                }
                '-' if picked(p, l) => '-',
                '-' => ' ',
                '\\' => {
                    if kept_last {
                        body.push_str("\\ No newline at end of file\n");
                    }
                    continue;
                }
                _ => ' ',
            };
            any |= kind != ' ';
            kept_last = true;
            if kind != '+' {
                olds += 1;
            }
            if kind != '-' {
                news += 1;
            }
            body.push(kind);
            body.push_str(&line.text);
            body.push('\n');
        }
        if any {
            let new_start = (part.old_start as isize + shift).max(0) as usize;
            out.push_str(&format!(
                "@@ -{},{olds} +{new_start},{news} @@\n{body}",
                part.old_start
            ));
            shift += news as isize - olds as isize;
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(format!("{}{out}", file.head))
}

/// Commit some of the folder's changes: whole files, and patches of picked lines, built
/// in an index of its own so nothing else in the folder is touched. With `amend` they
/// join the last commit instead, keeping its message when `message` is empty.
pub fn commit_parts(
    dir: &str,
    whole: &[String],
    patch: &str,
    message: &str,
    amend: bool,
) -> Result<String, String> {
    let git_dir = git(dir, &["rev-parse", "--absolute-git-dir"])?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let index = format!("{}/runo-index-{nanos}", git_dir.trim());
    let env = [("GIT_INDEX_FILE", index.as_str()), ("GIT_EDITOR", "true")];
    let result = (|| {
        if rev(dir, "HEAD").is_some() {
            git_with(dir, &["read-tree", "HEAD"], &env, None)?;
        } else {
            git_with(dir, &["read-tree", "--empty"], &env, None)?;
        }
        if !whole.is_empty() {
            let mut args = vec!["add", "-A", "--"];
            args.extend(whole.iter().map(String::as_str));
            git_with(dir, &args, &env, None)?;
        }
        if !patch.is_empty() {
            git_with(
                dir,
                &["apply", "--cached", "--recount", "--whitespace=nowarn", "-"],
                &env,
                Some(patch.as_bytes()),
            )
            .map_err(|e| format!("Couldn't pick those lines out: {}", e.trim()))?;
        }
        let mut args = vec!["commit", "-q"];
        if amend {
            args.push("--amend");
        }
        if message.trim().is_empty() {
            args.push("--no-edit");
        } else {
            args.extend(["-m", message]);
        }
        git_with(dir, &args, &env, None)
    })();
    let _ = std::fs::remove_file(&index);
    result?;
    // The folder's own index catches up on the files committed, so what's left shows as
    // still to commit.
    let mut paths: Vec<String> = whole.to_vec();
    paths.extend(patch.lines().filter_map(|l| {
        l.strip_prefix("+++ b/")
            .or_else(|| l.strip_prefix("--- a/"))
            .map(str::to_string)
    }));
    paths.sort();
    paths.dedup();
    if !paths.is_empty() {
        let mut args = vec!["reset", "-q", "--"];
        args.extend(paths.iter().map(String::as_str));
        let _ = git(dir, &args);
    }
    git(dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string())
}

/// Throw a patch's changes out of the folder. Applying the patch again puts them back.
pub fn throw_away(dir: &str, patch: &str) -> Result<(), String> {
    git_with(
        dir,
        &["apply", "-R", "--recount", "--whitespace=nowarn", "-"],
        &[],
        Some(patch.as_bytes()),
    )
    .map(|_| ())
}

/// The last commit's subject, and whether it is only on this computer, so it can still
/// be added to.
pub fn last_commit(dir: &str) -> Option<(String, bool)> {
    let subject = git(dir, &["log", "-1", "--format=%s"])
        .ok()?
        .trim()
        .to_string();
    if subject.is_empty() {
        return None;
    }
    let sent = git(dir, &["branch", "-r", "--contains", "HEAD"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    Some((subject, !sent))
}

/// Every file git knows or would add (tracked plus untracked, minus ignored).
pub fn tracked_files(dir: &str) -> Vec<String> {
    git(
        dir,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )
    .map(|out| {
        let mut files = split_z(&out);
        // A file both staged and changed is listed once.
        files.dedup();
        files
    })
    .unwrap_or_default()
}

/// One line that matched a search in files.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

/// Search tracked and untracked text files for `query` (plain text, any case). Returns
/// at most `limit` hits and whether there were more.
pub fn grep(dir: &str, query: &str, limit: usize) -> (Vec<Hit>, bool) {
    if query.trim().is_empty() {
        return (Vec::new(), false);
    }
    // Exit status 1 means no match.
    let out = git(
        dir,
        &[
            "grep",
            "-z",
            "-n",
            "-I",
            "-i",
            "-F",
            "--untracked",
            "-e",
            query,
        ],
    )
    .unwrap_or_default();
    let mut hits = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(3, '\0');
        if let (Some(path), Some(n), Some(text)) = (parts.next(), parts.next(), parts.next()) {
            if hits.len() == limit {
                return (hits, true);
            }
            hits.push(Hit {
                path: path.to_string(),
                line: n.parse().unwrap_or(0),
                text: text.trim().chars().take(200).collect(),
            });
        }
    }
    (hits, false)
}

/// Copy git-ignored files that match the repository's `.worktreeinclude` patterns
/// (gitignore syntax, e.g. `.env` or `config/*.local.json`) into a new worktree, so
/// it has the local secrets and config a fresh checkout lacks. Returns the files copied.
pub fn copy_included(repo: &str, worktree: &str) -> Result<Vec<String>, String> {
    let include = Path::new(repo).join(".worktreeinclude");
    if !include.is_file() {
        return Ok(Vec::new());
    }
    let include = include.to_string_lossy().into_owned();
    // Ignored files matching the include patterns: `--exclude-standard` alone lists what
    // is ignored, and the include file narrows it, in one walk of the tree.
    let listed = git(
        repo,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
        ],
    )?;
    let patterns = git(
        repo,
        &[
            "ls-files",
            "--others",
            "--ignored",
            &format!("--exclude-from={include}"),
            "-z",
        ],
    )?;
    let ignored: Vec<String> = split_z(&listed);
    let mut copied = Vec::new();
    for path in split_z(&patterns) {
        // Only files that git ignores qualify: tracked files are already in the worktree.
        let is_ignored = ignored
            .iter()
            .any(|i| *i == path || (i.ends_with('/') && path.starts_with(i.as_str())));
        if !is_ignored {
            continue;
        }
        let dest = Path::new(worktree).join(&path);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(Path::new(repo).join(&path), &dest).map_err(|e| format!("{path}: {e}"))?;
        copied.push(path);
    }
    Ok(copied)
}

/// Throw away the working-tree and staged changes to one file. Files that are new
/// since the last commit are deleted.
pub fn discard(dir: &str, path: &str) -> Result<(), String> {
    if git(dir, &["cat-file", "-e", &format!("HEAD:{path}")]).is_ok() {
        git(
            dir,
            &[
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "--",
                path,
            ],
        )
        .map(|_| ())
    } else {
        git(
            dir,
            &["rm", "-q", "--cached", "--ignore-unmatch", "--", path],
        )?;
        match std::fs::remove_file(Path::new(dir).join(path)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
            _ => Ok(()),
        }
    }
}

/// Add one file's changes to the index.
pub fn stage(dir: &str, path: &str) -> Result<(), String> {
    git(dir, &["add", "-A", "--", path]).map(|_| ())
}

/// Take one file's changes out of the index, leaving the working tree alone.
pub fn unstage(dir: &str, path: &str) -> Result<(), String> {
    if exists(dir, "HEAD") {
        git(dir, &["reset", "-q", "HEAD", "--", path]).map(|_| ())
    } else {
        git(dir, &["rm", "-q", "--cached", "--", path]).map(|_| ())
    }
}

/// Commit what is staged, or every change when nothing is. Returns the short hash of
/// the new commit.
pub fn commit(dir: &str, message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first.".into());
    }
    let staged = git(dir, &["diff", "--cached", "--quiet"]).is_err();
    if !staged {
        git(dir, &["add", "-A"])?;
    }
    git(dir, &["commit", "-q", "-m", message])?;
    git(dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string())
}

/// Run git with a private index file, so snapshots never touch the user's staging area.
fn git_index(dir: &str, index: &Path, args: &[&str]) -> Result<String, String> {
    let index = index.to_string_lossy();
    git_with(dir, args, &[("GIT_INDEX_FILE", index.as_ref())], None)
}

fn temp_index(dir: &str) -> std::path::PathBuf {
    let git_dir = git(dir, &["rev-parse", "--absolute-git-dir"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Path::new(&git_dir).join(format!("runo-index-{}-{nanos}", std::process::id()))
}

/// Tree id of the whole working tree (tracked and untracked, minus ignored files).
fn snapshot_tree(dir: &str) -> Result<String, String> {
    let index = temp_index(dir);
    // Start from HEAD's index when there is one, so unchanged files cost nothing.
    let _ = git_index(dir, &index, &["read-tree", "HEAD"]);
    let tree = git_index(dir, &index, &["add", "-A", "--", "."])
        .and_then(|_| git_index(dir, &index, &["write-tree"]));
    let _ = std::fs::remove_file(&index);
    Ok(tree?.trim().to_string())
}

/// Tree id of the user's index, read from a copy so the real one is never rewritten.
fn index_tree(dir: &str) -> Option<String> {
    let real = git(dir, &["rev-parse", "--git-path", "index"]).ok()?;
    let real = Path::new(dir).join(real.trim());
    let copy = temp_index(dir);
    if real.exists() {
        std::fs::copy(&real, &copy).ok()?;
    }
    let tree = git_index(dir, &copy, &["write-tree"]);
    let _ = std::fs::remove_file(&copy);
    tree.ok().map(|t| t.trim().to_string())
}

fn commit_tree(dir: &str, tree: &str, parents: &[&str], message: &str) -> Result<String, String> {
    let mut args = vec!["commit-tree", tree, "-m", message];
    for p in parents {
        args.extend(["-p", p]);
    }
    let who = [
        ("GIT_AUTHOR_NAME", "Runo"),
        ("GIT_AUTHOR_EMAIL", "runo@localhost"),
        ("GIT_COMMITTER_NAME", "Runo"),
        ("GIT_COMMITTER_EMAIL", "runo@localhost"),
    ];
    git_with(dir, &args, &who, None).map(|s| s.trim().to_string())
}

/// Snapshot the working tree as a commit that no branch points at, so a turn can be
/// undone (or an undo redone). Its first parent is HEAD at the time and its second a
/// commit of the index, so both come back too. Kept alive by a ref under `refs/runo/`.
pub fn checkpoint(dir: &str) -> Result<String, String> {
    let tree = snapshot_tree(dir)?;
    let head = rev(dir, "HEAD");
    let index = index_tree(dir)
        .map(|t| commit_tree(dir, &t, &[], "runo index"))
        .transpose()?;
    let mut parents: Vec<&str> = Vec::new();
    let message = match &head {
        Some(head) => {
            parents.push(head);
            "runo checkpoint"
        }
        None => "runo checkpoint\n\nno-head",
    };
    if let Some(index) = &index {
        parents.push(index);
    }
    let commit = commit_tree(dir, &tree, &parents, message)?;
    let _ = git(
        dir,
        &[
            "update-ref",
            &format!("refs/runo/checkpoints/{commit}"),
            &commit,
        ],
    );
    Ok(commit)
}

/// HEAD and index commit recorded in a checkpoint. Older checkpoints have no index.
fn checkpoint_parts(dir: &str, checkpoint: &str) -> (Option<String>, Option<String>) {
    let parents: Vec<String> = git(dir, &["rev-list", "--parents", "-n", "1", checkpoint])
        .unwrap_or_default()
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect();
    let headless = git(dir, &["log", "-1", "--format=%B", checkpoint])
        .unwrap_or_default()
        .contains("no-head");
    if headless {
        (None, parents.first().cloned())
    } else {
        (parents.first().cloned(), parents.get(1).cloned())
    }
}

/// Paths that differ between two trees or commits.
fn diff_names(dir: &str, a: &str, b: &str) -> Result<Vec<String>, String> {
    git(dir, &["diff", "--name-only", "--no-renames", "-z", a, b]).map(|s| split_z(&s))
}

/// Files that differ between a checkpoint and the working tree now.
pub fn changed_since(dir: &str, checkpoint: &str) -> Vec<String> {
    let Ok(tree) = snapshot_tree(dir) else {
        return Vec::new();
    };
    diff_names(dir, checkpoint, &tree).unwrap_or_default()
}

/// The patch of everything that changed since a checkpoint, files made since included,
/// cut to about `limit` bytes.
pub fn patch_since(dir: &str, checkpoint: &str, limit: usize) -> String {
    let Ok(tree) = snapshot_tree(dir) else {
        return String::new();
    };
    let mut text = git(dir, &["diff", "--stat", checkpoint, &tree]).unwrap_or_default();
    text.push('\n');
    text.push_str(&git(dir, &["diff", checkpoint, &tree]).unwrap_or_default());
    if text.len() > limit {
        let mut cut = limit;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n[diff truncated]");
    }
    text
}

/// What a restore did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Restored {
    /// Files put back.
    pub files: Vec<String>,
    /// Files left alone because they changed after `guard`.
    pub skipped: Vec<String>,
    /// The branch was moved back (or forward) to where it was.
    pub moved_head: bool,
}

/// Put the working tree back as it was at `checkpoint`: changed files are restored with
/// their modes and symlinks, files created since are deleted, and the index entries of
/// those files are restored too. Ignored files are left alone. When the branch gained
/// or lost commits since, it is moved back to the checkpoint's HEAD.
///
/// With `guard` (a later snapshot, such as the end of the last turn) only the files
/// that differ between the checkpoint and the guard are touched, and of those only the
/// ones nobody changed after it; the rest are reported as skipped.
pub fn restore(dir: &str, checkpoint: &str, guard: Option<&str>) -> Result<Restored, String> {
    let now = snapshot_tree(dir)?;
    let mut done = Restored::default();
    let paths = match guard {
        Some(guard) => {
            let later = diff_names(dir, guard, &now)?;
            let mut paths = Vec::new();
            for path in diff_names(dir, checkpoint, guard)? {
                if later.contains(&path) {
                    done.skipped.push(path);
                } else {
                    paths.push(path);
                }
            }
            paths
        }
        None => diff_names(dir, checkpoint, &now)?,
    };
    let (head, index) = checkpoint_parts(dir, checkpoint);
    // Move the branch only along its own history, never onto another line of work.
    if let (Some(target), Some(current)) = (head.as_deref(), rev(dir, "HEAD")) {
        if target != current
            && (is_ancestor(dir, target, &current) || is_ancestor(dir, &current, target))
        {
            git(dir, &["reset", "-q", "--soft", target])?;
            done.moved_head = true;
        }
    }
    if paths.is_empty() {
        return Ok(done);
    }
    let in_checkpoint = |source: &str| -> Result<Vec<(String, String)>, String> {
        // `mode type sha<TAB>path` for every file in the tree.
        let listing = git(dir, &["ls-tree", "-r", "-z", source])?;
        Ok(listing
            .split('\0')
            .filter_map(|e| e.split_once('\t'))
            .map(|(meta, path)| (path.to_string(), meta.to_string()))
            .collect())
    };
    let wanted: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
    let saved: std::collections::HashMap<String, String> = in_checkpoint(checkpoint)?
        .into_iter()
        .filter(|(p, _)| wanted.contains(p.as_str()))
        .collect();
    // Files the checkpoint has come back through a private index, so git writes the
    // executable bit and symlinks exactly as they were.
    let restore_index = temp_index(dir);
    let back: Vec<&String> = paths.iter().filter(|p| saved.contains_key(*p)).collect();
    if !back.is_empty() {
        git_index(dir, &restore_index, &["read-tree", checkpoint])?;
        let mut list = Vec::new();
        for path in &back {
            let target = Path::new(dir).join(path);
            // A folder or link where a file was would block the checkout.
            if target.is_dir() && !target.is_symlink() {
                let _ = std::fs::remove_dir_all(&target);
            } else {
                let _ = std::fs::remove_file(&target);
            }
            list.extend_from_slice(path.as_bytes());
            list.push(0);
        }
        let index = restore_index.to_string_lossy().into_owned();
        let result = git_with(
            dir,
            &["checkout-index", "-f", "-z", "--stdin"],
            &[("GIT_INDEX_FILE", index.as_str())],
            Some(&list),
        );
        let _ = std::fs::remove_file(&restore_index);
        result?;
    }
    for path in paths.iter().filter(|p| !saved.contains_key(*p)) {
        let target = Path::new(dir).join(path);
        let _ = std::fs::remove_file(&target);
        // Drop folders the removed files leave empty.
        let mut parent = target.parent();
        while let Some(p) = parent.filter(|p| *p != Path::new(dir)) {
            if std::fs::remove_dir(p).is_err() {
                break;
            }
            parent = p.parent();
        }
    }
    // The index entries of these files go back to the checkpoint's index (or to HEAD
    // for older checkpoints); every other file keeps the user's staging.
    let source = index.or(head);
    let staged: std::collections::HashMap<String, String> = match &source {
        Some(source) => in_checkpoint(source)?
            .into_iter()
            .filter(|(p, _)| wanted.contains(p.as_str()))
            .collect(),
        None => Default::default(),
    };
    let mut info = Vec::new();
    for path in &paths {
        let entry = match staged.get(path) {
            // `mode blob sha` becomes `mode sha`.
            Some(meta) => {
                let parts: Vec<&str> = meta.split_whitespace().collect();
                format!("{} {}\t{path}", parts[0], parts.get(2).unwrap_or(&""))
            }
            None => format!("0 {}\t{path}", "0".repeat(40)),
        };
        info.extend_from_slice(entry.as_bytes());
        info.push(0);
    }
    git_with(
        dir,
        &["update-index", "-z", "--index-info"],
        &[],
        Some(&info),
    )?;
    done.files = paths;
    Ok(done)
}

/// The changes as one patch for writing a commit message: the diff against HEAD,
/// plus the names of new files, cut to `limit` characters.
pub fn patch_for_message(dir: &str, limit: usize) -> String {
    let mut text = git(dir, &["diff", "HEAD", "--stat"]).unwrap_or_default();
    let untracked = git(dir, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    if !untracked.trim().is_empty() {
        text.push_str("\nNew files:\n");
        text.push_str(&untracked);
    }
    text.push('\n');
    text.push_str(&git(dir, &["diff", "HEAD"]).unwrap_or_default());
    if text.len() > limit {
        let mut cut = limit;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n[diff truncated]");
    }
    text
}

/// How the branch relates to its remote.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Sync {
    /// The remote to push to, empty when the repository has none.
    pub remote: String,
    /// Whether the branch already tracks a remote branch.
    pub upstream: bool,
    /// Commits that are not on the remote yet.
    pub ahead: usize,
}

pub fn sync_state(dir: &str) -> Sync {
    let remote = remote_name(dir);
    let count = |args: &[&str]| count(dir, args);
    let upstream_ahead = count(&["rev-list", "--count", "@{u}..HEAD"]);
    let ahead = match upstream_ahead {
        Some(n) => n,
        // No upstream yet: count the commits no remote branch has.
        None if !remote.is_empty() => {
            count(&["rev-list", "--count", "HEAD", "--not", "--remotes"]).unwrap_or(0)
        }
        None => 0,
    };
    Sync {
        remote,
        upstream: upstream_ahead.is_some(),
        ahead,
    }
}

/// Where a project folder stands: its branch and how it compares with the remote's copy.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct RepoState {
    /// The checked-out branch, empty when HEAD is detached.
    pub branch: String,
    /// The remote branch it tracks, such as `origin/main`; empty when there is none.
    pub upstream: String,
    /// The remote to push to, empty when the repository has none.
    pub remote: String,
    /// The remote as a person would call it: "GitHub" for one there, else its name.
    pub host: String,
    /// Commits only here, and commits only on the remote branch.
    pub ahead: usize,
    pub behind: usize,
    /// Local branches, the checked-out one included, most recently committed first.
    pub branches: Vec<String>,
}

pub fn repo_state(dir: &str) -> RepoState {
    let branch = match current_branch(dir) {
        b if b == "HEAD" => String::new(),
        b => b,
    };
    let upstream = git(
        dir,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .map(|s| s.trim().to_string())
    .unwrap_or_default();
    let remote = remote_name(dir);
    let host = match git(dir, &["remote", "get-url", &remote]) {
        Ok(url) if url.contains("github.com") => "GitHub".to_string(),
        _ => remote.clone(),
    };
    let (behind, ahead) = if upstream.is_empty() {
        let ahead = if remote.is_empty() {
            0
        } else {
            count(dir, &["rev-list", "--count", "HEAD", "--not", "--remotes"]).unwrap_or(0)
        };
        (0, ahead)
    } else {
        git(dir, &["rev-list", "--left-right", "--count", "@{u}...HEAD"])
            .ok()
            .and_then(|s| {
                let mut n = s.split_whitespace().filter_map(|n| n.parse().ok());
                Some((n.next()?, n.next()?))
            })
            .unwrap_or((0, 0))
    };
    let branches = git(
        dir,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )
    .unwrap_or_default()
    .lines()
    .map(str::to_string)
    .collect();
    RepoState {
        branch,
        upstream,
        remote,
        host,
        ahead,
        behind,
        branches,
    }
}

/// Bring the remote's new commits into the checked-out branch, replaying local commits
/// after them and keeping uncommitted work. On clashing lines nothing changes: the
/// update is undone and the clashing files are returned.
pub fn pull(dir: &str) -> Result<Update, String> {
    fetch(dir)?;
    if rev(dir, "@{u}").is_none() {
        return Err(
            "This branch is not on the remote yet, so there is nothing to bring in. Send it first."
                .into(),
        );
    }
    // Clashing lines leave the rebase stopped, for the clash page to sort out.
    update_from(dir, "@{u}", true)
}

/// Check out another local branch, or a remote one under its own name. Uncommitted work
/// comes along when it does not clash with the other branch.
pub fn switch(dir: &str, branch: &str) -> Result<(), String> {
    git(dir, &["switch", branch]).map(|_| ())
}

/// How to take a git step back.
#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    /// Put the folder back on this branch (empty when HEAD was detached), at this commit.
    Folder { branch: String, head: String },
    /// Point these branches back at these commits; `None` deletes one that the step made.
    Branches(Vec<(String, Option<String>)>),
    /// Give a renamed branch its old name back.
    Rename { from: String, to: String },
    /// Put a dropped set-aside back in the list.
    Restash { commit: String, message: String },
    /// Bring back what was just set aside.
    Unstash,
    /// Remove a tag the step made.
    Untag(String),
    /// Take the last commit back off the branch, keeping its changes in the folder.
    Uncommit { head: String },
    /// Put back a part that was thrown away, applying its patch to the folder again.
    Reapply(String),
    /// Put the folder's files back as they were in this checkpoint.
    Restore(String),
    /// Several of these, in order.
    Steps(Vec<Undo>),
}

/// A step taken in a folder, with how to take it back.
#[derive(Clone, Debug, PartialEq)]
pub struct UndoPoint {
    pub dir: String,
    /// The step, in words: "Brought in 3 commits".
    pub label: String,
    pub undo: Undo,
}

/// Keep `commit` under `refs/runo/undo`, so git's clean-up never drops it while an undo
/// can still reach it.
fn keep_for_undo(dir: &str, commit: &str) {
    let _ = git(dir, &["update-ref", "refs/runo/undo", commit]);
}

/// Where `dir` is now, to come back to.
pub fn folder_point(dir: &str) -> Option<Undo> {
    let head = rev(dir, "HEAD")?;
    keep_for_undo(dir, &head);
    Some(Undo::Folder {
        branch: match current_branch(dir) {
            b if b == "HEAD" => String::new(),
            b => b,
        },
        head,
    })
}

/// Where these branches point now (`None` for one that does not exist yet), to come
/// back to.
pub fn branches_point(dir: &str, names: &[&str]) -> Undo {
    Undo::Branches(
        names
            .iter()
            .map(|name| {
                let head = rev(dir, &format!("refs/heads/{name}"));
                if let Some(head) = &head {
                    keep_for_undo(dir, head);
                }
                (name.to_string(), head)
            })
            .collect(),
    )
}

/// Take a step back. Uncommitted work is kept, and git refuses rather than overwrite
/// any of it.
pub fn undo(point: &UndoPoint) -> Result<(), String> {
    let dir = point.dir.as_str();
    match &point.undo {
        Undo::Folder { branch, head } => {
            if !branch.is_empty() && current_branch(dir) != *branch {
                switch(dir, branch)?;
            }
            if rev(dir, "HEAD").as_deref() != Some(head.as_str()) {
                git(dir, &["reset", "--keep", head])?;
            }
        }
        Undo::Branches(list) => {
            for (name, head) in list {
                let checked_out = checked_out_at(dir, name);
                match (head, checked_out) {
                    (Some(head), Some(at)) => {
                        git(&at, &["reset", "--keep", head])?;
                    }
                    (Some(head), None) => {
                        git(dir, &["update-ref", &format!("refs/heads/{name}"), head])?;
                    }
                    (None, Some(_)) => {
                        return Err(format!(
                            "{name} is checked out, so it can't be removed. Switch away from it first."
                        ));
                    }
                    (None, None) => {
                        if has_branch(dir, name) {
                            delete_branch(dir, name, true)?;
                        }
                    }
                }
            }
        }
        Undo::Rename { from, to } => {
            git(dir, &["branch", "-m", to, from])?;
        }
        Undo::Restash { commit, message } => {
            git(dir, &["stash", "store", "-m", message, commit])?;
        }
        Undo::Unstash => {
            stash_apply(dir, 0)?;
        }
        Undo::Untag(name) => {
            delete_tag(dir, name)?;
        }
        Undo::Uncommit { head } => {
            git(dir, &["reset", "-q", "--mixed", head])?;
        }
        Undo::Reapply(patch) => {
            git_with(
                dir,
                &["apply", "--recount", "--whitespace=nowarn", "-"],
                &[],
                Some(patch.as_bytes()),
            )?;
        }
        Undo::Restore(checkpoint) => {
            restore(dir, checkpoint, None)?;
        }
        Undo::Steps(steps) => {
            for undo in steps {
                self::undo(&UndoPoint {
                    dir: point.dir.clone(),
                    label: point.label.clone(),
                    undo: undo.clone(),
                })?;
            }
        }
    }
    Ok(())
}

/// One branch in the Branches list.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct BranchRow {
    pub name: String,
    /// Only the remote has it.
    pub remote_only: bool,
    /// Commits it has that the base lacks, and the other way round.
    pub ahead: usize,
    pub behind: usize,
    /// The remote branch it tracks, empty when none.
    pub upstream: String,
    pub subject: String,
    /// When its last commit was made, as git says it: "3 hours ago".
    pub when: String,
    /// The folder it is checked out in, if any.
    pub checked_out: Option<String>,
}

/// Every local branch, then the remote's that have no local copy, each against `base`,
/// most recently committed first.
pub fn branch_rows(dir: &str, base: &str) -> Vec<BranchRow> {
    let base_ref = base_ref(dir, base);
    let remote = remote_name(dir);
    let mut refs = vec!["refs/heads".to_string()];
    if !remote.is_empty() {
        refs.push(format!("refs/remotes/{remote}"));
    }
    let mut args = vec![
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname)%00%(upstream:short)%00%(subject)%00%(committerdate:relative)",
    ];
    args.extend(refs.iter().map(String::as_str));
    let out = git(dir, &args).unwrap_or_default();
    let worktrees = worktree_branches(dir);
    let mut rows: Vec<BranchRow> = Vec::new();
    let prefix = format!("refs/remotes/{remote}/");
    for line in out.lines() {
        let mut f = line.split('\0');
        let (Some(full), Some(upstream), Some(subject), Some(when)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let (name, remote_only) = if let Some(n) = full.strip_prefix("refs/heads/") {
            (n.to_string(), false)
        } else if let Some(n) = full.strip_prefix(prefix.as_str()) {
            if n == "HEAD" {
                continue;
            }
            (n.to_string(), true)
        } else {
            continue;
        };
        if remote_only && rows.iter().any(|r| r.name == name) {
            continue;
        }
        let rev = if remote_only {
            full.to_string()
        } else {
            format!("refs/heads/{name}")
        };
        let (behind, ahead) = if base_ref.is_empty() {
            (0, 0)
        } else {
            git(
                dir,
                &[
                    "rev-list",
                    "--left-right",
                    "--count",
                    &format!("{base_ref}...{rev}"),
                ],
            )
            .ok()
            .and_then(|s| {
                let mut n = s.split_whitespace().filter_map(|n| n.parse().ok());
                Some((n.next()?, n.next()?))
            })
            .unwrap_or((0, 0))
        };
        rows.push(BranchRow {
            checked_out: worktrees
                .iter()
                .find(|(_, b)| *b == name && !remote_only)
                .map(|(d, _)| d.clone()),
            name,
            remote_only,
            ahead,
            behind,
            upstream: upstream.to_string(),
            subject: subject.to_string(),
            when: when.to_string(),
        });
    }
    // A remote branch listed before its local copy was read: drop it now.
    let local: Vec<String> = rows
        .iter()
        .filter(|r| !r.remote_only)
        .map(|r| r.name.clone())
        .collect();
    rows.retain(|r| !r.remote_only || !local.contains(&r.name));
    rows
}

/// Every worktree's folder with the branch checked out there.
fn worktree_branches(dir: &str) -> Vec<(String, String)> {
    let Ok(list) = git(dir, &["worktree", "list", "--porcelain", "-z"]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut path = None;
    for field in list.split('\0') {
        if let Some(p) = field.strip_prefix("worktree ") {
            path = Some(p.to_string());
        } else if let Some(b) = field.strip_prefix("branch refs/heads/") {
            if let Some(p) = path.clone() {
                out.push((p, b.to_string()));
            }
        }
    }
    out
}

/// One commit in a list.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct CommitRow {
    pub short: String,
    pub subject: String,
    pub author: String,
    pub when: String,
}

/// Commits on `branch` that `base` lacks, newest first.
pub fn commits_only_on(dir: &str, branch: &str, base: &str, limit: usize) -> Vec<CommitRow> {
    let limit = format!("-n{limit}");
    git(
        dir,
        &[
            "log",
            &limit,
            "--format=%h%x00%s%x00%an%x00%cr",
            &format!("{base}..{branch}"),
        ],
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|line| {
        let mut f = line.split('\0');
        Some(CommitRow {
            short: f.next()?.to_string(),
            subject: f.next()?.to_string(),
            author: f.next()?.to_string(),
            when: f.next()?.to_string(),
        })
    })
    .collect()
}

/// Files `branch` changes since it left `base`.
pub fn branch_changes(dir: &str, branch: &str, base: &str) -> Vec<Change> {
    let range = format!("{base}...{branch}");
    let stats = numstat(
        &git(dir, &["diff", "--numstat", "--no-renames", "-z", &range]).unwrap_or_default(),
    );
    git(
        dir,
        &["diff", "--name-status", "--no-renames", "-z", &range],
    )
    .unwrap_or_default()
    .split('\0')
    .collect::<Vec<_>>()
    .chunks(2)
    .filter_map(|pair| {
        let (status, path) = (pair.first()?, pair.get(1)?);
        if status.is_empty() {
            return None;
        }
        let (added, removed, binary) = stats.get(*path).copied().unwrap_or_default();
        Some(Change {
            path: path.to_string(),
            status: status.chars().next()?.to_string(),
            added,
            removed,
            binary,
            ..Default::default()
        })
    })
    .collect()
}

/// Start a branch at `start` without switching to it.
pub fn new_branch(dir: &str, name: &str, start: &str) -> Result<(), String> {
    git(dir, &["branch", name, start]).map(|_| ())
}

/// A local copy of a branch only the remote has, tracking it.
pub fn get_remote_branch(dir: &str, name: &str) -> Result<(), String> {
    let remote = remote_name(dir);
    git(
        dir,
        &["branch", "--track", name, &format!("{remote}/{name}")],
    )
    .map(|_| ())
}

/// Send a branch to the remote under its own name.
pub fn push_branch(dir: &str, remote: &str, branch: &str) -> Result<(), String> {
    git(dir, &["push", remote, branch])
        .map(|_| ())
        .map_err(|e| auth_hint(&e))
}

/// What a branch changes against `base`, commit subjects first, cut at `max` bytes, for
/// an agent to read.
pub fn branch_patch(dir: &str, branch: &str, base: &str, max: usize) -> String {
    let range = format!("{base}...{branch}");
    let subjects = git(
        dir,
        &[
            "log",
            "--reverse",
            "--format=- %s",
            &format!("{base}..{branch}"),
        ],
    )
    .unwrap_or_default();
    let stat = git(dir, &["diff", "--stat", &range]).unwrap_or_default();
    let mut diff = git(dir, &["diff", "--no-color", &range]).unwrap_or_default();
    if diff.len() > max {
        let mut cut = max;
        while !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        diff.truncate(cut);
        diff.push_str("\n… (cut short)");
    }
    format!("Commits:\n{subjects}\nFiles:\n{stat}\n{diff}")
}

/// Split a branch's work into a stack of new branches, one per part, each built on the
/// one before and holding its part's files as they are at `tip`. The last branch has
/// every file the parts name. Nothing checked out is touched. Returns the new names.
pub fn stack_parts(
    dir: &str,
    base: &str,
    tip: &str,
    prefix: &str,
    parts: &[(String, Vec<String>)],
) -> Result<Vec<String>, String> {
    let git_dir = git(dir, &["rev-parse", "--absolute-git-dir"])?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let index = format!("{}/runo-split-{nanos}", git_dir.trim());
    let env = [("GIT_INDEX_FILE", index.as_str())];
    let start = git(dir, &["merge-base", base, tip])?.trim().to_string();
    let mut parent = start.clone();
    let mut names = Vec::new();
    let result = (|| {
        git_with(dir, &["read-tree", &start], &env, None)?;
        for (i, (title, files)) in parts.iter().enumerate() {
            for path in files {
                let entry = git(dir, &["ls-tree", tip, "--", path])?;
                match entry.split_once('\t') {
                    Some((meta, _)) => {
                        let f: Vec<&str> = meta.split_whitespace().collect();
                        let info = format!("{},{},{path}", f[0], f[2]);
                        git_with(
                            dir,
                            &["update-index", "--add", "--cacheinfo", &info],
                            &env,
                            None,
                        )?;
                    }
                    // Gone at the tip: this part deletes it.
                    None => {
                        git_with(
                            dir,
                            &["update-index", "--force-remove", "--", path],
                            &env,
                            None,
                        )?;
                    }
                }
            }
            let tree = git_with(dir, &["write-tree"], &env, None)?;
            let commit = git(
                dir,
                &["commit-tree", tree.trim(), "-p", &parent, "-m", title],
            )?;
            let name = format!("{prefix}-part-{}", i + 1);
            git(dir, &["branch", "-f", &name, commit.trim()])?;
            names.push(name);
            parent = commit.trim().to_string();
        }
        Ok::<(), String>(())
    })();
    let _ = std::fs::remove_file(&index);
    result.map(|_| names)
}

/// Open a pull request for `branch` onto `base` with the GitHub CLI, returning its link.
pub fn open_pull_request(
    dir: &str,
    branch: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<String, String> {
    let out = Command::new("gh")
        .args([
            "pr", "create", "--head", branch, "--base", base, "--title", title, "--body", body,
        ])
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("The GitHub CLI (gh) is needed for pull requests: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(text
        .lines()
        .rev()
        .find(|l| l.starts_with("https://"))
        .unwrap_or(text.trim())
        .to_string())
}

pub fn rename_branch(dir: &str, from: &str, to: &str) -> Result<(), String> {
    git(dir, &["branch", "-m", from, to]).map(|_| ())
}

/// How a branch's commits land on the base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeHow {
    /// One new commit with all the branch's changes.
    Squash,
    /// The branch's commits plus a merge commit.
    Merge,
    /// The branch's commits copied on top of the base, for a straight history.
    Rebase,
}

/// Land `branch` on `base` without touching the branch itself. Runs where `base` is
/// checked out, or in a temporary worktree. Clashing lines change nothing.
pub fn merge_branch(
    repo: &str,
    branch: &str,
    base: &str,
    how: MergeHow,
    message: &str,
) -> Result<String, String> {
    let editor = [("GIT_EDITOR", "true")];
    on_branch(repo, base, |dir| {
        let result = match how {
            MergeHow::Merge => git_with(dir, &["merge", "--no-edit", branch], &editor, None),
            MergeHow::Squash => git_with(dir, &["merge", "--squash", branch], &editor, None)
                .and_then(|_| git(dir, &["commit", "-q", "-m", message])),
            MergeHow::Rebase => {
                let start = git(dir, &["merge-base", "HEAD", branch])?;
                git_with(
                    dir,
                    &[
                        "cherry-pick",
                        "--allow-empty",
                        &format!("{}..{branch}", start.trim()),
                    ],
                    &editor,
                    None,
                )
            }
        };
        match result {
            Ok(_) => git(dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string()),
            Err(error) => {
                let files = conflicted(dir);
                // A merge in the project folder itself waits for the clash page.
                if how == MergeHow::Merge && !files.is_empty() && dir == repo {
                    return Err(format!(
                        "{branch} and {base} change the same lines in {}. Pick what to keep, then Finish.",
                        files.join(", ")
                    ));
                }
                let _ = match how {
                    MergeHow::Rebase => git(dir, &["cherry-pick", "--abort"]),
                    _ => git(dir, &["merge", "--abort"])
                        .or_else(|_| git(dir, &["reset", "--hard", "-q", "HEAD"])),
                };
                Err(if files.is_empty() {
                    error
                } else {
                    format!(
                        "{branch} and {base} change the same lines in {}, so nothing was merged. Bring {base}'s updates into {branch} first to sort it out there.",
                        files.join(", ")
                    )
                })
            }
        }
    })
}

/// Run `work` in a folder with `branch` checked out: where it already is, or a temporary
/// worktree. Refuses when that folder has uncommitted changes to tracked files.
fn on_branch<T>(
    repo: &str,
    branch: &str,
    work: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, String> {
    if !exists(repo, &format!("refs/heads/{branch}")) {
        // Only the remote has it: start a local branch there first.
        let remote = remote_name(repo);
        git(
            repo,
            &["branch", "--track", branch, &format!("{remote}/{branch}")],
        )?;
    }
    let (dir, temporary) = match checked_out_at(repo, branch) {
        Some(dir) => {
            let dirty = git(&dir, &["status", "--porcelain", "--untracked-files=no"])?;
            if !dirty.trim().is_empty() {
                return Err(format!(
                    "{branch} has changes that aren't committed yet. Commit or set them aside first."
                ));
            }
            (dir, false)
        }
        None => {
            let common = git(repo, &["rev-parse", "--git-common-dir"])?;
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = Path::new(repo)
                .join(common.trim())
                .join(format!("runo-merge-{nanos}"))
                .to_string_lossy()
                .into_owned();
            git(repo, &["worktree", "add", "--quiet", &dir, branch])?;
            (dir, true)
        }
    };
    let outcome = work(&dir);
    if temporary {
        let _ = remove_worktree(repo, &dir);
    }
    outcome
}

/// One commit in History, with the branch and tag names pointing at it.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct LogRow {
    pub sha: String,
    pub short: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub when: String,
    /// Names as git decorates them: "HEAD -> main", "origin/main", "tag: v1.0".
    pub refs: Vec<String>,
}

/// What History lists.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct LogFilter {
    /// Every branch, or only the checked-out one.
    pub all: bool,
    /// Words in the message or the author's name.
    pub text: String,
    /// Only commits that touch this file or folder.
    pub path: String,
}

/// Commits newest first, children before parents, at most `limit`. Found by words, they
/// come newest first only.
pub fn log(dir: &str, filter: &LogFilter, limit: usize) -> Vec<LogRow> {
    let format = "--format=%H%x00%h%x00%P%x00%s%x00%an%x00%cr%x00%D";
    let limit = format!("-n{limit}");
    let mut scope: Vec<&str> = Vec::new();
    if filter.all {
        scope.extend(["--branches", "--remotes", "--tags", "HEAD"]);
    }
    let mut path: Vec<&str> = Vec::new();
    if !filter.path.trim().is_empty() {
        path.extend(["--", filter.path.as_str()]);
    }
    let text = filter.text.trim();
    let out = if text.is_empty() {
        // --parents rewrites parents past commits a path filter leaves out, so the
        // graph stays joined up.
        let args: Vec<&str> = ["log", "--topo-order", "--parents", format, &limit]
            .into_iter()
            .chain(scope.iter().copied())
            .chain(path.iter().copied())
            .collect();
        git(dir, &args)
    } else {
        // Message or author: git ANDs --grep with --author, so ask for each and join them.
        let grep = format!("--grep={text}");
        let author = format!("--author={text}");
        let mut found = String::new();
        for by in [&grep, &author] {
            let args: Vec<&str> = ["rev-list", "-i", "--fixed-strings", by.as_str(), &limit]
                .into_iter()
                .chain(scope.iter().copied())
                .chain(if filter.all { None } else { Some("HEAD") })
                .chain(path.iter().copied())
                .collect();
            found.push_str(&git(dir, &args).unwrap_or_default());
        }
        if found.trim().is_empty() {
            return Vec::new();
        }
        git_with(
            dir,
            &["log", "--no-walk=sorted", "--stdin", format, &limit],
            &[],
            Some(found.as_bytes()),
        )
    };
    out.unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\0');
            Some(LogRow {
                sha: f.next()?.to_string(),
                short: f.next()?.to_string(),
                parents: f.next()?.split_whitespace().map(str::to_string).collect(),
                subject: f.next()?.to_string(),
                author: f.next()?.to_string(),
                when: f.next()?.to_string(),
                refs: f
                    .next()
                    .unwrap_or("")
                    .split(", ")
                    .filter(|r| !r.is_empty() && *r != "refs/runo/undo" && !r.ends_with("/HEAD"))
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
}

/// How one row of the commit graph is drawn: which lane its dot sits in, the lines
/// coming in from the row above and going out to the row below, as (from lane, to lane).
#[derive(Clone, Default, Debug, PartialEq)]
pub struct GraphRow {
    pub lane: usize,
    pub top: Vec<(usize, usize)>,
    pub bottom: Vec<(usize, usize)>,
}

/// Lay commits (children before parents) out in lanes. Each lane waits for one commit;
/// a commit takes the first lane waiting for it, the others waiting for it join it,
/// its first parent carries the lane on and other parents get a lane of their own.
pub fn graph(rows: &[LogRow]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let waiting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_deref() == Some(row.sha.as_str()))
            .map(|(i, _)| i)
            .collect();
        let lane = match waiting.first() {
            Some(&l) => l,
            None => match lanes.iter().position(Option::is_none) {
                Some(free) => free,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            },
        };
        let mut top = Vec::new();
        for (i, l) in lanes.iter().enumerate() {
            if l.is_some() {
                top.push((i, if waiting.contains(&i) { lane } else { i }));
            }
        }
        for &i in &waiting {
            lanes[i] = None;
        }
        let mut bottom = Vec::new();
        let mut parents = row.parents.iter();
        lanes[lane] = parents.next().cloned();
        for parent in parents {
            let to = match lanes.iter().position(|l| l.as_deref() == Some(parent)) {
                Some(existing) => existing,
                None => {
                    let free = match lanes.iter().position(Option::is_none) {
                        Some(free) => free,
                        None => {
                            lanes.push(None);
                            lanes.len() - 1
                        }
                    };
                    lanes[free] = Some(parent.clone());
                    free
                }
            };
            bottom.push((lane, to));
        }
        for (i, l) in lanes.iter().enumerate() {
            if l.is_some() && !bottom.iter().any(|(_, to)| *to == i) {
                bottom.push((i, i));
            }
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        out.push(GraphRow { lane, top, bottom });
    }
    out
}

/// A commit's message, and the files it changed against its first parent.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct CommitInfo {
    pub sha: String,
    pub body: String,
    pub files: Vec<Change>,
}

pub fn commit_info(dir: &str, sha: &str) -> CommitInfo {
    let parent = format!("{sha}^");
    let from = if rev(dir, &parent).is_some() {
        parent
    } else {
        // The first commit: compare with the empty tree.
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
    };
    CommitInfo {
        sha: sha.to_string(),
        body: git(dir, &["log", "-1", "--format=%b", sha])
            .unwrap_or_default()
            .trim()
            .to_string(),
        files: changes_between(dir, &from, sha),
    }
}

/// Files changed from `a` to `b`, with line counts.
fn changes_between(dir: &str, a: &str, b: &str) -> Vec<Change> {
    let stats =
        numstat(&git(dir, &["diff", "--numstat", "--no-renames", "-z", a, b]).unwrap_or_default());
    git(dir, &["diff", "--name-status", "--no-renames", "-z", a, b])
        .unwrap_or_default()
        .split('\0')
        .collect::<Vec<_>>()
        .chunks(2)
        .filter_map(|pair| {
            let (status, path) = (pair.first()?, pair.get(1)?);
            if status.is_empty() {
                return None;
            }
            let (added, removed, binary) = stats.get(*path).copied().unwrap_or_default();
            Some(Change {
                path: path.to_string(),
                status: status.chars().next()?.to_string(),
                added,
                removed,
                binary,
                ..Default::default()
            })
        })
        .collect()
}

/// A commit's message and changes, cut at `max` bytes, for an agent to read.
pub fn commit_patch(dir: &str, sha: &str, max: usize) -> String {
    let mut text = git(dir, &["show", "--format=%s%n%n%b", "--no-color", sha]).unwrap_or_default();
    if text.len() > max {
        let mut cut = max;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n… (cut short)");
    }
    text
}

/// Add a commit that takes `sha` back, on the checked-out branch. Clashing lines change
/// nothing.
pub fn revert(dir: &str, sha: &str) -> Result<(), String> {
    let editor = [("GIT_EDITOR", "true")];
    let mut args = vec!["revert", "--no-edit"];
    let parents = git(dir, &["rev-list", "--parents", "-n1", sha]).unwrap_or_default();
    if parents.split_whitespace().count() > 2 {
        // A merge: take back what it brought in, against the branch it landed on.
        args.extend(["-m", "1"]);
    }
    args.push(sha);
    git_with(dir, &args, &editor, None).map(|_| ()).map_err(|error| {
        let files = conflicted(dir);
        let _ = git(dir, &["revert", "--abort"]);
        if files.is_empty() {
            error
        } else {
            format!(
                "Later commits changed the same lines in {}, so it can't be taken back on its own. Nothing was changed.",
                files.join(", ")
            )
        }
    })
}

/// Copy one commit onto `branch`, wherever it is checked out. Clashing lines change
/// nothing.
pub fn copy_to(repo: &str, sha: &str, branch: &str) -> Result<(), String> {
    let editor = [("GIT_EDITOR", "true")];
    if is_ancestor(repo, sha, branch) {
        return Err(format!("{branch} already has this commit."));
    }
    on_branch(repo, branch, |dir| {
        git_with(dir, &["cherry-pick", sha], &editor, None)
            .map(|_| ())
            .map_err(|error| {
                let files = conflicted(dir);
                let _ = git(dir, &["cherry-pick", "--abort"]);
                if files.is_empty() && error.contains("empty") {
                    format!("{branch} already has these changes, so nothing was copied.")
                } else if files.is_empty() {
                    error
                } else {
                    format!(
                        "{branch} has different changes on the same lines in {}, so nothing was copied.",
                        files.join(", ")
                    )
                }
            })
    })
}

/// Name a commit as a release.
pub fn tag(dir: &str, name: &str, sha: &str) -> Result<(), String> {
    git(dir, &["tag", "-a", name, "-m", name, sha]).map(|_| ())
}

pub fn delete_tag(dir: &str, name: &str) -> Result<(), String> {
    git(dir, &["tag", "-d", name]).map(|_| ())
}

/// Move the checked-out branch back to `sha`, one of its own commits, keeping
/// uncommitted work. Git refuses rather than overwrite any of it.
pub fn go_back(dir: &str, sha: &str) -> Result<usize, String> {
    if !is_ancestor(dir, sha, "HEAD") {
        return Err("That commit isn't part of this branch, so it can't go back to it.".into());
    }
    let dropped = commits_ahead(dir, "HEAD", sha);
    git(dir, &["reset", "--keep", sha])?;
    Ok(dropped)
}

/// What kind of moment the time machine shows.
#[derive(Clone, Debug, PartialEq)]
pub enum MomentKind {
    /// A step git recorded: a commit, a switch, a sync, a go back.
    Step,
    /// A snapshot of every file Runo took before an agent's turn.
    Snapshot,
}

/// One moment in a folder's past.
#[derive(Clone, Debug, PartialEq)]
pub struct Moment {
    pub kind: MomentKind,
    /// Seconds since the epoch.
    pub at: i64,
    /// What happened, in plain words.
    pub label: String,
    /// The commit (or snapshot) the folder was at right after it.
    pub commit: String,
    /// The branch the folder was on then; empty when it wasn't on one.
    pub branch: String,
}

/// A reflog subject in plain words, or `None` for the in-between steps git records while
/// it works (a rebase's picks, say).
fn moment_words(subject: &str) -> Option<String> {
    let (what, rest) = subject.split_once(": ").unwrap_or((subject, ""));
    let rest = rest.trim();
    let quoted = |s: &str| format!("\u{201c}{}\u{201d}", s.lines().next().unwrap_or(""));
    Some(match what {
        "commit" => format!("Committed {}", quoted(rest)),
        "commit (initial)" => format!("First commit, {}", quoted(rest)),
        "commit (amend)" => format!("Changed the last commit, {}", quoted(rest)),
        "commit (merge)" => "Finished bringing changes in".into(),
        "checkout" => match rest
            .strip_prefix("moving from ")
            .and_then(|r| r.split_once(" to "))
        {
            Some((from, to)) if from == to => return None,
            Some((from, to)) => format!("Switched from {from} to {to}"),
            None => "Switched branch".into(),
        },
        "reset" => match rest.strip_prefix("moving to ") {
            Some(to) => format!("Moved back to {}", to.get(..7).unwrap_or(to)),
            None => "Moved the branch".into(),
        },
        "cherry-pick" => format!("Copied {} here", quoted(rest)),
        "revert" => format!("Took back {}", quoted(rest.trim_start_matches("Revert "))),
        w if w.starts_with("merge ") => format!("Brought in {}", &w[6..]),
        w if w.starts_with("pull") => "Synced with GitHub".into(),
        "rebase (finish)" => "Put your commits on top".into(),
        w if w.starts_with("rebase") => return None,
        "branch" => "Made the branch".into(),
        _ => {
            let mut c = subject.chars();
            c.next()?.to_uppercase().chain(c).collect()
        }
    })
}

/// The folder's past, newest first: every step git recorded on HEAD, and the snapshots
/// in `snapshots` (checkpoint commits) with their labels, up to `limit` steps.
pub fn moments(dir: &str, snapshots: &[(String, String)], limit: usize) -> Vec<Moment> {
    let mut out = Vec::new();
    let log = git(
        dir,
        &[
            "log",
            "-g",
            "--date=unix",
            &format!("-n{limit}"),
            "--format=%H%x1f%gd%x1f%gs",
            "HEAD",
        ],
    )
    .unwrap_or_default();
    // Walking back in time, a switch tells which branch the steps before it were on.
    let mut branch = current_branch(dir);
    for line in log.lines() {
        let mut f = line.split('\x1f');
        let (Some(commit), Some(selector), Some(subject)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let at = selector
            .rsplit('{')
            .next()
            .and_then(|t| t.trim_end_matches('}').parse().ok())
            .unwrap_or(0);
        let on = branch.clone();
        if let Some((from, _)) = subject
            .strip_prefix("checkout: moving from ")
            .and_then(|r| r.split_once(" to "))
        {
            // A sha (detached) is no branch.
            branch = if from.len() == 40 && from.chars().all(|c| c.is_ascii_hexdigit()) {
                String::new()
            } else {
                from.to_string()
            };
        }
        if let Some(label) = moment_words(subject) {
            out.push(Moment {
                kind: MomentKind::Step,
                at,
                label,
                commit: commit.to_string(),
                branch: on,
            });
        }
    }
    let shas: Vec<&str> = snapshots.iter().map(|(s, _)| s.as_str()).collect();
    if !shas.is_empty() {
        let mut args = vec!["show", "-s", "--format=%H %ct"];
        args.extend(&shas);
        let times = git(dir, &args).unwrap_or_default();
        for line in times.lines() {
            let Some((sha, at)) = line.split_once(' ') else {
                continue;
            };
            let Some((_, label)) = snapshots.iter().find(|(s, _)| s == sha) else {
                continue;
            };
            out.push(Moment {
                kind: MomentKind::Snapshot,
                at: at.trim().parse().unwrap_or(0),
                label: label.clone(),
                commit: sha.to_string(),
                branch: String::new(),
            });
        }
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.at));
    out
}

/// Files that differ between `commit` (a commit or a snapshot) and the folder now.
pub fn differs_from_now(dir: &str, commit: &str) -> Vec<String> {
    changed_since(dir, commit)
}

/// Take the folder back to a moment, safely. A step moves `branch` to its commit: the
/// checked-out branch keeps uncommitted work, another branch just moves, and a branch
/// that's gone is made again. A snapshot puts every file back, after taking a snapshot
/// of now. Returns the words for what happened and how to take it back.
pub fn travel(dir: &str, moment: &Moment) -> Result<(String, Undo), String> {
    let short = moment.commit.get(..7).unwrap_or(&moment.commit);
    match moment.kind {
        MomentKind::Snapshot => {
            let now = checkpoint(dir)?;
            let done = restore(dir, &moment.commit, None)?;
            let n = done.files.len();
            let note = match n {
                0 if !done.moved_head => "The files already match that moment.".to_string(),
                1 => "Put 1 file back as it was.".to_string(),
                n => format!("Put {n} files back as they were."),
            };
            Ok((note, Undo::Restore(now)))
        }
        MomentKind::Step if moment.branch.is_empty() => {
            Err("The folder wasn't on a branch then. Start a branch from that commit in History instead.".into())
        }
        MomentKind::Step if moment.branch == current_branch(dir) => {
            let head = rev(dir, "HEAD").ok_or("This folder has no commits yet.")?;
            if head == moment.commit {
                return Err("The branch is already there.".into());
            }
            keep_for_undo(dir, &head);
            git(dir, &["reset", "--keep", &moment.commit]).map_err(|e| {
                if e.contains("not uptodate") || e.contains("would be overwritten") {
                    "Some files you're changing would be overwritten. Commit or set them aside first.".to_string()
                } else {
                    e
                }
            })?;
            Ok((
                format!("Took {} back to {short}.", moment.branch),
                Undo::Folder {
                    branch: moment.branch.clone(),
                    head,
                },
            ))
        }
        MomentKind::Step => {
            let name = moment.branch.as_str();
            let was = rev(dir, &format!("refs/heads/{name}"));
            if was.as_deref() == Some(moment.commit.as_str()) {
                return Err(format!("{name} is already there."));
            }
            match checked_out_at(dir, name) {
                Some(at) => {
                    git(&at, &["reset", "--keep", &moment.commit])?;
                }
                None => {
                    git(dir, &["branch", "-f", name, &moment.commit])?;
                }
            }
            if let Some(was) = &was {
                keep_for_undo(dir, was);
            }
            let note = match was {
                Some(_) => format!("Took {name} back to {short}."),
                None => format!("Brought {name} back, at {short}."),
            };
            Ok((note, Undo::Branches(vec![(name.to_string(), was)])))
        }
    }
}

/// Work set aside, newest first.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Stash {
    pub message: String,
    pub when: String,
    pub commit: String,
}

pub fn stashes(dir: &str) -> Vec<Stash> {
    git(dir, &["stash", "list", "--format=%H%x00%s%x00%cr"])
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\0');
            let commit = f.next()?.to_string();
            let message = f.next()?;
            // "On main: note" or "WIP on main: abc123 subject": keep the part after the colon.
            let message = message
                .split_once(": ")
                .map_or(message, |(_, m)| m)
                .to_string();
            Some(Stash {
                commit,
                message,
                when: f.next()?.to_string(),
            })
        })
        .collect()
}

/// Set uncommitted work aside, new files included, leaving the folder clean.
pub fn stash_push(dir: &str, message: &str) -> Result<(), String> {
    git(
        dir,
        &["stash", "push", "--include-untracked", "-m", message],
    )
    .map(|_| ())
}

/// Bring set-aside work back and take it off the list. On clashing lines it stays on the
/// list and nothing changes.
pub fn stash_apply(dir: &str, index: usize) -> Result<(), String> {
    let name = format!("stash@{{{index}}}");
    git(dir, &["stash", "pop", "--index", &name])
        .or_else(|_| {
            // --index fails when staged changes clash; try without keeping staging.
            let _ = git(dir, &["reset", "--merge"]);
            git(dir, &["stash", "pop", &name])
        })
        .map(|_| ())
        .map_err(|e| {
            let _ = git(dir, &["reset", "--merge"]);
            format!("It clashes with changes in the folder now, so it stays set aside. ({e})")
        })
}

pub fn stash_drop(dir: &str, index: usize) -> Result<(), String> {
    git(dir, &["stash", "drop", &format!("stash@{{{index}}}")]).map(|_| ())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Hunk,
    Added,
    Removed,
    Context,
    /// Lines such as `\ No newline at end of file`.
    Note,
}

/// One displayable row of a unified diff, with the line number to show beside it
/// (the new file's number, or the old file's for removed lines).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiffRow {
    pub kind: LineKind,
    pub number: Option<usize>,
    pub text: String,
}

/// Start lines of a hunk header such as `@@ -12,7 +12,9 @@ fn x()`. A count may be
/// omitted (`@@ -1 +1,3 @@`), so the numbers are read per side, not by position.
fn hunk_starts(header: &str) -> Option<(usize, usize)> {
    let mut parts = header.trim_start_matches('@').split_whitespace();
    let start = |side: &str, sign: char| -> Option<usize> {
        side.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    let old = start(parts.next()?, '-')?;
    let new = start(parts.next()?, '+')?;
    Some((old, new))
}

/// Turn `git diff` output into rows, dropping file headers.
pub fn diff_rows(diff: &str) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    let (mut old, mut new) = (0usize, 0usize);
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("@@") {
            if let Some((o, n)) = hunk_starts(line) {
                (old, new) = (o, n);
            }
            in_hunk = true;
            rows.push(DiffRow {
                kind: LineKind::Hunk,
                number: None,
                text: line.to_string(),
            });
            continue;
        }
        if !in_hunk {
            // `diff --git`, `index`, `---`, `+++`, `new file mode`… before the first hunk.
            if line.starts_with("Binary files ") {
                rows.push(DiffRow {
                    kind: LineKind::Note,
                    number: None,
                    text: "Binary file changed".into(),
                });
            }
            continue;
        }
        if line.starts_with("diff --git") {
            in_hunk = false;
            continue;
        }
        let text = line.get(1..).unwrap_or("").to_string();
        let row = match line.chars().next() {
            Some('+') => {
                new += 1;
                DiffRow {
                    kind: LineKind::Added,
                    number: Some(new - 1),
                    text,
                }
            }
            Some('-') => {
                old += 1;
                DiffRow {
                    kind: LineKind::Removed,
                    number: Some(old - 1),
                    text,
                }
            }
            Some('\\') => DiffRow {
                kind: LineKind::Note,
                number: None,
                text: line.to_string(),
            },
            _ => {
                old += 1;
                new += 1;
                DiffRow {
                    kind: LineKind::Context,
                    number: Some(new - 1),
                    text,
                }
            }
        };
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn repo(name: &str) -> std::path::PathBuf {
        let repo = std::env::temp_dir().join(format!("runo-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        // Git reports real paths, and on a Mac the temporary folder is a symlink.
        repo.canonicalize().unwrap()
    }

    #[test]
    fn checkpoints_restore_the_working_tree() {
        let repo = repo("checkpoint");
        let dir = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        // Uncommitted work from before the turn must come back too.
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        std::fs::write(repo.join("draft.txt"), "draft\n").unwrap();
        let cp = checkpoint(dir).unwrap();
        // The "agent" edits, creates, deletes; ignored files are not touched.
        std::fs::write(repo.join("a.txt"), "three\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new\n").unwrap();
        std::fs::remove_file(repo.join("draft.txt")).unwrap();
        std::fs::write(repo.join("ignored.txt"), "keep\n").unwrap();
        let mut changed = changed_since(dir, &cp);
        changed.sort();
        assert_eq!(changed, ["a.txt", "draft.txt", "new.txt"]);
        assert_eq!(restore(dir, &cp, None).unwrap().files.len(), 3);
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "two\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("draft.txt")).unwrap(),
            "draft\n"
        );
        assert!(!repo.join("new.txt").exists());
        assert!(repo.join("ignored.txt").exists());
        assert!(changed_since(dir, &cp).is_empty());
        // The user's own index was not used for snapshots.
        assert!(git(dir, &["diff", "--cached", "--name-only"])
            .unwrap()
            .trim()
            .is_empty());
        assert!(patch_for_message(dir, 10_000).contains("draft.txt"));
        assert!(file_diff_context(dir, "a.txt", true, "HEAD").contains("+two"));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn forks_carry_uncommitted_work_at_the_checkpoint() {
        let repo = repo("fork");
        let dir = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "x\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        std::fs::write(repo.join("draft.txt"), "draft\n").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        let cp = checkpoint(dir).unwrap();
        // Later work in the original folder stays out of the fork.
        std::fs::write(repo.join("a.txt"), "three\n").unwrap();
        sh(&repo, &["commit", "-q", "-am", "later"]);
        let fork = std::path::PathBuf::from(fork_worktree(dir, "runo/fork-test", &cp).unwrap());
        assert_eq!(
            std::fs::read_to_string(fork.join("a.txt")).unwrap(),
            "two\n"
        );
        assert_eq!(
            std::fs::read_to_string(fork.join("draft.txt")).unwrap(),
            "draft\n"
        );
        assert!(!fork.join("gone.txt").exists());
        let fork_dir = fork.to_str().unwrap();
        assert_eq!(current_branch(fork_dir), "runo/fork-test");
        assert_eq!(
            git(fork_dir, &["log", "-1", "--format=%s"]).unwrap().trim(),
            "init"
        );
        let _ = remove_worktree(dir, fork_dir);
        let _ = std::fs::remove_dir_all(worktree_path(dir, "runo/fork-test"));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn diff_rows_number_each_side() {
        let diff = "diff --git a/m.rs b/m.rs
index 1..2 100644
--- a/m.rs
+++ b/m.rs
@@ -1 +1,3 @@
-fn main() {}
+fn main() {
+    hi();
+}
\\ No newline at end of file
@@ -10,3 +12,3 @@ impl X {
 keep
-old
+new
 
";
        let rows = diff_rows(diff);
        let shown: Vec<(LineKind, Option<usize>, &str)> = rows
            .iter()
            .map(|r| (r.kind, r.number, r.text.as_str()))
            .collect();
        use LineKind::*;
        assert_eq!(
            shown,
            [
                (Hunk, None, "@@ -1 +1,3 @@"),
                (Removed, Some(1), "fn main() {}"),
                (Added, Some(1), "fn main() {"),
                (Added, Some(2), "    hi();"),
                (Added, Some(3), "}"),
                (Note, None, "\\ No newline at end of file"),
                (Hunk, None, "@@ -10,3 +12,3 @@ impl X {"),
                (Context, Some(12), "keep"),
                (Removed, Some(11), "old"),
                (Added, Some(13), "new"),
                (Context, Some(14), ""),
            ]
        );
        assert_eq!(hunk_starts("@@ -0,0 +1 @@"), Some((0, 1)));
        assert_eq!(hunk_starts("@@ garbage @@"), None);
        assert!(diff_rows("").is_empty());
    }

    #[test]
    fn worktree_paths_sit_beside_the_repository() {
        let p = worktree_path("/code/app", "feature/nav");
        assert_eq!(p, Path::new("/code/.runo-worktrees/app/feature/nav"));
        // `a/b` and `a-b` are different branches, so they get different folders.
        assert_ne!(
            worktree_path("/code/app", "a/b"),
            worktree_path("/code/app", "a-b")
        );
        assert_eq!(
            worktree_path("/code/app", "../x"),
            Path::new("/code/.runo-worktrees/app/x")
        );
    }

    #[test]
    fn branch_names_are_sanitized() {
        assert_eq!(
            sanitize_branch("  Fix the login bug! "),
            "Fix-the-login-bug"
        );
        assert_eq!(sanitize_branch("feature/nav"), "feature/nav");
        assert_eq!(sanitize_branch("/.weird-/"), "weird");
        assert_eq!(sanitize_branch("ünïcode"), "n-code");
        assert_eq!(sanitize_branch(""), "");
    }

    #[test]
    fn non_repositories_are_detected() {
        let dir = std::env::temp_dir().join(format!("runo-git-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The temp dir itself could sit inside a repository on some machines; only assert
        // the parts that hold either way.
        let d = dir.to_str().unwrap();
        if !is_repo(d) {
            assert!(changes(d).is_empty());
            assert!(tracked_files(d).is_empty());
            assert_eq!(current_branch(d), "");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deletions_renames_and_spaces() {
        let repo = repo("kinds");
        std::fs::write(repo.join("gone.txt"), "a\nb\nc\n").unwrap();
        std::fs::write(repo.join("old name.txt"), "1\n2\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        sh(&repo, &["mv", "old name.txt", "new name.txt"]);
        let dir = repo.to_str().unwrap();
        let found = changes(dir);
        let gone = found.iter().find(|c| c.path == "gone.txt").unwrap();
        assert_eq!(
            (gone.status.as_str(), gone.added, gone.removed),
            ("D", 0, 3)
        );
        let renamed = found.iter().find(|c| c.path == "new name.txt").unwrap();
        assert_eq!(renamed.status, "R");
        assert_eq!(renamed.added, 2, "renames count as the new file's lines");
        assert!(file_diff(dir, "gone.txt").contains("-b"));
        let files = tracked_files(dir);
        assert!(files.contains(&"new name.txt".to_string()));
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn fresh_repository_without_commits() {
        let repo = repo("fresh");
        std::fs::write(repo.join("first.txt"), "hello\n").unwrap();
        let dir = repo.to_str().unwrap();
        assert!(is_repo(dir));
        let found = changes(dir);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].status.as_str(), found[0].added), ("A", 1));
        assert!(file_diff(dir, "first.txt").contains("+hello"));
        // No worktree can branch from nothing; an empty first commit fixes that.
        assert!(!has_commits(dir));
        let error = add_worktree(dir, "runo/fresh", Some("HEAD")).unwrap_err();
        assert!(error.contains("no commits"));
        initial_commit(dir).unwrap();
        assert!(has_commits(dir));
        let tree = add_worktree(dir, "runo/fresh", Some("HEAD")).unwrap();
        assert_eq!(current_branch(&tree), "runo/fresh");
        // The work in progress stays in the project folder, uncommitted.
        assert_eq!(changes(dir).len(), 1);
        remove_worktree(dir, &tree).unwrap();
        let _ = std::fs::remove_dir_all(
            repo.parent()
                .unwrap()
                .join(".runo-worktrees")
                .join(repo.file_name().unwrap()),
        );
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn worktree_on_an_existing_branch() {
        let repo = repo("existing");
        std::fs::write(repo.join("a"), "a\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);
        sh(&repo, &["branch", "topic"]);
        let r = repo.to_str().unwrap();
        let dir = add_worktree(r, "topic", None).unwrap();
        assert_eq!(current_branch(&dir), "topic");
        assert!(dir.contains(".runo-worktrees"));
        remove_worktree(r, &dir).unwrap();
        let _ = std::fs::remove_dir_all(
            repo.parent()
                .unwrap()
                .join(".runo-worktrees")
                .join(repo.file_name().unwrap()),
        );
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn worktree_changes_and_diff() {
        let root = std::env::temp_dir().join(format!("runo-test-{}", std::process::id()));
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let repo = repo.canonicalize().unwrap();
        sh(&repo, &["init", "-q"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);

        let repo_str = repo.to_str().unwrap();
        assert!(is_repo(repo_str));
        let dir = add_worktree(repo_str, "runo/test", None).unwrap();
        assert!(add_worktree(repo_str, "runo/test", None).is_err());
        assert_eq!(current_branch(&dir), "runo/test");

        std::fs::write(Path::new(&dir).join("a.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(Path::new(&dir).join("new.txt"), "x\ny\n").unwrap();
        let changes = changes(&dir);
        let a = changes.iter().find(|c| c.path == "a.txt").unwrap();
        assert_eq!((a.added, a.removed), (2, 1));
        let n = changes.iter().find(|c| c.path == "new.txt").unwrap();
        assert_eq!((n.status.as_str(), n.added), ("A", 2));
        assert!(file_diff(&dir, "a.txt").contains("+TWO"));
        assert!(file_diff(&dir, "new.txt").contains("+x"));

        remove_worktree(repo_str, &dir).unwrap();
        assert!(!Path::new(&dir).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn discard_commit_and_sync() {
        let repo = repo("ship");
        let dir = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        assert_eq!(sync_state(dir), Sync::default());

        // A modified tracked file goes back to its committed text, even when staged.
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        sh(&repo, &["add", "a.txt"]);
        discard(dir, "a.txt").unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "one\n"
        );
        // New files are removed, staged or not.
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        std::fs::write(repo.join("staged.txt"), "x").unwrap();
        sh(&repo, &["add", "staged.txt"]);
        discard(dir, "new.txt").unwrap();
        discard(dir, "staged.txt").unwrap();
        assert!(changes(dir).is_empty());

        assert!(commit(dir, "  ").is_err());
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        let hash = commit(dir, "Add b").unwrap();
        assert!(!hash.is_empty());
        assert!(changes(dir).is_empty());

        // With a remote but no upstream, unpushed commits are counted.
        let remote = repo.with_extension("remote");
        let _ = std::fs::remove_dir_all(&remote);
        sh(
            Path::new("."),
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        let s = sync_state(dir);
        assert_eq!(
            (s.remote.as_str(), s.upstream, s.ahead),
            ("origin", false, 2)
        );
        sh(&repo, &["push", "-q", "-u", "origin", "HEAD"]);
        std::fs::write(repo.join("c.txt"), "c").unwrap();
        commit(dir, "Add c").unwrap();
        let s = sync_state(dir);
        assert_eq!((s.upstream, s.ahead), (true, 1));
        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&remote);
    }

    #[test]
    fn worktreeinclude_copies_matching_ignored_files() {
        let repo = repo("include");
        let dir = repo.to_str().unwrap();
        std::fs::write(repo.join(".gitignore"), ".env\n*.log\nsecrets/\n").unwrap();
        std::fs::write(repo.join(".worktreeinclude"), ".env\nsecrets/\n").unwrap();
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join(".env"), "KEY=1").unwrap();
        std::fs::write(repo.join("debug.log"), "noise").unwrap();
        std::fs::create_dir_all(repo.join("secrets")).unwrap();
        std::fs::write(repo.join("secrets/key.pem"), "pem").unwrap();
        // Untracked but not ignored: the user's own work, never copied.
        std::fs::write(repo.join("draft.txt"), "wip").unwrap();

        let wt = add_worktree(dir, "include-test", None).unwrap();
        let mut copied = copy_included(dir, &wt).unwrap();
        copied.sort();
        assert_eq!(copied, [".env", "secrets/key.pem"]);
        assert_eq!(
            std::fs::read_to_string(Path::new(&wt).join(".env")).unwrap(),
            "KEY=1"
        );
        assert!(!Path::new(&wt).join("debug.log").exists());
        assert!(!Path::new(&wt).join("draft.txt").exists());
        // Running again leaves existing files alone.
        assert!(copy_included(dir, &wt).unwrap().is_empty());
        remove_worktree(dir, &wt).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn staging_unicode_and_binary_files() {
        let repo = repo("stage");
        let dir = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);
        std::fs::write(repo.join("a.txt"), "a2\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b2\n").unwrap();
        std::fs::write(repo.join("ünï cødé.txt"), "x\ny\n").unwrap();
        std::fs::write(repo.join("pic.bin"), b"\x89PNG\0\0\x01").unwrap();
        let found = changes(dir);
        let named = found.iter().find(|c| c.path == "ünï cødé.txt").unwrap();
        assert_eq!((named.status.as_str(), named.added), ("A", 2));
        assert!(found.iter().find(|c| c.path == "pic.bin").unwrap().binary);
        assert!(file_diff(dir, "ünï cødé.txt").contains("+y"));
        discard(dir, "ünï cødé.txt").unwrap();
        assert!(!repo.join("ünï cødé.txt").exists());
        let rows = diff_rows("diff --git a/p b/p\nindex 1..2\nBinary files a/p and b/p differ\n");
        assert_eq!(rows[0].text, "Binary file changed");

        stage(dir, "a.txt").unwrap();
        let a = changes(dir)
            .into_iter()
            .find(|c| c.path == "a.txt")
            .unwrap();
        assert!(a.staged && !a.unstaged);
        unstage(dir, "a.txt").unwrap();
        assert!(!changes(dir).iter().any(|c| c.staged));
        // With something staged, only that is committed.
        stage(dir, "b.txt").unwrap();
        commit(dir, "Only b").unwrap();
        let left: Vec<String> = changes(dir).into_iter().map(|c| c.path).collect();
        assert!(left.contains(&"a.txt".to_string()));
        assert!(!left.contains(&"b.txt".to_string()));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[cfg(unix)]
    #[test]
    fn undo_restores_commits_modes_links_and_staging() {
        use std::os::unix::fs::PermissionsExt;
        let repo = repo("undo");
        let dir = repo.to_str().unwrap();
        std::fs::write(repo.join("run.sh"), "echo hi\n").unwrap();
        std::fs::set_permissions(repo.join("run.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::os::unix::fs::symlink("run.sh", repo.join("link")).unwrap();
        std::fs::write(repo.join("keep.txt"), "1\n").unwrap();
        std::fs::write(repo.join("staged.txt"), "1\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);
        let start = rev(dir, "HEAD").unwrap();
        // The person staged some work before the turn.
        std::fs::write(repo.join("staged.txt"), "2\n").unwrap();
        sh(&repo, &["add", "staged.txt"]);
        let cp = checkpoint(dir).unwrap();

        // The agent rewrites the script without its exec bit, replaces the link, and
        // commits; the person keeps writing elsewhere.
        std::fs::remove_file(repo.join("run.sh")).unwrap();
        std::fs::write(repo.join("run.sh"), "echo bye\n").unwrap();
        std::fs::remove_file(repo.join("link")).unwrap();
        std::fs::write(repo.join("link"), "not a link\n").unwrap();
        std::fs::create_dir_all(repo.join("new/deep")).unwrap();
        std::fs::write(repo.join("new/deep/file.txt"), "n\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "agent work"]);
        let end = checkpoint(dir).unwrap();
        std::fs::write(repo.join("keep.txt"), "mine\n").unwrap();

        let redo = checkpoint(dir).unwrap();
        let done = restore(dir, &cp, Some(&end)).unwrap();
        assert!(done.moved_head);
        assert_eq!(rev(dir, "HEAD").unwrap(), start);
        assert!(done.skipped.is_empty());
        let mode = std::fs::metadata(repo.join("run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "exec bit is back");
        assert_eq!(
            std::fs::read_to_string(repo.join("run.sh")).unwrap(),
            "echo hi\n"
        );
        assert!(repo.join("link").is_symlink());
        assert!(!repo.join("new").exists(), "emptied folders go too");
        // The person's edit after the turn and their staging survive.
        assert_eq!(
            std::fs::read_to_string(repo.join("keep.txt")).unwrap(),
            "mine\n"
        );
        let cached = git(dir, &["diff", "--cached", "--name-only"]).unwrap();
        assert_eq!(cached.trim(), "staged.txt");

        // Redo puts the agent's commit and files back.
        let after = checkpoint(dir).unwrap();
        let again = restore(dir, &redo, Some(&after)).unwrap();
        assert!(again.moved_head);
        assert_eq!(
            git(dir, &["log", "-1", "--format=%s"]).unwrap().trim(),
            "agent work"
        );
        assert!(repo.join("new/deep/file.txt").exists());

        // A file changed after the guard is skipped, not overwritten.
        std::fs::write(repo.join("run.sh"), "edited later\n").unwrap();
        let skipped = restore(dir, &cp, Some(&end)).unwrap();
        assert_eq!(skipped.skipped, ["run.sh"]);
        assert_eq!(
            std::fs::read_to_string(repo.join("run.sh")).unwrap(),
            "edited later\n"
        );
        drop_checkpoints(dir, &[cp.clone(), end, redo, after]);
        assert!(git(dir, &["for-each-ref", "refs/runo"])
            .unwrap()
            .trim()
            .is_empty());
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// A repository with a bare `origin` that has `main` and one commit.
    fn with_origin(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let repo = repo(name);
        sh(&repo, &["checkout", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "1\n2\n3\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "init"]);
        let remote = repo.with_extension("origin");
        let _ = std::fs::remove_dir_all(&remote);
        sh(
            Path::new("."),
            &[
                "init",
                "-q",
                "--bare",
                "--initial-branch=main",
                remote.to_str().unwrap(),
            ],
        );
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-q", "-u", "origin", "main"]);
        sh(&repo, &["remote", "set-head", "origin", "main"]);
        (repo, remote)
    }

    #[test]
    fn worktrees_start_from_the_fetched_default_branch() {
        let (repo, remote) = with_origin("fresh-base");
        let dir = repo.to_str().unwrap();
        assert_eq!(default_branch(dir), "main");
        // Someone else pushes to main; the local main is now behind.
        let other = repo.with_extension("other");
        let _ = std::fs::remove_dir_all(&other);
        sh(
            Path::new("."),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        sh(&other, &["config", "user.email", "t@t"]);
        sh(&other, &["config", "user.name", "t"]);
        std::fs::write(other.join("b.txt"), "b\n").unwrap();
        sh(&other, &["add", "."]);
        sh(&other, &["commit", "-qm", "upstream"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        fetch(dir).unwrap();
        let start = base_ref(dir, "main");
        assert_eq!(start, "origin/main");
        let wt = add_worktree(dir, "runo/fresh", Some(&start)).unwrap();
        assert!(Path::new(&wt).join("b.txt").exists());
        // Commits made in the session count against main, not just uncommitted work.
        std::fs::write(Path::new(&wt).join("c.txt"), "c\n").unwrap();
        sh(Path::new(&wt), &["add", "."]);
        sh(Path::new(&wt), &["commit", "-qm", "session"]);
        assert!(changes(&wt).is_empty());
        let since = changes_since(&wt, &start);
        assert_eq!(totals(&since), (1, 1, 0));
        assert!(
            file_diff_context(&wt, "c.txt", false, &merge_base(&wt, &start).unwrap())
                .contains("+c")
        );
        remove_worktree(dir, &wt).unwrap();
        // Local work not pushed yet wins over the remote copy.
        std::fs::write(repo.join("d.txt"), "d\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "local"]);
        assert_eq!(base_ref(dir, "main"), "main");
        for p in [&repo, &remote, &other] {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn sync_switch_and_undo_a_project_folder() {
        let (repo, remote) = with_origin("sync");
        let dir = repo.to_str().unwrap();
        let state = repo_state(dir);
        assert_eq!(
            (state.branch.as_str(), state.upstream.as_str()),
            ("main", "origin/main")
        );
        assert_eq!((state.ahead, state.behind), (0, 0));
        // Someone else pushes; this folder has a commit of its own and uncommitted work.
        let other = repo.with_extension("sync-other");
        let _ = std::fs::remove_dir_all(&other);
        sh(
            Path::new("."),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        sh(&other, &["config", "user.email", "t@t"]);
        sh(&other, &["config", "user.name", "t"]);
        std::fs::write(other.join("b.txt"), "b\n").unwrap();
        sh(&other, &["add", "."]);
        sh(&other, &["commit", "-qm", "theirs"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "mine"]);
        std::fs::write(repo.join("a.txt"), "1\n2\n3\n4\n").unwrap();
        fetch(dir).unwrap();
        let state = repo_state(dir);
        assert_eq!((state.ahead, state.behind), (1, 1));
        let point = |dir: &str| UndoPoint {
            dir: dir.to_string(),
            label: String::new(),
            undo: folder_point(dir).unwrap(),
        };
        let before = point(dir);
        assert_eq!(pull(dir).unwrap(), Update::Updated);
        assert!(repo.join("b.txt").exists());
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "1\n2\n3\n4\n"
        );
        let state = repo_state(dir);
        assert_eq!((state.ahead, state.behind), (1, 0));
        // Undo takes the folder back and keeps the uncommitted edit.
        undo(&before).unwrap();
        assert!(!repo.join("b.txt").exists());
        assert!(repo.join("c.txt").exists());
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "1\n2\n3\n4\n"
        );
        // Clashing lines leave everything as it was.
        sh(&repo, &["checkout", "-q", "a.txt"]);
        std::fs::write(other.join("a.txt"), "theirs\n").unwrap();
        sh(&other, &["commit", "-qam", "theirs again"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        std::fs::write(repo.join("a.txt"), "mine\n").unwrap();
        sh(&repo, &["commit", "-qam", "mine again"]);
        let head = rev(dir, "HEAD");
        assert_eq!(
            pull(dir).unwrap(),
            Update::Conflicts(vec!["a.txt".to_string()])
        );
        // The clash waits to be sorted out; calling it off puts everything back.
        let found = clash(dir).unwrap();
        assert_eq!((found.kind, found.yours.as_str()), ("rebase", "main"));
        assert_eq!(found.theirs, "origin/main");
        assert!(!found.yours_is_ours);
        assert_eq!(
            found.files[0].stretches,
            Some(vec![Stretch::Spot {
                ours: "theirs\n".into(),
                theirs: "mine\n".into()
            }])
        );
        abort_update(dir).unwrap();
        assert_eq!(in_progress(dir), None);
        assert_eq!(rev(dir, "HEAD"), head);
        // Sorted out: keep both, and finish.
        pull(dir).unwrap();
        settle(dir, "a.txt", "mine\ntheirs\n").unwrap();
        assert!(clash(dir).unwrap().files.is_empty());
        assert_eq!(continue_update(dir).unwrap(), Update::Updated);
        assert_eq!(clash(dir), None);
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "mine\ntheirs\n"
        );
        assert!(is_ancestor(dir, "origin/main", "HEAD"));
        sh(&repo, &["reset", "-q", "--hard", head.as_deref().unwrap()]);
        // Switching branches, and undoing the switch.
        sh(&repo, &["branch", "side"]);
        assert_eq!(repo_state(dir).branches.len(), 2);
        let before = point(dir);
        switch(dir, "side").unwrap();
        assert_eq!(current_branch(dir), "side");
        undo(&before).unwrap();
        assert_eq!(current_branch(dir), "main");
        for p in [&repo, &remote, &other] {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn a_branch_splits_into_a_stack() {
        let (repo, _remote) = with_origin("split");
        let dir = repo.to_str().unwrap();
        new_branch(dir, "big", "main").unwrap();
        switch(dir, "big").unwrap();
        std::fs::write(repo.join("one.txt"), "1\n").unwrap();
        std::fs::write(repo.join("two.txt"), "2\n").unwrap();
        std::fs::write(repo.join("three.txt"), "3\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "everything"]);
        switch(dir, "main").unwrap();
        assert!(branch_patch(dir, "big", "main", 10_000).contains("- everything"));
        let names = stack_parts(
            dir,
            "main",
            "big",
            "big",
            &[
                ("First".into(), vec!["one.txt".into()]),
                ("Rest".into(), vec!["two.txt".into(), "three.txt".into()]),
            ],
        )
        .unwrap();
        assert_eq!(names, vec!["big-part-1", "big-part-2"]);
        let files = |b: &str| git(dir, &["ls-tree", "--name-only", b]).unwrap();
        assert!(
            files("big-part-1").contains("one.txt") && !files("big-part-1").contains("two.txt")
        );
        // The top of the stack is the whole branch, built on the part below it.
        assert!(git(dir, &["diff", "--quiet", "big", "big-part-2"]).is_ok());
        assert!(is_ancestor(dir, "big-part-1", "big-part-2"));
        assert_eq!(
            git(dir, &["log", "-1", "--format=%s", "big-part-1"])
                .unwrap()
                .trim(),
            "First"
        );
        assert_eq!(current_branch(dir), "main");
        assert!(changes(dir).is_empty());
    }

    #[test]
    fn radar_finds_clashes_without_touching_folders() {
        let (repo, _remote) = with_origin("radar");
        let dir = repo.to_str().unwrap();
        let lines: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
        std::fs::write(repo.join("f.txt"), lines.join("\n") + "\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "twenty"]);
        for (branch, text, file) in [
            ("one", "one's", "f.txt"),
            ("two", "two's", "f.txt"),
            ("three", "x", "g.txt"),
        ] {
            new_branch(dir, branch, "main").unwrap();
            switch(dir, branch).unwrap();
            let mut edited = lines.clone();
            edited[9] = text.into();
            std::fs::write(repo.join(file), edited.join("\n") + "\n").unwrap();
            sh(&repo, &["add", "."]);
            sh(&repo, &["commit", "-qm", branch]);
            switch(dir, "main").unwrap();
        }
        let hit = test_merge(dir, "one", "two").unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].path, "f.txt");
        assert_eq!(hit[0].lines, vec![10]);
        assert!(test_merge(dir, "one", "three").unwrap().is_empty());
        assert!(changes(dir).is_empty());
        // Uncommitted work counts too, and stays where it is.
        let mut edited = lines.clone();
        edited[9] = "main's own".into();
        std::fs::write(repo.join("f.txt"), edited.join("\n") + "\n").unwrap();
        let tip = work_tip(dir).unwrap();
        assert_ne!(Some(tip.clone()), rev(dir, "HEAD"));
        assert_eq!(test_merge(dir, &tip, "one").unwrap()[0].path, "f.txt");
        assert_eq!(changes(dir).len(), 1);
        assert!(stashes(dir).is_empty());
    }

    #[test]
    fn clash_markers_split_into_stretches() {
        let text =
            "a\n<<<<<<< HEAD\nmine\n||||||| base\nold\n=======\ntheirs\nmore\n>>>>>>> x\nb\n";
        assert_eq!(
            parse_clash(text),
            Some(vec![
                Stretch::Same("a\n".into()),
                Stretch::Spot {
                    ours: "mine\n".into(),
                    theirs: "theirs\nmore\n".into()
                },
                Stretch::Same("b\n".into()),
            ])
        );
        assert_eq!(parse_clash("no markers\n"), None);
        assert_eq!(parse_clash("<<<<<<< HEAD\nunfinished\n"), None);
    }

    #[test]
    fn commit_and_throw_away_parts_of_files() {
        let (repo, _remote) = with_origin("parts");
        let dir = repo.to_str().unwrap();
        let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        std::fs::write(repo.join("f.txt"), lines.join("\n") + "\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "thirty lines"]);
        // Two parts far apart: change line 2, add after line 25.
        let mut edited = lines.clone();
        edited[1] = "line two".into();
        edited.insert(25, "added one".into());
        edited.insert(26, "added two".into());
        std::fs::write(repo.join("f.txt"), edited.join("\n") + "\n").unwrap();
        std::fs::write(repo.join("new.txt"), "a\nb\n").unwrap();
        let files = file_parts(dir);
        let f = files.iter().find(|f| f.change.path == "f.txt").unwrap();
        assert_eq!(f.parts.len(), 2);
        let second = &f.parts[1];
        let added: Vec<usize> = second
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind == '+')
            .map(|(i, _)| i)
            .collect();
        assert_eq!(added.len(), 2);
        assert_eq!(second.lines[added[0]].new, 26);
        // Commit only the first added line of the second part.
        let patch = picked_patch(f, |p, l| p == 1 && l == added[0]).unwrap();
        let head = rev(dir, "HEAD").unwrap();
        commit_parts(dir, &[], &patch, "Add one line", false).unwrap();
        let committed = git(dir, &["show", "HEAD:f.txt"]).unwrap();
        assert!(committed.contains("added one") && !committed.contains("added two"));
        assert!(!committed.contains("line two"));
        // The rest is still in the folder, and nothing was lost.
        let left = file_parts(dir);
        let f = left.iter().find(|f| f.change.path == "f.txt").unwrap();
        assert_eq!(f.parts.len(), 2);
        assert!(std::fs::read_to_string(repo.join("f.txt"))
            .unwrap()
            .contains("added two"));
        assert!(left.iter().any(|f| f.change.path == "new.txt"));
        // Undo puts the commit's changes back as uncommitted.
        undo(&UndoPoint {
            dir: dir.to_string(),
            label: String::new(),
            undo: Undo::Uncommit { head: head.clone() },
        })
        .unwrap();
        assert_eq!(rev(dir, "HEAD").unwrap(), head);
        assert!(std::fs::read_to_string(repo.join("f.txt"))
            .unwrap()
            .contains("added one"));
        // Whole files and a new file go in whole; amending keeps the message.
        commit_parts(dir, &["new.txt".into()], "", "Add new", false).unwrap();
        let files = file_parts(dir);
        let f = files.iter().find(|f| f.change.path == "f.txt").unwrap();
        let patch = picked_patch(f, |p, _| p == 0).unwrap();
        commit_parts(dir, &[], &patch, "", true).unwrap();
        assert_eq!(
            git(dir, &["log", "-1", "--format=%s"]).unwrap().trim(),
            "Add new"
        );
        assert!(git(dir, &["show", "HEAD:f.txt"])
            .unwrap()
            .contains("line two"));
        assert_eq!(last_commit(dir), Some(("Add new".into(), true)));
        // Throw away the second part, then put it back.
        let files = file_parts(dir);
        let f = files.iter().find(|f| f.change.path == "f.txt").unwrap();
        assert_eq!(f.parts.len(), 1);
        let part = picked_patch(f, |p, _| p == 0).unwrap();
        throw_away(dir, &part).unwrap();
        assert!(changes(dir).is_empty());
        undo(&UndoPoint {
            dir: dir.to_string(),
            label: String::new(),
            undo: Undo::Reapply(part),
        })
        .unwrap();
        assert!(std::fs::read_to_string(repo.join("f.txt"))
            .unwrap()
            .contains("added two"));
    }

    #[test]
    fn time_machine_goes_back_to_any_moment() {
        let (repo, _remote) = with_origin("moments");
        let dir = repo.to_str().unwrap();
        let commit = |file: &str, text: &str| {
            std::fs::write(repo.join(file), text).unwrap();
            sh(&repo, &["add", "."]);
            sh(&repo, &["commit", "-qm", file]);
        };
        commit("b.txt", "b\n");
        let at_b = rev(dir, "HEAD").unwrap();
        new_branch(dir, "side", "main").unwrap();
        switch(dir, "side").unwrap();
        commit("s.txt", "s\n");
        switch(dir, "main").unwrap();
        commit("c.txt", "c\n");
        // A snapshot with a file nobody committed, then the file changes.
        std::fs::write(repo.join("draft.txt"), "first\n").unwrap();
        let snap = checkpoint(dir).unwrap();
        std::fs::write(repo.join("draft.txt"), "second\n").unwrap();
        delete_branch(dir, "side", true).unwrap();

        let list = moments(dir, &[(snap.clone(), "Before the agent".into())], 50);
        // Steps in the same second can sort either side of the snapshot.
        let snapshot = list
            .iter()
            .find(|m| m.kind == MomentKind::Snapshot)
            .unwrap();
        assert_eq!(snapshot.commit, snap);
        let labels: Vec<&str> = list.iter().map(|m| m.label.as_str()).collect();
        assert!(
            labels.contains(&"Committed \u{201c}c.txt\u{201d}"),
            "{labels:?}"
        );
        assert!(labels.contains(&"Switched from side to main"), "{labels:?}");
        let on_side = list
            .iter()
            .find(|m| m.label == "Committed \u{201c}s.txt\u{201d}")
            .unwrap();
        assert_eq!(on_side.branch, "side");
        let point = |undo: Undo| UndoPoint {
            dir: dir.to_string(),
            label: String::new(),
            undo,
        };

        // A deleted branch comes back, and Undo removes it again.
        let (note, undo) = travel(dir, on_side).unwrap();
        assert!(note.starts_with("Brought side back"), "{note}");
        assert!(has_branch(dir, "side"));
        super::undo(&point(undo)).unwrap();
        assert!(!has_branch(dir, "side"));

        // The checked-out branch goes back and keeps the uncommitted draft.
        let at_b_moment = list
            .iter()
            .find(|m| m.commit == at_b && m.branch == "main")
            .unwrap();
        let (_, undo) = travel(dir, at_b_moment).unwrap();
        assert_eq!(rev(dir, "HEAD").unwrap(), at_b);
        assert_eq!(
            std::fs::read_to_string(repo.join("draft.txt")).unwrap(),
            "second\n"
        );
        super::undo(&point(undo)).unwrap();
        assert!(repo.join("c.txt").exists());

        // A snapshot puts the files back, and Undo brings now back.
        assert_eq!(differs_from_now(dir, &snap), vec!["draft.txt"]);
        let (_, undo) = travel(dir, snapshot).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("draft.txt")).unwrap(),
            "first\n"
        );
        super::undo(&point(undo)).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("draft.txt")).unwrap(),
            "second\n"
        );
    }

    #[test]
    fn history_graph_and_commit_steps() {
        let (repo, _remote) = with_origin("history");
        let dir = repo.to_str().unwrap();
        let point = |undo: Undo| UndoPoint {
            dir: dir.to_string(),
            label: String::new(),
            undo,
        };
        let commit = |file: &str, text: &str| {
            std::fs::write(repo.join(file), text).unwrap();
            sh(&repo, &["add", "."]);
            sh(&repo, &["commit", "-qm", file]);
        };
        // main: base, then side branches off and merges back in.
        new_branch(dir, "side", "main").unwrap();
        switch(dir, "side").unwrap();
        commit("s.txt", "s\n");
        switch(dir, "main").unwrap();
        commit("m.txt", "m\n");
        sh(&repo, &["merge", "-q", "--no-edit", "side"]);
        let all = LogFilter {
            all: true,
            ..Default::default()
        };
        let rows = log(dir, &all, 50);
        assert_eq!(rows[0].parents.len(), 2);
        assert!(rows[0].refs.iter().any(|r| r == "HEAD -> main"));
        let lanes = graph(&rows);
        assert_eq!(lanes.len(), rows.len());
        // The merge opens a second lane, which joins back where side started.
        assert_eq!(lanes[0].lane, 0);
        assert!(lanes[0].bottom.contains(&(0, 1)));
        assert!(lanes.iter().any(|g| g.lane == 1));
        assert!(lanes.last().unwrap().bottom.is_empty());
        // Filters by words and by file.
        let words = LogFilter {
            text: "S.TXT".into(),
            ..all.clone()
        };
        let found = log(dir, &words, 50);
        assert!(found.iter().any(|r| r.subject == "s.txt"));
        assert!(!found.iter().any(|r| r.subject == "m.txt"));
        let people = LogFilter {
            text: "T".into(),
            ..all.clone()
        };
        assert_eq!(log(dir, &people, 50).len(), rows.len());
        let file = LogFilter {
            path: "m.txt".into(),
            ..all.clone()
        };
        assert_eq!(log(dir, &file, 50)[0].subject, "m.txt");
        let m = rows
            .iter()
            .find(|r| r.subject == "m.txt")
            .unwrap()
            .sha
            .clone();
        let info = commit_info(dir, &m);
        assert_eq!(info.files.len(), 1);
        assert_eq!(
            (info.files[0].status.as_str(), info.files[0].added),
            ("A", 1)
        );
        assert!(commit_patch(dir, &m, 10_000).contains("+m"));
        assert!(is_ancestor(dir, &m, "HEAD"));
        // Take a commit back, and undo that.
        let before = point(folder_point(dir).unwrap());
        revert(dir, &m).unwrap();
        assert!(!repo.join("m.txt").exists());
        undo(&before).unwrap();
        assert!(repo.join("m.txt").exists());
        // Copy a commit to a branch that isn't checked out.
        new_branch(dir, "release", "main~2").unwrap();
        copy_to(dir, &m, "release").unwrap();
        assert_eq!(commits_ahead(dir, "release", "main~2"), 1);
        assert!(copy_to(dir, &m, "main").unwrap_err().contains("already"));
        assert!(copy_to(dir, &m, "release").unwrap_err().contains("already"));
        assert_eq!(current_branch(dir), "main");
        // A release name, taken back.
        tag(dir, "v1.0", &m).unwrap();
        assert!(log(dir, &all, 50)
            .iter()
            .any(|r| r.refs.iter().any(|n| n == "tag: v1.0")));
        undo(&point(Undo::Untag("v1.0".into()))).unwrap();
        assert!(rev(dir, "refs/tags/v1.0").is_none());
        // Go back keeps uncommitted work, and can be undone.
        std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
        let before = point(folder_point(dir).unwrap());
        assert_eq!(go_back(dir, &m).unwrap(), 2);
        assert!(!repo.join("s.txt").exists());
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "edited\n"
        );
        undo(&before).unwrap();
        assert!(repo.join("s.txt").exists());
        // Not part of main: refused.
        switch(dir, "release").unwrap();
        commit("r.txt", "r\n");
        switch(dir, "main").unwrap();
        assert!(go_back(dir, "release").is_err());
    }

    #[test]
    fn branches_merge_three_ways_and_set_aside() {
        let (repo, remote) = with_origin("branches");
        let dir = repo.to_str().unwrap();
        let point = |undo: Undo| UndoPoint {
            dir: dir.to_string(),
            label: String::new(),
            undo,
        };
        // A branch with two commits, and main moving on by one.
        let made = point(branches_point(dir, &["topic"]));
        new_branch(dir, "topic", "main").unwrap();
        switch(dir, "topic").unwrap();
        for (file, text) in [("t1.txt", "1\n"), ("t2.txt", "2\n")] {
            std::fs::write(repo.join(file), text).unwrap();
            sh(&repo, &["add", "."]);
            sh(&repo, &["commit", "-qm", file]);
        }
        switch(dir, "main").unwrap();
        std::fs::write(repo.join("m.txt"), "m\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "main moves"]);
        let rows = branch_rows(dir, "main");
        let topic = rows.iter().find(|r| r.name == "topic").unwrap();
        assert_eq!((topic.ahead, topic.behind), (2, 1));
        assert_eq!(topic.subject, "t2.txt");
        assert_eq!(
            rows.iter()
                .find(|r| r.name == "main")
                .unwrap()
                .checked_out
                .as_deref(),
            Some(dir)
        );
        assert_eq!(commits_only_on(dir, "topic", "main", 10).len(), 2);
        assert_eq!(
            branch_changes(dir, "topic", "main")
                .iter()
                .map(|c| (c.path.as_str(), c.status.as_str(), c.added))
                .collect::<Vec<_>>(),
            vec![("t1.txt", "A", 1), ("t2.txt", "A", 1)]
        );
        // Squash: one commit on main; undo puts main back.
        let before = point(branches_point(dir, &["main"]));
        merge_branch(dir, "topic", "main", MergeHow::Squash, "Topic").unwrap();
        assert_eq!(commits_ahead(dir, "main", "main~1"), 1);
        assert!(repo.join("t2.txt").exists());
        undo(&before).unwrap();
        assert!(!repo.join("t2.txt").exists());
        // Rebase: both commits copied on top, main stays straight.
        merge_branch(dir, "topic", "main", MergeHow::Rebase, "").unwrap();
        assert_eq!(
            count(dir, &["rev-list", "--count", "--merges", "main"]),
            Some(0)
        );
        assert!(repo.join("t1.txt").exists() && repo.join("t2.txt").exists());
        undo(&before).unwrap();
        // Merge from a temporary worktree when main is not checked out anywhere.
        switch(dir, "topic").unwrap();
        merge_branch(dir, "topic", "main", MergeHow::Merge, "").unwrap();
        assert_eq!(
            count(dir, &["rev-list", "--count", "--merges", "main"]),
            Some(1)
        );
        undo(&before).unwrap();
        assert_eq!(
            count(dir, &["rev-list", "--count", "--merges", "main"]),
            Some(0)
        );
        switch(dir, "main").unwrap();
        // Rename, delete and making a branch can all be undone.
        rename_branch(dir, "topic", "feature").unwrap();
        undo(&point(Undo::Rename {
            from: "topic".into(),
            to: "feature".into(),
        }))
        .unwrap();
        assert!(has_branch(dir, "topic"));
        let gone = point(branches_point(dir, &["topic"]));
        delete_branch(dir, "topic", true).unwrap();
        undo(&gone).unwrap();
        assert_eq!(commits_ahead(dir, "topic", "main"), 2);
        switch(dir, "main").unwrap();
        // Set aside, bring back, drop and restore.
        std::fs::write(repo.join("a.txt"), "edited\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new\n").unwrap();
        stash_push(dir, "half done").unwrap();
        assert!(changes(dir).is_empty());
        let list = stashes(dir);
        assert_eq!(list[0].message, "half done");
        undo(&point(Undo::Unstash)).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).unwrap(),
            "edited\n"
        );
        assert!(repo.join("new.txt").exists());
        stash_push(dir, "again").unwrap();
        let kept = stashes(dir).remove(0);
        stash_drop(dir, 0).unwrap();
        assert!(stashes(dir).is_empty());
        undo(&point(Undo::Restash {
            commit: kept.commit,
            message: "again".into(),
        }))
        .unwrap();
        stash_apply(dir, 0).unwrap();
        assert!(repo.join("new.txt").exists());
        // A branch only on the remote.
        sh(&repo, &["push", "-q", "origin", "topic:shared"]);
        fetch(dir).unwrap();
        assert!(branch_rows(dir, "main")
            .iter()
            .any(|r| r.name == "shared" && r.remote_only));
        get_remote_branch(dir, "shared").unwrap();
        assert!(branch_rows(dir, "main")
            .iter()
            .any(|r| r.name == "shared" && !r.remote_only && r.upstream == "origin/shared"));
        // Taking back the step that made a branch deletes it.
        undo(&made).unwrap();
        assert!(!has_branch(dir, "topic"));
        for p in [&repo, &remote] {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[test]
    fn update_merge_and_clean_up_a_session_branch() {
        let (repo, remote) = with_origin("land");
        let dir = repo.to_str().unwrap();
        let wt = add_worktree(dir, "runo/land", Some("main")).unwrap();
        let w = Path::new(&wt);
        // The session edits line 1; main edits line 3; no overlap.
        std::fs::write(w.join("a.txt"), "one\n2\n3\n").unwrap();
        sh(w, &["commit", "-qam", "session"]);
        std::fs::write(repo.join("a.txt"), "1\n2\nthree\n").unwrap();
        sh(&repo, &["commit", "-qam", "main moved"]);
        // Uncommitted work rides along.
        std::fs::write(w.join("wip.txt"), "wip\n").unwrap();
        assert_eq!(update_from(&wt, "main", true).unwrap(), Update::Updated);
        assert_eq!(
            std::fs::read_to_string(w.join("a.txt")).unwrap(),
            "one\n2\nthree\n"
        );
        assert!(w.join("wip.txt").exists());
        assert_eq!(update_from(&wt, "main", true).unwrap(), Update::UpToDate);

        // Main and the session now both change line 2: a conflict stays for resolving.
        std::fs::remove_file(w.join("wip.txt")).unwrap();
        std::fs::write(w.join("a.txt"), "one\nTWO-session\nthree\n").unwrap();
        sh(w, &["commit", "-qam", "session 2"]);
        std::fs::write(repo.join("a.txt"), "1\nTWO-main\nthree\n").unwrap();
        sh(&repo, &["commit", "-qam", "main 2"]);
        assert_eq!(
            update_from(&wt, "main", false).unwrap(),
            Update::Conflicts(vec!["a.txt".into()])
        );
        assert_eq!(in_progress(&wt), Some("merge"));
        assert_eq!(
            continue_update(&wt).unwrap(),
            Update::Conflicts(vec!["a.txt".into()])
        );
        std::fs::write(w.join("a.txt"), "one\nTWO-both\nthree\n").unwrap();
        sh(w, &["add", "a.txt"]);
        assert_eq!(continue_update(&wt).unwrap(), Update::Updated);
        assert_eq!(in_progress(&wt), None);

        // A rebase that conflicts can be aborted back to where it was.
        std::fs::write(w.join("a.txt"), "x\n").unwrap();
        sh(w, &["commit", "-qam", "session 3"]);
        std::fs::write(repo.join("a.txt"), "y\n").unwrap();
        sh(&repo, &["commit", "-qam", "main 3"]);
        let before = rev(&wt, "HEAD").unwrap();
        assert!(matches!(
            update_from(&wt, "main", true).unwrap(),
            Update::Conflicts(_)
        ));
        assert_eq!(in_progress(&wt), Some("rebase"));
        abort_update(&wt).unwrap();
        assert_eq!(rev(&wt, "HEAD").unwrap(), before);

        // Merging into main where main is checked out: conflicts abort cleanly.
        assert!(merge_into(dir, "runo/land", "main").is_err());
        assert_eq!(in_progress(dir), None);
        sh(&repo, &["reset", "-q", "--hard", "HEAD~1"]);
        assert_eq!(checked_out_at(dir, "main").as_deref(), Some(dir));
        let hash = merge_into(dir, "runo/land", "main").unwrap();
        assert!(!hash.is_empty());
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "x\n");
        // Main checked out nowhere: a temporary worktree does the merge.
        sh(&repo, &["checkout", "-q", "-b", "elsewhere"]);
        std::fs::write(w.join("z.txt"), "z\n").unwrap();
        sh(w, &["add", "."]);
        sh(w, &["commit", "-qm", "more"]);
        merge_into(dir, "runo/land", "main").unwrap();
        assert_eq!(commits_ahead(dir, "runo/land", "main"), 0);
        assert!(checked_out_at(dir, "main").is_none());

        remove_worktree(dir, &wt).unwrap();
        assert!(delete_branch_if_merged(dir, "runo/land", "main"));
        assert!(!exists(dir, "refs/heads/runo/land"));
        // A branch with unmerged commits is kept.
        sh(&repo, &["checkout", "-q", "-b", "unmerged"]);
        std::fs::write(repo.join("u.txt"), "u\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "u"]);
        sh(&repo, &["checkout", "-q", "main"]);
        assert!(!delete_branch_if_merged(dir, "unmerged", "main"));
        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&remote);
    }

    #[test]
    fn search_in_files() {
        let repo = repo("grep");
        let dir = repo.to_str().unwrap();
        std::fs::write(repo.join("a.rs"), "fn main() {\n    Hello();\n}\n").unwrap();
        std::fs::write(repo.join("b.txt"), "say hello\n").unwrap();
        sh(&repo, &["add", "a.rs"]);
        let (hits, more) = grep(dir, "hello", 10);
        assert!(!more);
        assert_eq!(
            hits,
            [
                Hit {
                    path: "a.rs".into(),
                    line: 2,
                    text: "Hello();".into()
                },
                Hit {
                    path: "b.txt".into(),
                    line: 1,
                    text: "say hello".into()
                },
            ]
        );
        assert_eq!(grep(dir, "hello", 1), (hits[..1].to_vec(), true));
        assert!(grep(dir, "nothing-like-this", 10).0.is_empty());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn branch_names_come_from_the_task() {
        assert_eq!(
            branch_slug("Fix the login redirect\nmore detail"),
            "runo/fix-login-redirect"
        );
        assert_eq!(
            branch_slug("\n  Please add a dark-mode toggle to the Settings page"),
            "runo/add-dark-mode-toggle-settings"
        );
        assert_eq!(branch_slug("修复 bug"), "runo/bug");
        assert_eq!(branch_slug("!!! ???"), "");
        assert!(branch_slug(&"x".repeat(80)).len() <= "runo/".len() + 40);
        let taken = ["runo/a", "runo/a-2"];
        assert_eq!(unique_branch("runo/a", |b| taken.contains(&b)), "runo/a-3");
        assert_eq!(unique_branch("runo/b", |b| taken.contains(&b)), "runo/b");
    }

    #[test]
    fn clone_names_follow_the_url() {
        assert_eq!(clone_dir_name("https://github.com/a/runo.git"), "runo");
        assert_eq!(clone_dir_name("git@github.com:a/runo.git/"), "runo");
        assert_eq!(clone_dir_name("git@host:runo"), "runo");
        assert_eq!(clone_dir_name("/srv/repos/app"), "app");
    }

    #[test]
    fn clones_list_their_branches() {
        let origin = repo("clone-src");
        std::fs::write(origin.join("a.txt"), "one\n").unwrap();
        sh(&origin, &["add", "-A"]);
        sh(&origin, &["commit", "-q", "-m", "init"]);
        sh(&origin, &["branch", "feature"]);
        let dest = std::env::temp_dir().join(format!("runo-git-clone-dst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);
        let mut lines = Vec::new();
        clone(origin.to_str().unwrap(), &dest, |l| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert!(dest.join("a.txt").exists());
        assert!(!lines.is_empty());
        // Cloning again into the same folder fails with a clear reason.
        let again = clone(origin.to_str().unwrap(), &dest, |_| {}).unwrap_err();
        assert!(again.contains("already exists"));
        let d = dest.to_str().unwrap();
        let listed = branches(d);
        let current = current_branch(d);
        assert_eq!(listed.first(), Some(&(current.clone(), false)));
        assert!(listed.contains(&("feature".to_string(), true)));
        assert!(!listed.iter().any(|(n, _)| n == "HEAD"));
        assert!(has_branch(d, &current));
        assert!(!has_branch(d, "feature"));
        let missing = dest.with_extension("missing");
        assert!(clone("/no/such/repo", &missing, |_| {}).is_err());
        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&origin);
    }

    #[test]
    fn credential_errors_point_at_the_fix() {
        assert!(auth_hint(
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
        )
        .contains("gh auth setup-git"));
        assert_eq!(auth_hint("fatal: other"), "fatal: other");
    }
}
