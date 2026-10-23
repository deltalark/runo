//! The Pi driver. Pi turns run under Runo's agent host as `pi --mode rpc`, Pi's own
//! protocol of JSON records on stdin and stdout, instead of one `pi --mode json` run per
//! prompt: `Runo --agent-host runo-turn <turn folder> --pi` reads `pi.json` from the
//! turn folder and sends each prompt from `in`. A message sent while Pi works steers it
//! (`steer`), so Pi reads it once its current tool call is done.
//!
//! Pi asks nothing before it acts; Runo's mode picks the tools it is given instead (see
//! `agent::provider_args`). Its events go to `out` as they are and `PiStream` reads
//! them, the same records `--mode json` prints, so the fallback reads the same way.
//! When the installed Pi has no RPC mode, or it can't start, the host runs the
//! `--mode json` command instead.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant};

use super::rpc::{self, Agent, HostLines, Out};
use super::{AgentEvent, ToolStep};
use crate::stream::Parser;

/// The turn's settings, written by Runo into the turn folder.
pub const CONFIG: &str = "pi.json";

/// How long Pi has to start and answer.
const START: Duration = Duration::from_secs(60);
/// After a run settles, how long a message sent just then still joins the session.
const GRACE: Duration = Duration::from_millis(1500);
/// How long an answer to a steer or a queue check may take once the run has settled.
const SETTLE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// `pi --mode rpc` and the turn's flags (model, thinking, tools, session).
    pub command: Vec<String>,
    /// The `pi --mode json` command with the prompt in it, run when RPC can't start.
    #[serde(default)]
    pub fallback: Vec<String>,
    pub cwd: String,
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

/// `Runo --agent-host <mark> <turn folder> --pi`. Returns the exit code, which is also
/// written to `turn/exit`.
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

/// What came back while waiting for an answer.
enum Answer {
    Got(Value),
    Gone,
    TimedOut,
}

/// A Pi process and what it is in the middle of.
struct Pi<'a> {
    agent: Agent,
    turn: &'a Path,
    out: &'a Out,
    next: u64,
    /// The run started by the last prompt has settled.
    settled: bool,
    /// A run ended on an error.
    failed: bool,
    /// Steers sent during this run that Pi hasn't answered yet, by id.
    steering: Vec<(String, String)>,
    /// Steers Pi took during this run, which it may not have read yet when it settles.
    steered: Vec<String>,
    /// Lines that go back to the front of the queue: steers Pi refused or never read.
    back: Vec<String>,
}

impl Pi<'_> {
    /// Send `command` with a new id, which is returned.
    fn send(&mut self, kind: &str, mut command: Value) -> String {
        self.next += 1;
        let id = format!("runo-{kind}-{}", self.next);
        command["id"] = id.clone().into();
        command["type"] = kind.into();
        self.agent.send(&command);
        id
    }

    /// Read what Pi prints until the answer to `id`, or for `wait` when `id` is empty.
    fn pump(&mut self, id: &str, wait: Duration) -> Answer {
        let start = Instant::now();
        loop {
            let Some(left) = wait.checked_sub(start.elapsed()) else {
                return Answer::TimedOut;
            };
            let line = match self.agent.line(left.min(Duration::from_millis(100))) {
                Ok(Some(line)) => line,
                Ok(None) if id.is_empty() => return Answer::TimedOut,
                Ok(None) => continue,
                Err(()) => return Answer::Gone,
            };
            let Ok(record) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if record["type"] == "response" {
                let got = record["id"].as_str().unwrap_or("");
                if got == id {
                    return Answer::Got(record);
                }
                self.answered(got, &record);
                continue;
            }
            self.event(&record, &line);
        }
    }

    /// An answer to something sent earlier: a steer Pi refused goes back to the queue.
    fn answered(&mut self, id: &str, record: &Value) {
        let Some(at) = self.steering.iter().position(|(sid, _)| sid == id) else {
            return;
        };
        let (_, line) = self.steering.remove(at);
        if record["success"] == true && record["data"]["disposition"] != "handled" {
            self.steered.push(line);
        } else if record["success"] != true {
            self.back.push(line);
        }
    }

    /// One event: to `out`, answered when it asks something, noted when it is new.
    fn event(&mut self, record: &Value, line: &str) {
        let kind = record["type"].as_str().unwrap_or("");
        match kind {
            "agent_settled" => self.settled = true,
            "message_end"
                if record["message"]["role"] == "assistant"
                    && record["message"]["stopReason"] == "error" =>
            {
                self.failed = true;
            }
            "extension_ui_request" => {
                // Runo shows no extension dialogs yet; one that waits is turned down, so
                // the extension goes on without an answer.
                let waits = matches!(
                    record["method"].as_str(),
                    Some("select" | "confirm" | "input" | "editor")
                );
                if waits {
                    self.agent.send(&json!({
                        "type": "extension_ui_response",
                        "id": record["id"],
                        "cancelled": true,
                    }));
                }
            }
            _ => {}
        }
        rpc::write_line(self.out, line);
        if !KNOWN.contains(&kind) {
            rpc::drift(self.turn, kind);
        }
    }
}

/// Events Runo reads or knowingly passes over. Others are noted in `turn/drift`.
pub(crate) const KNOWN: &[&str] = &[
    "agent_start",
    "agent_end",
    "agent_settled",
    "turn_start",
    "turn_end",
    "message_start",
    "message_update",
    "message_end",
    "tool_execution_start",
    "tool_execution_update",
    "tool_execution_end",
    "queue_update",
    "entry_appended",
    "session_info_changed",
    "thinking_level_changed",
    "compaction_start",
    "compaction_end",
    "auto_retry_start",
    "auto_retry_end",
    "summarization_retry_scheduled",
    "summarization_retry_attempt_start",
    "summarization_retry_finished",
    "extension_error",
    "extension_ui_request",
];

/// Start Pi and run the prompts. `Err` means Pi couldn't start or refused the first
/// prompt, so nothing ran yet and the fallback can run instead.
fn session(turn: &Path, config: &Config, out: &Out) -> Result<i32, String> {
    let (program, args) = config
        .command
        .split_first()
        .ok_or("Pi has no RPC command")?;
    let agent = Agent::spawn(turn, program, args, &config.cwd)?;
    let mut pi = Pi {
        agent,
        turn,
        out,
        next: 0,
        settled: false,
        failed: false,
        steering: Vec::new(),
        steered: Vec::new(),
        back: Vec::new(),
    };
    let id = pi.send("get_state", json!({}));
    let state = match pi.pump(&id, START) {
        Answer::Got(r) if r["success"] == true => r,
        Answer::Got(r) => {
            pi.agent.kill();
            return Err(format!(
                "its RPC mode refused to start: {}",
                r["error"].as_str().unwrap_or("no reason given")
            ));
        }
        Answer::Gone => return Err("it has no RPC mode".into()),
        Answer::TimedOut => {
            pi.agent.kill();
            return Err("its RPC mode didn't answer".into());
        }
    };
    if let Some(sid) = state["data"]["sessionId"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        rpc::note(out, json!({ "session": sid }));
    }
    match prompts(&mut pi) {
        Ok(failed) => {
            pi.agent.close();
            Ok(if failed { 1 } else { 0 })
        }
        Err(why) => {
            pi.agent.kill();
            Err(why)
        }
    }
}

/// Send each prompt from `in`, steering the running one with messages that arrive while
/// Pi works. Returns true when a run failed.
fn prompts(pi: &mut Pi) -> Result<bool, String> {
    let path = pi.turn.join("in");
    let mut offset = 0u64;
    let mut waiting: VecDeque<String> = VecDeque::new();
    let mut sent = 0u64;
    let mut runs = 0u32;
    let count = |turn: &Path, sent: u64| {
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
            return Ok(pi.failed);
        };
        pi.settled = false;
        let id = pi.send("prompt", json!({ "message": rpc::prompt_text(&line) }));
        sent += 1;
        count(pi.turn, sent);
        match pi.pump(&id, START) {
            // A Pi whose RPC mode doesn't take Runo's first prompt (a protocol change in
            // a newer or older release) runs the turn in JSON mode instead.
            Answer::Got(r) if r["success"] != true && runs == 0 => {
                return Err(format!(
                    "its RPC mode refused the prompt: {}",
                    r["error"].as_str().unwrap_or("no reason given")
                ));
            }
            Answer::Got(r) if r["success"] != true => {
                let why = r["error"].as_str().unwrap_or("Pi refused the message.");
                rpc::note(pi.out, json!({ "error": why }));
                pi.failed = true;
                continue;
            }
            // An extension or command took it; no run started.
            Answer::Got(r) if r["data"]["disposition"] == "handled" => continue,
            Answer::Got(_) => {}
            Answer::Gone | Answer::TimedOut => {
                rpc::note(
                    pi.out,
                    json!({ "error": "Pi stopped answering before the turn started." }),
                );
                return Ok(true);
            }
        }
        runs += 1;
        while !pi.settled {
            if let Answer::Gone = pi.pump("", Duration::from_millis(100)) {
                rpc::note(
                    pi.out,
                    json!({ "error": "Pi exited in the middle of its turn." }),
                );
                return Ok(true);
            }
            if pi.settled {
                break;
            }
            for line in super::host::new_lines(&path, &mut offset) {
                let id = pi.send("steer", json!({ "message": rpc::prompt_text(&line) }));
                pi.steering.push((id, line));
                sent += 1;
                count(pi.turn, sent);
            }
        }
        // Steers Pi hadn't read when it settled stay in its queue; they come back out
        // and start the next run instead.
        let start = Instant::now();
        while !pi.steering.is_empty() && start.elapsed() < SETTLE {
            if let Answer::Gone = pi.pump("", Duration::from_millis(100)) {
                break;
            }
        }
        let unanswered: Vec<String> = pi.steering.drain(..).map(|(_, line)| line).collect();
        pi.back.extend(unanswered);
        if !pi.steered.is_empty() {
            let id = pi.send("clear_queue", json!({}));
            if let Answer::Got(r) = pi.pump(&id, SETTLE) {
                let left: Vec<&str> = ["steering", "followUp"]
                    .iter()
                    .flat_map(|k| r["data"][*k].as_array().into_iter().flatten())
                    .filter_map(Value::as_str)
                    .collect();
                for text in left {
                    if let Some(at) = pi.steered.iter().position(|l| rpc::prompt_text(l) == text) {
                        pi.back.push(pi.steered.remove(at));
                    }
                }
            }
            pi.steered.clear();
        }
        for line in pi.back.drain(..).rev() {
            waiting.push_front(line);
            sent -= 1;
        }
        count(pi.turn, sent);
    }
}

/// Reads a Pi turn's `out`: the events of RPC mode or of `--mode json`.
pub struct PiStream {
    host: HostLines,
    cwd: String,
    /// Text of the current message arrived as it was written.
    streamed: bool,
    any_text: bool,
    /// The turn's cost so far, in cents.
    cost: f64,
    last_error: String,
    named: bool,
}

impl PiStream {
    pub fn new(cwd: &str) -> PiStream {
        PiStream {
            // Pi's fallback prints the same events, so a fallback reads as itself.
            host: HostLines::new(Some(Parser::Pi(Box::new(PiStream::bare(cwd))))),
            ..PiStream::bare(cwd)
        }
    }

    fn bare(cwd: &str) -> PiStream {
        PiStream {
            host: HostLines::new(None),
            cwd: cwd.to_string(),
            streamed: false,
            any_text: false,
            cost: 0.0,
            last_error: String::new(),
            named: false,
        }
    }

    pub fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        let record = serde_json::from_str::<Value>(line.trim()).ok();
        if let Some(events) = self.host.feed(line, record.as_ref()) {
            if events
                .iter()
                .any(|e| matches!(e, AgentEvent::Conversation(_)))
            {
                self.named = true;
            }
            return events;
        }
        let Some(record) = record else {
            return vec![];
        };
        match record["type"].as_str().unwrap_or("") {
            // `--mode json` starts with the session it runs in.
            "session" => match record["id"].as_str() {
                Some(id) if !self.named => {
                    self.named = true;
                    vec![AgentEvent::Conversation(id.to_string())]
                }
                _ => vec![],
            },
            "message_start" => {
                self.streamed = false;
                vec![]
            }
            "message_update" => {
                let update = &record["assistantMessageEvent"];
                match (update["type"].as_str(), update["delta"].as_str()) {
                    (Some("text_delta"), Some(delta)) => {
                        let starts = !self.streamed;
                        self.streamed = true;
                        vec![self.text(delta, starts)]
                    }
                    _ => vec![],
                }
            }
            "message_end" if record["message"]["role"] == "assistant" => {
                self.message_end(&record["message"])
            }
            "tool_execution_start" => {
                let tool = record["toolName"].as_str().unwrap_or("tool");
                let label = crate::stream::step_label(&self.cwd, tool, &record["args"]);
                let id = record["toolCallId"].as_str().unwrap_or("");
                vec![AgentEvent::Tool(ToolStep::running(id, tool, label))]
            }
            "tool_execution_end" => vec![AgentEvent::ToolDone {
                id: record["toolCallId"].as_str().unwrap_or("").to_string(),
                ok: record["isError"] != true,
                output: crate::stream::result_text(&record["result"]["content"]),
            }],
            "auto_retry_start" => {
                let why = record["errorMessage"].as_str().unwrap_or("an error");
                vec![AgentEvent::Notice(format!("Pi is retrying after {why}."))]
            }
            "auto_retry_end" if record["success"] == false => match record["finalError"].as_str() {
                Some(text) => self.error(text),
                None => vec![],
            },
            "compaction_end" => match record["errorMessage"].as_str() {
                Some(text) => vec![AgentEvent::Notice(format!(
                    "Pi couldn't compact the conversation: {text}"
                ))],
                None => vec![],
            },
            "extension_error" => match record["error"].as_str() {
                Some(text) => vec![AgentEvent::Notice(format!("A Pi extension failed: {text}"))],
                None => vec![],
            },
            "extension_ui_request" if record["method"] == "notify" => {
                match record["message"].as_str() {
                    Some(text) if !text.trim().is_empty() => {
                        vec![AgentEvent::Notice(text.trim().to_string())]
                    }
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }

    /// An assistant message is complete: its text when none streamed, its usage, and
    /// its error.
    fn message_end(&mut self, message: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        if !self.streamed {
            let text: String = message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            if !text.trim().is_empty() {
                events.push(self.text(text.trim(), true));
            }
        }
        self.streamed = false;
        let usage = &message["usage"];
        let num = |k: &str| usage[k].as_u64().unwrap_or(0);
        self.cost += usage["cost"]["total"].as_f64().unwrap_or(0.0) * 100.0;
        let usage = crate::store::Usage {
            input: num("input"),
            cached: num("cacheRead") + num("cacheWrite"),
            output: num("output"),
            window: 0,
            cost_cents: self.cost,
        };
        if !usage.is_empty() {
            events.push(AgentEvent::Usage(usage));
        }
        if message["stopReason"] == "error" {
            let text = message["errorMessage"]
                .as_str()
                .unwrap_or("Pi reported an error.");
            events.extend(self.error(text));
        }
        events
    }

    fn error(&mut self, text: &str) -> Vec<AgentEvent> {
        let text = text.trim();
        if text.is_empty() || text == self.last_error {
            return vec![];
        }
        self.last_error = text.to_string();
        vec![AgentEvent::Error(text.to_string())]
    }

    /// Reply text, set apart from an earlier message when it starts a new one.
    fn text(&mut self, text: &str, starts: bool) -> AgentEvent {
        let sep = if starts && self.any_text { "\n\n" } else { "" };
        self.any_text = true;
        AgentEvent::Text(format!("{sep}{text}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::StepState;

    fn feed(p: &mut PiStream, records: &[Value]) -> Vec<AgentEvent> {
        records
            .iter()
            .flat_map(|r| p.feed(&r.to_string()))
            .collect()
    }

    #[test]
    fn a_json_mode_run_becomes_events() {
        let mut p = PiStream::new("/w");
        let events = feed(
            &mut p,
            &[
                json!({"type":"session","version":3,"id":"s-1","cwd":"/w"}),
                json!({"type":"agent_start"}),
                json!({"type":"message_start","message":{"role":"user","content":"hi"}}),
                json!({"type":"message_end","message":{"role":"user","content":"hi"}}),
                json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
                json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Look"}}),
                json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"ing."}}),
                json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Looking."}],
                    "usage":{"input":100,"output":5,"cacheRead":20,"cacheWrite":0,"cost":{"total":0.01}},"stopReason":"toolUse"}}),
                json!({"type":"tool_execution_start","toolCallId":"c1","toolName":"read","args":{"path":"/w/src/a.rs"}}),
                json!({"type":"tool_execution_end","toolCallId":"c1","toolName":"read","result":{"content":[{"type":"text","text":"fn a() {}"}]},"isError":false}),
                json!({"type":"tool_execution_start","toolCallId":"c2","toolName":"bash","args":{"command":"false"}}),
                json!({"type":"tool_execution_end","toolCallId":"c2","toolName":"bash","result":{"content":[{"type":"text","text":"exit 1"}]},"isError":true}),
                json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
                json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Done."}],
                    "usage":{"input":150,"output":3,"cacheRead":0,"cacheWrite":0,"cost":{"total":0.02}},"stopReason":"stop"}}),
                json!({"type":"agent_end","messages":[]}),
                json!({"type":"agent_settled","aborted":false}),
            ],
        );
        let mut read = ToolStep::running("c1", "read", "Read src/a.rs");
        read.state = StepState::Running;
        assert_eq!(events[0], AgentEvent::Conversation("s-1".into()));
        assert_eq!(events[1], AgentEvent::Text("Look".into()));
        assert_eq!(events[2], AgentEvent::Text("ing.".into()));
        assert!(
            matches!(&events[3], AgentEvent::Usage(u) if u.input == 100 && u.cached == 20 && (u.cost_cents - 1.0).abs() < 1e-9)
        );
        assert_eq!(events[4], AgentEvent::Tool(read));
        assert_eq!(
            events[5],
            AgentEvent::ToolDone {
                id: "c1".into(),
                ok: true,
                output: "fn a() {}".into()
            }
        );
        assert!(matches!(&events[6], AgentEvent::Tool(s) if s.label == "Ran false"));
        assert!(matches!(&events[7], AgentEvent::ToolDone { ok: false, .. }));
        assert_eq!(events[8], AgentEvent::Text("\n\nDone.".into()));
        assert!(matches!(&events[9], AgentEvent::Usage(u) if (u.cost_cents - 3.0).abs() < 1e-9));
        assert_eq!(events.len(), 10, "{events:?}");
    }

    #[test]
    fn an_error_shows_once_and_retries_are_notices() {
        let mut p = PiStream::new("/w");
        let events = feed(
            &mut p,
            &[
                json!({"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":2000,"errorMessage":"529 overloaded"}),
                json!({"type":"message_end","message":{"role":"assistant","content":[],"usage":{},"stopReason":"error","errorMessage":"529 overloaded"}}),
                json!({"type":"auto_retry_end","success":false,"attempt":3,"finalError":"529 overloaded"}),
            ],
        );
        assert_eq!(
            events,
            [
                AgentEvent::Notice("Pi is retrying after 529 overloaded.".into()),
                AgentEvent::Error("529 overloaded".into()),
            ]
        );
    }

    #[test]
    fn the_hosts_session_names_the_conversation_once() {
        let mut p = PiStream::new("/w");
        let events = feed(
            &mut p,
            &[
                json!({"runo":{"session":"s-9"}}),
                json!({"type":"session","id":"s-9"}),
            ],
        );
        assert_eq!(events, [AgentEvent::Conversation("s-9".into())]);
    }
}

#[cfg(all(test, unix))]
mod host_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-pi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn python() -> bool {
        std::process::Command::new("python3")
            .arg("-V")
            .output()
            .is_ok()
    }

    /// A small Pi RPC mode in Python: each prompt runs for a moment and replies with
    /// what it was asked and any steer it read, which it reads only while it runs; a
    /// steer that comes after stays queued until `clear_queue`.
    const PI: &str = r#"
import json, sys, threading, time
def out(m): sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
lock = threading.Lock()
state = {"running": False, "queue": []}
def run(text):
    time.sleep(0.6)
    with lock:
        steers = state["queue"]; state["queue"] = []
    msg = text + "".join("+" + s for s in steers)
    out({"type": "message_start", "message": {"role": "assistant", "content": []}})
    out({"type": "message_end", "message": {"role": "assistant", "content": [{"type": "text", "text": msg}], "usage": {}, "stopReason": "stop"}})
    out({"type": "brand_new_event"})
    with lock:
        state["running"] = False
    out({"type": "agent_settled", "aborted": False})
for line in sys.stdin:
    m = json.loads(line)
    t = m["type"]
    if t == "get_state":
        out({"id": m["id"], "type": "response", "command": t, "success": True, "data": {"sessionId": "pi-s1"}})
    elif t == "prompt":
        with lock:
            state["running"] = True
        out({"id": m["id"], "type": "response", "command": t, "success": True, "data": {"disposition": "started"}})
        threading.Thread(target=run, args=(m["message"],)).start()
    elif t == "steer":
        with lock:
            state["queue"].append(m["message"])
        out({"id": m["id"], "type": "response", "command": t, "success": True, "data": {"disposition": "queued"}})
    elif t == "clear_queue":
        with lock:
            left = state["queue"]; state["queue"] = []
        out({"id": m["id"], "type": "response", "command": t, "success": True, "data": {"steering": left, "followUp": []}})
"#;

    fn events(turn: &Path) -> Vec<AgentEvent> {
        let mut p = PiStream::new("/w");
        std::fs::read_to_string(turn.join("out"))
            .unwrap()
            .lines()
            .flat_map(|l| p.feed(l))
            .collect()
    }

    #[test]
    fn a_steer_reaches_the_running_prompt() {
        if !python() {
            return;
        }
        let turn = temp("steer");
        let script = turn.join("pi.py");
        std::fs::write(&script, PI).unwrap();
        super::super::host::send(&turn, "first").unwrap();
        Config {
            command: vec!["python3".into(), script.to_string_lossy().into()],
            cwd: turn.to_string_lossy().into(),
            ..Config::default()
        }
        .write(&turn)
        .unwrap();
        let steer = {
            let turn = turn.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                super::super::host::send(&turn, "steer").unwrap();
            })
        };
        assert_eq!(main(&turn), 0);
        steer.join().unwrap();
        assert_eq!(
            events(&turn),
            [
                AgentEvent::Conversation("pi-s1".into()),
                AgentEvent::Text("first+steer".into()),
            ]
        );
        assert_eq!(std::fs::read_to_string(turn.join("sent")).unwrap(), "2");
        assert_eq!(
            std::fs::read_to_string(turn.join("drift")).unwrap(),
            "brand_new_event\n"
        );
        let _ = std::fs::remove_dir_all(turn);
    }

    /// A Pi without RPC mode, which exits at once.
    #[test]
    fn a_pi_without_rpc_mode_falls_back_to_json_mode() {
        if !python() {
            return;
        }
        let turn = temp("fallback");
        super::super::host::send(&turn, "go").unwrap();
        Config {
            command: vec!["python3".into(), "-c".into(), "import sys; sys.exit(2)".into()],
            fallback: vec![
                "sh".into(),
                "-c".into(),
                r#"echo '{"type":"session","id":"j1"}'; echo '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"from json mode"}],"usage":{},"stopReason":"stop"}}'"#.into(),
            ],
            cwd: turn.to_string_lossy().into(),
        }
        .write(&turn)
        .unwrap();
        assert_eq!(main(&turn), 0);
        assert_eq!(
            events(&turn),
            [
                AgentEvent::Conversation("j1".into()),
                AgentEvent::Text("from json mode".into()),
            ]
        );
        let _ = std::fs::remove_dir_all(turn);
    }
}
