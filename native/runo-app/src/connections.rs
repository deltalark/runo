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

/// API keys forwarded to a new connection unless the person changes the list.
pub const DEFAULT_KEYS: &str =
    "ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN, OPENAI_API_KEY, OPENROUTER_API_KEY";

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
    /// Cove: the API key of a remote server (`cove key` on that server).
    pub api_key: String,
    /// Cove VM size and limits.
    pub cpus: u32,
    pub memory_mb: u64,
    /// "full" or "off".
    pub network: String,
    /// Stop an idle VM after this many minutes; 0 keeps it running.
    pub idle_stop: u32,
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
            cpus: 2,
            memory_mb: 4096,
            network: "full".into(),
            idle_stop: 0,
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
                let place = if self.remote() {
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

    pub fn key_names(&self) -> Vec<String> {
        self.keys
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(String::from)
            .collect()
    }

    /// The forwarded variables that are set on this machine, plus GitHub when allowed.
    pub fn forwarded_env(&self) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = self
            .key_names()
            .into_iter()
            .filter_map(|k| std::env::var(&k).ok().map(|v| (k, v)))
            .collect();
        if self.github {
            if let Some(token) = gh_token() {
                env.extend(github_env(&token));
            }
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
    fn cove(&self) -> Result<Command, String> {
        let bin = crate::cove::binary().ok_or("Cove is not installed on this machine.")?;
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
        if self.idle_stop > 0 {
            args.extend(["--idle-stop".into(), self.idle_stop.to_string()]);
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
                if self.host.trim().is_empty() {
                    return Err(format!("{} has no host set.", self.name));
                }
                let mut c = self.ssh_base();
                if pty {
                    c.arg("-tt");
                    c.args([
                        self.host.trim(),
                        "--",
                        &format!("bash -lc {}", quote(script)),
                    ]);
                    return Ok((c, None));
                }
                c.arg("-T");
                c.args([self.host.trim(), "--", "bash -l -s"]);
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
                    format!("{}:{dir}", self.host.trim()),
                    vec![("GIT_SSH_COMMAND".into(), ssh)],
                ))
            }
            Kind::Cove => {
                let bin = crate::cove::binary().ok_or("Cove is not installed on this machine.")?;
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

/// Check a connection in the background: reachable, git present, which agents exist.
/// `done` runs on the background thread when the result is in.
pub fn test(conn: Connection, done: impl FnOnce() + Send + 'static) {
    if let Ok(mut t) = tests().lock() {
        t.insert(conn.id, (None, "Testing…".into()));
    }
    std::thread::spawn(move || {
        let script = "printf 'host=%s\\n' \"$(uname -sm)\"; \
                      for t in git claude codex opencode; do command -v $t >/dev/null 2>&1 && printf '%s ' $t; done; echo";
        let result = match conn.run(&scratch_vm(), script, None) {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out);
                let mut lines = text.lines();
                let host = lines
                    .next()
                    .and_then(|l| l.strip_prefix("host="))
                    .unwrap_or("")
                    .to_string();
                let tools: Vec<&str> = lines.next().unwrap_or("").split_whitespace().collect();
                let git = if tools.contains(&"git") {
                    "git found"
                } else {
                    "git will be installed on first sync"
                };
                let agents: Vec<&&str> = tools.iter().filter(|t| **t != "git").collect();
                let agents = if agents.is_empty() {
                    "agents install on first use".to_string()
                } else {
                    agents
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                (Some(true), format!("Connected: {host}, {git}, {agents}."))
            }
            Err(e) => (Some(false), e.lines().last().unwrap_or("").to_string()),
        };
        if let Ok(mut t) = tests().lock() {
            t.insert(conn.id, result);
        }
        done();
    });
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
                let file = RunFile {
                    remote: (**r).clone(),
                    program: String::new(),
                    args: Vec::new(),
                    env: Vec::new(),
                    conversation: None,
                    backup: false,
                    resume: false,
                };
                let path = run_file_path(r.session);
                write_private(
                    &path,
                    &serde_json::to_vec(&file).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("could not prepare the terminal: {e}"))?;
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                Ok((
                    exe.to_string_lossy().into_owned(),
                    vec![
                        "--connection-shell".into(),
                        path.to_string_lossy().into_owned(),
                    ],
                ))
            }
        }
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
        let mut env = self.conn.forwarded_env();
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

/// Commit the whole working tree (untracked files too) to the ref named in $1 without
/// touching the branch or the index, and print "<commit> <branch>".
const CHECKPOINT: &str = r#"set -e
idx="$(mktemp -u)"; trap 'rm -f "$idx"' EXIT
export GIT_INDEX_FILE="$idx"
head="$(git rev-parse -q --verify HEAD || true)"
[ -n "$head" ] && git read-tree "$head"
git add -A -- .
tree="$(git write-tree)"
export GIT_AUTHOR_NAME=Runo GIT_AUTHOR_EMAIL=runo@localhost GIT_COMMITTER_NAME=Runo GIT_COMMITTER_EMAIL=runo@localhost
if [ -n "$head" ]; then c="$(git commit-tree "$tree" -p "$head" -m 'runo sync')"; else c="$(git commit-tree "$tree" -m 'runo sync')"; fi
git update-ref "$1" "$c"
echo "$c $(git symbolic-ref -q --short HEAD || true)""#;

/// Make the folder match checkpoint $1: the branch ($2, or detached) at its parent and
/// the working tree at its content, with the changes uncommitted as they were.
const RESTORE: &str = r#"set -e
c="$1"; b="$2"
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
        if let Some(base) = base.filter(|b| !b.is_empty()) {
            if let Ok((theirs, branch)) = self.fetch_remote() {
                if !self.same(&theirs, &base) {
                    let (now, _) = self.local_checkpoint()?;
                    if self.same(&now, &base) {
                        local_sh(dir, RESTORE, &[&theirs, &branch])?;
                        eprintln!("Runo: brought in changes made on {}.", self.conn.name);
                    } else {
                        let kept = self.keep(&theirs);
                        eprintln!(
                            "Runo: files changed both here and on {}. This machine's version is used; theirs is saved as {kept}.",
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
        Ok(SyncDown::Updated(commit))
    }

    /// Push the latest checkpoint to the project's `origin`, without waiting.
    fn backup(&self, commit: &str) {
        let has_origin = local_git(&self.local_dir, &["remote", "get-url", "origin"], &[]).is_ok();
        if !has_origin {
            return;
        }
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.local_dir)
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
}

impl Remote {
    /// The folder beside the session copy that holds a turn's script, output and exit
    /// code, relative to the remote home. Outside the copy, so git never sees it.
    fn job_dir(&self) -> String {
        format!("{}.runo", self.remote_dir)
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
        let mut script = String::from("cd \"$HOME\"\n");
        script.push_str(&format!(
            "job=\"$HOME/\"{}\necho $$ > \"$job/pid\"\nrm -f \"$0\"\n",
            quote(&job)
        ));
        for (k, v) in env {
            script.push_str(&format!("export {k}={}\n", quote(v)));
        }
        script.push_str(&format!(
            "cd {} && bash -lc {} runo {} {} </dev/null >>\"$job/out.log\" 2>>\"$job/err.log\"\n",
            quote(&self.remote_dir),
            quote(&launcher(program)),
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
            _ => {
                self.conn.run(
                    &self.vm,
                    &format!(
                        "if command -v setsid >/dev/null 2>&1; then setsid nohup bash {f} </dev/null >/dev/null 2>&1 & else nohup bash {f} </dev/null >/dev/null 2>&1 & fi",
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
                "job={}; if [ -f \"$job/exit\" ]; then echo \"done $(cat \"$job/exit\")\"; \
                 elif [ -s \"$job/pid\" ] && kill -0 \"$(cat \"$job/pid\")\" 2>/dev/null; then echo running; \
                 elif [ -f \"$job/run.sh\" ]; then echo running; \
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
            format!(
                "i=0; while [ ! -s {job}/pid ] && [ $i -lt 150 ]; do sleep 0.2; i=$((i+1)); done; \
                 [ -s {job}/pid ] || exit 0; \
                 touch {job}/out.log; exec tail -s 0.5 -c +{} -F --pid=\"$(cat {job}/pid)\" {job}/out.log 2>/dev/null",
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
                    "job={}; [ -s \"$job/pid\" ] || exit 0; p=\"$(cat \"$job/pid\")\"; \
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

fn stop_requested() -> bool {
    STOP.load(std::sync::atomic::Ordering::SeqCst)
}

pub enum SyncDown {
    Unchanged,
    Updated(String),
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
pub fn launcher(program: &str) -> String {
    crate::cove::launcher(program)
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
    let r = &file.remote;
    let in_git = local_git(&r.local_dir, &["rev-parse", "--git-dir"], &[]).is_ok();
    if !in_git {
        eprintln!(
            "Runo: {} needs the project to be a git repository, so the session's files can be synced there.",
            r.conn.name
        );
        return 1;
    }
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
                    "Runo: the conversation could not be copied to {}: {e}",
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
    let mut conversation = file.conversation.clone();
    let stdout = std::io::stdout();
    let mut show = |line: &str| {
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
    let errors = r.job_errors();
    if !errors.trim().is_empty() {
        eprint!("{errors}");
    }
    if let Some((provider, id)) = &conversation {
        if !id.is_empty() {
            if let Err(e) = r.carry_down(provider, id) {
                eprintln!("Runo: the conversation could not be copied back: {e}");
            }
        }
    }
    match r.sync_down(&sent) {
        Ok(SyncDown::Updated(c)) => {
            if file.backup {
                r.backup(&c);
            }
        }
        Ok(SyncDown::Kept(kept)) => eprintln!(
            "Runo: files here changed while the agent worked on {}, so they were kept. \
             The agent's version is saved as {kept}.",
            r.conn.name
        ),
        Ok(SyncDown::Unchanged) => {}
        Err(e) => eprintln!("Runo: could not sync back from {}: {e}", r.conn.name),
    }
    code
}

/// `runo --connection-shell <file>`: a terminal in the session's copy on a connection,
/// synced there first and back when the shell exits.
pub fn connection_shell(path: &str) -> i32 {
    let data = std::fs::read(path);
    let _ = std::fs::remove_file(path);
    let Ok(file) = data
        .map_err(|e| e.to_string())
        .and_then(|d| serde_json::from_slice::<RunFile>(&d).map_err(|e| e.to_string()))
    else {
        eprintln!("Runo: could not read the terminal settings.");
        return 1;
    };
    let r = &file.remote;
    let in_git = local_git(&r.local_dir, &["rev-parse", "--git-dir"], &[]).is_ok();
    let sent = if in_git {
        eprintln!("Syncing the session's files to {}…", r.conn.name);
        match r.sync_up() {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("Runo: could not sync to {}: {e}", r.conn.name);
                None
            }
        }
    } else {
        None
    };
    let script = format!("cd {} 2>/dev/null; exec bash -l", quote(&r.remote_dir));
    let code = match r.conn.shell(&r.vm, &script, &[], true) {
        Ok((mut c, _)) => c.status().map(|s| s.code().unwrap_or(0)).unwrap_or(1),
        Err(e) => {
            eprintln!("Runo: {e}");
            1
        }
    };
    if let Some(sent) = sent {
        match r.sync_down(&sent) {
            Ok(SyncDown::Kept(kept)) => eprintln!(
                "Runo: files changed here too, so they were kept; the terminal's version is saved as {kept}."
            ),
            Err(e) => eprintln!("Runo: could not sync back from {}: {e}", r.conn.name),
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

/// Sessions saved before connections existed have `cove: true`; point them at a Cove
/// connection, creating one for this machine's Cove if needed. True if anything changed.
pub fn migrate(sessions: &mut [crate::store::Session], all: &mut Vec<Connection>) -> bool {
    if !sessions.iter().any(|s| s.cove) {
        return false;
    }
    let id = match all.iter().find(|c| c.kind == Kind::Cove && !c.remote()) {
        Some(c) => c.id,
        None => {
            let c = Connection::new_cove(next_id(all));
            let id = c.id;
            all.push(c);
            id
        }
    };
    for s in sessions.iter_mut().filter(|s| s.cove) {
        s.cove = false;
        s.connection = id;
    }
    true
}

/// Delete what a session left on its connection, if it ran on one.
pub fn cleanup_session(session: &crate::store::Session) {
    let Some(conn) = get(session.connection) else {
        return;
    };
    let folder = Path::new(&session.dir)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    cleanup(&conn, &folder, session.id);
}

#[cfg(test)]
mod tests {
    use super::*;

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
            SyncDown::Updated(_)
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
    fn old_cove_sessions_get_a_cove_connection() {
        let mut sessions = vec![
            crate::store::Session {
                id: 1,
                cove: true,
                ..Default::default()
            },
            crate::store::Session {
                id: 2,
                ..Default::default()
            },
        ];
        let mut all = vec![Connection::new_ssh(4)];
        assert!(migrate(&mut sessions, &mut all));
        assert_eq!(all.len(), 2);
        assert_eq!((all[1].id, all[1].kind), (5, Kind::Cove));
        assert_eq!((sessions[0].connection, sessions[0].cove), (5, false));
        assert_eq!(sessions[1].connection, LOCAL);
        assert!(!migrate(&mut sessions, &mut all));
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
        assert_eq!(
            c.cove_ensure_args("runo-1").join(" "),
            "exec runo-1 --create --cpus 4 --memory 4096 --network off --idle-stop 30"
        );
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
