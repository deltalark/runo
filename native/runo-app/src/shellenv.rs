//! The PATH agents and Node.js live on.
//!
//! An app opened from the Dock, Finder or a desktop launcher does not run the person's
//! shell profile, so its PATH misses nvm, fnm, volta, asdf, Homebrew and npm's global
//! folder. Runo asks the login shell for its PATH once at startup and adds the folders
//! those tools use, so `codex`, `node` and `npm` are found the same way a terminal finds
//! them, and the agents Runo starts inherit the same PATH.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Where each folder on the PATH came from, for Settings.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// Already on the PATH Runo was started with.
    Inherited,
    /// From the login shell's PATH (`$SHELL -ilc env`).
    Shell,
    /// A folder a version manager or package manager uses, found on disk.
    Known(&'static str),
}

static FOUND: RwLock<Vec<(PathBuf, Source)>> = RwLock::new(Vec::new());

/// Build the full PATH and put it in this process's environment. Call before any thread
/// starts; Settings > Providers > Re-detect calls it again.
pub fn init() {
    let home = home();
    let inherited: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let shell = if cfg!(windows) || std::env::var_os("RUNO_NO_SHELL_PATH").is_some() {
        Vec::new()
    } else {
        login_shell_path(Duration::from_secs(4)).unwrap_or_default()
    };
    let known = known_dirs(&home);
    let merged = merge(&shell, &inherited, &known);
    if let Ok(joined) = std::env::join_paths(merged.iter().map(|(p, _)| p)) {
        // Single-threaded at startup; on Re-detect the change is what we want children to see.
        std::env::set_var("PATH", joined);
    }
    if let Ok(mut found) = FOUND.write() {
        *found = merged;
    }
}

/// The PATH folders and where each came from.
pub fn folders() -> Vec<(PathBuf, Source)> {
    FOUND.read().map(|f| f.clone()).unwrap_or_default()
}

/// Where a folder on the PATH came from, by the tool that uses it.
pub fn source_of(dir: &Path) -> Option<Source> {
    folders()
        .into_iter()
        .find(|(p, _)| p == dir)
        .map(|(_, source)| source)
}

/// A short label for where a program was found: "nvm", "Homebrew", "login shell".
pub fn found_via(program: &Path) -> Option<String> {
    let dir = program.parent()?;
    Some(match source_of(dir)? {
        Source::Known(tool) => tool.to_string(),
        Source::Shell => "login shell PATH".into(),
        Source::Inherited => "PATH".into(),
    })
}

/// The version managers and package managers whose folders are on the PATH, in order.
pub fn managers() -> Vec<&'static str> {
    let mut out = Vec::new();
    for (_, source) in folders() {
        if let Source::Known(tool) = source {
            if !out.contains(&tool) {
                out.push(tool);
            }
        }
    }
    out
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Login shell first (it is what the terminal uses), then the inherited PATH, then known
/// folders that exist. Each folder appears once.
fn merge(
    shell: &[PathBuf],
    inherited: &[PathBuf],
    known: &[(PathBuf, &'static str)],
) -> Vec<(PathBuf, Source)> {
    let mut out: Vec<(PathBuf, Source)> = Vec::new();
    let mut push = |p: &PathBuf, source: Source| {
        if p.as_os_str().is_empty() || out.iter().any(|(q, _)| q == p) {
            return;
        }
        out.push((p.clone(), source));
    };
    for p in shell {
        // Name the tool behind a shell folder when we know it, so Settings can say "nvm".
        let tool = known.iter().find(|(k, _)| k == p).map(|(_, t)| *t);
        let in_inherited = inherited.contains(p);
        push(
            p,
            match (tool, in_inherited) {
                (Some(t), _) => Source::Known(t),
                (None, true) => Source::Inherited,
                (None, false) => Source::Shell,
            },
        );
    }
    for p in inherited {
        let tool = known.iter().find(|(k, _)| k == p).map(|(_, t)| *t);
        push(p, tool.map(Source::Known).unwrap_or(Source::Inherited));
    }
    for (p, tool) in known {
        if p.is_dir() {
            push(p, Source::Known(tool));
        }
    }
    out
}

/// Ask the login shell for its PATH. Interactive and login, so .zshrc / .bashrc and
/// .profile both run, which is where nvm and friends put themselves.
fn login_shell_path(timeout: Duration) -> Option<Vec<PathBuf>> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".into()
            } else {
                "/bin/bash".into()
            }
        });
    let mut child = std::process::Command::new(&shell)
        .args(["-ilc", "command env"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        // Keeps oh-my-zsh and similar from prompting to update.
        .env("DISABLE_AUTO_UPDATE", "true")
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
        out
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = reader.join().ok()?;
    parse_env_path(&out)
}

/// The PATH from `env` output; the last one wins, after anything profiles printed.
fn parse_env_path(out: &str) -> Option<Vec<PathBuf>> {
    let line = out.lines().rev().find(|l| l.starts_with("PATH="))?;
    let paths: Vec<PathBuf> = std::env::split_paths(&line["PATH=".len()..])
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    (!paths.is_empty()).then_some(paths)
}

/// Version numbers of a folder name such as "v22.11.0" or "node-v20.1.0", for sorting.
fn version_key(name: &str) -> Vec<u64> {
    let start = name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(name.len());
    name[start..]
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// Subfolders of `dir`, newest version first.
fn versions(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort_by_key(|p| {
        std::cmp::Reverse(version_key(
            &p.file_name().unwrap_or_default().to_string_lossy(),
        ))
    });
    v
}

/// nvm's default version: `alias/default` names a version ("22", "v20.1.0", "lts/iron",
/// "node"); pick the newest installed one that matches.
fn nvm_dirs(nvm: &Path) -> Vec<PathBuf> {
    let installed = versions(&nvm.join("versions/node"));
    let mut alias = std::fs::read_to_string(nvm.join("alias/default"))
        .unwrap_or_default()
        .trim()
        .to_string();
    // lts/* and lts/<name> point at another alias file.
    for _ in 0..3 {
        if alias.starts_with("lts/") {
            let next = std::fs::read_to_string(nvm.join("alias").join(&alias))
                .unwrap_or_default()
                .trim()
                .to_string();
            if next.is_empty() || next == alias {
                break;
            }
            alias = next;
        }
    }
    let want = alias.trim_start_matches('v');
    let default = installed.iter().find(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        let name = name.trim_start_matches('v');
        !want.is_empty()
            && want != "node"
            && want != "stable"
            && (name == want || name.starts_with(&format!("{want}.")))
    });
    default
        .into_iter()
        .chain(installed.iter())
        .map(|p| p.join("bin"))
        .take(1)
        .collect()
}

/// Folders where Node.js version managers, Homebrew and package managers put programs.
pub fn known_dirs(home: &Path) -> Vec<(PathBuf, &'static str)> {
    let env_dir = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let mut dirs: Vec<(PathBuf, &'static str)> = Vec::new();
    let nvm = env_dir("NVM_DIR").unwrap_or_else(|| home.join(".nvm"));
    for d in nvm_dirs(&nvm) {
        dirs.push((d, "nvm"));
    }
    let fnm_roots = [
        env_dir("FNM_DIR"),
        Some(home.join(".local/share/fnm")),
        Some(home.join("Library/Application Support/fnm")),
        Some(home.join(".fnm")),
    ];
    for root in fnm_roots.into_iter().flatten() {
        dirs.push((root.join("aliases/default/bin"), "fnm"));
        if let Some(newest) = versions(&root.join("node-versions")).first() {
            dirs.push((newest.join("installation/bin"), "fnm"));
        }
    }
    dirs.push((
        env_dir("VOLTA_HOME")
            .unwrap_or_else(|| home.join(".volta"))
            .join("bin"),
        "Volta",
    ));
    dirs.push((
        env_dir("ASDF_DATA_DIR")
            .unwrap_or_else(|| home.join(".asdf"))
            .join("shims"),
        "asdf",
    ));
    dirs.push((home.join(".local/share/mise/shims"), "mise"));
    dirs.push((home.join(".nodenv/shims"), "nodenv"));
    for brew in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/home/linuxbrew/.linuxbrew/bin",
    ] {
        dirs.push((PathBuf::from(brew), "Homebrew"));
    }
    dirs.push((home.join(".linuxbrew/bin"), "Homebrew"));
    if let Some(prefix) = env_dir("NPM_CONFIG_PREFIX").or_else(|| npmrc_prefix(home)) {
        dirs.push((prefix.join("bin"), "npm global"));
    }
    for npm in [".npm-global/bin", ".npm-packages/bin", ".npm/bin"] {
        dirs.push((home.join(npm), "npm global"));
    }
    if let Some(pnpm) = env_dir("PNPM_HOME") {
        dirs.push((pnpm, "pnpm"));
    }
    dirs.push((home.join("Library/pnpm"), "pnpm"));
    dirs.push((home.join(".local/share/pnpm"), "pnpm"));
    dirs.push((home.join(".bun/bin"), "Bun"));
    dirs.push((home.join(".yarn/bin"), "Yarn"));
    dirs.push((home.join(".local/bin"), "~/.local/bin"));
    dirs.push((home.join(".cargo/bin"), "Cargo"));
    for (dir, tool) in [
        (".opencode/bin", "OpenCode installer"),
        (".factory/bin", "Droid installer"),
        (".grok/bin", "Grok installer"),
        (".claude/local", "Claude Code installer"),
    ] {
        dirs.push((home.join(dir), tool));
    }
    if let Some(appdata) = env_dir("APPDATA") {
        dirs.push((appdata.join("npm"), "npm global"));
    }
    if let Some(symlink) = env_dir("NVM_SYMLINK") {
        dirs.push((symlink, "nvm"));
    }
    if let Some(files) = env_dir("ProgramFiles") {
        dirs.push((files.join("nodejs"), "Node.js"));
    }
    if let Some(local) = env_dir("LOCALAPPDATA") {
        dirs.push((local.join("Volta/bin"), "Volta"));
    }
    dirs
}

/// `prefix=` from ~/.npmrc, where `npm config set prefix` writes it.
fn npmrc_prefix(home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(home.join(".npmrc")).ok()?;
    text.lines().find_map(|l| {
        let (key, value) = l.split_once('=')?;
        (key.trim() == "prefix").then(|| {
            let v = value.trim().trim_matches('"');
            match v.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(v),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_path_from_env_output_after_profile_noise() {
        let out = "Welcome!\nPATH=/old\nHOME=/h\nPATH=/a/bin:/b/bin\nSHELL=/bin/zsh\n";
        assert_eq!(
            parse_env_path(out),
            Some(vec![PathBuf::from("/a/bin"), PathBuf::from("/b/bin")])
        );
        assert_eq!(parse_env_path("nothing here"), None);
    }

    #[test]
    fn merge_keeps_shell_order_and_names_tools() {
        let dir = std::env::temp_dir().join(format!("runo-merge-{}", std::process::id()));
        let nvm_bin = dir.join("nvm/bin");
        let volta = dir.join("volta/bin");
        std::fs::create_dir_all(&nvm_bin).unwrap();
        std::fs::create_dir_all(&volta).unwrap();
        let missing = dir.join("missing/bin");
        let merged = merge(
            &[nvm_bin.clone(), PathBuf::from("/usr/bin")],
            &[PathBuf::from("/usr/bin"), PathBuf::from("/bin")],
            &[
                (nvm_bin.clone(), "nvm"),
                (volta.clone(), "Volta"),
                (missing.clone(), "asdf"),
            ],
        );
        assert_eq!(
            merged,
            vec![
                (nvm_bin, Source::Known("nvm")),
                (PathBuf::from("/usr/bin"), Source::Inherited),
                (PathBuf::from("/bin"), Source::Inherited),
                (volta, Source::Known("Volta")),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nvm_prefers_the_default_alias_then_newest() {
        let dir = std::env::temp_dir().join(format!("runo-nvm-{}", std::process::id()));
        for v in ["v18.20.1", "v20.11.0", "v22.3.0"] {
            std::fs::create_dir_all(dir.join("versions/node").join(v).join("bin")).unwrap();
        }
        assert_eq!(nvm_dirs(&dir), vec![dir.join("versions/node/v22.3.0/bin")]);
        std::fs::create_dir_all(dir.join("alias/lts")).unwrap();
        std::fs::write(dir.join("alias/default"), "lts/iron").unwrap();
        std::fs::write(dir.join("alias/lts/iron"), "v20.11.0").unwrap();
        assert_eq!(nvm_dirs(&dir), vec![dir.join("versions/node/v20.11.0/bin")]);
        std::fs::write(dir.join("alias/default"), "18").unwrap();
        assert_eq!(nvm_dirs(&dir), vec![dir.join("versions/node/v18.20.1/bin")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn npmrc_prefix_expands_home() {
        let dir = std::env::temp_dir().join(format!("runo-npmrc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".npmrc"), "registry=x\nprefix=~/.npm-global\n").unwrap();
        assert_eq!(npmrc_prefix(&dir), Some(dir.join(".npm-global")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
