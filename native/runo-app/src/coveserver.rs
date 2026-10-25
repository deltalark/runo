//! Setting a Cove server up from Runo, given only its SSH login: Runo checks the login,
//! installs or updates Cove there, keeps an SSH tunnel to it open, pairs with it, and
//! then offers to sign the agents in.
//!
//! Cove's API stays on the server's 127.0.0.1, so nothing new is opened to the internet:
//! Runo reaches it through `ssh -L`, which it starts again whenever the tunnel is gone
//! (after sleep or a network change).

use crate::connections::{quote, Connection};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Where Cove listens on the server.
const SERVER_PORT: u16 = 7457;
/// The first local port tried for a tunnel.
const FIRST_LOCAL_PORT: u16 = 17457;
/// Folders a root install of Cove puts its command in.
const SERVER_PATH: &str = "/root/.cove/bin:/usr/local/bin:$HOME/.cove/bin:$PATH";

/// SSH options for the setup's own commands and the tunnel. Password logins can't be
/// typed here, so the key (or the SSH agent) has to work.
fn ssh_args(conn: &Connection) -> Vec<String> {
    let mut args = vec!["-o".to_string(), "BatchMode=yes".into()];
    args.extend(ssh_common(conn));
    args
}

/// The options both kinds of SSH command share.
fn ssh_common(conn: &Connection) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "ConnectTimeout=15".into(),
        "-o".into(),
        "ServerAliveInterval=30".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ];
    if conn.port != 0 {
        args.extend(["-p".into(), conn.port.to_string()]);
    }
    let key = conn.identity.trim();
    if !key.is_empty() {
        args.extend(["-i".into(), expand_home(key)]);
    }
    args
}

fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), crate::store::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

/// The local URL a tunnelled connection uses.
pub fn tunnel_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .is_ok()
}

/// A local port for a new tunnel: the usual one when it is free.
pub fn free_local_port() -> u16 {
    (FIRST_LOCAL_PORT..FIRST_LOCAL_PORT + 50)
        .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
        .unwrap_or(FIRST_LOCAL_PORT)
}

fn tunnels() -> &'static Mutex<HashMap<u64, Instant>> {
    static T: OnceLock<Mutex<HashMap<u64, Instant>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Make sure a connection that goes through SSH has its tunnel open, starting it when it
/// isn't. Connections without an SSH login pass straight through.
pub fn ensure_tunnel(conn: &Connection) -> Result<(), String> {
    let host = conn.ssh.trim();
    if host.is_empty() || conn.tunnel_port == 0 {
        return Ok(());
    }
    let port = conn.tunnel_port;
    if answers(port) {
        return Ok(());
    }
    {
        // Two callers at once start one tunnel; the second waits for it below.
        let mut started = tunnels().lock().map_err(|_| "tunnel state is poisoned")?;
        let recent = started
            .get(&conn.id)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(15));
        if !recent {
            started.insert(conn.id, Instant::now());
            let mut ssh = Command::new("ssh");
            ssh.args(["-N", "-o", "ExitOnForwardFailure=yes"])
                .args(ssh_args(conn))
                .args([
                    "-L",
                    &format!("127.0.0.1:{port}:127.0.0.1:{SERVER_PORT}"),
                    host,
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                // Its own process group: it keeps running when Runo's turn or Runo ends.
                ssh.process_group(0);
            }
            let child = ssh
                .spawn()
                .map_err(|e| format!("couldn't start ssh: {e}"))?;
            let id = conn.id;
            std::thread::spawn(move || {
                // Reap it when it ends, and let the next call start it again at once.
                let out = child.wait_with_output();
                if let Ok(mut t) = tunnels().lock() {
                    t.remove(&id);
                }
                if let Ok(out) = out {
                    let err = String::from_utf8_lossy(&out.stderr);
                    if let Some(line) = err.lines().rev().find(|l| !l.trim().is_empty()) {
                        crate::applog::warn("cove", format!("SSH tunnel ended: {line}"));
                    }
                }
            });
        }
    }
    for _ in 0..60 {
        if answers(port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!(
        "Couldn't open the SSH tunnel to {host}. Check that `ssh {host}` works in a terminal."
    ))
}

/// Where a setup is, for Settings.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Running(String),
    /// sudo on the server wants a password: the root steps run in a terminal window,
    /// where the person types it, and the setup carries on from there.
    NeedsPassword,
    Failed(String),
    /// Set up and paired: the URL and key to save on the connection.
    Done {
        url: String,
        key: String,
        port: u16,
    },
}

fn steps() -> &'static Mutex<HashMap<u64, Step>> {
    static S: OnceLock<Mutex<HashMap<u64, Step>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

pub fn step(id: u64) -> Option<Step> {
    steps().lock().ok()?.get(&id).cloned()
}

fn set_step(id: u64, step: Step) {
    if let Ok(mut s) = steps().lock() {
        s.insert(id, step);
    }
    crate::caps::mark_updated();
}

pub fn clear_step(id: u64) {
    if let Ok(mut s) = steps().lock() {
        s.remove(&id);
    }
}

pub fn running(id: u64) -> bool {
    matches!(step(id), Some(Step::Running(_)))
}

/// What the first look at the server found.
#[derive(Debug, Default, PartialEq)]
struct Probe {
    root: bool,
    sudo: bool,
    /// Cove answers there, with agent logins.
    current: bool,
    /// Cove answers there, but without agent logins.
    old: bool,
}

const PROBE: &str = r#"echo "uid=$(id -u)"
if sudo -n true 2>/dev/null; then echo sudo=yes; else echo sudo=no; fi
h="$(curl -fsS --max-time 3 http://127.0.0.1:7457/v1/health 2>/dev/null)"
case "$h" in *'"logins"'*) echo cove=current ;; *'"ok"'*) echo cove=old ;; *) echo cove=none ;; esac"#;

fn parse_probe(text: &str) -> Probe {
    let has = |l: &str| text.lines().any(|line| line.trim() == l);
    Probe {
        root: has("uid=0"),
        sudo: has("sudo=yes"),
        current: has("cove=current"),
        old: has("cove=old"),
    }
}

/// Run `script` on the server with bash.
fn run(conn: &Connection, script: &str) -> Result<String, String> {
    let out = Command::new("ssh")
        .args(ssh_args(conn))
        .args([
            conn.ssh.trim(),
            "--",
            &format!("bash -lc {}", quote(script)),
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run ssh: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        return Ok(stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err
        .lines()
        .chain(stdout.lines())
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    Err(if out.status.code() == Some(255) {
        // ssh's own failure: no login, unknown host, no route.
        format!("Couldn't sign in to {} over SSH: {line}", conn.ssh.trim())
    } else if line.is_empty() {
        format!("The server said {}", out.status)
    } else {
        line
    })
}

/// How to run a command as root there: directly, or with sudo.
fn as_root(probe: &Probe) -> &'static str {
    if probe.root {
        ""
    } else {
        "sudo -H "
    }
}

/// The root steps: install or update Cove when it isn't current, then make a pairing
/// code. `pair_to` saves the code to a file instead of printing it.
fn root_script(probe: &Probe, pair_to: Option<&str>) -> String {
    let root = as_root(probe);
    let name = format!("Runo on {}", crate::cove::host_name());
    let mut script = String::from("set -e\n");
    if !probe.current {
        script.push_str(&format!(
            "f=$(mktemp)\ncurl -fsSL {} -o \"$f\"\n{root}sh \"$f\" </dev/null\nrm -f \"$f\"\n",
            crate::cove::INSTALL_URL
        ));
    }
    // A fresh install's server takes a moment to answer.
    script.push_str("for i in $(seq 1 30); do curl -fsS --max-time 2 http://127.0.0.1:7457/v1/health >/dev/null 2>&1 && break; sleep 1; done\n");
    script.push_str(&format!(
        "export PATH=\"{SERVER_PATH}\"\n{root}env PATH=\"$PATH\" cove --json pair --name {}",
        quote(&name)
    ));
    if let Some(file) = pair_to {
        script.push_str(&format!(" > {file}.tmp && mv {file}.tmp {file}\n"));
    }
    script
}

/// Where the terminal's root steps leave the pairing code for Runo, on the server.
const PAIR_FILE: &str = "$HOME/.runo-cove-pair.json";

/// Set the connection's server up in the background: check the SSH login, install or
/// update Cove there, open the tunnel and pair. Progress shows in [`step`]; `done` runs
/// when it stops, finished or waiting for a password.
pub fn set_up(conn: Connection, done: impl FnOnce() + Send + 'static) {
    if running(conn.id) {
        return;
    }
    set_step(conn.id, Step::Running("Signing in over SSH…".into()));
    std::thread::spawn(move || {
        let id = conn.id;
        let result = set_up_now(conn);
        set_step(
            id,
            match result {
                Ok(step) => step,
                Err(e) => Step::Failed(e),
            },
        );
        done();
    });
}

fn set_up_now(conn: Connection) -> Result<Step, String> {
    let id = conn.id;
    if conn.ssh.trim().is_empty() {
        return Err("Fill in the server's SSH login first, such as ubuntu@203.0.113.7.".into());
    }
    if crate::cove::binary().is_none() {
        return Err(
            "Runo talks to Cove servers with the cove command, which this Mac doesn't have yet: press Install under Cove on this machine first, then Set up again."
                .into(),
        );
    }
    let probe = parse_probe(&run(&conn, PROBE)?);
    if !probe.root && !probe.sudo {
        return Ok(Step::NeedsPassword);
    }
    set_step(
        id,
        Step::Running(if probe.current {
            "Cove is up to date there. Pairing…".into()
        } else if probe.old {
            "Updating Cove on the server (a few minutes the first time)…".into()
        } else {
            "Installing Cove on the server (a few minutes the first time)…".into()
        }),
    );
    let out = run(&conn, &root_script(&probe, None))
        .map_err(|e| format!("Setting Cove up on the server failed: {e}"))?;
    finish(conn, &out)
}

/// Take the pairing code the root steps printed, open the tunnel and pair.
fn finish(mut conn: Connection, paired: &str) -> Result<Step, String> {
    let id = conn.id;
    // The code is the JSON object at the end of the output, after any installer lines.
    let code = paired
        .rfind("{\n")
        .or_else(|| paired.rfind('{'))
        .and_then(|at| serde_json::from_str::<serde_json::Value>(paired[at..].trim()).ok())
        .and_then(|v| v["code"].as_str().map(String::from))
        .ok_or("The server didn't give a pairing code.")?;
    set_step(id, Step::Running("Opening a tunnel to the server…".into()));
    if conn.tunnel_port == 0 || (!answers(conn.tunnel_port) && !local_port_free(conn.tunnel_port)) {
        conn.tunnel_port = free_local_port();
    }
    let url = tunnel_url(conn.tunnel_port);
    conn.url = url.clone();
    ensure_tunnel(&conn)?;
    set_step(id, Step::Running("Pairing Runo with the server…".into()));
    let key = crate::connections::pair(&conn, &code)?;
    Ok(Step::Done {
        url,
        key,
        port: conn.tunnel_port,
    })
}

/// For a server whose sudo wants a password: run the root steps in a terminal window
/// (where the person types it), then wait for the pairing code they leave and finish.
pub fn set_up_in_terminal(
    conn: Connection,
    done: impl FnOnce() + Send + 'static,
) -> Result<(), String> {
    let probe = parse_probe(&run(&conn, PROBE)?);
    let script = format!(
        "{}echo; echo 'Done. You can close this window; Runo carries on.'",
        root_script(&probe, Some(PAIR_FILE))
    );
    // A terminal of its own, so sudo can ask for the password.
    let mut cleaned = vec!["ssh".to_string(), "-t".into()];
    cleaned.extend(ssh_common(&conn).iter().map(|a| quote(a)));
    cleaned.push(quote(conn.ssh.trim()));
    cleaned.push("--".into());
    cleaned.push(quote(&format!("bash -lc {}", quote(&script))));
    crate::install::open_in_terminal(&cleaned.join(" "))?;
    set_step(
        conn.id,
        Step::Running(
            "Waiting for the terminal window: type the server's sudo password there.".into(),
        ),
    );
    std::thread::spawn(move || {
        let id = conn.id;
        let deadline = Instant::now() + Duration::from_secs(20 * 60);
        let result = loop {
            if Instant::now() > deadline {
                break Err("The terminal window didn't finish within 20 minutes. Press Set up to try again.".into());
            }
            std::thread::sleep(Duration::from_secs(3));
            let read = format!("cat {PAIR_FILE} 2>/dev/null && rm -f {PAIR_FILE}");
            match run(&conn, &read) {
                Ok(out) if out.contains("\"code\"") => break finish(conn, &out),
                _ => continue,
            }
        };
        set_step(
            id,
            match result {
                Ok(step) => step,
                Err(e) => Step::Failed(e),
            },
        );
        done();
    });
    Ok(())
}

fn local_port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_says_what_to_do() {
        let p = parse_probe("uid=1000\nsudo=no\ncove=old\n");
        assert_eq!(
            p,
            Probe {
                root: false,
                sudo: false,
                current: false,
                old: true
            }
        );
        assert_eq!(as_root(&p), "sudo -H ");
        let root = parse_probe("uid=0\nsudo=yes\ncove=current");
        assert!(root.root && root.current);
        assert_eq!(as_root(&root), "");
        // Current: no install, just a code.
        let script = root_script(&root, None);
        assert!(!script.contains("install.sh") && script.contains("cove --json pair"));
        let script = root_script(&p, Some(PAIR_FILE));
        assert!(script.contains("sudo -H sh"), "{script}");
        assert!(
            script.contains("> $HOME/.runo-cove-pair.json.tmp"),
            "{script}"
        );
    }

    #[test]
    fn the_probe_script_reads_cove_health() {
        // The probe's own output, run here, parses (no Cove answers on this machine's
        // 7457 in tests unless one is running, which is fine either way).
        let out = Command::new("bash").args(["-c", PROBE]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("uid="), "{text}");
        assert!(text.contains("cove="), "{text}");
    }

    #[test]
    fn tunnel_options_use_the_key_and_port() {
        let mut c = Connection::new_cove(1);
        c.ssh = "ubuntu@203.0.113.7".into();
        c.identity = "/k/vps".into();
        c.port = 2222;
        let args = ssh_args(&c).join(" ");
        assert!(args.contains("BatchMode=yes"));
        assert!(args.contains("-p 2222"));
        assert!(args.ends_with("-i /k/vps"));
        assert_eq!(tunnel_url(17457), "http://127.0.0.1:17457");
        // No SSH login: nothing to open.
        assert!(ensure_tunnel(&Connection::new_cove(2)).is_ok());
    }
}

#[cfg(test)]
mod e2e {
    /// Sets a real server up over SSH. Needs a server with Cove (or one Runo may install
    /// it on): `COVE_SSH_E2E=root@host COVE_SSH_PORT=22 COVE_SSH_KEY=~/.ssh/key cargo test
    /// -p runo coveserver`.
    #[test]
    fn a_server_is_set_up_over_ssh() {
        let Ok(host) = std::env::var("COVE_SSH_E2E") else {
            return;
        };
        let mut conn = crate::connections::Connection::new_cove(7);
        conn.ssh = host;
        conn.port = std::env::var("COVE_SSH_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        conn.identity = std::env::var("COVE_SSH_KEY").unwrap_or_default();
        let step = super::set_up_now(conn.clone()).unwrap();
        let super::Step::Done { url, key, port } = step else {
            panic!("not done: {step:?}");
        };
        assert!(key.starts_with("cove_"));
        conn.url = url;
        conn.api_key = key;
        conn.tunnel_port = port;
        // Through the tunnel Runo opened.
        let logins = crate::cove::logins_now(&conn).unwrap();
        assert!(logins.iter().any(|l| l.agent == "claude"));
    }
}
