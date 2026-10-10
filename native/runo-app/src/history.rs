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
