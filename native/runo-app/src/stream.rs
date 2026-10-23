//! Turns agents' structured output into conversation updates: reply text as it streams,
//! a short label for each tool step, token use and a run summary. Claude Code's
//! `--output-format stream-json` (also spoken by Amp and Qwen Code) is read by
//! `ClaudeStream`; the JSON lines of Codex, Cursor, Droid, Gemini, OpenCode and Copilot
//! by `JsonStream`.

use serde_json::Value;

use crate::crew::{Mark, Step};
use crate::harness::{AgentEvent, ToolKind, ToolStep};
use crate::store::{Question, Usage};

/// The parser for one run, by the format the agent prints.
pub enum Parser {
    Claude(ClaudeStream),
    Json(Box<JsonStream>),
}

impl Parser {
    /// The parser for a run of `provider` in `cwd`, or `None` for plain text output.
    pub fn for_provider(provider: &str, cwd: &str) -> Option<Parser> {
        Parser::for_format(crate::agent::output(provider), cwd)
    }

    /// The parser for output in `format`, or `None` for plain text.
    pub fn for_format(format: crate::agent::Output, cwd: &str) -> Option<Parser> {
        match format {
            crate::agent::Output::Claude => Some(Parser::Claude(ClaudeStream::new(cwd))),
            crate::agent::Output::Json => Some(Parser::Json(Box::new(JsonStream::new(cwd)))),
            crate::agent::Output::Text => None,
        }
    }

    pub fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        match self {
            Parser::Claude(p) => p.feed(line),
            Parser::Json(p) => p.feed(line),
        }
    }
}

/// The conversation id a JSON event names, under any of the keys the CLIs use.
fn conversation_of(event: &Value) -> Option<String> {
    ["session_id", "sessionID", "sessionId", "thread_id"]
        .iter()
        .find_map(|k| event[*k].as_str())
        .or_else(|| event["data"]["sessionId"].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Per-run parser state.
pub struct ClaudeStream {
    /// Working directory, so file paths in steps can be shown relative to it.
    cwd: String,
    /// Whether text for the current API message already arrived as deltas, so the
    /// complete `assistant` message does not add it a second time.
    streamed_text: bool,
    /// Whether any reply text has been emitted, to separate later text blocks.
    any_text: bool,
    usage: Usage,
    /// Whether the conversation id was already reported.
    named: bool,
}

impl ClaudeStream {
    pub fn new(cwd: &str) -> Self {
        Self {
            cwd: cwd.trim_end_matches('/').to_string(),
            streamed_text: false,
            any_text: false,
            usage: Usage::default(),
            named: false,
        }
    }

    /// Feed one stdout line. Lines that are not JSON pass through as text.
    pub fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return if line.trim().is_empty() {
                vec![]
            } else {
                vec![self.text(line.to_string())]
            };
        };
        let mut out = Vec::new();
        let kind = event["type"].as_str().unwrap_or("");
        if !self.named && matches!(kind, "system" | "result") {
            if let Some(id) = conversation_of(&event) {
                self.named = true;
                out.push(AgentEvent::Conversation(id));
            }
        }
        match kind {
            "stream_event" => {
                if !event["parent_tool_use_id"].is_null() {
                    return out;
                }
                let inner = &event["event"];
                match inner["type"].as_str().unwrap_or("") {
                    "message_start" => self.streamed_text = false,
                    "content_block_start" if inner["content_block"]["type"] == "text" => {
                        if self.any_text {
                            out.push(AgentEvent::Text("\n\n".into()));
                        }
                    }
                    "content_block_delta" if inner["delta"]["type"] == "text_delta" => {
                        if let Some(text) = inner["delta"]["text"].as_str() {
                            self.streamed_text = true;
                            self.any_text = true;
                            out.push(AgentEvent::Text(text.to_string()));
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                // Subagent traffic is summarised by its parent's Task step.
                if !event["parent_tool_use_id"].is_null() {
                    return out;
                }
                let u = &event["message"]["usage"];
                if u.is_object() {
                    let n = |k: &str| u[k].as_u64().unwrap_or(0);
                    self.usage.input = n("input_tokens");
                    self.usage.cached =
                        n("cache_read_input_tokens") + n("cache_creation_input_tokens");
                    out.push(AgentEvent::Usage(self.usage));
                }
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        match block["name"].as_str().unwrap_or("") {
                            "ExitPlanMode" => {
                                if let Some(plan) = block["input"]["plan"].as_str() {
                                    out.push(AgentEvent::Plan(plan.to_string()));
                                }
                            }
                            name if is_todo_tool(name) => {
                                out.extend(todos(&block["input"]).map(AgentEvent::Todos));
                            }
                            "AskUserQuestion" => {
                                let questions: Vec<Question> = block["input"]["questions"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .map(|q| Question {
                                        question: q["question"].as_str().unwrap_or("").to_string(),
                                        options: q["options"]
                                            .as_array()
                                            .into_iter()
                                            .flatten()
                                            .filter_map(|o| {
                                                o["label"].as_str().or(o.as_str()).map(String::from)
                                            })
                                            .collect(),
                                    })
                                    .filter(|q| !q.question.is_empty())
                                    .collect();
                                if !questions.is_empty() {
                                    out.push(AgentEvent::Questions(questions));
                                }
                            }
                            _ => {}
                        }
                    }
                    match block["type"].as_str().unwrap_or("") {
                        "text" if !self.streamed_text => {
                            if let Some(text) = block["text"].as_str().filter(|t| !t.is_empty()) {
                                let sep = if self.any_text { "\n\n" } else { "" };
                                out.push(self.text(format!("{sep}{text}")));
                            }
                        }
                        "tool_use" => {
                            let tool = block["name"].as_str().unwrap_or("tool");
                            let label = self.step_label(tool, &block["input"]);
                            out.push(AgentEvent::Tool(
                                match block["id"].as_str().filter(|id| !id.is_empty()) {
                                    Some(id) => ToolStep::running(id, tool, label),
                                    None => ToolStep::of(tool, label),
                                },
                            ));
                        }
                        _ => {}
                    }
                }
            }
            // Tool results come back as a user message, one block per finished call.
            "user" => {
                if !event["parent_tool_use_id"].is_null() {
                    return out;
                }
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    if let Some(id) = block["tool_use_id"].as_str().filter(|id| !id.is_empty()) {
                        out.push(AgentEvent::ToolDone {
                            id: id.to_string(),
                            ok: block["is_error"].as_bool() != Some(true),
                            output: result_text(&block["content"]),
                        });
                    }
                }
            }
            "result" => {
                let n = |k: &str| event["usage"][k].as_u64().unwrap_or(0);
                if event["usage"].is_object() {
                    self.usage.output = n("output_tokens");
                    if self.usage.input == 0 && self.usage.cached == 0 {
                        self.usage.input = n("input_tokens");
                        self.usage.cached =
                            n("cache_read_input_tokens") + n("cache_creation_input_tokens");
                    }
                }
                self.usage.cost_cents = event["total_cost_usd"].as_f64().unwrap_or(0.0) * 100.0;
                if let Some(models) = event["modelUsage"].as_object() {
                    self.usage.window = models
                        .values()
                        .filter_map(|m| m["contextWindow"].as_u64())
                        .max()
                        .unwrap_or(0);
                }
                if !self.usage.is_empty() {
                    out.push(AgentEvent::Usage(self.usage));
                }
                let mut denied: Vec<String> = Vec::new();
                for d in event["permission_denials"].as_array().into_iter().flatten() {
                    if let Some(tool) = d["tool_name"].as_str() {
                        // Plan and question tools are answered in the UI, not by allowing them.
                        if !matches!(tool, "ExitPlanMode" | "AskUserQuestion")
                            && !denied.iter().any(|t| t == tool)
                        {
                            denied.push(tool.to_string());
                        }
                    }
                }
                if !denied.is_empty() {
                    out.push(AgentEvent::Denied(denied));
                }
                let mut parts = Vec::new();
                if let Some(ms) = event["duration_ms"].as_u64() {
                    parts.push(duration(ms));
                }
                if let Some(turns) = event["num_turns"].as_u64() {
                    parts.push(format!("{turns} turn{}", if turns == 1 { "" } else { "s" }));
                }
                if let Some(cost) = event["total_cost_usd"].as_f64().filter(|c| *c > 0.0) {
                    parts.push(format!("${cost:.2}"));
                }
                if !parts.is_empty() {
                    out.push(AgentEvent::Summary(parts.join(" · ")));
                }
                if event["is_error"].as_bool() == Some(true) {
                    let reason = event["result"]
                        .as_str()
                        .or_else(|| event["subtype"].as_str())
                        .unwrap_or("The agent reported an error.");
                    out.push(AgentEvent::Error(reason.to_string()));
                } else if !self.any_text {
                    if let Some(text) = event["result"].as_str().filter(|t| !t.is_empty()) {
                        out.push(self.text(text.to_string()));
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn text(&mut self, text: String) -> AgentEvent {
        self.any_text = true;
        AgentEvent::Text(text)
    }

    fn step_label(&self, tool: &str, input: &Value) -> String {
        step_label(&self.cwd, tool, input)
    }
}

/// First line of `text`, cut to `max` characters.
fn short(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").to_string();
    if line.chars().count() > max || text.contains('\n') {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line
    }
}

/// What a tool printed, from a result given as text or as content blocks.
fn result_text(content: &Value) -> String {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b["text"].as_str().or(b.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    crate::harness::event::tail(&text, 4096)
}

/// A short label for a tool call, such as "Read src/main.rs" or "Ran cargo test". Tool
/// names and argument keys differ between CLIs; unknown tools read "Used <tool>".
fn step_label(cwd: &str, tool: &str, input: &Value) -> String {
    let s = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| match &input[*k] {
                Value::String(v) => Some(v.clone()),
                // Codex and some others pass a command as its argument list.
                Value::Array(parts) => Some(
                    parts
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                _ => None,
            })
            .unwrap_or_default()
    };
    let rel = |path: String| {
        path.strip_prefix(cwd)
            .map(|p| p.trim_start_matches('/'))
            .filter(|p| !p.is_empty())
            .unwrap_or(&path)
            .replace('`', "'")
    };
    let path = || {
        rel(s(&[
            "file_path",
            "filePath",
            "path",
            "absolute_path",
            "target_file",
        ]))
    };
    match tool {
        "Read" | "read" | "view" | "read_file" | "ReadFile" => format!("Read {}", path()),
        "Edit" | "MultiEdit" | "edit" | "edit_file" | "replace" | "str_replace_editor"
        | "ApplyPatch" | "apply_patch" => match path() {
            p if p.is_empty() => "Edited files".to_string(),
            p => format!("Edited {p}"),
        },
        "Write" | "write" | "create" | "Create" | "write_file" | "create_file" => {
            format!("Wrote {}", path())
        }
        "NotebookEdit" => format!("Edited {}", rel(s(&["notebook_path"]))),
        "Bash" | "bash" | "shell" | "Execute" | "run_shell_command" | "exec_command" => {
            format!("Ran {}", short(&s(&["command", "cmd"]), 70))
        }
        "Grep" | "grep" | "search_file_content" | "search" => {
            format!("Searched for {}", short(&s(&["pattern", "query"]), 50))
        }
        "Glob" | "glob" | "LS" | "ls" | "list" | "list_directory" => {
            let what = s(&["pattern", "path", "dir_path"]);
            format!("Listed {}", short(&rel(what), 50))
        }
        "WebSearch" | "web_search" | "google_web_search" | "websearch" => {
            format!("Searched the web for {}", short(&s(&["query"]), 50))
        }
        "WebFetch" | "web_fetch" | "webfetch" | "fetch" => {
            format!("Fetched {}", short(&s(&["url", "prompt"]), 60))
        }
        "Task" | "Agent" | "task" => format!(
            "Ran a subagent: {}",
            short(&s(&["description", "prompt"]), 60)
        ),
        tool if is_todo_tool(tool) => "Updated the plan".to_string(),
        "ExitPlanMode" => "Proposed a plan".to_string(),
        "AskUserQuestion" => "Asked a question".to_string(),
        other => format!("Used {other}"),
    }
}

/// Whether `tool` is one of the CLIs' own todo tools.
fn is_todo_tool(tool: &str) -> bool {
    matches!(
        tool,
        "TodoWrite" | "todowrite" | "write_todos" | "update_todo" | "todo_write" | "update_plan"
    )
}

/// The todo list a todo tool was given, under any of the shapes the CLIs use:
/// `{"todos":[{"content":…,"status":"in_progress"}]}` (Claude, Gemini, OpenCode, Cursor),
/// `{"plan":[{"step":…,"status":…}]}` and Codex's `{"items":[{"text":…,"completed":true}]}`.
fn todos(input: &Value) -> Option<Vec<Step>> {
    let items = ["todos", "plan", "items"]
        .iter()
        .find_map(|k| input[*k].as_array())?;
    let steps: Vec<Step> = items
        .iter()
        .filter_map(|item| {
            let text = ["content", "text", "step", "description", "title", "task"]
                .iter()
                .find_map(|k| item[*k].as_str())
                .or(item.as_str())?;
            let status = ["status", "state"]
                .iter()
                .find_map(|k| item[*k].as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mark = if item["completed"] == true
                || status.contains("complete")
                || status.contains("done")
            {
                Mark::Done
            } else if status.contains("progress")
                || status.contains("doing")
                || status.contains("active")
            {
                Mark::Doing
            } else {
                Mark::Todo
            };
            Some(Step {
                text: text.to_string(),
                mark,
            })
        })
        .collect();
    let steps = crate::crew::clean_steps(steps);
    (!steps.is_empty()).then_some(steps)
}

/// Per-run parser for the JSON lines of Codex (`exec --json`), Cursor and Droid
/// (`stream-json`), Gemini (`stream-json`), OpenCode (`--format json`) and Copilot
/// (`--output-format json`). Unknown events are skipped; lines that are not JSON pass
/// through as text.
pub struct JsonStream {
    cwd: String,
    any_text: bool,
    /// Copilot sent the current message in pieces, so its full copy is not added again.
    streamed: bool,
    usage: Usage,
    named: bool,
    /// Codex items already shown as a step, by id.
    shown: std::collections::HashSet<String>,
    /// Codex calls shown while they ran, whose result is still to come, by id.
    running: std::collections::HashSet<String>,
    /// How much of each Codex message is already shown, by item id.
    written: std::collections::HashMap<String, usize>,
    /// The last error reported, so the same failure isn't shown twice.
    last_error: String,
    /// Warnings already shown, so a repeated one isn't.
    notices: std::collections::HashSet<String>,
}

impl JsonStream {
    pub fn new(cwd: &str) -> Self {
        Self {
            cwd: cwd.trim_end_matches('/').to_string(),
            any_text: false,
            streamed: false,
            usage: Usage::default(),
            named: false,
            shown: Default::default(),
            running: Default::default(),
            written: Default::default(),
            last_error: String::new(),
            notices: Default::default(),
        }
    }

    /// A warning, unless it was already shown in this run.
    fn notice(&mut self, message: String) -> Option<AgentEvent> {
        let message = message.trim().to_string();
        if message.is_empty() || !self.notices.insert(message.clone()) {
            return None;
        }
        Some(AgentEvent::Notice(message))
    }

    /// An error, unless it repeats the one just reported.
    fn error(&mut self, message: String) -> Option<AgentEvent> {
        if message == self.last_error {
            return None;
        }
        self.last_error = message.clone();
        Some(AgentEvent::Error(message))
    }

    /// A Codex item as it starts, grows (`done` false) or finishes. Messages stream in as
    /// their text grows; other items show as a step once, as soon as they can be named.
    fn codex_item(&mut self, item: &Value, done: bool) -> Vec<AgentEvent> {
        let id = item["id"].as_str().unwrap_or("").to_string();
        let kind = item["type"].as_str().unwrap_or("");
        if kind == "agent_message" {
            let text = item["text"].as_str().unwrap_or("");
            if id.is_empty() {
                return if done {
                    self.block(text).into_iter().collect()
                } else {
                    vec![]
                };
            }
            let written = self.written.get(&id).copied().unwrap_or(0);
            // Text that shrank or was rewritten mid-character is left as first shown.
            let Some(rest) = text.get(written..) else {
                return vec![];
            };
            let rest = if written == 0 {
                rest.trim_start()
            } else {
                rest
            };
            let rest = if done { rest.trim_end() } else { rest };
            if rest.is_empty() {
                return vec![];
            }
            self.written.insert(id, text.len());
            let sep = if written == 0 && self.any_text {
                "\n\n"
            } else {
                ""
            };
            self.any_text = true;
            return vec![AgentEvent::Text(format!("{sep}{rest}"))];
        }
        // A todo list is named once but each change to it is passed on.
        if kind == "todo_list" {
            let mut out = Vec::new();
            if id.is_empty() || self.shown.insert(id) {
                out.push(AgentEvent::Tool(ToolStep::of(
                    "update_plan",
                    "Updated the plan",
                )));
            }
            out.extend(todos(item).map(AgentEvent::Todos));
            return out;
        }
        if !id.is_empty() && self.shown.contains(&id) {
            // A call shown while it ran: its result.
            if done && self.running.remove(&id) {
                return vec![AgentEvent::ToolDone {
                    ok: codex_ok(item),
                    output: codex_output(item),
                    id,
                }];
            }
            return vec![];
        }
        let mut out = Vec::new();
        match kind {
            "command_execution" => {
                let command = item["command"].as_str().unwrap_or("");
                if !command.is_empty() {
                    let label = format!("Ran {}", short(&shell_body(command), 70));
                    out.extend(self.codex_call(&id, "shell", label, item, done));
                }
            }
            "file_change" => {
                for change in item["changes"].as_array().into_iter().flatten() {
                    let path = change["path"].as_str().unwrap_or("").to_string();
                    let verb = match change["kind"].as_str().unwrap_or("") {
                        "add" => "Wrote",
                        "delete" => "Deleted",
                        _ => "Edited",
                    };
                    let path = path
                        .strip_prefix(&self.cwd)
                        .map(|p| p.trim_start_matches('/').to_string())
                        .unwrap_or(path);
                    let kind = match verb {
                        "Wrote" => ToolKind::Write,
                        "Deleted" => ToolKind::Delete,
                        _ => ToolKind::Edit,
                    };
                    let mut step = ToolStep::of("apply_patch", format!("{verb} {path}")).kind(kind);
                    if !codex_ok(item) {
                        step.finish(false);
                    }
                    out.push(AgentEvent::Tool(step));
                }
            }
            "mcp_tool_call" => {
                let tool = item["tool"].as_str().unwrap_or("a tool");
                let label = format!("Used {} {tool}", item["server"].as_str().unwrap_or(""));
                let calls = self.codex_call(&id, tool, label, item, done);
                out.extend(calls.into_iter().map(|c| match c {
                    AgentEvent::Tool(step) => AgentEvent::Tool(step.kind(ToolKind::Mcp)),
                    other => other,
                }));
            }
            "web_search" => {
                let query = item["query"].as_str().unwrap_or("");
                if !query.is_empty() || done {
                    out.push(AgentEvent::Tool(ToolStep::of(
                        "web_search",
                        format!("Searched the web for {}", short(query, 50)),
                    )));
                }
            }
            // Warnings that don't stop the run, such as "Model metadata for … not found".
            "error" | "warning" => {
                let message = error_message(&item["message"])
                    .or_else(|| error_message(&item["text"]))
                    .unwrap_or_default();
                out.extend(self.notice(message));
            }
            // Reasoning summaries start with a bold heading, such as "**Reading the tests**".
            "reasoning" => {
                let text = item["text"].as_str().unwrap_or("");
                let heading = text
                    .lines()
                    .map(|l| l.trim().trim_matches('*').trim())
                    .find(|l| !l.is_empty())
                    .unwrap_or("");
                if !heading.is_empty() {
                    out.push(AgentEvent::Tool(
                        ToolStep::done(format!("Thought: {}", short(heading, 70)))
                            .kind(ToolKind::Think),
                    ));
                }
            }
            _ => {}
        }
        if !out.is_empty() && !id.is_empty() {
            self.shown.insert(id);
        }
        out
    }

    /// A Codex call (a command, an MCP tool): running until its item completes, or
    /// finished at once when it is first seen complete.
    fn codex_call(
        &mut self,
        id: &str,
        tool: &str,
        label: String,
        item: &Value,
        done: bool,
    ) -> Vec<AgentEvent> {
        if id.is_empty() {
            let mut step = ToolStep::of(tool, label);
            if done && !codex_ok(item) {
                step.finish(false);
            }
            return vec![AgentEvent::Tool(step)];
        }
        let mut out = vec![AgentEvent::Tool(ToolStep::running(id, tool, label))];
        if done {
            out.push(AgentEvent::ToolDone {
                id: id.to_string(),
                ok: codex_ok(item),
                output: codex_output(item),
            });
        } else {
            self.running.insert(id.to_string());
        }
        out
    }

    /// A complete block of reply text, set apart from earlier ones.
    fn block(&mut self, text: &str) -> Option<AgentEvent> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let sep = if self.any_text { "\n\n" } else { "" };
        self.any_text = true;
        Some(AgentEvent::Text(format!("{sep}{text}")))
    }

    /// A piece of reply text arriving as it is written.
    fn piece(&mut self, text: &str) -> Option<AgentEvent> {
        if text.is_empty() {
            return None;
        }
        self.any_text = true;
        Some(AgentEvent::Text(text.to_string()))
    }

    /// A tool call's step, and the todo list it wrote when it is a todo tool. With an
    /// `id` the call runs until a result names it; without, it is already done.
    fn tool(&self, tool: &str, input: &Value, id: &str) -> Vec<AgentEvent> {
        let label = step_label(&self.cwd, tool, input);
        let step = if id.is_empty() {
            ToolStep::of(tool, label)
        } else {
            ToolStep::running(id, tool, label)
        };
        let mut out = vec![AgentEvent::Tool(step)];
        if is_todo_tool(tool) {
            out.extend(todos(input).map(AgentEvent::Todos));
        }
        out
    }

    pub fn feed(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
            return if line.trim().is_empty() {
                vec![]
            } else {
                vec![AgentEvent::Text(line.to_string())]
            };
        };
        let mut out = Vec::new();
        if !event.is_object() {
            return out;
        }
        if !self.named {
            if let Some(id) = conversation_of(&event) {
                self.named = true;
                out.push(AgentEvent::Conversation(id));
            }
        }
        let num = |v: &Value| v.as_u64().unwrap_or(0);
        let data = &event["data"];
        match event["type"].as_str().unwrap_or("") {
            // Codex: items show as they start and grow, not only once they finish.
            "item.started" | "item.updated" | "item.completed" => {
                let done = event["type"] == "item.completed";
                out.extend(self.codex_item(&event["item"], done));
            }
            "turn.completed" => {
                let u = &event["usage"];
                let cached = num(&u["cached_input_tokens"]);
                // Codex counts cached tokens inside the input.
                self.usage.input = num(&u["input_tokens"]).saturating_sub(cached);
                self.usage.cached = cached;
                self.usage.output = num(&u["output_tokens"]);
                if !self.usage.is_empty() {
                    out.push(AgentEvent::Usage(self.usage));
                }
            }
            "turn.failed" => out.extend(
                self.error(
                    error_message(&event["error"])
                        .unwrap_or_else(|| "The agent reported an error.".into()),
                ),
            ),
            // Cursor: complete assistant messages; Amp-style content blocks.
            "assistant" => {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str().unwrap_or("") {
                        "text" => out.extend(self.block(block["text"].as_str().unwrap_or(""))),
                        "tool_use" => out.extend(self.tool(
                            block["name"].as_str().unwrap_or("a tool"),
                            &block["input"],
                            "",
                        )),
                        _ => {}
                    }
                }
            }
            "tool_call" => {
                if let Some(call) = event["tool_call"].as_object() {
                    // Cursor: {"subtype":"started","call_id":…,"tool_call":{"readToolCall":
                    // {"args":{…}}}}, then "completed" with the call's `result` added.
                    let id = event["call_id"].as_str().unwrap_or("");
                    if let Some((kind, call)) = call.iter().next() {
                        if event["subtype"] == "started" {
                            out.extend(self.cursor_step(kind, call, id));
                        } else if event["subtype"] == "completed" && !id.is_empty() {
                            let result = &call["result"];
                            let failed = result.get("success").is_none()
                                && ["error", "failure", "rejected", "permissionDenied"]
                                    .iter()
                                    .any(|k| result.get(*k).is_some());
                            out.push(AgentEvent::ToolDone {
                                id: id.to_string(),
                                ok: !failed,
                                output: String::new(),
                            });
                        }
                    }
                } else if let Some(tool) = event["toolName"].as_str() {
                    // Droid: {"type":"tool_call","toolName":"Read","parameters":{…}}.
                    out.extend(self.tool(tool, &event["parameters"], ""));
                }
            }
            // Droid and Gemini.
            "message" if event["role"] == "assistant" => {
                if let Some(text) = event["content"].as_str() {
                    // Gemini sends deltas.
                    if event["delta"] == true {
                        out.extend(self.piece(text));
                    } else {
                        out.extend(self.block(text));
                    }
                } else if let Some(text) = event["text"].as_str() {
                    out.extend(self.block(text));
                }
            }
            "tool_use" => {
                if let Some(tool) = event["tool_name"].as_str() {
                    // Gemini; its result follows as `tool_result` under `tool_id`.
                    let id = event["tool_id"].as_str().unwrap_or("");
                    out.extend(self.tool(tool, &event["parameters"], id));
                } else if let Some(tool) = event["part"]["tool"].as_str() {
                    // OpenCode, once the call is over.
                    let state = &event["part"]["state"];
                    let mut calls = self.tool(tool, &state["input"], "");
                    if state["status"] == "error" {
                        if let Some(AgentEvent::Tool(step)) = calls.first_mut() {
                            step.finish(false);
                        }
                    }
                    out.extend(calls);
                }
            }
            // Gemini.
            "tool_result" => {
                if let Some(id) = event["tool_id"].as_str().filter(|id| !id.is_empty()) {
                    let output = match &event["output"] {
                        Value::String(text) => text.clone(),
                        _ => error_message(&event["error"]).unwrap_or_default(),
                    };
                    out.push(AgentEvent::ToolDone {
                        id: id.to_string(),
                        ok: event["status"] != "error",
                        output: crate::harness::event::tail(&output, 4096),
                    });
                }
            }
            // OpenCode.
            "text" => out.extend(self.block(event["part"]["text"].as_str().unwrap_or(""))),
            "step_finish" => {
                let part = &event["part"];
                let tokens = &part["tokens"];
                if tokens.is_object() {
                    // The latest step's prompt is the context; output adds up.
                    self.usage.input = num(&tokens["input"]);
                    self.usage.cached =
                        num(&tokens["cache"]["read"]) + num(&tokens["cache"]["write"]);
                    self.usage.output += num(&tokens["output"]) + num(&tokens["reasoning"]);
                }
                self.usage.cost_cents += part["cost"].as_f64().unwrap_or(0.0) * 100.0;
                if !self.usage.is_empty() {
                    out.push(AgentEvent::Usage(self.usage));
                }
            }
            // Cursor and Gemini.
            "result" => {
                let stats = &event["stats"];
                let usage = if stats.is_object() {
                    stats
                } else {
                    &event["usage"]
                };
                if usage.is_object() {
                    let cached = num(&usage["cached"])
                        + num(&usage["cache_read_input_tokens"])
                        + num(&usage["cache_creation_input_tokens"]);
                    self.usage.input =
                        num(&usage["input_tokens"]).saturating_sub(num(&usage["cached"]));
                    self.usage.cached = cached;
                    self.usage.output = num(&usage["output_tokens"]);
                    if !self.usage.is_empty() {
                        out.push(AgentEvent::Usage(self.usage));
                    }
                }
                let ms = event["duration_ms"]
                    .as_u64()
                    .or(stats["duration_ms"].as_u64());
                if let Some(ms) = ms {
                    out.push(AgentEvent::Summary(duration(ms)));
                }
                if event["is_error"] == true || event["status"] == "error" {
                    let reason = event["result"]
                        .as_str()
                        .or(event["error"]["message"].as_str())
                        .unwrap_or("The agent reported an error.");
                    out.push(AgentEvent::Error(reason.to_string()));
                } else if !self.any_text {
                    out.extend(self.block(event["result"].as_str().unwrap_or("")));
                }
            }
            // Droid's final line.
            "completion" => {
                let u = &event["usage"];
                if u.is_object() {
                    self.usage.input = num(&u["input_tokens"]);
                    self.usage.cached =
                        num(&u["cache_read_input_tokens"]) + num(&u["cache_creation_input_tokens"]);
                    self.usage.output = num(&u["output_tokens"]);
                    if !self.usage.is_empty() {
                        out.push(AgentEvent::Usage(self.usage));
                    }
                }
                let mut parts = Vec::new();
                if let Some(ms) = event["durationMs"].as_u64() {
                    parts.push(duration(ms));
                }
                if let Some(turns) = event["numTurns"].as_u64() {
                    parts.push(format!("{turns} turn{}", if turns == 1 { "" } else { "s" }));
                }
                if !parts.is_empty() {
                    out.push(AgentEvent::Summary(parts.join(" · ")));
                }
                if !self.any_text {
                    out.extend(self.block(event["finalText"].as_str().unwrap_or("")));
                }
            }
            // Copilot's session events; a subagent's are summed up by its parent's step.
            "assistant.message_delta" if data["parentToolCallId"].is_null() => {
                self.streamed = true;
                out.extend(self.piece(data["deltaContent"].as_str().unwrap_or("")));
            }
            "assistant.message" if data["parentToolCallId"].is_null() => {
                if std::mem::take(&mut self.streamed) {
                    // Already shown piece by piece; keep the next message apart.
                    if self.any_text {
                        out.push(AgentEvent::Text("\n\n".into()));
                    }
                } else {
                    out.extend(self.block(data["content"].as_str().unwrap_or("")));
                }
            }
            "tool.execution_start" if data["parentToolCallId"].is_null() => {
                let tool = data["toolName"].as_str().unwrap_or("a tool");
                let args = match &data["arguments"] {
                    Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                    other => other.clone(),
                };
                let id = data["toolCallId"].as_str().unwrap_or("");
                out.extend(self.tool(tool, &args, id));
            }
            "tool.execution_complete" if data["parentToolCallId"].is_null() => {
                if let Some(id) = data["toolCallId"].as_str().filter(|id| !id.is_empty()) {
                    out.push(AgentEvent::ToolDone {
                        id: id.to_string(),
                        ok: data["success"] != false,
                        output: result_text(&data["result"]["content"]),
                    });
                }
            }
            "assistant.usage" => {
                self.usage.input = num(&data["inputTokens"]);
                self.usage.cached = num(&data["cacheReadTokens"]) + num(&data["cacheWriteTokens"]);
                self.usage.output += num(&data["outputTokens"]);
                if !self.usage.is_empty() {
                    out.push(AgentEvent::Usage(self.usage));
                }
            }
            "session.error" => out.push(AgentEvent::Error(
                data["message"]
                    .as_str()
                    .unwrap_or("The agent reported an error.")
                    .to_string(),
            )),
            "error" => {
                // Codex puts the text in `message`, OpenCode in `error.data.message`.
                let message = error_message(&event["message"])
                    .or_else(|| error_message(&event["error"]))
                    .unwrap_or_default();
                // Codex reports its own retries as errors; only the final failure counts.
                if !message.is_empty() && !message.starts_with("Reconnecting") {
                    out.extend(self.error(message));
                }
            }
            _ => {}
        }
        out
    }

    /// A Cursor tool call, keyed by its kind (`readToolCall`, `shellToolCall`…).
    fn cursor_step(&self, kind: &str, call: &Value, id: &str) -> Vec<AgentEvent> {
        let args = &call["args"];
        let tool = match kind {
            "readToolCall" => "Read",
            "editToolCall" => "Edit",
            "writeToolCall" => "Write",
            "shellToolCall" => "Bash",
            "grepToolCall" => "Grep",
            "globToolCall" => "Glob",
            "lsToolCall" => "LS",
            "updateTodosToolCall" | "todoToolCall" => "TodoWrite",
            "deleteToolCall" => {
                let path = args["path"].as_str().unwrap_or("");
                let label = format!(
                    "Deleted {}",
                    path.strip_prefix(&self.cwd)
                        .map(|p| p.trim_start_matches('/'))
                        .unwrap_or(path)
                );
                return vec![AgentEvent::Tool(cursor_call(id, "delete", label))];
            }
            "function" => {
                let name = call["name"].as_str().unwrap_or("a tool");
                return vec![AgentEvent::Tool(cursor_call(
                    id,
                    name,
                    format!("Used {name}"),
                ))];
            }
            other => {
                let name = other.strip_suffix("ToolCall").unwrap_or(other);
                return vec![AgentEvent::Tool(cursor_call(
                    id,
                    name,
                    format!("Used {name}"),
                ))];
            }
        };
        let mut input = args.clone();
        if tool == "Glob" {
            if let Some(p) = args["globPattern"].as_str() {
                input["pattern"] = p.into();
            }
        }
        self.tool(tool, &input, id)
    }
}

/// A Cursor call: running until its "completed" event when it has an id.
fn cursor_call(id: &str, tool: &str, label: String) -> ToolStep {
    if id.is_empty() {
        ToolStep::of(tool, label)
    } else {
        ToolStep::running(id, tool, label)
    }
}

/// Whether a Codex item finished well: not failed or declined, and a command that
/// exited 0.
fn codex_ok(item: &Value) -> bool {
    !matches!(item["status"].as_str(), Some("failed" | "declined"))
        && item["exit_code"].as_i64().is_none_or(|code| code == 0)
        && item["error"].is_null()
}

/// The end of what a Codex command or MCP call printed.
fn codex_output(item: &Value) -> String {
    if let Some(output) = item["aggregated_output"].as_str() {
        return crate::harness::event::tail(output, 4096);
    }
    match error_message(&item["error"]) {
        Some(error) => error,
        None => result_text(&item["result"]["content"]),
    }
}

/// The readable text of an error: a string, or the message inside an object such as
/// OpenCode's `{"name":…,"data":{"message":…}}`. Messages that are themselves JSON, as
/// Codex passes on from the API, or end in it ("unexpected status 400: {…}"), give the
/// message inside.
fn error_message(error: &Value) -> Option<String> {
    match error {
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let inner = text
                .find('{')
                .and_then(|at| serde_json::from_str::<Value>(&text[at..]).ok())
                .filter(Value::is_object)
                .and_then(|v| error_message(&v));
            Some(inner.unwrap_or_else(|| text.to_string()))
        }
        Value::Object(map) => ["message", "detail", "error_description", "error", "data"]
            .iter()
            .find_map(|k| map.get(*k).and_then(error_message))
            .or_else(|| map.get("name").and_then(Value::as_str).map(str::to_string)),
        _ => None,
    }
}

/// An error line from a CLI's log, with any JSON in it reduced to the message inside, as
/// in Codex's "ERROR: {\"error\":{\"message\":…}}".
pub fn readable_error(line: &str) -> String {
    let Some(at) = line.find('{') else {
        return line.to_string();
    };
    match error_message(&Value::String(line[at..].to_string())) {
        Some(message) if !message.starts_with('{') => {
            format!("{}{message}", &line[..at])
        }
        _ => line.to_string(),
    }
}

/// The command inside the shell wrapper Codex runs it in, such as `bash -lc '…'` or
/// `/bin/zsh -lc …` on macOS.
fn shell_body(command: &str) -> String {
    let mut words = command.splitn(3, ' ');
    let (Some(shell), Some(flag), Some(rest)) = (words.next(), words.next(), words.next()) else {
        return command.to_string();
    };
    let name = shell.rsplit('/').next().unwrap_or(shell);
    if !matches!(name, "bash" | "sh" | "zsh") || !matches!(flag, "-c" | "-lc") {
        return command.to_string();
    }
    let rest = rest.trim();
    rest.strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
        .or_else(|| rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
        .unwrap_or(rest)
        .to_string()
}

fn duration(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0 => format!("{ms}ms"),
        1..=59 => format!("{secs}s"),
        _ => format!("{}m {}s", secs / 60, secs % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_denials_plans_and_questions() {
        let out = feed_all(&[
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"usage":{"input_tokens":10,"cache_read_input_tokens":2000,"cache_creation_input_tokens":500,"output_tokens":5},"content":[{"type":"tool_use","name":"ExitPlanMode","input":{"plan":"1. Do it"}}]}}"#,
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","name":"AskUserQuestion","input":{"questions":[{"question":"Which DB?","options":[{"label":"Postgres"},{"label":"SQLite"}]}]}}]}}"#,
            r#"{"type":"result","is_error":false,"usage":{"input_tokens":30,"output_tokens":400},"total_cost_usd":0.05,"modelUsage":{"m":{"contextWindow":200000}},"permission_denials":[{"tool_name":"Bash"},{"tool_name":"Bash"},{"tool_name":"ExitPlanMode"}],"result":"ok"}"#,
        ]);
        let usage = Usage {
            input: 10,
            cached: 2500,
            output: 0,
            cost_cents: 0.0,
            window: 0,
        };
        assert_eq!(out[0], AgentEvent::Usage(usage));
        assert_eq!(out[1], AgentEvent::Plan("1. Do it".into()));
        assert_eq!(out[2], done("Proposed a plan"));
        assert_eq!(
            out[3],
            AgentEvent::Questions(vec![Question {
                question: "Which DB?".into(),
                options: vec!["Postgres".into(), "SQLite".into()]
            }])
        );
        assert_eq!(out[4], done("Asked a question"));
        assert_eq!(
            out[5],
            AgentEvent::Usage(Usage {
                output: 400,
                cost_cents: 5.0,
                window: 200_000,
                ..usage
            })
        );
        assert_eq!(out[6], AgentEvent::Denied(vec!["Bash".into()]));
        assert_eq!(usage.context(), 2510);
    }

    fn feed_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = ClaudeStream::new("/work/app/");
        lines
            .iter()
            .flat_map(|l| parser.feed(l))
            .map(plain)
            .collect()
    }

    /// Events as most tests write them: a step reduced to its id, label and state, a
    /// result to its id and outcome. Kinds and outputs have tests of their own.
    fn plain(event: AgentEvent) -> AgentEvent {
        match event {
            AgentEvent::Tool(s) => AgentEvent::Tool(ToolStep {
                id: s.id,
                label: s.label,
                state: s.state,
                ..ToolStep::default()
            }),
            AgentEvent::ToolDone { id, ok, .. } => AgentEvent::ToolDone {
                id,
                ok,
                output: String::new(),
            },
            other => other,
        }
    }

    fn done(label: &str) -> AgentEvent {
        AgentEvent::Tool(ToolStep::done(label))
    }

    fn started(id: &str, label: &str) -> AgentEvent {
        AgentEvent::Tool(ToolStep {
            id: id.into(),
            label: label.into(),
            state: crate::harness::StepState::Running,
            ..ToolStep::default()
        })
    }

    fn finished(id: &str, ok: bool) -> AgentEvent {
        AgentEvent::ToolDone {
            id: id.into(),
            ok,
            output: String::new(),
        }
    }

    #[test]
    fn complete_messages_become_steps_and_text() {
        let out = feed_all(&[
            r#"{"type":"system","subtype":"init","cwd":"/work/app"}"#,
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/work/app/src/main.rs"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"..."}]}}"#,
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test\nmore"}},{"type":"tool_use","name":"Edit","input":{"file_path":"/elsewhere/x.rs"}}]}}"#,
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"All done."}]}}"#,
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Second block."}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":12400,"num_turns":3,"total_cost_usd":0.0412,"result":"All done."}"#,
        ]);
        assert_eq!(
            out,
            [
                done("Read src/main.rs"),
                done("Ran cargo test…"),
                done("Edited /elsewhere/x.rs"),
                AgentEvent::Text("All done.".into()),
                AgentEvent::Text("\n\nSecond block.".into()),
                AgentEvent::Summary("12s · 3 turns · $0.04".into()),
            ]
        );
    }

    #[test]
    fn tool_steps_run_until_their_result_arrives() {
        let out = feed_all(&[
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"toolu_a","name":"Bash","input":{"command":"sleep 60"}},{"type":"tool_use","id":"toolu_b","name":"Bash","input":{"command":"false"}}]}}"#,
            r#"{"type":"user","parent_tool_use_id":"toolu_x","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_inner","content":"..."}]}}"#,
            r#"{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_b","is_error":true,"content":"exit 1"}]}}"#,
            r#"{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_a","content":""}]}}"#,
        ]);
        assert_eq!(
            out,
            [
                started("toolu_a", "Ran sleep 60"),
                started("toolu_b", "Ran false"),
                finished("toolu_b", false),
                finished("toolu_a", true),
            ]
        );
    }

    #[test]
    fn streamed_text_is_not_repeated() {
        let out = feed_all(&[
            r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"text"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"lo"}}}"#,
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"Hello"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"text"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Bye"}}}"#,
        ]);
        assert_eq!(
            out,
            [
                AgentEvent::Text("Hel".into()),
                AgentEvent::Text("lo".into()),
                AgentEvent::Text("\n\n".into()),
                AgentEvent::Text("Bye".into()),
            ]
        );
    }

    #[test]
    fn subagent_messages_are_skipped_and_errors_reported() {
        let out = feed_all(&[
            r#"{"type":"assistant","parent_tool_use_id":"toolu_1","message":{"content":[{"type":"text","text":"inner"}]}}"#,
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"duration_ms":900,"num_turns":1}"#,
        ]);
        assert_eq!(
            out,
            [
                AgentEvent::Summary("900ms · 1 turn".into()),
                AgentEvent::Error("error_max_turns".into())
            ]
        );
    }

    #[test]
    fn plain_lines_pass_through_and_result_fills_silent_runs() {
        let out = feed_all(&[
            "not json",
            "",
            r#"{"type":"result","is_error":false,"result":"Only the final answer","duration_ms":61000}"#,
        ]);
        assert_eq!(
            out,
            [
                AgentEvent::Text("not json".into()),
                AgentEvent::Summary("1m 1s".into()),
            ],
            "text already shown, so the result is not repeated"
        );
        let out =
            feed_all(&[r#"{"type":"result","is_error":false,"result":"Only the final answer"}"#]);
        assert_eq!(out, [AgentEvent::Text("Only the final answer".into())]);
    }

    fn json_all(lines: &[&str]) -> Vec<AgentEvent> {
        let mut parser = JsonStream::new("/work/app");
        lines
            .iter()
            .flat_map(|l| parser.feed(l))
            .map(plain)
            .collect()
    }

    #[test]
    fn codex_events_become_steps_text_usage_and_the_thread_id() {
        let out = json_all(&[
            r#"{"type":"thread.started","thread_id":"0199a213-81c0-7800-8aa1-bbab2a035a53"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"error","message":"Reconnecting... 2/5 (stream disconnected before completion)"}"#,
            r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"**Looking**"}}"#,
            r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"bash -lc 'ls -a'","status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"bash -lc 'ls -a'","aggregated_output":"a\nb","exit_code":0,"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_2","type":"file_change","changes":[{"path":"/work/app/src/lib.rs","kind":"update"},{"path":"/work/app/NEW.md","kind":"add"}],"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"Done."}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":24763,"cached_input_tokens":24448,"output_tokens":122}}"#,
        ]);
        assert_eq!(
            out,
            [
                AgentEvent::Conversation("0199a213-81c0-7800-8aa1-bbab2a035a53".into()),
                done("Thought: Looking"),
                started("item_1", "Ran ls -a"),
                finished("item_1", true),
                done("Edited src/lib.rs"),
                done("Wrote NEW.md"),
                AgentEvent::Text("Done.".into()),
                AgentEvent::Usage(Usage {
                    input: 315,
                    cached: 24448,
                    output: 122,
                    cost_cents: 0.0,
                    window: 0
                }),
            ]
        );
        assert_eq!(
            json_all(&[r#"{"type":"turn.failed","error":{"message":"usage limit reached"}}"#]),
            [AgentEvent::Error("usage limit reached".into())]
        );
    }

    #[test]
    fn codex_warnings_show_once() {
        let warning = "Model metadata for `gpt-6-sol` not found. Defaulting to fallback metadata; this can degrade performance and cause issues.";
        let out = json_all(&[
            &format!(
                r#"{{"type":"item.completed","item":{{"id":"item_0","type":"error","message":"{warning}"}}}}"#
            ),
            &format!(
                r#"{{"type":"item.completed","item":{{"id":"item_1","type":"error","message":"{warning}"}}}}"#
            ),
            r#"{"type":"item.completed","item":{"id":"item_2","type":"warning","message":"Heads up"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"Hi."}}"#,
        ]);
        assert_eq!(
            out,
            [
                AgentEvent::Notice(warning.into()),
                AgentEvent::Notice("Heads up".into()),
                AgentEvent::Text("Hi.".into()),
            ]
        );
    }

    #[test]
    fn codex_streams_messages_and_shows_steps_as_they_start() {
        let out = json_all(&[
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"First."}}"#,
            r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"bash -lc 'cargo test'","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#,
            r#"{"type":"item.updated","item":{"id":"item_1","type":"command_execution","command":"bash -lc 'cargo test'","aggregated_output":"running 3 tests","exit_code":null,"status":"in_progress"}}"#,
            r#"{"type":"item.started","item":{"id":"item_2","type":"agent_message","text":""}}"#,
            r#"{"type":"item.updated","item":{"id":"item_2","type":"agent_message","text":"All tests"}}"#,
            r#"{"type":"item.updated","item":{"id":"item_2","type":"agent_message","text":"All tests pass"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"bash -lc 'cargo test'","aggregated_output":"ok","exit_code":0,"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"All tests pass.\n"}}"#,
        ]);
        assert_eq!(
            out,
            [
                AgentEvent::Text("First.".into()),
                started("item_1", "Ran cargo test"),
                AgentEvent::Text("\n\nAll tests".into()),
                AgentEvent::Text(" pass".into()),
                finished("item_1", true),
                AgentEvent::Text(".".into()),
            ]
        );
    }

    #[test]
    fn error_lines_lose_their_json() {
        assert_eq!(
            readable_error(r#"ERROR: {"detail":"The model is not supported"}"#),
            "ERROR: The model is not supported"
        );
        assert_eq!(
            readable_error("Error: plain {not json"),
            "Error: plain {not json"
        );
    }

    #[test]
    fn errors_show_the_message_inside() {
        // Codex passes on the API's error as JSON inside the message, and says it twice.
        let codex = json_all(&[
            r#"{"type":"error","message":"{\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'gpt-6-sol' model is not supported when using Codex with a ChatGPT account.\"}}"}"#,
            r#"{"type":"turn.failed","error":{"message":"{\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'gpt-6-sol' model is not supported when using Codex with a ChatGPT account.\"}}"}}"#,
        ]);
        assert_eq!(
            codex,
            [AgentEvent::Error(
                "The 'gpt-6-sol' model is not supported when using Codex with a ChatGPT account."
                    .into()
            )]
        );
        assert_eq!(
            json_all(&[
                r#"{"type":"error","message":"unexpected status 401 Unauthorized: {\"detail\":\"Your token expired\"}"}"#
            ]),
            [AgentEvent::Error("Your token expired".into())]
        );
        let opencode = json_all(&[
            r#"{"type":"error","timestamp":1760000000000,"sessionID":"ses_9","error":{"name":"UnknownError","data":{"message":"Error: Token refresh failed: 401"}}}"#,
        ]);
        assert_eq!(
            opencode,
            [
                AgentEvent::Conversation("ses_9".into()),
                AgentEvent::Error("Error: Token refresh failed: 401".into())
            ]
        );
        assert_eq!(
            json_all(&[r#"{"type":"error","error":{"name":"ProviderAuthError","data":{}}}"#]),
            [AgentEvent::Error("ProviderAuthError".into())]
        );
    }

    fn step(text: &str, mark: Mark) -> Step {
        Step {
            text: text.into(),
            mark,
        }
    }

    #[test]
    fn todo_tools_pass_on_the_whole_list() {
        let out = feed_all(&[
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"Read docs","status":"completed","activeForm":"Reading docs"},{"content":"Start docker","status":"in_progress"},{"content":"Verify","status":"pending"}]}}]}}"#,
        ]);
        assert_eq!(
            out,
            [
                AgentEvent::Todos(vec![
                    step("Read docs", Mark::Done),
                    step("Start docker", Mark::Doing),
                    step("Verify", Mark::Todo),
                ]),
                done("Updated the plan"),
            ]
        );
        // Codex names its list once, then passes on every change to it.
        let codex = json_all(&[
            r#"{"type":"item.started","item":{"id":"t1","type":"todo_list","items":[{"text":"Build","completed":false},{"text":"Test","completed":false}]}}"#,
            r#"{"type":"item.updated","item":{"id":"t1","type":"todo_list","items":[{"text":"Build","completed":true},{"text":"Test","completed":false}]}}"#,
            r#"{"type":"item.completed","item":{"id":"t1","type":"todo_list","items":[{"text":"Build","completed":true},{"text":"Test","completed":true}]}}"#,
        ]);
        assert_eq!(
            codex,
            [
                done("Updated the plan"),
                AgentEvent::Todos(vec![step("Build", Mark::Todo), step("Test", Mark::Todo)]),
                AgentEvent::Todos(vec![step("Build", Mark::Done), step("Test", Mark::Todo)]),
                AgentEvent::Todos(vec![step("Build", Mark::Done), step("Test", Mark::Done)]),
            ]
        );
        let cursor = json_all(&[
            r#"{"type":"tool_call","subtype":"started","call_id":"z","tool_call":{"updateTodosToolCall":{"args":{"todos":[{"content":"Fix","status":"TODO_STATUS_COMPLETED"},{"content":"Ship","status":"TODO_STATUS_IN_PROGRESS"}]}}}}"#,
        ]);
        assert_eq!(
            cursor,
            [
                started("z", "Updated the plan"),
                AgentEvent::Todos(vec![step("Fix", Mark::Done), step("Ship", Mark::Doing)]),
            ]
        );
        // A list in a shape no CLI uses is only a step.
        let odd = json_all(&[
            r#"{"type":"tool_use","tool_name":"write_todos","parameters":{"todos":"- a"}}"#,
        ]);
        assert_eq!(odd, [done("Updated the plan")]);
    }

    #[test]
    fn cursor_and_droid_stream_json() {
        let cursor = json_all(&[
            r#"{"type":"system","subtype":"init","cwd":"/work/app","session_id":"c-1","model":"gpt-5"}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]},"session_id":"c-1"}"#,
            r#"{"type":"tool_call","subtype":"started","call_id":"x","tool_call":{"readToolCall":{"args":{"path":"/work/app/README.md"}}},"session_id":"c-1"}"#,
            r#"{"type":"tool_call","subtype":"completed","call_id":"x","tool_call":{"readToolCall":{"args":{"path":"/work/app/README.md"},"result":{"success":{}}}},"session_id":"c-1"}"#,
            r#"{"type":"tool_call","subtype":"started","call_id":"y","tool_call":{"shellToolCall":{"args":{"command":"cargo test"}}},"session_id":"c-1"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"All good."}]},"session_id":"c-1"}"#,
            r#"{"type":"result","subtype":"success","duration_ms":4200,"is_error":false,"result":"All good.","session_id":"c-1"}"#,
        ]);
        assert_eq!(
            cursor,
            [
                AgentEvent::Conversation("c-1".into()),
                started("x", "Read README.md"),
                finished("x", true),
                started("y", "Ran cargo test"),
                AgentEvent::Text("All good.".into()),
                AgentEvent::Summary("4s".into()),
            ]
        );
        let droid = json_all(&[
            r#"{"type":"system","subtype":"init","cwd":"/work/app","session_id":"d-1","tools":[],"model":"m"}"#,
            r#"{"type":"message","role":"user","id":"1","text":"hi","session_id":"d-1"}"#,
            r#"{"type":"tool_call","id":"t","messageId":"2","toolId":"Execute","toolName":"Execute","parameters":{"command":"ls"},"session_id":"d-1"}"#,
            r#"{"type":"message","role":"assistant","id":"3","text":"Two files.","session_id":"d-1"}"#,
            r#"{"type":"completion","finalText":"Two files.","numTurns":2,"durationMs":61000,"session_id":"d-1","usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":50,"cache_creation_input_tokens":0}}"#,
        ]);
        assert_eq!(
            droid,
            [
                AgentEvent::Conversation("d-1".into()),
                done("Ran ls"),
                AgentEvent::Text("Two files.".into()),
                AgentEvent::Usage(Usage {
                    input: 100,
                    cached: 50,
                    output: 20,
                    cost_cents: 0.0,
                    window: 0
                }),
                AgentEvent::Summary("1m 1s · 2 turns".into()),
            ]
        );
    }

    #[test]
    fn gemini_opencode_and_copilot_json() {
        let gemini = json_all(&[
            r#"{"type":"init","timestamp":"t","session_id":"g-1","model":"gemini-3"}"#,
            r#"{"type":"message","timestamp":"t","role":"user","content":"hi"}"#,
            r#"{"type":"tool_use","timestamp":"t","tool_name":"read_file","tool_id":"r1","parameters":{"file_path":"/work/app/a.rs"}}"#,
            r#"{"type":"message","timestamp":"t","role":"assistant","content":"Hel","delta":true}"#,
            r#"{"type":"message","timestamp":"t","role":"assistant","content":"lo","delta":true}"#,
            r#"{"type":"result","timestamp":"t","status":"success","stats":{"total_tokens":130,"input_tokens":100,"output_tokens":30,"cached":40,"input":60,"duration_ms":900,"tool_calls":1}}"#,
        ]);
        assert_eq!(
            gemini,
            [
                AgentEvent::Conversation("g-1".into()),
                started("r1", "Read a.rs"),
                AgentEvent::Text("Hel".into()),
                AgentEvent::Text("lo".into()),
                AgentEvent::Usage(Usage {
                    input: 60,
                    cached: 40,
                    output: 30,
                    cost_cents: 0.0,
                    window: 0
                }),
                AgentEvent::Summary("900ms".into()),
            ]
        );
        let opencode = json_all(&[
            r#"{"type":"tool_use","timestamp":1,"sessionID":"ses_1","part":{"type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"ls"}}}}"#,
            r#"{"type":"step_finish","timestamp":2,"sessionID":"ses_1","part":{"type":"step-finish","tokens":{"input":500,"output":10,"reasoning":5,"cache":{"read":100,"write":0}},"cost":0.002}}"#,
            r#"{"type":"text","timestamp":3,"sessionID":"ses_1","part":{"type":"text","text":"Yes"}}"#,
            r#"{"type":"step_finish","timestamp":4,"sessionID":"ses_1","part":{"type":"step-finish","tokens":{"input":600,"output":20,"reasoning":0,"cache":{"read":200,"write":0}},"cost":0.003}}"#,
        ]);
        assert_eq!(opencode[0], AgentEvent::Conversation("ses_1".into()));
        assert_eq!(opencode[1], done("Ran ls"));
        assert_eq!(opencode[3], AgentEvent::Text("Yes".into()));
        let AgentEvent::Usage(last) = opencode[4] else {
            panic!("usage expected")
        };
        assert_eq!((last.input, last.cached, last.output), (600, 200, 35));
        assert!((last.cost_cents - 0.5).abs() < 1e-9);
        let copilot = json_all(&[
            r#"{"type":"session.start","data":{"sessionId":"cp-1","copilotVersion":"1.0","producer":"cli","startTime":"t","version":1},"id":"e1","parentId":null,"timestamp":"t"}"#,
            r#"{"type":"tool.execution_start","data":{"toolCallId":"1","toolName":"view","arguments":{"path":"/work/app/x.md"}},"id":"e2","timestamp":"t"}"#,
            r#"{"type":"assistant.message_delta","data":{"messageId":"m","deltaContent":"Hi "},"id":"e3","timestamp":"t"}"#,
            r#"{"type":"assistant.message_delta","data":{"messageId":"m","deltaContent":"there"},"id":"e4","timestamp":"t"}"#,
            r#"{"type":"assistant.message","data":{"messageId":"m","content":"Hi there"},"id":"e5","timestamp":"t"}"#,
            r#"{"type":"assistant.usage","data":{"model":"m","inputTokens":900,"outputTokens":40,"cacheReadTokens":100},"id":"e6","timestamp":"t"}"#,
            r#"{"type":"session.error","data":{"errorType":"quota","message":"Out of credits"},"id":"e7","timestamp":"t"}"#,
        ]);
        assert_eq!(
            copilot,
            [
                AgentEvent::Conversation("cp-1".into()),
                started("1", "Read x.md"),
                AgentEvent::Text("Hi ".into()),
                AgentEvent::Text("there".into()),
                AgentEvent::Text("\n\n".into()),
                AgentEvent::Usage(Usage {
                    input: 900,
                    cached: 100,
                    output: 40,
                    cost_cents: 0.0,
                    window: 0
                }),
                AgentEvent::Error("Out of credits".into()),
            ]
        );
    }

    /// The finished steps and results a run's lines give, unreduced.
    fn tools(events: Vec<AgentEvent>) -> Vec<AgentEvent> {
        events
            .into_iter()
            .filter(|e| matches!(e, AgentEvent::Tool(_) | AgentEvent::ToolDone { .. }))
            .collect()
    }

    #[test]
    fn steps_carry_their_kind_tool_outcome_and_output() {
        let mut claude = ClaudeStream::new("/w");
        let out: Vec<AgentEvent> = [
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"false"}}]}}"#,
            r#"{"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"exit 1"}]}]}}"#,
        ]
        .iter()
        .flat_map(|l| claude.feed(l))
        .collect();
        assert_eq!(
            out,
            [
                AgentEvent::Tool(ToolStep::running("t1", "Bash", "Ran false")),
                AgentEvent::ToolDone {
                    id: "t1".into(),
                    ok: false,
                    output: "exit 1".into()
                },
            ]
        );
        let AgentEvent::Tool(step) = &out[0] else {
            unreachable!()
        };
        assert_eq!(step.kind, ToolKind::Shell);

        let mut json = JsonStream::new("/w");
        let mut feed = |lines: &[&str]| -> Vec<AgentEvent> {
            tools(lines.iter().flat_map(|l| json.feed(l)).collect())
        };
        // Codex: a command that fails, seen only once it is over, and an MCP call.
        let codex = feed(&[
            r#"{"type":"item.completed","item":{"id":"c1","type":"command_execution","command":"bash -lc 'cargo build'","aggregated_output":"error[E0425]","exit_code":101,"status":"failed"}}"#,
            r#"{"type":"item.started","item":{"id":"c2","type":"mcp_tool_call","server":"gh","tool":"get_me","status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"c2","type":"mcp_tool_call","server":"gh","tool":"get_me","status":"completed","result":{"content":[{"type":"text","text":"anish"}]}}}"#,
            r#"{"type":"item.completed","item":{"id":"c3","type":"file_change","status":"completed","changes":[{"path":"/w/a.rs","kind":"add"}]}}"#,
        ]);
        assert_eq!(
            codex,
            [
                AgentEvent::Tool(ToolStep::running("c1", "shell", "Ran cargo build")),
                AgentEvent::ToolDone {
                    id: "c1".into(),
                    ok: false,
                    output: "error[E0425]".into()
                },
                AgentEvent::Tool(
                    ToolStep::running("c2", "get_me", "Used gh get_me").kind(ToolKind::Mcp)
                ),
                AgentEvent::ToolDone {
                    id: "c2".into(),
                    ok: true,
                    output: "anish".into()
                },
                AgentEvent::Tool(ToolStep::of("apply_patch", "Wrote a.rs").kind(ToolKind::Write)),
            ]
        );
        // Gemini and Copilot name each result after its call.
        let gemini = feed(&[
            r#"{"type":"tool_use","tool_name":"run_shell_command","tool_id":"g1","parameters":{"command":"ls"}}"#,
            r#"{"type":"tool_result","tool_id":"g1","status":"error","error":{"message":"denied"}}"#,
        ]);
        assert_eq!(
            gemini[1],
            AgentEvent::ToolDone {
                id: "g1".into(),
                ok: false,
                output: "denied".into()
            }
        );
        let copilot = feed(&[
            r#"{"type":"tool.execution_start","data":{"toolCallId":"p1","toolName":"bash","arguments":"{\"command\":\"ls\"}"}}"#,
            r#"{"type":"tool.execution_complete","data":{"toolCallId":"p1","success":true,"result":{"content":"a.rs"}}}"#,
        ]);
        assert_eq!(
            copilot,
            [
                AgentEvent::Tool(ToolStep::running("p1", "bash", "Ran ls")),
                AgentEvent::ToolDone {
                    id: "p1".into(),
                    ok: true,
                    output: "a.rs".into()
                },
            ]
        );
        // Cursor reports a failed call; OpenCode sends calls once they're over.
        let cursor = feed(&[
            r#"{"type":"tool_call","subtype":"completed","call_id":"k1","tool_call":{"shellToolCall":{"args":{"command":"x"},"result":{"failure":{"exitCode":1}}}}}"#,
        ]);
        assert_eq!(
            cursor,
            [AgentEvent::ToolDone {
                id: "k1".into(),
                ok: false,
                output: String::new()
            }]
        );
        let opencode = feed(&[
            r#"{"type":"tool_use","part":{"tool":"bash","callID":"o1","state":{"status":"error","input":{"command":"x"}}}}"#,
        ]);
        let AgentEvent::Tool(step) = &opencode[0] else {
            panic!("{opencode:?}")
        };
        assert_eq!(step.state, crate::harness::StepState::Failed);
    }

    #[test]
    fn codex_commands_lose_their_shell_wrapper() {
        assert_eq!(shell_body("bash -lc 'cargo test'"), "cargo test");
        assert_eq!(shell_body("/bin/zsh -lc ls"), "ls");
        assert_eq!(
            shell_body("/bin/zsh -lc 'ls /nonexistent'"),
            "ls /nonexistent"
        );
        assert_eq!(shell_body("/bin/sh -c \"make\""), "make");
        assert_eq!(shell_body("python -c 'x'"), "python -c 'x'");
        assert_eq!(shell_body("ls"), "ls");
    }

    #[test]
    fn claude_format_reports_the_conversation_once() {
        let out = feed_all(&[
            r#"{"type":"system","subtype":"init","session_id":"T-1"}"#,
            r#"{"type":"result","is_error":false,"result":"ok","session_id":"T-1"}"#,
        ]);
        assert_eq!(out[0], AgentEvent::Conversation("T-1".into()));
        assert_eq!(
            out.iter()
                .filter(|u| matches!(u, AgentEvent::Conversation(_)))
                .count(),
            1
        );
    }

    #[test]
    fn step_labels_for_common_tools() {
        let p = ClaudeStream::new("/w");
        let label =
            |tool: &str, input: &str| p.step_label(tool, &serde_json::from_str(input).unwrap());
        assert_eq!(label("Write", r#"{"file_path":"/w/a.txt"}"#), "Wrote a.txt");
        assert_eq!(
            label("Grep", r#"{"pattern":"fn main"}"#),
            "Searched for fn main"
        );
        assert_eq!(label("TodoWrite", "{}"), "Updated the plan");
        assert_eq!(
            label("Task", r#"{"description":"explore"}"#),
            "Ran a subagent: explore"
        );
        assert_eq!(label("mcp__x__y", "{}"), "Used mcp__x__y");
    }
}
