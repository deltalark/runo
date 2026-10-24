//! The plumbing the agent host's protocol drivers share (`acp`, `codex`, `pi`): an
//! agent process with its stdout read on a thread, numbered requests, answers to the
//! agent's own requests, and the host's lines in the turn's `out`.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::AgentEvent;
use crate::stream::Parser;

/// The turn's `out`, shared with threads that answer the agent.
pub type Out = Arc<Mutex<std::fs::File>>;

/// The agent's input, shared with threads that answer it later.
pub type Input = Arc<Mutex<Option<ChildStdin>>>;

/// What the agent sent.
pub enum Message {
    /// The answer to one of our requests.
    Response { id: u64, reply: Reply },
    /// A request the agent waits on.
    Request {
        method: String,
        id: Value,
        params: Value,
        line: String,
    },
    Notification {
        method: String,
        params: Value,
        line: String,
    },
    /// Nothing within the wait.
    Quiet,
    /// The agent's output ended.
    Gone,
}

/// How the agent answered a request.
pub enum Reply {
    Result(Value),
    Error(String),
    /// It exited first.
    Gone,
    TimedOut,
}

/// A running agent that speaks JSON-RPC on its stdin and stdout.
pub struct Agent {
    child: Child,
    pub input: Input,
    lines: Receiver<String>,
    next: u64,
}

impl Agent {
    /// Start `program` in `cwd` with its stderr in `turn/err`.
    pub fn spawn(turn: &Path, program: &str, args: &[String], cwd: &str) -> Result<Agent, String> {
        let err = std::fs::File::create(turn.join("err")).map_err(|e| e.to_string())?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(err))
            .spawn()
            .map_err(|e| format!("couldn't start {program}: {e}"))?;
        let input = Arc::new(Mutex::new(child.stdin.take()));
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
        Ok(Agent {
            child,
            input,
            lines,
            next: 0,
        })
    }

    /// Send a request; its answer comes as a `Message::Response` with the id returned.
    pub fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        send(
            &self.input,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        );
        id
    }

    /// Send a notification; `Value::Null` sends it without params.
    pub fn notify(&self, method: &str, params: Value) {
        let mut message = json!({ "jsonrpc": "2.0", "method": method });
        if !params.is_null() {
            message["params"] = params;
        }
        send(&self.input, &message);
    }

    /// Send one JSON record as it is, for agents whose protocol isn't JSON-RPC (Pi's).
    pub fn send(&self, record: &Value) {
        send(&self.input, record);
    }

    /// The next line the agent prints, as it is: `Ok(None)` when nothing came within
    /// `wait`, `Err` once its output ended.
    pub fn line(&mut self, wait: Duration) -> Result<Option<String>, ()> {
        match self.lines.recv_timeout(wait) {
            Ok(line) => Ok(Some(line)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(()),
        }
    }

    /// The next thing the agent sends, waiting at most `wait`.
    pub fn next(&mut self, wait: Duration) -> Message {
        let line = match self.line(wait) {
            Ok(Some(line)) => line,
            Ok(None) => return Message::Quiet,
            Err(()) => return Message::Gone,
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            return Message::Quiet;
        };
        let method = message["method"].as_str().map(str::to_string);
        match (method, message.get("id")) {
            (Some(method), Some(id)) => Message::Request {
                method,
                id: id.clone(),
                params: message["params"].clone(),
                line,
            },
            (Some(method), None) => Message::Notification {
                method,
                params: message["params"].clone(),
                line,
            },
            (None, Some(id)) => {
                let Some(id) = id.as_u64() else {
                    return Message::Quiet;
                };
                let reply = match message.get("error") {
                    Some(error) => Reply::Error(
                        error["message"]
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| error.to_string()),
                    ),
                    None => Reply::Result(message["result"].clone()),
                };
                Message::Response { id, reply }
            }
            (None, None) => Message::Quiet,
        }
    }

    /// Wait for the answer to request `id`, passing everything else the agent sends to
    /// `other`.
    pub fn wait(
        &mut self,
        id: u64,
        limit: Option<Duration>,
        other: &mut dyn FnMut(&mut Agent, Message),
    ) -> Reply {
        let start = Instant::now();
        loop {
            let left = match limit {
                Some(limit) => match limit.checked_sub(start.elapsed()) {
                    Some(left) => left,
                    None => return Reply::TimedOut,
                },
                None => Duration::from_secs(3600),
            };
            match self.next(left) {
                Message::Response { id: got, reply } if got == id => return reply,
                Message::Gone => return Reply::Gone,
                Message::Quiet | Message::Response { .. } => {}
                message => other(self, message),
            }
        }
    }

    /// Close the agent's input and let it exit; one that lingers is stopped.
    pub fn close(mut self) {
        self.input.lock().map(|mut s| s.take()).ok();
        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) if start.elapsed() > Duration::from_secs(5) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// Stop the agent now.
    pub fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn send(input: &Mutex<Option<ChildStdin>>, message: &Value) {
    if let Ok(mut input) = input.lock() {
        if let Some(pipe) = input.as_mut() {
            let _ = writeln!(pipe, "{message}").and_then(|_| pipe.flush());
        }
    }
}

pub fn respond(input: &Mutex<Option<ChildStdin>>, id: &Value, result: Value) {
    send(
        input,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    );
}

pub fn respond_error(input: &Mutex<Option<ChildStdin>>, id: &Value, code: i64, message: &str) {
    send(
        input,
        &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    );
}

/// The turn's `out`, new and empty.
pub fn create_out(turn: &Path) -> Option<Out> {
    let file = std::fs::File::create(turn.join("out")).ok()?;
    Some(Arc::new(Mutex::new(file)))
}

/// One line of the agent's output, as it is.
pub fn write_line(out: &Mutex<std::fs::File>, line: &str) {
    if let Ok(mut out) = out.lock() {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// One line for `out` from the host itself: `{"runo": …}`.
pub fn note(out: &Mutex<std::fs::File>, line: Value) {
    write_line(out, &json!({ "runo": line }).to_string());
}

/// Note in `turn/drift`, once each, a message type the driver doesn't know, so a
/// protocol change in a new release of the agent shows up early.
pub fn drift(turn: &Path, kind: &str) {
    let path = turn.join("drift");
    let seen = std::fs::read_to_string(&path).unwrap_or_default();
    if seen.lines().any(|l| l == kind) {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{kind}");
    }
}

/// The prompt in a line of `turn/in`.
pub fn prompt_text(line: &str) -> String {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v["message"]["content"].as_str().map(str::to_string))
        .unwrap_or_else(|| line.to_string())
}

/// Run the turn's print-mode command instead, with its output in `out` as usual. Its
/// first message is the prompt in that command.
pub fn fall_back(turn: &Path, fallback: &[String], cwd: &str, out: &Out, why: &str) -> i32 {
    let Some((program, args)) = fallback.split_first() else {
        note(
            out,
            json!({ "error": format!("The agent couldn't start: {}.", why.trim_end_matches('.')) }),
        );
        return 1;
    };
    note(out, json!({ "fallback": why }));
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
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file));
    if let Ok(err) = err {
        command.stderr(Stdio::from(err));
    }
    match command.status() {
        Ok(status) => crate::harness::host::exit_code(status),
        Err(e) => {
            note(
                out,
                json!({ "error": format!("Runo could not start {program}: {e}") }),
            );
            127
        }
    }
}

/// Reads the host's own lines in `out`, which every driver writes the same way, and
/// what follows a fallback.
pub struct HostLines {
    fallback: Option<Parser>,
    falling_back: bool,
    named: bool,
}

impl HostLines {
    /// `fallback` reads the print-mode output (`None` for plain text).
    pub fn new(fallback: Option<Parser>) -> HostLines {
        HostLines {
            fallback,
            falling_back: false,
            named: false,
        }
    }

    /// The events of `line` when it is past a fallback or is the host's own; `None`
    /// leaves it to the driver's parser.
    pub fn feed(&mut self, line: &str, message: Option<&Value>) -> Option<Vec<AgentEvent>> {
        if self.falling_back {
            return Some(match self.fallback.as_mut() {
                Some(parser) => parser.feed(line),
                None => vec![AgentEvent::Text(format!("{line}\n"))],
            });
        }
        let own = message?.get("runo")?;
        if let Some(sid) = own["session"].as_str() {
            if self.named {
                return Some(vec![]);
            }
            self.named = true;
            return Some(vec![AgentEvent::Conversation(sid.to_string())]);
        }
        if let Some(text) = own["notice"].as_str() {
            return Some(vec![AgentEvent::Notice(text.to_string())]);
        }
        if let Some(text) = own["error"].as_str() {
            return Some(vec![AgentEvent::Error(text.to_string())]);
        }
        if own.get("fallback").is_some() {
            self.falling_back = true;
            return Some(vec![]);
        }
        // Drivers read their own end-of-prompt lines.
        None
    }
}
