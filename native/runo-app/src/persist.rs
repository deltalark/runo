//! How projects and sessions are kept on disk: an index in `state.json` and one file per
//! session at `sessions/<id>/session.json`. A background thread writes them, a moment
//! after the last save and only the files whose contents changed.

use crate::store::{Session, State};
use serde::Serialize;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Quiet time after a save before the files are written.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// A steady stream of saves is still written at least this often.
const MAX_DELAY: Duration = Duration::from_secs(2);

/// The folder holding a session's file and the output of its running turn.
pub fn session_dir(root: &Path, id: u64) -> PathBuf {
    root.join("sessions").join(id.to_string())
}

fn session_file(root: &Path, id: u64) -> PathBuf {
    session_dir(root, id).join("session.json")
}

/// JSON as Runo writes it, with object keys sorted so unchanged data gives the same bytes.
fn render<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .ok()
}

fn digest(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Write `text` atomically and readable only by the person (`store::write_private`).
fn write_text(path: &Path, text: &str) -> bool {
    match crate::store::write_private(path, text.as_bytes()) {
        Ok(()) => true,
        Err(e) => {
            // Saves are retried every half second; the same failure is written once.
            crate::applog::error_once("save", format!("could not save {}: {e}", path.display()));
            false
        }
    }
}

/// What was last written for one data folder: a digest per session file, and the index's.
#[derive(Default)]
pub struct Written {
    sessions: HashMap<u64, u64>,
    index: u64,
}

/// The index file: everything but the sessions, plus the ids of the session files.
fn index_text(state: &mut State) -> Option<String> {
    let sessions = std::mem::take(&mut state.sessions);
    let ids: Vec<u64> = sessions.iter().map(|s| s.id).collect();
    let value = serde_json::to_value(&*state);
    state.sessions = sessions;
    let mut value = value.ok()?;
    value["session_files"] = serde_json::json!(ids);
    serde_json::to_string_pretty(&value).ok()
}

/// Write every session whose file changed, then the index, then remove the files of
/// sessions that are gone. Sessions go first, so a new one is never listed before its
/// file exists.
pub fn write_all(root: &Path, state: &mut State, written: &mut Written) {
    for session in &state.sessions {
        let Some(text) = render(session) else {
            continue;
        };
        let hash = digest(&text);
        if written.sessions.get(&session.id) == Some(&hash) {
            continue;
        }
        if write_text(&session_file(root, session.id), &text) {
            written.sessions.insert(session.id, hash);
        }
    }
    if let Some(text) = index_text(state) {
        let hash = digest(&text);
        if written.index != hash && write_text(&root.join("state.json"), &text) {
            written.index = hash;
        }
    }
    let gone: Vec<u64> = written
        .sessions
        .keys()
        .copied()
        .filter(|id| !state.sessions.iter().any(|s| s.id == *id))
        .collect();
    for id in gone {
        written.sessions.remove(&id);
        remove_session(root, id);
    }
}

/// Delete a session's folder, unless a turn there is still running; then only its file.
fn remove_session(root: &Path, id: u64) {
    let _ = std::fs::remove_file(session_file(root, id));
    if crate::detach::left_behind(root, id) != Some(crate::detach::Left::Running) {
        let _ = std::fs::remove_dir_all(session_dir(root, id));
    }
}

/// Read the state at `index` (and the session files next to it) without changing
/// anything on disk.
pub fn read(index: &Path) -> Result<(State, Written), String> {
    let Ok(text) = std::fs::read_to_string(index) else {
        return Ok((State::default(), Written::default()));
    };
    let mut value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let listed: Vec<u64> = value
        .as_object_mut()
        .and_then(|o| o.remove("session_files"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let mut state: State = serde_json::from_value(value).map_err(|e| e.to_string())?;
    let mut written = Written {
        index: digest(&text),
        ..Default::default()
    };
    let root = index.parent().unwrap_or(Path::new("."));
    // A session written just before Runo stopped may be missing from the index. Ids up to
    // `next_id` that aren't listed were deleted.
    let mut ids = listed.clone();
    if let Ok(entries) = std::fs::read_dir(root.join("sessions")) {
        let mut newer: Vec<u64> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .filter(|id| *id > state.next_id && !listed.contains(id))
            .filter(|id| session_file(root, *id).is_file())
            .collect();
        newer.sort_unstable();
        state.next_id = newer.last().copied().unwrap_or(state.next_id);
        ids.extend(newer);
    }
    for id in ids {
        let path = session_file(root, id);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<Session>(&text) {
            Ok(session) => {
                written.sessions.insert(id, digest(&text));
                state.sessions.push(session);
            }
            Err(error) => crate::store::move_aside(&path, &error.to_string()),
        }
    }
    Ok((state, written))
}

enum Job {
    /// Start from what is on disk for this folder, as `read` found it.
    Seed(PathBuf, Written),
    Save(PathBuf, Box<State>),
    /// Write what is waiting now, then answer.
    Flush(Sender<()>),
}

fn writer() -> &'static Mutex<Sender<Job>> {
    static WRITER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    WRITER.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("runo-save".into())
            .spawn(move || run_writer(rx))
            .expect("could not start the save thread");
        Mutex::new(tx)
    })
}

fn send(job: Job) -> bool {
    writer().lock().is_ok_and(|tx| tx.send(job).is_ok())
}

fn run_writer(rx: Receiver<Job>) {
    let mut written: HashMap<PathBuf, Written> = HashMap::new();
    let mut waiting: Option<(PathBuf, Box<State>, Instant)> = None;
    loop {
        let job = match &waiting {
            Some((.., since)) => {
                let left = MAX_DELAY.saturating_sub(since.elapsed()).min(DEBOUNCE);
                match rx.recv_timeout(left) {
                    Ok(job) => Some(job),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            None => match rx.recv() {
                Ok(job) => Some(job),
                Err(_) => return,
            },
        };
        match job {
            Some(Job::Seed(root, seen)) => {
                written.insert(root, seen);
            }
            Some(Job::Save(root, state)) => {
                let since = waiting.take().map_or_else(Instant::now, |w| w.2);
                waiting = Some((root, state, since));
                if since.elapsed() < MAX_DELAY {
                    continue;
                }
            }
            Some(Job::Flush(done)) => {
                if let Some((root, mut state, _)) = waiting.take() {
                    write_all(&root, &mut state, written.entry(root.clone()).or_default());
                }
                let _ = done.send(());
                continue;
            }
            None => {}
        }
        if let Some((root, mut state, _)) = waiting.take() {
            write_all(&root, &mut state, written.entry(root.clone()).or_default());
        }
    }
}

/// Tell the writer what is on disk for `root`, so unchanged files aren't written again.
pub fn seed(root: &Path, written: Written) {
    send(Job::Seed(root.to_path_buf(), written));
}

/// Queue `state` to be written to `root` shortly, on the save thread.
pub fn queue(root: &Path, state: State) {
    send(Job::Save(root.to_path_buf(), Box::new(state)));
}

/// Write anything still waiting, giving up after `limit`.
pub fn flush(limit: Duration) {
    let (tx, rx) = channel();
    if send(Job::Flush(tx)) {
        let _ = rx.recv_timeout(limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Message, Role};

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-persist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn modified(path: &Path) -> std::time::SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }

    #[test]
    fn sessions_get_their_own_files_and_only_changes_are_written() {
        let root = temp("split");
        let mut state = State::sample();
        state.sessions[0].opts.insert("b".into(), "2".into());
        state.sessions[0].opts.insert("a".into(), "1".into());
        let mut written = Written::default();
        write_all(&root, &mut state, &mut written);
        let first = session_file(&root, state.sessions[0].id);
        let second = session_file(&root, state.sessions[1].id);
        assert!(first.is_file() && second.is_file());
        let index = std::fs::read_to_string(root.join("state.json")).unwrap();
        assert!(!index.contains("\"messages\""));
        let before = (modified(&first), modified(&second));
        std::thread::sleep(Duration::from_millis(20));
        state.sessions[1]
            .messages
            .push(Message::new(Role::User, "more"));
        write_all(&root, &mut state, &mut written);
        assert_eq!(modified(&first), before.0);
        assert!(modified(&second) > before.1);
        // Reading it back gives the same sessions in the same order, and knows the files.
        let (loaded, seen) = read(&root.join("state.json")).unwrap();
        let ids: Vec<u64> = loaded.sessions.iter().map(|s| s.id).collect();
        assert_eq!(ids, state.sessions.iter().map(|s| s.id).collect::<Vec<_>>());
        assert_eq!(loaded.sessions[1].messages.last().unwrap().text, "more");
        assert_eq!(seen.sessions, written.sessions);
        assert_eq!(seen.index, written.index);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deleted_sessions_lose_their_files() {
        let root = temp("delete");
        let mut state = State::sample();
        let mut written = Written::default();
        write_all(&root, &mut state, &mut written);
        let gone = state.sessions.remove(0).id;
        write_all(&root, &mut state, &mut written);
        assert!(!session_dir(&root, gone).exists());
        let (loaded, ..) = read(&root.join("state.json")).unwrap();
        assert!(loaded.session(gone).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_session_missing_from_the_index_is_still_found() {
        let root = temp("orphan");
        let mut state = State::sample();
        let mut written = Written::default();
        write_all(&root, &mut state, &mut written);
        // A new session's file was written, but Runo stopped before the index was.
        let mut extra = state.sessions[0].clone();
        extra.id = state.next_id + 5;
        write_text(&session_file(&root, extra.id), &render(&extra).unwrap());
        // A deleted session's file left behind is not.
        let mut stale = state.sessions[0].clone();
        stale.id = 1;
        write_text(&session_file(&root, stale.id), &render(&stale).unwrap());
        let (loaded, ..) = read(&root.join("state.json")).unwrap();
        assert!(loaded.session(extra.id).is_some());
        assert!(loaded.next_id >= extra.id);
        assert!(loaded.session(1).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_writer_waits_for_quiet_then_writes() {
        let root = temp("writer");
        let mut state = State::sample();
        queue(&root, State::sample());
        state.sessions[0].title = "Latest".into();
        queue(&root, state);
        flush(Duration::from_secs(5));
        let (loaded, ..) = read(&root.join("state.json")).unwrap();
        assert_eq!(loaded.sessions[0].title, "Latest");
        let _ = std::fs::remove_dir_all(root);
    }
}
