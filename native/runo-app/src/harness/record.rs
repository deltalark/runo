//! Recording agents' raw output for fixtures. With `RUNO_RECORD=<folder>` set, every line
//! a structured agent prints in a turn is copied to `<folder>/<agent>-<session>-<turn>.jsonl`
//! as it arrives, untouched, ready to become a contract test under `tests/fixtures` once
//! home folders and accounts are taken out.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

fn folder() -> Option<&'static PathBuf> {
    static FOLDER: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOLDER
        .get_or_init(|| {
            std::env::var_os("RUNO_RECORD")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .as_ref()
}

/// Copy one line of a turn's output, when recording.
pub fn line(provider: &str, session: u64, turn: usize, line: &str) {
    let Some(folder) = folder() else {
        return;
    };
    let path = folder.join(format!("{provider}-{session}-{turn}.jsonl"));
    let _ = std::fs::create_dir_all(folder);
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = written {
        crate::applog::error_once(
            "record",
            format!("could not record to {}: {e}", path.display()),
        );
    }
}
