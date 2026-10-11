//! Turns Claude Code's `--output-format stream-json` lines into conversation updates:
//! reply text as it streams, a short label for each tool step, and a run summary.

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
}

impl ClaudeStream {
    pub fn new(cwd: &str) -> Self {
        Self {
            cwd: cwd.trim_end_matches('/').to_string(),
            streamed_text: false,
            any_text: false,
            usage: Usage::default(),
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
        match event["type"].as_str().unwrap_or("") {
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

    fn rel(&self, path: &str) -> String {
        path.strip_prefix(&self.cwd)
            .map(|p| p.trim_start_matches('/'))
            .filter(|p| !p.is_empty())
            .unwrap_or(path)
            .to_string()
    }

    fn step_label(&self, tool: &str, input: &Value) -> String {
        let s = |key: &str| input[key].as_str().unwrap_or("").to_string();
        let path = || self.rel(&s("file_path")).replace('`', "'");
        let short = |text: String, max: usize| {
            let line = text.lines().next().unwrap_or("").to_string();
            if line.chars().count() > max || text.contains('\n') {
                format!("{}…", line.chars().take(max).collect::<String>())
            } else {
                line
            }
        };
        match tool {
            "Read" => format!("Read {}", path()),
            "Edit" | "MultiEdit" => format!("Edited {}", path()),
            "Write" => format!("Wrote {}", path()),
            "NotebookEdit" => format!("Edited {}", self.rel(&s("notebook_path"))),
            "Bash" => format!("Ran {}", short(s("command"), 70)),
            "Grep" => format!("Searched for {}", short(s("pattern"), 50)),
            "Glob" => format!("Listed {}", short(s("pattern"), 50)),
            "WebSearch" => format!("Searched the web for {}", short(s("query"), 50)),
            "WebFetch" => format!("Fetched {}", short(s("url"), 60)),
            "Task" | "Agent" => format!("Ran a subagent: {}", short(s("description"), 60)),
            "TodoWrite" => "Updated the plan".to_string(),
            "ExitPlanMode" => "Proposed a plan".to_string(),
            "AskUserQuestion" => "Asked a question".to_string(),
            other => format!("Used {other}"),
        }
    }
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
