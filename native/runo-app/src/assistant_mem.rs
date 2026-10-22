//! What the Runo assistant keeps between runs: its chats, so a conversation survives a
//! restart and earlier ones can be picked up again, and the notes the person asked it to
//! remember, which go to the agent with every question. Both live in `assistant.json` in
//! Runo's data folder. A chat from an earlier day isn't reopened on its own: the panel
//! starts on the briefing instead, with the chat a click away under Chats.

use super::*;
use crate::assistant::{article, Msg};
use crate::assistant_act::{Action, Outcome};
use serde::{Deserialize, Serialize};

/// The most chats kept; the oldest go first.
const MAX_CHATS: usize = 30;
/// The most messages kept of one chat; its oldest go first.
const MAX_MESSAGES: usize = 80;
/// The most notes kept.
pub(crate) const MAX_NOTES: usize = 50;

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Kept {
    #[serde(default)]
    pub chats: Vec<Chat>,
    #[serde(default)]
    pub notes: Vec<Note>,
}

/// One conversation, newest message last.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Chat {
    pub id: u64,
    /// When it last changed, in seconds since 1970.
    pub updated: i64,
    pub msgs: Vec<KeptMsg>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct KeptMsg {
    pub you: bool,
    pub text: String,
    #[serde(default)]
    pub actions: Vec<(Action, Outcome)>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub open: Option<String>,
}

/// Something the person asked the assistant to remember, for one project or everywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Note {
    pub text: String,
    /// The project it is about; empty for every project.
    #[serde(default)]
    pub project: String,
}

fn path() -> std::path::PathBuf {
    store::data_dir().join("assistant.json")
}

pub(crate) fn load() -> Kept {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn now() -> i64 {
    chrono::Local::now().timestamp()
}

/// Whether a chat last changed at `updated` is still today's, so it reopens as it was.
pub(crate) fn same_day(updated: i64, now: i64) -> bool {
    use chrono::TimeZone;
    let day = |t: i64| {
        chrono::Local
            .timestamp_opt(t, 0)
            .single()
            .map(|d| d.date_naive())
    };
    day(updated).is_some() && day(updated) == day(now)
}

/// When a chat last changed, the way the list of chats shows it.
pub(crate) fn when(updated: i64) -> String {
    use chrono::TimeZone;
    let Some(at) = chrono::Local.timestamp_opt(updated, 0).single() else {
        return String::new();
    };
    if same_day(updated, now()) {
        at.format("Today %H:%M").to_string()
    } else {
        at.format("%b %-d").to_string()
    }
}

/// A chat's name: its first question, on one line.
pub(crate) fn title(msgs: &[KeptMsg]) -> String {
    let first = msgs.iter().find(|m| m.you).map_or("", |m| m.text.as_str());
    let one: String = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 {
        format!("{}…", one.chars().take(60).collect::<String>())
    } else {
        one
    }
}

fn kept_msg(m: &Msg) -> KeptMsg {
    KeptMsg {
        you: m.you,
        text: m.text.clone(),
        actions: m.actions.clone(),
        sources: m.sources.iter().map(|s| s.to_string()).collect(),
        open: m.open.clone(),
    }
}

pub(crate) fn msg(k: &KeptMsg) -> Msg {
    let mut m = Msg::new(k.you, k.text.clone(), k.actions.clone());
    // Articles that are no longer in the guide are left out.
    m.sources = k
        .sources
        .iter()
        .filter_map(|id| article(id))
        .map(|a| a.id)
        .collect();
    m.open = k.open.clone();
    m
}

/// The notes that apply in `project`, as the agent is told them.
pub(crate) fn notes_for(notes: &[Note], project: &str) -> Vec<String> {
    notes
        .iter()
        .filter(|n| n.project.is_empty() || n.project == project)
        .map(|n| {
            if n.project.is_empty() {
                n.text.clone()
            } else {
                format!("{} (in {})", n.text, n.project)
            }
        })
        .collect()
}

impl RunoApp {
    /// Write the chats and notes, with the open chat as it is now.
    pub(crate) fn assistant_keep(&mut self) {
        let a = &mut self.assistant;
        a.kept.chats.retain(|c| c.id != a.chat_id);
        if !a.chat.is_empty() {
            let skip = a.chat.len().saturating_sub(MAX_MESSAGES);
            a.kept.chats.push(Chat {
                id: a.chat_id,
                updated: now(),
                msgs: a.chat.iter().skip(skip).map(kept_msg).collect(),
            });
        }
        a.kept.chats.sort_by_key(|c| std::cmp::Reverse(c.updated));
        a.kept.chats.truncate(MAX_CHATS);
        if let Ok(json) = serde_json::to_vec_pretty(&a.kept) {
            let _ = crate::approvals::write_atomic(&path(), &json);
        }
    }

    /// Put the open chat away and start an empty one, which shows the briefing.
    pub(crate) fn assistant_new_chat(&mut self) {
        self.assistant_stop();
        self.assistant_keep();
        self.assistant.chat.clear();
        self.assistant.asking = None;
        self.assistant.streamed.clear();
        self.assistant.chat_id = self
            .assistant
            .kept
            .chats
            .iter()
            .map(|c| c.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.assistant.chats_open = false;
    }

    /// Reopen an earlier chat where it left off.
    pub(crate) fn assistant_open_chat(&mut self, id: u64) {
        if id == self.assistant.chat_id {
            self.assistant.chats_open = false;
            return;
        }
        self.assistant_new_chat();
        if let Some(c) = self.assistant.kept.chats.iter().find(|c| c.id == id) {
            self.assistant.chat = c.msgs.iter().map(msg).collect();
            self.assistant.chat_id = id;
        }
        self.assistant.scroll.scroll_to_bottom();
    }

    pub(crate) fn assistant_delete_chat(&mut self, id: u64) {
        self.assistant.kept.chats.retain(|c| c.id != id);
        if id == self.assistant.chat_id {
            self.assistant.chat.clear();
        }
        self.assistant_keep();
    }

    /// The top bar's project, by name, which is what notes are kept under.
    pub(crate) fn assistant_project_name(&self) -> String {
        self.current_project()
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    /// Remember `text` for the current project. Says what was kept, or why not.
    pub(crate) fn assistant_remember(&mut self, text: &str) -> Result<String, String> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Err("There's nothing to remember.".into());
        }
        let project = self.assistant_project_name();
        let note = Note { text, project };
        let notes = &mut self.assistant.kept.notes;
        if !notes.contains(&note) {
            if notes.len() >= MAX_NOTES {
                return Err(format!(
                    "I already remember {MAX_NOTES} things; forget one first with /forget."
                ));
            }
            notes.push(note.clone());
        }
        self.assistant_keep();
        Ok(if note.project.is_empty() {
            "Remembered".into()
        } else {
            format!("Remembered for {}", note.project)
        })
    }

    pub(crate) fn assistant_forget(&mut self, text: &str) -> Result<String, String> {
        let before = self.assistant.kept.notes.len();
        self.assistant.kept.notes.retain(|n| n.text != text);
        if self.assistant.kept.notes.len() == before {
            return Err("I don't remember that.".into());
        }
        self.assistant_keep();
        Ok("Forgotten".into())
    }

    /// What the agent is told to keep in mind, for the project on screen.
    pub(crate) fn assistant_notes(&self) -> String {
        let notes = notes_for(&self.assistant.kept.notes, &self.assistant_project_name());
        if notes.is_empty() {
            return String::new();
        }
        format!(
            "What the person asked you to remember; follow it unless they say otherwise now:\n- {}\n",
            notes.join("\n- ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn kept_chats_read_back() {
        let kept = Kept {
            chats: vec![Chat {
                id: 3,
                updated: 1_700_000_000,
                msgs: vec![
                    KeptMsg {
                        you: true,
                        text: "start codex on the docs".into(),
                        actions: Vec::new(),
                        sources: Vec::new(),
                        open: None,
                    },
                    KeptMsg {
                        you: false,
                        text: "Ready when you are:".into(),
                        actions: vec![(
                            Action::Stop { session: 4 },
                            Outcome::Done("Stopped".into()),
                        )],
                        sources: vec!["new-session".into(), "gone".into()],
                        open: Some("inbox".into()),
                    },
                ],
            }],
            notes: vec![Note {
                text: "Use worktrees for Codex".into(),
                project: "web".into(),
            }],
        };
        let json = serde_json::to_string(&kept).unwrap();
        let back: Kept = serde_json::from_str(&json).unwrap();
        assert_eq!(back.notes, kept.notes);
        let m = msg(&back.chats[0].msgs[1]);
        assert_eq!(m.actions, kept.chats[0].msgs[1].actions);
        // Only articles still in the guide come back.
        assert_eq!(m.sources, vec!["new-session"]);
        assert_eq!(title(&back.chats[0].msgs), "start codex on the docs");
        // A file from an older Runo, or none at all, reads as empty.
        assert!(serde_json::from_str::<Kept>("{}").unwrap().chats.is_empty());
    }

    #[test]
    fn only_todays_chat_reopens() {
        let now = chrono::Local::now().timestamp();
        assert!(same_day(now - 1, now));
        assert!(!same_day(now - 3 * 86_400, now));
    }

    #[test]
    fn notes_follow_the_project() {
        let notes = vec![
            Note {
                text: "Answer briefly".into(),
                project: String::new(),
            },
            Note {
                text: "Use worktrees for Codex".into(),
                project: "web".into(),
            },
        ];
        assert_eq!(notes_for(&notes, "api"), vec!["Answer briefly"]);
        assert_eq!(
            notes_for(&notes, "web"),
            vec!["Answer briefly", "Use worktrees for Codex (in web)"]
        );
    }
}
