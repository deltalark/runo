//! Why a turn failed when the agent itself is the problem (signed out, a model the account
//! can't use, a model that needs a newer CLI), and a Test that tries each model with a
//! tiny prompt so the person can see which ones work before picking one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// What stopped a turn, when it is something the person can fix from Runo.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    SignedOut,
    /// The account can't use this model ("" when the error didn't name it).
    NoAccess(String),
    /// The installed CLI is too old for this model.
    NeedsNewer(String),
}

/// Read a CLI's error output. `version` is the installed CLI's; `client_version` is the
/// release that wrote its model list. A model the CLI rejects while a newer release
/// listed it is a version problem, not an account one.
pub fn classify(provider: &str, log: &str, version: &str, client_version: &str) -> Option<Problem> {
    let lower = log.to_lowercase();
    let model = model_in(log);
    let older = crate::alerts::is_newer(client_version, version);
    if lower.contains("requires a newer version") {
        return Some(Problem::NeedsNewer(model));
    }
    if lower.contains("model") && lower.contains("is not supported when using codex") {
        return Some(if older {
            Problem::NeedsNewer(model)
        } else {
            Problem::NoAccess(model)
        });
    }
    if lower.contains("isn't signed in") || crate::agent::sign_in_hint(provider, log).is_some() {
        return Some(Problem::SignedOut);
    }
    if crate::agent::model_hint(provider, log).is_some() {
        // Codex warns of a model it has no metadata for when the model is newer than it.
        return Some(if older && lower.contains("model metadata") {
            Problem::NeedsNewer(model)
        } else {
            Problem::NoAccess(model)
        });
    }
    None
}

/// The model an error names, as in "The 'gpt-6-sol' model is not supported".
pub fn model_in(log: &str) -> String {
    for quote in ['\'', '`', '"'] {
        let parts: Vec<&str> = log.split(quote).collect();
        for (i, part) in parts.iter().enumerate() {
            let near = |j: usize| {
                parts
                    .get(j)
                    .is_some_and(|p| p.to_lowercase().contains("model"))
            };
            let looks = !part.is_empty()
                && part.len() < 80
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._/:[]".contains(c))
                && part.chars().any(|c| c.is_ascii_alphabetic());
            if i % 2 == 1 && looks && (near(i - 1) || near(i + 1)) {
                return part.to_string();
            }
        }
    }
    String::new()
}

/// The tip added under a failed turn's error, for a problem with the model.
pub fn tip(provider: &str, log: &str) -> Option<String> {
    let caps = crate::caps::get(provider);
    let name = crate::agent::provider(provider).name;
    match classify(provider, log, &caps.version, &caps.client_version)? {
        Problem::NeedsNewer(model) => {
            let model = if model.is_empty() {
                "this model".to_string()
            } else {
                model
            };
            let have = crate::caps::short_version(&caps.version);
            let known = if caps.client_version.is_empty() {
                String::new()
            } else {
                format!(
                    " {name} {} knows it.",
                    crate::caps::short_version(&caps.client_version)
                )
            };
            Some(format!(
                "Tip: {name} {have} is too old for {model}.{known} Update {name}, switch to a newer copy, or pick another model."
            ))
        }
        Problem::NoAccess(_) => crate::agent::model_hint(provider, log),
        Problem::SignedOut => None,
    }
}

/// How a model did when tested.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Verdict {
    Testing,
    Works,
    NoAccess,
    NeedsNewer,
    SignedOut,
    Failed,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Testing => "Testing…",
            Verdict::Works => "Works",
            Verdict::NoAccess => "Not available on your account",
            Verdict::NeedsNewer => "Needs a newer CLI",
            Verdict::SignedOut => "Signed out",
            Verdict::Failed => "Didn't answer",
        }
    }
}

/// Test results of one agent, for the CLI version they were made with.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Tested {
    version: String,
    models: HashMap<String, Verdict>,
}

static RESULTS: RwLock<Option<HashMap<String, Tested>>> = RwLock::new(None);

fn results_path() -> PathBuf {
    crate::store::data_dir().join("model-tests.json")
}

fn with_results<T>(f: impl FnOnce(&mut HashMap<String, Tested>) -> T) -> Option<T> {
    let mut lock = RESULTS.write().ok()?;
    let map = lock.get_or_insert_with(|| {
        std::fs::read_to_string(results_path())
            .ok()
            .and_then(|t| serde_json::from_str::<HashMap<String, Tested>>(&t).ok())
            .map(|mut map| {
                // A test cut short when Runo closed tells nothing.
                for tested in map.values_mut() {
                    tested.models.retain(|_, v| *v != Verdict::Testing);
                }
                map
            })
            .unwrap_or_default()
    });
    Some(f(map))
}

/// How `model` did when last tested with the agent's current CLI.
pub fn verdict(provider: &str, model: &str) -> Option<Verdict> {
    let version = crate::caps::version(provider);
    with_results(|map| {
        map.get(provider)
            .filter(|t| t.version == version)
            .and_then(|t| t.models.get(model).copied())
    })
    .flatten()
}

fn record(provider: &str, version: &str, model: &str, verdict: Verdict) {
    let saved = with_results(|map| {
        let tested = map.entry(provider.to_string()).or_default();
        if tested.version != version {
            *tested = Tested {
                version: version.to_string(),
                models: HashMap::new(),
            };
        }
        tested.models.insert(model.to_string(), verdict);
        serde_json::to_string_pretty(map).ok()
    })
    .flatten();
    if let Some(text) = saved {
        let _ = std::fs::create_dir_all(crate::store::data_dir());
        let _ = std::fs::write(results_path(), text);
    }
    crate::caps::mark_updated();
}

/// Whether a test of this agent's models is running.
pub fn testing(provider: &str) -> bool {
    with_results(|map| {
        map.get(provider)
            .is_some_and(|t| t.models.values().any(|v| *v == Verdict::Testing))
    })
    .unwrap_or(false)
}

/// A model other than `except` that tested as working, best first in the CLI's order.
pub fn working(provider: &str, except: &str) -> Option<String> {
    let caps = crate::caps::get(provider);
    let found = crate::caps::all_models(&caps)
        .map(|m| m.choice.value.clone())
        .find(|m| m != except && verdict(provider, m) == Some(Verdict::Works));
    found
}

/// A model worth trying instead of `except`: one that tested as working, else the first
/// listed one that this CLI is new enough for and that hasn't failed a test.
pub fn alternative(provider: &str, except: &str) -> Option<String> {
    if let Some(m) = working(provider, except) {
        return Some(m);
    }
    let caps = crate::caps::get(provider);
    caps.models
        .iter()
        .filter(|m| m.choice.value != except)
        .filter(|m| m.needs.is_empty() || !crate::alerts::is_newer(&m.needs, &caps.version))
        .find(|m| verdict(provider, &m.choice.value).is_none())
        .map(|m| m.choice.value.clone())
}

/// The flag an agent CLI takes a model with.
fn model_flag(provider: &str) -> &'static str {
    match provider {
        "claude" | "copilot" => "--model",
        _ => "-m",
    }
}

/// Whether Runo can test this agent's models one by one.
pub fn can_test(provider: &str) -> bool {
    provider != "amp"
}

const PROMPT: &str = "Reply with the single word OK.";

/// A one-shot run of `bin` with `model` and a tiny prompt, changing nothing.
fn test_command(provider: &str, bin: &str, model: &str) -> Command {
    let mut command = Command::new(bin);
    let flag = model_flag(provider);
    match provider {
        "codex" => {
            command.args([
                "exec",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                flag,
                model,
                PROMPT,
            ]);
        }
        "claude" => {
            command.args([
                "-p",
                "--no-session-persistence",
                "--permission-mode",
                "plan",
                flag,
                model,
                "--",
                PROMPT,
            ]);
        }
        "opencode" => {
            command.args(["run", flag, model, PROMPT]);
        }
        "cursor" => {
            command.args(["-p", "--output-format", "text"]);
            command.args(crate::agent::cursor_trust());
            command.args(crate::agent::cursor_mode("ask"));
            command.args([flag, model, PROMPT]);
        }
        "droid" => {
            command.args(["exec", flag, model, PROMPT]);
        }
        _ => {
            command.args([flag, model, "-p", PROMPT]);
        }
    }
    command
}

/// What a test run's output says about the model.
pub fn judge(ok: bool, output: &str, provider: &str, version: &str, client: &str) -> Verdict {
    match classify(provider, output, version, client) {
        Some(Problem::NeedsNewer(_)) => Verdict::NeedsNewer,
        Some(Problem::NoAccess(_)) => Verdict::NoAccess,
        Some(Problem::SignedOut) => Verdict::SignedOut,
        None if ok => Verdict::Works,
        None => Verdict::Failed,
    }
}

/// Test each model in the background, one after another, with the CLI Runo runs.
pub fn test_all(provider: &'static crate::agent::Provider, models: Vec<String>, env: String) {
    if testing(provider.id) {
        return;
    }
    let caps = crate::caps::get(provider.id);
    let bin = crate::caps::binary(provider);
    for model in &models {
        record(provider.id, &caps.version, model, Verdict::Testing);
    }
    std::thread::spawn(move || {
        for model in models {
            let mut command = test_command(provider.id, &bin, &model);
            for line in env.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    command.env(k.trim(), v.trim());
                }
            }
            let (ok, out) = run(command, Duration::from_secs(90)).unwrap_or((false, String::new()));
            let verdict = judge(ok, &out, provider.id, &caps.version, &caps.client_version);
            record(provider.id, &caps.version, &model, verdict);
        }
    });
}

/// Run a command in a temporary folder for up to `limit`: whether it succeeded, and its
/// output and errors.
fn run(mut command: Command, limit: Duration) -> Option<(bool, String)> {
    use std::io::Read;
    let mut child = command
        .current_dir(std::env::temp_dir())
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let (mut out, mut err) = (child.stdout.take()?, child.stderr.take()?);
    let read_err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = err.read_to_string(&mut text);
        text
    });
    let read_out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.read_to_string(&mut text);
        text
    });
    let start = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(200)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let mut text = read_out.join().unwrap_or_default();
    text.push('\n');
    text.push_str(&read_err.join().unwrap_or_default());
    Some((ok, text))
}

/// Codex's config file: $CODEX_HOME/config.toml, else ~/.codex/config.toml.
pub fn codex_config() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home).join("config.toml"));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".codex").join("config.toml"))
}

/// The model Codex's config file picks, when it picks one.
pub fn codex_config_model() -> Option<String> {
    let text = std::fs::read_to_string(codex_config()?).ok()?;
    let table: toml::Table = text.parse().ok()?;
    table.get("model")?.as_str().map(str::to_string)
}

/// `text` with its top-level `model` key set to `model`, and nothing else changed. The
/// key is added before the first table when it isn't there.
pub fn set_toml_model(text: &str, model: &str) -> String {
    let value = format!(
        "model = \"{}\"",
        model.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut first_table = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with('[') {
            first_table = Some(i);
            break;
        }
        let key = t.split('=').next().unwrap_or("").trim();
        if t.contains('=') && (key == "model" || key == "\"model\"") {
            lines[i] = value;
            return join(lines, text);
        }
    }
    match first_table {
        Some(i) => {
            lines.insert(i, value);
            if i > 0 || !lines.get(i + 1).is_some_and(|l| l.trim().is_empty()) {
                lines.insert(i + 1, String::new());
            }
        }
        None => lines.insert(0, value),
    }
    join(lines, text)
}

fn join(lines: Vec<String>, original: &str) -> String {
    let mut out = lines.join("\n");
    if original.ends_with('\n') || original.is_empty() {
        out.push('\n');
    }
    out
}

/// Set the model in Codex's config file, after copying the file to a backup beside it.
/// Returns the backup's path, or None when there was no file yet.
pub fn save_codex_model(model: &str) -> Result<Option<PathBuf>, String> {
    let path = codex_config().ok_or("Couldn't find your home folder.")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let backup = if path.is_file() {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let backup = path.with_file_name(format!("config.toml.runo-{stamp}.bak"));
        std::fs::copy(&path, &backup)
            .map_err(|e| format!("Couldn't back up {}: {e}", path.display()))?;
        Some(backup)
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        None
    };
    std::fs::write(&path, set_toml_model(&text, model))
        .map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = "ERROR: unexpected status 400 Bad Request: {\"detail\":\"The 'gpt-6-sol' model is not supported when using Codex with a ChatGPT account.\"}";
    const NEWER: &str = "ERROR: {\"detail\":\"The 'gpt-6-astra' model requires a newer version of Codex. Please upgrade to the latest app or CLI and try again.\"}";

    #[test]
    fn errors_name_the_model() {
        assert_eq!(model_in(ACCOUNT), "gpt-6-sol");
        assert_eq!(model_in(NEWER), "gpt-6-astra");
        assert_eq!(model_in("Model metadata for `gpt-x` not found."), "gpt-x");
        assert_eq!(model_in("it's broken, isn't it"), "");
    }

    #[test]
    fn classifies_model_version_and_sign_in_errors() {
        // An old CLI with a model list written by a newer one: a version problem.
        assert_eq!(
            classify("codex", ACCOUNT, "0.147.0", "0.162.1"),
            Some(Problem::NeedsNewer("gpt-6-sol".into()))
        );
        // The CLI is current: the account really can't use it.
        assert_eq!(
            classify("codex", ACCOUNT, "0.162.1", "0.162.1"),
            Some(Problem::NoAccess("gpt-6-sol".into()))
        );
        assert_eq!(
            classify("codex", NEWER, "0.162.1", ""),
            Some(Problem::NeedsNewer("gpt-6-astra".into()))
        );
        assert_eq!(
            classify("codex", "Not logged in", "0.162.1", ""),
            Some(Problem::SignedOut)
        );
        assert_eq!(
            classify("gemini", "Gemini isn't signed in. Open a terminal…", "", ""),
            Some(Problem::SignedOut)
        );
        assert_eq!(
            classify("claude", "model claude-x does not exist", "", ""),
            Some(Problem::NoAccess(String::new()))
        );
        assert_eq!(classify("codex", "file not found", "0.1", "0.2"), None);
    }

    #[test]
    fn tests_read_as_verdicts() {
        assert_eq!(judge(true, "OK\n", "codex", "0.162.1", ""), Verdict::Works);
        assert_eq!(
            judge(false, ACCOUNT, "codex", "0.162.1", "0.162.1"),
            Verdict::NoAccess
        );
        assert_eq!(
            judge(false, ACCOUNT, "codex", "0.147.0", "0.162.1"),
            Verdict::NeedsNewer
        );
        assert_eq!(
            judge(false, NEWER, "codex", "0.147.0", ""),
            Verdict::NeedsNewer
        );
        assert_eq!(judge(false, "boom", "codex", "", ""), Verdict::Failed);
        let args: Vec<String> = test_command("codex", "codex", "gpt-5.6-sol")
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w == ["-m", "gpt-5.6-sol"]));
        assert!(args.contains(&"read-only".to_string()));
    }

    #[test]
    fn config_edit_changes_only_the_model_key() {
        let text = "# mine\nmodel = \"gpt-6-sol\" # pinned\nmodel_reasoning_effort = \"high\"\n\n[profiles.x]\nmodel = \"other\"\n";
        assert_eq!(
            set_toml_model(text, "gpt-5.6-sol"),
            "# mine\nmodel = \"gpt-5.6-sol\"\nmodel_reasoning_effort = \"high\"\n\n[profiles.x]\nmodel = \"other\"\n"
        );
        let no_key = "approval_policy = \"never\"\n[mcp_servers.a]\ncommand = \"x\"\n";
        let set = set_toml_model(no_key, "m");
        assert_eq!(
            set,
            "approval_policy = \"never\"\nmodel = \"m\"\n\n[mcp_servers.a]\ncommand = \"x\"\n"
        );
        assert_eq!(set_toml_model("", "m"), "model = \"m\"\n");
        assert_eq!(
            set_toml_model("[a]\nb = 1", "m"),
            "model = \"m\"\n\n[a]\nb = 1"
        );
        for edited in [set_toml_model(text, "q\"x"), set] {
            assert!(edited.parse::<toml::Table>().is_ok());
        }
    }
}
