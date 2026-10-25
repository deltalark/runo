//! Connections: the places a session can run besides this machine.
//!
//! A connection is a Cove server (this machine's, or a remote one by URL) or an SSH
//! server. A session picks one ("Runs on") and can move to another between turns.
//!
//! - **Local Cove** shares the session folder into the VM, so nothing needs copying.
//! - **Remote connections** (SSH, or a Cove server elsewhere) keep their own copy of the
//!   session folder. Every agent turn runs through `runo --connection-run`, which
//!   syncs the folder up with git (uncommitted changes included), runs the agent
//!   there, and syncs the result back. The agent's own conversation file travels too,
//!   so a session can move between machines and resume the same conversation.
//!
//! Git data moves over the connection itself (`ssh`, or `cove exec` through git's
//! `ext::` transport), so a VM or server never needs GitHub access just to sync.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// The session runs directly on this machine.
pub const LOCAL: u64 = 0;

/// API keys forwarded to a new connection unless the person changes the list: every
/// variable a supported agent signs in with.
pub const DEFAULT_KEYS: &str = "ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, OPENAI_API_KEY, CODEX_API_KEY, OPENROUTER_API_KEY, GEMINI_API_KEY, GOOGLE_API_KEY, CURSOR_API_KEY, COPILOT_GITHUB_TOKEN, AMP_API_KEY, FACTORY_API_KEY, DASHSCOPE_API_KEY, GROK_API_KEY, XAI_API_KEY";

/// The list connections were saved with before, which now means the full one above.
const OLD_DEFAULT_KEYS: &str =
    "ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, OPENAI_API_KEY, OPENROUTER_API_KEY";

/// How an agent signs in on a machine that doesn't share this one's home folder.
pub struct SignIn {
    pub provider: &'static str,
    /// API key variables the CLI reads.
    pub keys: &'static [&'static str],
    /// A login file the CLI keeps in the home folder and that works when copied.
    pub file: Option<&'static str>,
    /// What to do when it isn't signed in there.
    pub fix: &'static str,
}

pub const SIGN_INS: [SignIn; 11] = [
    SignIn {
        provider: "claude",
        keys: &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"],
        // Linux keeps the login here; macOS keeps it in the keychain, which can't travel.
        file: Some(".claude/.credentials.json"),
        fix: "run `claude setup-token` on this computer and export the token as CLAUDE_CODE_OAUTH_TOKEN in your shell profile, or set ANTHROPIC_API_KEY",
    },
    SignIn {
        provider: "codex",
        keys: &["OPENAI_API_KEY", "CODEX_API_KEY"],
        file: Some(".codex/auth.json"),
        fix: "run `codex login` on this computer (Runo copies ~/.codex/auth.json there), or set OPENAI_API_KEY",
    },
    SignIn {
        provider: "opencode",
        keys: &[
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "GEMINI_API_KEY",
        ],
        file: Some(".local/share/opencode/auth.json"),
        fix: "run `opencode auth login` on this computer, or set a provider key such as ANTHROPIC_API_KEY",
    },
    SignIn {
        provider: "cursor",
        keys: &["CURSOR_API_KEY"],
        file: None,
        fix: "create an API key in Cursor's dashboard and export it as CURSOR_API_KEY",
    },
    SignIn {
        provider: "gemini",
        keys: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        file: None,
        fix: "export GEMINI_API_KEY (from Google AI Studio) in your shell profile",
    },
    SignIn {
        provider: "copilot",
        keys: &["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"],
        file: None,
        fix: "turn on GitHub access for this connection, or export COPILOT_GITHUB_TOKEN with Copilot access",
    },
    SignIn {
        provider: "amp",
        keys: &["AMP_API_KEY"],
        file: None,
        fix: "export AMP_API_KEY (from ampcode.com/settings)",
    },
    SignIn {
        provider: "droid",
        keys: &["FACTORY_API_KEY"],
        file: None,
        fix: "export FACTORY_API_KEY (from app.factory.ai/settings/api-keys)",
    },
    SignIn {
        provider: "qwen",
        keys: &["DASHSCOPE_API_KEY", "OPENAI_API_KEY"],
        file: None,
        fix: "export DASHSCOPE_API_KEY, or OPENAI_API_KEY with OPENAI_BASE_URL for another service",
    },
    SignIn {
        provider: "grok",
        keys: &["GROK_API_KEY", "XAI_API_KEY"],
        file: None,
        fix: "export GROK_API_KEY (from console.x.ai)",
    },
    SignIn {
        provider: "pi",
        keys: &[
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
            "OPENROUTER_API_KEY",
        ],
        file: None,
        fix: "export a provider key such as ANTHROPIC_API_KEY or OPENROUTER_API_KEY in your shell profile",
    },
];

pub fn sign_in(provider: &str) -> Option<&'static SignIn> {
    SIGN_INS.iter().find(|s| s.provider == provider)
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Cove,
    Ssh,
    /// Tests only: "remote" commands run in a local folder.
    #[cfg(test)]
    Folder,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct Connection {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    /// Cove: the server's URL; empty for this machine's server.
    pub url: String,
    /// Cove: the API key of a remote server, from pairing (`cove pair` on that server)
    /// or `cove key` there.
    pub api_key: String,
    /// Cove: a pairing code typed in Settings, until it is traded for a key.
    #[serde(skip)]
    pub pair_code: String,
    /// Cove VM size and limits.
    pub cpus: u32,
    pub memory_mb: u64,
    /// "full" or "off".
    pub network: String,
    /// Stop an idle VM after this many minutes; 0 keeps it running.
    pub idle_stop: u32,
    /// Pause an idle VM instead of stopping it: its memory and running programs are
    /// kept, and it carries on as soon as it is used. Needs a Cove with auto-pause.
    pub idle_pause: bool,
    /// Cove: the server's SSH login (user@host). Set, Runo installs and pairs Cove over
    /// SSH and reaches it through a tunnel it keeps open itself.
    pub ssh: String,
    /// Cove over SSH: the local port of the tunnel; `url` points at it.
    pub tunnel_port: u16,
    /// SSH: a host from ~/.ssh/config, or user@host.
    pub host: String,
    /// SSH: 0 for the default port.
    pub port: u16,
    /// SSH: a private key file; empty uses the SSH agent and ~/.ssh/config.
    pub identity: String,
    /// Remote connections: the folder session copies live in, relative to the home folder.
    pub workdir: String,
    /// Environment variables passed to the agent, by name, comma separated.
    pub keys: String,
    /// Pass this machine's GitHub login (`gh auth token`) to the agent's commands.
    pub github: bool,
    /// Agents may run commands here themselves (Runo's `run_on` tool).
    pub agents: bool,
}

impl Default for Connection {
    fn default() -> Self {
        Connection {
            id: 0,
            name: String::new(),
            kind: Kind::Cove,
            url: String::new(),
            api_key: String::new(),
            pair_code: String::new(),
            cpus: 2,
            memory_mb: 4096,
            network: "full".into(),
            idle_stop: 0,
            idle_pause: true,
            ssh: String::new(),
            tunnel_port: 0,
            host: String::new(),
            port: 0,
            identity: String::new(),
            workdir: "runo-work".into(),
            keys: DEFAULT_KEYS.into(),
            github: false,
            agents: true,
        }
    }
}

/// A connection's SSH host, ready to pass to ssh: set, and not starting with `-`, which
/// ssh would read as one of its options (`-oProxyCommand=…` runs a command here).
pub(crate) fn ssh_host<'a>(host: &'a str, name: &str) -> Result<&'a str, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err(format!("{name} has no host set."));
    }
    if host.starts_with('-') || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!(
            "{name}'s host “{host}” isn't a host name: it can't start with - or contain spaces."
        ));
    }
    Ok(host)
}

impl Connection {
    pub fn new_cove(id: u64) -> Self {
        Connection {
            id,
            name: "Cove".into(),
            ..Default::default()
        }
    }

    pub fn new_ssh(id: u64) -> Self {
        Connection {
            id,
            name: "Server".into(),
            kind: Kind::Ssh,
            agents: false,
            ..Default::default()
        }
    }

    /// Keeps its own copy of the session folder, synced every turn.
    pub fn remote(&self) -> bool {
        match self.kind {
            Kind::Cove => !self.url.trim().is_empty(),
            _ => true,
        }
    }

    /// One line for menus and cards: "Cove · 2 CPU · 4 GB" or "anish@build-box".
    pub fn summary(&self) -> String {
        match self.kind {
            Kind::Cove => {
                let place = if !self.ssh.trim().is_empty() {
                    self.ssh.trim().to_string()
                } else if self.remote() {
                    self.url.trim().to_string()
                } else {
                    "Cove on this machine".into()
                };
                let mut parts = vec![
                    place,
                    format!("{} CPU", self.cpus),
                    format!("{} GB", self.memory_mb / 1024),
                ];
                if self.network == "off" {
                    parts.push("no network".into());
                }
                parts.join(" · ")
            }
            Kind::Ssh => {
                if self.host.trim().is_empty() {
                    "SSH server, no host set".into()
                } else {
                    format!("SSH · {}", self.host.trim())
                }
            }
            #[cfg(test)]
            Kind::Folder => self.host.clone(),
        }
    }

    /// Where things run, for sentences: "in the Cove VM" or "on Build box".
    pub fn place(&self) -> String {
        match self.kind {
            Kind::Cove => "in the Cove VM".into(),
            _ => format!("on {}", self.name),
        }
    }

    pub fn key_names(&self) -> Vec<String> {
        let keys = if self.keys.trim() == OLD_DEFAULT_KEYS {
            DEFAULT_KEYS
        } else {
            &self.keys
        };
        keys.split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(String::from)
            .collect()
    }

    /// The forwarded variables that are set on this machine (in Runo's environment or
    /// the login shell's), plus GitHub when allowed.
    pub fn forwarded_env(&self) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = self
            .key_names()
            .into_iter()
            .filter_map(|k| local_var(&k).map(|v| (k, v)))
            .collect();
        if self.github {
            if let Some(token) = gh_token() {
                env.extend(github_env(&token));
            }
        }
        env
    }

    /// The environment an agent CLI gets there: the forwarded keys, the version to match
    /// this computer's, and its login file when it keeps one that can travel.
    pub fn agent_env(&self, program: &str) -> Vec<(String, String)> {
        let mut env = self.forwarded_env();
        let Some(provider) = crate::agent::PROVIDERS
            .iter()
            .find(|p| p.bin == program || (p.id == "cursor" && program == "agent"))
        else {
            return env;
        };
        if crate::cove::pins_version(provider.id) {
            if let Some(v) = version_number(&crate::caps::get(provider.id).version) {
                env.push(("RUNO_AGENT_VERSION".into(), v));
            }
        }
        if let Some((path, data)) = sign_in(provider.id).and_then(login_file) {
            env.push(("RUNO_AUTH_PATH".into(), path));
            env.push(("RUNO_AUTH_DATA".into(), data));
        }
        env
    }

    /// Where a session's copy lives on a remote connection, relative to the home folder.
    pub fn remote_dir(&self, project: &str, session: u64) -> String {
        let base = self.workdir.trim().trim_matches('/');
        let base = if base.is_empty() { "runo-work" } else { base };
        format!("{base}/{}-{session}", slug(project))
    }

    /// The `cove` command for this connection, pointed at its server.
    pub(crate) fn cove(&self) -> Result<Command, String> {
        let bin = crate::cove::binary().ok_or("Cove is not installed on this machine.")?;
        crate::coveserver::ensure_tunnel(self)?;
        let mut c = Command::new(bin);
        if self.remote() {
            c.env("COVE_URL", self.url.trim());
            if !self.api_key.trim().is_empty() {
                c.env("COVE_API_KEY", self.api_key.trim());
            }
        }
        Ok(c)
    }

    /// `cove exec` arguments that create or start the VM with this connection's settings.
    pub fn cove_ensure_args(&self, vm: &str) -> Vec<String> {
        let mut args = vec![
            "exec".to_string(),
            vm.to_string(),
            "--create".into(),
            "--cpus".into(),
            self.cpus.max(1).to_string(),
            "--memory".into(),
            self.memory_mb.max(512).to_string(),
        ];
        if self.network == "off" {
            args.extend(["--network".into(), "off".into()]);
        }
        // The scratch VM agents use for one-off commands stops itself when left idle.
        let idle = if vm == scratch_vm() && self.idle_stop == 0 {
            15
        } else {
            self.idle_stop
        };
        if idle > 0 {
            // A Cove too old to pause gets only the stop timer; a newer one checks the
            // pause timer first, and a server too old for it still stops the VM.
            if self.idle_pause && crate::cove::has_auto_pause() {
                args.extend(["--auto-pause".into(), idle.to_string()]);
            }
            args.extend(["--idle-stop".into(), idle.to_string()]);
        }
        args
    }

    fn ssh_base(&self) -> Command {
        let mut c = Command::new("ssh");
        c.args(ssh_options(self));
        c
    }

    /// Run a bash script on the connection, in the home folder. Environment values
    /// travel without appearing on the remote command line. With `pty`, the script
    /// gets a terminal (stdin stays with the caller).
    pub fn shell(
        &self,
        vm: &str,
        script: &str,
        env: &[(String, String)],
        pty: bool,
    ) -> Result<(Command, Option<String>), String> {
        match self.kind {
            Kind::Cove => {
                let mut c = self.cove()?;
                c.args(self.cove_ensure_args(vm));
                c.args(["--cwd", "/root"]);
                if pty {
                    c.arg("-t");
                }
                for (k, v) in env {
                    c.args(["-e", &format!("{k}={v}")]);
                }
                c.args(["--", "bash", "-lc", script]);
                Ok((c, None))
            }
            Kind::Ssh => {
                let host = ssh_host(&self.host, &self.name)?;
                let mut c = self.ssh_base();
                if pty {
                    c.arg("-tt");
                    c.args([host, "--", &format!("bash -lc {}", quote(script))]);
                    return Ok((c, None));
                }
                c.arg("-T");
                c.args([host, "--", "bash -l -s"]);
                // The script, with its environment, goes over stdin.
                let mut body = String::new();
                for (k, v) in env {
                    body.push_str(&format!("export {k}={}\n", quote(v)));
                }
                body.push_str(script);
                body.push('\n');
                Ok((c, Some(body)))
            }
            #[cfg(test)]
            Kind::Folder => {
                let mut c = Command::new("bash");
                c.args(["-c", script]).current_dir(&self.host);
                c.env("HOME", &self.host);
                for (k, v) in env {
                    c.env(k, v);
                }
                Ok((c, None))
            }
        }
    }

    /// Run a script to completion and return its output.
    pub fn run(&self, vm: &str, script: &str, input: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let (mut command, stdin) = self.shell(vm, script, &[], false)?;
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not reach {}: {e}", self.name))?;
        if let Some(mut pipe) = child.stdin.take() {
            let mut data = stdin.unwrap_or_default().into_bytes();
            if let Some(input) = input {
                // SSH reads the script from stdin, so the input follows a heredoc marker.
                data.extend_from_slice(input);
            }
            std::thread::spawn(move || {
                let _ = pipe.write_all(&data);
            });
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() {
                format!("{} failed ({})", self.name, out.status)
            } else {
                err
            });
        }
        Ok(out.stdout)
    }

    /// Write `data` to `path` (relative to the remote home folder).
    pub fn write_file(&self, vm: &str, path: &str, data: &[u8]) -> Result<(), String> {
        let dir = Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let script = format!(
            "mkdir -p {} && base64 -d > {}",
            quote(if dir.is_empty() { "." } else { &dir }),
            quote(path)
        );
        let encoded = base64_encode(data);
        match self.kind {
            // Over SSH the script itself comes from stdin, so the data is a heredoc.
            Kind::Ssh => self
                .run(
                    vm,
                    &format!("{script} <<'RUNO_EOF'\n{encoded}\nRUNO_EOF"),
                    None,
                )
                .map(|_| ()),
            _ => self.run(vm, &script, Some(encoded.as_bytes())).map(|_| ()),
        }
    }

    /// A git URL for the session copy at `dir` (relative to the remote home).
    fn git_remote(&self, vm: &str, dir: &str) -> Result<(String, Vec<(String, String)>), String> {
        match self.kind {
            Kind::Ssh => {
                let ssh = std::iter::once("ssh".to_string())
                    .chain(ssh_options(self))
                    .map(|a| quote(&a))
                    .collect::<Vec<_>>()
                    .join(" ");
                Ok((
                    format!("{}:{dir}", ssh_host(&self.host, &self.name)?),
                    vec![("GIT_SSH_COMMAND".into(), ssh)],
                ))
            }
            Kind::Cove => {
                let bin = crate::cove::binary().ok_or("Cove is not installed on this machine.")?;
                crate::coveserver::ensure_tunnel(self)?;
                let mut env = Vec::new();
                if self.remote() {
                    env.push(("COVE_URL".into(), self.url.trim().to_string()));
                    if !self.api_key.trim().is_empty() {
                        env.push(("COVE_API_KEY".into(), self.api_key.trim().to_string()));
                    }
                }
                let mut parts = vec![bin.to_string_lossy().into_owned()];
                parts.extend(self.cove_ensure_args(vm));
                parts.extend(["--cwd".into(), "/root".into(), "--".into()]);
                let ext = parts
                    .iter()
                    .map(|p| p.replace('%', "%%").replace(' ', "% "))
                    .collect::<Vec<_>>()
                    .join(" ");
                Ok((format!("ext::{ext} %S {dir}"), env))
            }
            #[cfg(test)]
            Kind::Folder => Ok((format!("{}/{dir}", self.host), Vec::new())),
        }
    }
}

fn ssh_options(c: &Connection) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
        "-o".into(),
        "ServerAliveInterval=30".into(),
    ];
    // One login shared by the agent, terminals and syncs.
    if !cfg!(windows) {
        args.extend([
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            "ControlPath=~/.ssh/runo-%C".into(),
            "-o".into(),
            "ControlPersist=10m".into(),
        ]);
    }
    if c.port != 0 {
        args.extend(["-p".into(), c.port.to_string()]);
    }
    if !c.identity.trim().is_empty() {
        args.extend(["-i".into(), c.identity.trim().to_string()]);
    }
    args
}

/// Single-quote for a POSIX shell.
pub fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@+%".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Lowercase letters, digits and dashes.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "project".into()
    } else {
        out
    }
}

/// Claude Code keeps conversations in ~/.claude/projects/<this>/, named from the folder.
pub fn claude_folder_name(dir: &str) -> String {
    dir.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4 + data.len() / 57);
    for (i, chunk) in data.chunks(3).enumerate() {
        if i > 0 && i % 19 == 0 {
            out.push('\n');
        }
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A variable from Runo's environment, or the login shell's when Runo was opened from
/// the Dock and didn't inherit it.
fn local_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| crate::shellenv::login_var(name))
}

/// "1.2.3" out of a CLI's `--version` output, such as "codex-cli 1.2.3".
pub fn version_number(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_digit() && c != '.')
        .map(|w| w.trim_matches('.'))
        .find(|w| w.split('.').count() >= 3 && w.split('.').all(|p| !p.is_empty()))
        .map(|w| w.split('.').take(3).collect::<Vec<_>>().join("."))
}

/// An agent's login file on this machine, as (path from home, base64 content).
fn login_file(sign_in: &SignIn) -> Option<(String, String)> {
    let rel = sign_in.file?;
    let data = std::fs::read(home()?.join(rel)).ok()?;
    (!data.is_empty()).then(|| (rel.to_string(), base64_encode(&data).replace('\n', "")))
}

/// This machine's GitHub CLI token, if it is signed in.
fn gh_token() -> Option<String> {
    let out = Command::new("gh")
        .args(["auth", "token"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !token.is_empty()).then_some(token)
}

/// `gh` reads GH_TOKEN; git gets a credential helper that hands it out for github.com.
fn github_env(token: &str) -> Vec<(String, String)> {
    vec![
        ("GH_TOKEN".into(), token.into()),
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        (
            "GIT_CONFIG_KEY_0".into(),
            "credential.https://github.com.helper".into(),
        ),
        (
            "GIT_CONFIG_VALUE_0".into(),
            r#"!f() { echo username=x-access-token; echo "password=$GH_TOKEN"; }; f"#.into(),
        ),
    ]
}

// ---------- the connections Runo knows, for code without access to settings ----------

fn store() -> &'static Mutex<Vec<Connection>> {
    static ALL: OnceLock<Mutex<Vec<Connection>>> = OnceLock::new();
    ALL.get_or_init(|| Mutex::new(Vec::new()))
}

/// Called whenever settings change.
pub fn set_all(all: &[Connection]) {
    if let Ok(mut g) = store().lock() {
        *g = all.to_vec();
    }
}

pub fn get(id: u64) -> Option<Connection> {
    store().lock().ok()?.iter().find(|c| c.id == id).cloned()
}

pub fn all() -> Vec<Connection> {
    store().lock().map(|g| g.clone()).unwrap_or_default()
}

/// The name a session's "Runs on" shows.
pub fn name(id: u64) -> String {
    if id == LOCAL {
        return "this machine".into();
    }
    get(id)
        .map(|c| c.name)
        .unwrap_or_else(|| "A removed connection".into())
}

/// Trade a pairing code from `cove pair` on the server for an API key of Runo's own,
/// named after this computer. The key is returned, not saved anywhere by Cove.
pub fn pair(conn: &Connection, code: &str) -> Result<String, String> {
    let bin = crate::cove::binary().ok_or("Cove is not installed on this machine.")?;
    let url = conn.url.trim();
    if url.is_empty() {
        return Err("Pairing is for a Cove server elsewhere: fill in its URL first.".into());
    }
    let name = format!("Runo on {}", crate::cove::host_name());
    let out = Command::new(bin)
        .args([
            "--json",
            "login",
            url,
            code.trim(),
            "--name",
            &name,
            "--no-save",
        ])
        .env_remove("COVE_URL")
        .env_remove("COVE_API_KEY")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run cove: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .last()
            .unwrap_or("")
            .trim_start_matches("cove: ");
        return Err(if line.contains("--no-save") {
            "Update Cove on this machine to pair (cove login --no-save is new).".into()
        } else if line.is_empty() {
            "Pairing failed.".into()
        } else {
            line.to_string()
        });
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|_| "Cove gave an answer Runo couldn't read.".to_string())?;
    v["key"]
        .as_str()
        .filter(|k| !k.is_empty())
        .map(String::from)
        .ok_or_else(|| "The server sent no key.".into())
}

/// What the last pairing attempt per connection said (None while it runs).
pub fn pair_result(id: u64) -> Option<TestResult> {
    pairings().lock().ok()?.get(&id).cloned()
}

pub fn set_pair_result(id: u64, result: TestResult) {
    if let Ok(mut p) = pairings().lock() {
        p.insert(id, result);
    }
}

fn pairings() -> &'static Mutex<std::collections::HashMap<u64, TestResult>> {
    static P: OnceLock<Mutex<std::collections::HashMap<u64, TestResult>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// A test's outcome (None while it runs) and what it found.
type TestResult = (Option<bool>, String);

/// Results of the last "Test connection" per connection.
fn tests() -> &'static Mutex<std::collections::HashMap<u64, TestResult>> {
    static T: OnceLock<Mutex<std::collections::HashMap<u64, TestResult>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

pub fn test_result(id: u64) -> Option<TestResult> {
    tests().lock().ok()?.get(&id).cloned()
}

/// Check a connection in the background: reachable, git present, which agents are
/// installed there and whether each can sign in. Cove is asked through its API, so no VM
/// starts. `done` runs on the background thread when the result is in.
pub fn test(conn: Connection, done: impl FnOnce() + Send + 'static) {
    if let Ok(mut t) = tests().lock() {
        t.insert(conn.id, (None, "Testing…".into()));
    }
    std::thread::spawn(move || {
        let result = match conn.kind {
            Kind::Cove => test_cove(&conn),
            _ => test_server(&conn),
        };
        if let Ok(mut t) = tests().lock() {
            t.insert(conn.id, result);
        }
        done();
    });
}

fn test_cove(conn: &Connection) -> TestResult {
    let out = conn.cove().and_then(|mut c| {
        c.args(["--json", "ls"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run cove: {e}"))
    });
    match out {
        Ok(o) if o.status.success() => {
            let place = if conn.remote() {
                conn.url.trim().to_string()
            } else {
                "Cove on this machine".into()
            };
            let mut lines = vec![format!(
                "Connected to {place}. Agents install in each session's VM on first use."
            )];
            // Logins the server keeps count as signed in there.
            let kept: Vec<String> = crate::cove::logins_now(conn)
                .unwrap_or_default()
                .into_iter()
                .filter(|l| l.signed_in)
                .map(|l| l.agent)
                .collect();
            lines.extend(sign_in_lines(conn, &[], &[], &kept));
            (Some(true), lines.join("\n"))
        }
        Ok(o) => (
            Some(false),
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .last()
                .unwrap_or("cove failed")
                .to_string(),
        ),
        Err(e) => (Some(false), e),
    }
}

fn test_server(conn: &Connection) -> TestResult {
    let bins: Vec<&str> = crate::agent::PROVIDERS.iter().map(|p| p.bin).collect();
    let files: Vec<&str> = SIGN_INS.iter().filter_map(|s| s.file).collect();
    let script = format!(
        "printf 'host=%s\\n' \"$(uname -sm)\"; \
         export PATH=\"$HOME/.local/bin:$HOME/.local/runo-node/bin:$HOME/.opencode/bin:$HOME/.factory/bin:$HOME/.grok/bin:$PATH\"; \
         for t in git {}; do command -v $t >/dev/null 2>&1 && printf '%s ' $t; done; echo; \
         for f in {}; do [ -s \"$HOME/$f\" ] && printf '%s ' \"$f\"; done; echo",
        bins.join(" "),
        files.join(" ")
    );
    match conn.run("", &script, None) {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out);
            let mut lines = text.lines();
            let host = lines
                .next()
                .and_then(|l| l.strip_prefix("host="))
                .unwrap_or("")
                .to_string();
            let tools: Vec<String> = lines
                .next()
                .unwrap_or("")
                .split_whitespace()
                .map(String::from)
                .collect();
            let there: Vec<String> = lines
                .next()
                .unwrap_or("")
                .split_whitespace()
                .map(String::from)
                .collect();
            let git = if tools.iter().any(|t| t == "git") {
                "git found"
            } else {
                "git will be installed on first sync"
            };
            let mut out = vec![format!("Connected: {host}, {git}.")];
            out.extend(sign_in_lines(conn, &tools, &there, &[]));
            (Some(true), out.join("\n"))
        }
        Err(e) => (Some(false), e.lines().last().unwrap_or("").to_string()),
    }
}

/// One line per agent installed here or there: whether it is installed there and how it
/// signs in, or what to do so it can.
fn sign_in_lines(
    conn: &Connection,
    tools_there: &[String],
    files_there: &[String],
    kept_there: &[String],
) -> Vec<String> {
    let forwarded: Vec<String> = conn.forwarded_env().into_iter().map(|(k, _)| k).collect();
    let listed = conn.key_names();
    crate::agent::PROVIDERS
        .iter()
        .filter(|p| crate::caps::get(p.id).installed || tools_there.iter().any(|t| t == p.bin))
        .filter_map(|p| {
            let s = sign_in(p.id)?;
            let local_file = s
                .file
                .is_some_and(|f| home().is_some_and(|h| h.join(f).is_file()));
            let check = AgentCheck {
                installed_there: (conn.kind != Kind::Cove)
                    .then(|| tools_there.iter().any(|t| t == p.bin)),
                forwarded: &forwarded,
                set_here_not_listed: s
                    .keys
                    .iter()
                    .find(|k| local_var(k).is_some() && !listed.iter().any(|l| l == *k))
                    .copied(),
                local_file,
                file_there: s.file.is_some_and(|f| files_there.iter().any(|t| t == f)),
                kept_there: kept_there.iter().any(|k| k == p.id),
                server_can_keep: conn.kind == Kind::Cove && crate::cove::keeps_login(p.id),
            };
            Some(agent_line(p.name, s, &check))
        })
        .collect()
}

/// What Test found for one agent.
struct AgentCheck<'a> {
    /// None on Cove, where each session's VM installs it.
    installed_there: Option<bool>,
    forwarded: &'a [String],
    /// A key set on this computer that isn't in the connection's list.
    set_here_not_listed: Option<&'a str>,
    local_file: bool,
    file_there: bool,
    /// The Cove server keeps a login for it (`cove agents`).
    kept_there: bool,
    /// The Cove server could keep one, once signed in there.
    server_can_keep: bool,
}

fn agent_line(name: &str, s: &SignIn, c: &AgentCheck) -> String {
    let install = match c.installed_there {
        Some(true) => "installed, ",
        Some(false) => "installs on first use, ",
        None => "",
    };
    if let Some(key) = s.keys.iter().find(|k| c.forwarded.iter().any(|f| f == *k)) {
        return format!("✓ {name}: {install}signs in with {key}");
    }
    if c.file_there || c.kept_there {
        return format!("✓ {name}: {install}signed in there");
    }
    if c.local_file {
        return format!(
            "✓ {name}: {install}signs in with ~/{} from this computer",
            s.file.unwrap_or_default()
        );
    }
    if let Some(key) = c.set_here_not_listed {
        return format!(
            "✗ {name}: {install}not signed in there. {key} is set here; add it to Keys passed in"
        );
    }
    if c.server_can_keep {
        return format!(
            "✗ {name}: {install}not signed in there. To fix: press Sign in at {name} login below"
        );
    }
    format!("✗ {name}: {install}not signed in there. To fix: {}", s.fix)
}

/// The VM agents' one-off commands run in, on a Cove connection.
pub fn scratch_vm() -> String {
    "runo-scratch".into()
}

/// Delete what a session left on a connection: its VM, or its folder on a server.
pub fn cleanup(conn: &Connection, project: &str, session: u64) {
    let conn = conn.clone();
    let project = project.to_string();
    std::thread::spawn(move || match conn.kind {
        Kind::Cove => {
            if let Ok(mut c) = conn.cove() {
                let _ = c
                    .args(["rm", &crate::cove::vm_name(session)])
                    .stdin(Stdio::null())
                    .output();
            }
        }
        _ => {
            let dir = conn.remote_dir(&project, session);
            let _ = conn.run(
                "",
                &format!("rm -rf {} {}", quote(&dir), quote(&format!("{dir}.runo"))),
                None,
            );
        }
    });
}

/// Delete the scratch space agents' one-off commands used on a connection.
pub fn cleanup_scratch(conn: &Connection) {
    let conn = conn.clone();
    std::thread::spawn(move || match conn.kind {
        Kind::Cove => {
            if let Ok(mut c) = conn.cove() {
                let _ = c.args(["rm", &scratch_vm()]).stdin(Stdio::null()).output();
            }
        }
        _ => {
            let dir = format!("{}/scratch", conn.workdir.trim().trim_matches('/'));
            let _ = conn.run("", &format!("rm -rf {}", quote(&dir)), None);
        }
    });
}

// ---------- running a session's commands on a connection ----------

/// How to run things for one session on one connection.
#[derive(Clone, Debug)]
pub enum Sandbox {
    /// A VM on this machine's Cove that shares the session folder.
    Cove(crate::cove::Sandbox),
    /// A connection with its own copy of the session folder.
    Remote(Box<Remote>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Remote {
    pub conn: Connection,
    pub session: u64,
    pub vm: String,
    pub local_dir: String,
    pub remote_dir: String,
}

impl Sandbox {
    pub fn for_session(conn: &Connection, session: u64, project: &str, dir: &str) -> Sandbox {
        if conn.kind == Kind::Cove && !conn.remote() {
            return Sandbox::Cove(crate::cove::Sandbox::for_session(session, dir, conn));
        }
        Sandbox::Remote(Box::new(Remote {
            conn: conn.clone(),
            session,
            vm: crate::cove::vm_name(session),
            local_dir: dir.to_string(),
            remote_dir: conn.remote_dir(project, session),
        }))
    }

    /// Turn a command meant for this machine into the same run on the connection.
    pub fn wrap(&self, command: &Command, cwd: &str, pty: bool) -> Result<Command, String> {
        match self {
            Sandbox::Cove(s) => s.wrap(command, cwd, pty),
            Sandbox::Remote(r) => r.wrap(command),
        }
    }

    /// The program and arguments for a terminal tab on the connection.
    pub fn shell_program(&self, cwd: &str) -> Result<(String, Vec<String>), String> {
        match self {
            Sandbox::Cove(s) => {
                let bin = crate::cove::binary()
                    .ok_or("Cove is not installed, so this session's VM shell can't open.")?;
                Ok((bin.to_string_lossy().into_owned(), s.shell_args(cwd)))
            }
            Sandbox::Remote(r) => {
                // Runo's runner syncs the files there first and back when the shell ends.
                let runner = r.runner(String::new(), Vec::new(), Vec::new(), None)?;
                Ok((
                    runner.get_program().to_string_lossy().into_owned(),
                    runner
                        .get_args()
                        .map(|a| a.to_string_lossy().into_owned())
                        .collect(),
                ))
            }
        }
    }

    /// `command` as the same program on the connection, in a terminal. On a server
    /// Runo's runner syncs the files and `conversation` (agent, id) there first and back
    /// when the program ends.
    pub fn pty_command(
        &self,
        command: &Command,
        cwd: &str,
        conversation: Option<(String, String)>,
    ) -> Result<Command, String> {
        match self {
            Sandbox::Cove(s) => s.wrap(command, cwd, true),
            Sandbox::Remote(r) => {
                let program = Path::new(command.get_program())
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let mut env = r.conn.agent_env(&program);
                for (k, v) in command.get_envs() {
                    if let Some(v) = v {
                        let k = k.to_string_lossy().into_owned();
                        env.retain(|(key, _)| *key != k);
                        env.push((k, v.to_string_lossy().into_owned()));
                    }
                }
                let args = command
                    .get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                r.runner(program, args, env, conversation)
            }
        }
    }
}

impl Remote {
    /// `runo --connection-shell` for this session: a login shell there, or `program`.
    fn runner(
        &self,
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        conversation: Option<(String, String)>,
    ) -> Result<Command, String> {
        let file = RunFile {
            remote: self.clone(),
            program,
            args,
            env,
            conversation,
            backup: false,
            resume: false,
            untracked: false,
            stop_on_denial: false,
            approval_key: String::new(),
        };
        let path = run_file_path(self.session);
        write_private(
            &path,
            &serde_json::to_vec(&file).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("could not prepare the terminal: {e}"))?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut c = Command::new(exe);
        c.arg("--connection-shell").arg(path);
        Ok(c)
    }
}

/// What `runo --connection-run` needs, written to a private file for one run.
#[derive(Serialize, Deserialize, Debug)]
struct RunFile {
    remote: Remote,
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    /// The agent conversation to carry along: (provider, conversation id).
    conversation: Option<(String, String)>,
    /// Back each checkpoint up to the project's `origin` as `refs/runo/sessions/<id>`.
    backup: bool,
    /// Pick up a turn that kept running there while this machine was off or Runo was
    /// closed, instead of starting one.
    #[serde(default)]
    resume: bool,
    /// Back up untracked files too.
    #[serde(default)]
    untracked: bool,
    /// End the turn at the first command it isn't allowed to run, so the person can
    /// allow it now rather than find out when the turn ends.
    #[serde(default)]
    stop_on_denial: bool,
    /// The app's approval key, so answers to the agent's permission requests check out
    /// here (see `approvals`). The file is deleted as soon as it is read.
    #[serde(default)]
    approval_key: String,
}

/// Conversation ids and the backup choice come from the session, which `wrap` can't
/// see, so the caller sets them for the next run.
type NextRun = (Option<(String, String)>, bool);
static NEXT: OnceLock<Mutex<NextRun>> = OnceLock::new();

pub fn prepare_run(conversation: Option<(String, String)>, backup: bool) {
    let m = NEXT.get_or_init(|| Mutex::new((None, true)));
    if let Ok(mut g) = m.lock() {
        *g = (conversation, backup);
    }
}

impl Remote {
    fn wrap(&self, command: &Command) -> Result<Command, String> {
        let program = Path::new(command.get_program())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut env = self.conn.agent_env(&program);
        for (k, v) in command.get_envs() {
            if let Some(v) = v {
                let k = k.to_string_lossy().into_owned();
                env.retain(|(key, _)| *key != k);
                env.push((k, v.to_string_lossy().into_owned()));
            }
        }
        let (conversation, backup) = NEXT
            .get()
            .and_then(|m| m.lock().ok().map(|g| g.clone()))
            .unwrap_or((None, true));
        let file = RunFile {
            remote: self.clone(),
            program,
            args: command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
            env,
            conversation,
            backup,
            resume: false,
            untracked: crate::settings::sync_backup_untracked(),
            stop_on_denial: crate::settings::stop_on_denial(),
            approval_key: crate::approvals::export_key(),
        };
        let path = run_file_path(self.session);
        write_private(
            &path,
            &serde_json::to_vec(&file).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("could not prepare the run: {e}"))?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut c = Command::new(exe);
        c.arg("--connection-run").arg(path);
        Ok(c)
    }
}

/// The runner command that picks up a turn still running on the connection: it shows
/// the turn's output from the start and brings the work back when it ends.
pub fn resume_command(
    remote: &Remote,
    conversation: Option<(String, String)>,
    backup: bool,
) -> Result<Command, String> {
    let file = RunFile {
        remote: remote.clone(),
        program: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        conversation,
        backup,
        resume: true,
        untracked: crate::settings::sync_backup_untracked(),
        stop_on_denial: crate::settings::stop_on_denial(),
        approval_key: crate::approvals::export_key(),
    };
    let path = run_file_path(remote.session);
    write_private(
        &path,
        &serde_json::to_vec(&file).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("could not prepare the run: {e}"))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut c = Command::new(exe);
    c.arg("--connection-run").arg(path);
    Ok(c)
}

fn run_file_path(session: u64) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    crate::store::data_dir()
        .join("runs")
        .join(format!("{session}-{nonce}.json"))
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(data)
}

// ---------- sync ----------

/// Commit the whole working tree (untracked files too, unless $2 is `-u`) to the ref
/// named in $1 without touching the branch or the index, and print "<commit> <branch>".
const CHECKPOINT: &str = r#"set -e
idx="$(mktemp -u)"; trap 'rm -f "$idx"' EXIT
export GIT_INDEX_FILE="$idx"
head="$(git rev-parse -q --verify HEAD || true)"
[ -n "$head" ] && git read-tree "$head"
git add "${2:--A}" -- .
git add -f -A -- .runo/attachments 2>/dev/null || true
tree="$(git write-tree)"
export GIT_AUTHOR_NAME=Runo GIT_AUTHOR_EMAIL=runo@localhost GIT_COMMITTER_NAME=Runo GIT_COMMITTER_EMAIL=runo@localhost
if [ -n "$head" ]; then c="$(git commit-tree "$tree" -p "$head" -m 'runo sync')"; else c="$(git commit-tree "$tree" -m 'runo sync')"; fi
git update-ref "$1" "$c"
echo "$c $(git symbolic-ref -q --short HEAD || true)""#;

/// Make the folder match checkpoint $1: the branch ($2, or detached) at its parent and
/// the working tree at its content, with the changes uncommitted as they were. Images
/// attached to prompts travel in `.runo/attachments`, which git is told to ignore.
const RESTORE: &str = r#"set -e
c="$1"; b="$2"
ex="$(git rev-parse --git-path info/exclude)"; mkdir -p "$(dirname "$ex")"
grep -qxF '/.runo/' "$ex" 2>/dev/null || echo '/.runo/' >> "$ex"
p="$(git rev-parse -q --verify "$c^" || true)"
if [ -n "$p" ]; then
  if [ -n "$b" ]; then git checkout -q -f -B "$b" "$p"; else git checkout -q -f --detach "$p"; fi
fi
git clean -fdq
git read-tree -u --reset "$c"
git reset -q"#;

/// Make sure git exists and the session copy is a repository with the person's name.
fn prepare_script(dir: &str, name: &str, email: &str) -> String {
    format!(
        r#"set -e
command -v git >/dev/null 2>&1 || {{ echo "Installing git…" >&2; (apt-get update -qq && apt-get install -y -qq git) >&2 || sudo -n sh -c 'apt-get update -qq && apt-get install -y -qq git' >&2; }}
mkdir -p {dir}
cd {dir}
[ -d .git ] || git init -q
git config receive.denyCurrentBranch ignore
{name}{email}"#,
        dir = quote(dir),
        name = if name.is_empty() {
            String::new()
        } else {
            format!("git config user.name {}\n", quote(name))
        },
        email = if email.is_empty() {
            String::new()
        } else {
            format!("git config user.email {}\n", quote(email))
        },
    )
}

fn local_git(dir: &str, args: &[&str], env: &[(String, String)]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn local_sh(dir: &str, script: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn split_checkpoint(out: &str) -> Result<(String, String), String> {
    let line = out.lines().last().unwrap_or("").trim();
    let (commit, branch) = line.split_once(' ').unwrap_or((line, ""));
    if commit.len() < 40 {
        return Err(format!("unexpected checkpoint output: {out}"));
    }
    Ok((commit.to_string(), branch.trim().to_string()))
}

impl Remote {
    fn in_dir(&self, script: &str) -> String {
        format!("cd {} && {script}", quote(&self.remote_dir))
    }

    /// A ref in the local repository for this session's sync state.
    fn local_ref(&self, what: &str) -> String {
        format!("refs/runo/s{}/{what}", self.session)
    }

    fn local_checkpoint(&self) -> Result<(String, String), String> {
        split_checkpoint(&local_sh(
            &self.local_dir,
            CHECKPOINT,
            &[&self.local_ref("local")],
        )?)
    }

    /// Checkpoint the connection's copy and fetch it here. Returns (commit, branch).
    fn fetch_remote(&self) -> Result<(String, String), String> {
        let out = self.conn.run(
            &self.vm,
            &self.in_dir(&format!("sh -c {} sh refs/runo/sync", quote(CHECKPOINT))),
            None,
        )?;
        let (commit, branch) = split_checkpoint(&String::from_utf8_lossy(&out))?;
        let (url, env) = self.conn.git_remote(&self.vm, &self.remote_dir)?;
        local_git(
            &self.local_dir,
            &[
                "-c",
                "protocol.ext.allow=always",
                "fetch",
                "-q",
                "--force",
                &url,
                &format!("refs/runo/sync:{}", self.local_ref("remote")),
            ],
            &env,
        )
        .map_err(|e| format!("could not bring the session's files back: {e}"))?;
        Ok((commit, branch))
    }

    /// Same files and same commit underneath.
    fn same(&self, a: &str, b: &str) -> bool {
        let dir = &self.local_dir;
        let tree = |c: &str| local_git(dir, &["rev-parse", &format!("{c}^{{tree}}")], &[]);
        let parent =
            |c: &str| local_git(dir, &["rev-parse", "-q", "--verify", &format!("{c}^")], &[]);
        tree(a).ok() == tree(b).ok() && parent(a).ok() == parent(b).ok()
    }

    fn set_base(&self, commit: &str) {
        let _ = local_git(
            &self.local_dir,
            &["update-ref", &self.local_ref("base"), commit],
            &[],
        );
    }

    fn keep(&self, commit: &str) -> String {
        let name = self.local_ref(&format!(
            "kept-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default()
        ));
        let _ = local_git(&self.local_dir, &["update-ref", &name, commit], &[]);
        name
    }

    /// Bring the connection's copy up to date with this machine's. Returns the commit.
    /// Changes made there outside a turn (in a terminal) come here first; if both sides
    /// changed, this machine's version wins and theirs is kept under a ref.
    pub fn sync_up(&self) -> Result<String, String> {
        let dir = &self.local_dir;
        let name = local_git(dir, &["config", "user.name"], &[]).unwrap_or_default();
        let email = local_git(dir, &["config", "user.email"], &[]).unwrap_or_default();
        self.conn.run(
            &self.vm,
            &prepare_script(&self.remote_dir, &name, &email),
            None,
        )?;
        let base = local_git(
            dir,
            &["rev-parse", "-q", "--verify", &self.local_ref("base")],
            &[],
        )
        .ok();
        // A copy without the last sync's ref was made fresh (the folder or VM was wiped
        // or recreated). Its empty tree is not a change made there, so don't bring it in.
        let synced_before = self
            .conn
            .run(
                &self.vm,
                &self.in_dir("git rev-parse -q --verify refs/runo/sync >/dev/null && echo yes"),
                None,
            )
            .is_ok_and(|out| String::from_utf8_lossy(&out).trim() == "yes");
        if let Some(base) = base.filter(|b| !b.is_empty() && synced_before) {
            if let Ok((theirs, branch)) = self.fetch_remote() {
                if !self.same(&theirs, &base) {
                    let (now, _) = self.local_checkpoint()?;
                    if self.same(&now, &base) {
                        local_sh(dir, RESTORE, &[&theirs, &branch])?;
                        eprintln!("Runo: brought in changes made on {}.", self.conn.name);
                    } else {
                        let kept = self.keep(&theirs);
                        eprintln!(
                            "{WARNING}Files changed both here and on {} between turns. This computer's version was used; the files from there are kept as {kept}.",
                            self.conn.name
                        );
                    }
                }
            }
        }
        let (commit, branch) = self.local_checkpoint()?;
        let (url, env) = self.conn.git_remote(&self.vm, &self.remote_dir)?;
        local_git(
            dir,
            &[
                "-c",
                "protocol.ext.allow=always",
                "push",
                "-q",
                "--force",
                &url,
                &format!("{commit}:refs/runo/sync"),
            ],
            &env,
        )
        .map_err(|e| format!("could not send the session's files: {e}"))?;
        let restore = format!("sh -c {} sh {} {}", quote(RESTORE), commit, quote(&branch));
        self.conn.run(&self.vm, &self.in_dir(&restore), None)?;
        self.set_base(&commit);
        Ok(commit)
    }

    /// Bring this machine's copy up to date with the connection's. `sent` is the commit
    /// `sync_up` sent; if the local folder changed since, its changes are kept and the
    /// connection's version is saved under a ref instead.
    pub fn sync_down(&self, sent: &str) -> Result<SyncDown, String> {
        let (commit, branch) = self.fetch_remote()?;
        if self.same(&commit, sent) {
            return Ok(SyncDown::Unchanged);
        }
        // Did anything change here while the agent worked there?
        let (now, _) = self.local_checkpoint()?;
        if !self.same(&now, sent) {
            return Ok(SyncDown::Kept(self.keep(&commit)));
        }
        local_sh(&self.local_dir, RESTORE, &[&commit, &branch])?;
        self.set_base(&commit);
        Ok(SyncDown::Updated)
    }

    /// Push the latest checkpoint to the project's `origin` as `refs/runo/sessions/<id>`,
    /// without waiting. Untracked files stay out unless `untracked`, and public GitHub
    /// repositories are never backed up, since anyone could read the ref.
    fn backup(&self, untracked: bool) {
        let dir = self.local_dir.clone();
        let has_origin = local_git(&dir, &["remote", "get-url", "origin"], &[]).is_ok();
        if !has_origin || public_repo(&dir) {
            return;
        }
        let name = self.local_ref("backup");
        let mode = if untracked { "-A" } else { "-u" };
        let Ok((commit, _)) =
            local_sh(&dir, CHECKPOINT, &[&name, mode]).and_then(|o| split_checkpoint(&o))
        else {
            return;
        };
        let _ = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args([
                "push",
                "-q",
                "--force",
                "--no-verify",
                "origin",
                &format!("{commit}:refs/runo/sessions/{}", self.session),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    /// Copy the agent's conversation file to the connection.
    fn carry_up(&self, provider: &str, id: &str) -> Result<(), String> {
        let Some(home) = home() else { return Ok(()) };
        match provider {
            "claude" => {
                let file = home
                    .join(".claude/projects")
                    .join(claude_folder_name(&self.local_dir))
                    .join(format!("{id}.jsonl"));
                let Ok(data) = std::fs::read(&file) else {
                    return Ok(());
                };
                let remote = self.remote_claude_folder()?;
                self.conn
                    .write_file(&self.vm, &format!("{remote}/{id}.jsonl"), &data)
            }
            "codex" => {
                let Some(file) = find_named(&home.join(".codex/sessions"), id, 4) else {
                    return Ok(());
                };
                let Ok(rel) = file.strip_prefix(&home) else {
                    return Ok(());
                };
                let data = std::fs::read(&file).map_err(|e| e.to_string())?;
                self.conn
                    .write_file(&self.vm, &rel.to_string_lossy(), &data)
            }
            _ => Ok(()),
        }
    }

    /// Copy the agent's conversation file back to this machine.
    fn carry_down(&self, provider: &str, id: &str) -> Result<(), String> {
        let Some(home) = home() else { return Ok(()) };
        match provider {
            "claude" => {
                let remote = self.remote_claude_folder()?;
                let data = self.conn.run(
                    &self.vm,
                    &format!(
                        "cat {} 2>/dev/null || true",
                        quote(&format!("{remote}/{id}.jsonl"))
                    ),
                    None,
                )?;
                if data.is_empty() {
                    return Ok(());
                }
                let dir = home
                    .join(".claude/projects")
                    .join(claude_folder_name(&self.local_dir));
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                std::fs::write(dir.join(format!("{id}.jsonl")), data).map_err(|e| e.to_string())
            }
            "codex" => {
                let found = self.conn.run(
                    &self.vm,
                    &format!(
                        "f=\"$(find .codex/sessions -name {} 2>/dev/null | head -n1)\"; [ -n \"$f\" ] && echo \"$f\" && base64 < \"$f\" || true",
                        quote(&format!("*{id}*.jsonl"))
                    ),
                    None,
                )?;
                let text = String::from_utf8_lossy(&found);
                let mut lines = text.lines();
                let Some(rel) = lines.next().filter(|l| l.starts_with(".codex/")) else {
                    return Ok(());
                };
                let data = base64_decode(&lines.collect::<String>());
                let path = home.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(path, data).map_err(|e| e.to_string())
            }
            _ => Ok(()),
        }
    }

    /// ~/.claude/projects/<folder> on the connection, relative to its home.
    fn remote_claude_folder(&self) -> Result<String, String> {
        let out = self.conn.run(&self.vm, &self.in_dir("pwd"), None)?;
        let abs = String::from_utf8_lossy(&out).trim().to_string();
        Ok(format!(".claude/projects/{}", claude_folder_name(&abs)))
    }
}

// ---------- turns that keep running on the connection ----------

/// How a turn running on the connection stands.
#[derive(Debug, PartialEq)]
enum JobState {
    Running,
    Done(i32),
    /// Its process is gone without an exit code (the server or VM restarted).
    Lost,
    /// No turn was ever started there.
    Missing,
    /// Its script was put there but never ran.
    Unstarted,
}

/// Minutes a turn's script may wait to start (a VM booting, say) before it counts as
/// never started.
const START_MINUTES: u32 = 3;

impl Remote {
    /// The folder beside the session copy that holds a turn's script, output and exit
    /// code, relative to the remote home. Outside the copy, so git never sees it.
    fn job_dir(&self) -> String {
        format!("{}.runo", self.remote_dir)
    }

    /// The script that runs `program` in a terminal in the session's copy: its
    /// environment goes in a file only its owner can read, which deletes itself.
    fn pty_script(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<String, String> {
        let job = self.job_dir();
        self.conn.run(
            &self.vm,
            &format!("mkdir -p {j} && chmod 700 {j}", j = quote(&job)),
            None,
        )?;
        let mut env = env.to_vec();
        let args = self.agent_args(args, &mut env)?;
        let mut script = String::from("rm -f \"$0\"\n");
        for (k, v) in &env {
            script.push_str(&format!("export {k}={}\n", quote(v)));
        }
        script.push_str(&format!(
            "cd {} && exec bash -lc {} runo {} {}\n",
            quote(&self.remote_dir),
            quote(&launcher(program, &self.conn.place())),
            quote(program),
            args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
        ));
        let file = format!("{job}/pty.sh");
        self.conn.write_file(&self.vm, &file, script.as_bytes())?;
        self.conn
            .run(&self.vm, &format!("chmod 600 {}", quote(&file)), None)?;
        Ok(format!("bash {}", quote(&file)))
    }

    /// An agent's arguments for the connection: Runo's MCP server reaches it through a
    /// relay in the job folder (which must exist), and secrets stay off its command line.
    fn agent_args(
        &self,
        args: &[String],
        env: &mut Vec<(String, String)>,
    ) -> Result<Vec<String>, String> {
        let job = self.job_dir();
        let exe = runo_exe();
        let args = if !exe.is_empty() && args.iter().any(|a| a.contains(&exe)) {
            self.conn.write_file(
                &self.vm,
                &format!("{job}/mcp-relay.sh"),
                MCP_RELAY.as_bytes(),
            )?;
            self.conn.run(
                &self.vm,
                &format!(
                    "d={}; mkdir -p \"$d/in\" \"$d/out\" && touch \"$d/alive\"",
                    quote(&self.mailbox())
                ),
                None,
            )?;
            let relay = vec![
                "bash".to_string(),
                "-c".into(),
                format!(
                    "exec bash \"$HOME/{job}/mcp-relay.sh\" \"$HOME/{job}/mcp\" {}",
                    relay_wait()
                ),
            ];
            bridge_mcp(args, env, &exe, &relay)
        } else {
            bridge_mcp(args, env, "\u{0}", &["bash".into()])
        };
        let base = Path::new(&self.remote_dir)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let config = format!("{job}/mcp.json");
        hide_mcp_secrets(
            &args,
            env,
            &format!("../{base}.runo/mcp.json"),
            &mut |text| {
                self.conn.write_file(&self.vm, &config, text.as_bytes())?;
                self.conn
                    .run(&self.vm, &format!("chmod 600 {}", quote(&config)), None)
                    .map(|_| ())
            },
        )
    }

    /// Start the agent detached on the connection, so it keeps going when this machine
    /// sleeps, shuts down or loses the network. Its output goes to a log there.
    fn start_job(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<(), String> {
        let job = self.job_dir();
        // A turn left over from before is stopped, so two never write the same files.
        self.conn.run(
            &self.vm,
            &format!(
                "job={}; if [ -s \"$job/pid\" ]; then kill -TERM -- -\"$(cat \"$job/pid\")\" 2>/dev/null || true; fi; rm -rf \"$job\"; mkdir -p \"$job\"; chmod 700 \"$job\"",
                quote(&job)
            ),
            None,
        )?;
        let mut env = env.to_vec();
        let args = self.agent_args(args, &mut env)?;
        let mut script = String::from("cd \"$HOME\"\n");
        // A turn stopped before it got going never starts.
        script.push_str(&format!(
            "job=\"$HOME/\"{}\n[ -e \"$job/stop\" ] && exit 0\necho $$ > \"$job/pid\"\nrm -f \"$0\"\n",
            quote(&job)
        ));
        for (k, v) in &env {
            script.push_str(&format!("export {k}={}\n", quote(v)));
        }
        script.push_str(&format!(
            "cd {} && bash -lc {} runo {} {} </dev/null >>\"$job/out.log\" 2>>\"$job/err.log\"\n",
            quote(&self.remote_dir),
            quote(&launcher(program, &self.conn.place())),
            quote(program),
            args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
        ));
        script.push_str("echo $? > \"$job/exit.tmp\" && mv \"$job/exit.tmp\" \"$job/exit\"\n");
        // The script holds API keys: only its owner can read it, and it deletes itself.
        self.conn
            .write_file(&self.vm, &format!("{job}/run.sh"), script.as_bytes())?;
        self.conn.run(
            &self.vm,
            &format!("chmod 600 {}", quote(&format!("{job}/run.sh"))),
            None,
        )?;
        match self.conn.kind {
            // Cove's named processes outlive the connection and keep an idle VM awake.
            Kind::Cove => {
                let mut c = self.conn.cove()?;
                let name = format!(
                    "runo-turn-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or_default()
                );
                let out = c
                    .args(["spawn", &self.vm, &name, "--cwd", "/root", "--", "bash"])
                    .arg(format!("{job}/run.sh"))
                    .stdin(Stdio::null())
                    .output()
                    .map_err(|e| format!("could not reach {}: {e}", self.conn.name))?;
                if !out.status.success() {
                    return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
                }
            }
            // Its own process group, so Stop reaches everything it started: setsid on
            // Linux, perl's on macOS (which has no setsid command), else job control.
            _ => {
                self.conn.run(
                    &self.vm,
                    &format!(
                        "f={f}; if command -v setsid >/dev/null 2>&1; then setsid nohup bash \"$f\" </dev/null >/dev/null 2>&1 & \
                         elif command -v perl >/dev/null 2>&1; then nohup perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV or exit 127' bash \"$f\" </dev/null >/dev/null 2>&1 & \
                         else set -m; nohup bash \"$f\" </dev/null >/dev/null 2>&1 & fi",
                        f = quote(&format!("{job}/run.sh"))
                    ),
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn job_state(&self) -> Result<JobState, String> {
        let out = self.conn.run(
            &self.vm,
            &format!(
                // In the order the script moves on (run.sh goes once pid is written; exit
                // is written before the process ends), so a turn starting or ending
                // meanwhile is never taken for lost.
                "job={}; r=; [ -f \"$job/run.sh\" ] && r=1; a=; [ -s \"$job/pid\" ] && kill -0 \"$(cat \"$job/pid\")\" 2>/dev/null && a=1; \
                 if [ -f \"$job/exit\" ]; then echo \"done $(cat \"$job/exit\")\"; \
                 elif [ -n \"$a\" ]; then echo running; \
                 elif [ -n \"$r\" ] && [ -n \"$(find \"$job/run.sh\" -mmin +{START_MINUTES} 2>/dev/null)\" ]; then echo unstarted; \
                 elif [ -n \"$r\" ]; then echo running; \
                 elif [ -d \"$job\" ]; then echo lost; else echo missing; fi",
                quote(&self.job_dir())
            ),
            None,
        )?;
        let text = String::from_utf8_lossy(&out);
        let text = text.trim();
        Ok(match text.split_once(' ') {
            Some(("done", code)) => JobState::Done(code.trim().parse().unwrap_or(1)),
            _ if text == "running" => JobState::Running,
            _ if text == "unstarted" => JobState::Unstarted,
            _ if text == "lost" => JobState::Lost,
            _ => JobState::Missing,
        })
    }

    /// Show the turn's output from byte `from` on, as it is written, until the turn ends or
    /// the connection drops. Complete lines go to `line`; returns the bytes shown.
    fn follow_job(&self, from: u64, finished: bool, line: &mut dyn FnMut(&str)) -> u64 {
        let job = quote(&self.job_dir());
        let script = if finished {
            format!("tail -c +{} {job}/out.log 2>/dev/null; true", from + 1)
        } else {
            // tail's --pid is GNU only, so a loop watches the process instead.
            format!(
                "i=0; while [ ! -s {job}/pid ] && [ $i -lt 150 ]; do sleep 0.2; i=$((i+1)); done; \
                 [ -s {job}/pid ] || exit 0; p=\"$(cat {job}/pid)\"; \
                 touch {job}/out.log; tail -c +{} -f {job}/out.log 2>/dev/null & t=$!; \
                 while kill -0 \"$p\" 2>/dev/null; do sleep 0.5; done; sleep 0.5; kill $t 2>/dev/null; wait $t 2>/dev/null; exit 0",
                from + 1
            )
        };
        let Ok((mut command, stdin)) = self.conn.shell(&self.vm, &script, &[], false) else {
            return 0;
        };
        let Ok(mut child) = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return 0;
        };
        if let Some(mut pipe) = child.stdin.take() {
            let body = stdin.unwrap_or_default();
            std::thread::spawn(move || {
                let _ = pipe.write_all(body.as_bytes());
            });
        }
        let mut shown = 0;
        if let Some(out) = child.stdout.take() {
            let mut reader = BufReader::new(out);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // A line cut off by a dropped connection is shown on the next try,
                        // except at the very end, where nothing more will come.
                        if buf.last() != Some(&b'\n') && !finished {
                            break;
                        }
                        shown += n as u64;
                        let text = String::from_utf8_lossy(&buf);
                        line(text.trim_end_matches(['\n', '\r']));
                    }
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        shown
    }

    /// Stop the turn running there: its whole process group, then a hard kill.
    fn stop_job(&self) -> Result<(), String> {
        self.conn
            .run(
                &self.vm,
                &format!(
                    "job={}; [ -d \"$job\" ] || exit 0; touch \"$job/stop\"; \
                     i=0; while [ ! -s \"$job/pid\" ] && [ -f \"$job/run.sh\" ] && [ $i -lt 10 ]; do sleep 0.2; i=$((i+1)); done; \
                     [ -s \"$job/pid\" ] || {{ rm -f \"$job/run.sh\"; exit 0; }}; p=\"$(cat \"$job/pid\")\"; \
                     kill -TERM -- -\"$p\" 2>/dev/null || kill -TERM \"$p\" 2>/dev/null; \
                     i=0; while kill -0 \"$p\" 2>/dev/null && [ $i -lt 50 ]; do sleep 0.2; i=$((i+1)); done; \
                     kill -KILL -- -\"$p\" 2>/dev/null; true",
                    quote(&self.job_dir())
                ),
                None,
            )
            .map(|_| ())
    }

    /// Print the agent's own error output from the turn.
    fn job_errors(&self) -> String {
        self.conn
            .run(
                &self.vm,
                &format!("cat {}/err.log 2>/dev/null; true", quote(&self.job_dir())),
                None,
            )
            .map(|o| String::from_utf8_lossy(&o).into_owned())
            .unwrap_or_default()
    }
}

/// Set by SIGUSR1, which is how Runo asks a runner to stop its turn (SIGTERM, from a
/// shutdown or logout, leaves the turn running on the connection).
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_stop(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Set when the agent hit a command it isn't allowed to run and the turn should end there.
static DENIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn stop_requested() -> bool {
    STOP.load(std::sync::atomic::Ordering::SeqCst)
        || DENIED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Follows Claude Code's stream for a tool call refused for lack of permission, which in
/// a detached turn nobody can answer until it ends. Returns the tool's name.
#[derive(Default)]
struct DenialWatch {
    tools: std::collections::HashMap<String, String>,
}

impl DenialWatch {
    fn denied(&mut self, line: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let content = v["message"]["content"].as_array()?;
        match v["type"].as_str()? {
            "assistant" => {
                for b in content.iter().filter(|b| b["type"] == "tool_use") {
                    if let (Some(id), Some(name)) = (b["id"].as_str(), b["name"].as_str()) {
                        self.tools.insert(id.to_string(), name.to_string());
                    }
                }
                None
            }
            "user" => content.iter().find_map(|b| {
                if b["type"] != "tool_result" || b["is_error"] != true {
                    return None;
                }
                let text = match &b["content"] {
                    serde_json::Value::String(t) => t.clone(),
                    serde_json::Value::Array(parts) => parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => String::new(),
                };
                if !(text.contains("requested permissions")
                    || text.contains("haven't granted")
                    || text.contains("hasn't granted"))
                {
                    return None;
                }
                let tool = b["tool_use_id"]
                    .as_str()
                    .and_then(|id| self.tools.get(id))
                    .cloned()
                    .unwrap_or_else(|| "a tool".into());
                // Plans and questions are answered in Runo, not by allowing them.
                (!matches!(tool.as_str(), "ExitPlanMode" | "AskUserQuestion")).then_some(tool)
            }),
            _ => None,
        }
    }
}

/// Starts a runner line that the turn shows as a warning, even when it went fine.
pub const WARNING: &str = "Runo warning: ";

/// The runner's warnings among a turn's error output, without the prefix.
pub fn warnings(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        if let Some(w) = line.strip_prefix(WARNING) {
            if !out.iter().any(|o| o == w) {
                out.push(w.to_string());
            }
        }
    }
    out
}

/// The `refs/runo/s<id>/kept-…` refs a text names.
pub fn kept_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let word = word.trim_end_matches(['.', ',', ';', ')']);
        if word.starts_with("refs/runo/s")
            && word.contains("/kept-")
            && !out.iter().any(|o| o == word)
        {
            out.push(word.to_string());
        }
    }
    out
}

/// Put the files saved under a kept ref back into the session folder. What the folder
/// held is saved first, under a ref that is returned, so nothing is lost either way.
pub fn restore_kept(dir: &str, session: u64, kept: &str) -> Result<String, String> {
    if !kept.starts_with(&format!("refs/runo/s{session}/kept-")) {
        return Err("That isn't one of this session's kept versions.".into());
    }
    let commit = local_git(dir, &["rev-parse", "-q", "--verify", kept], &[]).map_err(|_| {
        "Those files are no longer kept; they may have been restored already.".to_string()
    })?;
    let saved = format!(
        "refs/runo/s{session}/replaced-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default()
    );
    let (_, branch) = split_checkpoint(&local_sh(dir, CHECKPOINT, &[&saved])?)?;
    local_sh(dir, RESTORE, &[&commit, &branch])?;
    let _ = local_git(dir, &["update-ref", "-d", kept], &[]);
    Ok(saved)
}

pub enum SyncDown {
    Unchanged,
    Updated,
    /// The local folder changed during the turn; the connection's version is kept
    /// under this ref.
    Kept(String),
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn find_named(dir: &Path, needle: &str, depth: usize) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                if let Some(found) = find_named(&path, needle, depth - 1) {
                    return Some(found);
                }
            }
        } else if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().contains(needle))
        {
            return Some(path);
        }
    }
    None
}

fn base64_decode(text: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let digits: Vec<u32> = text.bytes().filter_map(val).collect();
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, d)| n | d << (18 - 6 * i));
        for k in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * k)) as u8);
        }
    }
    out
}

/// A bash script that installs the agent CLI if missing, then runs it with "$@".
pub fn launcher(program: &str, place: &str) -> String {
    crate::cove::launcher(program, place)
}

/// `runo --connection-run <file>`: sync up, run the agent on the connection, sync back.
/// Prints the agent's output as it comes; Runo's own notes go to stderr.
pub fn connection_run(path: &str) -> i32 {
    let data = std::fs::read(path);
    let _ = std::fs::remove_file(path);
    let file: RunFile = match data
        .map_err(|e| e.to_string())
        .and_then(|d| serde_json::from_slice(&d).map_err(|e| e.to_string()))
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Runo: could not read the run: {e}");
            return 1;
        }
    };
    // Sign answers to the agent's permission requests as the app that started this does.
    crate::approvals::adopt_key(&file.approval_key);
    let r = &file.remote;
    let in_git = local_git(&r.local_dir, &["rev-parse", "--git-dir"], &[]).is_ok();
    if !in_git {
        eprintln!(
            "Runo: {} needs the project to be a git repository, so the session's files can be synced there.",
            r.conn.name
        );
        return 1;
    }
    DENIED.store(false, std::sync::atomic::Ordering::SeqCst);
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGUSR1, on_stop as *const () as libc::sighandler_t);
    }
    // One runner per session at a time: one left from before Runo closed finishes first.
    let lock = std::fs::create_dir_all(crate::store::data_dir().join("runs"))
        .and_then(|_| {
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(
                    crate::store::data_dir()
                        .join("runs")
                        .join(format!("s{}.lock", r.session)),
                )
        })
        .and_then(|f| f.lock().map(|_| f));
    let _lock = lock.ok();
    let sent = if file.resume {
        match local_git(
            &r.local_dir,
            &["rev-parse", "-q", "--verify", &r.local_ref("base")],
            &[],
        ) {
            Ok(c) if !c.is_empty() => c,
            _ => {
                eprintln!(
                    "Runo: this session has nothing to pick up on {}.",
                    r.conn.name
                );
                return 1;
            }
        }
    } else {
        let sent = match r.sync_up() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Runo: could not sync to {}: {e}", r.conn.name);
                return 1;
            }
        };
        if let Some((provider, id)) = &file.conversation {
            if let Err(e) = r.carry_up(provider, id) {
                eprintln!(
                    "{WARNING}The conversation could not be copied to {}, so the agent may start without it: {e}",
                    r.conn.name
                );
            }
        }
        if let Err(e) = r.start_job(&file.program, &file.args, &file.env) {
            eprintln!("Runo: could not start the agent on {}: {e}", r.conn.name);
            return 1;
        }
        sent
    };
    // Answer the agent's calls to Runo's MCP server while this runner follows the turn.
    let pumping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let exe = runo_exe();
    if file.resume || (!exe.is_empty() && file.args.iter().any(|a| a.contains(&exe))) {
        if file.resume {
            r.reclaim_mcp();
        }
        let r = r.clone();
        let flag = pumping.clone();
        std::thread::spawn(move || {
            while flag.load(std::sync::atomic::Ordering::SeqCst) {
                let busy = r.pump_mcp().unwrap_or(0) > 0;
                std::thread::sleep(std::time::Duration::from_millis(if busy {
                    250
                } else {
                    1000
                }));
            }
        });
    }
    let mut conversation = file.conversation.clone();
    let stdout = std::io::stdout();
    let mut watch = DenialWatch::default();
    let mut blocked: Option<String> = None;
    let mut show = |line: &str| {
        if file.stop_on_denial && blocked.is_none() {
            if let Some(tool) = watch.denied(line) {
                blocked = Some(tool);
                DENIED.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        if conversation.as_ref().is_none_or(|(_, id)| id.is_empty()) {
            if let Some(id) = conversation_id(line) {
                let provider = file
                    .conversation
                    .as_ref()
                    .map(|c| c.0.clone())
                    .unwrap_or_else(|| file.program.clone());
                conversation = Some((provider, id));
            }
        }
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "{line}");
        let _ = lock.flush();
    };
    // Follow the turn's output until it ends there, reconnecting after drops. A stop
    // request ends the turn there; the work it did so far still comes back.
    let mut shown = 0;
    let mut stopping = false;
    let mut waiting = std::time::Duration::from_secs(2);
    let mut told = false;
    let code = loop {
        if stop_requested() && !stopping {
            stopping = true;
            let _ = r.stop_job();
        }
        let watcher = {
            let r = r.clone();
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = done.clone();
            std::thread::spawn(move || {
                while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                    if stop_requested() {
                        let _ = r.stop_job();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            });
            done
        };
        shown += r.follow_job(shown, false, &mut show);
        watcher.store(true, std::sync::atomic::Ordering::SeqCst);
        stopping |= stop_requested();
        match r.job_state() {
            Ok(JobState::Done(code)) => {
                r.follow_job(shown, true, &mut show);
                break code;
            }
            Ok(JobState::Lost) => {
                r.follow_job(shown, true, &mut show);
                if !stopping {
                    eprintln!(
                        "Runo: the agent on {} ended without finishing; the server or VM may have restarted.",
                        r.conn.name
                    );
                }
                break 1;
            }
            Ok(JobState::Missing) => {
                eprintln!("Runo: the turn on {} is gone.", r.conn.name);
                break 1;
            }
            Ok(JobState::Unstarted) => {
                let _ = r.stop_job();
                eprintln!(
                    "Runo: the agent on {} never started. Check that the server or VM is running and has room, then send the message again.",
                    r.conn.name
                );
                break 1;
            }
            // The connection dropped, or this server's tail can't wait on a process.
            Ok(JobState::Running) => std::thread::sleep(std::time::Duration::from_secs(1)),
            Err(e) => {
                if stopping {
                    eprintln!(
                        "Runo: could not reach {} to stop the agent: {e}",
                        r.conn.name
                    );
                    return 1;
                }
                if !told {
                    eprintln!(
                        "Runo: lost the connection to {}; the agent keeps working there. Reconnecting…",
                        r.conn.name
                    );
                    told = true;
                }
                std::thread::sleep(waiting);
                waiting = (waiting * 2).min(std::time::Duration::from_secs(30));
                continue;
            }
        }
        waiting = std::time::Duration::from_secs(2);
    };
    pumping.store(false, std::sync::atomic::Ordering::SeqCst);
    let errors = r.job_errors();
    if !errors.trim().is_empty() {
        eprint!("{errors}");
    }
    // Stopped at a refused command: report it the way Claude Code reports refusals at the
    // end of a turn, so Allow and retry shows up.
    let code = match &blocked {
        Some(tool) => {
            let result = serde_json::json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "result": "",
                "permission_denials": [{ "tool_name": tool }],
            });
            println!("{result}");
            eprintln!(
                "{WARNING}The agent on {} wanted to use {tool}, which isn't allowed, so the turn stopped there instead of carrying on without it. Allow it below to continue.",
                r.conn.name
            );
            0
        }
        None => code,
    };
    if let Some((provider, id)) = &conversation {
        if !id.is_empty() {
            if let Err(e) = r.carry_down(provider, id) {
                eprintln!("{WARNING}The conversation could not be copied back from {}, so the next turn here may not remember this one: {e}", r.conn.name);
            }
        }
    }
    match r.sync_down(&sent) {
        Ok(SyncDown::Updated) => {
            if file.backup {
                r.backup(file.untracked);
            }
        }
        Ok(SyncDown::Kept(kept)) => eprintln!(
            "{WARNING}Files here changed while the agent worked on {}, so yours were kept and the agent's changes were not applied. They are saved as {kept}.",
            r.conn.name
        ),
        Ok(SyncDown::Unchanged) => {}
        Err(e) => eprintln!(
            "{WARNING}The agent's work could not be brought back from {}, so it is still only there. It comes back with the next turn, or open a terminal there. ({e})",
            r.conn.name
        ),
    }
    code
}

/// `runo --connection-shell <file>`: a terminal in the session's copy on a connection,
/// synced there first and back when the shell exits. With a program (an agent's own CLI,
/// a setup command) that runs instead of a login shell, the agent's conversation goes
/// along, and SIGUSR1 ends it early so its work still comes back.
pub fn connection_shell(path: &str) -> i32 {
    // The terminal may be gone by the time this has something to say.
    let note = |text: String| {
        let _ = writeln!(std::io::stderr(), "{text}");
    };
    let data = std::fs::read(path);
    let _ = std::fs::remove_file(path);
    let Ok(file) = data
        .map_err(|e| e.to_string())
        .and_then(|d| serde_json::from_slice::<RunFile>(&d).map_err(|e| e.to_string()))
    else {
        note("Runo: could not read the terminal settings.".into());
        return 1;
    };
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGUSR1, on_stop as *const () as libc::sighandler_t);
        // Runo closing the terminal ends the program there; the sync back still runs.
        libc::signal(libc::SIGHUP, on_stop as *const () as libc::sighandler_t);
    }
    let r = &file.remote;
    let in_git = local_git(&r.local_dir, &["rev-parse", "--git-dir"], &[]).is_ok();
    let sent = if in_git {
        note(format!("Syncing the session's files to {}…", r.conn.name));
        match r.sync_up() {
            Ok(c) => Some(c),
            Err(e) => {
                note(format!("Runo: could not sync to {}: {e}", r.conn.name));
                None
            }
        }
    } else {
        None
    };
    let conversation = file.conversation.clone().filter(|(_, id)| !id.is_empty());
    if let Some((provider, id)) = &conversation {
        if let Err(e) = r.carry_up(provider, id) {
            note(format!(
                "Runo: the conversation could not be copied to {}, so the CLI may start without it: {e}",
                r.conn.name
            ));
        }
    }
    let script = if file.program.is_empty() {
        format!("cd {} 2>/dev/null; exec bash -l", quote(&r.remote_dir))
    } else {
        match r.pty_script(&file.program, &file.args, &file.env) {
            Ok(script) => script,
            Err(e) => {
                note(format!(
                    "Runo: could not start {} on {}: {e}",
                    file.program, r.conn.name
                ));
                return 1;
            }
        }
    };
    // Answer the agent's calls to Runo's MCP server while it runs.
    let pumping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let exe = runo_exe();
    if !exe.is_empty() && file.args.iter().any(|a| a.contains(&exe)) {
        let (r, flag) = (r.clone(), pumping.clone());
        std::thread::spawn(move || {
            while flag.load(std::sync::atomic::Ordering::SeqCst) {
                let busy = r.pump_mcp().unwrap_or(0) > 0;
                std::thread::sleep(std::time::Duration::from_millis(if busy {
                    250
                } else {
                    1000
                }));
            }
        });
    }
    let code = match r.conn.shell(&r.vm, &script, &[], true) {
        Ok((mut c, _)) => match c.spawn() {
            Ok(mut child) => loop {
                if let Ok(Some(status)) = child.try_wait() {
                    break status.code().unwrap_or(1);
                }
                if stop_requested() {
                    let _ = child.kill();
                    let _ = child.wait();
                    break 130;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            },
            Err(e) => {
                note(format!("Runo: could not reach {}: {e}", r.conn.name));
                1
            }
        },
        Err(e) => {
            note(format!("Runo: {e}"));
            1
        }
    };
    pumping.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Some((provider, id)) = &conversation {
        if let Err(e) = r.carry_down(provider, id) {
            note(format!(
                "Runo: the conversation could not be copied back from {}: {e}",
                r.conn.name
            ));
        }
    }
    if let Some(sent) = sent {
        note(format!("Bringing the work back from {}…", r.conn.name));
        match r.sync_down(&sent) {
            Ok(SyncDown::Kept(kept)) => note(format!(
                "Runo: files changed here too, so they were kept; the terminal's version is saved as {kept}."
            )),
            Err(e) => note(format!("Runo: could not sync back from {}: {e}", r.conn.name)),
            _ => {}
        }
    }
    code
}

/// A conversation id in one line of an agent's JSON output.
fn conversation_id(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    ["session_id", "sessionID", "sessionId", "thread_id"]
        .iter()
        .find_map(|k| v.get(*k).and_then(|s| s.as_str()).map(String::from))
        .filter(|s| !s.is_empty())
}

// ---------- dev servers on a connection ----------

/// Lists listening TCP sockets with whichever tool the machine has.
const LISTENING: &str = "{ ss -ltnH 2>/dev/null || netstat -an 2>/dev/null || lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null; } | grep -i listen; true";

/// Listening ports in `ss -ltn`, `netstat -an` or `lsof` output: the first address in
/// each LISTEN line, written `host:port` or `host.port`.
pub fn parse_listening(text: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in text.lines().filter(|l| l.to_uppercase().contains("LISTEN")) {
        let port = line.split_whitespace().find_map(|word| {
            let (_, port) = word.rsplit_once([':', '.'])?;
            port.parse::<u16>().ok().filter(|p| *p > 0)
        });
        if let Some(port) = port.filter(|p| !ports.contains(p)) {
            ports.push(port);
        }
    }
    ports
}

/// A port on a connection reachable from this machine, as long as this lives.
pub struct Forward {
    /// The port on this machine.
    pub local: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ssh: Option<std::process::Child>,
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(child) = self.ssh.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A free port on this machine, the same number as `want` when it's free.
fn free_port(want: u16) -> Result<std::net::TcpListener, String> {
    std::net::TcpListener::bind(("127.0.0.1", want))
        .or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| format!("no free port on this machine: {e}"))
}

impl Sandbox {
    /// The listening-socket lines where the session runs; see `parse_listening`.
    pub fn listening(&self, cwd: &str) -> Result<String, String> {
        let out = match self {
            Sandbox::Cove(s) => {
                let mut c = Command::new("sh");
                c.args(["-c", LISTENING]);
                let out = s
                    .wrap(&c, cwd, false)?
                    .stdin(Stdio::null())
                    .output()
                    .map_err(|e| e.to_string())?;
                out.stdout
            }
            Sandbox::Remote(r) => r.conn.run(&r.vm, LISTENING, None)?,
        };
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Make `port` where the session runs reachable on this machine: an SSH tunnel to a
    /// server, and for Cove a local port whose connections each go through `cove exec`.
    pub fn forward(&self, cwd: &str, port: u16) -> Result<Forward, String> {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Sandbox::Remote(r) = self {
            if r.conn.kind == Kind::Ssh {
                return r.conn.ssh_forward(port);
            }
        }
        let listener = free_port(port)?;
        let local = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let (sandbox, cwd, flag) = (self.clone(), cwd.to_string(), stop.clone());
        std::thread::spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (sandbox, cwd) = (sandbox.clone(), cwd.clone());
                        std::thread::spawn(move || sandbox.pipe(stream, &cwd, port));
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
                }
            }
        });
        Ok(Forward {
            local,
            stop,
            ssh: None,
        })
    }

    /// Carry one connection to `port` there over a `cove exec` of bash's /dev/tcp.
    fn pipe(&self, mut stream: std::net::TcpStream, cwd: &str, port: u16) {
        let script =
            format!("exec 3<>/dev/tcp/127.0.0.1/{port} || exit 1; cat <&3 & cat >&3; wait");
        let command = match self {
            Sandbox::Cove(s) => {
                let mut c = Command::new("bash");
                c.args(["-c", &script]);
                s.wrap(&c, cwd, false)
            }
            Sandbox::Remote(r) => r.conn.shell(&r.vm, &script, &[], false).map(|(c, _)| c),
        };
        let Ok(mut command) = command else { return };
        let _ = stream.set_nonblocking(false);
        let Ok(mut child) = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return;
        };
        if let Ok(mut from) = stream.try_clone() {
            std::thread::spawn(move || {
                pass_bytes(&mut from, &mut stdin);
            });
        }
        pass_bytes(&mut stdout, &mut stream);
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Copy bytes as they come. `std::io::copy` from a socket into a pipe can hold on to a
/// short request, waiting for more.
fn pass_bytes(from: &mut impl Read, to: &mut impl Write) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).and_then(|_| to.flush()).is_err() {
                    break;
                }
            }
        }
    }
}

impl Connection {
    /// `ssh -L` from a local port to `port` on the server, once it's up. It runs on its
    /// own connection, so closing it ends the tunnel.
    fn ssh_forward(&self, port: u16) -> Result<Forward, String> {
        let host = ssh_host(&self.host, &self.name)?;
        let local = free_port(port)?
            .local_addr()
            .map_err(|e| e.to_string())?
            .port();
        let mut c = Command::new("ssh");
        c.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
        c.args(ssh_options(self));
        c.args([
            "-N",
            "-o",
            "ExitOnForwardFailure=yes",
            "-L",
            &format!("127.0.0.1:{local}:127.0.0.1:{port}"),
            host,
        ]);
        let mut child = c
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start ssh: {e}"))?;
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(20) {
            if let Ok(Some(_)) = child.try_wait() {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return Err(if err.trim().is_empty() {
                    format!("the tunnel to {} closed", self.name)
                } else {
                    err.trim().to_string()
                });
            }
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], local));
            if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(200))
                .is_ok()
            {
                return Ok(Forward {
                    local,
                    stop: Default::default(),
                    ssh: Some(child),
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(format!("the tunnel to {} didn't open in time", self.name))
    }
}

// ---------- Runo's MCP server for agents that run elsewhere ----------

/// A bash relay an agent starts as Runo's MCP server where Runo's own program can't run
/// (a VM, a server, another OS). Each request becomes a file in the mailbox folder given
/// as `$1`; Runo answers from this machine, where the session's memory and playbooks are.
/// When Runo hasn't looked for a minute (it is closed, or this machine is asleep), calls
/// fail fast instead of hanging the agent. Permission requests are the exception: they
/// wait up to `$2` seconds, so a Runo that reconnects in time can still answer, and are
/// refused when nobody did.
const MCP_RELAY: &str = r#"d="$1"; w="${2:-300}"; mkdir -p "$d/in" "$d/out"; n=0
m="The person hasn't granted this: nobody answered in Runo in time. Carry on without it if you can, or end your turn and say what you need allowed."
while IFS= read -r line || [ -n "$line" ]; do
  [ -n "$line" ] || continue
  n=$((n+1)); f="$$-$n"
  printf '%s\n' "$line" > "$d/in/.$f" && mv "$d/in/.$f" "$d/in/$f"
  case "$line" in *'"id":'*) ;; *) continue ;; esac
  case "$line" in *'"permission_prompt"'*) ask=1; limit=$((w*4)) ;; *) ask=; limit=1200 ;; esac
  i=0
  while [ ! -f "$d/out/$f" ]; do
    if [ -z "$ask" ] && [ $i -ge 8 ] && [ -z "$(find "$d" -maxdepth 1 -name alive -mmin -1 2>/dev/null)" ]; then break; fi
    [ $i -ge $limit ] && break
    sleep 0.25; i=$((i+1))
  done
  if [ -f "$d/out/$f" ]; then
    cat "$d/out/$f"; rm -f "$d/out/$f"
  else
    rm -f "$d/in/$f" "$d/in/.held-$f"
    id="$(printf '%s' "$line" | sed -nE 's/.*"id"[[:space:]]*:[[:space:]]*("[^"]*"|[0-9]+).*/\1/p')"
    if [ -n "$ask" ]; then
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"{\\"behavior\\":\\"deny\\",\\"message\\":\\"%s\\"}"}]}}\n' "${id:-null}" "$m"
    else
      printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32000,"message":"Runo is not connected right now, so its tools are unavailable until it is."}}\n' "${id:-null}"
    fi
  fi
done"#;

/// How long the relay holds a permission request: the time the person has to answer,
/// and a little more so Runo's own answer (even a refusal) gets there first.
pub fn relay_wait() -> String {
    (crate::approvals::wait().as_secs() + 30).to_string()
}

/// Whether an agent run in `sandbox` (None: this machine) can reach Runo's MCP server.
/// Agent argument builders give it the usual `<runo> --mcp-memory <folder>` server; on a
/// connection, `wrap` swaps that for the relay above.
pub fn mcp_reachable(sandbox: Option<&Sandbox>) -> bool {
    match sandbox {
        None | Some(Sandbox::Cove(_)) | Some(Sandbox::Remote(_)) => true,
    }
}

/// Answer one relayed request with Runo's own server, for session folder `dir`.
fn answer_mcp(dir: &str, request: &str) -> Option<String> {
    let request: serde_json::Value = serde_json::from_str(request.trim()).ok()?;
    crate::memory_mcp::handle(dir, &request).map(|r| r.to_string())
}

/// This program, as agent arguments name it for Runo's MCP server.
fn runo_exe() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A path that only means something on this machine.
fn local_path(command: &str) -> bool {
    command.starts_with('/') || command.starts_with('~') || command.get(1..3) == Some(":\\")
}

/// Rewrite MCP server entries inside one JSON value: Runo's own server (started from
/// `exe`) becomes `relay`, and servers started from a path on this machine are dropped.
/// Handles `{command, args}` entries and OpenCode's `{command: [..]}`.
fn bridge_json(value: &mut serde_json::Value, exe: &str, relay: &[String]) -> bool {
    use serde_json::Value;
    let Value::Object(map) = value else {
        if let Value::Array(items) = value {
            for item in items {
                bridge_json(item, exe, relay);
            }
        }
        return false;
    };
    match map.get("command").cloned() {
        Some(Value::String(c)) if c == exe => {
            map.insert("command".into(), relay[0].clone().into());
            map.insert("args".into(), relay[1..].to_vec().into());
            return false;
        }
        Some(Value::String(c)) if local_path(&c) => return true,
        Some(Value::Array(parts)) => match parts.first().and_then(Value::as_str) {
            Some(c) if c == exe => {
                map.insert("command".into(), relay.to_vec().into());
                return false;
            }
            Some(c) if local_path(c) => return true,
            _ => {}
        },
        _ => {}
    }
    let mut drop = Vec::new();
    for (k, v) in map.iter_mut() {
        if bridge_json(v, exe, relay) {
            drop.push(k.clone());
        }
    }
    for k in drop {
        map.remove(&k);
    }
    false
}

/// An agent's arguments and environment for a machine where Runo's program isn't: see
/// `bridge_json`, applied to JSON arguments and variables and to Codex's
/// `-c mcp_servers.<name>.*` overrides.
fn bridge_mcp(
    args: &[String],
    env: &mut [(String, String)],
    exe: &str,
    relay: &[String],
) -> Vec<String> {
    let fix_json = |text: &str| -> Option<String> {
        if !text.trim_start().starts_with('{') || !text.contains("command") {
            return None;
        }
        let mut v: serde_json::Value = serde_json::from_str(text).ok()?;
        bridge_json(&mut v, exe, relay);
        Some(v.to_string())
    };
    for (_, v) in env.iter_mut() {
        if let Some(fixed) = fix_json(v) {
            *v = fixed;
        }
    }
    // Codex: `-c mcp_servers.<name>.command="…"` names the program of server <name>.
    let override_of = |i: usize| -> Option<(String, String, String)> {
        if args.get(i)? != "-c" {
            return None;
        }
        let rest = args.get(i + 1)?.strip_prefix("mcp_servers.")?;
        let (path, value) = rest.split_once('=')?;
        let (name, key) = path.rsplit_once('.')?;
        Some((name.to_string(), key.to_string(), value.trim().to_string()))
    };
    let toml_text = |v: &str| serde_json::from_str::<String>(v).unwrap_or_else(|_| v.to_string());
    let mut bridged = Vec::new();
    let mut dropped = Vec::new();
    for i in 0..args.len() {
        if let Some((name, key, value)) = override_of(i) {
            if key == "command" {
                let command = toml_text(&value);
                if command == exe {
                    bridged.push(name);
                } else if local_path(&command) {
                    dropped.push(name);
                }
            }
        }
    }
    let quoted = |s: &str| serde_json::Value::String(s.to_string()).to_string();
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if let Some((name, key, _)) = override_of(i) {
            if dropped.contains(&name) {
                i += 2;
                continue;
            }
            if bridged.contains(&name) {
                out.push("-c".into());
                out.push(match key.as_str() {
                    "command" => format!("mcp_servers.{name}.command={}", quoted(&relay[0])),
                    "args" => format!(
                        "mcp_servers.{name}.args=[{}]",
                        relay[1..]
                            .iter()
                            .map(|a| quoted(a))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    _ => args[i + 1].clone(),
                });
                i += 2;
                continue;
            }
        }
        out.push(fix_json(&args[i]).unwrap_or_else(|| args[i].clone()));
        i += 1;
    }
    out
}

/// Move secrets out of an agent's command line, where anyone on a shared server could
/// read them: Claude Code's `--mcp-config` goes to a private file (`file` is its path
/// from the agent's folder, `write` saves it), and Codex's server `env` and
/// `http_headers` values become variables of the agent's environment that the server
/// is told to read.
fn hide_mcp_secrets(
    args: &[String],
    env: &mut Vec<(String, String)>,
    file: &str,
    write: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--mcp-config"
            && args
                .get(i + 1)
                .is_some_and(|v| v.trim_start().starts_with('{'))
        {
            write(&args[i + 1])?;
            out.push(a.clone());
            out.push(file.to_string());
            i += 2;
            continue;
        }
        if a == "-c" {
            if let Some(next) = args.get(i + 1) {
                let parsed = next.strip_prefix("mcp_servers.").and_then(|rest| {
                    let (path, value) = rest.split_once('=')?;
                    let (name, key) = path.rsplit_once('.')?;
                    matches!(key, "env" | "http_headers").then_some((name, key, value))
                });
                if let Some((name, key, value)) = parsed {
                    let table: Option<toml::Table> = toml::from_str(&format!("v = {value}"))
                        .ok()
                        .and_then(|t: toml::Table| t.get("v")?.as_table().cloned());
                    if let Some(table) = table {
                        let var = |k: &str| {
                            let clean: String = format!("{name}_{k}")
                                .chars()
                                .map(|c| {
                                    if c.is_ascii_alphanumeric() {
                                        c.to_ascii_uppercase()
                                    } else {
                                        '_'
                                    }
                                })
                                .collect();
                            format!("RUNO_MCP_{clean}")
                        };
                        let mut names = Vec::new();
                        for (k, v) in &table {
                            let v = v.as_str().unwrap_or_default().to_string();
                            if key == "env" {
                                env.push((k.clone(), v));
                                names.push(serde_json::Value::String(k.clone()).to_string());
                            } else {
                                env.push((var(k), v));
                                names.push(format!(
                                    "{} = {}",
                                    serde_json::Value::String(k.clone()),
                                    serde_json::Value::String(var(k))
                                ));
                            }
                        }
                        out.push("-c".into());
                        out.push(if key == "env" {
                            format!("mcp_servers.{name}.env_vars=[{}]", names.join(", "))
                        } else {
                            format!(
                                "mcp_servers.{name}.env_http_headers={{ {} }}",
                                names.join(", ")
                            )
                        });
                        i += 2;
                        continue;
                    }
                }
            }
        }
        out.push(a.clone());
        i += 1;
    }
    Ok(out)
}

impl Remote {
    /// The relay's mailbox, relative to the remote home.
    fn mailbox(&self) -> String {
        format!("{}/mcp", self.job_dir())
    }

    /// Answer the requests waiting in the relay's mailbox. Returns how many there were.
    /// Permission requests wait for the person on their own threads; they stay in the
    /// mailbox, set aside, until answered, so a runner after this one can take them up.
    fn pump_mcp(&self) -> Result<usize, String> {
        let d = quote(&self.mailbox());
        let out = self.conn.run(
            &self.vm,
            &format!(
                "d={d}; [ -d \"$d/in\" ] || exit 0; touch \"$d/alive\"; \
                 for f in \"$d\"/in/*; do [ -f \"$f\" ] || continue; n=\"${{f##*/}}\"; printf '%s\\t' \"$n\"; cat \"$f\"; \
                 case \"$(cat \"$f\")\" in *'\"{tool}\"'*) mv \"$f\" \"$d/in/.held-$n\" ;; *) rm -f \"$f\" ;; esac; done",
                tool = crate::approvals::TOOL
            ),
            None,
        )?;
        let text = String::from_utf8_lossy(&out);
        let mut script = format!("d={d}\n");
        let mut n = 0;
        for line in text.lines() {
            let Some((name, request)) = line.split_once('\t') else {
                continue;
            };
            n += 1;
            if crate::approvals::is_ask(request) {
                let (r, name, request) = (self.clone(), name.to_string(), request.to_string());
                std::thread::spawn(move || {
                    if let Some(reply) = answer_mcp(&r.local_dir, &request) {
                        let script =
                            format!("d={}\n{}", quote(&r.mailbox()), reply_script(&name, &reply));
                        let _ = r.conn.run(&r.vm, &script, None);
                    }
                });
                continue;
            }
            if let Some(reply) = answer_mcp(&self.local_dir, request) {
                script.push_str(&reply_script(name, &reply));
            }
        }
        if script.contains("RUNO_EOF") {
            self.conn.run(&self.vm, &script, None)?;
        }
        Ok(n)
    }

    /// Put permission requests a runner before this one set aside back in the mailbox.
    fn reclaim_mcp(&self) {
        let _ = self.conn.run(
            &self.vm,
            &format!(
                "d={}; for f in \"$d\"/in/.held-*; do [ -f \"$f\" ] && mv \"$f\" \"$d/in/${{f##*/.held-}}\"; done; true",
                quote(&self.mailbox())
            ),
            None,
        );
    }
}

/// The lines of a mailbox script (with `$d` set) that leave `reply` for request `name`.
fn reply_script(name: &str, reply: &str) -> String {
    let name = quote(name);
    format!(
        "cat > \"$d/out/.\"{name} <<'RUNO_EOF'\n{reply}\nRUNO_EOF\nmv \"$d/out/.\"{name} \"$d/out/\"{name}\nrm -f \"$d/in/.held-\"{name}\n"
    )
}

/// Local Cove VMs share the session folder, so their relay's mailbox lives in it and a
/// thread here answers it.
fn cove_mailboxes() -> &'static Mutex<Vec<String>> {
    static DIRS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    DIRS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Answer the relay in `dir`/.runo/mcp from now on; returns the relay command.
pub fn serve_cove_mailbox(dir: &str) -> Option<Vec<String>> {
    let base = Path::new(dir).join(".runo");
    let mailbox = base.join("mcp");
    std::fs::create_dir_all(mailbox.join("in")).ok()?;
    std::fs::create_dir_all(mailbox.join("out")).ok()?;
    std::fs::write(base.join("mcp-relay.sh"), MCP_RELAY).ok()?;
    crate::git::exclude(dir, "/.runo/");
    let mut dirs = cove_mailboxes().lock().ok()?;
    if !dirs.iter().any(|d| d == dir) {
        dirs.push(dir.to_string());
        if dirs.len() == 1 {
            std::thread::spawn(|| loop {
                let all = cove_mailboxes()
                    .lock()
                    .map(|d| d.clone())
                    .unwrap_or_default();
                for dir in all {
                    pump_folder(&dir);
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            });
        }
    }
    Some(vec![
        "bash".into(),
        base.join("mcp-relay.sh").to_string_lossy().into_owned(),
        mailbox.to_string_lossy().into_owned(),
        relay_wait(),
    ])
}

/// Answer the requests in one session folder's mailbox on this machine.
fn pump_folder(dir: &str) {
    let mailbox = Path::new(dir).join(".runo/mcp");
    let _ = std::fs::write(mailbox.join("alive"), b"");
    let Ok(entries) = std::fs::read_dir(mailbox.join("in")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let request = std::fs::read_to_string(entry.path()).unwrap_or_default();
        let _ = std::fs::remove_file(entry.path());
        let ask = crate::approvals::is_ask(&request);
        let (dir, out) = (dir.to_string(), mailbox.join("out"));
        let reply = move || {
            if let Some(reply) = answer_mcp(&dir, &request) {
                let tmp = out.join(format!(".{name}"));
                if std::fs::write(&tmp, format!("{reply}\n")).is_ok() {
                    let _ = std::fs::rename(&tmp, out.join(&name));
                }
            }
        };
        // A permission request waits for the person without holding up the others.
        if ask {
            std::thread::spawn(reply);
        } else {
            reply();
        }
    }
}

/// Bridge Runo's MCP server into an agent command headed for a local Cove VM.
pub fn bridge_for_cove(dir: &str, args: &[String], env: &mut [(String, String)]) -> Vec<String> {
    let exe = runo_exe();
    let mentions =
        args.iter().any(|a| a.contains(&exe)) || env.iter().any(|(_, v)| v.contains(&exe));
    if exe.is_empty() || !mentions {
        return bridge_mcp(args, env, "\u{0}", &["bash".into()]);
    }
    match serve_cove_mailbox(dir) {
        Some(relay) => bridge_mcp(args, env, &exe, &relay),
        None => bridge_mcp(args, env, "\u{0}", &["bash".into()]),
    }
}

// ---------- agents running commands on connections (Runo's run_on tool) ----------

/// Run one command for an agent on a connection that allows it. Runs in the connection's
/// scratch space: the `runo-scratch` VM on Cove, `<workdir>/scratch` on a server.
pub fn run_for_agent(conn: &Connection, command: &str) -> Result<String, String> {
    if !conn.agents {
        return Err(format!(
            "{} doesn't allow agents to run commands. The person can turn this on in Settings > Connections.",
            conn.name
        ));
    }
    let scratch = format!("{}/scratch", conn.workdir.trim().trim_matches('/'));
    let script = format!(
        "mkdir -p {s} && cd {s} && {{ {command}\n}} 2>&1",
        s = quote(&scratch)
    );
    let (mut c, stdin) = conn.shell(&scratch_vm(), &script, &conn.forwarded_env(), false)?;
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut pipe) = child.stdin.take() {
        let body = stdin.unwrap_or_default();
        std::thread::spawn(move || {
            let _ = pipe.write_all(body.as_bytes());
        });
    }
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out);
    }
    let mut err = Vec::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_end(&mut err);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&out).into_owned();
    if !err.is_empty() {
        text.push_str(&String::from_utf8_lossy(&err));
    }
    const MAX: usize = 30_000;
    if text.len() > MAX {
        let mut cut = text.len() - MAX;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        text = format!("[… {cut} bytes cut]\n{}", &text[cut..]);
    }
    Ok(format!(
        "{text}\n[exit code {}]",
        status.code().unwrap_or(-1)
    ))
}

/// A new connection id, above every one in use.
pub fn next_id(all: &[Connection]) -> u64 {
    all.iter().map(|c| c.id).max().unwrap_or(0) + 1
}

/// Whether `origin` is a public repository, as far as `gh` can tell.
fn public_repo(dir: &str) -> bool {
    Command::new("gh")
        .args(["repo", "view", "--json", "visibility", "-q", ".visibility"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "PUBLIC")
}

/// Delete a session's sync refs here and its backup on `origin`, if it made one.
fn forget_refs(dir: &str, session: u64) {
    let prefix = format!("refs/runo/s{session}/");
    let refs =
        local_git(dir, &["for-each-ref", "--format=%(refname)", &prefix], &[]).unwrap_or_default();
    let backed_up = refs.lines().any(|r| r.ends_with("/backup"));
    for r in refs.lines().filter(|r| !r.is_empty()) {
        let _ = local_git(dir, &["update-ref", "-d", r], &[]);
    }
    if !backed_up {
        return;
    }
    // The worktree may be gone by the time the push runs; the repository isn't.
    let Ok(common) = local_git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        &[],
    ) else {
        return;
    };
    std::thread::spawn(move || {
        let _ = Command::new("git")
            .arg("--git-dir")
            .arg(&common)
            .args([
                "push",
                "-q",
                "--no-verify",
                "origin",
                "--delete",
                &format!("refs/runo/sessions/{session}"),
            ])
            .stdin(Stdio::null())
            .output();
    });
}

/// Whether an agent's own conversation can follow a session between two places (None is
/// this machine). Claude Code's travels everywhere; Codex's to servers but not into a local
/// Cove VM, which shares only the session folder; other agents keep theirs where they ran.
pub fn carries_conversation(
    provider: &str,
    has_id: bool,
    from: Option<&Connection>,
    to: Option<&Connection>,
) -> bool {
    let local_cove =
        |c: Option<&Connection>| c.is_some_and(|c| c.kind == Kind::Cove && !c.remote());
    match provider {
        "claude" => true,
        "codex" => has_id && !local_cove(from) && !local_cove(to),
        _ => false,
    }
}

/// Delete what a session left behind: its sync refs and backup, and its VM or copy on
/// every connection it ran on.
pub fn cleanup_session(session: &crate::store::Session) {
    forget_refs(&session.dir, session.id);
    let folder = Path::new(&session.dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for id in std::iter::once(session.connection).chain(session.left_on.iter().copied()) {
        if let Some(conn) = get(id) {
            cleanup(&conn, &folder, session.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ssh_host_cant_be_an_option() {
        assert_eq!(
            ssh_host(" ubuntu@203.0.113.7 ", "box"),
            Ok("ubuntu@203.0.113.7")
        );
        assert!(ssh_host("-oProxyCommand=touch /tmp/x", "box").is_err());
        assert!(ssh_host("host -oX", "box").is_err());
        assert!(ssh_host("", "box").unwrap_err().contains("no host"));
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-conn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// A project with one commit and a "remote" that is another local folder.
    fn setup(name: &str) -> (PathBuf, PathBuf, Remote) {
        let root = temp(name);
        let local = root.join("local");
        let far = root.join("far");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::create_dir_all(&far).unwrap();
        git(&local, &["init", "-q", "-b", "main"]);
        std::fs::write(local.join("a.txt"), "one\n").unwrap();
        git(&local, &["add", "-A"]);
        git(&local, &["commit", "-q", "-m", "first"]);
        let conn = Connection {
            id: 9,
            name: "far".into(),
            kind: Kind::Folder,
            host: far.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let remote = Remote {
            remote_dir: conn.remote_dir("My App", 7),
            conn,
            session: 7,
            vm: String::new(),
            local_dir: local.to_string_lossy().into_owned(),
        };
        (local, far, remote)
    }

    #[test]
    fn uncommitted_work_travels_both_ways() {
        let (local, far, remote) = setup("both");
        assert_eq!(remote.remote_dir, "runo-work/my-app-7");
        // Uncommitted edits and a new file go up; the branch stays where it was.
        std::fs::write(local.join("a.txt"), "two\n").unwrap();
        std::fs::write(local.join("new.txt"), "new\n").unwrap();
        let sent = remote.sync_up().unwrap();
        let copy = far.join(&remote.remote_dir);
        assert_eq!(
            std::fs::read_to_string(copy.join("a.txt")).unwrap(),
            "two\n"
        );
        assert_eq!(
            std::fs::read_to_string(copy.join("new.txt")).unwrap(),
            "new\n"
        );
        assert_eq!(git(&copy, &["log", "--format=%s"]), "first");
        assert_eq!(git(&copy, &["symbolic-ref", "--short", "HEAD"]), "main");
        assert!(git(&copy, &["status", "--porcelain"]).contains("a.txt"));

        // The agent commits one change and leaves another uncommitted, there.
        std::fs::write(copy.join("a.txt"), "three\n").unwrap();
        git(&copy, &["commit", "-qam", "agent work"]);
        std::fs::write(copy.join("b.txt"), "draft\n").unwrap();
        std::fs::remove_file(copy.join("new.txt")).unwrap();
        assert!(matches!(
            remote.sync_down(&sent).unwrap(),
            SyncDown::Updated
        ));
        assert_eq!(
            std::fs::read_to_string(local.join("a.txt")).unwrap(),
            "three\n"
        );
        assert_eq!(
            std::fs::read_to_string(local.join("b.txt")).unwrap(),
            "draft\n"
        );
        assert!(!local.join("new.txt").exists());
        assert_eq!(git(&local, &["log", "-1", "--format=%s"]), "agent work");
        assert_eq!(git(&local, &["symbolic-ref", "--short", "HEAD"]), "main");
        assert_eq!(git(&local, &["status", "--porcelain"]), "?? b.txt");

        // Nothing changed there: nothing to do.
        let sent = remote.sync_up().unwrap();
        assert!(matches!(
            remote.sync_down(&sent).unwrap(),
            SyncDown::Unchanged
        ));
    }

    #[test]
    fn edits_made_there_between_turns_come_back_first() {
        let (local, far, remote) = setup("between");
        remote.sync_up().unwrap();
        // Someone edits in a terminal on the connection, then the next turn starts.
        let copy = far.join(&remote.remote_dir);
        std::fs::write(copy.join("a.txt"), "from the terminal\n").unwrap();
        remote.sync_up().unwrap();
        assert_eq!(
            std::fs::read_to_string(local.join("a.txt")).unwrap(),
            "from the terminal\n"
        );
        assert_eq!(
            std::fs::read_to_string(copy.join("a.txt")).unwrap(),
            "from the terminal\n"
        );
        // Both sides change: this machine wins, theirs is kept under a ref.
        std::fs::write(copy.join("a.txt"), "there\n").unwrap();
        std::fs::write(local.join("a.txt"), "here\n").unwrap();
        remote.sync_up().unwrap();
        assert_eq!(
            std::fs::read_to_string(copy.join("a.txt")).unwrap(),
            "here\n"
        );
        let kept = git(
            &local,
            &["for-each-ref", "--format=%(refname)", "refs/runo/s7/"],
        );
        let kept = kept
            .lines()
            .find(|r| r.contains("kept-"))
            .unwrap()
            .to_string();
        assert_eq!(git(&local, &["show", &format!("{kept}:a.txt")]), "there");
    }

    #[test]
    fn a_wiped_copy_there_does_not_delete_files_here() {
        let (local, far, remote) = setup("wiped");
        std::fs::write(local.join("todo.txt"), "note\n").unwrap();
        remote.sync_up().unwrap();
        // The VM is recreated or its folder deleted between turns.
        std::fs::remove_dir_all(far.join(&remote.remote_dir)).unwrap();
        remote.sync_up().unwrap();
        assert_eq!(
            std::fs::read_to_string(local.join("a.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(
            std::fs::read_to_string(local.join("todo.txt")).unwrap(),
            "note\n"
        );
        let copy = far.join(&remote.remote_dir);
        assert_eq!(
            std::fs::read_to_string(copy.join("todo.txt")).unwrap(),
            "note\n"
        );
    }

    #[test]
    fn local_edits_during_a_turn_are_kept() {
        let (local, far, remote) = setup("kept");
        let sent = remote.sync_up().unwrap();
        std::fs::write(far.join(&remote.remote_dir).join("a.txt"), "agent\n").unwrap();
        std::fs::write(local.join("a.txt"), "mine\n").unwrap();
        let SyncDown::Kept(theirs) = remote.sync_down(&sent).unwrap() else {
            panic!("expected the local edit to be kept");
        };
        assert_eq!(
            std::fs::read_to_string(local.join("a.txt")).unwrap(),
            "mine\n"
        );
        assert_eq!(git(&local, &["show", &format!("{theirs}:a.txt")]), "agent");
    }

    #[test]
    fn claude_conversations_follow_the_session() {
        let (local, far, remote) = setup("claude");
        let home = temp("claude-home");
        let folder = home
            .join(".claude/projects")
            .join(claude_folder_name(&remote.local_dir));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("abc.jsonl"), "{\"x\":1}\n").unwrap();
        let _env = crate::store::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        remote.sync_up().unwrap();
        remote.carry_up("claude", "abc").unwrap();
        let there = far
            .join(".claude/projects")
            .join(claude_folder_name(
                &far.join(&remote.remote_dir).to_string_lossy(),
            ))
            .join("abc.jsonl");
        assert_eq!(std::fs::read_to_string(&there).unwrap(), "{\"x\":1}\n");
        std::fs::write(&there, "{\"x\":1}\n{\"x\":2}\n").unwrap();
        remote.carry_down("claude", "abc").unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.join("abc.jsonl")).unwrap(),
            "{\"x\":1}\n{\"x\":2}\n"
        );
        match old_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        let _ = local;
    }

    #[test]
    fn agents_run_commands_only_where_allowed() {
        let (_, _, remote) = setup("agents");
        let mut conn = remote.conn.clone();
        let out = run_for_agent(&conn, "echo hi; exit 3").unwrap();
        assert!(out.starts_with("hi\n"), "{out}");
        assert!(out.ends_with("[exit code 3]"));
        conn.agents = false;
        assert!(run_for_agent(&conn, "true").is_err());
    }

    #[test]
    fn a_turn_runs_there_and_its_work_comes_back() {
        let (local, far, remote) = setup("turn");
        std::fs::write(local.join("todo.txt"), "fix it\n").unwrap();
        let file = RunFile {
            remote: remote.clone(),
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "cat todo.txt > seen.txt; echo \"$GREETING\"; echo '{\"session_id\":\"\"}'".into(),
            ],
            env: vec![("GREETING".into(), "hello there".into())],
            conversation: None,
            backup: false,
            resume: false,
            untracked: false,
            stop_on_denial: false,
            approval_key: String::new(),
        };
        let path = local.parent().unwrap().join("run.json");
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(connection_run(path.to_str().unwrap()), 0);
        assert!(!path.exists(), "the run file holds secrets and is removed");
        assert_eq!(
            std::fs::read_to_string(local.join("seen.txt")).unwrap(),
            "fix it\n"
        );
        assert!(far.join(&remote.remote_dir).join("seen.txt").exists());
    }

    /// A program in a terminal there (the agent's CLI, a setup command) runs in the
    /// session's copy with its environment, and what it did comes back when it exits.
    #[test]
    fn a_terminal_program_runs_there_and_its_work_comes_back() {
        let (local, far, remote) = setup("pty");
        std::fs::write(local.join("todo.txt"), "fix it\n").unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "cat todo.txt > seen.txt; echo \"$GREETING\" > env.txt; exit 3",
            ])
            .env("GREETING", "hello there");
        let runner = Sandbox::Remote(Box::new(remote.clone()))
            .pty_command(&command, &remote.local_dir, None)
            .unwrap();
        let args: Vec<String> = runner
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "--connection-shell");
        assert_eq!(connection_shell(&args[1]), 3);
        assert!(
            !Path::new(&args[1]).exists(),
            "the run file holds secrets and is removed"
        );
        assert_eq!(
            std::fs::read_to_string(local.join("seen.txt")).unwrap(),
            "fix it\n"
        );
        assert_eq!(
            std::fs::read_to_string(local.join("env.txt")).unwrap(),
            "hello there\n"
        );
        let job = far.join(remote.job_dir());
        assert!(!job.join("pty.sh").exists(), "the script deletes itself");
    }

    /// The turn keeps going there while nothing here follows it (this machine asleep or
    /// off, Runo closed); picking it up later shows its output and brings its work back.
    #[test]
    fn a_turn_keeps_going_while_this_machine_is_away() {
        let (local, far, remote) = setup("away");
        remote.sync_up().unwrap();
        remote
            .start_job(
                "sh",
                &[
                    "-c".into(),
                    "echo started; sleep 1; echo \"$GREETING\" > result.txt; echo finished".into(),
                ],
                &[("GREETING".into(), "built while away".into())],
            )
            .unwrap();
        // Nobody is watching: the turn finishes on its own.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while remote.job_state().unwrap() == JobState::Running {
            assert!(
                std::time::Instant::now() < deadline,
                "the turn never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(remote.job_state().unwrap(), JobState::Done(0));
        let job = far.join(remote.job_dir());
        assert!(
            !job.join("run.sh").exists(),
            "the script with the keys is deleted"
        );
        assert!(!local.join("result.txt").exists());
        // Runo opens again and picks the turn up.
        let mut lines = Vec::new();
        let shown = remote.follow_job(0, true, &mut |l| lines.push(l.to_string()));
        assert_eq!(lines, ["started", "finished"]);
        assert_eq!(shown, "started\nfinished\n".len() as u64);
        let mut rest = Vec::new();
        remote.follow_job(8, true, &mut |l| rest.push(l.to_string()));
        assert_eq!(
            rest,
            ["finished"],
            "a reconnect carries on where it left off"
        );
        let file = RunFile {
            remote: remote.clone(),
            program: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            conversation: None,
            backup: false,
            resume: true,
            untracked: false,
            stop_on_denial: false,
            approval_key: String::new(),
        };
        let path = local.parent().unwrap().join("resume.json");
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(connection_run(path.to_str().unwrap()), 0);
        assert_eq!(
            std::fs::read_to_string(local.join("result.txt")).unwrap(),
            "built while away\n"
        );
    }

    #[test]
    fn a_stopped_turn_ends_there() {
        let (_, _, remote) = setup("stop");
        remote.sync_up().unwrap();
        remote
            .start_job("sh", &["-c".into(), "sleep 60".into()], &[])
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while remote.job_state().unwrap() != JobState::Running {
            assert!(
                std::time::Instant::now() < deadline,
                "the turn never started"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        remote.stop_job().unwrap();
        assert_ne!(remote.job_state().unwrap(), JobState::Running);
        // A new turn replaces a finished one cleanly.
        remote
            .start_job("sh", &["-c".into(), "echo again".into()], &[])
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while remote.job_state().unwrap() != JobState::Done(0) {
            assert!(
                std::time::Instant::now() < deadline,
                "the second turn never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// The same turn against a real Cove server. Opt in with RUNO_COVE_E2E_URL and
    /// RUNO_COVE_E2E_KEY (and RUNO_COVE_E2E_VM to reuse a VM that already has git).
    #[test]
    fn a_turn_runs_on_a_cove_server() {
        let Ok(url) = std::env::var("RUNO_COVE_E2E_URL") else {
            return;
        };
        let root = temp("cove-e2e");
        let local = root.join("local");
        std::fs::create_dir_all(&local).unwrap();
        git(&local, &["init", "-q", "-b", "main"]);
        std::fs::write(local.join("a.txt"), "one\n").unwrap();
        git(&local, &["add", "-A"]);
        git(&local, &["commit", "-q", "-m", "first"]);
        std::fs::write(local.join("todo.txt"), "fix it\n").unwrap();
        let conn = Connection {
            id: 3,
            url,
            api_key: std::env::var("RUNO_COVE_E2E_KEY").unwrap_or_default(),
            ..Connection::new_cove(3)
        };
        let session = std::process::id() as u64;
        let remote = Remote {
            remote_dir: conn.remote_dir("Cove E2E", session),
            vm: std::env::var("RUNO_COVE_E2E_VM").unwrap_or_else(|_| "runo-e2e".into()),
            conn,
            session,
            local_dir: local.to_string_lossy().into_owned(),
        };
        let file = RunFile {
            remote: remote.clone(),
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "cat todo.txt > seen.txt; uname -n >> seen.txt; git add seen.txt; \
                 git -c user.name=a -c user.email=a@a commit -qm 'agent work'; echo \"$GREETING\""
                    .into(),
            ],
            env: vec![("GREETING".into(), "hello there".into())],
            conversation: None,
            backup: false,
            resume: false,
            untracked: false,
            stop_on_denial: false,
            approval_key: String::new(),
        };
        let path = root.join("run.json");
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(connection_run(path.to_str().unwrap()), 0);
        let seen = std::fs::read_to_string(local.join("seen.txt")).unwrap();
        assert!(seen.starts_with("fix it\n"), "{seen}");
        assert_eq!(git(&local, &["log", "-1", "--format=%s"]), "agent work");
        // A shell there sees the same copy.
        let out = remote
            .conn
            .run(
                &remote.vm,
                &format!("cat {}/seen.txt", remote.remote_dir),
                None,
            )
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out), seen);
        let _ = remote
            .conn
            .run(&remote.vm, &format!("rm -rf {}", remote.remote_dir), None);
    }

    #[test]
    fn warnings_and_kept_files_come_back_to_the_turn() {
        let (local, far, remote) = setup("warned");
        let sent = remote.sync_up().unwrap();
        std::fs::write(far.join(&remote.remote_dir).join("a.txt"), "agent\n").unwrap();
        std::fs::write(local.join("a.txt"), "mine\n").unwrap();
        let SyncDown::Kept(kept) = remote.sync_down(&sent).unwrap() else {
            panic!("expected the local edit to be kept");
        };
        let line = format!("{WARNING}Files here changed. They are saved as {kept}.");
        let found = warnings(&["some agent noise".into(), line.clone(), line.clone()]);
        assert_eq!(found.len(), 1);
        assert_eq!(kept_refs(&found[0]), std::slice::from_ref(&kept));
        // Restoring puts the agent's version back and saves what was here.
        let saved = restore_kept(&remote.local_dir, 7, &kept).unwrap();
        assert_eq!(
            std::fs::read_to_string(local.join("a.txt")).unwrap(),
            "agent\n"
        );
        assert_eq!(git(&local, &["show", &format!("{saved}:a.txt")]), "mine");
        assert!(restore_kept(&remote.local_dir, 7, &kept).is_err());
        assert!(restore_kept(&remote.local_dir, 8, "refs/runo/s7/kept-1").is_err());
        // Deleting the session deletes its refs.
        forget_refs(&remote.local_dir, 7);
        assert_eq!(
            git(
                &local,
                &["for-each-ref", "--format=%(refname)", "refs/runo/"]
            ),
            ""
        );
    }

    #[test]
    fn conversations_travel_only_where_the_agent_can_resume_them() {
        let far = Connection::new_ssh(2);
        let vm = Connection::new_cove(3);
        assert!(carries_conversation("claude", false, None, Some(&far)));
        assert!(carries_conversation("claude", true, Some(&vm), None));
        assert!(carries_conversation("codex", true, None, Some(&far)));
        assert!(!carries_conversation("codex", false, None, Some(&far)));
        assert!(!carries_conversation("codex", true, None, Some(&vm)));
        assert!(!carries_conversation("gemini", true, None, Some(&far)));
    }

    #[test]
    fn test_says_how_each_agent_signs_in() {
        let codex = sign_in("codex").unwrap();
        let forwarded = vec!["OPENAI_API_KEY".to_string()];
        fn check<'a>(
            forwarded: &'a [String],
            local_file: bool,
            file_there: bool,
            unlisted: Option<&'a str>,
        ) -> AgentCheck<'a> {
            AgentCheck {
                installed_there: Some(true),
                forwarded,
                set_here_not_listed: unlisted,
                local_file,
                file_there,
                kept_there: false,
                server_can_keep: false,
            }
        }
        assert_eq!(
            agent_line("Codex", codex, &check(&forwarded, false, false, None)),
            "✓ Codex: installed, signs in with OPENAI_API_KEY"
        );
        assert!(agent_line("Codex", codex, &check(&[], true, false, None))
            .contains("~/.codex/auth.json from this computer"));
        assert!(agent_line(
            "Codex",
            codex,
            &check(&[], false, false, Some("CODEX_API_KEY"))
        )
        .contains("add it to Keys passed in"));
        let claude = agent_line(
            "Claude Code",
            sign_in("claude").unwrap(),
            &check(&[], false, false, None),
        );
        assert!(
            claude.starts_with("✗") && claude.contains("claude setup-token"),
            "{claude}"
        );
        // On a Cove server that can keep it, the fix is signing in there.
        let mut on_cove = check(&[], false, false, None);
        on_cove.server_can_keep = true;
        let line = agent_line("Claude Code", sign_in("claude").unwrap(), &on_cove);
        assert!(line.contains("Sign in at Claude Code login"), "{line}");
        on_cove.kept_there = true;
        assert!(
            agent_line("Claude Code", sign_in("claude").unwrap(), &on_cove)
                .starts_with("✓ Claude Code: installed, signed in there")
        );
        for p in crate::agent::PROVIDERS.iter() {
            assert!(sign_in(p.id).is_some(), "{} has no sign-in", p.id);
        }
        let mut old = Connection::new_ssh(1);
        old.keys = OLD_DEFAULT_KEYS.into();
        assert!(old.key_names().contains(&"GEMINI_API_KEY".to_string()));
        assert_eq!(version_number("codex-cli 0.50.1"), Some("0.50.1".into()));
        assert_eq!(
            version_number("2.0.14 (Claude Code)"),
            Some("2.0.14".into())
        );
        assert_eq!(version_number("v1.2"), None);
    }

    #[test]
    fn runo_mcp_is_relayed_and_local_paths_stay_home() {
        let relay = vec!["bash".to_string(), "relay.sh".into(), "box".into()];
        let config = serde_json::json!({ "mcpServers": {
            "runo": { "command": "/Apps/Runo", "args": ["--mcp-memory", "/w"] },
            "Cove": { "command": "/opt/homebrew/bin/cove", "args": ["mcp"] },
            "fs": { "command": "npx", "args": ["-y", "fs"], "env": { "TOKEN": "s3cret" } },
        }});
        let args = vec![
            "-p".to_string(),
            "--mcp-config".into(),
            config.to_string(),
            "-c".into(),
            "mcp_servers.runo.command=\"/Apps/Runo\"".into(),
            "-c".into(),
            "mcp_servers.runo.args=[\"--mcp-memory\", \"/w\"]".into(),
            "-c".into(),
            "mcp_servers.Cove.command=\"/opt/homebrew/bin/cove\"".into(),
            "-c".into(),
            "mcp_servers.Cove.args=[\"mcp\"]".into(),
            "-c".into(),
            "mcp_servers.fs.env={ \"TOKEN\" = \"s3cret\" }".into(),
        ];
        let mut env = vec![(
            "OPENCODE_CONFIG_CONTENT".to_string(),
            r#"{"mcp":{"runo":{"type":"local","command":["/Apps/Runo","--mcp-memory","/w"]}}}"#
                .to_string(),
        )];
        let out = bridge_mcp(&args, &mut env, "/Apps/Runo", &relay);
        let claude: serde_json::Value = serde_json::from_str(&out[2]).unwrap();
        assert_eq!(claude["mcpServers"]["runo"]["command"], "bash");
        assert_eq!(
            claude["mcpServers"]["runo"]["args"],
            serde_json::json!(["relay.sh", "box"])
        );
        assert!(claude["mcpServers"].get("Cove").is_none());
        assert_eq!(claude["mcpServers"]["fs"]["command"], "npx");
        assert!(out.contains(&"mcp_servers.runo.command=\"bash\"".to_string()));
        assert!(out.contains(&"mcp_servers.runo.args=[\"relay.sh\", \"box\"]".to_string()));
        assert!(!out.iter().any(|a| a.contains("Cove")));
        assert!(env[0].1.contains(r#""command":["bash","relay.sh","box"]"#));

        // Secrets leave the command line.
        let mut written = String::new();
        let mut env = Vec::new();
        let hidden = hide_mcp_secrets(&out, &mut env, "../x.runo/mcp.json", &mut |t| {
            written = t.to_string();
            Ok(())
        })
        .unwrap();
        assert!(written.contains("s3cret"));
        assert!(!hidden.iter().any(|a| a.contains("s3cret")), "{hidden:?}");
        assert!(hidden.contains(&"../x.runo/mcp.json".to_string()));
        assert!(hidden.contains(&"mcp_servers.fs.env_vars=[\"TOKEN\"]".to_string()));
        assert_eq!(env, [("TOKEN".to_string(), "s3cret".to_string())]);
    }

    /// The relay script and the pump talk through a mailbox: a request the agent sends
    /// comes back answered by Runo's own server.
    #[test]
    fn the_relay_answers_through_the_mailbox() {
        let dir = temp("relay");
        let relay = serve_cove_mailbox(dir.to_str().unwrap()).unwrap();
        let mut child = Command::new(&relay[0])
            .args(&relay[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        {
            let mut stdin = child.stdin.take().unwrap();
            writeln!(
                stdin,
                r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
            )
            .unwrap();
            writeln!(stdin, r#"{{"jsonrpc":"2.0","id":7,"method":"ping"}}"#).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        let reply: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"], serde_json::json!({}));
        if let Ok(mut dirs) = cove_mailboxes().lock() {
            dirs.retain(|d| Path::new(d) != dir);
        }
    }

    #[test]
    fn a_relay_on_a_connection_is_answered_by_the_runner() {
        let (_, far, remote) = setup("pump");
        remote.sync_up().unwrap();
        let mailbox = far.join(remote.mailbox());
        std::fs::create_dir_all(&mailbox).unwrap();
        let script = far.join("relay.sh");
        std::fs::write(&script, MCP_RELAY).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pump = {
            let (r, stop) = (remote.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = r.pump_mcp();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            })
        };
        let mut child = Command::new("bash")
            .arg(&script)
            .arg(&mailbox)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(
            child.stdin.take().unwrap(),
            r#"{{"jsonrpc":"2.0","id":"a","method":"tools/list"}}"#
        )
        .unwrap();
        let out = child.wait_with_output().unwrap();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = pump.join();
        let reply: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_eq!(reply["id"], "a");
        assert!(reply["result"]["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty()));
    }

    /// A permission request nobody answers is refused in the shape Claude Code reads,
    /// with words that end a detached turn, and requests set aside are taken up again.
    #[test]
    fn an_unanswered_permission_request_is_refused() {
        let (_, far, remote) = setup("ask");
        let mailbox = far.join(remote.mailbox());
        std::fs::create_dir_all(mailbox.join("in")).unwrap();
        let script = far.join("relay.sh");
        std::fs::write(&script, MCP_RELAY).unwrap();
        let mut child = Command::new("bash")
            .arg(&script)
            .arg(&mailbox)
            .arg("1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(
            child.stdin.take().unwrap(),
            r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"permission_prompt","arguments":{{"tool_name":"Bash","input":{{}}}}}}}}"#
        )
        .unwrap();
        let out = child.wait_with_output().unwrap();
        let reply: serde_json::Value =
            serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
        assert_eq!(reply["id"], 4);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        let answer: crate::approvals::Answer = serde_json::from_str(text).unwrap();
        assert!(
            matches!(&answer, crate::approvals::Answer::Deny { message } if message.contains("hasn't granted"))
        );
        assert_eq!(std::fs::read_dir(mailbox.join("in")).unwrap().count(), 0);

        std::fs::write(mailbox.join("in/.held-9-1"), "{}\n").unwrap();
        remote.reclaim_mcp();
        assert!(mailbox.join("in/9-1").is_file());
        assert!(!mailbox.join("in/.held-9-1").exists());
    }

    #[test]
    fn a_turn_that_never_starts_is_noticed() {
        let (_, far, remote) = setup("unstarted");
        let job = far.join(remote.job_dir());
        std::fs::create_dir_all(&job).unwrap();
        let script = job.join("run.sh");
        std::fs::write(&script, "true\n").unwrap();
        assert_eq!(remote.job_state().unwrap(), JobState::Running);
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        std::fs::File::options()
            .write(true)
            .open(&script)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(remote.job_state().unwrap(), JobState::Unstarted);
    }

    #[test]
    fn listening_ports_are_read_from_any_tool() {
        let ss = "LISTEN 0      511          0.0.0.0:5173      0.0.0.0:*\n\
                  LISTEN 0      4096   [::ffff:127.0.0.1]:3000            *:*\n\
                  LISTEN 0      128             [::]:22             [::]:*";
        assert_eq!(parse_listening(ss), vec![5173, 3000, 22]);
        let netstat = "tcp        0      0 0.0.0.0:8080            0.0.0.0:*               LISTEN\n\
                       tcp4       0      0  *.4321                 *.*                    LISTEN\n\
                       tcp4       0      0  127.0.0.1.8000         127.0.0.1.52011        ESTABLISHED";
        assert_eq!(parse_listening(netstat), vec![8080, 4321]);
        let lsof = "node    4242 me   23u  IPv4 0xabc      0t0  TCP *:5174 (LISTEN)";
        assert_eq!(parse_listening(lsof), vec![5174]);
    }

    /// A port forwarded through a Cove-style pipe reaches the server behind it.
    #[test]
    fn a_forwarded_port_reaches_the_server_there() {
        let (_, _, remote) = setup("forward");
        let server = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = server.accept() {
                let mut buf = [0u8; 5];
                let _ = s.read_exact(&mut buf);
                let _ = s.write_all(b"pong:");
                let _ = s.write_all(&buf);
            }
        });
        let sandbox = Sandbox::Remote(Box::new(remote));
        let forward = sandbox.forward("/", port).unwrap();
        let mut client = std::net::TcpStream::connect(("127.0.0.1", forward.local)).unwrap();
        client.write_all(b"hello").unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut got = [0u8; 10];
        client.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"pong:hello");
    }

    #[test]
    fn a_refused_command_is_spotted_in_claudes_stream() {
        let mut watch = DenialWatch::default();
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Claude requested permissions to use Bash, but you haven't granted it yet."}]}}"#,
        ];
        assert_eq!(watch.denied(lines[0]), None);
        assert_eq!(watch.denied(lines[1]), Some("Bash".into()));
        let failed = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"exit code 1"}]}}"#;
        assert_eq!(watch.denied(failed), None);
        assert_eq!(watch.denied("not json"), None);
        // Asked in Runo mid-turn: an unanswered request ends the turn, a refusal doesn't.
        let timed_out = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":{}}}]}}}}"#,
            serde_json::Value::String(crate::approvals::timed_out(std::time::Duration::from_secs(
                300
            )))
        );
        assert_eq!(watch.denied(&timed_out), Some("Bash".into()));
        let refused = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"The person denied this in Runo."}]}}"#;
        assert_eq!(watch.denied(refused), None);
    }

    #[test]
    fn base64_round_trips() {
        for data in [
            &b""[..],
            b"a",
            b"ab",
            b"abc",
            b"hello world, this is longer than one line of base64 output for sure!",
        ] {
            assert_eq!(base64_decode(&base64_encode(data)), data);
        }
    }

    #[test]
    fn quoting_and_names() {
        assert_eq!(quote("plain/path-1"), "plain/path-1");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
        assert_eq!(slug("My App!"), "my-app");
        assert_eq!(claude_folder_name("/Users/a/b.c"), "-Users-a-b-c");
        let ssh = Connection::new_ssh(2);
        assert!(ssh.remote());
        assert!(!Connection::new_cove(1).remote());
        let mut far = Connection::new_cove(3);
        far.url = "http://vps:7457".into();
        assert!(far.remote());
        assert_eq!(far.summary(), "http://vps:7457 · 2 CPU · 4 GB");
    }

    #[test]
    fn cove_flags_follow_the_settings() {
        let mut c = Connection::new_cove(1);
        c.network = "off".into();
        c.idle_stop = 30;
        c.cpus = 4;
        c.idle_pause = false;
        assert_eq!(
            c.cove_ensure_args("runo-1").join(" "),
            "exec runo-1 --create --cpus 4 --memory 4096 --network off --idle-stop 30"
        );
        // Pausing adds the pause timer, when this machine's Cove has it.
        c.idle_pause = true;
        let args = c.cove_ensure_args("runo-1").join(" ");
        if crate::cove::has_auto_pause() {
            assert!(args.ends_with("--auto-pause 30 --idle-stop 30"), "{args}");
        } else {
            assert!(args.ends_with("--idle-stop 30"), "{args}");
        }
    }

    #[test]
    fn github_access_uses_a_credential_helper() {
        let env = github_env("tok");
        assert!(env.contains(&("GH_TOKEN".into(), "tok".into())));
        assert!(env
            .iter()
            .any(|(k, v)| k == "GIT_CONFIG_VALUE_0" && v.contains("$GH_TOKEN")));
    }
}
