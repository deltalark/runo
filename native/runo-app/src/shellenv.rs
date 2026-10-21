//! The PATH agents and Node.js live on.
//!
//! An app opened from the Dock, Finder or a desktop launcher does not run the person's
//! shell profile, so its PATH misses nvm, fnm, volta, asdf, Homebrew and npm's global
//! folder. Runo asks the login shell for its PATH once at startup and adds the folders
//! those tools use, so `codex`, `node` and `npm` are found the same way a terminal finds
//! them, and the agents Runo starts inherit the same PATH.
//!
//! The rest of the login shell's environment (API keys, `JAVA_HOME`, `GOPATH`, proxy
//! settings…) is brought over too, without overriding anything Runo was started with
//! and leaving out what only makes sense inside that shell.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock, RwLock};
use std::time::Duration;

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

/// The login shell's variables, as it had them, for forwarding to connections.
static LOGIN_ENV: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

/// Build the full PATH and put it in this process's environment. Settings > Providers >
/// Re-detect calls it again, off the window thread.
pub fn init() {
    apply(&ask_login_shell());
    mark_ready();
}

/// At launch: the PATH Runo was started with plus the folders version managers use, at
/// once, so the window opens without waiting; the login shell is asked in the background,
/// which takes most of a second with nvm, conda or oh-my-zsh in the profile. Agents and
/// terminals wait for it (`wait_ready`), so they still start with the shell's PATH and
/// variables.
pub fn init_at_launch() {
    apply(&[]);
    if skip_shell() {
        mark_ready();
        return;
    }
    std::thread::spawn(init);
}

fn skip_shell() -> bool {
    cfg!(windows) || std::env::var_os("RUNO_NO_SHELL_PATH").is_some()
}

fn ask_login_shell() -> Vec<(String, String)> {
    if skip_shell() {
        Vec::new()
    } else {
        login_shell_env(Duration::from_secs(4)).unwrap_or_default()
    }
}

/// The PATH Runo was started with, before anything was added to it.
static INHERITED: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Put the login shell's variables (`env`, empty when not asked yet) and the merged PATH
/// in this process's environment.
fn apply(env: &[(String, String)]) {
    let home = home();
    let inherited = INHERITED.get_or_init(|| {
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default()
    });
    let shell = env_path(env).unwrap_or_default();
    for (key, value) in importable(env, |k| std::env::var_os(k).is_some()) {
        std::env::set_var(key, value);
    }
    if let Ok(mut vars) = LOGIN_ENV.write() {
        vars.clear();
        vars.extend_from_slice(env);
    }
    let known = known_dirs(&home);
    let merged = merge(&shell, inherited, &known);
    if let Ok(joined) = std::env::join_paths(merged.iter().map(|(p, _)| p)) {
        std::env::set_var("PATH", joined);
    }
    if let Ok(mut found) = FOUND.write() {
        *found = merged;
    }
}

/// Whether the login shell has been asked since launch.
static READY: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

fn mark_ready() {
    let (lock, cvar) = &READY;
    if let Ok(mut ready) = lock.lock() {
        *ready = true;
        cvar.notify_all();
    }
}

/// Whether the login shell's PATH and variables are in place.
pub fn is_ready() -> bool {
    READY.0.lock().is_ok_and(|ready| *ready)
}

/// Wait until the login shell's PATH and variables are in place, before starting an agent
/// or a terminal. Returns at once after the first second or so of launch.
pub fn wait_ready() {
    let (lock, cvar) = &READY;
    let Ok(ready) = lock.lock() else {
        return;
    };
    // The shell is killed after 4 s; this is a backstop.
    let _ = cvar.wait_timeout_while(ready, Duration::from_secs(6), |ready| !*ready);
}

/// A variable as the login shell has it, when it is set and not empty.
pub fn login_var(name: &str) -> Option<String> {
    wait_ready();
    LOGIN_ENV
        .read()
        .ok()?
        .iter()
        .rev()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
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

/// Marks where the environment starts in the shell's output, after any profile noise.
const MARKER: &str = "__RUNO_ENV__";

/// Variables that describe the shell or terminal session itself, not the person's setup.
const SKIP: &[&str] = &[
    "PATH",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "_",
    "PS1",
    "PS2",
    "PS3",
    "PS4",
    "PROMPT",
    "RPROMPT",
    "PROMPT_COMMAND",
    "TERM",
    "COLORTERM",
    "COLUMNS",
    "LINES",
    "HISTFILE",
    "HISTSIZE",
    "HISTFILESIZE",
    "HISTCONTROL",
    "SAVEHIST",
    "DISABLE_AUTO_UPDATE",
    "TMUX",
    "TMUX_PANE",
    "STY",
    "WINDOWID",
    "SSH_TTY",
    "GPG_TTY",
    "SHELL_SESSION_ID",
    "MANPATH",
    "INFOPATH",
];

/// Prefixes of shell, prompt and terminal variables left out for the same reason.
const SKIP_PREFIX: &[&str] = &[
    "BASH_FUNC_",
    "BASH_",
    "ZSH",
    "P9K",
    "_P9K",
    "POWERLEVEL9K",
    "STARSHIP_",
    "TERM_",
    "ITERM",
    "KITTY_",
    "WEZTERM_",
    "ALACRITTY_",
    "VSCODE_",
    "VTE_",
    "COMP_",
    "__",
];

/// Whether `name` is a plain environment variable name.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The login shell's variables worth bringing over: valid names, not shell-session
/// state, and not already set (`is_set`) in Runo's own environment.
pub(crate) fn importable(
    env: &[(String, String)],
    is_set: impl Fn(&str) -> bool,
) -> Vec<(&str, &str)> {
    env.iter()
        .filter(|(k, v)| {
            valid_name(k)
                && !v.contains('\0')
                && !SKIP.contains(&k.as_str())
                && !SKIP_PREFIX.iter().any(|p| k.starts_with(p))
                && !is_set(k)
        })
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// The PATH in the shell's environment.
fn env_path(env: &[(String, String)]) -> Option<Vec<PathBuf>> {
    let (_, value) = env.iter().rev().find(|(k, _)| k == "PATH")?;
    let paths: Vec<PathBuf> = std::env::split_paths(value)
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    (!paths.is_empty()).then_some(paths)
}

/// The variables in `env` output after the marker: NUL-separated when `env -0` worked,
/// else one per line. Without a marker the whole output is read.
fn parse_env(out: &str) -> Vec<(String, String)> {
    let body = match out.rfind(MARKER) {
        Some(at) => out[at + MARKER.len()..].trim_start_matches('\n'),
        None => out,
    };
    let entries: Vec<&str> = if body.contains('\0') {
        body.split('\0').collect()
    } else {
        body.lines().collect()
    };
    let mut env: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        let key = key.trim_start_matches('\n');
        if !valid_name(key) {
            continue;
        }
        env.retain(|(k, _)| k != key);
        env.push((key.to_string(), value.to_string()));
    }
    env
}

/// Ask the login shell for its environment. Interactive and login, so .zshrc / .bashrc
/// and .profile both run, which is where nvm and friends put themselves.
fn login_shell_env(timeout: Duration) -> Option<Vec<(String, String)>> {
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
        .args([
            "-ilc",
            &format!("printf '\\n{MARKER}\\n'; command env -0 2>/dev/null || command env"),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        // Keeps oh-my-zsh and similar from prompting to update.
        .env("DISABLE_AUTO_UPDATE", "true")
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
        let _ = tx.send(out);
    });
    // The output ends when the shell exits, so take it as soon as it does rather than
    // polling for the exit.
    let Ok(out) = rx.recv_timeout(timeout) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let _ = child.wait();
    let env = parse_env(&out);
    (!env.is_empty()).then_some(env)
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
            env_path(&parse_env(out)),
            Some(vec![PathBuf::from("/a/bin"), PathBuf::from("/b/bin")])
        );
        assert_eq!(env_path(&parse_env("nothing here")), None);
    }

    #[test]
    fn imports_the_login_environment_without_overriding() {
        let out = format!(
            "motd PATH=/noise\n{MARKER}\nPATH=/a/bin\0API_KEY=k=v\0NOTE=two\nlines\0\
             PWD=/x\0SHLVL=2\0BASH_FUNC_f%%=() {{ :; }}\0HOME=/h\0P9K_TTY=old\0bad-name=1\0"
        );
        let env = parse_env(&out);
        assert_eq!(env_path(&env), Some(vec![PathBuf::from("/a/bin")]));
        let picked = importable(&env, |k| k == "HOME");
        assert_eq!(picked, vec![("API_KEY", "k=v"), ("NOTE", "two\nlines")]);
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
