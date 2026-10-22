//! The agent host: `Runo --agent-host runo-turn <turn folder> <agent> <args…>`. It runs
//! an agent that takes its prompts on stdin (Claude Code's `--input-format stream-json`)
//! as a detached turn. Like the plain turn wrapper, it outlives Runo: the agent's output
//! goes to `out` and `err` in the turn folder, and `exit` holds its exit code once it
//! ends. It also forwards each line Runo appends to `in` to the agent's stdin, so a
//! message sent while the agent works reaches it at once. `sent` counts the lines
//! forwarded.
//!
//! The agent answers each message with a `result` event. Once every message forwarded so
//! far has had one, the host closes stdin and the agent exits, ending the turn. A line
//! that arrives after that stays in `in` unsent, and Runo sends it as the next turn.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// After a result, with messages still unanswered, the agent may have taken several in
/// one go; when it prints nothing for this long its input is closed anyway.
const IDLE: Duration = Duration::from_secs(10);

/// `Runo --agent-host <mark> <turn folder> <agent> <args…>`. Returns the exit code.
pub fn main(args: &[String]) -> i32 {
    // args[0] is the mark that names Runo's turns in the process list.
    let (Some(turn), Some(program)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: Runo --agent-host runo-turn <turn folder> <agent> [args…]");
        return 2;
    };
    run(Path::new(turn), program, &args[3..], IDLE)
}

/// What the host knows about the conversation so far.
#[derive(Default)]
struct Progress {
    /// Lines forwarded to the agent.
    sent: u64,
    /// Results the agent printed.
    results: u64,
    /// When the agent last printed anything.
    last_output: Option<Instant>,
}

/// Run `program` with its input fed from `turn/in`, until it exits. Returns its exit
/// code, which is also written to `turn/exit`.
pub fn run(turn: &Path, program: &str, args: &[String], idle: Duration) -> i32 {
    let code = match host(turn, program, args, idle) {
        Ok(code) => code,
        Err(e) => {
            let _ = std::fs::write(
                turn.join("err"),
                format!("Runo could not start {program}: {e}\n"),
            );
            127
        }
    };
    let tmp = turn.join("exit.tmp");
    if std::fs::write(&tmp, format!("{code}\n")).is_ok() {
        let _ = std::fs::rename(&tmp, turn.join("exit"));
    }
    code
}

fn host(turn: &Path, program: &str, args: &[String], idle: Duration) -> std::io::Result<i32> {
    let mut out = std::fs::File::create(turn.join("out"))?;
    let err = std::fs::File::create(turn.join("err"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(err))
        .spawn()?;
    let stdin: Arc<Mutex<Option<ChildStdin>>> = Arc::new(Mutex::new(child.stdin.take()));
    let progress = Arc::new(Mutex::new(Progress::default()));
    let done = Arc::new(AtomicBool::new(false));
    std::thread::spawn({
        let (turn, stdin, progress, done) = (
            turn.to_path_buf(),
            stdin.clone(),
            progress.clone(),
            done.clone(),
        );
        move || forward(&turn, &stdin, &progress, &done)
    });
    std::thread::spawn({
        let (stdin, progress, done) = (stdin.clone(), progress.clone(), done.clone());
        move || watch_idle(&stdin, &progress, &done, idle)
    });
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
            let answered = {
                let mut p = progress.lock().unwrap();
                p.last_output = Some(Instant::now());
                if is_result(&line) {
                    p.results += 1;
                }
                is_result(&line) && p.results >= p.sent
            };
            if answered {
                close(&stdin);
            }
        }
    }
    done.store(true, Ordering::SeqCst);
    close(&stdin);
    let status = child.wait()?;
    Ok(status.code().unwrap_or(1))
}

/// Whether `line` is the agent's result for a message.
fn is_result(line: &str) -> bool {
    line.contains("\"result\"")
        && serde_json::from_str::<serde_json::Value>(line).is_ok_and(|v| v["type"] == "result")
}

fn close(stdin: &Mutex<Option<ChildStdin>>) {
    if let Ok(mut stdin) = stdin.lock() {
        stdin.take();
    }
}

/// Pass each new line of `turn/in` to the agent while its input is open.
fn forward(
    turn: &Path,
    stdin: &Mutex<Option<ChildStdin>>,
    progress: &Mutex<Progress>,
    done: &AtomicBool,
) {
    let path = turn.join("in");
    let mut offset = 0u64;
    while !done.load(Ordering::SeqCst) {
        for line in new_lines(&path, &mut offset) {
            let Ok(mut guard) = stdin.lock() else { return };
            let Some(pipe) = guard.as_mut() else {
                // Input is closed: the line stays in `in` for Runo to send next.
                return;
            };
            if writeln!(pipe, "{line}").and_then(|_| pipe.flush()).is_err() {
                return;
            }
            // Counted before the input lock is let go, so a result can't close the input
            // while this message is still uncounted.
            let sent = {
                let mut p = progress.lock().unwrap();
                p.sent += 1;
                p.sent
            };
            drop(guard);
            let _ = std::fs::write(turn.join("sent"), sent.to_string());
        }
        if stdin.lock().map(|s| s.is_none()).unwrap_or(true) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Close the agent's input when it went quiet after a result with messages unanswered.
fn watch_idle(
    stdin: &Mutex<Option<ChildStdin>>,
    progress: &Mutex<Progress>,
    done: &AtomicBool,
    idle: Duration,
) {
    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
        let p = progress.lock().unwrap();
        let quiet = p.last_output.is_some_and(|t| t.elapsed() >= idle);
        if p.results > 0 && p.sent > p.results && quiet {
            drop(p);
            close(stdin);
            return;
        }
    }
}

/// New complete, non-empty lines of `path` from `offset` on.
fn new_lines(path: &Path, offset: &mut u64) -> Vec<String> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    if file.seek(SeekFrom::Start(*offset)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    *offset += end as u64;
    String::from_utf8_lossy(&bytes[..end])
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The line a prompt is sent as: a user message in Claude Code's stream-json input.
pub fn user_line(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": text },
    })
    .to_string()
}

/// Add a message for a live turn's agent. It is sent at once while the agent still
/// takes input; otherwise it waits in `in` and `unsent` hands it back.
pub fn send(turn: &Path, text: &str) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(turn.join("in"))?;
    writeln!(file, "{}", user_line(text))
}

/// The messages of a live turn the agent never got, oldest first: those after the
/// lines the host forwarded.
pub fn unsent(turn: &Path) -> Vec<String> {
    let sent: usize = std::fs::read_to_string(turn.join("sent"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let mut offset = 0;
    new_lines(&turn.join("in"), &mut offset)
        .into_iter()
        .skip(sent)
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(&line).ok()?;
            v["message"]["content"].as_str().map(str::to_string)
        })
        .collect()
}

/// Whether a turn folder belongs to a live turn.
pub fn is_live(turn: &Path) -> bool {
    turn.join("in").is_file()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An agent that answers each line with a result naming it, and exits when its
    /// input closes.
    const ECHO: &str = r#"while read -r l; do printf '{"type":"assistant","got":%s}\n' "$l"; sleep 0.3; echo '{"type":"result","subtype":"success"}'; done; echo bye"#;

    fn sh(script: &str) -> Vec<String> {
        vec!["-c".into(), script.into()]
    }

    #[test]
    fn one_message_gets_its_result_and_the_agent_exits() {
        let turn = temp("one");
        send(&turn, "hello").unwrap();
        let code = run(&turn, "sh", &sh(ECHO), IDLE);
        assert_eq!(code, 0);
        let out = std::fs::read_to_string(turn.join("out")).unwrap();
        assert!(out.contains("hello"), "{out}");
        assert!(out.ends_with("bye\n"), "{out}");
        assert_eq!(
            std::fs::read_to_string(turn.join("exit")).unwrap().trim(),
            "0"
        );
        assert_eq!(std::fs::read_to_string(turn.join("sent")).unwrap(), "1");
        assert!(unsent(&turn).is_empty());
        let _ = std::fs::remove_dir_all(turn);
    }

    #[test]
    fn a_message_sent_while_the_agent_works_reaches_it() {
        let turn = temp("steer");
        send(&turn, "first").unwrap();
        let t = turn.clone();
        let host = std::thread::spawn(move || run(&t, "sh", &sh(ECHO), IDLE));
        // The agent is still on the first message.
        std::thread::sleep(Duration::from_millis(100));
        send(&turn, "second").unwrap();
        assert_eq!(host.join().unwrap(), 0);
        let out = std::fs::read_to_string(turn.join("out")).unwrap();
        assert!(out.contains("first") && out.contains("second"), "{out}");
        assert_eq!(out.matches("\"result\"").count(), 2);
        assert!(unsent(&turn).is_empty());
        let _ = std::fs::remove_dir_all(turn);
    }

    #[test]
    fn a_message_after_the_last_result_is_handed_back() {
        let turn = temp("late");
        send(&turn, "first").unwrap();
        run(&turn, "sh", &sh(ECHO), IDLE);
        send(&turn, "too late").unwrap();
        assert_eq!(unsent(&turn), ["too late"]);
        let _ = std::fs::remove_dir_all(turn);
    }

    #[test]
    fn an_agent_that_takes_two_messages_in_one_go_still_ends() {
        let turn = temp("merge");
        send(&turn, "a").unwrap();
        send(&turn, "b").unwrap();
        // Reads both, answers once.
        let script = r#"read -r a; read -r b; echo '{"type":"result"}'; cat >/dev/null; echo bye"#;
        let start = Instant::now();
        let code = run(&turn, "sh", &sh(script), Duration::from_millis(300));
        assert_eq!(code, 0);
        assert!(start.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(turn);
    }

    #[test]
    fn an_agent_that_cannot_start_reports_why() {
        let turn = temp("missing");
        send(&turn, "x").unwrap();
        let code = run(&turn, "/nonexistent/agent", &[], IDLE);
        assert_eq!(code, 127);
        let err = std::fs::read_to_string(turn.join("err")).unwrap();
        assert!(err.contains("could not start"), "{err}");
        let _ = std::fs::remove_dir_all(turn);
    }

    #[test]
    fn user_lines_are_claude_stream_json_messages() {
        let line: serde_json::Value = serde_json::from_str(&user_line("hi \"there\"")).unwrap();
        assert_eq!(line["type"], "user");
        assert_eq!(line["message"]["role"], "user");
        assert_eq!(line["message"]["content"], "hi \"there\"");
    }
}
