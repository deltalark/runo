//! Thin wrappers over the `git` CLI for worktrees, status and diffs.

use std::path::Path;
use std::process::Command;

#[derive(Clone)]
pub struct Change {
    pub path: String,
    pub status: String,
    pub added: usize,
    pub removed: usize,
}

fn git(dir: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
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
pub fn worktree_path(repo: &str, branch: &str) -> std::path::PathBuf {
    let repo_path = Path::new(repo);
    let name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    repo_path
        .parent()
        .unwrap_or(repo_path)
        .join(".runo-worktrees")
        .join(name)
        .join(branch.replace('/', "-"))
}

/// Create the session worktree on a new (or existing) branch.
pub fn add_worktree(repo: &str, branch: &str) -> Result<String, String> {
    let dest = worktree_path(repo, branch);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    let dest_str = dest.to_string_lossy().into_owned();
    let exists = git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok();
    if exists {
        git(repo, &["worktree", "add", &dest_str, branch])?;
    } else {
        git(repo, &["worktree", "add", "-b", branch, &dest_str])?;
    }
    Ok(dest_str)
}

pub fn remove_worktree(repo: &str, dir: &str) -> Result<(), String> {
    git(repo, &["worktree", "remove", "--force", dir]).map(|_| ())
}

pub fn changes(dir: &str) -> Vec<Change> {
    let mut numstat = std::collections::HashMap::new();
    if let Ok(out) = git(dir, &["diff", "HEAD", "--numstat", "--no-renames"]) {
        for line in out.lines() {
            let mut parts = line.splitn(3, '\t');
            if let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) {
                numstat.insert(
                    path.to_string(),
                    (a.parse().unwrap_or(0usize), r.parse().unwrap_or(0usize)),
                );
            }
        }
    }
    let mut result = Vec::new();
    let status = git(dir, &["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default();
    for line in status.lines() {
        if line.len() < 4 {
            continue;
        }
        let code = line[..2].trim().to_string();
        let path = line[3..]
            .rsplit(" -> ")
            .next()
            .unwrap_or("")
            .trim_matches('"')
            .to_string();
        let (added, removed) = if code == "??" {
            let lines = std::fs::read_to_string(Path::new(dir).join(&path))
                .map(|t| t.lines().count())
                .unwrap_or(0);
            (lines, 0)
        } else {
            numstat.get(&path).copied().unwrap_or((0, 0))
        };
        result.push(Change {
            path,
            status: if code == "??" { "A".into() } else { code },
            added,
            removed,
        });
    }
    result
}

pub fn file_diff(dir: &str, path: &str) -> String {
    let tracked = git(dir, &["ls-files", "--error-unmatch", "--", path]).is_ok();
    let out = if tracked {
        git(dir, &["diff", "HEAD", "--", path])
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

pub fn tracked_files(dir: &str) -> Vec<String> {
    git(
        dir,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
    )
    .map(|out| out.lines().take(2000).map(str::to_string).collect())
    .unwrap_or_default()
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
    let listed = |extra: &[&str]| -> Result<Vec<String>, String> {
        let mut args = vec!["ls-files", "--others", "--ignored"];
        args.extend_from_slice(extra);
        Ok(git(repo, &args)?.lines().map(str::to_string).collect())
    };
    // Only files that git ignores qualify: tracked files are already in the worktree.
    let ignored: std::collections::HashSet<String> =
        listed(&["--exclude-standard"])?.into_iter().collect();
    let mut copied = Vec::new();
    for path in listed(&[&format!("--exclude-from={include}")])? {
        if !ignored.contains(&path) {
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

/// Stage everything and commit it. Returns the short hash of the new commit.
pub fn commit_all(dir: &str, message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first.".into());
    }
    git(dir, &["add", "-A"])?;
    git(dir, &["commit", "-q", "-m", message])?;
    git(dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string())
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
    let remotes = git(dir, &["remote"]).unwrap_or_default();
    let remote = if remotes.lines().any(|r| r == "origin") {
        "origin".to_string()
    } else {
        remotes.lines().next().unwrap_or("").to_string()
    };
    let count = |args: &[&str]| {
        git(dir, args)
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
    };
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
        assert_eq!(p, Path::new("/code/.runo-worktrees/app/feature-nav"));
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
        let dir = add_worktree(r, "topic").unwrap();
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
        let dir = add_worktree(repo_str, "runo/test").unwrap();
        assert!(add_worktree(repo_str, "runo/test").is_err());
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

        assert!(commit_all(dir, "  ").is_err());
        std::fs::write(repo.join("b.txt"), "b").unwrap();
        let hash = commit_all(dir, "Add b").unwrap();
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
        commit_all(dir, "Add c").unwrap();
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

        let wt = add_worktree(dir, "include-test").unwrap();
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
}
