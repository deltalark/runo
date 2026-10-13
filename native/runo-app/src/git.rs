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

/// Create the session worktree on a new (or existing) branch. A new branch starts at
/// `start` (a commit-ish such as `origin/main`), or at HEAD when `None`.
pub fn add_worktree(repo: &str, branch: &str, start: Option<&str>) -> Result<String, String> {
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
        repo
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
    fn credential_errors_point_at_the_fix() {
        assert!(auth_hint(
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
        )
        .contains("gh auth setup-git"));
        assert_eq!(auth_hint("fatal: other"), "fatal: other");
    }
}
