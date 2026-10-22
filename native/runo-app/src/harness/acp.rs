//! The Agent Client Protocol (ACP) driver. Agents that speak ACP (OpenCode's `acp`,
//! Gemini CLI and Qwen Code's `--acp`, Copilot's `--acp --stdio`, Cursor's `acp`) run
//! under Runo's agent host as one live session per turn instead of a print-mode run:
//! `Runo --agent-host runo-turn <turn folder> --acp` reads `acp.json` from the turn
//! folder, starts the agent, opens or reloads the conversation and sends it each prompt
//! from `in`, one after another.
//!
//! While it works, the host answers the agent's requests: permission requests by the
//! session's mode, asking the person in Runo (the corner card and the Inbox) for anything
//! the mode doesn't settle, and file reads and writes inside the session folder. MCP
//! servers go in `session/new`, so nothing is merged into the agent's project files.
//!
//! The agent's `session/update` notifications go to `out` as they are, with the host's
//! own lines (`{"runo": …}`) for what only it knows: the conversation id, the end of a
//! prompt, an error. `AcpStream` turns those into `AgentEvent`s. When the agent can't
//! start a session at all (an older release without ACP, a crash at start), the host
//! runs the turn's print-mode command instead and says so in `out`, so the turn still
//! runs and is read as before.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{AgentEvent, ToolKind, ToolStep};
use crate::stream::Parser;

/// The turn's settings, written by Runo into the turn folder.
pub const CONFIG: &str = "acp.json";

/// How long the agent has to start and answer `initialize` or open a session. Agents
/// run through npx can take a while the first time.
const START: Duration = Duration::from_secs(90);
/// After a prompt ends, how long a message sent just then still makes it into the
/// same session.
const GRACE: Duration = Duration::from_millis(1500);

/// What a turn runs: the agent's ACP command, the print-mode command to fall back on,
/// and the session's folder, conversation, model, mode and MCP servers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub provider: String,
    /// The program and its arguments.
    pub command: Vec<String>,
    /// The print-mode command with the prompt in it, run when ACP can't start.
    #[serde(default)]
    pub fallback: Vec<String>,
    pub cwd: String,
    /// The conversation to pick up; empty starts a new one.
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub model: String,
    /// Runo's mode: "plan", "edit" (or empty) or "full".
    #[serde(default)]
    pub mode: String,
    /// MCP servers in ACP's shape (`McpSpec::acp_json`).
    #[serde(default)]
    pub servers: Vec<Value>,
}

impl Config {
    pub fn write(&self, turn: &Path) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(turn.join(CONFIG), data)
    }

    fn read(turn: &Path) -> Option<Config> {
        serde_json::from_slice(&std::fs::read(turn.join(CONFIG)).ok()?).ok()
    }
}

/// Whether a turn folder belongs to an ACP turn.
pub fn is_acp(turn: &Path) -> bool {
    turn.join(CONFIG).is_file()
}

/// `Runo --agent-host <mark> <turn folder> --acp`. Returns the exit code, which is
/// also written to `turn/exit`.
pub fn main(turn: &Path) -> i32 {
    let code = match Config::read(turn) {
        Some(config) => run(turn, &config),
        None => {
            let _ = std::fs::write(
                turn.join("err"),
                "Runo couldn't read the turn's settings.\n",
            );
            2
        }
    };
    super::host::finish(turn, code);
    code
}

/// Run the turn: an ACP session when the agent can open one, else the fallback.
fn run(turn: &Path, config: &Config) -> i32 {
    let Ok(out) = std::fs::File::create(turn.join("out")) else {
        return 1;
    };
    let out = Arc::new(Mutex::new(out));
    match session(turn, config, &out) {
        Ok(code) => code,
        Err(why) => fall_back(turn, config, &out, &why),
    }
}

/// One line for `out`, from the host itself.
fn note(out: &Mutex<std::fs::File>, line: Value) {
    write_line(out, &json!({ "runo": line }).to_string());
}

fn write_line(out: &Mutex<std::fs::File>, line: &str) {
    if let Ok(mut out) = out.lock() {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// The JSON-RPC side of the agent: its input, and the next request id.
struct Rpc {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    next: u64,
}

impl Rpc {
    fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        send(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        );
        id
    }
}

fn send(stdin: &Mutex<Option<ChildStdin>>, message: &Value) {
    if let Ok(mut stdin) = stdin.lock() {
        if let Some(pipe) = stdin.as_mut() {
            let _ = writeln!(pipe, "{message}").and_then(|_| pipe.flush());
        }
    }
}

fn respond(stdin: &Mutex<Option<ChildStdin>>, id: &Value, result: Value) {
    send(
        stdin,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    );
}

fn respond_error(stdin: &Mutex<Option<ChildStdin>>, id: &Value, code: i64, message: &str) {
    send(
        stdin,
        &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    );
}

/// How the agent answered a request.
enum Reply {
    Result(Value),
    Error(String),
    /// It exited first.
    Gone,
    TimedOut,
}

/// The running session: the agent, what it can do, and where its output goes.
struct Session<'a> {
    rpc: Rpc,
    lines: Receiver<String>,
    out: &'a Arc<Mutex<std::fs::File>>,
    config: &'a Config,
    /// `session/update`s are dropped while a reloaded conversation replays its history.
    quiet: bool,
}

impl Session<'_> {
    /// Wait for the answer to request `id`, handling whatever else the agent sends.
    fn wait(&mut self, id: u64, limit: Option<Duration>) -> Reply {
        let start = Instant::now();
        loop {
            let left = match limit {
                Some(limit) => match limit.checked_sub(start.elapsed()) {
                    Some(left) => left,
                    None => return Reply::TimedOut,
                },
                None => Duration::from_secs(3600),
            };
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Reply::Gone,
            };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let method = message["method"].as_str();
            match (method, message.get("id")) {
                // A request from the agent.
                (Some(method), Some(rid)) => {
                    write_line(self.out, &line);
                    self.handle(method, rid.clone(), &message["params"]);
                }
                // A notification.
                (Some(_), None) => {
                    if !self.quiet {
                        write_line(self.out, &line);
                    }
                }
                // A response.
                (None, Some(rid)) if rid.as_u64() == Some(id) => {
                    if let Some(error) = message.get("error") {
                        return Reply::Error(
                            error["message"]
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| error.to_string()),
                        );
                    }
                    return Reply::Result(message["result"].clone());
                }
                _ => {}
            }
        }
    }

    /// Answer a request from the agent.
    fn handle(&mut self, method: &str, id: Value, params: &Value) {
        let stdin = self.rpc.stdin.clone();
        match method {
            "session/request_permission" => {
                let params = params.clone();
                let config = self.config.clone();
                match decide(&config.mode, &params) {
                    Decision::Pick(option) => respond(&stdin, &id, selected(&option)),
                    // The person may take minutes; the session carries on reading meanwhile.
                    Decision::Ask => {
                        std::thread::spawn(move || {
                            let result = ask_person(&config, &params);
                            respond(&stdin, &id, result);
                        });
                    }
                }
            }
            "fs/read_text_file" => match read_file(params) {
                Ok(content) => respond(&stdin, &id, json!({ "content": content })),
                Err(e) => respond_error(&stdin, &id, -32603, &e),
            },
            "fs/write_text_file" => match write_file(&self.config.cwd, params) {
                Ok(()) => respond(&stdin, &id, Value::Null),
                Err(e) => respond_error(&stdin, &id, -32603, &e),
            },
            _ => respond_error(&stdin, &id, -32601, "Runo doesn't offer this"),
        }
    }
}

/// Open the session and run the prompts. `Err` means no session could be opened, so
/// nothing was sent yet and the fallback can run instead.
fn session(turn: &Path, config: &Config, out: &Arc<Mutex<std::fs::File>>) -> Result<i32, String> {
    let (program, args) = config
        .command
        .split_first()
        .ok_or("the agent has no ACP command")?;
    let err = std::fs::File::create(turn.join("err")).map_err(|e| e.to_string())?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(&config.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("couldn't start {program}: {e}"))?;
    let stdin = Arc::new(Mutex::new(child.stdin.take()));
    let (tx, lines) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let mut s = Session {
        rpc: Rpc { stdin, next: 0 },
        lines,
        out,
        config,
        quiet: false,
    };
    let opened = open(&mut s);
    let sid = match opened {
        Ok(sid) => sid,
        Err(why) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(why);
        }
    };
    note(out, json!({ "session": sid }));
    let failed = prompts(turn, &mut s, &sid);
    // The input closes and the agent exits; one that lingers is stopped.
    s.rpc.stdin.lock().map(|mut s| s.take()).ok();
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) if start.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    Ok(if failed { 1 } else { 0 })
}

/// Initialize and open the conversation, set its mode and model. Returns its id.
fn open(s: &mut Session) -> Result<String, String> {
    let id = s.rpc.request(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": true, "writeTextFile": true },
                "terminal": false,
            },
            "clientInfo": { "name": "Runo", "version": env!("CARGO_PKG_VERSION") },
        }),
    );
    let init = match s.wait(id, Some(START)) {
        Reply::Result(init) => init,
        Reply::Error(e) => return Err(format!("it refused to start a session: {e}")),
        Reply::Gone => return Err("it exited before starting a session".into()),
        Reply::TimedOut => return Err("it didn't answer".into()),
    };
    let caps = &init["agentCapabilities"];
    let servers: Vec<Value> = s
        .config
        .servers
        .iter()
        .filter(|server| match server["type"].as_str() {
            Some("http") => caps["mcpCapabilities"]["http"] == true,
            Some("sse") => caps["mcpCapabilities"]["sse"] == true,
            _ => true,
        })
        .cloned()
        .collect();
    let mut sid = String::new();
    let mut opened = Value::Null;
    if !s.config.session.is_empty() && caps["loadSession"] == true {
        s.quiet = true;
        let id = s.rpc.request(
            "session/load",
            json!({ "sessionId": s.config.session, "cwd": s.config.cwd, "mcpServers": servers }),
        );
        let reply = s.wait(id, Some(START));
        s.quiet = false;
        match reply {
            Reply::Result(result) => {
                sid = s.config.session.clone();
                opened = result;
            }
            Reply::Gone => return Err("it exited while opening the conversation".into()),
            _ => note(
                s.out,
                json!({ "notice": "The agent couldn't pick up the conversation, so this turn starts a new one." }),
            ),
        }
    }
    if sid.is_empty() {
        let id = s.rpc.request(
            "session/new",
            json!({ "cwd": s.config.cwd, "mcpServers": servers }),
        );
        match s.wait(id, Some(START)) {
            Reply::Result(result) => {
                sid = result["sessionId"].as_str().unwrap_or_default().to_string();
                opened = result;
            }
            Reply::Error(e) => return Err(format!("it couldn't start a session: {e}")),
            Reply::Gone => return Err("it exited while starting a session".into()),
            Reply::TimedOut => return Err("it didn't start a session".into()),
        }
        if sid.is_empty() {
            return Err("it started a session without an id".into());
        }
    }
    if let Some(mode) = pick_mode(&s.config.mode, &opened["modes"]) {
        let id = s.rpc.request(
            "session/set_mode",
            json!({ "sessionId": sid, "modeId": mode }),
        );
        s.wait(id, Some(Duration::from_secs(20)));
    }
    if !s.config.model.is_empty() {
        match pick_model(&s.config.model, &opened["models"]) {
            Some(model) => {
                let id = s.rpc.request(
                    "session/set_model",
                    json!({ "sessionId": sid, "modelId": model }),
                );
                if let Reply::Error(e) = s.wait(id, Some(Duration::from_secs(20))) {
                    note(
                        s.out,
                        json!({ "notice": format!("The agent kept its own model: {e}") }),
                    );
                }
            }
            None => note(
                s.out,
                json!({ "notice": format!("{} isn't one of the agent's models here, so it used its default.", s.config.model) }),
            ),
        }
    }
    Ok(sid)
}

/// Send each prompt in `in`, one after another, until none is left. Returns true when
/// a prompt failed.
fn prompts(turn: &Path, s: &mut Session, sid: &str) -> bool {
    let path = turn.join("in");
    let mut offset = 0u64;
    let mut waiting: std::collections::VecDeque<String> = Default::default();
    let mut sent = 0u64;
    let mut failed = false;
    loop {
        waiting.extend(super::host::new_lines(&path, &mut offset));
        if waiting.is_empty() {
            // A message sent as the prompt ended still joins this session.
            let start = Instant::now();
            while waiting.is_empty() && start.elapsed() < GRACE {
                std::thread::sleep(Duration::from_millis(100));
                waiting.extend(super::host::new_lines(&path, &mut offset));
            }
            if waiting.is_empty() {
                return failed;
            }
        }
        let Some(line) = waiting.pop_front() else {
            return failed;
        };
        let text = serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(|v| v["message"]["content"].as_str().map(str::to_string))
            .unwrap_or(line);
        let id = s.rpc.request(
            "session/prompt",
            json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] }),
        );
        sent += 1;
        let _ = std::fs::write(turn.join("sent"), sent.to_string());
        match s.wait(id, None) {
            Reply::Result(result) => {
                let stop = result["stopReason"].as_str().unwrap_or("end_turn");
                note(s.out, json!({ "done": stop }));
                if stop == "refusal" {
                    failed = true;
                }
            }
            Reply::Error(e) => {
                note(s.out, json!({ "error": e }));
                failed = true;
            }
            Reply::Gone => {
                note(
                    s.out,
                    json!({ "error": "The agent exited in the middle of its turn." }),
                );
                return true;
            }
            Reply::TimedOut => {}
        }
    }
}

/// Run the print-mode command instead, with its output in `out` as usual.
fn fall_back(turn: &Path, config: &Config, out: &Arc<Mutex<std::fs::File>>, why: &str) -> i32 {
    let Some((program, args)) = config.fallback.split_first() else {
        note(
            out,
            json!({ "error": format!("The agent couldn't start a session: {why}.") }),
        );
        return 1;
    };
    note(out, json!({ "fallback": why }));
    // The first message is the prompt in the fallback command.
    let _ = std::fs::write(turn.join("sent"), "1");
    let err = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(turn.join("err"));
    let out_file = match out.lock() {
        Ok(file) => file.try_clone(),
        Err(_) => return 1,
    };
    let Ok(out_file) = out_file else {
        return 1;
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&config.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file));
    if let Ok(err) = err {
        command.stderr(Stdio::from(err));
    }
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            note(
                out,
                json!({ "error": format!("Runo could not start {program}: {e}") }),
            );
            127
        }
    }
}

/// How a permission request is settled.
#[derive(Debug, PartialEq)]
enum Decision {
    /// Answer with this option at once.
    Pick(String),
    /// Ask the person.
    Ask,
}

/// Settle what the session's mode settles: Full access allows everything, Auto edit
/// allows reading and editing, Plan only refuses changes. The rest goes to the person.
fn decide(mode: &str, params: &Value) -> Decision {
    let kind = params["toolCall"]["kind"].as_str().unwrap_or("other");
    let options = params["options"].as_array().cloned().unwrap_or_default();
    let reads = matches!(kind, "read" | "search" | "think" | "fetch");
    let changes = matches!(kind, "edit" | "delete" | "move");
    let allow = mode == "full" || reads || (changes && mode != "plan");
    if allow {
        if let Some(option) = option_of(&options, true) {
            return Decision::Pick(option);
        }
    }
    if changes && mode == "plan" {
        if let Some(option) = option_of(&options, false) {
            return Decision::Pick(option);
        }
    }
    Decision::Ask
}

/// The option that allows (or refuses) this once, else always.
fn option_of(options: &[Value], allow: bool) -> Option<String> {
    let (once, always) = if allow {
        ("allow_once", "allow_always")
    } else {
        ("reject_once", "reject_always")
    };
    [once, always].iter().find_map(|kind| {
        options
            .iter()
            .find(|o| o["kind"] == *kind)
            .and_then(|o| o["optionId"].as_str())
            .map(str::to_string)
    })
}

fn selected(option: &str) -> Value {
    json!({ "outcome": { "outcome": "selected", "optionId": option } })
}

/// Ask the person in Runo and turn the answer into the agent's option.
fn ask_person(config: &Config, params: &Value) -> Value {
    let call = &params["toolCall"];
    let tool = call["title"]
        .as_str()
        .filter(|t| !t.is_empty())
        .or(call["kind"].as_str())
        .unwrap_or("a tool");
    let input = call.get("rawInput").cloned().unwrap_or_else(|| json!({}));
    let base = crate::approvals::folder();
    let answer = crate::approvals::ask(&base, &config.cwd, tool, &input, crate::approvals::wait());
    let options = params["options"].as_array().cloned().unwrap_or_default();
    let allowed = matches!(answer, crate::approvals::Answer::Allow { .. });
    match option_of(&options, allowed) {
        Some(option) => selected(&option),
        None => json!({ "outcome": { "outcome": "cancelled" } }),
    }
}

/// The agent's mode for Runo's: its plan mode for Plan only. Other modes are kept to
/// by answering permission requests.
fn pick_mode(mode: &str, modes: &Value) -> Option<String> {
    if mode != "plan" {
        return None;
    }
    let current = modes["currentModeId"].as_str().unwrap_or("");
    modes["availableModes"]
        .as_array()?
        .iter()
        .filter_map(|m| m["id"].as_str())
        .find(|id| id.to_ascii_lowercase().contains("plan"))
        .filter(|id| *id != current)
        .map(str::to_string)
}

/// The agent's id for the model picked in Runo: the same id, or one that ends in it
/// (`anthropic/claude-sonnet-4` for `claude-sonnet-4`).
fn pick_model(model: &str, models: &Value) -> Option<String> {
    let ids: Vec<&str> = models["availableModels"]
        .as_array()?
        .iter()
        .filter_map(|m| m["modelId"].as_str())
        .collect();
    ids.iter()
        .find(|id| **id == model)
        .or_else(|| ids.iter().find(|id| id.ends_with(&format!("/{model}"))))
        .map(|id| id.to_string())
}

/// A file the agent reads through Runo.
fn read_file(params: &Value) -> Result<String, String> {
    let path = params["path"].as_str().ok_or("no path")?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("couldn't read {path}: {e}"))?;
    let line = params["line"].as_u64().unwrap_or(1).max(1) as usize;
    let limit = params["limit"].as_u64().map(|l| l as usize);
    if line == 1 && limit.is_none() {
        return Ok(text);
    }
    let lines = text.split_inclusive('\n').skip(line - 1);
    Ok(match limit {
        Some(limit) => lines.take(limit).collect(),
        None => lines.collect(),
    })
}

/// A file the agent writes through Runo: only inside the session folder.
fn write_file(cwd: &str, params: &Value) -> Result<(), String> {
    let path = PathBuf::from(params["path"].as_str().ok_or("no path")?);
    let content = params["content"].as_str().ok_or("no content")?;
    if !inside(Path::new(cwd), &path) {
        return Err(format!(
            "{} is outside the session folder, so Runo didn't write it",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, content).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

/// Whether `path` is inside `folder`, after links and `..` are resolved as far as the
/// path exists.
fn inside(folder: &Path, path: &Path) -> bool {
    let Ok(folder) = folder.canonicalize() else {
        return false;
    };
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        folder.join(path)
    };
    // The nearest part of the path that exists, resolved, with the rest put back.
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return false;
        };
        rest.push(name.to_os_string());
        existing = parent;
    }
    let Ok(mut resolved) = existing.canonicalize() else {
        return false;
    };
    for name in rest.iter().rev() {
        if name == ".." {
            return false;
        }
        resolved.push(name);
    }
    resolved.starts_with(&folder)
}

/// Reads an ACP turn's `out`: the agent's `session/update`s and the host's own lines.
/// After a fallback, the lines are the print-mode run's, read by its own parser.
pub struct AcpStream {
    fallback: Option<Parser>,
    falling_back: bool,
    named: bool,
}

impl AcpStream {
    /// `fallback` reads the print-mode output (`None` for plain text).
    pub fn new(fallback: Option<Parser>) -> AcpStream {
        AcpStream {
            fallback,
            falling_back: false,
            named: false,
        }
    }

    pub fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        if self.falling_back {
            return match self.fallback.as_mut() {
                Some(parser) => parser.feed(line),
                None => vec![AgentEvent::Text(format!("{line}\n"))],
            };
        }
        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            return vec![];
        };
        if let Some(runo) = message.get("runo") {
            return self.own(runo);
        }
        if message["method"] == "session/update" {
            return update(&message["params"]["update"]);
        }
        vec![]
    }

    fn own(&mut self, line: &Value) -> Vec<AgentEvent> {
        if let Some(sid) = line["session"].as_str() {
            if self.named {
                return vec![];
            }
            self.named = true;
            return vec![AgentEvent::Conversation(sid.to_string())];
        }
        if let Some(text) = line["notice"].as_str() {
            return vec![AgentEvent::Notice(text.to_string())];
        }
        if let Some(text) = line["error"].as_str() {
            return vec![AgentEvent::Error(text.to_string())];
        }
        if line.get("fallback").is_some() {
            self.falling_back = true;
            return vec![];
        }
        match line["done"].as_str() {
            Some("max_tokens") => vec![AgentEvent::Notice(
                "The agent stopped at the model's output limit.".into(),
            )],
            Some("max_turn_requests") => vec![AgentEvent::Notice(
                "The agent stopped at its limit of model requests for one turn.".into(),
            )],
            Some("refusal") => vec![AgentEvent::Error("The model refused to go on.".into())],
            _ => vec![],
        }
    }
}

/// The events of one `session/update`.
fn update(update: &Value) -> Vec<AgentEvent> {
    match update["sessionUpdate"].as_str().unwrap_or("") {
        "agent_message_chunk" => match update["content"]["text"].as_str() {
            Some(text) if update["content"]["type"] == "text" => {
                vec![AgentEvent::Text(text.to_string())]
            }
            _ => vec![],
        },
        "tool_call" => {
            let id = update["toolCallId"].as_str().unwrap_or("");
            let kind = update["kind"].as_str().unwrap_or("other");
            let title = update["title"].as_str().unwrap_or(kind);
            let mut out = vec![AgentEvent::Tool(
                ToolStep::running(id, kind, title).kind(kind_of(kind)),
            )];
            out.extend(finished(update));
            out
        }
        "tool_call_update" => finished(update).into_iter().collect(),
        "plan" => crate::stream::todos(&json!({ "items": update["entries"] }))
            .map(AgentEvent::Todos)
            .into_iter()
            .collect(),
        "usage_update" => {
            let usage = crate::store::Usage {
                input: update["used"].as_u64().unwrap_or(0),
                window: update["size"].as_u64().unwrap_or(0),
                cost_cents: match update["cost"]["currency"].as_str() {
                    Some("USD") | None => update["cost"]["amount"].as_f64().unwrap_or(0.0) * 100.0,
                    Some(_) => 0.0,
                },
                ..Default::default()
            };
            if usage.input == 0 {
                vec![]
            } else {
                vec![AgentEvent::Usage(usage)]
            }
        }
        _ => vec![],
    }
}

/// The result of a tool call whose status says it ended.
fn finished(update: &Value) -> Option<AgentEvent> {
    let ok = match update["status"].as_str()? {
        "completed" => true,
        "failed" => false,
        _ => return None,
    };
    let text: String = update["content"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|c| c["content"]["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|t| !t.is_empty())
        .or_else(|| match &update["rawOutput"] {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => other["output"]
                .as_str()
                .map(str::to_string)
                .or_else(|| Some(other.to_string())),
        })
        .unwrap_or_default();
    Some(AgentEvent::ToolDone {
        id: update["toolCallId"].as_str().unwrap_or("").to_string(),
        ok,
        output: super::event::tail(&text, 4096),
    })
}

/// Runo's kind for an ACP tool kind.
fn kind_of(kind: &str) -> ToolKind {
    match kind {
        "read" => ToolKind::Read,
        "edit" | "move" => ToolKind::Edit,
        "delete" => ToolKind::Delete,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Shell,
        "think" => ToolKind::Think,
        "fetch" => ToolKind::Fetch,
        "switch_mode" => ToolKind::Plan,
        _ => ToolKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::StepState;

    fn options() -> Value {
        json!([
            { "optionId": "a1", "name": "Allow", "kind": "allow_once" },
            { "optionId": "a2", "name": "Always", "kind": "allow_always" },
            { "optionId": "r1", "name": "Reject", "kind": "reject_once" },
        ])
    }

    fn asking(kind: &str) -> Value {
        json!({ "sessionId": "s", "toolCall": { "toolCallId": "t", "kind": kind }, "options": options() })
    }

    #[test]
    fn the_mode_settles_what_it_can_and_asks_the_rest() {
        let pick = |s: &str| Decision::Pick(s.into());
        assert_eq!(decide("full", &asking("execute")), pick("a1"));
        assert_eq!(decide("edit", &asking("edit")), pick("a1"));
        assert_eq!(decide("", &asking("read")), pick("a1"));
        assert_eq!(decide("edit", &asking("execute")), Decision::Ask);
        assert_eq!(decide("plan", &asking("edit")), pick("r1"));
        assert_eq!(decide("plan", &asking("search")), pick("a1"));
        assert_eq!(decide("plan", &asking("execute")), Decision::Ask);
    }

    #[test]
    fn plan_only_uses_the_agents_plan_mode() {
        let modes = json!({
            "currentModeId": "build",
            "availableModes": [{ "id": "build" }, { "id": "plan" }],
        });
        assert_eq!(pick_mode("plan", &modes).as_deref(), Some("plan"));
        assert_eq!(pick_mode("edit", &modes), None);
        assert_eq!(pick_mode("plan", &Value::Null), None);
    }

    #[test]
    fn a_model_matches_by_id_or_its_last_part() {
        let models = json!({ "availableModels": [
            { "modelId": "anthropic/claude-sonnet-4" }, { "modelId": "gpt-5" },
        ]});
        assert_eq!(pick_model("gpt-5", &models).as_deref(), Some("gpt-5"));
        assert_eq!(
            pick_model("claude-sonnet-4", &models).as_deref(),
            Some("anthropic/claude-sonnet-4")
        );
        assert_eq!(pick_model("o3", &models), None);
    }

    #[test]
    fn writes_stay_inside_the_session_folder() {
        let dir = std::env::temp_dir().join(format!("runo-acp-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let file = dir.join("src/new/a.txt");
        write_file(&cwd, &json!({ "path": file, "content": "hi\nthere\n" })).unwrap();
        assert_eq!(
            read_file(&json!({ "path": file, "line": 2, "limit": 1 })).unwrap(),
            "there\n"
        );
        let outside = dir.join("../escape.txt");
        assert!(write_file(&cwd, &json!({ "path": outside, "content": "x" })).is_err());
        assert!(write_file(&cwd, &json!({ "path": "/etc/runo-x", "content": "x" })).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn updates_become_events() {
        let mut p = AcpStream::new(None);
        let line = |update: Value| {
            json!({ "jsonrpc": "2.0", "method": "session/update",
                    "params": { "sessionId": "s", "update": update } })
            .to_string()
        };
        assert_eq!(
            p.feed(&json!({ "runo": { "session": "ses_1" } }).to_string()),
            [AgentEvent::Conversation("ses_1".into())]
        );
        assert_eq!(
            p.feed(&line(json!({ "sessionUpdate": "agent_message_chunk",
                                 "content": { "type": "text", "text": "Hi" } }))),
            [AgentEvent::Text("Hi".into())]
        );
        assert!(p
            .feed(&line(json!({ "sessionUpdate": "agent_thought_chunk",
                                "content": { "type": "text", "text": "hmm" } })))
            .is_empty());
        let events = p.feed(&line(
            json!({ "sessionUpdate": "tool_call", "toolCallId": "t1",
            "title": "cargo test", "kind": "execute", "status": "pending" }),
        ));
        let AgentEvent::Tool(step) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(
            (step.kind, step.state, step.label.as_str()),
            (ToolKind::Shell, StepState::Running, "cargo test")
        );
        assert_eq!(
            p.feed(&line(json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1",
                "status": "failed",
                "content": [{ "type": "content", "content": { "type": "text", "text": "1 failed" } }] }))),
            [AgentEvent::ToolDone { id: "t1".into(), ok: false, output: "1 failed".into() }]
        );
        let todos = p.feed(&line(json!({ "sessionUpdate": "plan", "entries": [
            { "content": "Read", "status": "completed", "priority": "high" },
            { "content": "Fix", "status": "in_progress", "priority": "high" },
        ]})));
        assert!(matches!(&todos[..], [AgentEvent::Todos(steps)] if steps.len() == 2));
        assert_eq!(
            p.feed(&json!({ "runo": { "error": "boom" } }).to_string()),
            [AgentEvent::Error("boom".into())]
        );
        // A permission request is answered by the host; it isn't an event.
        assert!(p
            .feed(r#"{"jsonrpc":"2.0","id":4,"method":"session/request_permission","params":{}}"#)
            .is_empty());
    }

    #[test]
    fn after_a_fallback_the_print_mode_parser_reads_on() {
        let mut p = AcpStream::new(None);
        assert!(p
            .feed(&json!({ "runo": { "fallback": "no ACP" } }).to_string())
            .is_empty());
        assert_eq!(
            p.feed("plain words"),
            [AgentEvent::Text("plain words\n".into())]
        );
    }
}

#[cfg(all(test, unix))]
mod host_tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-acp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A small ACP agent in Python: it opens a session, asks to run a command on each
    /// prompt, and answers with what it was told.
    const AGENT: &str = r#"
import json, sys
def out(m): sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
pending = {}
for line in sys.stdin:
    m = json.loads(line)
    if m.get("method") == "initialize":
        out({"jsonrpc": "2.0", "id": m["id"], "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": False}}})
    elif m.get("method") == "session/new":
        out({"jsonrpc": "2.0", "id": m["id"], "result": {"sessionId": "ses_9"}})
    elif m.get("method") == "session/prompt":
        text = m["params"]["prompt"][0]["text"]
        pending[100 + m["id"]] = (m["id"], text)
        out({"jsonrpc": "2.0", "id": 100 + m["id"], "method": "session/request_permission", "params": {"sessionId": "ses_9", "toolCall": {"toolCallId": "t", "kind": "edit", "title": "Edit a.txt"}, "options": [{"optionId": "yes", "kind": "allow_once", "name": "Yes"}, {"optionId": "no", "kind": "reject_once", "name": "No"}]}})
    elif "result" in m and m["id"] in pending:
        pid, text = pending.pop(m["id"])
        choice = m["result"]["outcome"].get("optionId")
        out({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "ses_9", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text + ":" + choice}}}})
        out({"jsonrpc": "2.0", "id": pid, "result": {"stopReason": "end_turn"}})
"#;

    fn config(dir: &Path, command: Vec<String>, fallback: Vec<String>) -> Config {
        Config {
            provider: "opencode".into(),
            command,
            fallback,
            cwd: dir.to_string_lossy().into(),
            mode: "edit".into(),
            ..Config::default()
        }
    }

    fn events(turn: &Path) -> Vec<AgentEvent> {
        let mut p = AcpStream::new(None);
        std::fs::read_to_string(turn.join("out"))
            .unwrap()
            .lines()
            .flat_map(|l| p.feed(l))
            .collect()
    }

    #[test]
    fn a_session_runs_each_prompt_and_answers_permissions_by_mode() {
        if Command::new("python3").arg("-V").output().is_err() {
            return;
        }
        let turn = temp("session");
        let script = turn.join("agent.py");
        std::fs::write(&script, AGENT).unwrap();
        super::super::host::send(&turn, "first").unwrap();
        super::super::host::send(&turn, "second").unwrap();
        let cfg = config(
            &turn,
            vec!["python3".into(), script.to_string_lossy().into()],
            vec![],
        );
        cfg.write(&turn).unwrap();
        assert_eq!(main(&turn), 0);
        assert_eq!(
            std::fs::read_to_string(turn.join("exit")).unwrap().trim(),
            "0"
        );
        assert_eq!(std::fs::read_to_string(turn.join("sent")).unwrap(), "2");
        let events = events(&turn);
        assert_eq!(events[0], AgentEvent::Conversation("ses_9".into()));
        let text: Vec<&AgentEvent> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::Text(_)))
            .collect();
        // Auto edit allowed the edits without asking.
        assert_eq!(
            text,
            [
                &AgentEvent::Text("first:yes".into()),
                &AgentEvent::Text("second:yes".into())
            ]
        );
        assert!(super::super::host::unsent(&turn).is_empty());
        let _ = std::fs::remove_dir_all(turn);
    }

    #[test]
    fn an_agent_without_acp_falls_back_to_print_mode() {
        let turn = temp("fallback");
        super::super::host::send(&turn, "go").unwrap();
        let cfg = config(
            &turn,
            vec![
                "sh".into(),
                "-c".into(),
                "echo unknown flag >&2; exit 2".into(),
            ],
            vec!["sh".into(), "-c".into(), "echo printed".into()],
        );
        cfg.write(&turn).unwrap();
        assert_eq!(main(&turn), 0);
        assert_eq!(events(&turn), [AgentEvent::Text("printed\n".into())]);
        assert!(std::fs::read_to_string(turn.join("err"))
            .unwrap()
            .contains("unknown flag"));
        assert!(super::super::host::unsent(&turn).is_empty());
        let _ = std::fs::remove_dir_all(turn);
    }
}
