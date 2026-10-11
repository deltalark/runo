//! `runo --mcp-memory <folder>`: a small MCP server (JSON-RPC over stdio) that lets any
//! agent read the session memory, search earlier sessions of the project and save notes,
//! instead of having the whole history pasted into every prompt.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::handoff::{MEMORY_FILE, NOTES_FILE};
use crate::store::{Role, State};

const MAX_RESULTS: usize = 20;

pub fn serve(dir: &str) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(response) = handle(dir, &request) {
            let _ = writeln!(out, "{response}");
            let _ = out.flush();
        }
    }
}

/// The reply to one request, or `None` for notifications.
pub fn handle(dir: &str, request: &Value) -> Option<Value> {
    let id = request.get("id")?.clone();
    let params = &request["params"];
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "runo-memory", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Runo's session memory. Call read_memory to catch up on this task, search_history to find what earlier sessions did, and save_note to keep decisions for whoever works on this next."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call(
            dir,
            params["name"].as_str().unwrap_or(""),
            &params["arguments"],
        )),
        method => Err(json!({ "code": -32601, "message": format!("Unknown method {method}") })),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "read_memory",
            "description": "Read this session's memory: the task, earlier requests, the recent conversation, the latest plan, open items, notes and the state of the code. Pass a section (for example \"Latest plan\") to read only that part.",
            "inputSchema": {
                "type": "object",
                "properties": { "section": { "type": "string", "description": "Heading to read, such as \"The task\" or \"Code state\"." } }
            }
        },
        {
            "name": "search_history",
            "description": "Search every earlier session of this project (requests and replies, all agents) for words, and get matching excerpts with the session they came from.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }
        },
        {
            "name": "save_note",
            "description": "Save a short, durable note (a decision, a gotcha, what is left to do) to this session's memory, so the next agent or session sees it.",
            "inputSchema": {
                "type": "object",
                "properties": { "note": { "type": "string" } },
                "required": ["note"]
            }
        }
    ])
}

fn text(t: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }] })
}

fn failure(t: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": true })
}

fn call(dir: &str, name: &str, args: &Value) -> Value {
    match name {
        "read_memory" => {
            let memory =
                std::fs::read_to_string(Path::new(dir).join(MEMORY_FILE)).unwrap_or_default();
            if memory.trim().is_empty() {
                return text("There is no memory yet for this session.");
            }
            match args["section"].as_str().filter(|s| !s.trim().is_empty()) {
                Some(section) => match find_section(&memory, section) {
                    Some(body) => text(body),
                    None => text(format!(
                        "No section called \"{section}\". Sections: {}",
                        headings(&memory).join(", ")
                    )),
                },
                None => text(memory),
            }
        }
        "search_history" => {
            let query = args["query"].as_str().unwrap_or("").trim();
            if query.is_empty() {
                return failure("Give a query to search for.");
            }
            text(search(dir, query))
        }
        "save_note" => {
            let note = args["note"].as_str().unwrap_or("").trim();
            if note.is_empty() {
                return failure("The note is empty.");
            }
            let path = Path::new(dir).join(NOTES_FILE);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut current = std::fs::read_to_string(&path).unwrap_or_default();
            let date = chrono::Local::now().format("%Y-%m-%d %H:%M");
            current.push_str(&format!("- {date}: {}\n", note.replace('\n', " ")));
            match std::fs::write(&path, current) {
                Ok(()) => text("Saved."),
                Err(e) => failure(format!("Could not save the note: {e}")),
            }
        }
        other => failure(format!("Unknown tool {other}")),
    }
}

fn headings(memory: &str) -> Vec<String> {
    memory
        .lines()
        .filter_map(|l| l.strip_prefix("## "))
        .map(str::to_string)
        .collect()
}

fn find_section(memory: &str, name: &str) -> Option<String> {
    let want = name.trim().trim_start_matches('#').trim().to_lowercase();
    let mut out: Option<String> = None;
    for line in memory.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if out.is_some() {
                break;
            }
            if h.trim().to_lowercase().contains(&want) {
                out = Some(format!("{line}\n"));
            }
            continue;
        }
        if let Some(body) = out.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out.map(|s| s.trim().to_string())
}

/// Runo's saved sessions, read without changing anything.
fn load_state() -> State {
    let path: PathBuf = crate::store::data_dir().join("state.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn search(dir: &str, query: &str) -> String {
    let state = load_state();
    let project = state
        .sessions
        .iter()
        .find(|s| s.dir == dir)
        .map(|s| s.project_id)
        .or_else(|| {
            state
                .projects
                .iter()
                .find(|p| Path::new(dir).starts_with(&p.path))
                .map(|p| p.id)
        });
    let words: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let mut hits = Vec::new();
    for s in state
        .sessions
        .iter()
        .filter(|s| project.is_none_or(|p| s.project_id == p))
    {
        for m in &s.messages {
            if m.role == Role::System || m.text.trim().is_empty() {
                continue;
            }
            let lower = m.text.to_lowercase();
            if !words.iter().all(|w| lower.contains(w.as_str())) {
                continue;
            }
            let who = if m.role == Role::User {
                "request".to_string()
            } else {
                crate::agent::provider(&s.provider).name.to_string()
            };
            hits.push(format!(
                "- \"{}\" ({}, {who}): {}",
                s.title,
                if m.time.is_empty() {
                    "earlier"
                } else {
                    &m.time
                },
                excerpt(&m.text, &words[0])
            ));
            if hits.len() >= MAX_RESULTS {
                break;
            }
        }
    }
    if hits.is_empty() {
        format!("Nothing in this project's sessions mentions \"{query}\".")
    } else {
        format!("{} match(es):\n{}", hits.len(), hits.join("\n"))
    }
}

/// About 240 characters around the first match.
fn excerpt(text: &str, word: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let w: Vec<char> = word.chars().collect();
    let at = (0..lower.len().saturating_sub(w.len()) + 1)
        .find(|&i| lower[i..].starts_with(&w))
        .unwrap_or(0);
    let start = at.saturating_sub(100);
    let end = (at + 140).min(chars.len());
    let mut out: String = chars[start..end].iter().collect();
    out = out.replace('\n', " ");
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_server_speaks_mcp() {
        let dir = std::env::temp_dir().join(format!("runo-mcp-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".runo")).unwrap();
        std::fs::write(
            dir.join(MEMORY_FILE),
            "# Session memory\n\n## The task\nAdd login\n\n## Latest plan\n1. Form\n2. Tests\n",
        )
        .unwrap();
        let d = dir.to_str().unwrap();
        let init = handle(d, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(
            d,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());
        let list = handle(d, &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 3);
        let call = |name: &str, args: Value| {
            handle(d, &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap()["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(
            call("read_memory", json!({"section": "plan"})),
            "## Latest plan\n1. Form\n2. Tests"
        );
        assert!(call("read_memory", json!({})).contains("Add login"));
        assert_eq!(call("save_note", json!({"note": "Use bcrypt"})), "Saved.");
        assert!(std::fs::read_to_string(dir.join(NOTES_FILE))
            .unwrap()
            .contains("Use bcrypt"));
        let unknown = handle(d, &json!({"jsonrpc":"2.0","id":4,"method":"nope"})).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(
            excerpt("hello brave new world", "new"),
            "hello brave new world"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
