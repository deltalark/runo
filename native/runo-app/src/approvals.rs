//! Live approvals. Claude Code asks Runo's MCP server (its permission prompt tool) before
//! it uses a tool it isn't allowed, and the turn waits while the person answers in the
//! corner card or the Inbox. Requests and answers are small files in one folder, so the
//! server process, a connection's relay and the app all meet there.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The permission prompt tool's name in Runo's MCP server.
pub const TOOL: &str = "permission_prompt";

/// One question to the person: may the agent working in `dir` use `tool` with `input`?
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    /// The session folder the agent works in.
    pub dir: String,
    pub tool: String,
    #[serde(default)]
    pub input: Value,
    /// When it was asked, in seconds since 1970.
    pub asked: u64,
    /// How long the agent waits for an answer, in seconds.
    pub wait: u64,
}

impl Request {
    /// Whether nobody can be waiting for this any more.
    pub fn expired(&self, now: u64) -> bool {
        now > self.asked + self.wait + 60
    }
}

/// The answer, in the shape Claude Code reads from its permission prompt tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "behavior", rename_all = "lowercase")]
pub enum Answer {
    Allow {
        #[serde(rename = "updatedInput")]
        updated_input: Value,
    },
    Deny {
        message: String,
    },
}

/// Where requests and answers wait.
pub fn folder() -> PathBuf {
    crate::store::data_dir().join("approvals")
}

/// How long "wait indefinitely" waits: long enough that the turn ends some other way.
const FOREVER: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// How long an agent waits for the person (Settings > Agents).
pub fn wait() -> Duration {
    wait_for(crate::settings::AppSettings::load().approval_wait_mins)
}

/// The wait for a setting in minutes, where 0 means no limit.
fn wait_for(mins: u32) -> Duration {
    match mins {
        0 => FOREVER,
        mins => Duration::from_secs(mins.clamp(1, 24 * 60) as u64 * 60),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{nanos}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Write under a temporary name and rename, so nobody reads half a file.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

fn read_answer(path: &Path) -> Option<Answer> {
    let text = std::fs::read(path).ok()?;
    serde_json::from_slice(&text).ok()
}

/// Whether a relayed MCP request is a call to the permission prompt tool, which can wait
/// minutes for the person and so is answered on its own thread.
pub fn is_ask(request: &str) -> bool {
    request.contains(&format!("\"{TOOL}\""))
}

/// The message an unanswered request ends with. It says the permission wasn't granted,
/// which is what a detached turn stops at when it should.
pub fn timed_out(wait: Duration) -> String {
    let mins = (wait.as_secs() / 60).max(1);
    format!(
        "The person hasn't granted this: nobody answered in Runo within {mins} minute{}. Carry on without it if you can, or end your turn and say what you need allowed.",
        if mins == 1 { "" } else { "s" }
    )
}

/// Ask the person and wait for the answer, at most `wait`.
pub fn ask(base: &Path, dir: &str, tool: &str, input: &Value, wait: Duration) -> Answer {
    // Plans and questions are shown in Runo after the turn, as before.
    match tool {
        "ExitPlanMode" => {
            return Answer::Deny {
                message: "Runo shows this plan to the person for approval. Stop here; they answer in their next message.".into(),
            }
        }
        "AskUserQuestion" => {
            return Answer::Deny {
                message: "Runo shows these questions to the person. Stop here; they answer in their next message.".into(),
            }
        }
        _ => {}
    }
    let request = Request {
        id: new_id(),
        dir: dir.to_string(),
        tool: tool.to_string(),
        input: input.clone(),
        asked: now(),
        wait: wait.as_secs(),
    };
    let path = base.join(format!("{}.json", request.id));
    let answered = base.join(format!("{}.answer", request.id));
    let written = serde_json::to_vec(&request)
        .map_err(std::io::Error::other)
        .and_then(|data| write_atomic(&path, &data));
    if let Err(e) = written {
        return Answer::Deny {
            message: format!(
                "Runo couldn't ask the person for permission ({e}), so this was not allowed."
            ),
        };
    }
    let start = Instant::now();
    loop {
        if let Some(answer) = read_answer(&answered) {
            let _ = std::fs::remove_file(&answered);
            let _ = std::fs::remove_file(&path);
            return answer;
        }
        if !path.exists() {
            // Answered just now, or withdrawn because the turn ended.
            let answer = read_answer(&answered).unwrap_or(Answer::Deny {
                message: "The request was withdrawn in Runo, so this was not allowed.".into(),
            });
            let _ = std::fs::remove_file(&answered);
            return answer;
        }
        if start.elapsed() >= wait {
            let _ = std::fs::remove_file(&path);
            return Answer::Deny {
                message: timed_out(wait),
            };
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The requests waiting for an answer, oldest first.
pub fn pending(base: &Path) -> Vec<Request> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut out: Vec<Request> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| serde_json::from_slice(&std::fs::read(p).ok()?).ok())
        .collect();
    out.sort_by(|a, b| (a.asked, &a.id).cmp(&(b.asked, &b.id)));
    out
}

/// Answer a request. The answer is written before the request goes, so the agent's side
/// never takes a vanished request for a withdrawn one. A request nobody waits for any
/// more (it timed out) is not found.
pub fn answer(base: &Path, id: &str, answer: &Answer) -> std::io::Result<()> {
    if !base.join(format!("{id}.json")).is_file() {
        return Err(std::io::ErrorKind::NotFound.into());
    }
    let data = serde_json::to_vec(answer).map_err(std::io::Error::other)?;
    write_atomic(&base.join(format!("{id}.answer")), &data)?;
    let _ = std::fs::remove_file(base.join(format!("{id}.json")));
    Ok(())
}

/// Take a request back unanswered: its turn ended, or nobody waits for it any more.
pub fn withdraw(base: &Path, id: &str) {
    let _ = std::fs::remove_file(base.join(format!("{id}.json")));
    let _ = std::fs::remove_file(base.join(format!("{id}.answer")));
}

/// The MCP tool result for an answer: its JSON as text, as Claude Code expects.
pub fn result(answer: &Answer) -> Value {
    json!({ "content": [{ "type": "text", "text": serde_json::to_string(answer).unwrap_or_default() }] })
}

/// Handle a call to the permission prompt tool from the agent working in `dir`.
pub fn call(base: &Path, dir: &str, args: &Value, wait: Duration) -> Value {
    let tool = args["tool_name"].as_str().unwrap_or("a tool");
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    result(&ask(base, dir, tool, &input, wait))
}

/// What the agent wants to do, for the card: "run `cargo test`", "edit src/main.rs".
pub fn describe(tool: &str, input: &Value) -> String {
    let field = |k: &str| {
        input[k]
            .as_str()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let short = |t: String| {
        let line = t.lines().next().unwrap_or("").to_string();
        let more = t.lines().nth(1).is_some();
        let mut cut: String = line.chars().take(120).collect();
        if more || cut.len() < line.len() {
            cut.push('…');
        }
        cut
    };
    let what = match tool {
        "Bash" => field("command").map(|c| format!("run `{}`", short(c))),
        "Write" => field("file_path").map(|p| format!("write {p}")),
        "Edit" | "MultiEdit" => field("file_path").map(|p| format!("edit {p}")),
        "NotebookEdit" => field("notebook_path").map(|p| format!("edit {p}")),
        "WebFetch" => field("url").map(|u| format!("fetch {u}")),
        "WebSearch" => field("query").map(|q| format!("search the web for “{}”", short(q))),
        _ => None,
    };
    what.unwrap_or_else(|| match tool.strip_prefix("mcp__") {
        Some(rest) => match rest.split_once("__") {
            Some((server, name)) => format!("use {name} from {server}"),
            None => format!("use {tool}"),
        },
        None => format!("use {tool}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_minutes_waits_indefinitely() {
        assert_eq!(wait_for(5), Duration::from_secs(300));
        assert_eq!(wait_for(100_000), Duration::from_secs(24 * 60 * 60));
        assert!(wait_for(0) >= Duration::from_secs(7 * 24 * 60 * 60));
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "runo-approvals-{name}-{}-{}",
            std::process::id(),
            new_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn answers_have_claude_codes_shape() {
        let allow = Answer::Allow {
            updated_input: json!({ "command": "ls" }),
        };
        assert_eq!(
            serde_json::to_value(&allow).unwrap(),
            json!({ "behavior": "allow", "updatedInput": { "command": "ls" } })
        );
        let deny = Answer::Deny {
            message: "no".into(),
        };
        assert_eq!(
            serde_json::to_value(&deny).unwrap(),
            json!({ "behavior": "deny", "message": "no" })
        );
        let text = result(&deny)["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(serde_json::from_str::<Answer>(&text).unwrap(), deny);
    }

    #[test]
    fn a_request_waits_for_the_persons_answer() {
        let base = temp("ask");
        let asker = {
            let base = base.clone();
            std::thread::spawn(move || {
                call(
                    &base,
                    "/work/app",
                    &json!({ "tool_name": "Bash", "input": { "command": "cargo test" } }),
                    Duration::from_secs(20),
                )
            })
        };
        let request = loop {
            if let Some(r) = pending(&base).pop() {
                break r;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(request.dir, "/work/app");
        assert_eq!(request.tool, "Bash");
        assert_eq!(describe(&request.tool, &request.input), "run `cargo test`");
        answer(
            &base,
            &request.id,
            &Answer::Allow {
                updated_input: request.input.clone(),
            },
        )
        .unwrap();
        let reply = asker.join().unwrap();
        let text = reply["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!({ "behavior": "allow", "updatedInput": { "command": "cargo test" } })
        );
        assert!(pending(&base).is_empty());
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0);
        // Too late: the agent stopped waiting.
        assert!(answer(
            &base,
            &request.id,
            &Answer::Deny {
                message: String::new()
            }
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn unanswered_and_withdrawn_requests_are_denied() {
        let base = temp("deny");
        let answer = ask(&base, "/w", "Bash", &json!({}), Duration::from_millis(300));
        assert!(matches!(&answer, Answer::Deny { message } if message.contains("hasn't granted")));
        assert!(pending(&base).is_empty());

        let asker = {
            let base = base.clone();
            std::thread::spawn(move || {
                ask(&base, "/w", "Write", &json!({}), Duration::from_secs(20))
            })
        };
        let request = loop {
            if let Some(r) = pending(&base).pop() {
                break r;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        withdraw(&base, &request.id);
        assert!(
            matches!(asker.join().unwrap(), Answer::Deny { message } if message.contains("withdrawn"))
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn plans_and_questions_stay_with_the_chat() {
        let base = temp("plan");
        for tool in ["ExitPlanMode", "AskUserQuestion"] {
            assert!(matches!(
                ask(&base, "/w", tool, &json!({}), Duration::from_secs(5)),
                Answer::Deny { .. }
            ));
        }
        assert!(pending(&base).is_empty());
    }

    #[test]
    fn requests_read_plainly() {
        assert_eq!(
            describe("Edit", &json!({ "file_path": "src/main.rs" })),
            "edit src/main.rs"
        );
        assert_eq!(
            describe("Bash", &json!({ "command": "npm install\nnpm test" })),
            "run `npm install…`"
        );
        assert_eq!(
            describe("mcp__github__create_issue", &json!({})),
            "use create_issue from github"
        );
        assert_eq!(describe("Task", &json!({})), "use Task");
        assert!(is_ask(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"permission_prompt","arguments":{}}}"#
        ));
        assert!(!is_ask(
            r#"{"method":"tools/call","params":{"name":"read_memory"}}"#
        ));
    }
}
