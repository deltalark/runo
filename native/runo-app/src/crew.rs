//! Crew: one chat per project that answers the person and runs threads for them. A thread
//! is an ordinary session with a `ThreadInfo` beside it: the brief Crew gave it, the
//! checklist and reports its agent keeps, and whether it needs the person.
//!
//! Threads talk back to the app through Runo's MCP server (`memory_mcp.rs`), which leaves
//! one small event file per call in `events_dir()`, written under a temporary name and
//! renamed so the app never reads half a file. Agents with no MCP write the same things as
//! fenced `runo-…` blocks in their reply, which `TagFilter` takes out of the visible text.
//!
//! This file holds the data and the rules; nothing in the app reads events yet.

use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::store::Project;

/// `Project::mode` of a project that works through Crew. Any other value, the empty
/// default included, is Direct.
pub const MODE_CREW: &str = "crew";

/// Steps a checklist keeps; more are cut off.
pub const MAX_STEPS: usize = 20;
/// Characters kept of one step, a decision option or a link.
pub const MAX_LINE: usize = 200;
/// Characters kept of a report, a question or a message to Crew.
pub const MAX_TEXT: usize = 4000;
/// Reports a thread keeps; the oldest go first.
pub const MAX_REPORTS: usize = 50;
/// Event ids remembered so an event read twice is applied once.
const SEEN_LIMIT: usize = 1000;

pub fn is_crew(project: &Project) -> bool {
    project.mode == MODE_CREW
}

/// What a session that is a Crew thread keeps besides the session itself.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct ThreadInfo {
    #[serde(default)]
    pub origin: Origin,
    /// What Crew asked the thread to do, shown at the top of the thread.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub brief: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checklist: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reports: Vec<Report>,
    /// The decision the thread is waiting on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Decision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    /// Hidden from the open threads; the session itself stays.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
}

/// Who started a thread.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    /// A session the person started that Crew now routes to.
    #[default]
    Person,
    /// Crew, answering the person's message with this id.
    Crew { message: u64 },
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    #[default]
    Todo,
    Doing,
    Done,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Step {
    pub text: String,
    #[serde(default)]
    pub mark: Mark,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Report {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// Seconds since the epoch.
    #[serde(default)]
    pub at: u64,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Choice {
    pub label: String,
    /// What happens if the person picks it.
    #[serde(default)]
    pub consequence: String,
}

/// One question for the person, with two to four answers and the one the thread would pick.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Decision {
    pub question: String,
    pub options: Vec<Choice>,
    #[serde(default)]
    pub recommended: usize,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    /// A decision, permission or question is open.
    NeedsYou,
    /// The work is done and waits for review.
    Review,
}

/// What a thread may spend: dollars where its agent reports cost, otherwise turns. Zero
/// means no limit of that kind.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Budget {
    #[serde(default)]
    pub dollars: f64,
    #[serde(default)]
    pub turns: u32,
}

/// One thing a thread told the app.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Event {
    /// Unique, so an event read twice is applied once.
    pub id: String,
    pub thread: u64,
    /// Seconds since the epoch.
    pub at: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// The whole checklist, replacing the last one.
    Checklist {
        steps: Vec<Step>,
    },
    Report {
        summary: String,
        #[serde(default)]
        links: Vec<String>,
    },
    Decision(Decision),
    /// A message for Crew, such as a question for another thread.
    Message {
        text: String,
    },
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Where threads leave events for the app.
pub fn events_dir() -> PathBuf {
    crate::store::data_dir().join("crew").join("events")
}

/// A new event from `thread`, stamped now.
pub fn event(thread: u64, kind: EventKind) -> Event {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Event {
        id: format!("{thread}-{nanos}-{}", std::process::id()),
        thread,
        at: now(),
        kind,
    }
}

/// Leave `event` in `dir`. Written under a temporary name and renamed, so the app never
/// reads half a file; names sort in the order events were sent.
pub fn send_event(dir: &Path, event: &Event) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{nanos:020}-{}", std::process::id());
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(event)?)?;
    std::fs::rename(tmp, dir.join(format!("{name}.json")))
}

/// Ids of events already taken, the newest `SEEN_LIMIT` of them.
#[derive(Default)]
pub struct Seen {
    order: VecDeque<String>,
    ids: HashSet<String>,
}

impl Seen {
    /// Whether `id` is new; it is remembered either way.
    pub fn first(&mut self, id: &str) -> bool {
        if !self.ids.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        if self.order.len() > SEEN_LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
        true
    }
}

/// Take every event waiting in `dir`, oldest first, removing their files. A file that
/// can't be read as an event is renamed to `.bad`, so it is skipped from then on but kept
/// for a look; an event already seen is dropped.
pub fn take_events(dir: &Path, seen: &mut Seen) -> Vec<Event> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let parsed = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Event>(&bytes).ok());
        match parsed {
            Some(event) => {
                let _ = std::fs::remove_file(&path);
                if seen.first(&event.id) {
                    out.push(event);
                }
            }
            None => {
                crate::applog::warn(
                    "crew",
                    format!("Skipped an unreadable event: {}", path.display()),
                );
                let _ = std::fs::rename(&path, path.with_extension("bad"));
            }
        }
    }
    out
}

/// `text` cut to `max` characters, trimmed.
fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].trim_end().to_string(),
        None => text.to_string(),
    }
}

/// A checklist as it is kept: steps clipped, empty ones left out, at most `MAX_STEPS`.
pub fn clean_steps(steps: Vec<Step>) -> Vec<Step> {
    steps
        .into_iter()
        .map(|s| Step {
            text: clip(&s.text, MAX_LINE),
            mark: s.mark,
        })
        .filter(|s| !s.text.is_empty())
        .take(MAX_STEPS)
        .collect()
}

/// A decision as it is kept, or why it can't be: it needs a question, two to four options
/// with labels, and a recommended option among them.
pub fn check_decision(decision: Decision) -> Result<Decision, String> {
    let question = clip(&decision.question, MAX_TEXT);
    if question.is_empty() {
        return Err("The decision needs a question.".into());
    }
    let options: Vec<Choice> = decision
        .options
        .into_iter()
        .map(|c| Choice {
            label: clip(&c.label, MAX_LINE),
            consequence: clip(&c.consequence, MAX_LINE),
        })
        .collect();
    if !(2..=4).contains(&options.len()) || options.iter().any(|c| c.label.is_empty()) {
        return Err("Give two to four options, each with a label.".into());
    }
    if decision.recommended >= options.len() {
        return Err("recommended must be the index of one of the options, from 0.".into());
    }
    Ok(Decision {
        question,
        options,
        recommended: decision.recommended,
    })
}

/// Apply what a thread reported to what the app keeps for it. Returns whether anything
/// changed; a message for Crew changes nothing here.
pub fn apply(info: &mut ThreadInfo, kind: &EventKind, at: u64) -> bool {
    match kind {
        EventKind::Checklist { steps } => {
            let steps = clean_steps(steps.clone());
            if info.checklist == steps {
                return false;
            }
            info.checklist = steps;
            true
        }
        EventKind::Report { summary, links } => {
            let summary = clip(summary, MAX_TEXT);
            if summary.is_empty() {
                return false;
            }
            info.reports.push(Report {
                summary,
                links: links.iter().map(|l| clip(l, MAX_LINE)).collect(),
                at,
            });
            if info.reports.len() > MAX_REPORTS {
                info.reports.remove(0);
            }
            true
        }
        EventKind::Decision(decision) => match check_decision(decision.clone()) {
            Ok(decision) => {
                info.decision = Some(decision);
                info.attention = Some(Attention::NeedsYou);
                true
            }
            Err(_) => false,
        },
        EventKind::Message { .. } => false,
    }
}

/// Kinds of fenced block an agent with no MCP may write: ```runo-checklist and so on.
pub const BLOCK_KINDS: [&str; 4] = ["checklist", "report", "decision", "actions"];

/// A fenced `runo-…` block taken out of a reply.
#[derive(Clone, PartialEq, Debug)]
pub struct Tagged {
    pub kind: String,
    pub body: String,
}

/// Takes `runo-…` blocks out of a reply as it streams in, passing the rest through.
/// Text inside any other fenced code block is left alone, so an agent showing an example
/// block inside ```markdown doesn't trigger anything. A line that might open a block is
/// held back until it is complete; everything else passes through as soon as it arrives.
#[derive(Default)]
pub struct TagFilter {
    /// The current line, not yet ended by a newline.
    line: String,
    /// Bytes of `line` already passed through.
    shown: usize,
    /// The open block: its opening fence line and body so far.
    block: Option<(String, String, String)>,
    /// Inside a fenced code block that isn't Runo's.
    fenced: bool,
    blocks: Vec<Tagged>,
}

impl TagFilter {
    /// Feed the next piece of the reply; returns the text to show.
    pub fn push(&mut self, chunk: &str) -> String {
        let mut out = String::new();
        for part in chunk.split_inclusive('\n') {
            self.line.push_str(part);
            if self.line.ends_with('\n') {
                let line = std::mem::take(&mut self.line);
                let shown = std::mem::take(&mut self.shown);
                out.push_str(&self.end_line(line, shown));
            }
        }
        // Pass an unfinished line through when it can't be a fence.
        if self.block.is_none() {
            let start = self.line.trim_start();
            if !start.is_empty() && !start.starts_with('`') {
                out.push_str(&self.line[self.shown..]);
                self.shown = self.line.len();
            }
        }
        out
    }

    /// The text still held back, and the blocks found. A block never closed is shown as
    /// text, so nothing the agent wrote is lost.
    pub fn finish(mut self) -> (String, Vec<Tagged>) {
        let mut out = String::new();
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            let shown = self.shown;
            out.push_str(&self.end_line(line, shown));
        }
        if let Some((open, _, body)) = self.block.take() {
            out.push_str(&open);
            out.push_str(&body);
        }
        (out, self.blocks)
    }

    fn end_line(&mut self, line: String, shown: usize) -> String {
        let fence = line.trim();
        if let Some((open, kind, mut body)) = self.block.take() {
            if fence == "```" {
                self.blocks.push(Tagged {
                    kind,
                    body: body.trim_end().to_string(),
                });
            } else {
                body.push_str(&line);
                self.block = Some((open, kind, body));
            }
            return String::new();
        }
        if shown == 0 && fence.starts_with("```") {
            if !self.fenced {
                if let Some(kind) = fence.strip_prefix("```runo-") {
                    if BLOCK_KINDS.contains(&kind) {
                        let kind = kind.to_string();
                        self.block = Some((line, kind, String::new()));
                        return String::new();
                    }
                }
            }
            self.fenced = !self.fenced;
        }
        line[shown..].to_string()
    }
}

/// The event a `checklist`, `report` or `decision` block stands for. A checklist is one
/// step per line, `- [x]` done, `- [~]` doing and `- [ ]` to do; a report is its text, with
/// lines that are only a link kept as links; a decision is JSON like `ask_decision`'s.
pub fn block_event(block: &Tagged) -> Option<EventKind> {
    match block.kind.as_str() {
        "checklist" => {
            let steps = block
                .body
                .lines()
                .filter_map(|line| {
                    let line = line.trim().trim_start_matches(['-', '*']).trim_start();
                    let (mark, text) = [
                        ("[x]", Mark::Done),
                        ("[X]", Mark::Done),
                        ("[~]", Mark::Doing),
                        ("[ ]", Mark::Todo),
                    ]
                    .into_iter()
                    .find_map(|(box_, mark)| line.strip_prefix(box_).map(|t| (mark, t)))?;
                    Some(Step {
                        text: text.trim().to_string(),
                        mark,
                    })
                })
                .collect::<Vec<_>>();
            let steps = clean_steps(steps);
            (!steps.is_empty()).then_some(EventKind::Checklist { steps })
        }
        "report" => {
            let (links, text): (Vec<&str>, Vec<&str>) = block.body.lines().partition(|l| {
                let l = l.trim();
                (l.starts_with("https://") || l.starts_with("http://")) && !l.contains(' ')
            });
            let summary = text.join("\n").trim().to_string();
            (!summary.is_empty()).then(|| EventKind::Report {
                summary,
                links: links.iter().map(|l| l.trim().to_string()).collect(),
            })
        }
        "decision" => serde_json::from_str::<Decision>(&block.body)
            .ok()
            .and_then(|d| check_decision(d).ok())
            .map(EventKind::Decision),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-crew-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn filter_all(chunks: &[&str]) -> (String, Vec<Tagged>) {
        let mut f = TagFilter::default();
        let mut shown: String = chunks.iter().map(|c| f.push(c)).collect();
        let (rest, blocks) = f.finish();
        shown.push_str(&rest);
        (shown, blocks)
    }

    #[test]
    fn projects_are_direct_unless_set_to_crew() {
        let project: Project = serde_json::from_str(r#"{"id":1,"name":"a","path":"/a"}"#).unwrap();
        assert!(!is_crew(&project));
        let crew = Project {
            mode: MODE_CREW.into(),
            ..project.clone()
        };
        assert!(is_crew(&crew));
        // Direct writes nothing, so files stay as older builds wrote them.
        assert!(!serde_json::to_string(&project).unwrap().contains("mode"));
    }

    #[test]
    fn events_round_trip_once_in_order() {
        let dir = temp("events");
        let first = event(7, EventKind::Message { text: "hi".into() });
        let second = event(
            7,
            EventKind::Report {
                summary: "Done".into(),
                links: vec![],
            },
        );
        send_event(&dir, &first).unwrap();
        send_event(&dir, &second).unwrap();
        // A file left half written by a crash isn't read.
        std::fs::write(dir.join("zz.tmp"), "{").unwrap();
        let mut seen = Seen::default();
        assert_eq!(take_events(&dir, &mut seen), vec![first.clone(), second]);
        assert!(take_events(&dir, &mut seen).is_empty());
        // The same event sent again, as after a replay, is applied once.
        send_event(&dir, &first).unwrap();
        assert!(take_events(&dir, &mut seen).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_events_are_set_aside() {
        let dir = temp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.json"), "not json").unwrap();
        let good = event(1, EventKind::Message { text: "ok".into() });
        send_event(&dir, &good).unwrap();
        let mut seen = Seen::default();
        assert_eq!(take_events(&dir, &mut seen), vec![good]);
        assert!(dir.join("1.bad").exists());
        assert!(take_events(&dir, &mut seen).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn seen_forgets_the_oldest() {
        let mut seen = Seen::default();
        for i in 0..=SEEN_LIMIT {
            assert!(seen.first(&i.to_string()));
        }
        assert!(!seen.first(&SEEN_LIMIT.to_string()));
        assert!(seen.first("0"));
    }

    #[test]
    fn apply_keeps_what_threads_report() {
        let mut info = ThreadInfo::default();
        let steps = vec![
            Step {
                text: " Measure ".into(),
                mark: Mark::Done,
            },
            Step {
                text: "".into(),
                mark: Mark::Todo,
            },
        ];
        assert!(apply(
            &mut info,
            &EventKind::Checklist {
                steps: steps.clone()
            },
            1
        ));
        assert_eq!(info.checklist.len(), 1);
        assert_eq!(info.checklist[0].text, "Measure");
        assert!(!apply(&mut info, &EventKind::Checklist { steps }, 2));
        let long: Vec<Step> = (0..40)
            .map(|i| Step {
                text: format!("step {i}"),
                mark: Mark::Todo,
            })
            .collect();
        apply(&mut info, &EventKind::Checklist { steps: long }, 3);
        assert_eq!(info.checklist.len(), MAX_STEPS);

        for i in 0..MAX_REPORTS + 3 {
            let kind = EventKind::Report {
                summary: format!("r{i}"),
                links: vec![],
            };
            assert!(apply(&mut info, &kind, i as u64));
        }
        assert_eq!(info.reports.len(), MAX_REPORTS);
        assert_eq!(info.reports[0].summary, "r3");
        let empty = EventKind::Report {
            summary: "  ".into(),
            links: vec![],
        };
        assert!(!apply(&mut info, &empty, 9));

        let ask = |options: usize, recommended: usize| {
            EventKind::Decision(Decision {
                question: "Quit Runo?".into(),
                options: (0..options)
                    .map(|i| Choice {
                        label: format!("o{i}"),
                        consequence: String::new(),
                    })
                    .collect(),
                recommended,
            })
        };
        assert!(!apply(&mut info, &ask(1, 0), 1));
        assert!(!apply(&mut info, &ask(5, 0), 1));
        assert!(!apply(&mut info, &ask(2, 2), 1));
        assert!(info.attention.is_none());
        assert!(apply(&mut info, &ask(2, 1), 1));
        assert_eq!(info.attention, Some(Attention::NeedsYou));
        assert!(!apply(
            &mut info,
            &EventKind::Message { text: "x".into() },
            1
        ));
    }

    #[test]
    fn thread_info_saves_only_what_is_set() {
        assert_eq!(
            serde_json::to_string(&ThreadInfo::default()).unwrap(),
            r#"{"origin":{"kind":"person"}}"#
        );
        let info = ThreadInfo {
            origin: Origin::Crew { message: 4 },
            brief: "Time launches".into(),
            resolved: true,
            ..Default::default()
        };
        let back: ThreadInfo =
            serde_json::from_str(&serde_json::to_string(&info).unwrap()).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn tags_come_out_of_the_reply() {
        let reply = "Working on it.\n```runo-checklist\n- [x] Measure\n- [~] Fix\n- [ ] PR\n```\nDone for now.\n";
        let (shown, blocks) = filter_all(&[reply]);
        assert_eq!(shown, "Working on it.\nDone for now.\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "checklist");
        let Some(EventKind::Checklist { steps }) = block_event(&blocks[0]) else {
            panic!("no checklist");
        };
        let marks: Vec<Mark> = steps.iter().map(|s| s.mark).collect();
        assert_eq!(marks, vec![Mark::Done, Mark::Doing, Mark::Todo]);
    }

    #[test]
    fn tags_split_across_chunks() {
        let reply = "Hi\n```runo-report\nAll done.\nhttps://github.com/a/b/pull/1\n```\nBye";
        // Every split point gives the same result.
        for at in 0..=reply.len() {
            if !reply.is_char_boundary(at) {
                continue;
            }
            let (shown, blocks) = filter_all(&[&reply[..at], &reply[at..]]);
            assert_eq!(shown, "Hi\nBye", "split at {at}");
            assert_eq!(blocks.len(), 1, "split at {at}");
        }
        let (_, blocks) = filter_all(&[reply]);
        assert_eq!(
            block_event(&blocks[0]),
            Some(EventKind::Report {
                summary: "All done.".into(),
                links: vec!["https://github.com/a/b/pull/1".into()],
            })
        );
    }

    #[test]
    fn text_streams_without_waiting_for_the_line() {
        let mut f = TagFilter::default();
        assert_eq!(f.push("Hel"), "Hel");
        assert_eq!(f.push("lo\n``"), "lo\n");
        assert_eq!(f.push("`rust\nfn a() {}\n"), "```rust\nfn a() {}\n");
    }

    #[test]
    fn examples_inside_other_code_blocks_stay() {
        let reply = "```markdown\n```runo-checklist\n- [ ] x\n```\n";
        let (shown, blocks) = filter_all(&[reply]);
        // The inner fence closes the markdown block, as Markdown reads it; nothing is taken.
        assert_eq!(shown, reply);
        assert!(blocks.is_empty());
        let other = "```runo-unknown\nx\n```\n";
        assert_eq!(filter_all(&[other]).0, other);
    }

    #[test]
    fn unclosed_blocks_are_shown() {
        let reply = "Text\n```runo-report\nhalf a report";
        let (shown, blocks) = filter_all(&[reply]);
        assert_eq!(shown, reply);
        assert!(blocks.is_empty());
    }

    #[test]
    fn decision_blocks_are_checked() {
        let good = Tagged {
            kind: "decision".into(),
            body:
                r#"{"question":"Quit?","options":[{"label":"Yes"},{"label":"No"}],"recommended":1}"#
                    .into(),
        };
        assert!(matches!(block_event(&good), Some(EventKind::Decision(_))));
        let bad = Tagged {
            kind: "decision".into(),
            body: r#"{"question":"Quit?","options":[{"label":"Yes"}]}"#.into(),
        };
        assert_eq!(block_event(&bad), None);
        let junk = Tagged {
            kind: "decision".into(),
            body: "{".into(),
        };
        assert_eq!(block_event(&junk), None);
    }
}
