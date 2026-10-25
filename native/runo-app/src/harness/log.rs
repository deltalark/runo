//! Each session's event log, `sessions/<id>/events.jsonl`: one JSON line per event of
//! every turn, so a turn can be replayed, audited or picked up by another agent without
//! re-reading the CLI's own output. Streamed text is gathered into one line per run of
//! text. A turn picked up after Runo restarts replays its output from the start; events
//! already in the log are not written again.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use super::event::{tail, AgentEvent};

/// The log is set aside as `events.1.jsonl` when a turn starts with it this big.
const ROTATE_AT: u64 = 20 * 1024 * 1024;
/// Text gathered beyond this is written out even while more streams in.
const TEXT_CHUNK: usize = 8 * 1024;
/// A tool's output is kept to its last this many bytes.
const OUTPUT_TAIL: usize = 4 * 1024;

static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Keep turns' events under `root` (Runo's data folder). Only the app does this, so tests
/// and one-off runs keep no log.
pub fn enable(root: PathBuf) {
    let _ = ROOT.set(root);
}

pub fn root() -> Option<&'static Path> {
    ROOT.get().map(PathBuf::as_path)
}

pub fn path(root: &Path, session: u64) -> PathBuf {
    crate::persist::session_dir(root, session).join("events.jsonl")
}

/// One line of the log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Milliseconds since the Unix epoch.
    pub t: u64,
    /// The turn: the index of its prompt in the session's messages.
    pub turn: usize,
    /// The event's number within the turn, from 0. A line of gathered text carries the
    /// number of its last piece.
    pub seq: u64,
    pub event: AgentEvent,
}

/// The log of one running turn.
pub struct TurnLog {
    path: Option<PathBuf>,
    turn: usize,
    /// The number the next event gets.
    seq: u64,
    /// Events up to this number are already in the log (a replayed turn).
    written: Option<u64>,
    text: String,
    text_seq: u64,
}

impl TurnLog {
    /// A log that keeps nothing, for runs Runo doesn't keep, such as a phone preview.
    pub fn off() -> TurnLog {
        TurnLog::new(None, 0, 0, None)
    }

    fn new(path: Option<PathBuf>, turn: usize, seq: u64, written: Option<u64>) -> TurnLog {
        TurnLog {
            path,
            turn,
            seq,
            written,
            text: String::new(),
            text_seq: 0,
        }
    }

    /// The log for a new turn of `session` whose prompt is message `turn`. A big log is
    /// set aside first, so a turn never spans two files.
    pub fn start(
        root: Option<&Path>,
        session: u64,
        turn: usize,
        provider: &str,
        model: &str,
    ) -> TurnLog {
        let Some(root) = root else {
            return TurnLog::off();
        };
        let path = path(root, session);
        // Set aside on the writer thread, after what's already on its way, so a turn's
        // start never waits on the disk.
        let _ = writer().send(Job::Start(path.clone()));
        let mut log = TurnLog::new(Some(path), turn, 0, None);
        log.record(&AgentEvent::TurnStarted {
            provider: provider.to_string(),
            model: model.to_string(),
        });
        log
    }

    /// The log for a turn picked up again, whose output replays from the start: what the
    /// log already holds of it is skipped as it comes round again.
    pub fn resume(root: Option<&Path>, session: u64, turn: usize) -> TurnLog {
        let Some(root) = root else {
            return TurnLog::off();
        };
        let path = path(root, session);
        let written = last_seq(&path, turn);
        // The replay doesn't repeat the start, which was logged when the turn began.
        TurnLog::new(Some(path), turn, 1, written)
    }

    /// The turn: the index of its prompt in the session's messages.
    pub fn turn(&self) -> usize {
        self.turn
    }

    /// Add an event of the turn.
    pub fn record(&mut self, event: &AgentEvent) {
        let seq = self.seq;
        self.seq += 1;
        if self.path.is_none() || self.written.is_some_and(|w| seq <= w) {
            return;
        }
        if let AgentEvent::Text(text) = event {
            self.text.push_str(text);
            self.text_seq = seq;
            if self.text.len() >= TEXT_CHUNK {
                self.flush();
            }
            return;
        }
        self.flush();
        let event = match event {
            AgentEvent::ToolDone { id, ok, output } if output.len() > OUTPUT_TAIL => {
                AgentEvent::ToolDone {
                    id: id.clone(),
                    ok: *ok,
                    output: tail(output, OUTPUT_TAIL),
                }
            }
            other => other.clone(),
        };
        self.write(seq, event);
    }

    /// The turn ended: what's gathered is written, then the end.
    pub fn end(&mut self, ok: bool) {
        self.record(&AgentEvent::TurnEnded { ok });
    }

    /// Write out gathered text.
    pub fn flush(&mut self) {
        if !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            self.write(self.text_seq, AgentEvent::Text(text));
        }
    }

    fn write(&mut self, seq: u64, event: AgentEvent) {
        let Some(path) = &self.path else {
            return;
        };
        let record = Record {
            t: now_ms(),
            turn: self.turn,
            seq,
            event,
        };
        let Ok(mut line) = serde_json::to_string(&record) else {
            return;
        };
        line.push('\n');
        append(path.clone(), line);
    }
}

impl Drop for TurnLog {
    fn drop(&mut self) {
        self.flush();
        if let Some(path) = self.path.take() {
            let _ = writer().send(Job::Close(path));
        }
    }
}

/// What the log writer is asked to do.
enum Job {
    Append(PathBuf, String),
    /// A turn's log is done with: close its file.
    Close(PathBuf),
    /// A turn starts in this log: close it, try writing it again if it failed, and set
    /// it aside when it is big.
    Start(PathBuf),
    /// Reply once everything sent before is written, with the files closed.
    Drain(std::sync::mpsc::Sender<()>),
}

/// The thread that writes every turn's events, so a busy agent's output never waits on the
/// disk on the way to the screen. A running turn's file stays open between its events. A
/// file that fails to write is left alone until its next turn starts (or the next drain),
/// so one failure costs the rest of that turn's log as it did when writing inline.
fn writer() -> &'static std::sync::mpsc::Sender<Job> {
    static TX: std::sync::OnceLock<std::sync::mpsc::Sender<Job>> = std::sync::OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("runo-events".into())
            .spawn(move || {
                let mut open: std::collections::HashMap<PathBuf, std::fs::File> =
                    Default::default();
                let mut failed = std::collections::HashSet::new();
                for job in rx {
                    match job {
                        Job::Append(path, line) => {
                            if failed.contains(&path) {
                                continue;
                            }
                            let file = match open.entry(path.clone()) {
                                std::collections::hash_map::Entry::Occupied(f) => Ok(f.into_mut()),
                                std::collections::hash_map::Entry::Vacant(v) => {
                                    if let Some(dir) = path.parent() {
                                        let _ = std::fs::create_dir_all(dir);
                                    }
                                    std::fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open(&path)
                                        .map(|f| v.insert(f))
                                }
                            };
                            if let Err(e) = file.and_then(|f| f.write_all(line.as_bytes())) {
                                crate::applog::error_once(
                                    "events",
                                    format!("could not write {}: {e}", path.display()),
                                );
                                // One failure is enough to report; the turn goes on without its log.
                                open.remove(&path);
                                failed.insert(path);
                            }
                        }
                        Job::Close(path) => {
                            open.remove(&path);
                        }
                        Job::Start(path) => {
                            open.remove(&path);
                            failed.remove(&path);
                            rotate(&path);
                        }
                        Job::Drain(done) => {
                            open.clear();
                            failed.clear();
                            let _ = done.send(());
                        }
                    }
                }
            })
            .expect("start the event log writer");
        tx
    })
}

/// Append `line` to the log at `path`, on the writer thread.
fn append(path: PathBuf, line: String) {
    let _ = writer().send(Job::Append(path, line));
}

/// Wait until every event recorded so far is in its file, and close the files: before
/// reading or moving a log, and before Runo quits.
pub fn drain() {
    let (tx, rx) = std::sync::mpsc::channel();
    if writer().send(Job::Drain(tx)).is_ok() {
        let _ = rx.recv();
    }
}

/// Set a big log aside as `events.1.jsonl`, so a turn never spans two files.
fn rotate(path: &Path) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() >= ROTATE_AT) {
        let _ = std::fs::rename(path, path.with_file_name("events.1.jsonl"));
    }
}

/// The highest event number the log holds for `turn`, the log's last turn when it has
/// any events: read from the end, so a big log costs only that turn's lines.
fn last_seq(path: &Path, turn: usize) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    drain();
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut back = 64 * 1024u64;
    loop {
        let from = len.saturating_sub(back);
        file.seek(SeekFrom::Start(from)).ok()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = text.lines().collect();
        // The first line may be cut by where the read began.
        if from > 0 && !lines.is_empty() {
            lines.remove(0);
        }
        let mut best = None;
        let mut other = false;
        for record in lines
            .iter()
            .rev()
            .filter_map(|line| serde_json::from_str::<Record>(line).ok())
        {
            if record.turn != turn {
                other = true;
                break;
            }
            best = best.max(Some(record.seq));
        }
        if other || from == 0 {
            return best;
        }
        back *= 4;
    }
}

/// Every event in a session's log, oldest first (the set-aside file, then the current
/// one). Lines that don't parse, such as one cut short by a crash, are skipped.
pub fn read(root: &Path, session: u64) -> Vec<Record> {
    drain();
    let current = path(root, session);
    [current.with_file_name("events.1.jsonl"), current]
        .iter()
        .filter_map(|p| std::fs::File::open(p).ok())
        .flat_map(|f| {
            std::io::BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter_map(|line| serde_json::from_str::<Record>(&line).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Print a session's log to stdout, for `runo --events`. Exits 1 when it has none.
pub fn print(root: &Path, session: u64) -> i32 {
    let records = read(root, session);
    if records.is_empty() {
        eprintln!("session {session} has no events in {}", root.display());
        return 1;
    }
    let mut out = std::io::stdout().lock();
    for record in records {
        if let Ok(line) = serde_json::to_string(&record) {
            if writeln!(out, "{line}").is_err() {
                break;
            }
        }
    }
    0
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ToolStep;

    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-events-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn events(root: &Path) -> Vec<AgentEvent> {
        read(root, 7).into_iter().map(|r| r.event).collect()
    }

    fn turn(log: &mut TurnLog) {
        log.record(&AgentEvent::Text("Hel".into()));
        log.record(&AgentEvent::Text("lo".into()));
        log.record(&AgentEvent::Tool(ToolStep::running("t1", "Bash", "Ran ls")));
        log.record(&AgentEvent::ToolDone {
            id: "t1".into(),
            ok: true,
            output: String::new(),
        });
        log.record(&AgentEvent::Text("Done.".into()));
    }

    #[test]
    fn a_turn_logs_its_events_with_text_gathered() {
        let dir = root("turn");
        let mut log = TurnLog::start(Some(&dir), 7, 2, "claude", "opus");
        turn(&mut log);
        log.end(true);
        drop(log);
        let records = read(&dir, 7);
        assert_eq!(
            records.iter().map(|r| r.event.clone()).collect::<Vec<_>>(),
            vec![
                AgentEvent::TurnStarted {
                    provider: "claude".into(),
                    model: "opus".into()
                },
                AgentEvent::Text("Hello".into()),
                AgentEvent::Tool(ToolStep::running("t1", "Bash", "Ran ls")),
                AgentEvent::ToolDone {
                    id: "t1".into(),
                    ok: true,
                    output: String::new()
                },
                AgentEvent::Text("Done.".into()),
                AgentEvent::TurnEnded { ok: true },
            ]
        );
        assert!(records.iter().all(|r| r.turn == 2));
        assert_eq!(
            records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            [0, 2, 3, 4, 5, 6]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_replayed_turn_is_not_logged_twice() {
        let dir = root("replay");
        // Runo quit halfway: the start, the text and the tool call were written.
        let mut log = TurnLog::start(Some(&dir), 7, 0, "codex", "");
        log.record(&AgentEvent::Text("Hel".into()));
        log.record(&AgentEvent::Text("lo".into()));
        log.record(&AgentEvent::Tool(ToolStep::running("t1", "Bash", "Ran ls")));
        drop(log);
        // On the next launch the turn's whole output comes round again.
        let mut log = TurnLog::resume(Some(&dir), 7, 0);
        turn(&mut log);
        log.end(true);
        drop(log);
        let got = events(&dir);
        assert_eq!(got.len(), 6, "{got:?}");
        assert_eq!(
            got.iter()
                .filter(|e| matches!(e, AgentEvent::Tool(_)))
                .count(),
            1
        );
        assert_eq!(got[4], AgentEvent::Text("Done.".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_last_turn_is_found_from_the_end_of_a_big_log() {
        let dir = root("big");
        let mut log = TurnLog::start(Some(&dir), 7, 0, "codex", "");
        log.record(&AgentEvent::Text("x".repeat(100_000)));
        drop(log);
        let mut log = TurnLog::start(Some(&dir), 7, 2, "codex", "");
        for _ in 0..40 {
            log.record(&AgentEvent::ToolDone {
                id: "t".into(),
                ok: true,
                output: "y".repeat(3_000),
            });
        }
        drop(log);
        let path = path(&dir, 7);
        assert_eq!(last_seq(&path, 2), Some(40));
        // A turn that wrote nothing yet has nothing to skip.
        assert_eq!(last_seq(&path, 4), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn text_unwritten_at_a_crash_is_written_by_the_replay() {
        let dir = root("crash");
        let mut log = TurnLog::start(Some(&dir), 7, 0, "codex", "");
        log.record(&AgentEvent::Text("Hel".into()));
        // A crash: the gathered text never reaches the file.
        log.text.clear();
        drop(log);
        let mut log = TurnLog::resume(Some(&dir), 7, 0);
        turn(&mut log);
        drop(log);
        let got = events(&dir);
        assert_eq!(got[1], AgentEvent::Text("Hello".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_output_keeps_its_end() {
        let dir = root("tail");
        let mut log = TurnLog::start(Some(&dir), 7, 0, "claude", "");
        log.record(&AgentEvent::ToolDone {
            id: "t1".into(),
            ok: false,
            output: format!("{}END", "x".repeat(10_000)),
        });
        drop(log);
        match &events(&dir)[1] {
            AgentEvent::ToolDone { output, .. } => {
                assert!(output.len() <= OUTPUT_TAIL + 4);
                assert!(output.ends_with("END"));
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_big_log_is_set_aside_when_a_turn_starts() {
        let dir = root("rotate");
        let file = path(&dir, 7);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::File::create(&file)
            .unwrap()
            .set_len(ROTATE_AT)
            .unwrap();
        drop(TurnLog::start(Some(&dir), 7, 4, "claude", ""));
        // Set aside on the writer thread.
        drain();
        assert_eq!(
            std::fs::metadata(file.with_file_name("events.1.jsonl"))
                .unwrap()
                .len(),
            ROTATE_AT
        );
        assert_eq!(
            read(&dir, 7).len(),
            1,
            "the zeros don't parse; the new start does"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_log_that_failed_to_write_is_tried_again_by_the_next_turn() {
        let dir = root("retry");
        // A folder where the file should be: every write of this turn fails.
        let file = path(&dir, 7);
        std::fs::create_dir_all(&file).unwrap();
        let mut log = TurnLog::start(Some(&dir), 7, 0, "claude", "");
        turn(&mut log);
        drop(log);
        drain();
        std::fs::remove_dir_all(&file).unwrap();
        let mut log = TurnLog::start(Some(&dir), 7, 1, "claude", "");
        log.end(true);
        drop(log);
        assert_eq!(
            events(&dir),
            vec![
                AgentEvent::TurnStarted {
                    provider: "claude".into(),
                    model: String::new()
                },
                AgentEvent::TurnEnded { ok: true },
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_log_without_a_root_keeps_nothing() {
        let mut log = TurnLog::start(None, 7, 0, "claude", "");
        turn(&mut log);
        log.end(true);
        assert!(log.path.is_none());
    }
}
