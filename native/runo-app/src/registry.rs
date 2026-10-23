//! The agents Runo can run, described as data rather than code. Each agent's facts (its
//! command, how it installs and signs in, which output format and protocols it speaks,
//! and which of them only newer releases have) come from a manifest: `agents.json`,
//! built into Runo, with the person's own `agents.json` in Runo's folder on top. That
//! file can change a built-in agent's facts or add an agent of its own, such as any ACP
//! agent, without a new release of Runo.
//!
//! A fact that depends on the installed release is a `Gate`: true, false, or text the
//! CLI's `--help` must mention, checked against what `caps` found when Runo started.
//! Nothing here starts a model or costs tokens.

use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

/// The manifest built into Runo.
const BUILT_IN: &str = include_str!("agents.json");

/// A fact that holds always, never, or once the CLI's help mentions some text.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Gate {
    Always(bool),
    /// The installed CLI's `--help` mentions this text.
    Mentions(String),
}

impl Default for Gate {
    fn default() -> Gate {
        Gate::Always(false)
    }
}

impl Gate {
    pub fn holds(&self, provider: &str) -> bool {
        match self {
            Gate::Always(on) => *on,
            Gate::Mentions(text) => crate::caps::mentions(provider, text),
        }
    }
}

/// Arguments to use once the CLI's help mentions `when` (always, without it).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Args {
    #[serde(default)]
    pub when: Option<String>,
    pub args: Vec<String>,
}

/// An output format, once the CLI's help mentions `when`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Format {
    #[serde(default)]
    pub when: Option<String>,
    pub format: String,
}

/// How a turn's command takes its prompt and later messages on stdin instead: the
/// prompt after `flag` at the end of the arguments goes, and `args` take its place.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Live {
    pub flag: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub when: Option<String>,
}

/// How an agent installs: its npm package, or its vendor's install script.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Install {
    #[serde(default)]
    pub npm: Option<String>,
    /// Use npm when it is there, the script otherwise.
    #[serde(default)]
    pub prefer_npm: bool,
    #[serde(default)]
    pub unix: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
}

/// The command that lists an agent's models.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Models {
    pub args: Vec<String>,
    /// The list is a table whose header names `provider` and `model` columns.
    #[serde(default)]
    pub table: bool,
}

/// A print-mode command for an agent Runo has no code for: the arguments, the flag that
/// picks a model, and the prompt last.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Print {
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// One agent.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AgentSpec {
    pub id: String,
    pub name: String,
    pub bin: String,
    /// Other names the CLI installs under.
    #[serde(default)]
    pub alt_bins: Vec<String>,
    #[serde(default)]
    pub mark: String,
    /// "#rrggbb".
    #[serde(default)]
    pub color: String,
    /// How Runo talks to an agent it has no code of its own for: "acp", or "pi" for
    /// Pi's RPC mode. Empty for the agents Runo knows.
    #[serde(default)]
    pub driver: String,
    /// The npm package Runo checks for updates.
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub install: Install,
    /// Arguments of the CLI's sign-in command; none means the CLI asks on first run.
    #[serde(default)]
    pub login: Vec<String>,
    /// The CLI keeps its own conversation that a follow-up can resume.
    #[serde(default)]
    pub resumes: Gate,
    /// The CLI takes a conversation id Runo picks on the first run.
    #[serde(default)]
    pub names_conversation: Gate,
    /// Images can be attached for the agent.
    #[serde(default)]
    pub images: Gate,
    /// Runo's permission levels the CLI can hold to.
    #[serde(default = "all_modes")]
    pub modes: Vec<String>,
    /// The output format to ask for: the first whose `when` the help mentions.
    #[serde(default)]
    pub output: Vec<Format>,
    /// Arguments that run the agent over ACP: the first whose `when` the help mentions.
    #[serde(default)]
    pub acp: Vec<Args>,
    /// Arguments that run the agent over its own RPC mode (Pi).
    #[serde(default)]
    pub rpc: Vec<Args>,
    #[serde(default)]
    pub live: Vec<Live>,
    #[serde(default)]
    pub models: Option<Models>,
    /// Reasoning levels the agent takes, for the composer's Effort menu.
    #[serde(default)]
    pub efforts: Vec<String>,
    #[serde(default)]
    pub print: Option<Print>,
}

fn all_modes() -> Vec<String> {
    ["edit", "plan", "full"].map(String::from).to_vec()
}

impl AgentSpec {
    /// The first `args` whose `when` the installed CLI's help mentions.
    pub fn pick<'a>(&self, choices: &'a [Args]) -> Option<&'a [String]> {
        choices
            .iter()
            .find(|c| {
                c.when
                    .as_deref()
                    .is_none_or(|w| crate::caps::mentions(&self.id, w))
            })
            .map(|c| c.args.as_slice())
    }

    /// The output format to ask the installed CLI for.
    pub fn format(&self) -> &str {
        self.output
            .iter()
            .find(|f| {
                f.when
                    .as_deref()
                    .is_none_or(|w| crate::caps::mentions(&self.id, w))
            })
            .map(|f| f.format.as_str())
            .unwrap_or("text")
    }

    /// The colour as a number for the UI.
    pub fn color(&self) -> u32 {
        u32::from_str_radix(self.color.trim_start_matches('#'), 16).unwrap_or(0x8a8a93)
    }
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    agents: Vec<Value>,
}

/// The manifest `text` with `over` on top: an agent `over` names by an id `text` has
/// gets the fields `over` gives replaced; a new id adds an agent, run over ACP when it
/// gives ACP arguments and no driver.
pub fn merge(text: &str, over: Option<&str>) -> Result<Vec<AgentSpec>, String> {
    let base: Manifest = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut agents = base.agents;
    if let Some(over) = over {
        let over: Manifest = serde_json::from_str(over).map_err(|e| format!("agents.json: {e}"))?;
        for agent in over.agents {
            let Some(id) = agent["id"].as_str() else {
                continue;
            };
            match agents.iter_mut().find(|a| a["id"] == id) {
                Some(Value::Object(known)) => {
                    if let Value::Object(fields) = agent {
                        known.extend(fields);
                    }
                }
                _ => {
                    let mut agent = agent;
                    if agent.get("driver").is_none() && agent.get("acp").is_some() {
                        agent["driver"] = "acp".into();
                    }
                    agents.push(agent);
                }
            }
        }
    }
    let mut specs = Vec::new();
    for agent in agents {
        let id = agent["id"].as_str().unwrap_or("?").to_string();
        let spec: AgentSpec =
            serde_json::from_value(agent).map_err(|e| format!("agent {id}: {e}"))?;
        if spec.id.is_empty()
            || spec.bin.is_empty()
            || specs.iter().any(|s: &AgentSpec| s.id == spec.id)
        {
            continue;
        }
        specs.push(spec);
    }
    Ok(specs)
}

struct Registry {
    specs: &'static [AgentSpec],
    providers: &'static [crate::agent::Provider],
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// The person's own manifest, read once when Runo starts.
fn own() -> Option<String> {
    if cfg!(test) {
        return None;
    }
    std::fs::read_to_string(crate::store::data_dir().join("agents.json")).ok()
}

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| {
        let own = own();
        let specs = match merge(BUILT_IN, own.as_deref()) {
            Ok(specs) => specs,
            Err(why) => {
                // A broken file of the person's leaves the built-in agents as they are.
                eprintln!("Runo couldn't read your agents.json, so it was left out: {why}");
                merge(BUILT_IN, None).expect("the built-in manifest reads")
            }
        };
        let specs: &'static [AgentSpec] = Box::leak(specs.into_boxed_slice());
        let providers: Vec<crate::agent::Provider> = specs
            .iter()
            .map(|s| crate::agent::Provider {
                id: &s.id,
                name: &s.name,
                bin: &s.bin,
                mark: if s.mark.is_empty() {
                    &s.name[..s.name.chars().next().map_or(0, char::len_utf8)]
                } else {
                    &s.mark
                },
                color: s.color(),
            })
            .collect();
        Registry {
            specs,
            providers: Box::leak(providers.into_boxed_slice()),
        }
    })
}

/// Every agent, the built-in ones first, in a fixed order for the whole run.
pub fn providers() -> &'static [crate::agent::Provider] {
    registry().providers
}

/// The manifest entry of `id`.
pub fn spec(id: &str) -> Option<&'static AgentSpec> {
    registry().specs.iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_manifest_names_every_agent_once() {
        let specs = merge(BUILT_IN, None).unwrap();
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "claude", "codex", "opencode", "cursor", "gemini", "copilot", "amp", "droid",
                "qwen", "grok", "pi"
            ]
        );
        let claude = &specs[0];
        assert_eq!(claude.color(), 0xd9633f);
        assert_eq!(claude.resumes, Gate::Always(true));
        assert_eq!(claude.modes, ["edit", "plan", "full"]);
        let gemini = specs.iter().find(|s| s.id == "gemini").unwrap();
        assert_eq!(
            gemini.names_conversation,
            Gate::Mentions("--session-id".into())
        );
        assert_eq!(gemini.acp.len(), 2);
    }

    #[test]
    fn the_persons_file_changes_facts_and_adds_acp_agents() {
        let own = r#"{"agents":[
            {"id":"grok","modes":["full","edit"]},
            {"id":"kimi","name":"Kimi","bin":"kimi","acp":[{"args":["--acp"]}]},
            {"name":"no id","bin":"x"}
        ]}"#;
        let specs = merge(BUILT_IN, Some(own)).unwrap();
        let grok = specs.iter().find(|s| s.id == "grok").unwrap();
        assert_eq!(grok.modes, ["full", "edit"]);
        assert_eq!(grok.name, "Grok", "fields it doesn't give stay");
        let kimi = specs.last().unwrap();
        assert_eq!(kimi.id, "kimi");
        assert_eq!(kimi.driver, "acp");
        assert_eq!(kimi.acp[0].args, ["--acp"]);
        assert_eq!(kimi.modes, ["edit", "plan", "full"]);
        assert!(merge(BUILT_IN, Some("{not json")).is_err());
    }

    #[test]
    fn gates_and_choices_follow_the_installed_help() {
        let spec: AgentSpec = serde_json::from_value(serde_json::json!({
            "id": "registry-test", "name": "T", "bin": "t",
            "output": [{ "when": "--json", "format": "json" }],
            "acp": [{ "when": "--acp", "args": ["--acp"] }, { "args": ["acp"] }],
        }))
        .unwrap();
        assert_eq!(spec.format(), "text");
        assert_eq!(spec.pick(&spec.acp).unwrap(), ["acp"]);
        crate::caps::set(
            "registry-test",
            crate::caps::Caps {
                help: "--json  --acp".into(),
                ..Default::default()
            },
        );
        assert_eq!(spec.format(), "json");
        assert_eq!(spec.pick(&spec.acp).unwrap(), ["--acp"]);
        assert!(Gate::Mentions("--acp".into()).holds("registry-test"));
        assert!(!Gate::Always(false).holds("registry-test"));
    }
}
