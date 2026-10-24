//! Settings > Worktrees > Clean up: worktrees, folders and `runo/*` branches that no
//! session uses any more. Sessions remove their own worktree when they are deleted or
//! cleaned up after a merge; this finds what was left behind some other way.

use crate::git;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Branches Runo makes for sessions start with this.
pub const PREFIX: &str = "runo/";

/// One entry of `git worktree list --porcelain`.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub path: PathBuf,
    /// The checked-out branch, without `refs/heads/`; None when detached.
    pub branch: Option<String>,
    /// Its folder is gone, so git would prune it.
    pub missing: bool,
}

/// Parse `git worktree list --porcelain`.
pub fn parse_worktrees(porcelain: &str) -> Vec<Listed> {
    let mut out: Vec<Listed> = Vec::new();
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            out.push(Listed {
                path: PathBuf::from(path),
                branch: None,
                missing: false,
            });
        } else if let Some(last) = out.last_mut() {
            if let Some(branch) = line.strip_prefix("branch ") {
                last.branch = Some(branch.trim_start_matches("refs/heads/").to_string());
            } else if line == "prunable" || line.starts_with("prunable ") {
                last.missing = true;
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum What {
    /// A worktree git knows about.
    Worktree,
    /// A worktree git lists whose folder is gone.
    Missing,
    /// A folder under `.runo-worktrees` that is not a worktree git knows about.
    Folder,
    /// A local `runo/*` branch.
    Branch,
}

/// Something no session uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub what: What,
    /// The folder; empty for a branch.
    pub path: PathBuf,
    /// The branch, or the branch a worktree has checked out ("" when detached).
    pub branch: String,
}

/// What one repository has and what Runo's sessions use, for [`orphans`]. Paths are
/// compared as given, so they must all be made comparable first (see [`norm`]).
#[derive(Default)]
pub struct Facts {
    /// The repository's folder under `.runo-worktrees`, as `<parent>/.runo-worktrees/<repo>`.
    pub root: PathBuf,
    /// Folders sessions run in, archived ones included, and the projects themselves.
    pub used_dirs: Vec<PathBuf>,
    /// Branches this repository's sessions are on.
    pub used_branches: HashSet<String>,
    /// Branch prefixes of work in progress that has no session yet.
    pub held: Vec<String>,
    pub worktrees: Vec<Listed>,
    /// Top folders under `root`: worktrees and leftover folders (see [`folders`]).
    pub folders: Vec<PathBuf>,
    /// Local `runo/*` branches.
    pub branches: Vec<String>,
}

impl Facts {
    /// A folder is in use when a session runs in it, under it or above it.
    fn dir_used(&self, path: &Path) -> bool {
        self.used_dirs
            .iter()
            .any(|d| d == path || d.starts_with(path) || path.starts_with(d))
    }

    fn held(&self, name: &str) -> bool {
        self.held.iter().any(|p| name.starts_with(p.as_str()))
    }

    /// Only folders strictly inside this repository's `.runo-worktrees` folder qualify.
    fn inside(&self, path: &Path) -> bool {
        !self.root.as_os_str().is_empty() && path != self.root && path.starts_with(&self.root)
    }

    /// The folder's path under `root`, which is its branch for folders Runo made.
    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }
}

/// What no session uses: worktrees and folders first, then branches.
pub fn orphans(facts: &Facts) -> Vec<Found> {
    let mut out = Vec::new();
    let mut listed: HashSet<&Path> = HashSet::new();
    // Branches checked out somewhere that stays, such as the project folder or a
    // session's worktree, are in use too.
    let mut kept_branches: HashSet<&str> = HashSet::new();
    for wt in &facts.worktrees {
        listed.insert(&wt.path);
        let branch = wt.branch.as_deref().unwrap_or("");
        let free = facts.inside(&wt.path)
            && !facts.dir_used(&wt.path)
            && !facts.used_branches.contains(branch)
            && !facts.held(branch)
            && !facts.held(&facts.relative(&wt.path));
        if free {
            out.push(Found {
                what: if wt.missing {
                    What::Missing
                } else {
                    What::Worktree
                },
                path: wt.path.clone(),
                branch: branch.to_string(),
            });
        } else if !branch.is_empty() {
            kept_branches.insert(branch);
        }
    }
    for folder in &facts.folders {
        if listed.contains(folder.as_path())
            || !facts.inside(folder)
            || facts.dir_used(folder)
            || facts.held(&facts.relative(folder))
            // A registered worktree inside it: that one is listed on its own.
            || facts.worktrees.iter().any(|w| w.path.starts_with(folder))
        {
            continue;
        }
        out.push(Found {
            what: What::Folder,
            path: folder.clone(),
            branch: String::new(),
        });
    }
    for branch in &facts.branches {
        if !branch.starts_with(PREFIX)
            || facts.used_branches.contains(branch)
            || kept_branches.contains(branch.as_str())
            || facts.held(branch)
        {
            continue;
        }
        out.push(Found {
            what: What::Branch,
            path: PathBuf::new(),
            branch: branch.clone(),
        });
    }
    out
}

/// A path made comparable with what git reports: the part that exists resolved to its
/// real path (on a Mac the temporary folder is a symlink), the rest kept as written.
pub fn norm(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut head = path;
    loop {
        if let Ok(real) = head.canonicalize() {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (head.parent(), head.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                head = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The folders under `root` that could be worktrees, or are left over from one. Folders
/// made for branches with slashes (the `runo` of `runo/x`) are walked through: those
/// hold nothing but folders, and somewhere below them a worktree. Symlinks are never
/// followed.
pub fn folders(root: &Path) -> Vec<PathBuf> {
    /// Subfolders, or None when the folder holds files (`.DS_Store` aside) or a `.git`.
    fn only_folders(dir: &Path) -> Option<Vec<PathBuf>> {
        let mut subdirs = Vec::new();
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let name = entry.file_name();
            if name == ".git" {
                return None;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => subdirs.push(entry.path()),
                _ if name == ".DS_Store" => {}
                _ => return None,
            }
        }
        Some(subdirs)
    }
    /// Whether the folder only leads to worktrees further down.
    fn passes_through(dir: &Path, depth: usize) -> bool {
        depth < 8
            && only_folders(dir).is_some_and(|subs| {
                subs.iter().any(|s| {
                    s.join(".git").symlink_metadata().is_ok() || passes_through(s, depth + 1)
                })
            })
    }
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        for sub in only_folders(dir).unwrap_or_default() {
            if passes_through(&sub, depth) {
                walk(&sub, depth + 1, out);
            } else {
                out.push(sub);
            }
        }
    }
    let mut out = Vec::new();
    // The repository's own folder may hold stray files; look at its folders anyway.
    let top = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for sub in top {
        if passes_through(&sub, 1) {
            walk(&sub, 2, &mut out);
        } else {
            out.push(sub);
        }
    }
    out.sort();
    out
}

/// Whether `meta` (from `symlink_metadata`) is a link: a symlink, or on Windows any
/// reparse point, junctions included (pnpm's node_modules is full of them).
pub fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// Bytes the files under `path` take. Links are neither followed nor counted.
pub fn size(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else {
        return 0;
    };
    if is_link(&meta) {
        return 0;
    }
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| entries.flatten().map(|e| size(&e.path())).sum())
        .unwrap_or(0)
}

/// Bytes for people: 12 KB, 3.4 MB, 1.2 GB.
pub fn human(bytes: u64) -> String {
    const KB: f64 = 1024.;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} bytes")
    }
}

/// A project to scan.
#[derive(Clone)]
pub struct Project {
    pub name: String,
    pub path: String,
}

/// What a session holds on to.
#[derive(Clone)]
pub struct Use {
    /// The session's project folder.
    pub project: String,
    pub dir: String,
    pub branch: String,
}

/// Something to clean up, as listed in Settings.
#[derive(Clone, Debug)]
pub struct Item {
    pub repo: String,
    pub project: String,
    pub found: Found,
    pub size: u64,
    /// The project's default branch has all of the branch's commits (or there is no
    /// branch, as for a detached worktree).
    pub merged: bool,
    /// Commits on the branch that the default branch lacks.
    pub ahead: usize,
    pub base: String,
    /// A worktree with uncommitted changes, or a folder git can't tell about.
    pub dirty: bool,
}

impl Item {
    /// Stable across scans, for remembering what is picked.
    pub fn key(&self) -> String {
        format!(
            "{}\n{:?}\n{}\n{}",
            self.repo,
            self.found.what,
            self.found.path.display(),
            self.found.branch
        )
    }

    /// Removing it may lose work, so it is listed apart and never picked for you.
    pub fn careful(&self) -> bool {
        !self.merged || self.dirty
    }

    pub fn title(&self) -> String {
        match self.found.what {
            What::Branch => self.found.branch.clone(),
            _ => self
                .found
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    /// One line on what it is and why it may matter.
    pub fn detail(&self) -> String {
        let place = || crate::settings_ui::tilde(&self.found.path);
        let not_merged = || {
            format!(
                "{} commit{} not on {}",
                self.ahead,
                if self.ahead == 1 { "" } else { "s" },
                self.base
            )
        };
        let mut parts = vec![self.project.clone()];
        match self.found.what {
            What::Worktree => {
                parts.push(format!("worktree at {}", place()));
                if !self.found.branch.is_empty() {
                    parts.push(format!("on {}", self.found.branch));
                }
                if self.dirty {
                    parts.push("uncommitted changes".into());
                }
                if !self.merged {
                    parts.push(not_merged());
                }
            }
            What::Missing => parts.push(format!("folder already gone: {}", place())),
            What::Folder => {
                parts.push(format!("folder at {}", place()));
                if self.dirty {
                    parts.push("not a worktree git knows, may hold files never committed".into());
                }
            }
            What::Branch => {
                if self.merged {
                    parts.push(format!("merged into {}", self.base));
                } else {
                    parts.push(not_merged());
                }
            }
        }
        parts.join(" · ")
    }
}

/// The repository's folder under `.runo-worktrees`.
fn root_of(repo: &str) -> PathBuf {
    git::worktree_path(repo, "")
}

/// Find what no session uses in every project. Slow (it walks folders for their size),
/// so run it off the UI thread. `sizes` false skips the sizes.
pub fn scan(projects: &[Project], uses: &[Use], sizes: bool) -> Vec<Item> {
    let (run_dirs, run_branches, held) = crate::pbrunner::in_flight();
    let mut used_dirs: Vec<PathBuf> = uses
        .iter()
        .map(|u| u.dir.as_str())
        .chain(projects.iter().map(|p| p.path.as_str()))
        .chain(run_dirs.iter().map(String::as_str))
        .filter(|d| !d.is_empty())
        .map(|d| norm(Path::new(d)))
        .collect();
    used_dirs.sort();
    used_dirs.dedup();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for project in projects {
        let repo_path = norm(Path::new(&project.path));
        if !seen.insert(repo_path.clone()) || !git::is_repo(&project.path) {
            continue;
        }
        // Git reports where the repository's own worktree is; a project inside a
        // repository (a subfolder) has none of its own.
        let Ok(list) = git::worktree_list(&project.path) else {
            continue;
        };
        let mut worktrees = parse_worktrees(&list);
        if worktrees.first().map(|w| norm(&w.path)) != Some(repo_path.clone()) {
            continue;
        }
        for wt in &mut worktrees {
            wt.path = norm(&wt.path);
            wt.missing = wt.missing || !wt.path.exists();
        }
        let root = norm(&root_of(&project.path));
        let used_branches: HashSet<String> = uses
            .iter()
            .filter(|u| norm(Path::new(&u.project)) == repo_path)
            .map(|u| u.branch.clone())
            .chain(run_branches.iter().cloned())
            .filter(|b| !b.is_empty())
            .collect();
        let facts = Facts {
            folders: folders(&root),
            root,
            used_dirs: used_dirs.clone(),
            used_branches,
            held: held.clone(),
            worktrees,
            branches: git::local_branches(&project.path, PREFIX),
        };
        let found = orphans(&facts);
        if found.is_empty() {
            continue;
        }
        let base = git::default_branch(&project.path);
        let base_ref = git::base_ref(&project.path, &base);
        for found in found {
            let ahead = if found.branch.is_empty() || found.what == What::Missing {
                0
            } else {
                git::commits_ahead(&project.path, &found.branch, &base_ref)
            };
            let dirty = match found.what {
                What::Worktree => {
                    git::has_uncommitted(&found.path.to_string_lossy()).unwrap_or(true)
                }
                What::Folder => found.path.join(".git").symlink_metadata().is_ok(),
                _ => false,
            };
            let size = match found.what {
                What::Worktree | What::Folder if sizes => size(&found.path),
                _ => 0,
            };
            out.push(Item {
                repo: project.path.clone(),
                project: project.name.clone(),
                found,
                size,
                merged: ahead == 0,
                ahead,
                base: base.clone(),
                dirty,
            });
        }
    }
    out
}

/// Remove one item. `root` guards against anything outside `.runo-worktrees`.
fn remove_one(item: &Item) -> Result<(), String> {
    let root = norm(&root_of(&item.repo));
    let path = &item.found.path;
    let inside = || {
        if path != &root && path.starts_with(&root) {
            Ok(())
        } else {
            Err(format!("{} is outside .runo-worktrees", path.display()))
        }
    };
    let dir = path.to_string_lossy();
    match item.found.what {
        What::Worktree => {
            inside()?;
            git::remove_worktree(&item.repo, &dir)?;
        }
        What::Missing => {
            inside()?;
            if git::remove_worktree(&item.repo, &dir).is_err() {
                if path.exists() {
                    std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
                }
                git::prune_worktrees(&item.repo)?;
            }
        }
        What::Folder => {
            inside()?;
            let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
            }
        }
        What::Branch => {
            if !item.found.branch.starts_with(PREFIX) {
                return Err(format!("{} is not a Runo branch", item.found.branch));
            }
            git::delete_branch(&item.repo, &item.found.branch, true)?;
        }
    }
    // Folders left empty by the removal, such as `runo/` for `runo/x`.
    if item.found.what != What::Branch {
        let mut up = path.parent();
        while let Some(dir) = up.filter(|d| *d != root && d.starts_with(&root)) {
            if std::fs::remove_dir(dir).is_err() {
                break;
            }
            up = dir.parent();
        }
    }
    Ok(())
}

/// Remove the picked items that still count as unused when this runs, worktrees and
/// folders before branches (a branch checked out in a worktree can't be deleted).
/// Returns each item with how it went; items something now uses are skipped with a
/// reason.
pub fn remove(
    picked: Vec<Item>,
    projects: &[Project],
    uses: &[Use],
) -> Vec<(Item, Result<(), String>)> {
    let fresh: HashSet<String> = scan(projects, uses, false).iter().map(Item::key).collect();
    let mut picked = picked;
    picked.sort_by_key(|i| i.found.what == What::Branch);
    picked
        .into_iter()
        .map(|item| {
            let result = if fresh.contains(&item.key()) {
                remove_one(&item)
            } else {
                Err("now in use, or already gone; left alone".into())
            };
            (item, result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    const LIST: &str = "worktree /code/app
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /code/.runo-worktrees/app/runo/live
HEAD 2222222222222222222222222222222222222222
branch refs/heads/runo/live

worktree /code/.runo-worktrees/app/runo/old
HEAD 3333333333333333333333333333333333333333
branch refs/heads/runo/old

worktree /code/.runo-worktrees/app/runo/gone
HEAD 4444444444444444444444444444444444444444
branch refs/heads/runo/gone
prunable gitdir file points to non-existent location

worktree /code/.git/runo-merge-1
HEAD 5555555555555555555555555555555555555555
detached

worktree /elsewhere/app-copy
HEAD 6666666666666666666666666666666666666666
branch refs/heads/runo/manual
";

    fn facts() -> Facts {
        Facts {
            root: p("/code/.runo-worktrees/app"),
            used_dirs: vec![p("/code/app"), p("/code/.runo-worktrees/app/runo/live")],
            used_branches: ["runo/live".to_string()].into_iter().collect(),
            held: Vec::new(),
            worktrees: parse_worktrees(LIST),
            folders: vec![
                p("/code/.runo-worktrees/app/runo/live"),
                p("/code/.runo-worktrees/app/runo/old"),
                p("/code/.runo-worktrees/app/runo/stray"),
            ],
            branches: vec![
                "runo/live".into(),
                "runo/old".into(),
                "runo/gone".into(),
                "runo/manual".into(),
                "runo/lonely".into(),
                "feature/mine".into(),
            ],
        }
    }

    fn summary(found: &[Found]) -> Vec<(What, String)> {
        found
            .iter()
            .map(|f| {
                let name = if f.what == What::Branch {
                    f.branch.clone()
                } else {
                    f.path.display().to_string()
                };
                (f.what, name)
            })
            .collect()
    }

    #[test]
    fn parses_worktree_list() {
        let list = parse_worktrees(LIST);
        assert_eq!(list.len(), 6);
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert!(!list[2].missing);
        assert!(list[3].missing);
        assert_eq!(list[4].branch, None);
        assert_eq!(list[5].path, p("/elsewhere/app-copy"));
    }

    #[test]
    fn orphans_are_what_no_session_uses() {
        assert_eq!(
            summary(&orphans(&facts())),
            [
                (What::Worktree, "/code/.runo-worktrees/app/runo/old".into()),
                (What::Missing, "/code/.runo-worktrees/app/runo/gone".into()),
                (What::Folder, "/code/.runo-worktrees/app/runo/stray".into()),
                (What::Branch, "runo/old".into()),
                (What::Branch, "runo/gone".into()),
                (What::Branch, "runo/lonely".into()),
            ]
        );
    }

    #[test]
    fn archived_and_nested_session_folders_stay() {
        let mut f = facts();
        // An archived session still points at the old worktree.
        f.used_dirs.push(p("/code/.runo-worktrees/app/runo/old"));
        f.used_branches.insert("runo/old".into());
        // A session runs in a folder below the stray one, which is then in use too.
        f.used_dirs
            .push(p("/code/.runo-worktrees/app/runo/stray/inner"));
        let found = summary(&orphans(&f));
        assert!(!found.iter().any(|(_, n)| n.contains("old")));
        assert!(!found.iter().any(|(_, n)| n.contains("stray")));
    }

    #[test]
    fn nothing_outside_the_worktree_folder_or_off_prefix() {
        let found = orphans(&facts());
        // The merge's temporary worktree and a worktree elsewhere are never listed, and
        // the branch checked out in the one elsewhere is kept.
        assert!(found
            .iter()
            .filter(|f| f.what != What::Branch)
            .all(|f| f.path.starts_with("/code/.runo-worktrees/app/")));
        assert!(!found.iter().any(|f| f.branch == "runo/manual"));
        assert!(!found.iter().any(|f| f.branch == "feature/mine"));
        // No root, no folders.
        let mut f = facts();
        f.root = PathBuf::new();
        assert!(orphans(&f).iter().all(|f| f.what == What::Branch));
    }

    #[test]
    fn work_in_progress_is_held() {
        let mut f = facts();
        f.held.push("runo/ol".into());
        f.held.push("runo/lone".into());
        let found = summary(&orphans(&f));
        assert!(!found.iter().any(|(_, n)| n.contains("old")));
        assert!(!found.iter().any(|(_, n)| n == "runo/lonely"));
    }

    #[test]
    fn sizes_read_for_people() {
        assert_eq!(human(10), "10 bytes");
        assert_eq!(human(2048), "2 KB");
        assert_eq!(human(5 << 20), "5.0 MB");
        assert_eq!(human(3 << 30), "3.0 GB");
    }

    fn sh(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn scans_and_removes_in_a_real_repository() {
        let base = std::env::temp_dir().join(format!("runo-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        let repo = repo.canonicalize().unwrap();
        let r = repo.to_str().unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        sh(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);

        let live = git::add_worktree(r, "runo/live", None).unwrap();
        let old = git::add_worktree(r, "runo/old", None).unwrap();
        let gone = git::add_worktree(r, "runo/gone", None).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        // Unmerged work on a branch with no worktree.
        sh(&repo, &["branch", "runo/work"]);
        let work = git::add_worktree(r, "runo/work", None).unwrap();
        std::fs::write(Path::new(&work).join("f.txt"), "x").unwrap();
        sh(Path::new(&work), &["add", "f.txt"]);
        sh(Path::new(&work), &["commit", "-q", "-m", "work"]);
        git::remove_worktree(r, &work).unwrap();
        // A leftover folder, such as node_modules after a failed removal.
        let stray = root_of(r).join("runo").join("stray");
        std::fs::create_dir_all(stray.join("node_modules")).unwrap();
        std::fs::write(stray.join("node_modules").join("big"), vec![0u8; 4096]).unwrap();

        let projects = [Project {
            name: "app".into(),
            path: r.into(),
        }];
        let uses = [Use {
            project: r.into(),
            dir: live.clone(),
            branch: "runo/live".into(),
        }];
        let items = scan(&projects, &uses, true);
        let mut names: Vec<(What, String, bool)> = items
            .iter()
            .map(|i| (i.found.what, i.title(), i.careful()))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                (What::Worktree, "old".into(), false),
                (What::Missing, "gone".into(), false),
                (What::Folder, "stray".into(), false),
                (What::Branch, "runo/gone".into(), false),
                (What::Branch, "runo/old".into(), false),
                (What::Branch, "runo/work".into(), true),
            ]
        );
        let stray_item = items.iter().find(|i| i.found.what == What::Folder).unwrap();
        assert!(stray_item.size >= 4096);
        let work_item = items
            .iter()
            .find(|i| i.found.branch == "runo/work")
            .unwrap();
        assert_eq!((work_item.ahead, work_item.base.as_str()), (1, "main"));

        // A session picks up runo/old between the scan and the removal: it stays.
        let uses_now = [
            uses[0].clone(),
            Use {
                project: r.into(),
                dir: old.clone(),
                branch: "runo/old".into(),
            },
        ];
        let results = remove(items.clone(), &projects, &uses_now);
        for (item, result) in &results {
            let kept = item.found.path == norm(Path::new(&old)) || item.found.branch == "runo/old";
            assert_eq!(result.is_err(), kept, "{item:?}: {result:?}");
        }
        assert!(Path::new(&old).exists());
        assert!(Path::new(&live).exists());
        assert!(!stray.exists());
        assert!(git::has_branch(r, "runo/old"));
        assert!(git::has_branch(r, "runo/live"));
        assert!(!git::has_branch(r, "runo/work"));
        assert!(!git::has_branch(r, "runo/gone"));
        let list = parse_worktrees(&git::worktree_list(r).unwrap());
        assert_eq!(list.len(), 3, "{list:?}");
        assert!(scan(&projects, &uses_now, false).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
