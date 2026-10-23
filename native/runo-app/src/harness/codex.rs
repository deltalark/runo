//! The Codex app-server driver. Codex turns run under Runo's agent host as
//! `codex app-server`, the JSON-RPC server Codex's own apps use, instead of
//! `codex exec`: `Runo --agent-host runo-turn <turn folder> --codex` reads `codex.json`
//! from the turn folder, starts or resumes the thread and runs each prompt from `in` as a
//! turn. A message sent while a turn runs steers it (`turn/steer`), so Codex reads it
//! before its next step.
//!
//! The host answers Codex's approval requests: Full access allows them, Plan only
//! refuses file changes, and the rest wait for the person in Runo (the corner card and
//! the Inbox). The sandbox follows the session's mode as with `exec`, but a command that
//! needs more than the sandbox allows is asked about instead of failing.
//!
//! Codex's notifications go to `out` as they are; `CodexAppStream` reads them, passing
//! tool items through the `exec --json` reader so steps look the same either way. When
//! the installed Codex has no app server, the host runs the `exec` command instead.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant};

use super::rpc::{self, Agent, HostLines, Message, Out, Reply};
use super::AgentEvent;
use crate::stream::{JsonStream, Parser};

/// The turn's settings, written by Runo into the turn folder.
pub const CONFIG: &str = "codex.json";

/// How long Codex has to start and open the thread.
const START: Duration = Duration::from_secs(60);
/// After a turn ends, how long a message sent just then still joins the thread.
const GRACE: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// `codex app-server` and its `-c` settings.
    pub command: Vec<String>,
    /// The `codex exec` command with the prompt in it, run when there is no app server.
    #[serde(default)]
    pub fallback: Vec<String>,
    pub cwd: String,
    /// The thread to resume; empty starts a new one.
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
    /// Runo's mode: "plan", "edit" (or empty) or "full".
    #[serde(default)]
    pub mode: String,
}

impl Config {
    pub fn write(&self, turn: &Path) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(turn.join(CONFIG), data)
    }

    fn read(turn: &Path) -> Option<Config> {
        serde_json::from_slice(&std::fs::read(turn.join(CONFIG)).ok()?).ok()
    }

    /// The approval policy and sandbox for the session's mode.
    fn policy(&self) -> (&'static str, &'static str) {
        match self.mode.as_str() {
            "full" => ("never", "danger-full-access"),
            "plan" => ("on-request", "read-only"),
            _ => ("on-request", "workspace-write"),
        }
    }
}

/// Whether a turn folder belongs to a Codex app-server turn.
pub fn is_codex(turn: &Path) -> bool {
    turn.join(CONFIG).is_file()
}

/// `Runo --agent-host <mark> <turn folder> --codex`. Returns the exit code, which is
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

fn run(turn: &Path, config: &Config) -> i32 {
    let Some(out) = rpc::create_out(turn) else {
        return 1;
    };
    match session(turn, config, &out) {
        Ok(code) => code,
        Err(why) => rpc::fall_back(turn, &config.fallback, &config.cwd, &out, &why),
    }
}

/// Handles what Codex sends besides answers.
struct Handler<'a> {
    out: &'a Out,
    config: &'a Config,
}

impl Handler<'_> {
    fn on(&self, agent: &mut Agent, message: Message) {
        match message {
            Message::Notification { line, .. } => rpc::write_line(self.out, &line),
            Message::Request {
                method,
                id,
                params,
                line,
            } => {
                rpc::write_line(self.out, &line);
                self.handle(agent, &method, id, &params);
            }
            _ => {}
        }
    }

    /// Answer a request from Codex.
    fn handle(&self, agent: &Agent, method: &str, id: Value, params: &Value) {
        let input = agent.input.clone();
        let Some(ask) = approval(method, params) else {
            let result = match method {
                // Extra permissions and MCP forms aren't granted from Runo yet.
                "item/permissions/requestApproval" => {
                    json!({ "permissions": {}, "scope": "turn" })
                }
                "mcpServer/elicitation/request" => json!({ "action": "decline" }),
                _ => {
                    rpc::respond_error(&input, &id, -32601, "Runo doesn't offer this");
                    return;
                }
            };
            rpc::respond(&input, &id, result);
            return;
        };
        match (self.config.mode.as_str(), ask.file) {
            ("full", _) => rpc::respond(&input, &id, ask.answer(true)),
            ("plan", true) => rpc::respond(&input, &id, ask.answer(false)),
            _ => {
                let cwd = self.config.cwd.clone();
                std::thread::spawn(move || {
                    let base = crate::approvals::folder();
                    let answer = crate::approvals::ask(
                        &base,
                        &cwd,
                        ask.tool,
                        &ask.input,
                        crate::approvals::wait(),
                    );
                    let allowed = matches!(answer, crate::approvals::Answer::Allow { .. });
                    rpc::respond(&input, &id, ask.answer(allowed));
                });
            }
        }
    }

    fn wait(&self, agent: &mut Agent, id: u64, limit: Option<Duration>) -> Reply {
        agent.wait(id, limit, &mut |agent, message| self.on(agent, message))
    }
}

/// One approval Codex asks for.
#[derive(Debug, PartialEq)]
struct Approval {
    /// A file change rather than a command.
    file: bool,
    /// What the card shows it as.
    tool: &'static str,
    input: Value,
    /// The older requests answer "approved" and "denied".
    legacy: bool,
}

impl Approval {
    fn answer(&self, allow: bool) -> Value {
        let decision = match (self.legacy, allow) {
            (false, true) => "accept",
            (false, false) => "decline",
            (true, true) => "approved",
            (true, false) => "denied",
        };
        json!({ "decision": decision })
    }
}

/// The approval in a request, if it is one.
fn approval(method: &str, params: &Value) -> Option<Approval> {
    let command = |p: &Value| match &p["command"] {
        Value::Array(words) => words
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        other => other.as_str().unwrap_or("").to_string(),
    };
    let (file, legacy) = match method {
        "item/commandExecution/requestApproval" => (false, false),
        "item/fileChange/requestApproval" => (true, false),
        "execCommandApproval" => (false, true),
        "applyPatchApproval" => (true, true),
        _ => return None,
    };
    let input = if file {
        let path = params["grantRoot"]
            .as_str()
            .or_else(|| {
                params["fileChanges"]
                    .as_object()
                    .and_then(|m| m.keys().next().map(String::as_str))
            })
            .unwrap_or("");
        json!({ "file_path": path, "reason": params["reason"] })
    } else {
        json!({ "command": command(params), "reason": params["reason"] })
    };
    Some(Approval {
        file,
        tool: if file { "Edit" } else { "Bash" },
        input,
        legacy,
    })
}

/// Open the thread and run the turns. `Err` means Codex couldn't open a thread, so
/// nothing was sent yet and the fallback can run instead.
fn session(turn: &Path, config: &Config, out: &Out) -> Result<i32, String> {
    let (program, args) = config
        .command
        .split_first()
        .ok_or("Codex has no app-server command")?;
    let mut agent = Agent::spawn(turn, program, args, &config.cwd)?;
    let handler = Handler { out, config };
    let thread = match open(&mut agent, &handler) {
        Ok(thread) => thread,
        Err(why) => {
            agent.kill();
            return Err(why);
        }
    };
    rpc::note(out, json!({ "session": thread }));
    let failed = match turns(turn, &mut agent, &handler, &thread) {
        Ok(failed) => failed,
        Err(why) => {
            agent.kill();
            return Err(why);
        }
    };
    agent.close();
    Ok(if failed { 1 } else { 0 })
}

/// Initialize and start or resume the thread. Returns its id.
fn open(agent: &mut Agent, h: &Handler) -> Result<String, String> {
    let config = h.config;
    let id = agent.request(
        "initialize",
        json!({
            "clientInfo": { "name": "runo", "title": "Runo", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": null,
        }),
    );
    match h.wait(agent, id, Some(START)) {
        Reply::Result(_) => {}
        Reply::Error(e) => return Err(format!("its app server refused to start: {e}")),
        Reply::Gone => return Err("it has no app server".into()),
        Reply::TimedOut => return Err("its app server didn't answer".into()),
    }
    agent.notify("initialized", Value::Null);
    let (approval, sandbox) = config.policy();
    let mut settings = json!({
        "cwd": config.cwd,
        "approvalPolicy": approval,
        "sandbox": sandbox,
    });
    if !config.model.is_empty() {
        settings["model"] = config.model.clone().into();
    }
    if !config.session.is_empty() {
        let mut params = settings.clone();
        params["threadId"] = config.session.clone().into();
        params["excludeTurns"] = true.into();
        let id = agent.request("thread/resume", params);
        match h.wait(agent, id, Some(START)) {
            Reply::Result(result) => {
                if let Some(thread) = result["thread"]["id"].as_str() {
                    return Ok(thread.to_string());
                }
            }
            Reply::Gone => return Err("it exited while opening the thread".into()),
            _ => {}
        }
        rpc::note(
            h.out,
            json!({ "notice": "Codex couldn't pick up the conversation, so this turn starts a new one." }),
        );
    }
    let id = agent.request("thread/start", settings);
    match h.wait(agent, id, Some(START)) {
        Reply::Result(result) => result["thread"]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "it started a thread without an id".to_string()),
        Reply::Error(e) => Err(format!("it couldn't start a thread: {e}")),
        Reply::Gone => Err("it exited while starting a thread".into()),
        Reply::TimedOut => Err("it didn't start a thread".into()),
    }
}

fn user_input(text: &str) -> Value {
    json!([{ "type": "text", "text": text, "text_elements": [] }])
}

/// Run a turn for each prompt in `in`, steering the running turn with messages that
/// arrive while it works. Returns true when a turn failed.
fn turns(turn: &Path, agent: &mut Agent, h: &Handler, thread: &str) -> Result<bool, String> {
    let path = turn.join("in");
    let mut offset = 0u64;
    let mut waiting: VecDeque<String> = VecDeque::new();
    let mut sent = 0u64;
    let mut failed = false;
    // Turns Codex took.
    let mut started = 0u32;
    let count = |sent: u64| {
        let _ = std::fs::write(turn.join("sent"), sent.to_string());
    };
    loop {
        waiting.extend(super::host::new_lines(&path, &mut offset));
        if waiting.is_empty() {
            let start = Instant::now();
            while waiting.is_empty() && start.elapsed() < GRACE {
                std::thread::sleep(Duration::from_millis(100));
                waiting.extend(super::host::new_lines(&path, &mut offset));
            }
        }
        let Some(line) = waiting.pop_front() else {
            return Ok(failed);
        };
        let mut params =
            json!({ "threadId": thread, "input": user_input(&rpc::prompt_text(&line)) });
        if !h.config.effort.is_empty() {
            params["effort"] = h.config.effort.clone().into();
        }
        let id = agent.request("turn/start", params);
        sent += 1;
        count(sent);
        let turn_id = match h.wait(agent, id, Some(START)) {
            // A Codex whose app server doesn't take Runo's first turn (a protocol change
            // in a newer or older release) runs the turn with `exec` instead.
            Reply::Result(result) if started == 0 && result["turn"]["id"].as_str().is_none() => {
                return Err("its app server answered the turn in a shape Runo doesn't know".into());
            }
            Reply::Error(e) if started == 0 => {
                return Err(format!("its app server refused the turn: {e}"));
            }
            Reply::Result(result) => result["turn"]["id"].as_str().unwrap_or("").to_string(),
            Reply::Error(e) => {
                rpc::note(h.out, json!({ "error": e }));
                failed = true;
                continue;
            }
            Reply::Gone | Reply::TimedOut => {
                rpc::note(
                    h.out,
                    json!({ "error": "Codex stopped answering before the turn started." }),
                );
                return Ok(true);
            }
        };
        started += 1;
        // Steers waiting for their answer, by request id.
        let mut steering: HashMap<u64, String> = HashMap::new();
        let mut ended = false;
        let mut active = false;
        let mut settle_until: Option<Instant> = None;
        loop {
            match agent.next(Duration::from_millis(100)) {
                Message::Notification {
                    method,
                    params,
                    line,
                } => {
                    rpc::write_line(h.out, &line);
                    let completed =
                        method == "turn/completed" && params["turn"]["id"] == turn_id.as_str();
                    // The thread going idle ends the turn too, so a release that reshapes
                    // `turn/completed` can't leave the host waiting forever.
                    let status = (method == "thread/status/changed"
                        && params["threadId"] == thread)
                        .then(|| params["status"]["type"].as_str().unwrap_or(""));
                    // Only after the thread was seen working, so an idle state from before
                    // this turn started isn't taken for its end.
                    if status == Some("active") || method == "turn/started" {
                        active = true;
                    }
                    let idle = active && status == Some("idle");
                    if (completed || idle) && !ended {
                        if completed && params["turn"]["status"] == "failed" {
                            failed = true;
                        }
                        ended = true;
                        settle_until = Some(Instant::now() + Duration::from_secs(5));
                    }
                    if !KNOWN.contains(&method.as_str()) {
                        drift(turn, &method);
                    }
                }
                Message::Response { id, reply } => {
                    if let Some(line) = steering.remove(&id) {
                        // Too late for this turn: it starts the next one instead.
                        if !matches!(reply, Reply::Result(_)) {
                            waiting.push_front(line);
                            sent -= 1;
                            count(sent);
                        }
                    }
                }
                Message::Gone => {
                    rpc::note(
                        h.out,
                        json!({ "error": "Codex exited in the middle of its turn." }),
                    );
                    return Ok(true);
                }
                message => h.on(agent, message),
            }
            if !ended {
                for line in super::host::new_lines(&path, &mut offset) {
                    let id = agent.request(
                        "turn/steer",
                        json!({
                            "threadId": thread,
                            "input": user_input(&rpc::prompt_text(&line)),
                            "expectedTurnId": turn_id,
                        }),
                    );
                    steering.insert(id, line);
                    sent += 1;
                    count(sent);
                }
            }
            // Once the turn has ended, steers still unanswered get a moment to be refused.
            if ended && (steering.is_empty() || settle_until.is_some_and(|t| Instant::now() > t)) {
                for (_, line) in steering.drain() {
                    waiting.push_front(line);
                    sent -= 1;
                }
                count(sent);
                break;
            }
        }
    }
}

/// Notifications Runo reads or knowingly passes over. Others are noted in `turn/drift`,
/// so a protocol change in a new Codex release shows up early.
const KNOWN: &[&str] = &[
    "error",
    "warning",
    "configWarning",
    "deprecationNotice",
    "thread/started",
    "thread/status/changed",
    "thread/tokenUsage/updated",
    "thread/name/updated",
    "thread/settings/updated",
    "turn/started",
    "turn/completed",
    "turn/diff/updated",
    "turn/plan/updated",
    "item/started",
    "item/completed",
    "item/agentMessage/delta",
    "item/plan/delta",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/summaryPartAdded",
    "item/reasoning/textDelta",
    "item/commandExecution/outputDelta",
    "item/fileChange/outputDelta",
    "item/fileChange/patchUpdated",
    "item/mcpToolCall/progress",
    "serverRequest/resolved",
    "account/rateLimits/updated",
    "mcpServer/startupStatus/updated",
    "remoteControl/status/changed",
    "model/rerouted",
];

/// Note a notification Runo doesn't know, once per method.
fn drift(turn: &Path, method: &str) {
    let path = turn.join("drift");
    let seen = std::fs::read_to_string(&path).unwrap_or_default();
    if seen.lines().any(|l| l == method) {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{method}");
    }
}

/// Reads a Codex app-server turn's `out`.
pub struct CodexAppStream {
    host: HostLines,
    /// Tool items go through the `exec --json` reader, so they look the same.
    exec: JsonStream,
    /// Messages whose text arrived as it was written.
    streamed: HashSet<String>,
    any_text: bool,
    /// The last error shown, so the failed turn that follows doesn't repeat it.
    last_error: String,
}

impl CodexAppStream {
    pub fn new(cwd: &str, fallback: Option<Parser>) -> CodexAppStream {
        CodexAppStream {
            host: HostLines::new(fallback),
            exec: JsonStream::new(cwd),
            streamed: HashSet::new(),
            any_text: false,
            last_error: String::new(),
        }
    }

    pub fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        let message = serde_json::from_str::<Value>(line.trim()).ok();
        if let Some(events) = self.host.feed(line, message.as_ref()) {
            return events;
        }
        let Some(message) = message else {
            return vec![];
        };
        let params = &message["params"];
        match message["method"].as_str().unwrap_or("") {
            "item/agentMessage/delta" => {
                let id = params["itemId"].as_str().unwrap_or("").to_string();
                let first = self.streamed.insert(id);
                let delta = params["delta"].as_str().unwrap_or("");
                vec![self.text(delta, first)]
            }
            method @ ("item/started" | "item/completed") => {
                self.item(&params["item"], method == "item/completed")
            }
            "turn/plan/updated" => crate::stream::todos(&json!({ "plan": params["plan"] }))
                .map(AgentEvent::Todos)
                .into_iter()
                .collect(),
            "thread/tokenUsage/updated" => {
                let usage = &params["tokenUsage"];
                let last = &usage["last"];
                let num = |v: &Value| v.as_u64().unwrap_or(0);
                let cached = num(&last["cachedInputTokens"]);
                let usage = crate::store::Usage {
                    // Codex counts cached tokens inside the input.
                    input: num(&last["inputTokens"]).saturating_sub(cached),
                    cached,
                    output: num(&last["outputTokens"]),
                    window: num(&usage["modelContextWindow"]),
                    cost_cents: 0.0,
                };
                if usage.is_empty() {
                    vec![]
                } else {
                    vec![AgentEvent::Usage(usage)]
                }
            }
            "error" => {
                let text = params["error"]["message"].as_str().unwrap_or("").trim();
                if text.is_empty() {
                    return vec![];
                }
                if params["willRetry"] == true {
                    return vec![AgentEvent::Notice(text.to_string())];
                }
                self.last_error = text.to_string();
                vec![AgentEvent::Error(text.to_string())]
            }
            "turn/completed" if params["turn"]["status"] == "failed" => {
                let text = params["turn"]["error"]["message"]
                    .as_str()
                    .unwrap_or("Codex reported an error.")
                    .trim()
                    .to_string();
                if text == self.last_error {
                    return vec![];
                }
                self.last_error = text.clone();
                vec![AgentEvent::Error(text)]
            }
            "warning" => match params["message"].as_str() {
                Some(text) if !text.trim().is_empty() => {
                    vec![AgentEvent::Notice(text.trim().to_string())]
                }
                _ => vec![],
            },
            _ => vec![],
        }
    }

    /// Reply text, set apart from an earlier message when it starts a new one.
    fn text(&mut self, text: &str, starts: bool) -> AgentEvent {
        let sep = if starts && self.any_text { "\n\n" } else { "" };
        self.any_text = true;
        AgentEvent::Text(format!("{sep}{text}"))
    }

    fn item(&mut self, item: &Value, done: bool) -> Vec<AgentEvent> {
        let id = item["id"].as_str().unwrap_or("");
        let kind = item["type"].as_str().unwrap_or("");
        if kind == "agentMessage" {
            let text = item["text"].as_str().unwrap_or("").trim();
            if !done || self.streamed.contains(id) || text.is_empty() {
                return vec![];
            }
            return vec![self.text(text, true)];
        }
        // Steps that `exec` shows only once they're done are passed on then.
        let live = matches!(kind, "commandExecution" | "mcpToolCall");
        if !done && !live {
            return vec![];
        }
        let Some(exec) = exec_item(item) else {
            return vec![];
        };
        let kind = if done {
            "item.completed"
        } else {
            "item.started"
        };
        self.exec
            .feed(&json!({ "type": kind, "item": exec }).to_string())
    }
}

/// An app-server item in the shape `codex exec --json` prints it.
fn exec_item(item: &Value) -> Option<Value> {
    let status = match item["status"].as_str() {
        Some("inProgress") => "in_progress",
        Some(other) => other,
        None => "completed",
    };
    let id = &item["id"];
    Some(match item["type"].as_str()? {
        "commandExecution" => json!({
            "id": id,
            "type": "command_execution",
            "command": item["command"],
            "aggregated_output": item["aggregatedOutput"],
            "exit_code": item["exitCode"],
            "status": status,
        }),
        "mcpToolCall" => json!({
            "id": id,
            "type": "mcp_tool_call",
            "server": item["server"],
            "tool": item["tool"],
            "status": status,
            "error": item["error"]["message"],
            "result": item["result"],
        }),
        "fileChange" => json!({
            "id": id,
            "type": "file_change",
            "status": status,
            "changes": item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| json!({
                    "path": c["path"],
                    "kind": c["kind"]["type"].as_str().or(c["kind"].as_str()).unwrap_or("update"),
                }))
                .collect::<Vec<_>>(),
        }),
        "webSearch" => json!({ "id": id, "type": "web_search", "query": item["query"] }),
        "reasoning" => {
            let text = item["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            json!({ "id": id, "type": "reasoning", "text": text })
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{StepState, ToolKind, ToolStep};

    fn n(method: &str, params: Value) -> String {
        json!({ "method": method, "params": params }).to_string()
    }

    #[test]
    fn notifications_become_the_same_steps_as_exec() {
        let mut p = CodexAppStream::new("/work/app", None);
        let mut all = Vec::new();
        for line in [
            json!({ "runo": { "session": "01a1" } }).to_string(),
            n(
                "item/started",
                json!({ "item": { "type": "commandExecution", "id": "c1",
                "command": "/bin/zsh -lc 'cargo test'", "status": "inProgress",
                "aggregatedOutput": null, "exitCode": null } }),
            ),
            n(
                "item/completed",
                json!({ "item": { "type": "commandExecution", "id": "c1",
                "command": "/bin/zsh -lc 'cargo test'", "status": "failed",
                "aggregatedOutput": "1 failed", "exitCode": 101 } }),
            ),
            n(
                "item/started",
                json!({ "item": { "type": "fileChange", "id": "f1",
                "changes": [{ "path": "/work/app/src/lib.rs", "kind": { "type": "update", "move_path": null }, "diff": "" }],
                "status": "inProgress" } }),
            ),
            n(
                "item/completed",
                json!({ "item": { "type": "fileChange", "id": "f1",
                "changes": [{ "path": "/work/app/src/lib.rs", "kind": { "type": "update", "move_path": null }, "diff": "" }],
                "status": "completed" } }),
            ),
            n(
                "item/agentMessage/delta",
                json!({ "itemId": "m1", "delta": "Fixed " }),
            ),
            n(
                "item/agentMessage/delta",
                json!({ "itemId": "m1", "delta": "it." }),
            ),
            n(
                "item/completed",
                json!({ "item": { "type": "agentMessage", "id": "m1", "text": "Fixed it." } }),
            ),
            n(
                "item/completed",
                json!({ "item": { "type": "agentMessage", "id": "m2", "text": "Also docs." } }),
            ),
            n(
                "thread/tokenUsage/updated",
                json!({ "tokenUsage": {
                "last": { "inputTokens": 1000, "cachedInputTokens": 800, "outputTokens": 50 },
                "total": {}, "modelContextWindow": 272000 } }),
            ),
        ] {
            all.extend(p.feed(&line));
        }
        let mut step = ToolStep::running("c1", "shell", "Ran cargo test");
        step.state = StepState::Running;
        assert_eq!(all[0], AgentEvent::Conversation("01a1".into()));
        assert_eq!(all[1], AgentEvent::Tool(step));
        assert_eq!(
            all[2],
            AgentEvent::ToolDone {
                id: "c1".into(),
                ok: false,
                output: "1 failed".into()
            }
        );
        let AgentEvent::Tool(edit) = &all[3] else {
            panic!("{all:?}")
        };
        assert_eq!(
            (edit.label.as_str(), edit.kind),
            ("Edited src/lib.rs", ToolKind::Edit)
        );
        assert_eq!(
            all[4..7],
            [
                AgentEvent::Text("Fixed ".into()),
                AgentEvent::Text("it.".into()),
                AgentEvent::Text("\n\nAlso docs.".into()),
            ]
        );
        assert_eq!(
            all[7],
            AgentEvent::Usage(crate::store::Usage {
                input: 200,
                cached: 800,
                output: 50,
                window: 272000,
                cost_cents: 0.0
            })
        );
        assert_eq!(all.len(), 8);
    }

    #[test]
    fn an_error_shows_once_and_retries_are_warnings() {
        let mut p = CodexAppStream::new("/w", None);
        assert_eq!(
            p.feed(&n(
                "error",
                json!({ "error": { "message": "Reconnecting 1/5" }, "willRetry": true })
            )),
            [AgentEvent::Notice("Reconnecting 1/5".into())]
        );
        assert_eq!(
            p.feed(&n(
                "error",
                json!({ "error": { "message": "usage limit reached" }, "willRetry": false })
            )),
            [AgentEvent::Error("usage limit reached".into())]
        );
        assert!(p
            .feed(&n(
                "turn/completed",
                json!({ "turn": { "id": "t", "status": "failed",
                "error": { "message": "usage limit reached" } } })
            ))
            .is_empty());
    }

    #[test]
    fn approvals_follow_the_mode() {
        let cmd = approval(
            "item/commandExecution/requestApproval",
            &json!({ "command": "npm install", "reason": "needs network" }),
        )
        .unwrap();
        assert_eq!((cmd.file, cmd.tool), (false, "Bash"));
        assert_eq!(cmd.input["command"], "npm install");
        assert_eq!(cmd.answer(true), json!({ "decision": "accept" }));
        let legacy = approval(
            "applyPatchApproval",
            &json!({ "fileChanges": { "/x/a.rs": {} } }),
        )
        .unwrap();
        assert!(legacy.file);
        assert_eq!(legacy.input["file_path"], "/x/a.rs");
        assert_eq!(legacy.answer(false), json!({ "decision": "denied" }));
        assert!(approval("item/tool/call", &json!({})).is_none());
    }
}

#[cfg(all(test, unix))]
mod host_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-codex-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A small app server in Python: each turn takes a moment, reports what it was asked
    /// and any steer it got, then completes.
    const SERVER: &str = r#"
import json, sys, threading, time
def out(m): sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
state = {"turn": None, "n": 0, "steers": []}
lock = threading.Lock()
def finish(tid, text):
    time.sleep(0.6)
    with lock:
        steers = state["steers"]; state["steers"] = []; state["turn"] = None
    msg = text + "".join("+" + s for s in steers)
    out({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "m" + tid, "text": msg}}})
    out({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": tid, "status": "completed"}}})
for line in sys.stdin:
    m = json.loads(line)
    method = m.get("method")
    if method == "initialize":
        out({"id": m["id"], "result": {"userAgent": "fake"}})
    elif method == "thread/start":
        out({"id": m["id"], "result": {"thread": {"id": "th1"}}})
    elif method == "turn/start":
        with lock:
            state["n"] += 1; tid = "t%d" % state["n"]; state["turn"] = tid
        out({"id": m["id"], "result": {"turn": {"id": tid}}})
        threading.Thread(target=finish, args=(tid, m["params"]["input"][0]["text"])).start()
    elif method == "turn/steer":
        with lock:
            ok = state["turn"] == m["params"]["expectedTurnId"]
            if ok: state["steers"].append(m["params"]["input"][0]["text"])
        if ok: out({"id": m["id"], "result": {"turnId": m["params"]["expectedTurnId"]}})
        else: out({"id": m["id"], "error": {"code": -32600, "message": "no active turn"}})
"#;

    /// An app server from another release, which opens threads but takes turns in a
    /// shape Runo doesn't send.
    const OTHER: &str = r#"
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("method") == "initialize": print(json.dumps({"id": m["id"], "result": {}}), flush=True)
    elif m.get("method") == "thread/start": print(json.dumps({"id": m["id"], "result": {"thread": {"id": "th1"}}}), flush=True)
    elif m.get("method") == "turn/start": print(json.dumps({"id": m["id"], "error": {"code": -32602, "message": "missing field `items`"}}), flush=True)
"#;

    #[test]
    fn an_app_server_that_refuses_the_turn_falls_back_to_exec() {
        if std::process::Command::new("python3")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }
        let turn = temp("other");
        let script = turn.join("server.py");
        std::fs::write(&script, OTHER).unwrap();
        super::super::host::send(&turn, "go").unwrap();
        let cfg = Config {
            command: vec!["python3".into(), script.to_string_lossy().into()],
            fallback: vec![
                "sh".into(),
                "-c".into(),
                r#"echo '{"type":"item.completed","item":{"id":"i","type":"agent_message","text":"from exec"}}'"#.into(),
            ],
            cwd: turn.to_string_lossy().into(),
            ..Config::default()
        };
        cfg.write(&turn).unwrap();
        assert_eq!(main(&turn), 0);
        let mut p = CodexAppStream::new("/w", Parser::for_provider("codex", "/w"));
        let events: Vec<AgentEvent> = std::fs::read_to_string(turn.join("out"))
            .unwrap()
            .lines()
            .flat_map(|l| p.feed(l))
            .collect();
        assert!(
            events.contains(&AgentEvent::Text("from exec".into())),
            "{events:?}"
        );
        let _ = std::fs::remove_dir_all(turn);
    }

    /// An app server whose turns end with the thread going idle, without the
    /// `turn/completed` Runo knows.
    const IDLE: &str = r#"
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("method") == "initialize": print(json.dumps({"id": m["id"], "result": {}}), flush=True)
    elif m.get("method") == "thread/start": print(json.dumps({"id": m["id"], "result": {"thread": {"id": "th1"}}}), flush=True)
    elif m.get("method") == "turn/start":
        print(json.dumps({"id": m["id"], "result": {"turn": {"id": "t1"}}}), flush=True)
        print(json.dumps({"method": "thread/status/changed", "params": {"threadId": "th1", "status": {"type": "idle"}}}), flush=True)
        print(json.dumps({"method": "thread/status/changed", "params": {"threadId": "th1", "status": {"type": "active"}}}), flush=True)
        print(json.dumps({"method": "turn/finished2", "params": {"turnId": "t1"}}), flush=True)
        print(json.dumps({"method": "thread/status/changed", "params": {"threadId": "th1", "status": {"type": "idle"}}}), flush=True)
"#;

    #[test]
    fn a_turn_ends_when_the_thread_goes_idle_and_new_methods_are_noted() {
        if std::process::Command::new("python3")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }
        let turn = temp("idle");
        let script = turn.join("server.py");
        std::fs::write(&script, IDLE).unwrap();
        super::super::host::send(&turn, "go").unwrap();
        let cfg = Config {
            command: vec!["python3".into(), script.to_string_lossy().into()],
            cwd: turn.to_string_lossy().into(),
            ..Config::default()
        };
        cfg.write(&turn).unwrap();
        assert_eq!(main(&turn), 0);
        assert_eq!(
            std::fs::read_to_string(turn.join("drift")).unwrap(),
            "turn/finished2\n"
        );
        let _ = std::fs::remove_dir_all(turn);
    }

    #[test]
    fn a_turn_takes_steers_and_later_prompts_start_new_turns() {
        if std::process::Command::new("python3")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }
        let turn = temp("steer");
        let script = turn.join("server.py");
        std::fs::write(&script, SERVER).unwrap();
        super::super::host::send(&turn, "first").unwrap();
        let cfg = Config {
            command: vec!["python3".into(), script.to_string_lossy().into()],
            cwd: turn.to_string_lossy().into(),
            ..Config::default()
        };
        cfg.write(&turn).unwrap();
        let steer = {
            let turn = turn.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                super::super::host::send(&turn, "steer").unwrap();
            })
        };
        assert_eq!(main(&turn), 0);
        steer.join().unwrap();
        let mut p = CodexAppStream::new("/w", None);
        let events: Vec<AgentEvent> = std::fs::read_to_string(turn.join("out"))
            .unwrap()
            .lines()
            .flat_map(|l| p.feed(l))
            .collect();
        assert_eq!(
            events,
            [
                AgentEvent::Conversation("th1".into()),
                AgentEvent::Text("first+steer".into()),
            ]
        );
        assert_eq!(std::fs::read_to_string(turn.join("sent")).unwrap(), "2");
        assert!(super::super::host::unsent(&turn).is_empty());
        let _ = std::fs::remove_dir_all(turn);
    }
}
