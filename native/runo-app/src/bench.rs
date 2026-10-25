//! `RUNO_BENCH=1`, with `RUNO_FRAME_STATS=<file>` and a throwaway `RUNO_HOME`: plays the
//! same scenes every time and quits, so frame times can be compared before and after a
//! change. It makes up a long chat (prose, lists, code and tables) and opens it, then:
//!
//! - `idle`: nothing happens, no agent runs;
//! - `scroll`: the chat scrolls up and down, a step every frame;
//! - `type`: a character goes into the composer every 70 ms, as fast typing does;
//! - `stream`: three agents stream replies, the open chat's and two in the background,
//!   through the same path a real agent's output takes.
//!
//! Each scene is one line in the stats file (see `framestats::scene`).

use super::*;

/// How long each scene runs.
const SCENE: Duration = Duration::from_secs(12);
/// Messages in the open chat, and in the two background ones.
const LONG: usize = 300;
const SHORT: usize = 40;
/// How often a streaming agent sends a piece of its reply.
const STREAM_EVERY: Duration = Duration::from_millis(30);

pub(crate) fn on() -> bool {
    std::env::var_os("RUNO_BENCH").is_some()
}

/// Message `i` of a made-up chat: the person's question, or a reply with the Markdown a
/// real one has.
fn message(i: usize) -> Message {
    if i.is_multiple_of(2) {
        return Message::new(
            Role::User,
            format!("Question {i}: why does the list jump when a reply streams in?"),
        );
    }
    let text = format!(
        "## Step {i}\n\nThe list measures **each row** as it comes into view, so a row that \
         grows while it streams moves the ones below it. Three things matter here:\n\n\
         1. The row's height is measured again only when it *changes*.\n\
         2. Rows out of view keep the height they were measured at.\n\
         3. Following the bottom keeps the newest row in view.\n\n\
         ```rust\nfn update(list: &ListState, rows: Vec<Row>) {{\n    for (i, row) in rows.iter().enumerate() {{\n        if row.changed {{\n            list.remeasure_items(i..i + 1);\n        }}\n    }}\n}}\n```\n\n\
         | Case | Before | After |\n|---|---|---|\n| Short chat | 4 ms | 2 ms |\n| Long chat | 31 ms | 3 ms |\n\n\
         That leaves `remeasure` for settings that change every row, such as the font."
    );
    Message::new(Role::Agent, text)
}

/// Words a streaming reply is made of.
const WORDS: &[&str] = &[
    "The ",
    "list ",
    "now ",
    "lays ",
    "out ",
    "only ",
    "the ",
    "rows ",
    "in ",
    "view, ",
    "so ",
    "a ",
    "**long** ",
    "chat ",
    "scrolls ",
    "like ",
    "a ",
    "short ",
    "one.\n\n",
    "```rust\nlet x = 1;\n```\n\n",
    "- one\n",
    "- two\n\n",
];

/// One line of Claude Code's stream for piece `n` of a reply.
fn delta(n: usize) -> String {
    let text: String = (0..6).map(|k| WORDS[(n * 6 + k) % WORDS.len()]).collect();
    serde_json::json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": text },
        },
        "session_id": "bench",
        "parent_tool_use_id": null,
    })
    .to_string()
}

fn start_lines() -> Vec<String> {
    [
        serde_json::json!({
            "type": "stream_event",
            "event": { "type": "message_start", "message": {
                "model": "bench", "id": "msg_bench", "type": "message", "role": "assistant",
                "content": [], "stop_reason": null,
                "usage": { "input_tokens": 1, "output_tokens": 1 } } },
            "session_id": "bench", "parent_tool_use_id": null,
        }),
        serde_json::json!({
            "type": "stream_event",
            "event": { "type": "content_block_start", "index": 0,
                "content_block": { "type": "text", "text": "" } },
            "session_id": "bench", "parent_tool_use_id": null,
        }),
    ]
    .iter()
    .map(|v| v.to_string())
    .collect()
}

/// Start measuring scene `name` (`None`: stop), then note under the last scene's line
/// how many times it drew each part of the window and each chat row.
fn bench_scene(name: Option<&str>) {
    framestats::scene(name);
    framestats::note(&format!(
        "  drawn: {}, chat rows {}",
        panes::take_drawn(),
        chat_list::take_rows_drawn()
    ));
}

/// The event a line of an agent's output arrives as.
fn line_event(id: u64, line: String) -> Event {
    agent::line_event(Target::Session(id), false, line)
}

impl RunoApp {
    /// Make up the chats, open the long one, and play the scenes.
    pub(crate) fn bench_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = std::env::temp_dir().to_string_lossy().into_owned();
        let project = self.state.next_id();
        self.state.projects.push(store::Project {
            id: project,
            name: "Bench".into(),
            path: dir.clone(),
            ..Default::default()
        });
        let ids: Vec<u64> = [LONG, SHORT, SHORT]
            .iter()
            .enumerate()
            .map(|(n, &len)| {
                let id = self.state.next_id();
                self.state.sessions.push(Session {
                    id,
                    project_id: project,
                    title: format!("Bench chat {n}"),
                    provider: "claude".into(),
                    dir: dir.clone(),
                    messages: (0..len).map(message).collect(),
                    ..Default::default()
                });
                id
            })
            .collect();
        self.select(ids[0]);
        // The first-run tour would sit over every scene.
        self.tour = None;
        cx.spawn_in(window, async move |this, cx| {
            let executor = cx.background_executor().clone();
            let wait = move |d: Duration| executor.timer(d);
            // Let the window settle and the chat come up.
            wait(Duration::from_secs(3)).await;

            bench_scene(Some("idle"));
            wait(SCENE).await;

            bench_scene(Some("scroll"));
            let began = std::time::Instant::now();
            while began.elapsed() < SCENE {
                wait(Duration::from_millis(4)).await;
                // Up for two seconds, then down for two, and so on.
                let step = if (began.elapsed().as_secs() / 2) % 2 == 1 {
                    -40.
                } else {
                    40.
                };
                if this
                    .update(cx, |this, cx| {
                        this.bench_scroll(step);
                        // As the list does for a scroll wheel: only the chat draws again.
                        this.notify_chat(cx);
                    })
                    .is_err()
                {
                    return;
                }
            }

            bench_scene(Some("type"));
            let _ = this.update_in(cx, |this, window, cx| {
                this.jump_to_bottom();
                let focus = this.prompt.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            });
            let began = std::time::Instant::now();
            let mut n = 0usize;
            while began.elapsed() < SCENE {
                wait(Duration::from_millis(70)).await;
                let ch = if n % 7 == 6 { " " } else { "a" };
                n += 1;
                if this
                    .update_in(cx, |this, window, cx| {
                        this.prompt.update(cx, |p, cx| {
                            p.replace_text_in_range(None, ch, window, cx);
                        });
                    })
                    .is_err()
                {
                    return;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.prompt.update(cx, |p, cx| p.set_value("", window, cx));
            });

            bench_scene(Some("stream"));
            let Ok(tx) = this.update(cx, |this, cx| {
                for &id in &ids {
                    this.bench_turn(id, &dir);
                }
                cx.notify();
                this.tx.clone()
            }) else {
                return;
            };
            let streams = ids.clone();
            std::thread::spawn(move || {
                for &id in &streams {
                    for line in start_lines() {
                        let _ = tx.send(line_event(id, line));
                    }
                }
                let began = std::time::Instant::now();
                let mut n = 0;
                while began.elapsed() < SCENE {
                    std::thread::sleep(STREAM_EVERY);
                    for &id in &streams {
                        if tx.send(line_event(id, delta(n))).is_err() {
                            return;
                        }
                    }
                    n += 1;
                }
                for &id in &streams {
                    let _ = tx.send(Event::Done(Target::Session(id), true));
                }
            });
            wait(SCENE + Duration::from_millis(500)).await;

            bench_scene(None);
            let _ = cx.update(|_, cx| cx.quit());
        })
        .detach();
    }

    /// Scroll the open chat by `step` pixels, up for a positive step.
    fn bench_scroll(&mut self, step: f32) {
        self.chat_list.scroll_by(px(-step));
    }

    /// Start a turn of session `id` that the bench's made-up stream feeds.
    fn bench_turn(&mut self, id: u64, dir: &str) {
        let Some(session) = self.state.session_mut(id) else {
            return;
        };
        session
            .messages
            .push(Message::new(Role::User, "Explain the list, please."));
        session.status = Status::Working;
        let turn = session.messages.len() - 1;
        let running = crate::harness::turn::Turn::start(id, "claude", dir, turn, "bench");
        self.turns.insert(id, running);
    }
}
