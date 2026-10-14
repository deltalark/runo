//! Whether each agent CLI is signed in on this computer. The check uses the same API keys
//! and login files as a connection's Test, plus the CLI's own status command where it has
//! one. It runs off the UI thread and is cached, so New session, Settings and a turn about
//! to start can say so up front.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

static STATE: RwLock<Option<HashMap<String, bool>>> = RwLock::new(None);
static CHECKED: Mutex<Option<Instant>> = Mutex::new(None);

/// How long a check stands before opening a page checks again.
const FRESH: Duration = Duration::from_secs(60);

/// Whether the agent is signed in here, once checked; None when Runo can't tell.
pub fn get(provider: &str) -> Option<bool> {
    STATE.read().ok()?.as_ref()?.get(provider).copied()
}

/// Record what a check or a turn showed.
pub fn set(provider: &str, signed_in: bool) {
    let Ok(mut state) = STATE.write() else {
        return;
    };
    let map = state.get_or_insert_with(HashMap::new);
    if map.insert(provider.to_string(), signed_in) != Some(signed_in) {
        crate::caps::mark_updated();
    }
}

fn forget(provider: &str) {
    if let Ok(mut state) = STATE.write() {
        if let Some(map) = state.as_mut() {
            if map.remove(provider).is_some() {
                crate::caps::mark_updated();
            }
        }
    }
}

/// Check every installed agent again in the background; unless `force`, only when the
/// last check is more than a minute old.
pub fn refresh(force: bool) {
    if let Ok(mut checked) = CHECKED.lock() {
        if !force && checked.is_some_and(|at| at.elapsed() < FRESH) {
            return;
        }
        *checked = Some(Instant::now());
    }
    std::thread::spawn(|| {
        let settings = crate::settings::AppSettings::load();
        let Some(home) = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
        else {
            return;
        };
        for p in crate::agent::PROVIDERS.iter() {
            if !crate::caps::get(p.id).installed {
                continue;
            }
            let env = settings.agent(p.id).env;
            let has_var = |key: &str| {
                std::env::var(key).is_ok_and(|v| !v.is_empty())
                    || crate::shellenv::login_var(key).is_some_and(|v| !v.is_empty())
                    || env.lines().any(|line| {
                        line.split_once('=')
                            .is_some_and(|(k, v)| k.trim() == key && !v.trim().is_empty())
                    })
            };
            match judge(p.id, &crate::caps::binary(p), &home, &has_var, &run) {
                Some(signed_in) => set(p.id, signed_in),
                None => forget(p.id),
            }
        }
    });
}

/// Runs a program with arguments: whether it succeeded, and its output.
type Runner<'a> = &'a dyn Fn(&str, &[&str]) -> Option<(bool, String)>;

/// Run a command for up to ten seconds: whether it succeeded, and its output.
fn run(program: &str, args: &[&str]) -> Option<(bool, String)> {
    use std::io::Read;
    let mut child = Command::new(program)
        .args(args)
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let start = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(100))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut out = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut out);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut out);
    }
    Some((ok, out))
}

/// Whether `provider` is signed in, given the home folder, which variables are set and a
/// way to run a program. Says no only when every way the CLI signs in is missing.
fn judge(
    provider: &str,
    bin: &str,
    home: &Path,
    has_var: &dyn Fn(&str) -> bool,
    run: Runner,
) -> Option<bool> {
    let keys = crate::connections::sign_in(provider)
        .map(|s| s.keys)
        .unwrap_or_default();
    let more: &[&str] = match provider {
        "claude" => &[
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
        ],
        "gemini" => &[
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_APPLICATION_CREDENTIALS",
        ],
        _ => &[],
    };
    if keys.iter().chain(more).any(|k| has_var(k)) {
        return Some(true);
    }
    let file = |rel: &str| home.join(rel).is_file();
    let read = |rel: &str| std::fs::read_to_string(home.join(rel)).unwrap_or_default();
    match provider {
        "codex" => {
            let (ok, out) = run(bin, &["login", "status"])?;
            if out.to_lowercase().contains("not logged in") {
                Some(false)
            } else {
                ok.then_some(true)
            }
        }
        "cursor" => {
            let (_, out) = run(bin, &["status"])?;
            let out = out.to_lowercase();
            if out.contains("not logged in") || out.contains("not authenticated") {
                Some(false)
            } else {
                out.contains("logged in").then_some(true)
            }
        }
        "claude" => {
            if file(".claude/.credentials.json")
                || read(".claude/settings.json").contains("apiKeyHelper")
                || read(".claude.json").contains("primaryApiKey")
            {
                return Some(true);
            }
            if cfg!(target_os = "macos") {
                // macOS keeps the login in the keychain.
                let (found, _) = run(
                    "security",
                    &["find-generic-password", "-s", "Claude Code-credentials"],
                )?;
                Some(found)
            } else {
                Some(false)
            }
        }
        "gemini" => {
            if file(".gemini/oauth_creds.json") || file(".gemini/.env") {
                return Some(true);
            }
            // Other ways it signs in, such as Vertex AI or Cloud Shell, aren't checked.
            let settings = read(".gemini/settings.json");
            if settings.contains("vertex") || settings.contains("compute") {
                return None;
            }
            Some(false)
        }
        "opencode" => {
            let auth = read(".local/share/opencode/auth.json");
            let saved = serde_json::from_str::<serde_json::Value>(&auth)
                .ok()
                .and_then(|v| v.as_object().map(|m| !m.is_empty()))
                .unwrap_or(false);
            if saved {
                return Some(true);
            }
            // Providers of its own, such as a local model, may need no sign-in.
            let config =
                read(".config/opencode/opencode.json") + &read(".config/opencode/opencode.jsonc");
            if config.contains("\"provider\"") {
                return None;
            }
            Some(false)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-signin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_key_signs_any_agent_in() {
        let dir = home("key");
        let none = |_: &str, _: &[&str]| None;
        let set = |k: &str| k == "GEMINI_API_KEY";
        assert_eq!(judge("gemini", "gemini", &dir, &set, &none), Some(true));
        assert_eq!(judge("amp", "amp", &dir, &|_| false, &none), None);
    }

    #[test]
    fn codex_and_cursor_ask_their_cli() {
        let dir = home("status");
        let out = |ok: bool, text: &'static str| move |_: &str, _: &[&str]| Some((ok, text.into()));
        let unset = |_: &str| false;
        assert_eq!(
            judge(
                "codex",
                "codex",
                &dir,
                &unset,
                &out(false, "Not logged in\n")
            ),
            Some(false)
        );
        assert_eq!(
            judge(
                "codex",
                "codex",
                &dir,
                &unset,
                &out(true, "Logged in using ChatGPT\n")
            ),
            Some(true)
        );
        assert_eq!(
            judge(
                "cursor",
                "cursor-agent",
                &dir,
                &unset,
                &out(true, "✗ Not logged in")
            ),
            Some(false)
        );
        assert_eq!(
            judge("codex", "codex", &dir, &unset, &|_: &str, _: &[&str]| None),
            None
        );
    }

    #[test]
    fn login_files_count_and_their_absence_means_signed_out() {
        let dir = home("files");
        let none = |_: &str, _: &[&str]| None;
        let unset = |_: &str| false;
        assert_eq!(judge("gemini", "gemini", &dir, &unset, &none), Some(false));
        assert_eq!(
            judge("opencode", "opencode", &dir, &unset, &none),
            Some(false)
        );
        std::fs::create_dir_all(dir.join(".gemini")).unwrap();
        std::fs::write(dir.join(".gemini/oauth_creds.json"), "{}").unwrap();
        assert_eq!(judge("gemini", "gemini", &dir, &unset, &none), Some(true));
        std::fs::create_dir_all(dir.join(".local/share/opencode")).unwrap();
        std::fs::write(dir.join(".local/share/opencode/auth.json"), "{}").unwrap();
        assert_eq!(
            judge("opencode", "opencode", &dir, &unset, &none),
            Some(false)
        );
        std::fs::write(
            dir.join(".local/share/opencode/auth.json"),
            r#"{"anthropic":{"type":"oauth"}}"#,
        )
        .unwrap();
        assert_eq!(
            judge("opencode", "opencode", &dir, &unset, &none),
            Some(true)
        );
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join(".claude/.credentials.json"), "{}").unwrap();
        assert_eq!(judge("claude", "claude", &dir, &unset, &none), Some(true));
    }
}
