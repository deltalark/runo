//! Gemini CLI specifics. Gemini only applies an approval mode in folders it trusts, and
//! a new worktree isn't trusted yet, so Runo trusts the folder it runs Gemini in. Its
//! `--resume` takes "latest" or the session's number in `gemini --list-sessions`, so
//! Runo learns those numbers after each turn.

use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Session numbers in `gemini --list-sessions`, by session id.
static NUMBERS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Gemini's list of trusted folders: `GEMINI_CLI_TRUSTED_FOLDERS_PATH` when set (in the
/// agent's environment from Settings first), else `~/.gemini/trustedFolders.json`.
fn trust_file(env: &str) -> Option<PathBuf> {
    const KEY: &str = "GEMINI_CLI_TRUSTED_FOLDERS_PATH";
    let from_settings = env.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == KEY && !value.trim().is_empty()).then(|| PathBuf::from(value.trim()))
    });
    from_settings
        .or_else(|| std::env::var_os(KEY).map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".gemini").join("trustedFolders.json"))
        })
}

/// Trust `dir` for Gemini, so the session's mode applies there. A folder the person
/// marked untrusted stays so, and a file Runo can't read as JSON is left alone.
pub fn trust(dir: &str, env: &str) {
    if let Some(file) = trust_file(env) {
        let _ = trust_in(&file, Path::new(dir));
    }
}

fn trust_in(file: &Path, dir: &Path) -> Option<()> {
    let mut folders: Map<String, Value> = match std::fs::read_to_string(file) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).ok()?,
        Ok(_) => Map::new(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(_) => return None,
    };
    // Gemini compares the real path (on a Mac, /var is /private/var).
    let mut paths = vec![dir.to_string_lossy().into_owned()];
    if let Ok(real) = std::fs::canonicalize(dir) {
        let real = real.to_string_lossy().into_owned();
        if !paths.contains(&real) {
            paths.push(real);
        }
    }
    let mut changed = false;
    for path in paths {
        if !folders.contains_key(&path) {
            folders.insert(path, Value::String("TRUST_FOLDER".into()));
            changed = true;
        }
    }
    if !changed {
        return Some(());
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(folders)).ok()?;
    let tmp = file.with_extension("json.runo-tmp");
    std::fs::write(&tmp, text).ok()?;
    std::fs::rename(&tmp, file).ok()
}

/// What to pass to `--resume` for session `id`: its number when known, else "latest",
/// which is the session's own conversation in a worktree of its own.
pub fn resume_arg(id: &str) -> String {
    NUMBERS
        .lock()
        .ok()
        .and_then(|map| map.as_ref()?.get(id).cloned())
        .unwrap_or_else(|| "latest".into())
}

/// Learn the session numbers in `dir` from `gemini --list-sessions`, in the background.
pub fn learn_sessions(bin: String, dir: String) {
    let mut command = std::process::Command::new(bin);
    command.arg("--list-sessions");
    let (tx, rx) = std::sync::mpsc::channel();
    // A CLI that hangs (waiting for a sign-in, say) is stopped after a while.
    crate::agent::oneshot_within(
        "gemini-sessions".into(),
        command,
        &dir,
        std::time::Duration::from_secs(30),
        tx,
    );
    std::thread::spawn(move || {
        let Ok(crate::agent::Event::Output(_, result)) = rx.recv() else {
            return;
        };
        let text = result.unwrap_or_else(|error| error);
        let found = parse_sessions(&text);
        if let Ok(mut map) = NUMBERS.lock() {
            map.get_or_insert_with(HashMap::new).extend(found);
        }
    });
}

/// (id, number) pairs from `--list-sessions`, whose lines read like
/// "  3. Fix the login test (2 hours ago) [0f9c…-…]".
fn parse_sessions(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (number, rest) = line.split_once('.')?;
            if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let id = rest
                .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
                .rev()
                .find(|w| is_uuid(w))?;
            Some((id.to_string(), number.to_string()))
        })
        .collect()
}

fn is_uuid(word: &str) -> bool {
    let parts: Vec<&str> = word.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sessions_lines_give_numbers() {
        let text = "Available sessions for this project (2):\n  1. Fix the login test (2 hours ago) [0f9c1a2b-1111-4222-8333-444455556666]\n  2. Add a README (Just now) [aaaaaaaa-bbbb-4ccc-8ddd-eeeeffff0000]\n";
        assert_eq!(
            parse_sessions(text),
            [
                (
                    "0f9c1a2b-1111-4222-8333-444455556666".to_string(),
                    "1".to_string()
                ),
                (
                    "aaaaaaaa-bbbb-4ccc-8ddd-eeeeffff0000".to_string(),
                    "2".to_string()
                ),
            ]
        );
        assert!(parse_sessions("No previous sessions found.").is_empty());
        if let Ok(mut map) = NUMBERS.lock() {
            map.get_or_insert_with(HashMap::new)
                .extend(parse_sessions(text));
        }
        assert_eq!(resume_arg("aaaaaaaa-bbbb-4ccc-8ddd-eeeeffff0000"), "2");
        assert_eq!(resume_arg("unknown"), "latest");
    }

    #[test]
    fn folders_are_trusted_once_and_distrust_is_kept() {
        let root =
            std::env::temp_dir().join(format!("runo-gemini-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("wt");
        std::fs::create_dir_all(&dir).unwrap();
        let file = root.join(".gemini").join("trustedFolders.json");
        trust_in(&file, &dir).unwrap();
        let real = std::fs::canonicalize(&dir).unwrap();
        let read = || -> Map<String, Value> {
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap()
        };
        assert_eq!(read()[&*real.to_string_lossy()], "TRUST_FOLDER");
        // The person's own choice for a folder stays.
        let mut folders = read();
        folders.insert(real.to_string_lossy().into_owned(), "DO_NOT_TRUST".into());
        std::fs::write(&file, serde_json::to_string(&folders).unwrap()).unwrap();
        trust_in(&file, &dir).unwrap();
        assert_eq!(read()[&*real.to_string_lossy()], "DO_NOT_TRUST");
        // A file that isn't plain JSON is left as it is.
        std::fs::write(&file, "{ // comment\n}").unwrap();
        assert!(trust_in(&file, &dir).is_none());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ // comment\n}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
