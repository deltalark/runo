//! Turns agents' structured output into conversation updates: reply text as it streams,
//! a short label for each tool step, token use and a run summary. Claude Code's
//! `--output-format stream-json` (also spoken by Amp and Qwen Code) is read by
//! `ClaudeStream`; the JSON lines of Codex, Cursor, Droid, Gemini, OpenCode and Copilot
//! by `JsonStream`.

use serde_json::Value;

use crate::store::{Question, Usage};

#[derive(Debug, PartialEq)]
pub enum Update {
    /// Reply text to append to the agent's message.
    Text(String),
    /// A finished tool step, such as "Read src/main.rs".
    Step(String),
    /// The run's summary line, such as "12s · 3 turns · $0.04".
    Summary(String),
    /// The run failed; the CLI's own explanation.
    Error(String),
    /// Token use: the context after the latest model call, then totals from the result.
    Usage(Usage),
    /// Tools the agent wanted but its permission mode did not allow.
    Denied(Vec<String>),
    /// A plan proposed in plan mode.
    Plan(String),
    /// Multiple-choice questions for the person.
    Questions(Vec<Question>),
    /// The CLI's own id for the conversation, so follow-ups resume exactly this one.
    Conversation(String),
}

/// The parser for one run, by the format the agent prints.
pub enum Parser {
    Claude(ClaudeStream),
    Json(JsonStream),
}

impl Parser {
    /// The parser for a run of `provider` in `cwd`, or `None` for plain text output.
    pub fn for_provider(provider: &str, cwd: &str) -> Option<Parser> {
        match crate::agent::output(provider) {
            crate::agent::Output::Claude => Some(Parser::Claude(ClaudeStream::new(cwd))),
            crate::agent::Output::Json => Some(Parser::Json(JsonStream::new(cwd))),
            crate::agent::Output::Text => None,
        }
    }

    pub fn feed(&mut self, line: &str) -> Vec<Update> {
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
    pub fn feed(&mut self, line: &str) -> Vec<Update> {
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
                out.push(Update::Conversation(id));
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
                            out.push(Update::Text("\n\n".into()));
                        }
                    }
                    "content_block_delta" if inner["delta"]["type"] == "text_delta" => {
                        if let Some(text) = inner["delta"]["text"].as_str() {
                            self.streamed_text = true;
                            self.any_text = true;
                            out.push(Update::Text(text.to_string()));
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
                    out.push(Update::Usage(self.usage));
                }
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        match block["name"].as_str().unwrap_or("") {
                            "ExitPlanMode" => {
                                if let Some(plan) = block["input"]["plan"].as_str() {
                                    out.push(Update::Plan(plan.to_string()));
                                }
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
                                    out.push(Update::Questions(questions));
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
                        "tool_use" => out.push(Update::Step(self.step_label(
                            block["name"].as_str().unwrap_or("tool"),
                            &block["input"],
                        ))),
                        _ => {}
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
                    out.push(Update::Usage(self.usage));
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
                    out.push(Update::Denied(denied));
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
                    out.push(Update::Summary(parts.join(" · ")));
                }
                if event["is_error"].as_bool() == Some(true) {
                    let reason = event["result"]
                        .as_str()
                        .or_else(|| event["subtype"].as_str())
                        .unwrap_or("The agent reported an error.");
                    out.push(Update::Error(reason.to_string()));
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

    fn text(&mut self, text: String) -> Update {
        self.any_text = true;
        Update::Text(text)
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
        "TodoWrite" | "todowrite" | "write_todos" | "update_todo" => "Updated the plan".to_string(),
        "ExitPlanMode" => "Proposed a plan".to_string(),
        "AskUserQuestion" => "Asked a question".to_string(),
        other => format!("Used {other}"),
    }
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
}

impl JsonStream {
    pub fn new(cwd: &str) -> Self {
        Self {
            cwd: cwd.trim_end_matches('/').to_string(),
            any_text: false,
            streamed: false,
            usage: Usage::default(),
            named: false,
        }
    }

    /// A complete block of reply text, set apart from earlier ones.
    fn block(&mut self, text: &str) -> Option<Update> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let sep = if self.any_text { "\n\n" } else { "" };
        self.any_text = true;
        Some(Update::Text(format!("{sep}{text}")))
    }

    /// A piece of reply text arriving as it is written.
    fn piece(&mut self, text: &str) -> Option<Update> {
        if text.is_empty() {
            return None;
        }
        self.any_text = true;
        Some(Update::Text(text.to_string()))
    }

    fn step(&self, tool: &str, input: &Value) -> Update {
        Update::Step(step_label(&self.cwd, tool, input))
    }

    pub fn feed(&mut self, line: &str) -> Vec<Update> {
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
            return if line.trim().is_empty() {
                vec![]
            } else {
                vec![Update::Text(line.to_string())]
            };
        };
        let mut out = Vec::new();
        if !event.is_object() {
            return out;
        }
        if !self.named {
            if let Some(id) = conversation_of(&event) {
                self.named = true;
                out.push(Update::Conversation(id));
            }
        }
        let num = |v: &Value| v.as_u64().unwrap_or(0);
        let data = &event["data"];
        match event["type"].as_str().unwrap_or("") {
            // Codex.
            "item.completed" => {
                let item = &event["item"];
                match item["type"].as_str().unwrap_or("") {
                    "agent_message" => out.extend(self.block(item["text"].as_str().unwrap_or(""))),
                    "command_execution" => {
                        let command = item["command"].as_str().unwrap_or("");
                        out.push(Update::Step(format!(
                            "Ran {}",
                            short(&shell_body(command), 70)
                        )));
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
                            out.push(Update::Step(format!("{verb} {path}")));
                        }
                    }
                    "mcp_tool_call" => out.push(Update::Step(format!(
                        "Used {} {}",
                        item["server"].as_str().unwrap_or(""),
                        item["tool"].as_str().unwrap_or("a tool")
                    ))),
                    "web_search" => out.push(Update::Step(format!(
                        "Searched the web for {}",
                        short(item["query"].as_str().unwrap_or(""), 50)
                    ))),
                    "todo_list" => out.push(Update::Step("Updated the plan".into())),
                    _ => {}
                }
            }
            "turn.completed" => {
                let u = &event["usage"];
                let cached = num(&u["cached_input_tokens"]);
                // Codex counts cached tokens inside the input.
                self.usage.input = num(&u["input_tokens"]).saturating_sub(cached);
                self.usage.cached = cached;
                self.usage.output = num(&u["output_tokens"]);
                if !self.usage.is_empty() {
                    out.push(Update::Usage(self.usage));
                }
            }
            "turn.failed" => out.push(Update::Error(
                event["error"]["message"]
                    .as_str()
                    .unwrap_or("The agent reported an error.")
                    .to_string(),
            )),
            // Cursor: complete assistant messages; Amp-style content blocks.
            "assistant" => {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str().unwrap_or("") {
                        "text" => out.extend(self.block(block["text"].as_str().unwrap_or(""))),
                        "tool_use" => out.push(
                            self.step(block["name"].as_str().unwrap_or("a tool"), &block["input"]),
                        ),
                        _ => {}
                    }
                }
            }
            "tool_call" => {
                if let Some(call) = event["tool_call"].as_object() {
                    // Cursor: {"subtype":"started","tool_call":{"readToolCall":{"args":{…}}}}.
                    if event["subtype"] == "started" {
                        if let Some((kind, call)) = call.iter().next() {
                            out.push(self.cursor_step(kind, call));
                        }
                    }
                } else if let Some(tool) = event["toolName"].as_str() {
                    // Droid: {"type":"tool_call","toolName":"Read","parameters":{…}}.
                    out.push(self.step(tool, &event["parameters"]));
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
                    // Gemini.
                    out.push(self.step(tool, &event["parameters"]));
                } else if let Some(tool) = event["part"]["tool"].as_str() {
                    // OpenCode.
                    out.push(self.step(tool, &event["part"]["state"]["input"]));
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
                    out.push(Update::Usage(self.usage));
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
                        out.push(Update::Usage(self.usage));
                    }
                }
                let ms = event["duration_ms"]
                    .as_u64()
                    .or(stats["duration_ms"].as_u64());
                if let Some(ms) = ms {
                    out.push(Update::Summary(duration(ms)));
                }
                if event["is_error"] == true || event["status"] == "error" {
                    let reason = event["result"]
                        .as_str()
                        .or(event["error"]["message"].as_str())
                        .unwrap_or("The agent reported an error.");
                    out.push(Update::Error(reason.to_string()));
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
                        out.push(Update::Usage(self.usage));
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
                    out.push(Update::Summary(parts.join(" · ")));
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
                        out.push(Update::Text("\n\n".into()));
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
                out.push(self.step(tool, &args));
            }
            "assistant.usage" => {
                self.usage.input = num(&data["inputTokens"]);
                self.usage.cached = num(&data["cacheReadTokens"]) + num(&data["cacheWriteTokens"]);
                self.usage.output += num(&data["outputTokens"]);
                if !self.usage.is_empty() {
                    out.push(Update::Usage(self.usage));
                }
            }
            "session.error" => out.push(Update::Error(
                data["message"]
                    .as_str()
                    .unwrap_or("The agent reported an error.")
                    .to_string(),
            )),
            "error" => {
                let message = event["message"]
                    .as_str()
                    .or(event["error"]["message"].as_str())
                    .or(event["error"].as_str())
                    .unwrap_or("");
                // Codex reports its own retries as errors; only the final failure counts.
                if !message.is_empty() && !message.starts_with("Reconnecting") {
                    out.push(Update::Error(message.to_string()));
                }
            }
            _ => {}
        }
        out
    }

    /// A Cursor tool call, keyed by its kind (`readToolCall`, `shellToolCall`…).
    fn cursor_step(&self, kind: &str, call: &Value) -> Update {
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
                return Update::Step(format!(
                    "Deleted {}",
                    path.strip_prefix(&self.cwd)
                        .map(|p| p.trim_start_matches('/'))
                        .unwrap_or(path)
                ));
            }
            "function" => {
                return Update::Step(format!(
                    "Used {}",
                    call["name"].as_str().unwrap_or("a tool")
                ))
            }
            other => {
                let name = other.strip_suffix("ToolCall").unwrap_or(other);
                return Update::Step(format!("Used {name}"));
            }
        };
        let mut input = args.clone();
        if tool == "Glob" {
            if let Some(p) = args["globPattern"].as_str() {
                input["pattern"] = p.into();
            }
        }
        self.step(tool, &input)
    }
}

/// The command inside Codex's `bash -lc '…'` wrapper.
fn shell_body(command: &str) -> String {
    for prefix in [
        "bash -lc ",
        "/bin/bash -lc ",
        "sh -c ",
        "/bin/sh -c ",
        "zsh -lc ",
    ] {
        if let Some(rest) = command.strip_prefix(prefix) {
            let rest = rest.trim();
            return rest
                .strip_prefix('\'')
                .and_then(|r| r.strip_suffix('\''))
                .or_else(|| rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
                .unwrap_or(rest)
                .to_string();
        }
    }
    command.to_string()
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
        assert_eq!(out[0], Update::Usage(usage));
        assert_eq!(out[1], Update::Plan("1. Do it".into()));
        assert_eq!(out[2], Update::Step("Proposed a plan".into()));
        assert_eq!(
            out[3],
            Update::Questions(vec![Question {
                question: "Which DB?".into(),
                options: vec!["Postgres".into(), "SQLite".into()]
            }])
        );
        assert_eq!(out[4], Update::Step("Asked a question".into()));
        assert_eq!(
            out[5],
            Update::Usage(Usage {
                output: 400,
                cost_cents: 5.0,
                window: 200_000,
                ..usage
            })
        );
        assert_eq!(out[6], Update::Denied(vec!["Bash".into()]));
        assert_eq!(usage.context(), 2510);
    }

    fn feed_all(lines: &[&str]) -> Vec<Update> {
        let mut parser = ClaudeStream::new("/work/app/");
        lines.iter().flat_map(|l| parser.feed(l)).collect()
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
                Update::Step("Read src/main.rs".into()),
                Update::Step("Ran cargo test…".into()),
                Update::Step("Edited /elsewhere/x.rs".into()),
                Update::Text("All done.".into()),
                Update::Text("\n\nSecond block.".into()),
                Update::Summary("12s · 3 turns · $0.04".into()),
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
                Update::Text("Hel".into()),
                Update::Text("lo".into()),
                Update::Text("\n\n".into()),
                Update::Text("Bye".into()),
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
                Update::Summary("900ms · 1 turn".into()),
                Update::Error("error_max_turns".into())
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
                Update::Text("not json".into()),
                Update::Summary("1m 1s".into()),
            ],
            "text already shown, so the result is not repeated"
        );
        let out =
            feed_all(&[r#"{"type":"result","is_error":false,"result":"Only the final answer"}"#]);
        assert_eq!(out, [Update::Text("Only the final answer".into())]);
    }

    fn json_all(lines: &[&str]) -> Vec<Update> {
        let mut parser = JsonStream::new("/work/app");
        lines.iter().flat_map(|l| parser.feed(l)).collect()
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
                Update::Conversation("0199a213-81c0-7800-8aa1-bbab2a035a53".into()),
                Update::Step("Ran ls -a".into()),
                Update::Step("Edited src/lib.rs".into()),
                Update::Step("Wrote NEW.md".into()),
                Update::Text("Done.".into()),
                Update::Usage(Usage {
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
            [Update::Error("usage limit reached".into())]
        );
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
                Update::Conversation("c-1".into()),
                Update::Step("Read README.md".into()),
                Update::Step("Ran cargo test".into()),
                Update::Text("All good.".into()),
                Update::Summary("4s".into()),
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
                Update::Conversation("d-1".into()),
                Update::Step("Ran ls".into()),
                Update::Text("Two files.".into()),
                Update::Usage(Usage {
                    input: 100,
                    cached: 50,
                    output: 20,
                    cost_cents: 0.0,
                    window: 0
                }),
                Update::Summary("1m 1s · 2 turns".into()),
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
                Update::Conversation("g-1".into()),
                Update::Step("Read a.rs".into()),
                Update::Text("Hel".into()),
                Update::Text("lo".into()),
                Update::Usage(Usage {
                    input: 60,
                    cached: 40,
                    output: 30,
                    cost_cents: 0.0,
                    window: 0
                }),
                Update::Summary("900ms".into()),
            ]
        );
        let opencode = json_all(&[
            r#"{"type":"tool_use","timestamp":1,"sessionID":"ses_1","part":{"type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"ls"}}}}"#,
            r#"{"type":"step_finish","timestamp":2,"sessionID":"ses_1","part":{"type":"step-finish","tokens":{"input":500,"output":10,"reasoning":5,"cache":{"read":100,"write":0}},"cost":0.002}}"#,
            r#"{"type":"text","timestamp":3,"sessionID":"ses_1","part":{"type":"text","text":"Yes"}}"#,
            r#"{"type":"step_finish","timestamp":4,"sessionID":"ses_1","part":{"type":"step-finish","tokens":{"input":600,"output":20,"reasoning":0,"cache":{"read":200,"write":0}},"cost":0.003}}"#,
        ]);
        assert_eq!(opencode[0], Update::Conversation("ses_1".into()));
        assert_eq!(opencode[1], Update::Step("Ran ls".into()));
        assert_eq!(opencode[3], Update::Text("Yes".into()));
        let Update::Usage(last) = opencode[4] else {
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
                Update::Conversation("cp-1".into()),
                Update::Step("Read x.md".into()),
                Update::Text("Hi ".into()),
                Update::Text("there".into()),
                Update::Text("\n\n".into()),
                Update::Usage(Usage {
                    input: 900,
                    cached: 100,
                    output: 40,
                    cost_cents: 0.0,
                    window: 0
                }),
                Update::Error("Out of credits".into()),
            ]
        );
    }

    #[test]
    fn claude_format_reports_the_conversation_once() {
        let out = feed_all(&[
            r#"{"type":"system","subtype":"init","session_id":"T-1"}"#,
            r#"{"type":"result","is_error":false,"result":"ok","session_id":"T-1"}"#,
        ]);
        assert_eq!(out[0], Update::Conversation("T-1".into()));
        assert_eq!(
            out.iter()
                .filter(|u| matches!(u, Update::Conversation(_)))
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
