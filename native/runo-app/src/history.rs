//! Past agent conversations on this machine, read from the CLIs' own transcripts:
//! Claude Code keeps `~/.claude/projects/<folder>/<session-id>.jsonl` and Codex keeps
//! `~/.codex/sessions/YYYY/MM/DD/rollout-…-<session-id>.jsonl`.

use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One past conversation that can be resumed.
#[derive(Clone, Debug, PartialEq)]
pub struct Past {
    pub provider: &'static str,
    pub id: String,
    /// Folder the conversation ran in.
    pub cwd: String,
    /// The first thing the user asked.
    pub title: String,
    pub modified: SystemTime,
}

/// Only the start of each transcript is read: the folder and first prompt are near the top.
const HEAD_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 400;

pub fn scan(home: &Path) -> Vec<Past> {
    let mut files: Vec<(&'static str, PathBuf, SystemTime)> = Vec::new();
    let mtime = |p: &Path| {
        p.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    };
    for dir in read_dirs(&home.join(".claude").join("projects")) {
        for file in jsonl_in(&dir) {
            let time = mtime(&file);
            files.push(("claude", file, time));
        }
    }
    // Codex nests by date: sessions/YYYY/MM/DD.
    let mut stack = vec![home.join(".codex").join("sessions")];
    while let Some(dir) = stack.pop() {
        stack.extend(read_dirs(&dir));
        for file in jsonl_in(&dir) {
            let time = mtime(&file);
            files.push(("codex", file, time));
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.2));
    // A resumed Codex conversation can be written to a second file under the same id;
    // list each conversation once, by its newest file.
    let mut seen = std::collections::HashSet::new();
    files
        .into_iter()
        .take(MAX_FILES)
        .filter_map(|(provider, path, modified)| {
            let text = read_head(&path)?;
            let mut past = match provider {
                "claude" => parse_claude(&text)?,
                _ => parse_codex(&text)?,
            };
            if past.id.is_empty() {
                past.id = path.file_stem()?.to_string_lossy().into_owned();
            }
            past.modified = modified;
            Some(past)
        })
        .filter(|past| seen.insert((past.provider, past.id.clone())))
        .collect()
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn jsonl_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect()
        })
        .unwrap_or_default()
}

fn read_head(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Text a person typed, skipping harness wrappers such as `<command-name>` or
/// `<environment_context>` that the CLIs record as user turns.
fn typed_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| matches!(b["type"].as_str(), Some("text" | "input_text")))
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() || text.starts_with('<') {
        return None;
    }
    let first = text.lines().next().unwrap_or(text);
    Some(first.chars().take(90).collect())
}

pub fn parse_claude(text: &str) -> Option<Past> {
    let (mut id, mut cwd, mut title) = (String::new(), String::new(), String::new());
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if id.is_empty() {
            if let Some(s) = event["sessionId"].as_str() {
                id = s.to_string();
            }
        }
        if cwd.is_empty() {
            if let Some(s) = event["cwd"].as_str() {
                cwd = s.to_string();
            }
        }
        let human = event["type"] == "user"
            && event["isMeta"] != true
            && event["isSidechain"] != true
            && event["message"]["role"] == "user";
        if title.is_empty() && human {
            if let Some(t) = typed_text(&event["message"]["content"]) {
                title = t;
            }
        }
        if !cwd.is_empty() && !title.is_empty() && !id.is_empty() {
            break;
        }
    }
    // A transcript without a prompt (for example only a summary) has nothing to resume.
    (!cwd.is_empty() && !title.is_empty()).then_some(Past {
        provider: "claude",
        id,
        cwd,
        title,
        modified: SystemTime::UNIX_EPOCH,
    })
}

pub fn parse_codex(text: &str) -> Option<Past> {
    let (mut id, mut cwd, mut title) = (String::new(), String::new(), String::new());
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = &event["payload"];
        match event["type"].as_str().unwrap_or("") {
            "session_meta" => {
                id = payload["id"].as_str().unwrap_or("").to_string();
                cwd = payload["cwd"].as_str().unwrap_or("").to_string();
            }
            "response_item" if title.is_empty() && payload["role"] == "user" => {
                if let Some(t) = typed_text(&payload["content"]) {
                    title = t;
                }
            }
            "event_msg" if title.is_empty() && payload["type"] == "user_message" => {
                if let Some(t) = typed_text(&payload["message"]) {
                    title = t;
                }
            }
            _ => {}
        }
        if !cwd.is_empty() && !title.is_empty() {
            break;
        }
    }
    (!cwd.is_empty() && !title.is_empty()).then_some(Past {
        provider: "codex",
        id,
        cwd,
        title,
        modified: SystemTime::UNIX_EPOCH,
    })
}

/// The transcript of one conversation, found by its id.
pub fn transcript(home: &Path, provider: &str, id: &str) -> Option<PathBuf> {
    if id.is_empty() {
        return None;
    }
    match provider {
        "claude" => read_dirs(&home.join(".claude").join("projects"))
            .into_iter()
            .map(|d| d.join(format!("{id}.jsonl")))
            .find(|p| p.is_file()),
        "codex" => {
            let mut stack = vec![home.join(".codex").join("sessions")];
            while let Some(dir) = stack.pop() {
                stack.extend(read_dirs(&dir));
                if let Some(file) = jsonl_in(&dir).into_iter().find(|p| {
                    p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().ends_with(id))
                }) {
                    return Some(file);
                }
            }
            None
        }
        _ => None,
    }
}

/// The newest Codex conversation that ran in `dir` and was written after `since`.
pub fn newest_codex(home: &Path, dir: &str, since: SystemTime) -> Option<(String, PathBuf)> {
    scan(home)
        .into_iter()
        .filter(|p| p.provider == "codex" && p.cwd == dir && p.modified >= since)
        .max_by_key(|p| p.modified)
        .and_then(|p| transcript(home, "codex", &p.id).map(|path| (p.id, path)))
}

/// Size of a file, or 0: where to start reading new turns later.
pub fn length(path: Option<&Path>) -> u64 {
    path.and_then(|p| p.metadata().ok()).map_or(0, |m| m.len())
}

/// The transcript from byte `from` on.
pub fn read_from(path: &Path, from: u64) -> String {
    use std::io::{Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(from)).is_ok() {
        let _ = file.read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Full text a person typed, or `None` for tool results and harness wrappers.
fn prompt_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| matches!(b["type"].as_str(), Some("text" | "input_text")))
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| text.to_string())
}

/// A short label for a tool call, such as "Ran cargo test" or "Edited src/main.rs".
fn step_label(name: &str, input: &Value) -> String {
    let field = |k: &str| input[k].as_str().unwrap_or("").to_string();
    let short = |t: String| {
        let line = t.lines().next().unwrap_or("").to_string();
        if line.chars().count() > 70 {
            format!("{}…", line.chars().take(70).collect::<String>())
        } else {
            line
        }
    };
    match name {
        "Bash" | "shell" | "exec_command" | "local_shell" => {
            let cmd = match &input["command"] {
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                v => v.as_str().unwrap_or("").to_string(),
            };
            format!("Ran {}", short(cmd))
        }
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            format!("Edited {}", short(field("file_path")))
        }
        "Read" => format!("Read {}", short(field("file_path"))),
        "apply_patch" => "Edited files".to_string(),
        other => format!("Used {other}"),
    }
}

/// Turns recorded by the agent's CLI, as chat messages: each prompt, then one reply
/// holding the agent's text, its tool steps and its token use.
pub fn turns(provider: &str, text: &str) -> Vec<crate::store::Message> {
    use crate::store::{Message, Role};
    let mut out: Vec<Message> = Vec::new();
    let reply = |out: &mut Vec<Message>| -> usize {
        if out.last().is_none_or(|m| m.role != Role::Agent) {
            out.push(Message::new(Role::Agent, ""));
        }
        out.len() - 1
    };
    let add_text = |m: &mut Message, t: &str| {
        let t = t.trim();
        if t.is_empty() {
            return;
        }
        if !m.text.is_empty() {
            m.text.push_str("\n\n");
        }
        m.text.push_str(t);
    };
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if provider == "claude" {
            if event["isSidechain"] == true || event["isMeta"] == true {
                continue;
            }
            let message = &event["message"];
            match event["type"].as_str().unwrap_or("") {
                "user" if message["role"] == "user" => {
                    if let Some(t) = prompt_text(&message["content"]) {
                        out.push(Message::new(Role::User, t));
                    }
                }
                "assistant" => {
                    let i = reply(&mut out);
                    for block in message["content"].as_array().into_iter().flatten() {
                        match block["type"].as_str().unwrap_or("") {
                            "text" => add_text(&mut out[i], block["text"].as_str().unwrap_or("")),
                            "tool_use" => {
                                let label = step_label(
                                    block["name"].as_str().unwrap_or("a tool"),
                                    &block["input"],
                                );
                                out[i].steps.push(label);
                            }
                            _ => {}
                        }
                    }
                    // Output adds up over the turn; the prompt side is the context size, so
                    // the latest call's figure is the one that counts.
                    let usage = &message["usage"];
                    if usage.is_object() {
                        let u = &mut out[i].usage;
                        u.input = usage["input_tokens"].as_u64().unwrap_or(0);
                        u.output += usage["output_tokens"].as_u64().unwrap_or(0);
                        u.cached = usage["cache_read_input_tokens"].as_u64().unwrap_or(0)
                            + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                    }
                }
                _ => {}
            }
        } else {
            let payload = &event["payload"];
            if event["type"] != "response_item" {
                continue;
            }
            match payload["type"].as_str().unwrap_or("") {
                "message" if payload["role"] == "user" => {
                    if let Some(t) = prompt_text(&payload["content"]) {
                        out.push(Message::new(Role::User, t));
                    }
                }
                "message" if payload["role"] == "assistant" => {
                    let i = reply(&mut out);
                    for block in payload["content"].as_array().into_iter().flatten() {
                        if block["type"] == "output_text" {
                            add_text(&mut out[i], block["text"].as_str().unwrap_or(""));
                        }
                    }
                }
                "function_call" | "local_shell_call" | "custom_tool_call" => {
                    let i = reply(&mut out);
                    let name = payload["name"].as_str().unwrap_or("shell");
                    let input = payload["arguments"]
                        .as_str()
                        .and_then(|a| serde_json::from_str::<Value>(a).ok())
                        .unwrap_or_else(|| payload["action"].clone());
                    out[i].steps.push(step_label(name, &input));
                }
                _ => {}
            }
        }
    }
    // A prompt the agent never answered (the CLI was closed first) still shows.
    out.retain(|m| m.role == Role::User || !m.text.is_empty() || !m.steps.is_empty());
    out
}

/// "3m ago", "2h ago", "5d ago" or the date, for list rows.
pub fn ago(time: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(time)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        86_400..=2_592_000 => format!("{}d ago", secs / 86_400),
        _ => {
            let date: chrono::DateTime<chrono::Local> = time.into();
            date.format("%b %-d, %Y").to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const CLAUDE: &str = r#"{"type":"queue-operation","operation":"enqueue","sessionId":"abc"}
{"type":"user","isMeta":true,"message":{"role":"user","content":"<command-name>/init</command-name>"},"cwd":"/work/app","sessionId":"abc"}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Fix the login bug\nIt fails on Safari"}]},"cwd":"/work/app","sessionId":"abc"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"On it"}]}}
"#;

    #[test]
    fn cli_turns_become_chat_messages() {
        use crate::store::Role;
        let claude = r#"{"type":"user","message":{"role":"user","content":"Add a test"},"sessionId":"abc"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Adding it."},{"type":"tool_use","name":"Edit","input":{"file_path":"src/lib.rs"}}],"usage":{"input_tokens":10,"output_tokens":5}}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}],"usage":{"input_tokens":3,"output_tokens":2}}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}
{"type":"user","isSidechain":true,"message":{"role":"user","content":"sub agent"}}
"#;
        let m = turns("claude", claude);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].role, m[0].text.as_str()), (Role::User, "Add a test"));
        assert_eq!(m[1].text, "Adding it.\n\nDone.");
        assert_eq!(m[1].steps, vec!["Edited src/lib.rs", "Ran cargo test"]);
        assert_eq!((m[1].usage.input, m[1].usage.output), (3, 7));
        let codex = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context></environment_context>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"List files"}]}}
{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"ls\",\"-a\"]}"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Two files."}]}}
"#;
        let m = turns("codex", codex);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].text, "List files");
        assert_eq!(m[1].steps, vec!["Ran ls -a"]);
        assert_eq!(m[1].text, "Two files.");
    }

    const CODEX: &str = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"0199-uuid","cwd":"/work/api"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n</environment_context>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add rate limiting"}]}}
"#;

    #[test]
    fn reads_claude_transcripts() {
        let past = parse_claude(CLAUDE).unwrap();
        assert_eq!(
            (
                past.provider,
                past.id.as_str(),
                past.cwd.as_str(),
                past.title.as_str()
            ),
            ("claude", "abc", "/work/app", "Fix the login bug")
        );
        // Only harness turns: nothing worth listing.
        assert!(parse_claude(
            CLAUDE
                .lines()
                .take(2)
                .collect::<Vec<_>>()
                .join("\n")
                .as_str()
        )
        .is_none());
    }

    #[test]
    fn reads_codex_rollouts() {
        let past = parse_codex(CODEX).unwrap();
        assert_eq!(
            (
                past.provider,
                past.id.as_str(),
                past.cwd.as_str(),
                past.title.as_str()
            ),
            ("codex", "0199-uuid", "/work/api", "Add rate limiting")
        );
        let from_event = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"x\",\"cwd\":\"/a\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"Ship it\"}}";
        assert_eq!(parse_codex(from_event).unwrap().title, "Ship it");
    }

    #[test]
    fn scans_both_folders_newest_first() {
        let home = std::env::temp_dir().join(format!("runo-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let claude = home.join(".claude/projects/-work-app");
        let codex = home.join(".codex/sessions/2026/01/01");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        // Without a sessionId field the file name is the id.
        std::fs::write(
            claude.join("abc.jsonl"),
            CLAUDE.replace("\"sessionId\":\"abc\"", "\"x\":1"),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(codex.join("rollout-1.jsonl"), CODEX).unwrap();
        // The same conversation resumed into a second file is listed once.
        std::fs::write(codex.join("rollout-2.jsonl"), CODEX).unwrap();
        std::fs::write(codex.join("notes.txt"), "ignored").unwrap();
        let found = scan(&home);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].provider, "codex");
        assert_eq!(found[1].id, "abc");
        assert_eq!(ago(SystemTime::now()), "just now");
        assert_eq!(ago(SystemTime::now() - Duration::from_secs(7200)), "2h ago");
        let _ = std::fs::remove_dir_all(&home);
    }
}
