//! Contract tests from recorded runs. Each `tests/fixtures/<agent>-<what>.jsonl` holds an
//! agent CLI's raw output from a real turn (recorded with `RUNO_RECORD`, home folders and
//! accounts taken out), and the `.events.json` beside it the events Runo must turn it
//! into. A parser change that alters what a real run produces fails here; when the change
//! is meant, `RUNO_UPDATE_FIXTURES=1 cargo test fixtures` rewrites the expectations, and
//! the diff shows what moved.

use std::path::{Path, PathBuf};

use crate::agent::Output;
use crate::harness::AgentEvent;
use crate::stream::Parser;

/// The folder the recorded turns ran in, as the fixtures name it.
const CWD: &str = "/work/app";

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The output format of the agent a fixture was recorded from, by its name's first part.
/// Fixed here rather than read from the installed CLI's help, so tests don't depend on
/// what is installed.
fn format(name: &str) -> Output {
    match name.split('-').next().unwrap_or("") {
        "claude" | "amp" | "qwen" => Output::Claude,
        "codex" | "cursor" | "droid" | "opencode" | "gemini" | "copilot" => Output::Json,
        other => panic!("{name}: no agent named {other:?}; name fixtures <agent>-<what>.jsonl"),
    }
}

/// The events a recorded run gives.
fn replay(path: &Path) -> Vec<AgentEvent> {
    let name = path.file_stem().unwrap().to_string_lossy().to_string();
    let mut parser = Parser::for_format(format(&name), CWD).expect("a structured format");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| parser.feed(line))
        .collect()
}

fn fixtures() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir())
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

#[test]
fn recorded_runs_give_the_expected_events() {
    let update = std::env::var_os("RUNO_UPDATE_FIXTURES").is_some();
    let mut failed = Vec::new();
    let found = fixtures();
    assert!(!found.is_empty(), "no fixtures in {}", dir().display());
    for path in found {
        let events = replay(&path);
        let expected_path = path.with_extension("events.json");
        let got = serde_json::to_string_pretty(&events).unwrap() + "\n";
        if update {
            std::fs::write(&expected_path, &got).unwrap();
            continue;
        }
        let Ok(expected) = std::fs::read_to_string(&expected_path) else {
            failed.push(format!(
                "{}: no expected events; run with RUNO_UPDATE_FIXTURES=1",
                path.display()
            ));
            continue;
        };
        let expected: Vec<AgentEvent> = serde_json::from_str(&expected).unwrap();
        if events != expected {
            let first = events
                .iter()
                .zip(&expected)
                .position(|(a, b)| a != b)
                .unwrap_or(events.len().min(expected.len()));
            failed.push(format!(
                "{}: events differ from event {first}: got {:?}, expected {:?}",
                path.display(),
                events.get(first),
                expected.get(first)
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// What every recorded run must hold whatever the agent: each running step gets exactly
/// one result, no result comes without its step, and a turn reports some text or an
/// error.
#[test]
fn recorded_runs_close_every_step() {
    for path in fixtures() {
        let events = replay(&path);
        let mut running: Vec<String> = Vec::new();
        for event in &events {
            match event {
                AgentEvent::Tool(step) if step.state == crate::harness::StepState::Running => {
                    assert!(
                        !running.contains(&step.id),
                        "{}: step {} started twice",
                        path.display(),
                        step.id
                    );
                    running.push(step.id.clone());
                }
                AgentEvent::ToolDone { id, .. } => {
                    let at = running.iter().position(|r| r == id).unwrap_or_else(|| {
                        panic!("{}: result {id} has no running step", path.display())
                    });
                    running.remove(at);
                }
                _ => {}
            }
        }
        assert!(
            running.is_empty(),
            "{}: steps never finished: {running:?}",
            path.display()
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Text(_) | AgentEvent::Error(_))),
            "{}: no reply and no error",
            path.display()
        );
    }
}
