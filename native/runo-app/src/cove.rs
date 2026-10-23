//! Running a session inside its own Cove VM (https://github.com/deltalark/cove).
//!
//! Cove is a separate product. Runo only talks to it through the `cove` command:
//! every process a Cove session starts (the agent, the setup command, terminal tabs)
//! is wrapped as `cove exec runo-<id> --create … -- <program>`, which creates or
//! starts the VM on first use. The worktree is shared into the VM at the same path,
//! so git, diffs and commits keep running on the Mac against the same files.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Credentials passed into the VM when set in Runo's environment. Nothing else from
/// the Mac (home folder, keychain, SSH agent) reaches the VM.
const FORWARDED_ENV: [&str; 4] = [
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
];

/// The `cove` executable, looking in the usual install folders too because apps
/// started from the Dock don't get the shell's PATH.
pub fn binary() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".cargo/bin"));
        // Where Cove's install script puts it.
        dirs.push(Path::new(&home).join(".cove/bin"));
    }
    dirs.into_iter()
        .map(|d| d.join("cove"))
        .find(|p| p.is_file())
}

/// Whether this machine's `cove` takes `--auto-pause` (Cove 0.1 with pause support).
/// Asked once.
pub fn has_auto_pause() -> bool {
    static ANSWER: OnceLock<bool> = OnceLock::new();
    *ANSWER.get_or_init(|| {
        binary().is_some_and(|bin| {
            Command::new(bin)
                .args(["exec", "--help"])
                .stdin(std::process::Stdio::null())
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("--auto-pause"))
        })
    })
}

/// This computer's name, for naming the key Runo pairs with.
pub fn host_name() -> String {
    Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "this computer".into())
}

/// Cove's install script: the same one line for a Mac and for a server.
pub const INSTALL_URL: &str =
    "https://raw.githubusercontent.com/deltalark/cove/main/scripts/install.sh";

/// What to run on a Linux server or VPS to install Cove there.
pub fn server_install_command() -> String {
    format!("curl -fsSL {INSTALL_URL} | sudo sh")
}

/// The search path for Cove's installer and server when Runo was started from the
/// Dock, which gives apps almost no PATH: Homebrew (vfkit, zig) and cargo too.
fn tool_path() -> String {
    let mut dirs = vec![
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        dirs.push(home.join(".cargo/bin").display().to_string());
        dirs.push(home.join(".cove/bin").display().to_string());
    }
    if let Ok(path) = std::env::var("PATH") {
        dirs.push(path);
    }
    dirs.push("/usr/bin:/bin:/usr/sbin:/sbin".into());
    dirs.join(":")
}

/// What setting Cove up on this machine is doing: None while it runs, then whether
/// it worked, and a line to show.
pub type SetupStatus = (Option<bool>, String);

fn setup() -> &'static Mutex<Option<SetupStatus>> {
    static S: OnceLock<Mutex<Option<SetupStatus>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

pub fn setup_status() -> Option<SetupStatus> {
    setup().lock().ok()?.clone()
}

fn set_setup(status: SetupStatus) {
    if let Ok(mut s) = setup().lock() {
        *s = Some(status);
    }
}

/// Whether an install or start is running now.
pub fn setting_up() -> bool {
    matches!(setup_status(), Some((None, _)))
}

/// A line of installer output without terminal colours, for showing in Settings.
fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip an escape sequence: ESC [ ... letter.
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Install Cove on this machine with its install script, in the background. Its
/// progress shows in [`setup_status`]; `done` runs when it ends.
pub fn install(done: impl FnOnce() + Send + 'static) {
    if setting_up() {
        return;
    }
    set_setup((None, "Installing Cove…".into()));
    std::thread::spawn(move || {
        let result = run_installer();
        refresh();
        set_setup(result);
        done();
    });
}

fn run_installer() -> SetupStatus {
    use std::io::BufRead;
    let child = Command::new("/bin/sh")
        .args(["-c", &format!("curl -fsSL {INSTALL_URL} | sh")])
        .env("PATH", tool_path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (Some(false), format!("Couldn't run the installer: {e}")),
    };
    // Errors go to stderr; keep the last line of either to show.
    let last = std::sync::Arc::new(Mutex::new(String::new()));
    let err_last = last.clone();
    let stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let Some(stderr) = stderr else {
            return;
        };
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            let line = plain(&line);
            if !line.is_empty() {
                if let Ok(mut l) = err_last.lock() {
                    *l = line;
                }
            }
        }
    });
    if let Some(stdout) = child.stdout.take() {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            let line = plain(&line);
            if !line.is_empty() {
                set_setup((
                    None,
                    format!("Installing Cove: {}", line.trim_start_matches("==> ")),
                ));
                if let Ok(mut l) = last.lock() {
                    *l = line;
                }
            }
        }
    }
    let _ = errors.join();
    let ok = child.wait().is_ok_and(|s| s.success());
    let last = last.lock().map(|l| l.clone()).unwrap_or_default();
    if ok && binary().is_some() {
        (
            Some(true),
            "Installed. Cove starts by itself when you log in.".into(),
        )
    } else if last.contains("rustup") || last.contains("needs Rust") {
        (
            Some(false),
            "Cove has no release for this Mac yet, so it builds from source, which needs Rust: install it from rustup.rs, then Install again.".into(),
        )
    } else {
        (Some(false), format!("The install didn't finish: {last}"))
    }
}

/// Start this machine's Cove server: its launchd agent when the installer set one
/// up, otherwise `cove serve` on its own.
pub fn start(done: impl FnOnce() + Send + 'static) {
    if setting_up() {
        return;
    }
    set_setup((None, "Starting Cove…".into()));
    std::thread::spawn(move || {
        let result = start_server();
        refresh();
        set_setup(result);
        done();
    });
}

fn start_server() -> SetupStatus {
    const LABEL: &str = "com.deltalark.cove";
    let plist = std::env::var_os("HOME")
        .map(|h| Path::new(&h).join(format!("Library/LaunchAgents/{LABEL}.plist")));
    #[cfg(unix)]
    let uid = unsafe { libc::getuid() };
    // launchd is macOS only; elsewhere the plist never exists.
    #[cfg(not(unix))]
    let uid = 0;
    let started = if plist.as_ref().is_some_and(|p| p.is_file()) {
        let target = format!("gui/{uid}/{LABEL}");
        let kick = Command::new("launchctl")
            .args(["kickstart", "-k", &target])
            .output()
            .is_ok_and(|o| o.status.success());
        kick || plist.is_some_and(|p| {
            Command::new("launchctl")
                .args(["bootstrap", &format!("gui/{uid}")])
                .arg(&p)
                .output()
                .is_ok_and(|o| o.status.success())
        })
    } else if let Some(bin) = binary() {
        let mut cmd = Command::new(bin);
        cmd.arg("serve")
            .env("PATH", tool_path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn().is_ok()
    } else {
        false
    };
    if !started {
        return (
            Some(false),
            "Couldn't start Cove. Run cove serve in a terminal to see why.".into(),
        );
    }
    // Give the server a moment to answer.
    for _ in 0..20 {
        if answers() {
            return (Some(true), "Cove is running.".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    (
        Some(false),
        "Cove started but doesn't answer yet. Run cove doctor in a terminal to see what's missing."
            .into(),
    )
}

/// Whether this machine's Cove server answers now (not cached).
fn answers() -> bool {
    binary().is_some_and(|bin| {
        Command::new(bin)
            .args(["--json", "ls"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn availability() -> &'static Mutex<(Option<Instant>, bool)> {
    static STATE: OnceLock<Mutex<(Option<Instant>, bool)>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new((None, false)))
}

/// Ask again on the next [`available`] call, after installing or starting Cove.
pub fn refresh() {
    let ok = answers();
    if let Ok(mut g) = availability().lock() {
        *g = (Some(Instant::now()), ok);
    }
}

/// Whether Cove is installed and its server answers. Cached and refreshed in the
/// background, so it is cheap to call while rendering.
pub fn available() -> bool {
    let state = availability();
    let Ok(mut guard) = state.lock() else {
        return false;
    };
    let stale = guard
        .0
        .is_none_or(|t| t.elapsed() > Duration::from_secs(15));
    if stale {
        guard.0 = Some(Instant::now());
        std::thread::spawn(|| {
            let ok = answers();
            if let Ok(mut g) = availability().lock() {
                g.1 = ok;
            }
        });
    }
    guard.1
}

/// A VM as `cove --json ls` reports it.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Vm {
    pub name: String,
    #[serde(default)]
    pub state: serde_json::Value,
    #[serde(default)]
    pub cpus: u32,
    #[serde(default)]
    pub memory_mb: u64,
}

impl Vm {
    pub fn state(&self) -> String {
        self.state.as_str().unwrap_or("unknown").to_string()
    }
}

/// This machine's Cove VMs, refreshed in the background every few seconds.
pub fn vms() -> Vec<Vm> {
    static STATE: OnceLock<Mutex<(Option<Instant>, Vec<Vm>)>> = OnceLock::new();
    let state = STATE.get_or_init(|| Mutex::new((None, Vec::new())));
    let Ok(mut guard) = state.lock() else {
        return Vec::new();
    };
    if guard.0.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
        guard.0 = Some(Instant::now());
        std::thread::spawn(|| {
            let found: Vec<Vm> = binary()
                .and_then(|bin| {
                    Command::new(bin)
                        .args(["--json", "ls"])
                        .stdin(std::process::Stdio::null())
                        .output()
                        .ok()
                })
                .filter(|o| o.status.success())
                .and_then(|o| serde_json::from_slice(&o.stdout).ok())
                .unwrap_or_default();
            if let Some(state) = STATE.get() {
                if let Ok(mut g) = state.lock() {
                    g.1 = found;
                }
            }
        });
    }
    guard.1.clone()
}

/// `cove stop` or `cove rm` a VM, in the background.
pub fn vm_action(action: &'static str, name: &str) {
    let Some(bin) = binary() else { return };
    let name = name.to_string();
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args([action, &name])
            .stdin(std::process::Stdio::null())
            .output();
    });
}

/// Agents a Cove server keeps the login of (`cove agents`): signed in once there, every
/// VM on it is signed in, and tokens the agent refreshes come back to the server.
const SERVER_LOGINS: [&str; 5] = ["claude", "codex", "gemini", "opencode", "qwen"];

pub fn keeps_login(provider: &str) -> bool {
    SERVER_LOGINS.contains(&provider)
}

/// The VM Runo signs agents in from, on a Cove server.
const SIGN_IN_VM: &str = "runo-signin";

/// An agent's login as a Cove server keeps it (`cove --json agents ls`).
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AgentLogin {
    pub agent: String,
    pub signed_in: bool,
}

type LoginCache = std::collections::HashMap<u64, (Instant, Result<Vec<AgentLogin>, String>)>;

fn login_cache() -> &'static Mutex<LoginCache> {
    static CACHE: OnceLock<Mutex<LoginCache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Ask the server which agents are signed in there, now.
fn fetch_logins(conn: &crate::connections::Connection) -> Result<Vec<AgentLogin>, String> {
    let out = conn
        .cove()?
        .args(["--json", "agents", "ls"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run cove: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("unrecognized subcommand") {
            return Err(format!(
                "This Mac's Cove is too old to sign agents in on a server. Update it with: curl -fsSL {INSTALL_URL} | sh"
            ));
        }
        if err.contains("404") || err.to_lowercase().contains("not found") {
            return Err(format!(
                "This Cove server is too old to keep agent logins. Update it with: {}",
                server_install_command()
            ));
        }
        return Err(err
            .lines()
            .last()
            .unwrap_or("cove failed")
            .trim()
            .to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cove said something unexpected: {e}"))
}

/// The agents signed in on a Cove connection's server, refreshed in the background every
/// 30 seconds. None until the first answer is in.
pub fn logins(conn: &crate::connections::Connection) -> Option<Result<Vec<AgentLogin>, String>> {
    let Ok(mut cache) = login_cache().lock() else {
        return None;
    };
    let stale = cache
        .get(&conn.id)
        .is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(30));
    let known = cache.get(&conn.id).map(|(_, r)| r.clone());
    if stale {
        // Asked again in 30 seconds at the earliest, whatever this answer is.
        let at = Instant::now();
        let held = known.clone().unwrap_or_else(|| Ok(Vec::new()));
        cache.insert(conn.id, (at, held));
        let conn = conn.clone();
        std::thread::spawn(move || {
            let answer = fetch_logins(&conn);
            if let Ok(mut cache) = login_cache().lock() {
                cache.insert(conn.id, (at, answer));
            }
            crate::caps::mark_updated();
        });
    }
    known
}

/// Ask the server now, off the UI thread (as Test does), and keep the answer.
pub fn logins_now(conn: &crate::connections::Connection) -> Result<Vec<AgentLogin>, String> {
    let answer = fetch_logins(conn);
    if let Ok(mut cache) = login_cache().lock() {
        cache.insert(conn.id, (Instant::now(), answer.clone()));
    }
    answer
}

/// Ask every server again on the next [`logins`] call, after signing in or out.
pub fn forget_logins() {
    if let Ok(mut cache) = login_cache().lock() {
        cache.clear();
    }
}

/// Whether `provider` is signed in on the connection's server, once known.
pub fn signed_in_there(conn: &crate::connections::Connection, provider: &str) -> Option<bool> {
    let logins = logins(conn)?.ok()?;
    logins
        .iter()
        .find(|l| l.agent == provider)
        .map(|l| l.signed_in)
}

/// The command that signs `provider` in on a Cove connection's server: the agent's own
/// login in a terminal in a small VM there, after which the server keeps the login for
/// every VM.
pub fn sign_in_command(
    conn: &crate::connections::Connection,
    provider: &str,
) -> Result<Command, String> {
    if !keeps_login(provider) {
        return Err(format!(
            "Cove can't keep {}'s login yet. Pass its API key to the connection under Keys passed in.",
            crate::agent::provider(provider).name
        ));
    }
    let mut c = conn.cove()?;
    c.args(["agents", "login", provider, "--vm", SIGN_IN_VM]);
    Ok(c)
}

/// Whether this Mac has a login file for `provider` that Cove can copy to a server.
pub fn has_local_login(provider: &str) -> bool {
    keeps_login(provider)
        && crate::connections::sign_in(provider)
            .and_then(|s| s.file)
            .zip(std::env::var_os("HOME"))
            .is_some_and(|(f, home)| Path::new(&home).join(f).is_file())
}

/// Copy this Mac's login for `provider` to a Cove connection's server (`cove agents
/// import`), for every VM there.
pub fn import_login(conn: &crate::connections::Connection, provider: &str) -> Result<(), String> {
    let out = conn
        .cove()?
        .args(["agents", "import", provider])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run cove: {e}"))?;
    forget_logins();
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .trim()
            .trim_start_matches("cove: ")
            .to_string())
    }
}

/// Sign `provider` out on a Cove connection's server and its VMs.
pub fn sign_out(conn: &crate::connections::Connection, provider: &str) -> Result<(), String> {
    let out = conn
        .cove()?
        .args(["agents", "logout", provider])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run cove: {e}"))?;
    forget_logins();
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// What a failed turn says when the agent isn't signed in on a Cove server.
pub fn sign_in_help(provider: &str, place: &str) -> String {
    format!(
        "{} isn't signed in on {place}. Press Sign in on {place} below: you sign in once in a terminal there, and every session on {place} uses that login.",
        crate::agent::provider(provider).name
    )
}

pub fn vm_name(session: u64) -> String {
    format!("runo-{session}")
}

/// How to run things for one Cove session.
#[derive(Clone, Debug)]
pub struct Sandbox {
    pub name: String,
    /// `cove exec` arguments that create or start the VM (size, network, idle stop).
    pub ensure: Vec<String>,
    /// The connection it belongs to, for the variables it forwards.
    pub connection: u64,
    /// `--mount` values: the session folder, and the repository's shared `.git`
    /// read-only when the folder is a worktree (so git works inside but the agent
    /// can't rewrite the main repository).
    pub mounts: Vec<String>,
}

impl Sandbox {
    pub fn for_session(session: u64, dir: &str, conn: &crate::connections::Connection) -> Sandbox {
        let mut mounts = vec![format!("{dir}:{dir}")];
        if let Some(common) = git_common_dir(dir) {
            if !Path::new(&common).starts_with(dir) {
                mounts.push(format!("{common}:{common}:ro"));
            }
        }
        // Claude Code's conversations for this folder, shared so the session can move
        // between this machine and the VM and resume the same conversation.
        if let Some(home) = std::env::var_os("HOME") {
            let name = crate::connections::claude_folder_name(dir);
            let host = Path::new(&home).join(".claude/projects").join(&name);
            if std::fs::create_dir_all(&host).is_ok() {
                mounts.push(format!("{}:/root/.claude/projects/{name}", host.display()));
            }
        }
        let name = vm_name(session);
        Sandbox {
            ensure: conn.cove_ensure_args(&name),
            connection: conn.id,
            name,
            mounts,
        }
    }

    /// The `cove exec` arguments up to (not including) the program.
    fn exec_args(&self, cwd: &str, pty: bool) -> Vec<String> {
        let mut args = if self.ensure.is_empty() {
            vec!["exec".to_string(), self.name.clone(), "--create".into()]
        } else {
            self.ensure.clone()
        };
        for m in &self.mounts {
            args.push("--mount".into());
            args.push(m.clone());
        }
        args.push("--cwd".into());
        args.push(cwd.into());
        if pty {
            args.push("-t".into());
        }
        args
    }

    /// Turn `command` (an agent or shell run on the Mac) into the same run inside the
    /// VM. Its environment travels as `-e` flags; the agent CLI is installed in the
    /// VM on first use.
    pub fn wrap(&self, command: &Command, cwd: &str, pty: bool) -> Result<Command, String> {
        let bin = binary().ok_or("Cove is not installed, so this session can't start its VM.")?;
        let program = Path::new(command.get_program())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let forwarded = crate::connections::get(self.connection)
            .map(|c| c.agent_env(&program))
            .unwrap_or_else(|| {
                FORWARDED_ENV
                    .iter()
                    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
                    .collect()
            });
        let mut args = self.exec_args(cwd, pty);
        let mut env = forwarded;
        for (k, v) in command.get_envs() {
            if let Some(v) = v {
                env.retain(|(key, _)| key.as_str() != k);
                env.push((
                    k.to_string_lossy().into_owned(),
                    v.to_string_lossy().into_owned(),
                ));
            }
        }
        // Runo's MCP server reaches the VM through a relay in the shared folder.
        let own: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let own = crate::connections::bridge_for_cove(cwd, &own, &mut env);
        for (k, v) in env {
            args.push("-e".into());
            args.push(format!("{k}={v}"));
        }
        args.extend([
            "--".into(),
            "bash".into(),
            "-lc".into(),
            launcher(&program, "in the Cove VM"),
            "runo".into(),
            program,
        ]);
        args.extend(own);
        let mut wrapped = Command::new(bin);
        wrapped.args(args);
        Ok(wrapped)
    }

    /// Arguments for an interactive login shell in the VM (a terminal tab).
    pub fn shell_args(&self, cwd: &str) -> Vec<String> {
        let mut args = self.exec_args(cwd, true);
        args.extend(["--".into(), "bash".into(), "-l".into()]);
        args
    }
}

/// Shell helpers for installers: apt with sudo when not root, curl, and npm with a
/// private Node.js when the machine has none. Nothing here needs a password.
const INSTALL_HELPERS: &str = r#"runo_apt() { command -v apt-get >/dev/null 2>&1 || return 1; if [ "$(id -u)" = 0 ]; then apt-get update -qq && apt-get install -y -qq "$@"; else sudo -n apt-get update -qq && sudo -n apt-get install -y -qq "$@"; fi; }
runo_curl() { command -v curl >/dev/null 2>&1 || runo_apt curl ca-certificates; }
runo_npm() {
  runo_curl || return 1
  if ! command -v npm >/dev/null 2>&1; then
    case "$(uname -s)" in Linux) os=linux ;; Darwin) os=darwin ;; *) return 1 ;; esac
    case "$(uname -m)" in x86_64|amd64) arch=x64 ;; aarch64|arm64) arch=arm64 ;; *) return 1 ;; esac
    base=https://nodejs.org/dist/latest-v22.x
    f="$(curl -fsSL "$base/SHASUMS256.txt" | awk '{print $2}' | grep -E "^node-v[0-9.]+-$os-$arch\.tar\.gz$" | head -n1)"
    [ -n "$f" ] || return 1
    echo "Installing Node.js for npm…"
    mkdir -p "$HOME/.local/runo-node" && curl -fsSL "$base/$f" | tar -xzf - -C "$HOME/.local/runo-node" --strip-components=1 || return 1
  fi
  npm install -g --prefix "$HOME/.local" "$1@${want:-latest}"
}"#;

/// How to install an agent CLI on a Linux or macOS machine, by its program name: the
/// vendor's script, or its npm package (into ~/.local, so no sudo is needed).
fn install_command(program: &str) -> Option<String> {
    let provider = crate::agent::PROVIDERS
        .iter()
        .find(|p| p.bin == program || (p.id == "cursor" && program == "agent"))?;
    Some(match provider.id {
        "claude" => {
            r#"runo_curl && curl -fsSL https://claude.ai/install.sh | bash -s "${want:-latest}""#
                .to_string()
        }
        id => match crate::alerts::npm_package(id) {
            Some(package) => format!("runo_npm {package}"),
            None => format!(
                "runo_curl && {}",
                crate::registry::spec(id)?.install.unix.as_deref()?
            ),
        },
    })
}

/// Whether the install command above can install an exact version, so the copy there
/// can match this computer's.
pub(crate) fn pins_version(provider: &str) -> bool {
    matches!(
        provider,
        "claude" | "codex" | "opencode" | "gemini" | "copilot" | "qwen"
    )
}

/// A `bash -lc` script that runs the agent CLI given as `$1` with the remaining
/// arguments. First it installs the CLI if it is missing, or updates it once when
/// `RUNO_AGENT_VERSION` asks for another version, and writes the login file passed in
/// `RUNO_AUTH_PATH` / `RUNO_AUTH_DATA` if the machine has none yet. `place` reads like
/// "in the Cove VM" or "on Build box".
pub(crate) fn launcher(program: &str, place: &str) -> String {
    let Some(install) = install_command(program) else {
        return r#"exec "$@""#.to_string();
    };
    format!(
        r#"export PATH="$HOME/.local/bin:$HOME/.local/runo-node/bin:$HOME/.opencode/bin:$HOME/.factory/bin:$HOME/.grok/bin:$PATH"
{INSTALL_HELPERS}
if [ -n "$RUNO_AUTH_PATH" ] && [ ! -s "$HOME/$RUNO_AUTH_PATH" ]; then
  mkdir -p "$(dirname "$HOME/$RUNO_AUTH_PATH")" && (umask 077; printf '%s' "$RUNO_AUTH_DATA" | base64 --decode > "$HOME/$RUNO_AUTH_PATH")
fi
unset RUNO_AUTH_PATH RUNO_AUTH_DATA
want="$RUNO_AGENT_VERSION"; unset RUNO_AGENT_VERSION
if command -v "$1" >/dev/null 2>&1; then
  have=""; [ -z "$want" ] || have="$("$1" --version 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -n1)"
  tried="$HOME/.cache/runo/$1-$want"
  if [ -z "$want" ] || [ "$have" = "$want" ] || [ -e "$tried" ]; then exec "$@"; fi
  mkdir -p "$HOME/.cache/runo" && touch "$tried"
  echo "Updating $1 {place} to $want to match this computer…" >&2
  {{ {install}; }} >&2 || echo "Couldn't update $1 {place}; using version $have." >&2
  exec "$@"
fi
echo "Installing $1 {place}…" >&2
{{ {install}; }} >&2 && command -v "$1" >/dev/null 2>&1 || {{ echo "Couldn't install $1 {place}. Install it there yourself, then send the message again." >&2; exit 127; }}
exec "$@""#
    )
}

/// The repository's shared git folder for a worktree, as an absolute path.
fn git_common_dir(dir: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn installer_lines_lose_their_colours() {
        assert_eq!(
            plain("\u{1b}[1;34m==> Installing Cove\u{1b}[0m\r"),
            "==> Installing Cove"
        );
        assert_eq!(plain("  plain  "), "plain");
        assert!(server_install_command().ends_with("install.sh | sudo sh"));
    }

    use super::*;

    fn sandbox() -> Sandbox {
        Sandbox {
            name: "runo-7".into(),
            ensure: Vec::new(),
            connection: 1,
            mounts: vec![
                "/code/.runo-worktrees/app/x:/code/.runo-worktrees/app/x".into(),
                "/code/app/.git:/code/app/.git:ro".into(),
            ],
        }
    }

    #[test]
    fn agents_sign_in_on_the_server() {
        let mut conn = crate::connections::Connection::new_cove(1);
        conn.url = "http://127.0.0.1:17457".into();
        conn.api_key = "cove_k".into();
        if binary().is_some() {
            let c = sign_in_command(&conn, "claude").unwrap();
            let args: Vec<String> = c
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(args.join(" "), "agents login claude --vm runo-signin");
            assert!(c
                .get_envs()
                .any(|(k, v)| k == "COVE_URL" && v.is_some_and(|v| v == "http://127.0.0.1:17457")));
        }
        assert!(sign_in_command(&conn, "cursor").is_err());
        assert!(keeps_login("codex") && !keeps_login("amp"));
        assert!(sign_in_help("claude", "VPS").contains("isn't signed in on VPS"));
    }

    #[test]
    fn shell_runs_in_the_session_folder() {
        assert_eq!(
            sandbox().shell_args("/w").join(" "),
            "exec runo-7 --create --mount /code/.runo-worktrees/app/x:/code/.runo-worktrees/app/x \
             --mount /code/app/.git:/code/app/.git:ro --cwd /w -t -- bash -l"
        );
    }

    #[test]
    fn launcher_installs_every_agent() {
        assert!(launcher("claude", "in the Cove VM").contains("claude.ai/install.sh"));
        assert!(launcher("codex", "on Build box").contains("runo_npm @openai/codex"));
        assert!(launcher("codex", "on Build box").contains("Installing $1 on Build box"));
        for p in crate::agent::PROVIDERS.iter() {
            assert!(
                install_command(p.bin).is_some(),
                "{} has no installer",
                p.id
            );
        }
        assert_eq!(launcher("sh", "on Build box"), r#"exec "$@""#);
    }

    /// The launcher against a fake agent: it writes the login file once, runs the CLI,
    /// and asks for an update only when the versions differ.
    #[test]
    fn launcher_writes_the_login_file_and_runs_the_agent() {
        let home = std::env::temp_dir().join(format!("runo-launch-{}", std::process::id()));
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("codex");
        std::fs::write(&fake, "#!/bin/sh\n[ \"$1\" = --version ] && echo 'codex-cli 1.2.3' && exit 0\necho \"ran $*\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let run = || {
            Command::new("bash")
                .args([
                    "-c",
                    &launcher("codex", "here"),
                    "runo",
                    "codex",
                    "exec",
                    "hi",
                ])
                .env("HOME", &home)
                .env("RUNO_AGENT_VERSION", "1.2.3")
                .env("RUNO_AUTH_PATH", ".codex/auth.json")
                .env("RUNO_AUTH_DATA", "eyJhIjoxfQ==")
                .output()
                .unwrap()
        };
        let out = run();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "ran exec hi\n");
        assert_eq!(
            std::fs::read_to_string(home.join(".codex/auth.json")).unwrap(),
            "{\"a\":1}"
        );
        // A login there is left alone.
        std::fs::write(home.join(".codex/auth.json"), "theirs").unwrap();
        run();
        assert_eq!(
            std::fs::read_to_string(home.join(".codex/auth.json")).unwrap(),
            "theirs"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn worktrees_get_the_shared_git_folder_read_only() {
        let root = std::env::temp_dir().join(format!("runo-cove-{}", std::process::id()));
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ],
        );
        let wt = root.join("wt");
        git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);
        let wt = wt.canonicalize().unwrap();
        let conn = crate::connections::Connection::new_cove(1);
        let s = Sandbox::for_session(3, wt.to_str().unwrap(), &conn);
        let git_dir = repo.canonicalize().unwrap().join(".git");
        assert_eq!(s.name, "runo-3");
        assert_eq!(s.mounts[1], format!("{0}:{0}:ro", git_dir.display()));
        // The project folder itself needs no extra mount.
        let own = Sandbox::for_session(4, repo.canonicalize().unwrap().to_str().unwrap(), &conn);
        assert!(!own.mounts.iter().any(|m| m.ends_with(":ro")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Runs a real command through Cove. Needs a running `cove serve` that allows the
    /// dev backend: `COVE_E2E=1 COVE_BACKEND=host cargo test -p runo cove`.
    #[test]
    fn wrapped_commands_run_in_the_vm() {
        if std::env::var("COVE_E2E").is_err() || binary().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("runo-cove-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let dir = dir.to_str().unwrap();
        let sandbox = Sandbox::for_session(
            900_000 + std::process::id() as u64,
            dir,
            &crate::connections::Connection::new_cove(1),
        );
        let mut command = Command::new("sh");
        command
            .args(["-c", "pwd; echo \"$GREETING\""])
            .env("GREETING", "hi from runo");
        let out = sandbox
            .wrap(&command, dir, false)
            .unwrap()
            .output()
            .unwrap();
        let _ = Command::new(binary().unwrap())
            .args(["rm", &sandbox.name])
            .output();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{dir}\nhi from runo\n")
        );
    }
}
