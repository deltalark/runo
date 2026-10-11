//! Launches provider CLIs and shell commands, streaming their output as events.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::settings::AgentOptions;

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub bin: &'static str,
    pub mark: &'static str,
    pub color: u32,
}

pub const PROVIDERS: [Provider; 10] = [
    Provider {
        id: "claude",
        name: "Claude Code",
        bin: "claude",
        mark: "✳",
        color: 0xd9633f,
    },
    Provider {
        id: "codex",
        name: "Codex",
        bin: "codex",
        mark: "◆",
        color: 0x1767e8,
    },
    Provider {
        id: "opencode",
        name: "OpenCode",
        bin: "opencode",
        mark: ">_",
        color: 0x258a5b,
    },
    Provider {
        id: "cursor",
        name: "Cursor",
        bin: "cursor-agent",
        mark: "▲",
        color: 0x7c7c86,
    },
    Provider {
        id: "gemini",
        name: "Gemini",
        bin: "gemini",
        mark: "✦",
        color: 0x4f7cf7,
    },
    Provider {
        id: "copilot",
        name: "Copilot",
        bin: "copilot",
        mark: "◎",
        color: 0x8957e5,
    },
    Provider {
        id: "amp",
        name: "Amp",
        bin: "amp",
        mark: "∿",
        color: 0xe5484d,
    },
    Provider {
        id: "droid",
        name: "Droid",
        bin: "droid",
        mark: "◈",
        color: 0xf27b2a,
    },
    Provider {
        id: "qwen",
        name: "Qwen Code",
        bin: "qwen",
        mark: "◇",
        color: 0x615ced,
    },
    Provider {
        id: "grok",
        name: "Grok",
        bin: "grok",
        mark: "𝕏",
        color: 0x9a9aa3,
    },
];

/// Whether the CLI keeps its own conversation that a follow-up can resume. Others get
/// the recent conversation written into each follow-up prompt instead.
pub fn resumes(provider: &str) -> bool {
    matches!(provider, "claude" | "codex" | "opencode")
}

pub fn provider(id: &str) -> &'static Provider {
    PROVIDERS
        .iter()
        .find(|p| p.id == id)
        .unwrap_or(&PROVIDERS[0])
}

/// Whether the agent can run: its configured executable exists, or with none
/// configured, its default command is on PATH.
pub fn available(bin: &str, configured: &str) -> bool {
    let configured = configured.trim();
    if configured.is_empty() {
        installed(bin)
    } else if configured.contains(std::path::MAIN_SEPARATOR) {
        std::path::Path::new(configured).is_file()
    } else {
        installed(configured)
    }
}

pub fn installed(bin: &str) -> bool {
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                suffixes
                    .iter()
                    .any(|s| dir.join(format!("{bin}{s}")).is_file())
            })
        })
        .unwrap_or(false)
}

/// Shell-style splitting with single/double quotes, for the "extra arguments" setting.
pub fn split_args(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut quote, mut any) = (String::new(), None::<char>, false);
    for c in input.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn list(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Display name for Runo's provider-neutral permission levels (Settings > Chat), which
/// each agent maps to its closest mode when launched.
pub fn generic_mode_label(mode: &str) -> Option<&'static str> {
    match mode {
        "edit" => Some("Auto edit"),
        "plan" => Some("Plan only"),
        "full" => Some("Full access"),
        _ => None,
    }
}

pub struct RunSpec<'a> {
    pub provider: &'a str,
    pub prompt: &'a str,
    pub resume: bool,
    /// The session's selections: model, effort, tier, mode, approval, agent.
    pub sel: &'a std::collections::HashMap<String, String>,
    pub opts: &'a AgentOptions,
    /// Whether the working directory is a git repository.
    pub in_git: bool,
    /// Claude Code conversation id for this session; empty for sessions saved before ids.
    pub session_id: &'a str,
    /// Image files attached to the prompt.
    pub images: &'a [String],
    /// Runo's memory server for this folder: (Runo's executable, the session folder).
    pub memory: Option<(&'a str, &'a str)>,
}

/// A random version-4 UUID, for naming a Claude Code conversation up front.
pub fn new_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        hasher.write_usize(i);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn provider_args(spec: &RunSpec) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let opts = spec.opts;
    let get = |id: &str| spec.sel.get(id).cloned().unwrap_or_default();
    let or = |a: String, b: &str| if a.is_empty() { b.to_string() } else { a };
    let model = or(get("model"), &opts.model_override);
    let effort = or(get("effort"), &opts.effort);
    let mode = get("mode");
    let mut args: Vec<String> = Vec::new();
    match spec.provider {
        "codex" => {
            args.push(s("exec"));
            let sandbox = match mode.as_str() {
                "" | "edit" => "workspace-write".to_string(),
                "plan" => s("read-only"),
                "full" => s("danger-full-access"),
                other => other.to_string(),
            };
            args.extend([s("--sandbox"), sandbox]);
            if !model.is_empty() {
                args.extend([s("-m"), model]);
            }
            if !effort.is_empty() {
                args.extend([s("-c"), format!("model_reasoning_effort=\"{effort}\"")]);
            }
            let tier = get("tier");
            if !tier.is_empty() {
                args.extend([s("-c"), format!("service_tier=\"{tier}\"")]);
            }
            let approval = get("approval");
            if !approval.is_empty() {
                args.extend([s("-c"), format!("approval_policy=\"{approval}\"")]);
            }
            if !opts.profile.is_empty() {
                args.extend([s("-p"), opts.profile.clone()]);
            }
            for dir in list(&opts.add_dirs) {
                args.extend([s("--add-dir"), dir]);
            }
            if !spec.in_git {
                args.push(s("--skip-git-repo-check"));
            }
            if let Some((exe, dir)) = spec.memory {
                args.extend([
                    s("-c"),
                    format!("mcp_servers.{MEMORY_SERVER}.command={}", toml_string(exe)),
                    s("-c"),
                    format!(
                        "mcp_servers.{MEMORY_SERVER}.args=[\"--mcp-memory\", {}]",
                        toml_string(dir)
                    ),
                ]);
            }
            args.extend(split_args(&opts.extra_args));
            for image in spec.images {
                args.extend([s("-i"), image.clone()]);
            }
            if spec.resume {
                // A known conversation id resumes exactly that one; `--last` is the
                // newest in the folder, which may belong to another session.
                if spec.session_id.is_empty() {
                    args.extend([s("resume"), s("--last")]);
                } else {
                    args.extend([s("resume"), spec.session_id.to_string()]);
                }
            }
            args.push(spec.prompt.to_string());
        }
        "cursor" => {
            args.extend([s("-p"), s("--output-format"), s("text")]);
            if matches!(mode.as_str(), "" | "edit" | "full") {
                args.push(s("--force"));
            }
            if !model.is_empty() {
                args.extend([s("-m"), model]);
            }
            args.extend(split_args(&opts.extra_args));
            args.push(with_images(spec));
        }
        "gemini" | "qwen" => {
            let approval = match mode.as_str() {
                "plan" => "default",
                "full" => "yolo",
                _ => "auto_edit",
            };
            args.extend([s("--approval-mode"), s(approval)]);
            if !model.is_empty() {
                args.extend([s("-m"), model]);
            }
            for dir in list(&opts.add_dirs) {
                args.extend([s("--include-directories"), dir]);
            }
            args.extend(split_args(&opts.extra_args));
            args.extend([s("-p"), with_images(spec)]);
        }
        "copilot" => {
            if mode != "plan" {
                args.push(s("--allow-all-tools"));
            }
            if !model.is_empty() {
                args.extend([s("--model"), model]);
            }
            for dir in list(&opts.add_dirs) {
                args.extend([s("--add-dir"), dir]);
            }
            args.extend(split_args(&opts.extra_args));
            args.extend([s("-p"), with_images(spec)]);
        }
        "amp" => {
            if mode != "plan" {
                args.push(s("--dangerously-allow-all"));
            }
            args.extend(split_args(&opts.extra_args));
            args.extend([s("-x"), with_images(spec)]);
        }
        "droid" => {
            args.push(s("exec"));
            match mode.as_str() {
                "plan" => {}
                "full" => args.extend([s("--auto"), s("high")]),
                _ => args.extend([s("--auto"), s("medium")]),
            }
            if !model.is_empty() {
                args.extend([s("-m"), model]);
            }
            args.extend(split_args(&opts.extra_args));
            args.push(with_images(spec));
        }
        "grok" => {
            if !model.is_empty() {
                args.extend([s("-m"), model]);
            }
            args.extend(split_args(&opts.extra_args));
            args.extend([s("-p"), with_images(spec)]);
        }
        "opencode" => {
            args.push(s("run"));
            if spec.resume {
                args.push(s("--continue"));
            }
            if !model.is_empty() {
                args.extend([s("-m"), model]);
            }
            let agent = if !get("agent").is_empty() {
                Some(get("agent"))
            } else if !opts.sub_agent.is_empty() {
                Some(opts.sub_agent.clone())
            } else if mode == "plan" {
                Some(s("plan"))
            } else {
                None
            };
            if let Some(agent) = agent {
                args.extend([s("--agent"), agent]);
            }
            let variant = or(effort, &opts.variant);
            if !variant.is_empty() {
                args.extend([s("--variant"), variant]);
            }
            if opts.thinking {
                args.push(s("--thinking"));
            }
            if mode == "full" || mode == "auto" {
                args.push(s("--auto"));
            }
            args.extend(split_args(&opts.extra_args));
            for image in spec.images {
                args.extend([s("-f"), image.clone()]);
            }
            args.push(spec.prompt.to_string());
        }
        _ => {
            let permission = match mode.as_str() {
                "" | "edit" => "acceptEdits",
                "full" => "bypassPermissions",
                other => other,
            };
            // Structured output lets the app show each tool step live instead of a wall of text.
            args.extend([
                s("-p"),
                s("--output-format"),
                s("stream-json"),
                s("--verbose"),
                s("--include-partial-messages"),
                s("--permission-mode"),
                s(permission),
            ]);
            if !model.is_empty() {
                args.extend([s("--model"), model]);
            }
            if !effort.is_empty() {
                args.extend([s("--effort"), effort]);
            }
            if !opts.append_prompt.trim().is_empty() {
                args.extend([s("--append-system-prompt"), opts.append_prompt.clone()]);
            }
            let mut allowed = list(&opts.allowed_tools);
            if let Some((exe, dir)) = spec.memory {
                let config = serde_json::json!({
                    "mcpServers": { MEMORY_SERVER: { "command": exe, "args": ["--mcp-memory", dir] } }
                });
                args.extend([s("--mcp-config"), config.to_string()]);
                // Reading and searching memory needs no approval.
                allowed.push(format!("mcp__{MEMORY_SERVER}"));
            }
            if !allowed.is_empty() {
                args.extend([s("--allowed-tools"), allowed.join(",")]);
            }
            let denied = list(&opts.disallowed_tools);
            if !denied.is_empty() {
                args.extend([s("--disallowed-tools"), denied.join(",")]);
            }
            for dir in list(&opts.add_dirs) {
                args.extend([s("--add-dir"), dir]);
            }
            if !opts.fallback_model.is_empty() {
                args.extend([s("--fallback-model"), opts.fallback_model.clone()]);
            }
            args.extend(split_args(&opts.extra_args));
            match (spec.resume, spec.session_id.is_empty()) {
                // Pin the conversation to this session: `--continue` would pick up whatever
                // conversation ran last in the folder, which may belong to another session.
                (true, false) => args.extend([s("--resume"), spec.session_id.to_string()]),
                (false, false) => args.extend([s("--session-id"), spec.session_id.to_string()]),
                (true, true) => args.push(s("--continue")),
                (false, true) => {}
            }
            // Several Claude flags are variadic; `--` keeps them from swallowing the prompt.
            args.push(s("--"));
            args.push(with_images(spec));
        }
    }
    args
}

/// Name of Runo's memory MCP server in the agents' config.
pub const MEMORY_SERVER: &str = "runo_memory";

/// A TOML basic string.
fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The prompt with attached image paths listed, for CLIs that read images from disk.
fn with_images(spec: &RunSpec) -> String {
    if spec.images.is_empty() {
        return spec.prompt.to_string();
    }
    let mut prompt = spec.prompt.to_string();
    prompt.push_str("\n\nAttached images (read them):");
    for image in spec.images {
        prompt.push_str(&format!("\n- {image}"));
    }
    prompt
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Session(u64),
    Terminal(u64),
}

pub enum Event {
    Line(Target, String),
    /// stderr of an agent run: kept out of the conversation unless the run fails.
    Err(Target, String),
    Done(Target, bool),
    /// Result of a one-off background command, by the key it was started with.
    Output(String, Result<String, String>),
    /// An interactive shell has new output, or exited.
    Pty(u64),
}

/// Run a command to completion on a background thread and send its stdout back.
pub fn oneshot(key: String, mut command: Command, dir: &str, tx: Sender<Event>) {
    command.current_dir(dir).stdin(Stdio::null());
    std::thread::spawn(move || {
        let result = match command.output() {
            Ok(out) if out.status.success() => {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                Err(if err.is_empty() {
                    String::from_utf8_lossy(&out.stdout).trim().to_string()
                } else {
                    err
                })
            }
            Err(error) => Err(format!(
                "could not start {:?}: {error}",
                command.get_program()
            )),
        };
        let _ = tx.send(Event::Output(key, result));
    });
}

/// A plain, one-shot run of an agent CLI with a prompt, for small jobs such as
/// writing a commit message. Read-only where the CLI supports it.
pub fn ask_command(provider: &str, configured_bin: &str, prompt: &str) -> Command {
    let bin = if configured_bin.trim().is_empty() {
        self::provider(provider).bin
    } else {
        configured_bin.trim()
    };
    let mut command = Command::new(bin);
    match provider {
        "codex" => {
            command.args([
                "exec",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                prompt,
            ]);
        }
        "opencode" => {
            command.args(["run", prompt]);
        }
        "cursor" => {
            command.args(["-p", "--output-format", "text", prompt]);
        }
        "amp" => {
            command.args(["-x", prompt]);
        }
        "droid" => {
            command.args(["exec", prompt]);
        }
        "claude" => {
            command.args(["-p", "--permission-mode", "plan", "--", prompt]);
        }
        _ => {
            command.args(["-p", prompt]);
        }
    }
    command
}

#[derive(Clone)]
pub struct Handle(Arc<Mutex<Child>>);

impl Handle {
    pub fn kill(&self) {
        if let Ok(mut child) = self.0.lock() {
            let _ = child.kill();
        }
    }
}

pub fn run_agent(
    target: Target,
    spec: &RunSpec,
    dir: &str,
    tx: Sender<Event>,
) -> Result<Handle, String> {
    let bin = if spec.opts.binary.trim().is_empty() {
        self::provider(spec.provider).bin
    } else {
        spec.opts.binary.trim()
    };
    let mut command = Command::new(bin);
    command.args(provider_args(spec));
    for line in spec.opts.env.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if !key.trim().is_empty() {
                command.env(key.trim(), value.trim());
            }
        }
    }
    spawn(target, command, dir, tx)
}

pub fn run_shell(
    target: Target,
    line: &str,
    dir: &str,
    tx: Sender<Event>,
) -> Result<Handle, String> {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", line]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", line]);
        c
    };
    command.env("TERM", "dumb");
    spawn(target, command, dir, tx)
}

fn pump<R: Read + Send + 'static>(reader: R, target: Target, is_err: bool, tx: Sender<Event>) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let event = if is_err && matches!(target, Target::Session(_)) {
                Event::Err(target, line)
            } else {
                Event::Line(target, line)
            };
            if tx.send(event).is_err() {
                break;
            }
        }
    });
}

fn spawn(
    target: Target,
    mut command: Command,
    dir: &str,
    tx: Sender<Event>,
) -> Result<Handle, String> {
    let mut child = command
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start {:?}: {error}", command.get_program()))?;
    if let Some(out) = child.stdout.take() {
        pump(out, target, false, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        pump(err, target, true, tx.clone());
    }
    let child = Arc::new(Mutex::new(child));
    let waiter = child.clone();
    std::thread::spawn(move || loop {
        let finished = waiter
            .lock()
            .ok()
            .and_then(|mut c| c.try_wait().ok().flatten());
        if let Some(status) = finished {
            // Let the reader threads flush the final lines first.
            std::thread::sleep(Duration::from_millis(150));
            let _ = tx.send(Event::Done(target, status.success()));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    });
    Ok(Handle(child))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_streams_lines_then_done() {
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = std::env::temp_dir();
        run_shell(
            Target::Terminal(1),
            "echo hi && echo err 1>&2",
            dir.to_str().unwrap(),
            tx,
        )
        .unwrap();
        let mut lines = vec![];
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::Line(_, l) | Event::Err(_, l) => lines.push(l),
                Event::Done(_, ok) => {
                    assert!(ok);
                    break;
                }
                Event::Output(..) | Event::Pty(_) => {}
            }
        }
        lines.sort();
        assert_eq!(lines, ["err", "hi"]);
    }
}

#[cfg(test)]
mod arg_tests {
    use super::*;
    use std::collections::HashMap;

    fn sel(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn args(
        provider: &str,
        resume: bool,
        selections: &[(&str, &str)],
        opts: &AgentOptions,
        in_git: bool,
    ) -> Vec<String> {
        let sel = sel(selections);
        provider_args(&RunSpec {
            provider,
            prompt: "hi",
            resume,
            sel: &sel,
            opts,
            in_git,
            session_id: "",
            images: &[],
            memory: None,
        })
    }

    #[test]
    fn codex_resumes_a_known_conversation_by_id() {
        let sel = HashMap::new();
        let o = AgentOptions::default();
        let run = |session_id| {
            provider_args(&RunSpec {
                provider: "codex",
                prompt: "hi",
                resume: true,
                sel: &sel,
                opts: &o,
                in_git: true,
                session_id,
                images: &[],
                memory: None,
            })
        };
        assert!(run("0199-id").ends_with(&["resume".into(), "0199-id".into(), "hi".into()]));
        assert!(run("").ends_with(&["resume".into(), "--last".into(), "hi".into()]));
    }

    #[test]
    fn claude_pins_follow_ups_to_its_own_conversation() {
        let sel = HashMap::new();
        let o = AgentOptions::default();
        let spec = |resume, session_id| RunSpec {
            provider: "claude",
            prompt: "hi",
            resume,
            sel: &sel,
            opts: &o,
            in_git: true,
            session_id,
            images: &[],
            memory: None,
        };
        let id = "0b7c6a52-3f7e-4f3a-9d61-2a1c9b3e8f10";
        let first = provider_args(&spec(false, id));
        assert!(first.windows(2).any(|w| w == ["--session-id", id]));
        assert!(!first.contains(&"--continue".to_string()));
        let next = provider_args(&spec(true, id));
        assert!(next.windows(2).any(|w| w == ["--resume", id]));
        assert!(!next.contains(&"--continue".to_string()));
        // The prompt always comes last, after `--`.
        assert_eq!(&next[next.len() - 2..], ["--", "hi"]);
    }

    #[test]
    fn uuids_are_v4_and_distinct() {
        let a = new_uuid();
        let b = new_uuid();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!(matches!(
            parts[3].chars().next(),
            Some('8' | '9' | 'a' | 'b')
        ));
        assert!(a.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
    }

    #[test]
    fn generic_modes_have_labels() {
        assert_eq!(generic_mode_label("edit"), Some("Auto edit"));
        assert_eq!(generic_mode_label("acceptEdits"), None);
    }

    #[test]
    fn provider_lookup_falls_back_to_claude() {
        assert_eq!(provider("codex").name, "Codex");
        assert_eq!(provider("nope").id, "claude");
    }

    #[cfg(unix)]
    #[test]
    fn agent_gets_env_args_and_reports_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("runo-agent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-agent");
        std::fs::write(
            &script,
            "#!/bin/sh\necho \"env=$RUNO_T\"\necho \"args=$*\"\necho oops 1>&2\nexit 3\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let sel = sel(&[("model", "opus")]);
        let o = AgentOptions {
            binary: script.to_string_lossy().into_owned(),
            env: " RUNO_T = from-env \nNOT_A_PAIR\n=ignored".into(),
            ..Default::default()
        };
        let spec = RunSpec {
            provider: "claude",
            prompt: "do it",
            resume: false,
            sel: &sel,
            opts: &o,
            in_git: true,
            session_id: "",
            images: &[],
            memory: None,
        };
        run_agent(Target::Session(5), &spec, dir.to_str().unwrap(), tx).unwrap();
        let (mut out, mut err) = (vec![], vec![]);
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::Line(_, l) => out.push(l),
                Event::Err(_, l) => err.push(l),
                Event::Done(target, ok) => {
                    assert!(target == Target::Session(5));
                    assert!(!ok);
                    break;
                }
                Event::Output(..) | Event::Pty(_) => unreachable!(),
            }
        }
        assert_eq!(out[0], "env=from-env");
        assert_eq!(
            out[1],
            "args=-p --output-format stream-json --verbose --include-partial-messages --permission-mode acceptEdits --model opus -- do it"
        );
        // stderr from an agent stays out of the conversation stream.
        assert_eq!(err, ["oops"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_binary_is_an_error_not_a_panic() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let sel = HashMap::new();
        let o = AgentOptions {
            binary: "/definitely/not/a/runo/binary".into(),
            ..Default::default()
        };
        let spec = RunSpec {
            provider: "claude",
            prompt: "hi",
            resume: false,
            sel: &sel,
            opts: &o,
            in_git: false,
            session_id: "",
            images: &[],
            memory: None,
        };
        let error = run_agent(Target::Session(1), &spec, "/", tx).err().unwrap();
        assert!(error.contains("could not start"));
    }

    #[test]
    fn kill_stops_a_running_command() {
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = std::env::temp_dir();
        let handle = run_shell(Target::Terminal(7), "sleep 30", dir.to_str().unwrap(), tx).unwrap();
        handle.kill();
        loop {
            if let Event::Done(_, ok) = rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                assert!(!ok);
                break;
            }
        }
    }

    #[test]
    fn configured_binaries_count_as_available() {
        assert!(available("definitely-not-a-real-agent", "/bin/sh"));
        assert!(!available("sh", "/no/such/agent"));
        assert!(available("definitely-not-a-real-agent", "sh"));
        assert!(!available("definitely-not-a-real-agent", " "));
    }

    #[test]
    fn claude_args() {
        let o = AgentOptions::default();
        assert_eq!(
            args(
                "claude",
                true,
                &[("mode", "plan"), ("model", "opus")],
                &o,
                true
            ),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--permission-mode",
                "plan",
                "--model",
                "opus",
                "--continue",
                "--",
                "hi"
            ]
        );
        assert_eq!(
            args("claude", false, &[], &o, true),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--permission-mode",
                "acceptEdits",
                "--",
                "hi"
            ]
        );
        let o = AgentOptions {
            effort: "high".into(),
            allowed_tools: "Read, Edit".into(),
            add_dirs: "/a,/b".into(),
            extra_args: "--verbose".into(),
            ..Default::default()
        };
        assert_eq!(
            args(
                "claude",
                false,
                &[("mode", "dontAsk"), ("effort", "max")],
                &o,
                true
            ),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--permission-mode",
                "dontAsk",
                "--effort",
                "max",
                "--allowed-tools",
                "Read,Edit",
                "--add-dir",
                "/a",
                "--add-dir",
                "/b",
                "--verbose",
                "--",
                "hi"
            ]
        );
    }

    #[test]
    fn codex_args() {
        let o = AgentOptions::default();
        assert_eq!(
            args("codex", false, &[], &o, true),
            ["exec", "--sandbox", "workspace-write", "hi"]
        );
        assert_eq!(
            args("codex", false, &[("mode", "full")], &o, true),
            ["exec", "--sandbox", "danger-full-access", "hi"]
        );
        let o = AgentOptions {
            profile: "work".into(),
            ..Default::default()
        };
        assert_eq!(
            args(
                "codex",
                true,
                &[
                    ("mode", "read-only"),
                    ("model", "gpt-5.6-sol"),
                    ("effort", "high"),
                    ("tier", "priority"),
                    ("approval", "never")
                ],
                &o,
                false
            ),
            [
                "exec",
                "--sandbox",
                "read-only",
                "-m",
                "gpt-5.6-sol",
                "-c",
                "model_reasoning_effort=\"high\"",
                "-c",
                "service_tier=\"priority\"",
                "-c",
                "approval_policy=\"never\"",
                "-p",
                "work",
                "--skip-git-repo-check",
                "resume",
                "--last",
                "hi"
            ]
        );
    }

    #[test]
    fn opencode_args() {
        let o = AgentOptions::default();
        assert_eq!(args("opencode", false, &[], &o, true), ["run", "hi"]);
        assert_eq!(
            args("opencode", true, &[("mode", "plan")], &o, true),
            ["run", "--continue", "--agent", "plan", "hi"]
        );
        let o = AgentOptions {
            variant: "high".into(),
            thinking: true,
            ..Default::default()
        };
        assert_eq!(
            args(
                "opencode",
                false,
                &[
                    ("mode", "auto"),
                    ("model", "opencode/big-pickle"),
                    ("agent", "build")
                ],
                &o,
                true
            ),
            [
                "run",
                "-m",
                "opencode/big-pickle",
                "--agent",
                "build",
                "--variant",
                "high",
                "--thinking",
                "--auto",
                "hi"
            ]
        );
    }

    #[test]
    fn splits_quoted_args() {
        assert_eq!(
            split_args(r#"--foo "a b" 'c d' e"#),
            ["--foo", "a b", "c d", "e"]
        );
        assert!(split_args("   ").is_empty());
    }
}
