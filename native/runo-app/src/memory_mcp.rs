//! `runo --mcp-memory <folder>`: a small MCP server (JSON-RPC over stdio) that lets any
//! agent read the session memory, search earlier sessions of the project and save notes,
//! instead of having the whole history pasted into every prompt. It also offers the
//! playbooks: as tools, and as MCP prompts that CLIs like Claude Code show as slash commands.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;

use crate::handoff::{MEMORY_FILE, NOTES_FILE};
use crate::store::{Role, State};

const MAX_RESULTS: usize = 20;

/// Serve the tools for the session in `dir`. `thread` is the session's id when it is one
/// of Crew's threads, which adds the tools threads report through.
pub fn serve(dir: &str, thread: Option<u64>) {
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
        if let Some(response) = handle_thread(dir, thread, &request) {
            let _ = writeln!(out, "{response}");
            let _ = out.flush();
        }
    }
}

/// The reply to one request, or `None` for notifications. Requests relayed from a
/// connection come through here, so threads there don't get the thread tools yet.
pub fn handle(dir: &str, request: &Value) -> Option<Value> {
    handle_thread(dir, None, request)
}

/// The reply to one request from the session in `dir`, a Crew thread when `thread` is set.
pub fn handle_thread(dir: &str, thread: Option<u64>, request: &Value) -> Option<Value> {
    let id = request.get("id")?.clone();
    let params = &request["params"];
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => {
            if let Some(name) = params["clientInfo"]["name"].as_str() {
                let _ = CLIENT.set(name.to_string());
            }
            Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {}, "prompts": {} },
                "serverInfo": { "name": "runo", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Runo's session memory and playbooks. Call read_memory to catch up on this task, search_history to find what earlier sessions did, and save_note to keep decisions for whoever works on this next. Playbooks are saved tasks: list_playbooks shows them, get_playbook gives one's instructions to carry out now, and run_playbook starts one as its own session in Runo."
            }))
        }
        "prompts/list" => Ok(json!({ "prompts": prompt_list(&load_state(), dir) })),
        "prompts/get" => prompt_get(&load_state(), dir, params["name"].as_str().unwrap_or("")),
        "ping" => Ok(json!({})),
        "tools/list" => {
            let mut all = tools();
            if crate::designdraw::agentops::offered(dir) {
                if let Some(list) = all.as_array_mut() {
                    list.extend(crate::designdraw::agentops::tools());
                }
            }
            if thread.is_some() {
                if let Some(list) = all.as_array_mut() {
                    list.extend(thread_tools());
                }
            }
            Ok(json!({ "tools": all }))
        }
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            match thread.filter(|_| THREAD_TOOLS.contains(&name)) {
                Some(thread) => Ok(thread_call(
                    &crate::crew::events_dir(),
                    thread,
                    name,
                    &params["arguments"],
                    &MESSAGES_SENT,
                )),
                None => Ok(call(dir, name, &params["arguments"])),
            }
        }
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
            "name": "list_playbooks",
            "description": "List the playbooks (saved tasks) that can run in this project, with what each does.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_playbook",
            "description": "Get a playbook's instructions, to carry them out yourself now in this conversation.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "The playbook's name, as list_playbooks shows it." } },
                "required": ["name"]
            }
        },
        {
            "name": "run_playbook",
            "description": "Start a playbook as its own new session in Runo, in this project. It runs alongside this one; use get_playbook instead to do it here.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }
        },
        {
            "name": "list_connections",
            "description": "List the places besides this machine where commands can run: Cove VMs and the person's servers, with their size, whether they have network access, and whether agents may use them.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "run_on",
            "description": "Run a shell command on one of the connections from list_connections and get its output and exit code. It runs in a scratch folder there (a dedicated Cove VM, or a scratch folder on a server), not in this session's files. Use it for heavy or long jobs, research that needs network access, trying untrusted code safely, or anything that needs that machine's hardware.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": { "type": "string", "description": "The connection's name." },
                    "command": { "type": "string", "description": "A bash command line." }
                },
                "required": ["connection", "command"]
            }
        },
        {
            "name": crate::approvals::TOOL,
            "description": "Used by Claude Code to ask the person, in Runo, for permission to use a tool. Don't call it yourself.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool_name": { "type": "string" },
                    "input": { "type": "object" },
                    "tool_use_id": { "type": "string" }
                },
                "required": ["tool_name", "input"]
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

/// Tools only Crew's threads get; every other session's list is unchanged.
pub(crate) const THREAD_TOOLS: [&str; 4] =
    ["set_checklist", "report", "ask_decision", "message_crew"];

/// Messages a thread may send Crew from one run of this server, which lasts one turn in
/// Chat. More would let two agents keep each other busy.
const MESSAGES_PER_RUN: u32 = 1;
static MESSAGES_SENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn thread_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "set_checklist",
            "description": "Show the person your plan for this task as a checklist, and keep it current: call it again with the whole list whenever a step starts or finishes. At most 20 short steps.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string" },
                                "state": { "type": "string", "enum": ["todo", "doing", "done"] }
                            },
                            "required": ["text", "state"]
                        }
                    }
                },
                "required": ["steps"]
            }
        }),
        json!({
            "name": "report",
            "description": "Report a result to the person: what you did or found, in a few plain sentences, with links to anything they should open (a pull request, a file, a document). Call it when the task is done or you reach something they should know.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "summary": { "type": "string" },
                    "links": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["summary"]
            }
        }),
        json!({
            "name": "ask_decision",
            "description": "Ask the person to choose, when the choice changes what they get or can't be undone. Give one question, two to four options with what happens if each is picked, and the index of the one you recommend. Keep working on the recommended option unless it can't be undone; their answer comes back as your next message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "options": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string" },
                                "consequence": { "type": "string" }
                            },
                            "required": ["label"]
                        }
                    },
                    "recommended": { "type": "integer" }
                },
                "required": ["question", "options", "recommended"]
            }
        }),
        json!({
            "name": "message_crew",
            "description": "Send a short message to Crew, which runs this project's threads: for example to ask another thread for something your work depends on. One message per turn.",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }
        }),
    ]
}

/// A thread tool's call: checked, then left for the app as an event in `events`.
fn thread_call(
    events: &Path,
    thread: u64,
    name: &str,
    args: &Value,
    sent: &std::sync::atomic::AtomicU32,
) -> Value {
    use crate::crew::{check_decision, clean_steps, Decision, EventKind, Mark, Step};
    let kind = match name {
        "set_checklist" => {
            let Some(items) = args["steps"].as_array() else {
                return failure("Give steps: a list of {text, state}.");
            };
            let steps = items
                .iter()
                .map(|item| Step {
                    text: item["text"].as_str().unwrap_or("").to_string(),
                    mark: match item["state"].as_str() {
                        Some("done") => Mark::Done,
                        Some("doing") => Mark::Doing,
                        _ => Mark::Todo,
                    },
                })
                .collect();
            let steps = clean_steps(steps);
            if steps.is_empty() {
                return failure("The checklist has no steps.");
            }
            EventKind::Checklist { steps }
        }
        "report" => {
            let summary = args["summary"].as_str().unwrap_or("").trim();
            if summary.is_empty() {
                return failure("The report is empty.");
            }
            let links = args["links"]
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            EventKind::Report {
                summary: summary.to_string(),
                links,
            }
        }
        "ask_decision" => {
            let decision = match serde_json::from_value::<Decision>(args.clone()) {
                Ok(decision) => decision,
                Err(_) => {
                    return failure(
                        "Give a question, options as a list of {label, consequence}, and recommended as an index.",
                    )
                }
            };
            match check_decision(decision) {
                Ok(decision) => EventKind::Decision(decision),
                Err(why) => return failure(why),
            }
        }
        "message_crew" => {
            let text = args["text"].as_str().unwrap_or("").trim();
            if text.is_empty() {
                return failure("The message is empty.");
            }
            use std::sync::atomic::Ordering;
            if sent.fetch_add(1, Ordering::SeqCst) >= MESSAGES_PER_RUN {
                return failure(
                    "You already sent Crew a message this turn. Put anything else in your report.",
                );
            }
            EventKind::Message {
                text: text.to_string(),
            }
        }
        other => return failure(format!("Unknown tool {other}")),
    };
    match crate::crew::send_event(events, &crate::crew::event(thread, kind)) {
        Ok(()) => text(match name {
            "ask_decision" => "Asked. Keep working on the recommended option unless it can't be undone; the answer comes as your next message.",
            "message_crew" => "Sent to Crew.",
            _ => "Shown to the person.",
        }),
        Err(e) => failure(format!("Runo could not take this: {e}")),
    }
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
        "list_connections" | "run_on" => connection_call(
            &crate::settings::AppSettings::load().connections,
            name,
            args,
        ),
        "list_playbooks" | "get_playbook" | "run_playbook" => {
            playbook_call(&load_state(), dir, name, args, &crate::store::inbox_dir())
        }
        crate::approvals::TOOL => crate::approvals::call(
            &crate::approvals::folder(),
            dir,
            args,
            crate::approvals::wait(),
        ),
        design if design.starts_with("design_") => crate::designdraw::agentops::call(
            &crate::designdraw::agentops::folder(),
            dir,
            design,
            args,
        ),
        other => failure(format!("Unknown tool {other}")),
    }
}

fn connection_call(all: &[crate::connections::Connection], name: &str, args: &Value) -> Value {
    if name == "list_connections" {
        if all.is_empty() {
            return text("No connections are set up. The person can add Cove VMs and servers in Runo's Settings > Connections.");
        }
        let lines: Vec<String> = all
            .iter()
            .map(|c| {
                format!(
                    "- {}: {}{}",
                    c.name,
                    c.summary(),
                    if c.agents {
                        ""
                    } else {
                        " (agents may not run commands here)"
                    }
                )
            })
            .collect();
        return text(lines.join("\n"));
    }
    let wanted = args["connection"].as_str().unwrap_or("").trim();
    let command = args["command"].as_str().unwrap_or("").trim();
    if command.is_empty() {
        return failure("Give a command to run.");
    }
    let Some(conn) = all.iter().find(|c| c.name.eq_ignore_ascii_case(wanted)) else {
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        return failure(format!(
            "No connection called \"{wanted}\". Connections: {}",
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        ));
    };
    match crate::connections::run_for_agent(conn, command) {
        Ok(out) => text(out),
        Err(e) => failure(e),
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
/// The name of the CLI this server talks to, from its `initialize` request.
static CLIENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The project a session folder belongs to.
fn project_of(state: &State, dir: &str) -> Option<u64> {
    state
        .sessions
        .iter()
        .find(|s| s.dir == dir)
        .map(|s| s.project_id)
        .or_else(|| {
            state
                .projects
                .iter()
                .filter(|p| Path::new(dir).starts_with(&p.path))
                .max_by_key(|p| p.path.len())
                .map(|p| p.id)
        })
}

/// Playbooks that can run in the project of `dir`: its own and the any-project ones.
fn playbooks_here<'a>(state: &'a State, dir: &str) -> Vec<&'a crate::store::Automation> {
    let project = project_of(state, dir);
    state
        .automations
        .iter()
        .filter(|a| a.project_id == 0 || Some(a.project_id) == project)
        .collect()
}

fn find_playbook<'a>(
    state: &'a State,
    dir: &str,
    name: &str,
) -> Option<&'a crate::store::Automation> {
    let want = name.trim().trim_start_matches('/').to_lowercase();
    let here = playbooks_here(state, dir);
    here.iter()
        .find(|a| a.slug() == want || a.name.to_lowercase() == want)
        .or_else(|| here.iter().find(|a| a.slug().contains(&want)))
        .copied()
}

fn prompt_list(state: &State, dir: &str) -> Value {
    Value::Array(
        playbooks_here(state, dir)
            .into_iter()
            .map(|a| json!({ "name": a.slug(), "title": a.name, "description": a.summary() }))
            .collect(),
    )
}

fn prompt_get(state: &State, dir: &str, name: &str) -> Result<Value, Value> {
    match find_playbook(state, dir, name) {
        Some(a) => Ok(json!({
            "description": a.summary(),
            "messages": [{ "role": "user", "content": { "type": "text", "text": a.prompt } }]
        })),
        None => Err(json!({ "code": -32602, "message": format!("No playbook called {name}") })),
    }
}

fn playbook_call(state: &State, dir: &str, tool: &str, args: &Value, inbox: &Path) -> Value {
    if tool == "list_playbooks" {
        let here = playbooks_here(state, dir);
        if here.is_empty() {
            return text("There are no playbooks yet. People add them on Runo's Playbooks page.");
        }
        return text(
            here.iter()
                .map(|a| format!("- {} ({}): {}", a.slug(), a.provider, a.summary()))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let name = args["name"].as_str().unwrap_or("");
    let Some(playbook) = find_playbook(state, dir, name) else {
        return failure(format!(
            "No playbook called \"{name}\" here. Call list_playbooks to see the names."
        ));
    };
    if tool == "get_playbook" {
        return text(format!(
            "Playbook \"{}\". Carry out these instructions now:\n\n{}",
            playbook.name, playbook.prompt
        ));
    }
    let project = match playbook.project_id {
        0 => project_of(state, dir),
        id => Some(id),
    };
    let Some(project) = project else {
        return failure(
            "This folder is not one of Runo's projects, so there is nowhere to run it.",
        );
    };
    let request = crate::store::PlaybookRequest {
        playbook: playbook.id,
        project,
        from: CLIENT.get().cloned().unwrap_or_else(|| "an agent".into()),
        dir: dir.to_string(),
    };
    match crate::store::send_request(inbox, &request) {
        Ok(()) => text(format!(
            "Asked Runo to start \"{}\" as a new session. Runo may ask the person to confirm first. It shows up in Runo's sidebar; if Runo is closed it starts when Runo opens.",
            playbook.name
        )),
        Err(e) => failure(format!("Could not reach Runo: {e}")),
    }
}

fn load_state() -> State {
    State::peek()
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
        let dir = std::env::temp_dir().join(format!("runo-memory-mcp-{}", std::process::id()));
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
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 9);
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

    #[test]
    fn only_crew_threads_get_thread_tools() {
        let names = |thread: Option<u64>| -> Vec<String> {
            let list = handle_thread(
                "/nowhere",
                thread,
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            )
            .unwrap();
            list["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        // An ordinary session's tools are exactly what they were before Crew.
        let plain = names(None);
        assert_eq!(
            plain,
            [
                "read_memory",
                "search_history",
                "list_playbooks",
                "get_playbook",
                "run_playbook",
                "list_connections",
                "run_on",
                crate::approvals::TOOL,
                "save_note"
            ]
        );
        let thread = names(Some(7));
        assert_eq!(thread[..plain.len()], plain[..]);
        assert_eq!(thread[plain.len()..], THREAD_TOOLS);
        // Calling a thread tool from an ordinary session finds no such tool.
        let call = handle(
            "/nowhere",
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"report","arguments":{"summary":"x"}}}),
        )
        .unwrap();
        assert_eq!(call["result"]["isError"], true);
    }

    #[test]
    fn thread_tools_leave_events() {
        use crate::crew::{take_events, EventKind, Mark, Seen};
        use std::sync::atomic::AtomicU32;
        let events = std::env::temp_dir().join(format!("runo-thread-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&events);
        let sent = AtomicU32::new(0);
        let call = |name: &str, args: Value| thread_call(&events, 7, name, &args, &sent);
        let ok = |v: Value| v.get("isError").is_none();

        assert!(ok(call(
            "set_checklist",
            json!({"steps":[{"text":"Measure","state":"done"},{"text":"Fix","state":"doing"}]})
        )));
        assert!(!ok(call("set_checklist", json!({"steps":[]}))));
        assert!(!ok(call("set_checklist", json!({}))));
        assert!(ok(call(
            "report",
            json!({"summary":"Opened the PR","links":["https://github.com/a/b/pull/1"]})
        )));
        assert!(!ok(call("report", json!({"summary":" "}))));
        let options =
            json!([{"label":"Second copy","consequence":"Agents keep running"},{"label":"Quit"}]);
        assert!(ok(call(
            "ask_decision",
            json!({"question":"Quit Runo?","options":options,"recommended":0})
        )));
        assert!(!ok(call(
            "ask_decision",
            json!({"question":"Quit Runo?","options":options,"recommended":5})
        )));
        assert!(!ok(call("ask_decision", json!({"question":"Quit Runo?"}))));
        assert!(ok(call(
            "message_crew",
            json!({"text":"Ask Cove for its API"})
        )));
        assert!(!ok(call("message_crew", json!({"text":"Again"}))));

        let taken = take_events(&events, &mut Seen::default());
        assert_eq!(taken.len(), 4);
        assert!(taken.iter().all(|e| e.thread == 7));
        let EventKind::Checklist { steps } = &taken[0].kind else {
            panic!("first event is not the checklist");
        };
        assert_eq!(steps[1].mark, Mark::Doing);
        assert!(matches!(taken[1].kind, EventKind::Report { .. }));
        assert!(matches!(taken[2].kind, EventKind::Decision(_)));
        assert!(matches!(taken[3].kind, EventKind::Message { .. }));
        let _ = std::fs::remove_dir_all(events);
    }

    #[test]
    fn playbooks_list_get_and_run() {
        use crate::store::{Automation, Project};
        let mut state = State::default();
        for (id, path) in [(1, "/w/app"), (2, "/w/site")] {
            state.projects.push(Project {
                id,
                path: path.into(),
                name: format!("p{id}"),
                ..Default::default()
            });
        }
        let book = |id, name: &str, project_id, prompt: &str| Automation {
            id,
            name: name.into(),
            project_id,
            provider: "claude".into(),
            prompt: prompt.into(),
            description: String::new(),
            every_minutes: 0,
            last_run: 0,
            enabled: true,
            ran: 0,
            ..Default::default()
        };
        state.automations = vec![
            book(10, "Find bugs", 0, "Review the code for bugs. List them."),
            book(11, "Ship site", 2, "Deploy the site."),
        ];
        // Only playbooks for this project (and any-project ones) show up.
        let prompts = prompt_list(&state, "/w/app/src");
        assert_eq!(prompts.as_array().unwrap().len(), 1);
        assert_eq!(prompts[0]["name"], "find-bugs");
        assert_eq!(prompts[0]["description"], "Review the code for bugs.");
        let got = prompt_get(&state, "/w/app", "find-bugs").unwrap();
        assert_eq!(
            got["messages"][0]["content"]["text"],
            "Review the code for bugs. List them."
        );
        assert!(prompt_get(&state, "/w/app", "ship-site").is_err());

        let inbox = std::env::temp_dir().join(format!("runo-inbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&inbox);
        let say = |tool: &str, args: Value| {
            playbook_call(&state, "/w/site", tool, &args, &inbox)["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(say("list_playbooks", json!({})).contains("ship-site (claude)"));
        assert!(say("get_playbook", json!({"name": "/Find bugs"})).contains("List them."));
        assert!(say("run_playbook", json!({"name": "ship"})).contains("Asked Runo"));
        assert!(say("run_playbook", json!({"name": "nope"})).contains("No playbook"));
        let requests = crate::store::take_requests(&inbox);
        assert_eq!(requests.len(), 1);
        assert_eq!((requests[0].playbook, requests[0].project), (11, 2));
        assert!(crate::store::take_requests(&inbox).is_empty());
        let _ = std::fs::remove_dir_all(&inbox);
    }
}
