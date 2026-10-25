//! `RUNO_BENCH=1`, with `RUNO_FRAME_STATS=<file>` and a throwaway `RUNO_HOME`: plays the
//! same scenes every time and quits, so frame times can be compared before and after a
//! change. It makes up a long chat (prose, lists, code and tables) and opens it, then:
//!
//! - `idle`: nothing happens, no agent runs;
//! - `scroll`: the chat scrolls up and down, a step every frame;
//! - `type`: a character goes into the composer every 70 ms, as fast typing does;
//! - `stream`: three agents stream replies, the open chat's and two in the background,
//!   through the same path a real agent's output takes;
//! - `stream-bg`: one agent streams in the background.
//!
//! Then, in groups that `RUNO_BENCH=<group>,<group>` picks from (any other value plays
//! them all):
//!
//! - `many`: 600 more sessions in 12 projects, one a Crew project; `many-idle`,
//!   `many-type` and `many-stream` as above;
//! - `crew`: that Crew project's chat (120 messages, 50 threads) with its Threads panel
//!   open: `crew-idle`, `crew-type`;
//! - `settings`: every Settings page in turn, a new one every 0.8 s;
//! - `diff`: a 6,000-line file with half its lines changed, open in the Changes panel:
//!   `diff-idle`, `diff-type`, and `diff-bg` with an agent streaming in the background;
//! - `file`: an 18,000-line file in the file viewer: `file-idle`, `file-bg`;
//! - `terminal`: `seq` printing 2.4 million lines in a terminal tab.
//!
//! The diff, file and terminal scenes work in a git repository made for them in the
//! system's temporary folder. The window must be on screen and uncovered: a covered
//! window draws nothing.
//!
//! Each scene is one line in the stats file (see `framestats::scene`), with how many times
//! each part of the window drew under it and how long that took.

use super::*;

/// How long each scene runs; idle ones, where little draws, run shorter.
const SCENE: Duration = Duration::from_secs(12);
const IDLE: Duration = Duration::from_secs(4);
/// The made-up sessions of `many`, and how many projects they are spread over.
const MANY: usize = 600;
const PROJECTS: usize = 12;
/// Crew's chat and its threads, in `many`'s Crew project.
const CREW_MESSAGES: usize = 120;
const THREADS: usize = 50;
/// Messages in the open chat, and in the two background ones.
const LONG: usize = 300;
const SHORT: usize = 40;
/// How often a streaming agent sends a piece of its reply.
const STREAM_EVERY: Duration = Duration::from_millis(30);

pub(crate) fn on() -> bool {
    std::env::var_os("RUNO_BENCH").is_some()
}

/// Whether the group of scenes `name` plays.
fn plays(name: &str) -> bool {
    let picked = std::env::var("RUNO_BENCH").unwrap_or_default();
    let groups = ["many", "crew", "settings", "diff", "file", "terminal"];
    let list: Vec<&str> = picked.split(',').map(str::trim).collect();
    // Crew's scenes need `many`'s Crew project.
    if !list.iter().any(|g| groups.contains(g)) {
        return true;
    }
    list.contains(&name) || (name == "many" && list.contains(&"crew"))
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

/// The window the bench plays in, and its app.
type Ctx = gpui::AsyncWindowContext;

/// Wait `d`.
async fn wait(cx: &Ctx, d: Duration) {
    cx.background_executor().timer(d).await;
}

/// Type into the composer for `span`, a character every 70 ms, then clear it. Returns
/// whether the app is still there.
async fn typing(this: &WeakEntity<RunoApp>, cx: &mut Ctx, span: Duration) -> bool {
    let _ = this.update_in(cx, |this, window, cx| {
        let focus = this.prompt.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    let began = std::time::Instant::now();
    let mut n = 0usize;
    while began.elapsed() < span {
        wait(cx, Duration::from_millis(70)).await;
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
            return false;
        }
    }
    this.update_in(cx, |this, window, cx| {
        this.prompt.update(cx, |p, cx| p.set_value("", window, cx));
    })
    .is_ok()
}

/// Stream a reply into each of sessions `ids` for a scene, through the path a real agent's
/// output takes, and wait for it to end. Returns whether the app is still there.
async fn streaming(this: &WeakEntity<RunoApp>, cx: &mut Ctx, ids: Vec<u64>) -> bool {
    let Ok(tx) = this.update(cx, |this, cx| {
        for &id in &ids {
            let dir = this
                .state
                .session(id)
                .map(|s| s.dir.clone())
                .unwrap_or_default();
            this.bench_turn(id, &dir);
        }
        cx.notify();
        this.tx.clone()
    }) else {
        return false;
    };
    std::thread::spawn(move || {
        for &id in &ids {
            for line in start_lines() {
                let _ = tx.send(line_event(id, line));
            }
        }
        let began = std::time::Instant::now();
        let mut n = 0;
        while began.elapsed() < SCENE {
            std::thread::sleep(STREAM_EVERY);
            for &id in &ids {
                if tx.send(line_event(id, delta(n))).is_err() {
                    return;
                }
            }
            n += 1;
        }
        for &id in &ids {
            let _ = tx.send(Event::Done(Target::Session(id), true));
        }
    });
    wait(cx, SCENE + Duration::from_millis(500)).await;
    true
}

/// Run `git` with `args` in `dir`, for the bench's repository.
fn git_in(dir: &std::path::Path, args: &[&str]) {
    let _ = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Bench",
            "-c",
            "user.email=bench@example.com",
        ])
        .args(args)
        .current_dir(dir)
        .output();
}

/// A repository with `big.txt` committed at 6,000 lines, then every other line changed,
/// and `long.rs`, 18,000 lines, not committed.
fn bench_repo() -> String {
    let dir = std::env::temp_dir().join(format!("runo-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    git_in(&dir, &["init", "-q"]);
    let line = |i: usize, changed: bool| {
        if changed {
            format!("let value_{i} = compute({i}, \"changed\"); // the new line {i}\n")
        } else {
            format!("let value_{i} = compute({i}, \"first\"); // line {i} as it was\n")
        }
    };
    let first: String = (0..6000).map(|i| line(i, false)).collect();
    let _ = std::fs::write(dir.join("big.txt"), first);
    git_in(&dir, &["add", "."]);
    git_in(&dir, &["commit", "-q", "-m", "Bench"]);
    let changed: String = (0..6000).map(|i| line(i, i % 2 == 1)).collect();
    let _ = std::fs::write(dir.join("big.txt"), changed);
    let long: String = (0..18000)
        .map(|i| format!("fn step_{i}(x: u64) -> u64 {{ x.wrapping_mul({i}) ^ 0x5bd1e995 }}\n"))
        .collect();
    let _ = std::fs::write(dir.join("long.rs"), long);
    dir.to_string_lossy().into_owned()
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
            // Let the window settle and the chat come up.
            wait(cx, Duration::from_secs(3)).await;

            bench_scene(Some("idle"));
            wait(cx, SCENE).await;

            bench_scene(Some("scroll"));
            let began = std::time::Instant::now();
            while began.elapsed() < SCENE {
                wait(cx, Duration::from_millis(4)).await;
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
                        this.notify_chat(false, cx);
                    })
                    .is_err()
                {
                    return;
                }
            }

            bench_scene(Some("type"));
            let _ = this.update(cx, |this, _| this.jump_to_bottom());
            if !typing(&this, cx, SCENE).await {
                return;
            }

            bench_scene(Some("stream"));
            if !streaming(&this, cx, ids.clone()).await {
                return;
            }

            bench_scene(Some("stream-bg"));
            if !streaming(&this, cx, vec![ids[1]]).await {
                return;
            }

            if plays("many") {
                let Ok(many) = this.update(cx, |this, cx| {
                    let many = this.bench_many();
                    cx.notify();
                    many
                }) else {
                    return;
                };
                wait(cx, Duration::from_secs(2)).await;
                bench_scene(Some("many-idle"));
                wait(cx, IDLE).await;
                bench_scene(Some("many-type"));
                if !typing(&this, cx, SCENE).await {
                    return;
                }
                bench_scene(Some("many-stream"));
                if !streaming(&this, cx, ids.clone()).await {
                    return;
                }

                if plays("crew") {
                    bench_scene(None);
                    let _ = this.update(cx, |this, cx| {
                        this.select(many);
                        this.show_panel = true;
                        this.right_tab = RightTab::Threads;
                        this.jump_to_bottom();
                        cx.notify();
                    });
                    wait(cx, Duration::from_secs(2)).await;
                    bench_scene(Some("crew-idle"));
                    wait(cx, IDLE).await;
                    bench_scene(Some("crew-type"));
                    if !typing(&this, cx, SCENE).await {
                        return;
                    }
                }
            }

            if plays("settings") {
                bench_scene(Some("settings"));
                let pages: Vec<SettingsPage> = SettingsPage::pages().collect();
                let _ = this.update(cx, |this, cx| {
                    this.open_settings();
                    cx.notify();
                });
                let began = std::time::Instant::now();
                for page in pages.iter().cycle() {
                    if began.elapsed() >= SCENE {
                        break;
                    }
                    wait(cx, Duration::from_millis(800)).await;
                    let _ = this.update(cx, |this, cx| {
                        this.settings_page = *page;
                        cx.notify();
                    });
                }
                let _ = this.update(cx, |this, cx| {
                    this.view = View::Session;
                    cx.notify();
                });
            }

            let wants_repo = plays("diff") || plays("file") || plays("terminal");
            let repo = if wants_repo {
                bench_scene(None);
                let Ok(id) = this.update(cx, |this, cx| {
                    let id = this.bench_repo_session(project);
                    cx.notify();
                    id
                }) else {
                    return;
                };
                wait(cx, Duration::from_secs(2)).await;
                Some(id)
            } else {
                None
            };

            if repo.is_some() && plays("diff") {
                let _ = this.update(cx, |this, cx| {
                    this.right_tab = RightTab::Changes;
                    this.open_change("big.txt");
                    cx.notify();
                });
                wait(cx, Duration::from_secs(1)).await;
                bench_scene(Some("diff-idle"));
                wait(cx, IDLE).await;
                bench_scene(Some("diff-type"));
                if !typing(&this, cx, SCENE).await {
                    return;
                }
                bench_scene(Some("diff-bg"));
                if !streaming(&this, cx, vec![ids[1]]).await {
                    return;
                }
            }

            if repo.is_some() && plays("file") {
                bench_scene(None);
                let _ = this.update(cx, |this, cx| {
                    this.right_tab = RightTab::Files;
                    this.open_preview("long.rs");
                    cx.notify();
                });
                wait(cx, Duration::from_secs(1)).await;
                bench_scene(Some("file-idle"));
                wait(cx, IDLE).await;
                bench_scene(Some("file-bg"));
                if !streaming(&this, cx, vec![ids[1]]).await {
                    return;
                }
            }

            if let Some(repo) = repo.filter(|_| plays("terminal")) {
                bench_scene(None);
                let _ = this.update(cx, |this, cx| {
                    this.right_tab = RightTab::Changes;
                    cx.notify();
                });
                wait(cx, Duration::from_secs(1)).await;
                bench_scene(Some("terminal"));
                let _ = this.update(cx, |this, cx| {
                    let _ = this.spawn_line_tab(repo, "seq 1 2400000", "Bench".into());
                    cx.notify();
                });
                wait(cx, SCENE).await;
            }

            bench_scene(None);
            let _ = cx.update(|_, cx| cx.quit());
        })
        .detach();
    }

    /// Scroll the open chat by `step` pixels, up for a positive step.
    fn bench_scroll(&mut self, step: f32) {
        self.chat_list.scroll_by(px(-step));
    }

    /// Make up `many`'s sessions, and its Crew project's chat and threads. Returns the
    /// Crew chat.
    fn bench_many(&mut self) -> u64 {
        let dir = std::env::temp_dir().to_string_lossy().into_owned();
        let projects: Vec<u64> = (0..PROJECTS)
            .map(|p| {
                let id = self.state.next_id();
                self.state.projects.push(store::Project {
                    id,
                    name: format!("Project {p}"),
                    path: dir.clone(),
                    mode: if p == 0 {
                        crew::MODE_CREW.into()
                    } else {
                        String::new()
                    },
                    ..Default::default()
                });
                id
            })
            .collect();
        let crew_project = projects[0];
        let chat = self.state.next_id();
        let mut threads = Vec::new();
        for n in 0..MANY {
            let id = self.state.next_id();
            let project = projects[n % PROJECTS];
            let thread = project == crew_project && threads.len() < THREADS;
            if thread {
                threads.push(id);
            }
            self.state.sessions.push(Session {
                id,
                project_id: project,
                title: format!("Made-up session {n}"),
                provider: "claude".into(),
                dir: dir.clone(),
                messages: (0..6).map(message).collect(),
                crew: thread.then(|| crew::ThreadInfo {
                    origin: crew::Origin::Crew { message: 1 },
                    brief: format!("Fix made-up issue {n} and open a pull request."),
                    checklist: (0..5)
                        .map(|k| crew::Step {
                            text: format!("Step {k} of the work"),
                            mark: if k < n % 5 {
                                crew::Mark::Done
                            } else {
                                crew::Mark::Todo
                            },
                        })
                        .collect(),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        let messages = (0..CREW_MESSAGES)
            .map(|i| {
                let mut m = message(i);
                match i % 6 {
                    3 => {
                        let start = (i / 6) * 3 % THREADS;
                        m = Message::new(Role::Agent, "Started threads for this.");
                        m.crew = Some(crew::Card::Started {
                            threads: threads.iter().skip(start).take(3).copied().collect(),
                        });
                    }
                    5 => {
                        let thread = threads[(i * 7) % THREADS];
                        m = Message::new(Role::Agent, format!("Made-up session: finished {i}."));
                        m.crew = Some(crew::Card::News { thread });
                    }
                    _ => {}
                }
                m
            })
            .collect();
        self.state.sessions.push(Session {
            id: chat,
            project_id: crew_project,
            title: "Crew".into(),
            provider: "claude".into(),
            dir,
            canvas: crew::CANVAS.into(),
            messages,
            ..Default::default()
        });
        chat
    }

    /// A session in the bench's repository (see `bench_repo`), opened. Returns its id.
    fn bench_repo_session(&mut self, project: u64) -> u64 {
        let dir = bench_repo();
        let id = self.state.next_id();
        self.state.sessions.push(Session {
            id,
            project_id: project,
            title: "Bench repository".into(),
            provider: "claude".into(),
            dir,
            messages: (0..SHORT).map(message).collect(),
            ..Default::default()
        });
        self.select(id);
        self.show_panel = true;
        id
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
