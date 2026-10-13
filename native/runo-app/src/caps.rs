//! Discovers what each installed agent CLI supports (version, models, permission modes,
//! reasoning levels, service tiers…) by asking the CLI itself, so nothing is hard-coded
//! beyond how to parse each tool's own help output.

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

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
    /// The oldest CLI release that can use this model ("0.162.0"), when the CLI says.
    pub needs: String,
    /// When the model is retired ("2026-10-13"), when the CLI says.
    pub retires: String,
    /// The model the CLI suggests moving to.
    pub upgrade_to: String,
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
    /// The model the CLI uses when none is picked, by name ("Sonnet 5.5"), when it says.
    pub default_model: String,
    /// The CLI's own `--help`, for flags that only newer releases have.
    pub help: String,
    /// Models the CLI keeps out of its menu but still accepts (Codex's hidden ones).
    pub hidden_models: Vec<ModelInfo>,
    /// The CLI release that wrote its model list, which can be newer than the one installed.
    pub client_version: String,
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
/// Binaries set in Settings, by agent, for CLIs installed off PATH.
static BINARIES: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Remember the binaries set in Settings, so detection runs the right file.
pub fn set_binaries(binaries: HashMap<String, String>) {
    if let Ok(mut map) = BINARIES.write() {
        *map = Some(binaries);
    }
}

/// The program to run for an agent: its Settings binary, or the usual name on PATH.
pub fn binary(provider: &'static crate::agent::Provider) -> String {
    BINARIES
        .read()
        .ok()
        .and_then(|map| map.as_ref()?.get(provider.id).cloned())
        .filter(|b| !b.trim().is_empty())
        .map(|b| b.trim().to_string())
        .unwrap_or_else(|| provider.bin.to_string())
}

fn store() -> &'static RwLock<HashMap<String, Caps>> {
    CAPS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Whether the installed CLI's help mentions `text`, such as a flag newer releases added.
pub fn mentions(provider: &str, text: &str) -> bool {
    store()
        .read()
        .ok()
        .and_then(|m| m.get(provider).map(|c| c.help.contains(text)))
        .unwrap_or(false)
}

/// Record what a CLI supports; used by tests to stand in for detection.
#[cfg(test)]
pub fn set(provider: &str, caps: Caps) {
    if let Ok(mut map) = store().write() {
        map.insert(provider.to_string(), caps);
    }
}

pub fn get(provider: &str) -> Caps {
    store()
        .read()
        .ok()
        .and_then(|m| m.get(provider).cloned())
        .unwrap_or_default()
}

/// Ask the UI to repaint, as when discovery finished.
pub fn mark_updated() {
    UPDATED.store(true, Ordering::Relaxed);
}

/// True once after discovery finished, so the UI can repaint.
pub fn take_updated() -> bool {
    UPDATED.swap(false, Ordering::Relaxed)
}

/// Discover every provider on a background thread.
pub fn refresh() {
    std::thread::spawn(|| {
        crate::install::refresh_node();
        for provider in crate::agent::PROVIDERS.iter() {
            detect(provider);
            UPDATED.store(true, Ordering::Relaxed);
        }
        for provider in crate::agent::PROVIDERS.iter() {
            crate::copies::refresh(provider);
        }
        crate::signin::refresh(true);
    });
}

/// Discover one provider now, on this thread.
pub fn detect(provider: &'static crate::agent::Provider) {
    let bin = binary(provider);
    let caps = match provider.id {
        "claude" => claude(&bin),
        "codex" => codex(&bin),
        "opencode" => opencode(&bin),
        id => generic(id, &bin),
    };
    if let Ok(mut map) = store().write() {
        map.insert(provider.id.to_string(), caps);
    }
}

/// Agents without parseable help: the version, and Runo's three permission levels,
/// which `agent::provider_args` maps onto each CLI's own flags.
fn generic(provider: &str, bin: &str) -> Caps {
    let installed = crate::agent::available(bin, bin);
    let version = if installed {
        run(bin, &["--version"])
            .lines()
            .find(|l| l.chars().any(|c| c.is_ascii_digit()))
            .unwrap_or("")
            .trim()
            .chars()
            .take(40)
            .collect()
    } else {
        String::new()
    };
    let level = |value: &str, label: &str, desc: &str| Choice {
        value: value.into(),
        label: label.into(),
        desc: desc.into(),
    };
    let help = if installed {
        run(bin, &["--help"])
    } else {
        String::new()
    };
    // Models: the CLI's own list when it has one, else the choices its help names.
    let models = if !installed {
        Vec::new()
    } else if provider == "cursor" {
        crate::models::parse_model_list(&run_quiet(bin, &["models"], Duration::from_secs(20)))
    } else {
        crate::models::help_model_choices(&help)
    };
    // Only the levels the CLI can really hold to: one without a read-only mode doesn't
    // offer Plan only, and one that always runs every tool offers only Full access.
    let modes = crate::agent::modes(provider)
        .iter()
        .map(|mode| match *mode {
            "edit" => level("edit", "Auto edit", "Edit files without asking"),
            "plan" => level("plan", "Plan only", "Read and suggest; no edits"),
            _ => level("full", "Full access", "Run anything without asking"),
        })
        .collect();
    Caps {
        installed,
        version,
        models,
        modes,
        detected: true,
        help,
        ..Caps::default()
    }
}

fn run(bin: &str, args: &[&str]) -> String {
    Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
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

fn claude(bin: &str) -> Caps {
    let version = run(bin, &["--version"]);
    if version.trim().is_empty() {
        return not_installed();
    }
    let mut caps = parse_claude(&version, &run(bin, &["--help"]));
    // `/model` is answered by Claude Code itself (no model call, nothing saved): it names
    // the default model with its version and lists the aliases.
    let menu = run_quiet(
        bin,
        &["-p", "/model", "--no-session-persistence"],
        Duration::from_secs(20),
    );
    // The model ids this build of Claude Code knows, and the ones used here before.
    let known = crate::updater::resolve(bin)
        .and_then(|p| std::fs::canonicalize(p).ok())
        .filter(|p| p.metadata().is_ok_and(|m| m.len() < 600 << 20))
        .and_then(|p| std::fs::read(p).ok())
        .map(|bytes| crate::models::claude_ids_in(&bytes))
        .unwrap_or_default();
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let used = crate::models::claude_used_ids(&home);
    let aliases: Vec<String> = caps.models.iter().map(|m| m.choice.value.clone()).collect();
    let (models, default) = crate::models::claude_menu(&menu, &aliases, &known, &used);
    if !models.is_empty() {
        caps.models = models;
    }
    caps.default_model = default;
    caps
}

/// Run a CLI for a short answer, outside any agent session, giving up after `limit`.
fn run_quiet(bin: &str, args: &[&str], limit: Duration) -> String {
    use std::io::Read;
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    out
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

fn codex(bin: &str) -> Caps {
    let version = run(bin, &["--version"]);
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
        &run(bin, &["exec", "--help"]),
        &run(bin, &["--help"]),
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
    let mut hidden_models: Vec<(i64, ModelInfo)> = Vec::new();
    if let Some(list) = cache
        .and_then(|v| v.get("models"))
        .and_then(|m| m.as_array())
    {
        for m in list {
            let hidden = m.get("visibility").and_then(|v| v.as_str()) != Some("list");
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
            let (upgrade_to, retires) = codex_upgrade(m);
            let info = ModelInfo {
                choice: Choice {
                    value: slug.to_string(),
                    label: name.to_string(),
                    desc: desc.to_string(),
                },
                efforts: pick("supported_reasoning_levels", "effort", "effort"),
                tiers: pick("service_tiers", "id", "name"),
                needs: m
                    .get("minimal_client_version")
                    .or_else(|| m.get("min_client_version"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                retires,
                upgrade_to,
            };
            let priority = m.get("priority").and_then(|p| p.as_i64()).unwrap_or(999);
            if hidden {
                hidden_models.push((priority, info));
            } else {
                models.push((priority, info));
            }
        }
    }
    models.sort_by_key(|(p, _)| *p);
    hidden_models.sort_by_key(|(p, _)| *p);
    Caps {
        installed: true,
        version: version
            .trim()
            .trim_start_matches("codex-cli")
            .trim()
            .to_string(),
        models: models.into_iter().map(|(_, m)| m).collect(),
        hidden_models: hidden_models.into_iter().map(|(_, m)| m).collect(),
        client_version: cache
            .and_then(|v| v.get("client_version"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        modes,
        approvals,
        detected: true,
        ..Default::default()
    }
}

/// The model a Codex cache entry says to move to, and the day it retires. Codex has
/// written this in a few shapes, so each is read when present.
fn codex_upgrade(m: &serde_json::Value) -> (String, String) {
    let text = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).unwrap_or("").to_string();
    let upgrade = m.get("upgrade");
    let to = match upgrade {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(u) => ["model", "id", "slug", "to"]
            .iter()
            .map(|k| text(u.get(*k)))
            .find(|v| !v.is_empty())
            .unwrap_or_default(),
        None => String::new(),
    };
    let keys = [
        "retires_at",
        "retirement_date",
        "deprecation_date",
        "sunset_date",
        "shutdown_date",
        "date",
    ];
    let mut date = keys
        .iter()
        .flat_map(|k| [upgrade.and_then(|u| u.get(*k)), m.get(*k)])
        .map(text)
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    if date.is_empty() {
        // Otherwise a date written in the upgrade notice.
        let notice = upgrade
            .map(|u| {
                ["upgrade_copy", "migration_markdown", "message"]
                    .iter()
                    .map(|k| text(u.get(*k)))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        date = first_date(&notice).unwrap_or_default();
    }
    (to, date.chars().take(10).collect())
}

/// The first YYYY-MM-DD date in a text.
fn first_date(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(9)).find_map(|i| {
        let w = &bytes[i..i + 10];
        let digits = |r: std::ops::Range<usize>| w[r].iter().all(u8::is_ascii_digit);
        (digits(0..4) && w[4] == b'-' && digits(5..7) && w[7] == b'-' && digits(8..10))
            .then(|| text[i..i + 10].to_string())
    })
}

/// A note for a model's menu entry: that it needs a newer CLI than `version`, or when it
/// retires and what replaces it. Empty when there's nothing to say.
pub fn model_note(provider: &str, m: &ModelInfo, version: &str, today: &str) -> String {
    let name = crate::agent::provider(provider).name;
    if !m.needs.is_empty() && crate::alerts::is_newer(&m.needs, version) {
        return format!("Needs {name} {}+", short_version(&m.needs));
    }
    if m.retires.is_empty() {
        return String::new();
    }
    let when = match m.retires.as_str().cmp(today) {
        std::cmp::Ordering::Less => format!("Retired {}", m.retires),
        std::cmp::Ordering::Equal => "Retires today".to_string(),
        std::cmp::Ordering::Greater => format!("Retires {}", m.retires),
    };
    if m.upgrade_to.is_empty() {
        when
    } else {
        format!("{when}, switch to {}", m.upgrade_to)
    }
}

/// "0.162.1" from "codex-cli 0.162.1", or the text as given when it has no number.
pub fn short_version(version: &str) -> String {
    let parts = crate::alerts::version_parts(version);
    if parts.is_empty() {
        version.trim().to_string()
    } else {
        parts
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }
}

/// Every model the CLI accepts: its menu, then the hidden ones.
pub fn all_models(caps: &Caps) -> impl Iterator<Item = &ModelInfo> {
    caps.models.iter().chain(caps.hidden_models.iter())
}

fn opencode(bin: &str) -> Caps {
    let version = run(bin, &["--version"]);
    if version.trim().is_empty() {
        return not_installed();
    }
    parse_opencode(
        &version,
        &run(bin, &["models"]),
        &run(bin, &["agent", "list"]),
        &run(bin, &["run", "--help"]),
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
    let today = crate::store::today();
    let model_choices: Vec<Choice> = caps
        .models
        .iter()
        .map(|m| {
            let mut c = m.choice.clone();
            let note = model_note(provider, m, &caps.version, &today);
            if !note.is_empty() {
                c.desc = if c.desc.is_empty() {
                    note
                } else {
                    format!("{note} · {}", c.desc)
                };
            }
            c
        })
        .collect();
    if !model_choices.is_empty() {
        out.push(Control {
            id: "model",
            label: "Model".into(),
            choices: with_default(
                &if caps.default_model.is_empty() {
                    "Default".to_string()
                } else {
                    format!("Default · {}", caps.default_model)
                },
                model_choices,
            ),
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
    // No Approval chip: `codex exec` never stops to ask, so its policy changes nothing
    // in Chat.
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
    fn codex_cache_keeps_hidden_retiring_and_newer_models() {
        let cache: serde_json::Value = serde_json::from_str(
            r#"{"client_version":"0.162.1","models":[
              {"slug":"gpt-6-sol","priority":1,"visibility":"list","minimal_client_version":"0.160.0"},
              {"slug":"gpt-5.5","priority":2,"visibility":"list",
               "upgrade":{"model":"gpt-6.1-sol","migration_markdown":"GPT-5.5 retires on 2026-10-13."}},
              {"slug":"gpt-5.4","priority":3,"visibility":"list","upgrade":"gpt-5.6-sol","retires_at":"2026-11-01T00:00:00Z"},
              {"slug":"gpt-6-astra","priority":4,"visibility":"hide"}
            ]}"#,
        )
        .unwrap();
        let caps = parse_codex("codex-cli 0.147.0", "", "", Some(&cache));
        assert_eq!(caps.client_version, "0.162.1");
        assert_eq!(caps.models.len(), 3);
        assert_eq!(values(&caps.hidden_models.iter().map(|m| m.choice.clone()).collect::<Vec<_>>()), ["gpt-6-astra"]);
        assert_eq!(all_models(&caps).count(), 4);
        let note = |i: usize| model_note("codex", &caps.models[i], &caps.version, "2026-10-13");
        assert_eq!(note(0), "Needs Codex 0.160.0+");
        assert_eq!(note(1), "Retires today, switch to gpt-6.1-sol");
        assert_eq!(note(2), "Retires 2026-11-01, switch to gpt-5.6-sol");
        assert_eq!(model_note("codex", &caps.models[0], "0.162.1", "2026-10-13"), "");
        assert_eq!(model_note("codex", &caps.models[1], "0.162.1", "2026-10-14"), "Retired 2026-10-13, switch to gpt-6.1-sol");
        assert_eq!(first_date("no date"), None);
        assert_eq!(short_version("codex-cli 0.162.1\n"), "0.162.1");
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
                    ..Default::default()
                },
                ModelInfo {
                    choice: level("b"),
                    efforts: vec![level("high"), level("max")],
                    tiers: vec![level("priority")],
                    ..Default::default()
                },
            ],
            modes: vec![level("read-only")],
            approvals: vec![level("never")],
            ..Default::default()
        };
        let ids = |c: &[Control]| c.iter().map(|c| c.id).collect::<Vec<_>>();
        let none = controls_for("codex", &caps, &HashMap::new());
        assert_eq!(ids(&none), ["model", "effort", "mode"]);
        assert_eq!(none[1].label, "Reasoning");
        assert_eq!(
            values(&none[1].choices),
            ["", "low"],
            "first model's levels, after Default"
        );
        assert_eq!(none[2].label, "Sandbox");

        let sel = HashMap::from([("model".to_string(), "b".to_string())]);
        let b = controls_for("codex", &caps, &sel);
        assert_eq!(ids(&b), ["model", "effort", "tier", "mode"]);
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
        let caps = claude("claude");
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
        let caps = codex("codex");
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
        let caps = opencode("opencode");
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
