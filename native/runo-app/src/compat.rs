//! `Runo --check-agents`: how well the installed agent CLIs match what the manifest
//! says about them, without signing in or starting a model. For each agent it records
//! the version, whether the help still mentions each text a manifest fact waits on, and
//! whether its protocol (ACP, Codex's app server, Pi's RPC mode) still answers the
//! opening handshake. The nightly Agent compatibility workflow runs it on the newest
//! release of every agent and compares the report with the night before, so a release
//! that renames a flag or changes its protocol shows up before people meet it.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::harness::rpc::{Agent, Message, Reply};
use crate::registry::{AgentSpec, Gate};

/// How long an agent has to answer the handshake.
const WAIT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub id: String,
    pub installed: bool,
    #[serde(default)]
    pub version: String,
    /// Each text a manifest fact waits on, and whether the help mentions it.
    #[serde(default)]
    pub mentions: BTreeMap<String, bool>,
    /// The protocol Runo would use: "acp", "codex" or "pi", if any.
    #[serde(default)]
    pub protocol: String,
    /// "ok", or why the handshake failed.
    #[serde(default)]
    pub handshake: String,
    /// Messages the agent sent during the handshake that Runo doesn't know.
    #[serde(default)]
    pub unknown: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub agents: Vec<AgentReport>,
}

/// Every text a fact of `spec` waits on in the CLI's help.
fn texts(spec: &AgentSpec) -> Vec<String> {
    let gates = [&spec.resumes, &spec.names_conversation, &spec.images];
    let mut out: Vec<String> = gates
        .into_iter()
        .filter_map(|g| match g {
            Gate::Mentions(text) => Some(text.clone()),
            Gate::Always(_) => None,
        })
        .chain(spec.output.iter().filter_map(|f| f.when.clone()))
        .chain(
            spec.acp
                .iter()
                .chain(&spec.rpc)
                .filter_map(|a| a.when.clone()),
        )
        .chain(spec.live.iter().filter_map(|l| l.when.clone()))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Check one agent, as installed here.
fn check(provider: &'static crate::agent::Provider, scratch: &Path) -> AgentReport {
    crate::caps::detect(provider);
    let caps = crate::caps::get(provider.id);
    let mut report = AgentReport {
        id: provider.id.to_string(),
        installed: caps.installed,
        version: caps.version.clone(),
        ..AgentReport::default()
    };
    let Some(spec) = crate::registry::spec(provider.id) else {
        return report;
    };
    report.mentions = texts(spec)
        .into_iter()
        .map(|t| {
            let held = caps.help.contains(&t);
            (t, held)
        })
        .collect();
    if !caps.installed {
        return report;
    }
    let bin = crate::caps::binary(provider);
    let dir = scratch.join(provider.id);
    let _ = std::fs::create_dir_all(&dir);
    let result = if let Some(args) = crate::agent::acp_args(provider.id) {
        report.protocol = "acp".into();
        acp(&dir, &bin, &args, &mut report.unknown)
    } else if let Some(args) = spec.pick(&spec.rpc).filter(|_| provider.id == "codex") {
        report.protocol = "codex".into();
        codex(&dir, &bin, args, &mut report.unknown)
    } else if let Some(args) = spec.pick(&spec.rpc).filter(|_| spec.driver == "pi") {
        report.protocol = "pi".into();
        pi(&dir, &bin, args, &mut report.unknown)
    } else {
        return report;
    };
    report.handshake = match result {
        Ok(()) => "ok".into(),
        Err(why) => why,
    };
    report
}

/// Wait for the answer to `id`, noting methods outside `known`.
fn answer(
    agent: &mut Agent,
    id: u64,
    known: &[&str],
    unknown: &mut Vec<String>,
) -> Result<Value, String> {
    match agent.wait(id, Some(WAIT), &mut |_, message| {
        if let Message::Notification { method, .. } | Message::Request { method, .. } = message {
            if !known.contains(&method.as_str()) && !unknown.contains(&method) {
                unknown.push(method);
            }
        }
    }) {
        Reply::Result(result) => Ok(result),
        Reply::Error(e) => Err(format!("refused: {e}")),
        Reply::Gone => Err("exited before answering".into()),
        Reply::TimedOut => Err("didn't answer".into()),
    }
}

fn acp(dir: &Path, bin: &str, args: &[String], unknown: &mut Vec<String>) -> Result<(), String> {
    let mut agent = Agent::spawn(dir, bin, args, &dir.to_string_lossy())?;
    let id = agent.request(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true }, "terminal": false },
            "clientInfo": { "name": "Runo", "version": env!("CARGO_PKG_VERSION") },
        }),
    );
    let result = answer(&mut agent, id, &["session/update"], unknown);
    agent.kill();
    let init = result?;
    if init["protocolVersion"].is_null() {
        return Err("answered without a protocol version".into());
    }
    Ok(())
}

fn codex(dir: &Path, bin: &str, args: &[String], unknown: &mut Vec<String>) -> Result<(), String> {
    let mut agent = Agent::spawn(dir, bin, args, &dir.to_string_lossy())?;
    let id = agent.request(
        "initialize",
        json!({
            "clientInfo": { "name": "runo", "title": "Runo", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": null,
        }),
    );
    let result = answer(&mut agent, id, crate::harness::codex::KNOWN, unknown);
    agent.kill();
    result.map(|_| ())
}

fn pi(dir: &Path, bin: &str, args: &[String], unknown: &mut Vec<String>) -> Result<(), String> {
    let mut args = args.to_vec();
    args.push("--no-session".into());
    let mut agent = Agent::spawn(dir, bin, &args, &dir.to_string_lossy())?;
    agent.send(&json!({ "id": "check", "type": "get_state" }));
    let start = Instant::now();
    let result = loop {
        let Some(left) = WAIT.checked_sub(start.elapsed()) else {
            break Err("didn't answer".to_string());
        };
        match agent.line(left) {
            Ok(Some(line)) => {
                let Ok(record) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let kind = record["type"].as_str().unwrap_or("").to_string();
                if kind == "response" && record["id"] == "check" {
                    break match record["data"]["sessionId"].as_str() {
                        Some(_) if record["success"] == true => Ok(()),
                        _ => Err(format!(
                            "refused: {}",
                            record["error"].as_str().unwrap_or("no session")
                        )),
                    };
                }
                if kind != "response"
                    && !crate::harness::pi::KNOWN.contains(&kind.as_str())
                    && !unknown.contains(&kind)
                {
                    unknown.push(kind);
                }
            }
            Ok(None) => break Err("didn't answer".into()),
            Err(()) => break Err("exited before answering".into()),
        }
    };
    agent.kill();
    result
}

/// Every agent in the manifest, checked here.
pub fn report() -> Report {
    let scratch = std::env::temp_dir().join(format!("runo-check-agents-{}", std::process::id()));
    let agents = crate::agent::PROVIDERS
        .iter()
        .map(|p| check(p, &scratch))
        .collect();
    let _ = std::fs::remove_dir_all(&scratch);
    Report { agents }
}

/// What changed from `old` to `new`, worst first: `(regression, line)`. A regression is
/// a text a fact waits on that the help no longer mentions, or a handshake that worked
/// and now fails.
pub fn changes(old: &Report, new: &Report) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    for agent in &new.agents {
        let Some(was) = old.agents.iter().find(|a| a.id == agent.id) else {
            continue;
        };
        if !agent.installed {
            if was.installed {
                out.push((true, format!("{}: no longer installs", agent.id)));
            }
            continue;
        }
        if agent.version != was.version {
            out.push((
                false,
                format!(
                    "{}: {} → {}",
                    agent.id,
                    or_none(&was.version),
                    agent.version
                ),
            ));
        }
        for (text, held) in &agent.mentions {
            match (was.mentions.get(text), held) {
                (Some(true), false) => out.push((
                    true,
                    format!("{}: help no longer mentions `{text}`", agent.id),
                )),
                (Some(false) | None, true) => {
                    out.push((false, format!("{}: help now mentions `{text}`", agent.id)))
                }
                _ => {}
            }
        }
        if agent.handshake != "ok" && !agent.handshake.is_empty() && was.handshake == "ok" {
            out.push((
                true,
                format!(
                    "{}: the {} handshake fails: {}",
                    agent.id, agent.protocol, agent.handshake
                ),
            ));
        }
        if agent.handshake == "ok" && was.handshake != "ok" && !was.handshake.is_empty() {
            out.push((
                false,
                format!("{}: the {} handshake works again", agent.id, agent.protocol),
            ));
        }
        for method in agent.unknown.iter().filter(|m| !was.unknown.contains(m)) {
            out.push((
                false,
                format!(
                    "{}: sends `{method}`, which Runo doesn't read yet",
                    agent.id
                ),
            ));
        }
    }
    out.sort_by_key(|(regression, _)| !regression);
    out
}

fn or_none(v: &str) -> &str {
    if v.is_empty() {
        "nothing"
    } else {
        v
    }
}

/// The report as a Markdown table.
pub fn table(report: &Report) -> String {
    let mut out = String::from(
        "| Agent | Version | Protocol | Handshake | Help mentions |\n|---|---|---|---|---|\n",
    );
    for a in &report.agents {
        if !a.installed {
            out.push_str(&format!("| {} | not installed | | | |\n", a.id));
            continue;
        }
        let mentions: Vec<String> = a
            .mentions
            .iter()
            .map(|(t, held)| format!("{} `{t}`", if *held { "✓" } else { "✗" }))
            .collect();
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            a.id,
            a.version.replace('|', "/"),
            a.protocol,
            a.handshake.replace('|', "/"),
            mentions.join(", ")
        ));
    }
    out
}

/// `Runo --check-agents [--report <file>] [--since <file>]`: check every agent, write
/// the report to `--report` (JSON), and print it with what changed since `--since`.
/// Exits 1 when a handshake fails, 2 when something regressed since `--since`.
pub fn main(args: &[String]) -> i32 {
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    crate::shellenv::init();
    let report = report();
    if let Some(path) = flag("--report") {
        if let Err(e) = std::fs::write(
            &path,
            serde_json::to_vec_pretty(&report).unwrap_or_default(),
        ) {
            eprintln!("Runo couldn't write {path}: {e}");
            return 2;
        }
    }
    println!("{}", table(&report));
    let old = flag("--since")
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Report>(&b).ok());
    let mut regressed = false;
    if let Some(old) = old {
        let changes = changes(&old, &report);
        if changes.is_empty() {
            println!("Nothing changed since the last check.");
        } else {
            println!("Since the last check:\n");
            for (regression, line) in &changes {
                regressed |= regression;
                println!(
                    "- {}{line}",
                    if *regression { "**Regression:** " } else { "" }
                );
            }
        }
    }
    let failed = report
        .agents
        .iter()
        .any(|a| a.installed && !a.handshake.is_empty() && a.handshake != "ok");
    if regressed {
        2
    } else if failed {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, version: &str, mentions: &[(&str, bool)], handshake: &str) -> AgentReport {
        AgentReport {
            id: id.into(),
            installed: true,
            version: version.into(),
            mentions: mentions.iter().map(|(t, h)| (t.to_string(), *h)).collect(),
            protocol: "acp".into(),
            handshake: handshake.into(),
            unknown: vec![],
        }
    }

    #[test]
    fn the_texts_a_manifest_waits_on_are_checked() {
        let gemini = crate::registry::spec("gemini").unwrap();
        assert_eq!(
            texts(gemini),
            ["--acp", "--experimental-acp", "--session-id", "stream-json"]
        );
        assert!(texts(crate::registry::spec("claude").unwrap()).is_empty());
    }

    #[test]
    fn regressions_come_first_and_new_things_are_noted() {
        let old = Report {
            agents: vec![
                agent(
                    "gemini",
                    "0.65",
                    &[("--acp", true), ("stream-json", true)],
                    "ok",
                ),
                agent("qwen", "1.0", &[("--acp", false)], "didn't answer"),
            ],
        };
        let mut gemini = agent(
            "gemini",
            "0.66",
            &[("--acp", false), ("stream-json", true)],
            "refused: no",
        );
        gemini.unknown = vec!["session/new_thing".into()];
        let new = Report {
            agents: vec![gemini, agent("qwen", "1.0", &[("--acp", true)], "ok")],
        };
        let found = changes(&old, &new);
        assert_eq!(
            found,
            [
                (true, "gemini: help no longer mentions `--acp`".to_string()),
                (
                    true,
                    "gemini: the acp handshake fails: refused: no".to_string()
                ),
                (false, "gemini: 0.65 → 0.66".to_string()),
                (
                    false,
                    "gemini: sends `session/new_thing`, which Runo doesn't read yet".to_string()
                ),
                (false, "qwen: help now mentions `--acp`".to_string()),
                (false, "qwen: the acp handshake works again".to_string()),
            ]
        );
        assert!(changes(&new, &new).is_empty());
        let table = table(&new);
        assert!(
            table.contains("| gemini | 0.66 | acp | refused: no | ✗ `--acp`, ✓ `stream-json` |"),
            "{table}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn handshakes_reach_the_protocols() {
        if std::process::Command::new("python3")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("runo-compat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let acp_agent = dir.join("acp.py");
        std::fs::write(
            &acp_agent,
            r#"
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    print(json.dumps({"jsonrpc": "2.0", "method": "brand/new", "params": {}}), flush=True)
    print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": {"protocolVersion": 1}}), flush=True)
"#,
        )
        .unwrap();
        let mut unknown = Vec::new();
        let args = vec![acp_agent.to_string_lossy().to_string()];
        assert_eq!(acp(&dir, "python3", &args, &mut unknown), Ok(()));
        assert_eq!(unknown, ["brand/new"]);
        let pi_agent = dir.join("pi.py");
        std::fs::write(
            &pi_agent,
            r#"
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    print(json.dumps({"type": "queue_update", "steering": [], "followUp": []}), flush=True)
    print(json.dumps({"id": m["id"], "type": "response", "command": "get_state", "success": True, "data": {"sessionId": "s"}}), flush=True)
"#,
        )
        .unwrap();
        let mut unknown = Vec::new();
        let args = vec![pi_agent.to_string_lossy().to_string()];
        assert_eq!(pi(&dir, "python3", &args, &mut unknown), Ok(()));
        assert!(unknown.is_empty());
        let args = vec!["-c".to_string(), "import sys; sys.exit(1)".to_string()];
        assert_eq!(
            acp(&dir, "python3", &args, &mut Vec::new()),
            Err("exited before answering".into())
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
