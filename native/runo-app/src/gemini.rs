//! Gemini CLI specifics. Gemini only applies an approval mode in folders it trusts, and
//! a new worktree isn't trusted yet, so Runo trusts the folder it runs Gemini in.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

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
                .map(|home| {
                    PathBuf::from(home)
                        .join(".gemini")
                        .join("trustedFolders.json")
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_trusted_once_and_distrust_is_kept() {
        let root = std::env::temp_dir().join(format!("runo-gemini-trust-{}", std::process::id()));
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
