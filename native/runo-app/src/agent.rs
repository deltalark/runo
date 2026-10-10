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

pub const PROVIDERS: [Provider; 3] = [
    Provider { id: "claude", name: "Claude Code", bin: "claude", mark: "✳", color: 0xd9633f },
    Provider { id: "codex", name: "Codex", bin: "codex", mark: "◆", color: 0x1767e8 },
    Provider { id: "opencode", name: "OpenCode", bin: "opencode", mark: ">_", color: 0x258a5b },
];

pub fn provider(id: &str) -> &'static Provider {
    PROVIDERS.iter().find(|p| p.id == id).unwrap_or(&PROVIDERS[0])
}

pub fn installed(bin: &str) -> bool {
    let suffixes: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                suffixes.iter().any(|s| dir.join(format!("{bin}{s}")).is_file())
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
    input.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
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
            args.extend(split_args(&opts.extra_args));
            if spec.resume {
                args.extend([s("resume"), s("--last")]);
            }
            args.push(spec.prompt.to_string());
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
            args.push(spec.prompt.to_string());
        }
        _ => {
            let permission = match mode.as_str() {
                "" | "edit" => "acceptEdits",
                "full" => "bypassPermissions",
                other => other,
            };
            args.extend([s("-p"), s("--permission-mode"), s(permission)]);
            if !model.is_empty() {
                args.extend([s("--model"), model]);
            }
            if !effort.is_empty() {
                args.extend([s("--effort"), effort]);
            }
            if !opts.append_prompt.trim().is_empty() {
                args.extend([s("--append-system-prompt"), opts.append_prompt.clone()]);
            }
            let allowed = list(&opts.allowed_tools);
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
            if spec.resume {
                args.push(s("--continue"));
            }
            // Several Claude flags are variadic; `--` keeps them from swallowing the prompt.
            args.push(s("--"));
            args.push(spec.prompt.to_string());
        }
    }
    args
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

pub fn run_agent(target: Target, spec: &RunSpec, dir: &str, tx: Sender<Event>) -> Result<Handle, String> {
    let bin = if spec.opts.binary.trim().is_empty() { self::provider(spec.provider).bin } else { spec.opts.binary.trim() };
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

pub fn run_shell(target: Target, line: &str, dir: &str, tx: Sender<Event>) -> Result<Handle, String> {
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

fn spawn(target: Target, mut command: Command, dir: &str, tx: Sender<Event>) -> Result<Handle, String> {
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
        let finished = waiter.lock().ok().and_then(|mut c| c.try_wait().ok().flatten());
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
        run_shell(Target::Terminal(1), "echo hi && echo err 1>&2", dir.to_str().unwrap(), tx).unwrap();
        let mut lines = vec![];
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::Line(_, l) | Event::Err(_, l) => lines.push(l),
                Event::Done(_, ok) => {
                    assert!(ok);
                    break;
                }
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
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn args(provider: &str, resume: bool, selections: &[(&str, &str)], opts: &AgentOptions, in_git: bool) -> Vec<String> {
        let sel = sel(selections);
        provider_args(&RunSpec { provider, prompt: "hi", resume, sel: &sel, opts, in_git })
    }

    #[test]
    fn claude_args() {
        let o = AgentOptions::default();
        assert_eq!(
            args("claude", true, &[("mode", "plan"), ("model", "opus")], &o, true),
            ["-p", "--permission-mode", "plan", "--model", "opus", "--continue", "--", "hi"]
        );
        assert_eq!(args("claude", false, &[], &o, true), ["-p", "--permission-mode", "acceptEdits", "--", "hi"]);
        let o = AgentOptions { effort: "high".into(), allowed_tools: "Read, Edit".into(), add_dirs: "/a,/b".into(), extra_args: "--verbose".into(), ..Default::default() };
        assert_eq!(
            args("claude", false, &[("mode", "dontAsk"), ("effort", "max")], &o, true),
            ["-p", "--permission-mode", "dontAsk", "--effort", "max", "--allowed-tools", "Read,Edit", "--add-dir", "/a", "--add-dir", "/b", "--verbose", "--", "hi"]
        );
    }

    #[test]
    fn codex_args() {
        let o = AgentOptions::default();
        assert_eq!(args("codex", false, &[], &o, true), ["exec", "--sandbox", "workspace-write", "hi"]);
        assert_eq!(args("codex", false, &[("mode", "full")], &o, true), ["exec", "--sandbox", "danger-full-access", "hi"]);
        let o = AgentOptions { profile: "work".into(), ..Default::default() };
        assert_eq!(
            args("codex", true, &[("mode", "read-only"), ("model", "gpt-5.6-sol"), ("effort", "high"), ("tier", "priority"), ("approval", "never")], &o, false),
            [
                "exec", "--sandbox", "read-only", "-m", "gpt-5.6-sol", "-c", "model_reasoning_effort=\"high\"", "-c", "service_tier=\"priority\"", "-c",
                "approval_policy=\"never\"", "-p", "work", "--skip-git-repo-check", "resume", "--last", "hi"
            ]
        );
    }

    #[test]
    fn opencode_args() {
        let o = AgentOptions::default();
        assert_eq!(args("opencode", false, &[], &o, true), ["run", "hi"]);
        assert_eq!(args("opencode", true, &[("mode", "plan")], &o, true), ["run", "--continue", "--agent", "plan", "hi"]);
        let o = AgentOptions { variant: "high".into(), thinking: true, ..Default::default() };
        assert_eq!(
            args("opencode", false, &[("mode", "auto"), ("model", "opencode/big-pickle"), ("agent", "build")], &o, true),
            ["run", "-m", "opencode/big-pickle", "--agent", "build", "--variant", "high", "--thinking", "--auto", "hi"]
        );
    }

    #[test]
    fn splits_quoted_args() {
        assert_eq!(split_args(r#"--foo "a b" 'c d' e"#), ["--foo", "a b", "c d", "e"]);
        assert!(split_args("   ").is_empty());
    }
}
