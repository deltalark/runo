//! Build output in Runo's worktrees: the dependency and build folders a session's
//! builds leave behind, which finished threads don't need until they are opened and built
//! again. Shared by the auto-cleanup when a thread is done, the worktree chip in a
//! session's header and a project's Worktrees view.
//!
//! Only folders that are on [`NAMES`] *and* ignored by git count, and only strictly inside
//! a worktree under `.runo-worktrees`. Symlinks are never followed, nested repositories
//! are left alone, and a folder holding a tracked file is kept.

use crate::cleanup::{self, norm};
use crate::git;
use std::path::{Path, PathBuf};

/// Folder names that hold build output or installed dependencies.
pub const NAMES: &[&str] = &[
    "target",
    "node_modules",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    "__pycache__",
    ".pytest_cache",
    ".gradle",
    "DerivedData",
    ".build",
    "out",
    "coverage",
    ".cache",
];

/// How deep into a worktree to look for build folders.
const DEPTH: usize = 12;

/// A folder, and not a link to one (see [`cleanup::is_link`]).
fn real_dir(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|m| m.is_dir() && !cleanup::is_link(&m))
}

/// Folders under each of `dirs` named like build output, without going into them, into
/// `.git`, into nested repositories or through symlinks.
fn named(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !real_dir(&path) {
                continue;
            }
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            if NAMES.iter().any(|n| name == *n) {
                out.push(path);
            } else if depth < DEPTH && path.join(".git").symlink_metadata().is_err() {
                walk(&path, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        walk(dir, 1, &mut out);
    }
    out
}

/// The worktree as git and the file system agree on it, if it is strictly inside its
/// repository's `.runo-worktrees` folder and is a folder, not a link to one.
pub fn checked(repo: &str, worktree: &Path) -> Result<PathBuf, String> {
    let root = norm(&git::worktree_path(repo, ""));
    let path = norm(worktree);
    // Path comparisons, by component: separators and, on Windows, the case of the
    // parts that exist come out the same once both are normalized.
    if path == root || !path.starts_with(&root) || norm(Path::new(repo)) == path {
        return Err(format!("{} is outside .runo-worktrees", worktree.display()));
    }
    if !real_dir(worktree) {
        return Err(format!("{} is not a folder", worktree.display()));
    }
    Ok(path)
}

/// The build output in `worktree` (already [`checked`]): folders on [`NAMES`] that git
/// ignores and that hold no tracked file. A named folder git doesn't ignore, such as a
/// `build` folder of sources, is looked into instead.
pub fn find(worktree: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = worktree.to_string_lossy().into_owned();
    let rel = |p: &Path| {
        p.strip_prefix(worktree)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    let mut look = vec![worktree.to_path_buf()];
    // Each round looks inside the named folders the round before kept.
    for _ in 0..4 {
        let found: Vec<PathBuf> = named(&look)
            .into_iter()
            .filter(|p| p.starts_with(worktree) && p != worktree)
            .collect();
        if found.is_empty() {
            break;
        }
        let names: Vec<String> = found.iter().map(|p| rel(p)).collect();
        let ignored = git::ignored(&dir, &names)?;
        let tracked = git::tracked_in(&dir, &names)?;
        look.clear();
        for (path, name) in found.into_iter().zip(names) {
            let holds_tracked = tracked
                .iter()
                .any(|t| t == &name || t.starts_with(&format!("{name}/")));
            if ignored.iter().any(|i| i.trim_end_matches('/') == name) && !holds_tracked {
                out.push(path);
            } else {
                look.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// What Runo itself has running in a worktree's session.
#[derive(Clone, Copy, Default, Debug)]
pub struct Activity {
    pub turn: bool,
    pub checks: bool,
    pub terminal: bool,
}

/// Why build output in `worktree` must be left alone now, if it must: Runo's own work
/// there, or any process whose working folder is in it (`cwds`, normalized).
pub fn busy(activity: Activity, cwds: &[PathBuf], worktree: &Path) -> Option<String> {
    if activity.turn {
        return Some("the agent is working there".into());
    }
    if activity.checks {
        return Some("its checks are running".into());
    }
    if activity.terminal {
        return Some("its terminal is open".into());
    }
    cwds.iter()
        .any(|c| c.starts_with(worktree))
        .then(|| "a program is running in it".into())
}

/// Working folders from `lsof -d cwd -Fn`: its `n` lines.
#[cfg(any(target_os = "macos", test))]
fn parse_lsof_cwds(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|l| l.strip_prefix('n'))
        .filter(|p| p.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "macos")]
fn raw_cwds() -> Vec<PathBuf> {
    std::process::Command::new("lsof")
        .args(["-nP", "-d", "cwd", "-Fn"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| parse_lsof_cwds(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn raw_cwds() -> Vec<PathBuf> {
    std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .bytes()
                        .all(|b| b.is_ascii_digit())
                })
                .filter_map(|e| std::fs::read_link(e.path().join("cwd")).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Elsewhere (Windows) there is no cheap way to tell; Runo's own state and skipping
/// files another program holds open cover it.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn raw_cwds() -> Vec<PathBuf> {
    Vec::new()
}

/// Every process's working folder that can be read, normalized.
pub fn cwds() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = raw_cwds().iter().map(|p| norm(p)).collect();
    out.sort();
    out.dedup();
    out
}

/// A worktree's size, and how much of it is build output.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measure {
    pub total: u64,
    pub build: u64,
}

/// Measure a worktree. Slow: run it off the UI thread.
pub fn measure(repo: &str, worktree: &Path) -> Result<Measure, String> {
    let path = checked(repo, worktree)?;
    let build = find(&path)?.iter().map(|p| cleanup::size(p)).sum();
    Ok(Measure {
        total: cleanup::size(&path),
        build,
    })
}

/// How a [`free`] went.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Freed {
    pub bytes: u64,
    /// Bytes in files that couldn't be deleted, most often held open by another program
    /// on Windows; they are skipped and the rest goes on.
    pub in_use: u64,
}

impl Freed {
    /// "1.8 GB", or "1.8 GB, 20 MB in use".
    pub fn describe(&self) -> String {
        if self.in_use == 0 {
            cleanup::human(self.bytes)
        } else {
            format!(
                "{}, {} in use",
                cleanup::human(self.bytes),
                cleanup::human(self.in_use)
            )
        }
    }
}

/// Windows won't delete a read-only file; clear the attribute first. Elsewhere it is the
/// folder's permissions that count, and those are left alone.
#[cfg(windows)]
fn make_deletable(path: &Path, meta: &std::fs::Metadata) {
    let mut perms = meta.permissions();
    if perms.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(windows))]
fn make_deletable(_: &Path, _: &std::fs::Metadata) {}

/// Delete `dir` and what is in it, file by file: links are removed themselves, never
/// followed, and a file that can't be deleted is counted in `in_use` and skipped.
/// (Rust's long-path handling covers deep node_modules on Windows: the paths here are
/// absolute.)
fn remove_tree(dir: &Path, out: &mut Freed) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = path.symlink_metadata() else {
                continue;
            };
            if cleanup::is_link(&meta) {
                // A link to a folder is a folder entry on Windows, a file elsewhere.
                if std::fs::remove_file(&path).is_err() {
                    let _ = std::fs::remove_dir(&path);
                }
            } else if meta.is_dir() {
                remove_tree(&path, out);
            } else {
                make_deletable(&path, &meta);
                match std::fs::remove_file(&path) {
                    Ok(()) => out.bytes += meta.len(),
                    Err(_) => out.in_use += meta.len(),
                }
            }
        }
    }
    if let Ok(meta) = dir.symlink_metadata() {
        make_deletable(dir, &meta);
    }
    let _ = std::fs::remove_dir(dir);
}

/// Delete the build output in a worktree unless something runs there (`activity` is
/// what Runo knows of; processes are looked up here). Returns how much was freed, or why
/// it was left alone. Slow: run it off the UI thread.
pub fn free(repo: &str, worktree: &Path, activity: Activity) -> Result<Freed, String> {
    let path = checked(repo, worktree)?;
    if let Some(why) = busy(activity, &cwds(), &path) {
        return Err(format!("left alone: {why}"));
    }
    let mut freed = Freed::default();
    for dir in find(&path)? {
        // Checked again just before: still a real folder at the same real place.
        let same = real_dir(&dir) && dir.canonicalize().ok().as_deref() == Some(dir.as_path());
        if same && dir.starts_with(&path) && dir != path {
            remove_tree(&dir, &mut freed);
        }
    }
    Ok(freed)
}

/// One of a project's worktrees under `.runo-worktrees`.
#[derive(Clone, Debug, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    /// Its branch; empty when detached.
    pub branch: String,
}

/// A project's worktrees under `.runo-worktrees` that exist, from `git worktree list`.
pub fn worktrees(repo: &str) -> Vec<Worktree> {
    let Ok(list) = git::worktree_list(repo) else {
        return Vec::new();
    };
    let root = norm(&git::worktree_path(repo, ""));
    cleanup::parse_worktrees(&list)
        .into_iter()
        .map(|w| (norm(&w.path), w))
        .filter(|(p, w)| !w.missing && p != &root && p.starts_with(&root) && p.is_dir())
        .map(|(path, w)| Worktree {
            path,
            branch: w.branch.unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

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

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    /// A link to the folder `to` at `at`: a symlink, or a junction on Windows.
    fn link(to: &Path, at: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(to, at).unwrap();
        #[cfg(windows)]
        assert!(Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(at)
            .arg(to)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success());
    }

    /// A repository with a worktree under `.runo-worktrees` and an assortment of
    /// folders in it; returns (base, repo, worktree).
    fn setup(tag: &str) -> (PathBuf, String, PathBuf) {
        let base = std::env::temp_dir().join(format!("runo-build-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        let repo = repo.canonicalize().unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.email", "t@t"]);
        sh(&repo, &["config", "user.name", "t"]);
        std::fs::write(
            repo.join(".gitignore"),
            "target/\nnode_modules/\n__pycache__/\ndist/\nnotes/\n",
        )
        .unwrap();
        // A tracked file inside a folder git ignores: force-added.
        write(&repo.join("dist/keep.js"), 3);
        // A `build` folder of sources, tracked, with ignored output below it.
        write(&repo.join("build/src.rs"), 3);
        sh(&repo, &["add", ".gitignore", "build/src.rs"]);
        sh(&repo, &["add", "-f", "dist/keep.js"]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        let r = repo.to_str().unwrap().to_string();
        let wt = PathBuf::from(git::add_worktree(&r, "runo/x", None).unwrap());
        write(&wt.join("target/debug/big"), 4096);
        write(&wt.join("web/node_modules/pkg/index.js"), 2048);
        write(&wt.join("build/node_modules/a.js"), 100);
        write(&wt.join("py/__pycache__/m.pyc"), 10);
        // Named like build output but not ignored: untracked work.
        write(&wt.join("out/report.txt"), 10);
        // Ignored but not on the list.
        write(&wt.join("notes/todo.txt"), 10);
        (base, r, wt)
    }

    #[test]
    fn finds_only_ignored_build_folders() {
        let (base, repo, wt) = setup("find");
        // A link named like build output, pointing outside the worktree, and one
        // inside node_modules, as pnpm makes.
        let outside = base.join("outside");
        write(&outside.join("precious"), 10);
        link(&outside, &wt.join("web").join("dist"));
        link(
            &outside,
            &wt.join("web").join("node_modules").join("linked"),
        );
        let wt = checked(&repo, &wt).unwrap();
        let found: Vec<String> = find(&wt)
            .unwrap()
            .iter()
            .map(|p| {
                p.strip_prefix(&wt)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(
            found,
            [
                "build/node_modules",
                "py/__pycache__",
                "target",
                "web/node_modules"
            ]
        );
        let m = measure(&repo, &wt).unwrap();
        assert_eq!(m.build, 4096 + 2048 + 100 + 10);
        assert!(m.total > m.build);

        let freed = free(&repo, &wt, Activity::default()).unwrap();
        assert_eq!(
            freed,
            Freed {
                bytes: m.build,
                in_use: 0
            }
        );
        assert!(!wt.join("target").exists());
        assert!(!wt.join("web/node_modules").exists());
        // Tracked, untracked-not-ignored, ignored-but-not-listed and the link stay.
        assert!(wt.join("dist/keep.js").exists());
        assert!(wt.join("build/src.rs").exists());
        assert!(wt.join("out/report.txt").exists());
        assert!(wt.join("notes/todo.txt").exists());
        assert!(outside.join("precious").exists());
        // Links are never counted in a size.
        assert_eq!(cleanup::size(&wt.join("web").join("dist")), 0);
        // The worktree, its branch and git's record of it stay.
        assert!(git::has_branch(&repo, "runo/x"));
        assert_eq!(worktrees(&repo).len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn nothing_outside_runo_worktrees() {
        let (base, repo, _wt) = setup("outside");
        // The project folder itself is never cleaned, nor a folder elsewhere.
        write(&Path::new(&repo).join("target/debug/big"), 10);
        assert!(checked(&repo, Path::new(&repo)).is_err());
        assert!(free(&repo, Path::new(&repo), Activity::default()).is_err());
        assert!(Path::new(&repo).join("target/debug/big").exists());
        assert!(checked(&repo, &base).is_err());
        assert!(checked(&repo, &git::worktree_path(&repo, "")).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_busy_worktree_is_skipped() {
        let wt = PathBuf::from("/code/.runo-worktrees/app/runo/x");
        let idle = Activity::default();
        assert_eq!(busy(idle, &[], &wt), None);
        let others = [
            PathBuf::from("/code/app"),
            PathBuf::from("/code/.runo-worktrees/app/runo/xy"),
        ];
        assert_eq!(busy(idle, &others, &wt), None);
        assert!(busy(idle, &[wt.join("web")], &wt).is_some());
        assert!(busy(idle, std::slice::from_ref(&wt), &wt).is_some());
        for activity in [
            Activity { turn: true, ..idle },
            Activity {
                checks: true,
                ..idle
            },
            Activity {
                terminal: true,
                ..idle
            },
        ] {
            assert!(busy(activity, &[], &wt).is_some());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_program_running_in_it_keeps_its_build_output() {
        let (base, repo, wt) = setup("busy");
        let wt = checked(&repo, &wt).unwrap();
        assert!(free(
            &repo,
            &wt,
            Activity {
                turn: true,
                ..Activity::default()
            }
        )
        .is_err());
        let mut child = Command::new("sleep")
            .arg("30")
            .current_dir(wt.join("web"))
            .spawn()
            .unwrap();
        // lsof and /proc both see it at once.
        let result = free(&repo, &wt, Activity::default());
        let _ = child.kill();
        let _ = child.wait();
        assert!(result.is_err(), "{result:?}");
        assert!(wt.join("target/debug/big").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn reads_lsof_cwds() {
        let text = "p123\nfcwd\nn/Users/me/app\np456\nfcwd\nn/\n";
        assert_eq!(
            parse_lsof_cwds(text),
            [PathBuf::from("/Users/me/app"), PathBuf::from("/")]
        );
    }
}
