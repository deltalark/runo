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

fn rev(dir: &str, rev: &str) -> Option<String> {
    git(
        dir,
        &["rev-parse", "-q", "--verify", &format!("{rev}^{{commit}}")],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// Whether `a` is `b` or one of its ancestors.
fn is_ancestor(dir: &str, a: &str, b: &str) -> bool {
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
    match update_from(dir, "@{u}", true)? {
        Update::Conflicts(files) => {
            abort_update(dir)?;
            Ok(Update::Conflicts(files))
        }
        done => Ok(done),
    }
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
                    "{base} has changes that aren't committed yet. Commit or set them aside first."
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
    let result = match how {
        MergeHow::Merge => git_with(&dir, &["merge", "--no-edit", branch], &editor, None),
        MergeHow::Squash => git_with(&dir, &["merge", "--squash", branch], &editor, None)
            .and_then(|_| git(&dir, &["commit", "-q", "-m", message])),
        MergeHow::Rebase => {
            let start = git(&dir, &["merge-base", "HEAD", branch])?;
            git_with(
                &dir,
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
    let outcome = match result {
        Ok(_) => git(&dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string()),
        Err(error) => {
            let files = conflicted(&dir);
            let _ = match how {
                MergeHow::Rebase => git(&dir, &["cherry-pick", "--abort"]),
                _ => git(&dir, &["merge", "--abort"])
                    .or_else(|_| git(&dir, &["reset", "--hard", "-q", "HEAD"])),
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
    };
    if temporary {
        let _ = remove_worktree(repo, &dir);
    }
    outcome
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
        assert_eq!(in_progress(dir), None);
        assert_eq!(rev(dir, "HEAD"), head);
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
