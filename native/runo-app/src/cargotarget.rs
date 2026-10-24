//! One Rust build folder for a project and its worktrees.
//!
//! Each session's worktree is a full copy of the project, so `cargo` run inside one
//! builds into `<worktree>/target`: gigabytes per session. What Runo runs in a worktree
//! (the checks after a turn, the agent, the session's terminal) gets `CARGO_TARGET_DIR`
//! set to the main checkout's `target` instead, unless the person chose a build folder
//! themselves (`CARGO_TARGET_DIR`, or `build.target-dir` in a Cargo config) or turned
//! this off in Settings > Worktrees. Cargo locks the folder, so builds take turns.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The build folder for what Runo runs in `dir`, when `dir` is inside a session's
/// worktree of a Rust project and nothing else says where builds go.
pub fn for_dir(dir: &str) -> Option<PathBuf> {
    if !crate::settings::share_cargo_target() || dir.is_empty() {
        return None;
    }
    let set = |key: &str| std::env::var_os(key).is_some_and(|v| !v.is_empty());
    choose(Path::new(dir), set, cargo_home().as_deref())
}

/// Set `CARGO_TARGET_DIR` on `command`, which runs in `dir`, unless the command already
/// sets it (an agent's own environment in Settings).
pub fn apply(command: &mut Command, dir: &str) {
    if command
        .get_envs()
        .any(|(key, _)| key == "CARGO_TARGET_DIR" || key == "CARGO_BUILD_TARGET_DIR")
    {
        return;
    }
    if let Some(target) = for_dir(dir) {
        command.env("CARGO_TARGET_DIR", target);
    }
}

/// `for_dir` without the setting: `env_set` tells whether a variable is set, `cargo_home`
/// is where the person's own Cargo config lives.
pub fn choose(
    dir: &Path,
    env_set: impl Fn(&str) -> bool,
    cargo_home: Option<&Path>,
) -> Option<PathBuf> {
    if env_set("CARGO_TARGET_DIR") || env_set("CARGO_BUILD_TARGET_DIR") {
        return None;
    }
    let (main, worktree) = checkouts(dir)?;
    let workspace = workspace_root(dir, &worktree)?;
    if configured(dir, &workspace, cargo_home) {
        return None;
    }
    let inside = workspace.strip_prefix(&worktree).ok()?;
    main.is_dir().then(|| main.join(inside).join("target"))
}

/// The main checkout and the top of the worktree `dir` is in, when it is in one of
/// Runo's worktrees: `<parent>/.runo-worktrees/<repo>/<branch…>` belongs to
/// `<parent>/<repo>` (see `git::worktree_path`).
fn checkouts(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let parts: Vec<_> = dir.components().collect();
    let at = parts
        .iter()
        .rposition(|c| c.as_os_str() == OsStr::new(".runo-worktrees"))?;
    let repo = parts.get(at + 1)?;
    let main = parts[..at].iter().collect::<PathBuf>().join(repo);
    let repo_root: PathBuf = parts[..=at + 1].iter().collect();
    // A branch can be nested folders, so the worktree's top is the folder with `.git`.
    let worktree = dir
        .ancestors()
        .take_while(|a| a.starts_with(&repo_root) && *a != repo_root)
        .find(|a| a.join(".git").exists())?
        .to_path_buf();
    Some((main, worktree))
}

/// The Cargo workspace that `cargo` run in `dir` builds: the nearest manifest with a
/// `[workspace]` at or above `dir` (within the worktree), else the nearest manifest. An
/// agent starts at the worktree's top while the crate may sit in a folder below it, so
/// with nothing above, a single folder just below with a manifest counts.
fn workspace_root(dir: &Path, worktree: &Path) -> Option<PathBuf> {
    let found: Vec<&Path> = dir
        .ancestors()
        .take_while(|a| a.starts_with(worktree))
        .filter(|a| a.join("Cargo.toml").is_file())
        .collect();
    if let Some(root) = found.iter().find(|a| is_workspace(a)).or(found.first()) {
        return Some(root.to_path_buf());
    }
    let below: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let hidden = p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'));
            !hidden && p.join("Cargo.toml").is_file()
        })
        .collect();
    match below.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

fn is_workspace(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Cargo.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .is_some_and(|t| t.contains_key("workspace"))
}

/// Whether a Cargo config cargo would read from `dir` or `workspace` sets
/// `build.target-dir`.
fn configured(dir: &Path, workspace: &Path, cargo_home: Option<&Path>) -> bool {
    let project = dir
        .ancestors()
        .chain(workspace.ancestors())
        .map(|a| a.join(".cargo"));
    project
        .chain(cargo_home.map(Path::to_path_buf))
        .flat_map(|d| [d.join("config.toml"), d.join("config")])
        .any(|file| sets_target_dir(&file))
}

fn sets_target_dir(file: &Path) -> bool {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|t| t.get("build")?.get("target-dir").cloned())
        .is_some()
}

fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join(".cargo"))
}

/// Whether `dir` holds a Cargo project.
pub fn is_rust(dir: &str) -> bool {
    Path::new(dir).join("Cargo.toml").is_file()
}

/// Parallel jobs for Runo's own Rust checks: 2 on a computer with 8 GB of memory or
/// less, where a full-width build swaps; `None` leaves Cargo's default.
pub fn check_jobs() -> Option<u32> {
    (memory_bytes()? <= 8 << 30).then_some(2)
}

#[cfg(unix)]
fn memory_bytes() -> Option<u64> {
    // SAFETY: sysconf only reads system constants.
    let (pages, size) = unsafe {
        (
            libc::sysconf(libc::_SC_PHYS_PAGES),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    };
    (pages > 0 && size > 0).then(|| pages as u64 * size as u64)
}

#[cfg(not(unix))]
fn memory_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A main checkout at `<tmp>/app` and a worktree of it on `runo/x`.
    fn layout(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("runo-cargotarget-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let main = base.join("app");
        let tree = crate::git::worktree_path(main.to_str().unwrap(), "runo/x");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: elsewhere\n").unwrap();
        (base, main, tree)
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    const PACKAGE: &str = "[package]\nname = \"a\"\n";
    const WORKSPACE: &str = "[workspace]\nmembers = [\"crates/a\"]\n";
    const UNSET: fn(&str) -> bool = |_| false;

    #[test]
    fn a_worktree_builds_into_the_main_checkout() {
        let (base, main, tree) = layout("main");
        write(&tree.join("Cargo.toml"), PACKAGE);
        assert_eq!(choose(&tree, UNSET, None), Some(main.join("target")));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_main_checkout_itself_is_left_alone() {
        let (base, main, _) = layout("self");
        write(&main.join("Cargo.toml"), PACKAGE);
        assert_eq!(choose(&main, UNSET, None), None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_project_without_cargo_is_left_alone() {
        let (base, _, tree) = layout("plain");
        write(&tree.join("package.json"), "{}");
        assert_eq!(choose(&tree, UNSET, None), None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_persons_own_target_dir_wins() {
        let (base, _, tree) = layout("env");
        write(&tree.join("Cargo.toml"), PACKAGE);
        assert_eq!(choose(&tree, |k| k == "CARGO_TARGET_DIR", None), None);
        assert_eq!(choose(&tree, |k| k == "CARGO_BUILD_TARGET_DIR", None), None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_target_dir_in_cargo_config_wins() {
        let (base, _, tree) = layout("config");
        write(&tree.join("Cargo.toml"), PACKAGE);
        write(
            &tree.join(".cargo/config.toml"),
            "[build]\ntarget-dir = \"out\"\n",
        );
        assert_eq!(choose(&tree, UNSET, None), None);
        let _ = std::fs::remove_dir_all(base);

        let (base, _, tree) = layout("home");
        write(&tree.join("Cargo.toml"), PACKAGE);
        let home = base.join("cargo-home");
        write(&home.join("config.toml"), "build.target-dir = \"/t\"\n");
        assert_eq!(choose(&tree, UNSET, Some(&home)), None);
        // Other settings in the config don't count.
        write(&home.join("config.toml"), "[build]\njobs = 2\n");
        assert!(choose(&tree, UNSET, Some(&home)).is_some());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_member_crate_builds_into_its_workspace_target() {
        let (base, main, tree) = layout("member");
        write(&tree.join("Cargo.toml"), WORKSPACE);
        write(&tree.join("crates/a/Cargo.toml"), PACKAGE);
        assert_eq!(
            choose(&tree.join("crates/a"), UNSET, None),
            Some(main.join("target"))
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_workspace_below_the_top_uses_its_own_target() {
        let (base, main, tree) = layout("below");
        write(&tree.join("native/Cargo.toml"), WORKSPACE);
        write(&tree.join("native/crates/a/Cargo.toml"), PACKAGE);
        assert_eq!(choose(&tree, UNSET, None), Some(main.join("native/target")));
        assert_eq!(
            choose(&tree.join("native/crates/a"), UNSET, None),
            Some(main.join("native/target"))
        );
        // Two crates side by side below the top: nothing to pick between them.
        write(&tree.join("other/Cargo.toml"), PACKAGE);
        assert_eq!(choose(&tree, UNSET, None), None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_folder_outside_runos_worktrees_is_left_alone() {
        let base =
            std::env::temp_dir().join(format!("runo-cargotarget-out-{}", std::process::id()));
        write(&base.join("Cargo.toml"), PACKAGE);
        assert_eq!(choose(&base, UNSET, None), None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_commands_own_target_dir_is_kept() {
        let mut command = Command::new("true");
        command.env("CARGO_TARGET_DIR", "/mine");
        apply(&mut command, "/anywhere/.runo-worktrees/app/runo/x");
        let value = command
            .get_envs()
            .find(|(k, _)| *k == "CARGO_TARGET_DIR")
            .and_then(|(_, v)| v);
        assert_eq!(value, Some(OsStr::new("/mine")));
    }
}
