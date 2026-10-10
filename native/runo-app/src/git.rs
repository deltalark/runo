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

/// Create `<repo-parent>/.runo-worktrees/<repo-name>/<branch>` on a new (or existing) branch.
pub fn add_worktree(repo: &str, branch: &str) -> Result<String, String> {
    let repo_path = Path::new(repo);
    let name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let dest = repo_path
        .parent()
        .unwrap_or(repo_path)
        .join(".runo-worktrees")
        .join(name)
        .join(branch.replace('/', "-"));
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
    if let Ok(out) = git(dir, &["diff", "HEAD", "--numstat"]) {
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
}
