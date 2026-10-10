//! Discovers what each installed agent CLI supports (version, models, permission modes,
//! reasoning levels, service tiers…) by asking the CLI itself, so nothing is hard-coded
//! beyond how to parse each tool's own help output.

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Default, PartialEq)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub desc: String,
}

#[derive(Clone, Default)]
pub struct ModelInfo {
    pub choice: Choice,
    /// Reasoning / effort levels this model accepts (empty: use the provider-wide list).
    pub efforts: Vec<Choice>,
    pub tiers: Vec<Choice>,
}

#[derive(Clone, Default)]
pub struct Caps {
    pub installed: bool,
    pub version: String,
    pub models: Vec<ModelInfo>,
    /// Claude: permission modes. Codex: sandbox modes. OpenCode: permission styles.
    pub modes: Vec<Choice>,
    /// Provider-wide effort levels (Claude).
    pub efforts: Vec<Choice>,
    /// Codex approval policies.
    pub approvals: Vec<Choice>,
    /// OpenCode named agents.
    pub agents: Vec<Choice>,
    pub detected: bool,
}

/// One dropdown shown in the composer.
#[derive(Clone)]
pub struct Control {
    pub id: &'static str,
    pub label: String,
    pub choices: Vec<Choice>,
}

static CAPS: OnceLock<RwLock<HashMap<String, Caps>>> = OnceLock::new();
static UPDATED: AtomicBool = AtomicBool::new(false);

fn store() -> &'static RwLock<HashMap<String, Caps>> {
    CAPS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn get(provider: &str) -> Caps {
    store()
        .read()
        .ok()
        .and_then(|m| m.get(provider).cloned())
        .unwrap_or_default()
}

/// True once after discovery finished, so the UI can repaint.
pub fn take_updated() -> bool {
    UPDATED.swap(false, Ordering::Relaxed)
}

/// Discover every provider on a background thread.
pub fn refresh() {
    std::thread::spawn(|| {
        for provider in crate::agent::PROVIDERS.iter() {
            let caps = match provider.id {
                "claude" => claude(),
                "codex" => codex(),
                _ => opencode(),
            };
            if let Ok(mut map) = store().write() {
                map.insert(provider.id.to_string(), caps);
            }
            UPDATED.store(true, Ordering::Relaxed);
        }
    });
}

fn run(bin: &str, args: &[&str]) -> String {
    Command::new(bin)
        .args(args)
        .output()
        .map(|o| {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            text
        })
        .unwrap_or_default()
}

fn pretty(value: &str) -> String {
    let mut out = String::new();
    for (i, c) in value.chars().enumerate() {
        match c {
            '-' | '_' => out.push(' '),
            c if c.is_uppercase() && i > 0 => {
                out.push(' ');
                out.push(c.to_ascii_lowercase());
            }
            c => out.push(c),
        }
    }
    let mut chars = out.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn choice(value: &str) -> Choice {
    Choice {
        value: value.to_string(),
        label: pretty(value),
        desc: String::new(),
    }
}

/// The text of one option in `--help`: from its flag line up to the next flag line.
fn flag_block(help: &str, flag: &str) -> String {
    let mut block = String::new();
    let mut inside = false;
    for line in help.lines() {
        let trimmed = line.trim_start();
        let is_flag_line = trimmed.starts_with('-') && line.starts_with("  ");
        if inside && is_flag_line {
            break;
        }
        if !inside && is_flag_line && line.contains(flag) {
            inside = true;
        }
        if inside {
            block.push_str(line);
            block.push('\n');
        }
    }
    block
}

fn between(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)? + 1;
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

fn quoted(text: &str, quote: char) -> Vec<String> {
    text.split(quote)
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect()
}

fn not_installed() -> Caps {
    Caps {
        detected: true,
        ..Default::default()
    }
}

fn claude() -> Caps {
    let version = run("claude", &["--version"]);
    if version.trim().is_empty() {
        return not_installed();
    }
    parse_claude(&version, &run("claude", &["--help"]))
}

/// Claude Code: modes, efforts and model aliases from `claude --help`.
fn parse_claude(version: &str, help: &str) -> Caps {
    let mode_block = flag_block(help, "--permission-mode").replace('\n', " ");
    let modes = quoted(mode_block.split("choices:").nth(1).unwrap_or(""), '"')
        .iter()
        .map(|v| choice(v))
        .collect();
    let effort_block = flag_block(help, "--effort").replace('\n', " ");
    let efforts = between(&effort_block, '(', ')')
        .map(|list| list.split(',').map(|v| choice(v.trim())).collect())
        .unwrap_or_default();
    let model_block = flag_block(help, "--model").replace('\n', " ");
    let models = quoted(&model_block, '\'')
        .into_iter()
        .filter(|v| !v.contains(' '))
        .map(|alias| ModelInfo {
            choice: choice(&alias),
            ..Default::default()
        })
        .collect();
    Caps {
        installed: true,
        version: version.split_whitespace().next().unwrap_or("").to_string(),
        models,
        modes,
        efforts,
        detected: true,
        ..Default::default()
    }
}

fn codex() -> Caps {
    let version = run("codex", &["--version"]);
    if version.trim().is_empty() {
        return not_installed();
    }
    // Models, reasoning levels and service tiers come from Codex's own cache for this account.
    let cache = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| {
            std::path::PathBuf::from(h)
                .join(".codex")
                .join("models_cache.json")
        })
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    parse_codex(
        &version,
        &run("codex", &["exec", "--help"]),
        &run("codex", &["--help"]),
        cache.as_ref(),
    )
}

/// Codex: sandbox modes from `codex exec --help`, approval policies from `codex --help`,
/// and models from `~/.codex/models_cache.json`.
fn parse_codex(
    version: &str,
    exec_help: &str,
    top_help: &str,
    cache: Option<&serde_json::Value>,
) -> Caps {
    let exec_help = exec_help.replace("\r", "");
    let sandbox_block = flag_block(&exec_help, "--sandbox").replace('\n', " ");
    let modes = between(
        sandbox_block
            .split("possible values:")
            .nth(1)
            .map(|s| format!("[{s}"))
            .as_deref()
            .unwrap_or(""),
        '[',
        ']',
    )
    .map(|list| list.split(',').map(|v| choice(v.trim())).collect())
    .unwrap_or_default();

    let mut approvals = Vec::new();
    let mut inside = false;
    for line in top_help.lines() {
        if line.contains("--ask-for-approval") {
            inside = true;
            continue;
        }
        if inside {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("- ") {
                let (name, desc) = rest.split_once(':').unwrap_or((rest, ""));
                approvals.push(Choice {
                    value: name.trim().to_string(),
                    label: pretty(name.trim()),
                    desc: desc.trim().to_string(),
                });
            } else if t.starts_with('-') && !t.starts_with("- ") {
                break;
            }
        }
    }

    let mut models: Vec<(i64, ModelInfo)> = Vec::new();
    if let Some(list) = cache
        .and_then(|v| v.get("models"))
        .and_then(|m| m.as_array())
    {
        for m in list {
            if m.get("visibility").and_then(|v| v.as_str()) != Some("list") {
                continue;
            }
            let Some(slug) = m.get("slug").and_then(|v| v.as_str()) else {
                continue;
            };
            let name = m
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(slug);
            let desc = m.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let pick = |key: &str, id_key: &str, name_key: &str| -> Vec<Choice> {
                m.get(key)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| {
                                let id = e.get(id_key)?.as_str()?;
                                Some(Choice {
                                    value: id.to_string(),
                                    label: e
                                        .get(name_key)
                                        .and_then(|n| n.as_str())
                                        .map(str::to_string)
                                        .unwrap_or_else(|| pretty(id)),
                                    desc: e
                                        .get("description")
                                        .and_then(|d| d.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            models.push((
                m.get("priority").and_then(|p| p.as_i64()).unwrap_or(999),
                ModelInfo {
                    choice: Choice {
                        value: slug.to_string(),
                        label: name.to_string(),
                        desc: desc.to_string(),
                    },
                    efforts: pick("supported_reasoning_levels", "effort", "effort"),
                    tiers: pick("service_tiers", "id", "name"),
                },
            ));
        }
    }
    models.sort_by_key(|(p, _)| *p);
    Caps {
        installed: true,
        version: version
            .trim()
            .trim_start_matches("codex-cli")
            .trim()
            .to_string(),
        models: models.into_iter().map(|(_, m)| m).collect(),
        modes,
        approvals,
        detected: true,
        ..Default::default()
    }
}

fn opencode() -> Caps {
    let version = run("opencode", &["--version"]);
    if version.trim().is_empty() {
        return not_installed();
    }
    parse_opencode(
        &version,
        &run("opencode", &["models"]),
        &run("opencode", &["agent", "list"]),
        &run("opencode", &["run", "--help"]),
    )
}

/// OpenCode: models from `opencode models`, agents from `opencode agent list`, and whether
/// `opencode run` accepts `--auto`.
fn parse_opencode(version: &str, models: &str, agents: &str, run_help: &str) -> Caps {
    let models = models
        .lines()
        .map(str::trim)
        .filter(|l| l.contains('/') && !l.contains(' '))
        .map(|l| ModelInfo {
            choice: Choice {
                value: l.to_string(),
                label: l.to_string(),
                desc: String::new(),
            },
            ..Default::default()
        })
        .collect();
    // `opencode agent list` prints `name (primary|subagent)` followed by a JSON permission dump.
    let agents = agents
        .lines()
        .filter(|l| {
            !l.starts_with(' ') && !l.is_empty() && !l.starts_with('[') && !l.starts_with(']')
        })
        .filter(|l| {
            !["compaction", "summary", "title"]
                .iter()
                .any(|hidden| l.starts_with(hidden))
        })
        .filter_map(|l| {
            let (name, kind) = l.split_once(" (")?;
            Some(Choice {
                value: name.to_string(),
                label: pretty(name),
                desc: kind.trim_end_matches(')').to_string(),
            })
        })
        .collect();
    let mut modes = Vec::new();
    if run_help.contains("--auto") {
        modes.push(Choice {
            value: "auto".into(),
            label: "Auto-approve".into(),
            desc: "Approve permissions that are not explicitly denied".into(),
        });
    }
    Caps {
        installed: true,
        version: version.trim().to_string(),
        models,
        modes,
        agents,
        detected: true,
        ..Default::default()
    }
}

fn with_default(label: &str, mut rest: Vec<Choice>) -> Vec<Choice> {
    rest.insert(
        0,
        Choice {
            value: String::new(),
            label: label.to_string(),
            desc: String::new(),
        },
    );
    rest
}

/// The composer dropdowns for a provider, given the session's current selections.
pub fn controls(provider: &str, sel: &HashMap<String, String>) -> Vec<Control> {
    controls_for(provider, &get(provider), sel)
}

fn controls_for(provider: &str, caps: &Caps, sel: &HashMap<String, String>) -> Vec<Control> {
    let value = |id: &str| sel.get(id).cloned().unwrap_or_default();
    let mut out = Vec::new();
    let model_choices: Vec<Choice> = caps.models.iter().map(|m| m.choice.clone()).collect();
    if !model_choices.is_empty() {
        out.push(Control {
            id: "model",
            label: "Model".into(),
            choices: with_default("Default", model_choices),
        });
    }
    // Effort / tier follow the selected model, or the first listed one when none is chosen.
    let current = caps
        .models
        .iter()
        .find(|m| m.choice.value == value("model"))
        .or_else(|| caps.models.first());
    let efforts = match current {
        Some(m) if !m.efforts.is_empty() => m.efforts.clone(),
        _ => caps.efforts.clone(),
    };
    if !efforts.is_empty() {
        let label = if provider == "codex" {
            "Reasoning"
        } else {
            "Effort"
        };
        out.push(Control {
            id: "effort",
            label: label.into(),
            choices: with_default("Default", efforts),
        });
    }
    if let Some(m) = current {
        if !m.tiers.is_empty() {
            out.push(Control {
                id: "tier",
                label: "Speed".into(),
                choices: with_default("Standard", m.tiers.clone()),
            });
        }
    }
    if !caps.modes.is_empty() {
        let label = match provider {
            "codex" => "Sandbox",
            "opencode" => "Permissions",
            _ => "Permission",
        };
        out.push(Control {
            id: "mode",
            label: label.into(),
            choices: with_default("Default", caps.modes.clone()),
        });
    }
    if !caps.approvals.is_empty() {
        out.push(Control {
            id: "approval",
            label: "Approval".into(),
            choices: with_default("Default", caps.approvals.clone()),
        });
    }
    if !caps.agents.is_empty() {
        out.push(Control {
            id: "agent",
            label: "Agent".into(),
            choices: with_default("Default", caps.agents.clone()),
        });
    }
    out
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    fn values(choices: &[Choice]) -> Vec<&str> {
        choices.iter().map(|c| c.value.as_str()).collect()
    }

    // Trimmed from `claude --help` 2.1.x; wrapping and apostrophes are as the CLI prints them.
    const CLAUDE_HELP: &str = "Options:
  --effort <level>                      Effort level for the current session
                                        (low, medium, high, xhigh, max)
  --fallback-model <model>              Enable automatic fallback
  --model <model>                       Model for the current session. Provide
                                        an alias for the latest model (e.g.
                                        'fable', 'opus', or 'sonnet') or a
                                        model's full name.
  --permission-mode <mode>              Permission mode to use for the session
                                        (choices: \"acceptEdits\", \"auto\",
                                        \"bypassPermissions\", \"manual\",
                                        \"dontAsk\", \"plan\")
  -r, --resume [value]                  Resume a conversation by session ID
";

    #[test]
    fn parses_claude_help() {
        let caps = parse_claude("2.1.296 (Claude Code)\n", CLAUDE_HELP);
        assert!(caps.installed && caps.detected);
        assert_eq!(caps.version, "2.1.296");
        assert_eq!(
            values(&caps.modes),
            [
                "acceptEdits",
                "auto",
                "bypassPermissions",
                "manual",
                "dontAsk",
                "plan"
            ]
        );
        assert_eq!(caps.modes[0].label, "Accept edits");
        assert_eq!(
            values(&caps.efforts),
            ["low", "medium", "high", "xhigh", "max"]
        );
        let models: Vec<&str> = caps
            .models
            .iter()
            .map(|m| m.choice.value.as_str())
            .collect();
        assert_eq!(models, ["fable", "opus", "sonnet"]);
    }

    #[test]
    fn parses_codex_help_and_model_cache() {
        let exec_help = "Usage: codex exec [OPTIONS] [PROMPT]\r
  -m, --model <MODEL>\r
          Model the agent should use\r
  -s, --sandbox <SANDBOX_MODE>\r
          Select the sandbox policy to use when executing model-generated shell commands\r
\r
          [possible values: read-only, workspace-write, danger-full-access]\r
  -p, --profile <CONFIG_PROFILE>\r
";
        let top_help = "  -a, --ask-for-approval <APPROVAL_POLICY>
          Configure when the model requires human approval before executing a command

          Possible values:
          - untrusted:  Only run trusted commands without asking
          - on-request: The model decides when to ask the user for approval
          - never:      Never ask for user approval

  -c, --config <key=value>
          - not: an approval
";
        let cache: serde_json::Value = serde_json::from_str(
            r#"{"models":[
              {"slug":"gpt-mini","display_name":"GPT mini","priority":5,"visibility":"list",
               "supported_reasoning_levels":[{"effort":"low"},{"effort":"high","description":"Deep"}]},
              {"slug":"hidden","visibility":"hide","priority":0},
              {"slug":"gpt-big","priority":1,"visibility":"list","description":"Best",
               "service_tiers":[{"id":"priority","name":"Fast"}]},
              {"visibility":"list"}
            ]}"#,
        )
        .unwrap();
        let caps = parse_codex("codex-cli 0.50.1\n", exec_help, top_help, Some(&cache));
        assert_eq!(caps.version, "0.50.1");
        assert_eq!(
            values(&caps.modes),
            ["read-only", "workspace-write", "danger-full-access"]
        );
        assert_eq!(
            values(&caps.approvals),
            ["untrusted", "on-request", "never"]
        );
        assert_eq!(caps.approvals[2].desc, "Never ask for user approval");
        let models: Vec<&str> = caps
            .models
            .iter()
            .map(|m| m.choice.value.as_str())
            .collect();
        assert_eq!(
            models,
            ["gpt-big", "gpt-mini"],
            "sorted by priority, hidden ones skipped"
        );
        assert_eq!(caps.models[0].choice.label, "gpt-big");
        assert_eq!(caps.models[0].tiers[0].label, "Fast");
        assert_eq!(values(&caps.models[1].efforts), ["low", "high"]);
        assert_eq!(caps.models[1].efforts[1].desc, "Deep");

        let bare = parse_codex("codex-cli 1.0", "", "", None);
        assert!(bare.installed && bare.models.is_empty() && bare.modes.is_empty());
    }

    #[test]
    fn parses_opencode_output() {
        let models = "opencode/big-pickle\nanthropic/claude-x\nnot a model\n  \n";
        let agents = "build (primary)\n  {\n    \"edit\": \"allow\"\n  }\nplan (primary)\ngeneral (subagent)\ntitle (primary)\n[\n]\n";
        let caps = parse_opencode("1.2.3\n", models, agents, "  --auto  approve\n");
        assert_eq!(caps.version, "1.2.3");
        assert_eq!(caps.models.len(), 2);
        assert_eq!(values(&caps.agents), ["build", "plan", "general"]);
        assert_eq!(caps.agents[2].desc, "subagent");
        assert_eq!(values(&caps.modes), ["auto"]);
        assert!(parse_opencode("1", "", "", "").modes.is_empty());
    }

    #[test]
    fn composer_controls_follow_the_selected_model() {
        let level = |v: &str| Choice {
            value: v.into(),
            label: pretty(v),
            desc: String::new(),
        };
        let caps = Caps {
            installed: true,
            detected: true,
            models: vec![
                ModelInfo {
                    choice: level("a"),
                    efforts: vec![level("low")],
                    tiers: vec![],
                },
                ModelInfo {
                    choice: level("b"),
                    efforts: vec![level("high"), level("max")],
                    tiers: vec![level("priority")],
                },
            ],
            modes: vec![level("read-only")],
            approvals: vec![level("never")],
            ..Default::default()
        };
        let ids = |c: &[Control]| c.iter().map(|c| c.id).collect::<Vec<_>>();
        let none = controls_for("codex", &caps, &HashMap::new());
        assert_eq!(ids(&none), ["model", "effort", "mode", "approval"]);
        assert_eq!(none[1].label, "Reasoning");
        assert_eq!(
            values(&none[1].choices),
            ["", "low"],
            "first model's levels, after Default"
        );
        assert_eq!(none[2].label, "Sandbox");

        let sel = HashMap::from([("model".to_string(), "b".to_string())]);
        let b = controls_for("codex", &caps, &sel);
        assert_eq!(ids(&b), ["model", "effort", "tier", "mode", "approval"]);
        assert_eq!(values(&b[1].choices), ["", "high", "max"]);
        assert_eq!(b[2].choices[0].label, "Standard");

        assert!(controls_for("claude", &Caps::default(), &HashMap::new()).is_empty());
    }

    #[test]
    fn prettifies_cli_values() {
        assert_eq!(pretty("acceptEdits"), "Accept edits");
        assert_eq!(pretty("workspace-write"), "Workspace write");
        assert_eq!(pretty("on_request"), "On request");
        assert_eq!(pretty(""), "");
    }

    #[test]
    fn flag_block_stops_at_the_next_flag() {
        let help = "  --a <x>  first\n           more\n  --b      second\n";
        assert_eq!(
            flag_block(help, "--a"),
            "  --a <x>  first\n           more\n"
        );
        assert_eq!(flag_block(help, "--zzz"), "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(choices: &[Choice]) -> Vec<&str> {
        choices.iter().map(|c| c.value.as_str()).collect()
    }

    #[test]
    fn parses_claude_help_when_installed() {
        let caps = claude();
        if !caps.installed {
            return;
        }
        println!(
            "claude {} modes={:?} efforts={:?} models={:?}",
            caps.version,
            names(&caps.modes),
            names(&caps.efforts),
            caps.models
                .iter()
                .map(|m| m.choice.value.as_str())
                .collect::<Vec<_>>()
        );
        assert!(caps.modes.iter().any(|m| m.value == "plan"));
        assert!(caps.efforts.iter().any(|m| m.value == "high"));
        assert!(!caps.models.is_empty());
    }

    #[test]
    fn parses_codex_when_installed() {
        let caps = codex();
        if !caps.installed {
            return;
        }
        println!(
            "codex {} modes={:?} approvals={:?} models={:?}",
            caps.version,
            names(&caps.modes),
            names(&caps.approvals),
            caps.models
                .iter()
                .map(|m| (m.choice.value.as_str(), m.efforts.len(), m.tiers.len()))
                .collect::<Vec<_>>()
        );
        assert!(caps.modes.iter().any(|m| m.value == "read-only"));
        assert!(caps.approvals.iter().any(|m| m.value == "never"));
    }

    #[test]
    fn parses_opencode_when_installed() {
        let caps = opencode();
        if !caps.installed {
            return;
        }
        println!(
            "opencode {} models={} agents={:?} modes={:?}",
            caps.version,
            caps.models.len(),
            names(&caps.agents),
            names(&caps.modes)
        );
        assert!(!caps.models.is_empty());
    }
}
