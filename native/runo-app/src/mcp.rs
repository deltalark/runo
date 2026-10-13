//! MCP servers configured in every app on this computer, and the ones the user shares
//! with Runo's sessions.
//!
//! Each agent already loads its own servers. A shared server is handed to the other
//! agents per run where the CLI takes servers that way: Claude Code and Qwen Code through
//! `--mcp-config`, Codex through `-c mcp_servers.*`, Amp through `--mcp-config`, Copilot
//! through `--additional-mcp-config` and OpenCode through `OPENCODE_CONFIG_CONTENT`.
//! Cursor, Gemini, Droid and Grok only read their own config files, so they get none.

use crate::settings::McpInfo;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How to start or reach a server.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct McpSpec {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// The URL speaks the older SSE transport rather than streamable HTTP.
    pub sse: bool,
}

impl McpSpec {
    pub fn transport(&self) -> &'static str {
        if !self.command.is_empty() {
            "stdio"
        } else if !self.url.is_empty() {
            "http"
        } else {
            "unknown"
        }
    }

    pub fn detail(&self) -> String {
        if !self.command.is_empty() {
            self.command.clone()
        } else {
            self.url.clone()
        }
    }

    /// Claude Code's `mcpServers` entry, which Qwen Code reads too.
    pub fn claude_json(&self) -> Value {
        let mut obj = Map::new();
        if !self.command.is_empty() {
            obj.insert("command".into(), self.command.clone().into());
            obj.insert("args".into(), self.args.clone().into());
            if !self.env.is_empty() {
                obj.insert("env".into(), pairs(&self.env));
            }
        } else {
            obj.insert("type".into(), if self.sse { "sse" } else { "http" }.into());
            obj.insert("url".into(), self.url.clone().into());
            if !self.headers.is_empty() {
                obj.insert("headers".into(), pairs(&self.headers));
            }
        }
        Value::Object(obj)
    }

    /// Amp's `amp.mcpServers` entry.
    pub fn amp_json(&self) -> Value {
        let mut obj = Map::new();
        if !self.command.is_empty() {
            obj.insert("command".into(), self.command.clone().into());
            obj.insert("args".into(), self.args.clone().into());
            if !self.env.is_empty() {
                obj.insert("env".into(), pairs(&self.env));
            }
        } else {
            obj.insert("url".into(), self.url.clone().into());
            if !self.headers.is_empty() {
                obj.insert("headers".into(), pairs(&self.headers));
            }
        }
        Value::Object(obj)
    }

    /// Copilot's `mcpServers` entry; it lists the tools the agent may use.
    pub fn copilot_json(&self) -> Value {
        let mut obj = Map::new();
        if !self.command.is_empty() {
            obj.insert("type".into(), "local".into());
            obj.insert("command".into(), self.command.clone().into());
            obj.insert("args".into(), self.args.clone().into());
            obj.insert("env".into(), pairs(&self.env));
        } else {
            obj.insert("type".into(), if self.sse { "sse" } else { "http" }.into());
            obj.insert("url".into(), self.url.clone().into());
            obj.insert("headers".into(), pairs(&self.headers));
        }
        obj.insert("tools".into(), serde_json::json!(["*"]));
        Value::Object(obj)
    }

    /// OpenCode's `mcp` entry: the command and its arguments in one list.
    pub fn opencode_json(&self) -> Value {
        let mut obj = Map::new();
        if !self.command.is_empty() {
            obj.insert("type".into(), "local".into());
            let mut command = vec![self.command.clone()];
            command.extend(self.args.iter().cloned());
            obj.insert("command".into(), command.into());
            if !self.env.is_empty() {
                obj.insert("environment".into(), pairs(&self.env));
            }
        } else {
            obj.insert("type".into(), "remote".into());
            obj.insert("url".into(), self.url.clone().into());
            if !self.headers.is_empty() {
                obj.insert("headers".into(), pairs(&self.headers));
            }
        }
        Value::Object(obj)
    }

    /// Codex `-c` overrides for `mcp_servers.<name>`.
    pub fn codex_overrides(&self, name: &str) -> Vec<String> {
        let quote = |s: &str| Value::String(s.to_string()).to_string();
        let mut out = Vec::new();
        if !self.command.is_empty() {
            out.push(format!(
                "mcp_servers.{name}.command={}",
                quote(&self.command)
            ));
            let args: Vec<String> = self.args.iter().map(|a| quote(a)).collect();
            out.push(format!("mcp_servers.{name}.args=[{}]", args.join(", ")));
            if !self.env.is_empty() {
                let env: Vec<String> = self
                    .env
                    .iter()
                    .map(|(k, v)| format!("{} = {}", quote(k), quote(v)))
                    .collect();
                out.push(format!("mcp_servers.{name}.env={{ {} }}", env.join(", ")));
            }
        } else {
            out.push(format!("mcp_servers.{name}.url={}", quote(&self.url)));
            if !self.headers.is_empty() {
                let headers: Vec<String> = self
                    .headers
                    .iter()
                    .map(|(k, v)| format!("{} = {}", quote(k), quote(v)))
                    .collect();
                out.push(format!(
                    "mcp_servers.{name}.http_headers={{ {} }}",
                    headers.join(", ")
                ));
            }
        }
        out
    }
}

fn pairs(list: &[(String, String)]) -> Value {
    Value::Object(
        list.iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect(),
    )
}

/// The key a server is shared under in settings: `<app>/<name>`.
pub fn key(info: &McpInfo) -> String {
    format!("{}/{}", info.provider, info.name)
}

/// A name every CLI accepts as a server id and tool prefix.
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

static FOUND: Mutex<Vec<McpInfo>> = Mutex::new(Vec::new());
static SHARED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Remember the servers found on this computer.
pub fn set_found(found: &[McpInfo]) {
    if let Ok(mut all) = FOUND.lock() {
        *all = found.to_vec();
    }
}

/// Remember which servers the user shares with Runo's sessions.
pub fn set_shared(keys: &[String]) {
    if let Ok(mut shared) = SHARED.lock() {
        *shared = keys.to_vec();
    }
}

/// Shared servers to hand to `agent`, leaving out the ones that agent already loads.
/// Servers in Runo's own list always go to every agent.
fn shared_for(agent: &str) -> Vec<(String, McpSpec)> {
    let keys = SHARED.lock().map(|k| k.clone()).unwrap_or_default();
    let mut all = FOUND.lock().map(|f| f.clone()).unwrap_or_default();
    // Runo's own come first, so they win a clash of names.
    all.sort_by_key(|m| m.provider != OWN);
    let mut seen = std::collections::HashSet::new();
    all.into_iter()
        .filter(|m| m.enabled && (m.provider == OWN || keys.contains(&key(m))))
        .filter(|m| m.spec.transport() != "unknown" && m.provider != "Project")
        // Codex only speaks streamable HTTP, not SSE.
        .filter(|m| !(agent == "codex" && m.spec.sse))
        .filter(|m| match agent {
            "claude" => m.provider != "Claude Code",
            "codex" => m.provider != "Codex",
            "opencode" => m.provider != "OpenCode",
            "qwen" => m.provider != "Qwen Code",
            "amp" => m.provider != "Amp",
            _ => true,
        })
        .map(|m| (safe_name(&m.name), m.spec))
        .filter(|(name, _)| seen.insert(name.clone()))
        .collect()
}

/// Shared servers for one run of `agent`. A run on a connection or in a VM can't start
/// programs on this machine, so only servers reached by URL go along there.
pub fn shared_servers(agent: &str, local: bool) -> Vec<(String, McpSpec)> {
    shared_for(agent)
        .into_iter()
        .filter(|(_, spec)| local || spec.command.is_empty())
        .collect()
}

/// The name Runo's own servers are listed under.
pub const OWN: &str = "Runo";

/// Runo's own list of servers, in Claude Code's `mcpServers` form.
pub fn own_path() -> PathBuf {
    crate::store::data_dir().join("mcp.json")
}

fn edit_own(path: &Path, change: impl FnOnce(&mut Map<String, Value>)) -> std::io::Result<()> {
    let mut value = read_json(path).unwrap_or_else(|| Value::Object(Map::new()));
    if !value.is_object() {
        value = Value::Object(Map::new());
    }
    let root = value.as_object_mut().expect("made an object above");
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    if !servers.is_object() {
        *servers = Value::Object(Map::new());
    }
    change(servers.as_object_mut().expect("made an object above"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    std::fs::write(path, text + "\n")
}

/// Add a blank server to Runo's list for the person to fill in; returns its name.
pub fn add_own(path: &Path) -> std::io::Result<String> {
    let mut name = String::new();
    edit_own(path, |servers| {
        name = (1..)
            .map(|n| {
                if n == 1 {
                    "new-server".to_string()
                } else {
                    format!("new-server-{n}")
                }
            })
            .find(|n| !servers.contains_key(n))
            .expect("some name is free");
        servers.insert(
            name.clone(),
            serde_json::json!({ "command": "", "args": [], "env": {} }),
        );
    })?;
    Ok(name)
}

/// Remove a server from Runo's list.
pub fn remove_own(path: &Path, name: &str) -> std::io::Result<()> {
    edit_own(path, |servers| {
        servers.remove(name);
    })
}

/// Remove `//` and `/* */` comments and trailing commas, for VS Code style JSON.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    // Trailing commas before a closing bracket.
    let mut cleaned = String::with_capacity(out.len());
    let bytes: Vec<char> = out.chars().collect();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            cleaned.push(c);
            if c == '\\' && i + 1 < bytes.len() {
                cleaned.push(bytes[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            cleaned.push(c);
        } else if c == ',' {
            let next = bytes[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                cleaned.push(c);
            }
        } else {
            cleaned.push(c);
        }
        i += 1;
    }
    cleaned
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text)
        .ok()
        .or_else(|| serde_json::from_str(&strip_jsonc(&text)).ok())
}

fn string_pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The common `{command, args, env}` / `{url, headers}` shape.
pub fn spec_from_json(cfg: &Value) -> McpSpec {
    let url = ["url", "serverUrl", "httpUrl"]
        .iter()
        .find_map(|k| cfg.get(*k).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let sse = cfg.get("type").and_then(Value::as_str) == Some("sse")
        || cfg.get("transport").and_then(Value::as_str) == Some("sse")
        || url.trim_end_matches('/').ends_with("/sse");
    McpSpec {
        sse,
        command: cfg
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        args: strings(cfg.get("args")),
        env: string_pairs(cfg.get("env")),
        url,
        headers: string_pairs(cfg.get("headers")),
    }
}

/// OpenCode's `mcp` entries: `{type: "local", command: [...], environment}` or
/// `{type: "remote", url, headers}`.
fn spec_from_opencode(cfg: &Value) -> McpSpec {
    let command = strings(cfg.get("command"));
    let (head, rest) = command
        .split_first()
        .map_or((String::new(), Vec::new()), |(h, r)| {
            (h.clone(), r.to_vec())
        });
    McpSpec {
        command: head,
        args: rest,
        env: string_pairs(cfg.get("environment")),
        url: cfg
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        headers: string_pairs(cfg.get("headers")),
        sse: false,
    }
}

/// The folder apps keep their settings in: Application Support, ~/.config or %APPDATA%.
fn app_config(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        home.join(".config")
    }
}

/// Config files of other apps, with the key their servers live under.
pub fn other_sources(home: &Path) -> Vec<(&'static str, PathBuf, &'static str)> {
    let conf = app_config(home);
    vec![
        (
            "Claude Desktop",
            conf.join("Claude/claude_desktop_config.json"),
            "mcpServers",
        ),
        ("Cursor", home.join(".cursor/mcp.json"), "mcpServers"),
        (
            "Windsurf",
            home.join(".codeium/windsurf/mcp_config.json"),
            "mcpServers",
        ),
        ("VS Code", conf.join("Code/User/mcp.json"), "servers"),
        (
            "Cline",
            conf.join(
                "Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
            ),
            "mcpServers",
        ),
        (
            "Gemini CLI",
            home.join(".gemini/settings.json"),
            "mcpServers",
        ),
        ("Qwen Code", home.join(".qwen/settings.json"), "mcpServers"),
        (
            "Amp",
            home.join(".config/amp/settings.json"),
            "amp.mcpServers",
        ),
        ("LM Studio", home.join(".lmstudio/mcp.json"), "mcpServers"),
        (
            "OpenCode",
            home.join(".config/opencode/opencode.json"),
            "mcp",
        ),
    ]
}

/// Servers in one JSON config file.
pub fn scan_json(provider: &'static str, path: &Path, field: &str, out: &mut Vec<McpInfo>) {
    let Some(value) = read_json(path) else {
        return;
    };
    let Some(servers) = value.get(field).and_then(Value::as_object) else {
        return;
    };
    for (name, cfg) in servers {
        let spec = if provider == "OpenCode" {
            spec_from_opencode(cfg)
        } else {
            spec_from_json(cfg)
        };
        let enabled = !(cfg.get("disabled").and_then(Value::as_bool) == Some(true)
            || cfg.get("enabled").and_then(Value::as_bool) == Some(false));
        out.push(McpInfo {
            name: name.clone(),
            provider,
            transport: spec.transport(),
            enabled,
            detail: spec.detail(),
            path: path.to_path_buf(),
            spec,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn info(provider: &'static str, name: &str, spec: McpSpec) -> McpInfo {
        McpInfo {
            name: name.into(),
            provider,
            transport: spec.transport(),
            enabled: true,
            detail: spec.detail(),
            path: PathBuf::new(),
            spec,
        }
    }

    #[test]
    fn reads_vscode_style_json_and_opencode() {
        let dir = std::env::temp_dir().join(format!("runo-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let vscode = dir.join("mcp.json");
        std::fs::write(
            &vscode,
            "{\n  // GitHub\n  \"servers\": {\n    \"github\": { \"url\": \"https://api.githubcopilot.com/mcp/\", },\n    /* local */ \"fs\": { \"command\": \"npx\", \"args\": [\"-y\", \"fs-mcp\"], \"env\": {\"A\": \"b//c\"} },\n  },\n}\n",
        )
        .unwrap();
        let mut out = Vec::new();
        scan_json("VS Code", &vscode, "servers", &mut out);
        assert_eq!(out.len(), 2);
        let fs = out.iter().find(|m| m.name == "fs").unwrap();
        assert_eq!(fs.spec.args, ["-y", "fs-mcp"]);
        assert_eq!(fs.spec.env, [("A".to_string(), "b//c".to_string())]);
        assert_eq!(
            out.iter().find(|m| m.name == "github").unwrap().transport,
            "http"
        );

        let opencode = dir.join("opencode.json");
        std::fs::write(
            &opencode,
            r#"{"mcp":{"local":{"type":"local","command":["bun","x","srv"],"enabled":false}}}"#,
        )
        .unwrap();
        let mut out = Vec::new();
        scan_json("OpenCode", &opencode, "mcp", &mut out);
        assert_eq!(out[0].spec.command, "bun");
        assert_eq!(out[0].spec.args, ["x", "srv"]);
        assert!(!out[0].enabled);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shares_servers_with_the_agents_that_lack_them() {
        let stdio = McpSpec {
            command: "uvx".into(),
            args: vec!["blender-mcp".into()],
            ..Default::default()
        };
        let http = McpSpec {
            url: "https://mcp.figma.com/mcp".into(),
            ..Default::default()
        };
        set_found(&[
            info("Codex", "blender lab", stdio.clone()),
            info("Cursor", "figma", http),
            info("Claude Code", "files", stdio),
        ]);
        set_shared(&[
            "Codex/blender lab".into(),
            "Cursor/figma".into(),
            "Claude Code/files".into(),
        ]);
        let names = |agent: &str, local: bool| -> Vec<String> {
            shared_servers(agent, local)
                .into_iter()
                .map(|(n, _)| n)
                .collect()
        };
        assert_eq!(names("claude", true), ["blender_lab", "figma"]);
        let codex: Vec<String> = shared_servers("codex", true)
            .into_iter()
            .flat_map(|(name, spec)| spec.codex_overrides(&name))
            .collect();
        assert!(codex.contains(&"mcp_servers.files.command=\"uvx\"".to_string()));
        assert!(codex.contains(&"mcp_servers.figma.url=\"https://mcp.figma.com/mcp\"".to_string()));
        assert!(!codex.iter().any(|c| c.contains("blender_lab")));
        // A run elsewhere can't start this machine's programs.
        assert_eq!(names("claude", false), ["figma"]);
        let opencode = shared_servers("opencode", true);
        assert_eq!(
            opencode[0].1.opencode_json(),
            serde_json::json!({"type": "local", "command": ["uvx", "blender-mcp"]})
        );
        assert_eq!(
            opencode[1].1.copilot_json()["tools"],
            serde_json::json!(["*"])
        );
        set_shared(&[]);
        assert!(names("claude", true).is_empty());

        // Runo's own servers need no switch, and win over a same-named one.
        let own = McpSpec {
            command: "runo-tools".into(),
            ..Default::default()
        };
        set_found(&[
            info(
                "Cursor",
                "figma",
                McpSpec {
                    url: "https://mcp.figma.com/mcp".into(),
                    ..Default::default()
                },
            ),
            info(OWN, "figma", own),
        ]);
        set_shared(&["Cursor/figma".into()]);
        let claude = shared_servers("claude", true);
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].1.command, "runo-tools");
        set_found(&[]);
        set_shared(&[]);
    }

    #[test]
    fn runo_keeps_its_own_list_of_servers() {
        let dir = std::env::temp_dir().join(format!("runo-own-mcp-{}", std::process::id()));
        let path = dir.join("mcp.json");
        assert_eq!(add_own(&path).unwrap(), "new-server");
        assert_eq!(add_own(&path).unwrap(), "new-server-2");
        remove_own(&path, "new-server").unwrap();
        let mut out = Vec::new();
        scan_json(OWN, &path, "mcpServers", &mut out);
        let names: Vec<&str> = out.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["new-server-2"]);
        assert_eq!(out[0].transport, "unknown", "blank until filled in");
        let _ = std::fs::remove_dir_all(dir);
    }
}
