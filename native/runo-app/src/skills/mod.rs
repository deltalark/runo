//! Runo's built-in skills: small MCP tools every agent gets automatically, no matter
//! which CLI runs. They ride on the existing memory server stream (see `memory_mcp`),
//! so one wiring reaches Claude Code, Codex, Gemini, Cursor and the rest.
//!
//! A skill is a trait with a stable id, an MCP tool schema, a `call` that runs the tool,
//! and a settings slice (persisted in `AppSettings::skills`). The registry lives in
//! [`ALL`]; add a new file and one line here to ship another one. The first skill is
//! image generation (see `image`). PDF reading, web search and charts are the planned
//! follow-ups.
//!
//! API keys are kept in `skill_secrets.json` under the private data folder, written 0600
//! by [`crate::store::write_private`]. They never land in a log line.
//!
//! Cross-platform: HTTP calls go through `curl`, as everywhere else in Runo
//! (see `applog` and `coveserver`); image files are written with the same safe paths the
//! chat attachments viewer already handles.

pub mod image;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// What every built-in skill implements. A skill may expose more than one MCP tool.
pub trait BuiltinSkill: Sync {
    /// A stable id used in settings (`enabled` map) and the UI.
    fn id(&self) -> &'static str;
    /// The person-facing name shown in Settings.
    fn name(&self) -> &'static str;
    /// One sentence on what the skill does, shown next to the switch.
    fn description(&self) -> &'static str;
    /// MCP tool schemas as `tools/list` would return them. Called only when enabled.
    fn tools(&self) -> Vec<Value>;
    /// Whether this skill owns the tool named `name` (what [`BuiltinSkill::tools`] lists).
    fn owns(&self, tool: &str) -> bool {
        self.tools()
            .iter()
            .any(|t| t.get("name").and_then(Value::as_str) == Some(tool))
    }
    /// Carry out a tool call. `dir` is the session's folder, so results (like generated
    /// images) can land under its attachments and show up inline in the chat. The whole
    /// settings slice is passed because some skills (image generation) read their
    /// per-backend keys from there; others may ignore it.
    fn call(&self, dir: &str, settings: &SkillSettings, tool: &str, args: &Value) -> Value;
}

/// Every skill Runo ships. One line per new skill. Keep ids short and stable: they
/// appear in `settings.json` and in the Settings UI, and user memory may reference them.
pub fn all() -> &'static [&'static dyn BuiltinSkill] {
    &[&image::ImageGeneration]
}

/// The skill that handles `tool`, when any does. The lookup is linear; the list is tiny.
pub fn skill_for(tool: &str) -> Option<&'static dyn BuiltinSkill> {
    all().iter().copied().find(|s| s.owns(tool))
}

/// Settings for every built-in skill, saved under `AppSettings::skills`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillSettings {
    /// Which skills are turned on, by id. A missing entry is off.
    pub enabled: BTreeMap<String, bool>,
    pub image: image::ImageSettings,
}

impl SkillSettings {
    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.get(id).copied().unwrap_or(false)
    }
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        self.enabled.insert(id.to_string(), on);
    }
}

/// MCP tool schemas for every enabled skill, flattened; the server concatenates these
/// into its own `tools/list` reply.
pub fn enabled_tools(settings: &SkillSettings) -> Vec<Value> {
    all()
        .iter()
        .filter(|s| settings.is_enabled(s.id()))
        .flat_map(|s| s.tools())
        .collect()
}

/// Every tool name every built-in skill ever exposes, enabled or not. The agent
/// harness adds these to its no-ask list so a disabled skill still gets an in-band
/// "turned off" reply from the server, rather than a per-call permission prompt.
pub const ALL_TOOL_NAMES: &[&str] = &["generate_image"];

#[allow(dead_code)]
pub fn enabled_tool_names(settings: &SkillSettings) -> Vec<String> {
    enabled_tools(settings)
        .into_iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect()
}

/// Call an enabled skill's tool. Returns an MCP tool reply, or an "unknown tool" error
/// if nothing claims it or the owning skill is disabled.
pub fn call(dir: &str, settings: &SkillSettings, tool: &str, args: &Value) -> Value {
    match skill_for(tool) {
        Some(skill) if settings.is_enabled(skill.id()) => skill.call(dir, settings, tool, args),
        Some(_) => failure(format!(
            "The {tool} tool is turned off in Runo Settings > Skills."
        )),
        None => failure(format!("Unknown tool {tool}")),
    }
}

/// MCP success reply.
#[allow(dead_code)]
pub(crate) fn text(t: impl Into<String>) -> Value {
    serde_json::json!({ "content": [{ "type": "text", "text": t.into() }] })
}

/// MCP error reply (tool reports an error without crashing the server).
pub(crate) fn failure(t: impl Into<String>) -> Value {
    serde_json::json!({ "content": [{ "type": "text", "text": t.into() }], "isError": true })
}

/// An MCP reply that attaches an image as well as the summary text, so the chat image
/// viewer picks it up and the agent sees the path too.
pub(crate) fn image_reply(path: &std::path::Path, summary: &str) -> Value {
    let path_str = path.to_string_lossy();
    serde_json::json!({
        "content": [
            { "type": "text", "text": format!("{summary}\n\nSaved: {path_str}") },
            // MCP's resource shape, the way clients that know images (Claude Code, Cursor)
            // pick them up for display; ones that don't just see the text above.
            {
                "type": "resource",
                "resource": {
                    "uri": format!("file://{path_str}"),
                    "mimeType": "image/png"
                }
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn enabled_tools_follow_the_switch() {
        let mut s = SkillSettings::default();
        assert!(enabled_tools(&s).is_empty());
        s.set_enabled("image", true);
        let tools = enabled_tools(&s);
        assert!(tools.iter().any(|t| t["name"] == "generate_image"));
        assert_eq!(enabled_tool_names(&s), ["generate_image"]);
    }

    #[test]
    fn call_rejects_when_the_skill_is_off() {
        let s = SkillSettings::default();
        let reply = call("/tmp", &s, "generate_image", &serde_json::json!({}));
        assert_eq!(reply["isError"], true);
        assert!(reply["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("turned off"));
    }

    #[test]
    fn unknown_tool_is_reported() {
        let s = SkillSettings::default();
        let reply = call("/tmp", &s, "nope", &serde_json::json!({}));
        assert_eq!(reply["isError"], true);
        assert!(reply["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Unknown"));
    }
}
