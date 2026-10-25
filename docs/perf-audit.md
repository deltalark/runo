# Runo performance audit (v0.5.8, 25 October 2026)

This is an audit only: no app code was changed. It finds and ranks the performance work
left in `native/runo-app` (GPUI) after these changes, which are already done:

- #235: startup no longer waits for the login shell;
- #244: post-turn checks run at low priority, one at a time;
- #250: the chat is virtualized, with a markdown cache;
- the cached panes plus the benchmark (`panes.rs`, `bench.rs`, `framestats.rs`).

**The short version.** Scrolling, typing and streaming are smooth now: every frame
under 4 ms at p95, at the display's full rate. What users will still feel:

- **Freezes from git work on the UI thread.** Sending a message runs a git checkpoint
  that takes 140 ms in a 400-file repo and about 1 s in a 10,000-file one. A turn ending
  runs about 5 more git commands.
- **Slow full-window redraws.** One costs 6–13 ms with many sessions, and **130 ms with a
  large diff open**. They happen 4 times a second whenever a background agent streams, and
  on every terminal update.
- **Typing in Crew's chat stutters.** 12% of frames are slow, p95 35 ms, max 105–136 ms.
- **Idle cost.** About 109 wakeups a second and 3–7% CPU while doing nothing, plus a stream
  of `lsof`, git and `gh` processes. Every timer keeps running when Runo is hidden.
- **Post-turn checks rebuild each Rust worktree from scratch.** The proof check runs plain
  `cargo test` with each worktree's own `target/` (`proof.rs:110`). During this audit, the
  check for this thread's docs-only turn spent over 15 minutes compiling a fresh debug
  build and filled 1.5 GB+ of disk (see [Found while measuring](#found-while-measuring)).

## How it was measured

- **Build.** One release build (`cargo build --release -j 2`; `CARGO_PROFILE_RELEASE_STRIP=none`
  so `sample` shows symbols) on an M-series Mac with 8 GB of RAM, drawing to a 75 Hz
  display.
- **Frame times.** `RUNO_BENCH=1 RUNO_FRAME_STATS=…` with an empty throwaway `RUNO_HOME`. The
  bench only covers idle, scroll, type and three streams. Extra scenes were added to
  `bench.rs` **for this build only**, behind `RUNO_BENCH_EXTRA`:
  - one agent streaming, visible and in the background;
  - 600 more sessions in 12 projects, one of them a Crew project with a 120-message chat
    and 50 threads;
  - Crew's chat with its Threads panel;
  - every Settings page in turn;
  - a 6,000-line file with half its lines changed, in the Changes panel;
  - an 18,000-line file in the file viewer;
  - `seq` printing 2.4 M lines in the terminal;
  - the Arcade.

  The patch was reverted and never committed. Its scene list is above, so it can be
  rebuilt; making the extra scenes a permanent part of the bench is the first quick win.
- **Two bench runs.** Both had the window uncovered on screen. The second reproduced the
  Crew numbers and took a `sample` profile during that scene.
  - **Caveat:** both on-screen runs overlapped a `cargo test` build that the installed Runo
    had started in this worktree as a post-turn check. The frames are main-thread bound, so
    comparisons between scenes hold, but absolute times may read a little high.
- **Idle.** `top -stats cpu,idlew` (idle wakeups a second), `sample`, and a poll of the
  child processes Runo starts. These used a copy of `~/.runo` with build-output cleanup,
  automations, phone, sync and updates switched off.
- **Startup.** `RUNO_TRACE_STARTUP=exit`, read from `$RUNO_HOME/logs`, with three data sets:
  - **real**: a copy of `~/.runo`, 40 sessions, 809 messages, 62 MB with turn logs;
  - **big**: the real copy ×10, 400 sessions;
  - **bench**: about 600 small sessions.
- **Reading the frame table.**
  - "Frame" is build + layout + paint of the whole window. "Root" is just
    `RunoApp::render`, since the panes are cached children.
  - "Slow" counts frames over 16.7 ms.
  - Framestats' "late" column uses a 12.5 ms (120 Hz) threshold, so at 75 Hz it flags every
    frame; it is left out.
  - "Panes drawn" counts how often each cached pane rendered during the scene.
- **Covered windows.** gpui stops a window's display link as soon as macOS reports it
  covered (`gpui-pre-macos-0.3.8/src/window.rs:2982`). A covered or hidden Runo draws no
  frames, which is good, so the background cost is all timers and processes. It also
  means benchmarks only measure with the window uncovered on screen.

## Results

### Frame times (release build, 75 Hz display)

| Scene | fps | Frame p50 / p95 / max (ms) | Slow | Root render max | CPU | Panes drawn (TopBar, Sidebar, Right, Chat) |
|---|---|---|---|---|---|---|
| idle, long chat open (300 messages) | 2 | 3.5 / 11.7 / 12.9 | 0/21 | 0.3 | **7%** | 21 / 21 / 21 / 21 |
| scroll the long chat | 75 | 2.3 / 2.8 / 3.8 | 0/900 | 0.3 | 25% | 2 / 2 / **900** / 900 |
| type in the composer | 14 | 1.7 / 2.9 / 4.5 | 0/168 | 0.3 | 5% | 4 / 4 / 4 / 4 |
| 3 agents stream (visible + 2 background) | 37 | 3.5 / 5.1 / 11.7 | 0/468 | 0.2 | 18% | 48 / 48 / 237 / 465 |
| 1 agent streams, visible | 37 | 3.6 / 5.4 / 11.7 | 0/467 | 0.2 | 18% | 44 / 44 / 236 / 465 |
| 1 agent streams, background | 4 | 5.9 / 9.0 / 9.8 | 0/56 | 0.3 | 4% | 53 / 53 / 41 / 41 |
| +600 sessions: idle | 0 | 6.5 / 6.6 / 6.6 | 0/3 | 0.2 | 2% | 3 each |
| +600 sessions: type | 14 | 1.9 / 3.5 / 6.5 | 0/166 | 0.4 | 6% | 2 each |
| +600 sessions: 3 agents stream | 33 | 3.9 / 6.5 / **23.6** | 1/483 | 0.4 | 19% | 64 / 64 / 252 / 479 |
| Crew chat + 50 threads: idle | 0 | 8.0 / 8.2 / 8.2 | 0/3 | 0.2 | 1% | 3 each |
| **Crew chat: type** (run 1 / run 2) | 14 | 2.1 / **35.8 / 105** · 2.1 / 35.4 / 136 | **26/192 · 24/191** | **58 · 50** | 12% | TopBar 28, Rail 28, Status 28; Sidebar/Right/Chat 2 |
| Settings, a new page every 0.8 s | 23 | 2.8 / 7.3 / 39 | 2/264 | 2.4 | 11% | TopBar/Rail/Status 264 |
| **Large diff open: idle** | 0 | **131** (every frame) | 2/2 | 0.2 | 3% | 2 each |
| Large diff open: type | 14 | 3.5 / 4.4 / **131** | 3/163 | 0.4 | 13% | 3 each |
| **Large diff open + 1 background agent** | 4 | **130 / 137 / 138** | **34/53** | 0.2 | **35%** | 39 / 39 / 50 / 36 |
| 18,000-line file open: idle | 0 | 7.9 / 8.2 / 8.2 | 0/3 | 0.2 | 1% | 3 each |
| 18,000-line file + 1 background agent | 4 | 7.8 / 13.3 / 23.7 | 1/65 | 0.5 | 6% | 64 / 65 / 57 / 57 |
| **Terminal, heavy output** | **4** | 12.7 / 13.5 / 13.7 | 0/46 | 0.4 | 21% | **46 each** (the whole window, every update) |
| Arcade open (no key pressed, so no round started) | 0 | — | — | — | 2% | — |

What the frame table shows:

- **The finished work holds up.** Scrolling runs at the display's full 75 fps with p95
  2.8 ms. Typing redraws the panes 4 times in 168 frames, with p95 2.9 ms. Streaming into
  the open chat stays under 6 ms at p95.
- **The Right pane redraws on every chat frame**, during scrolling (900/900) and streaming
  (about 236 in 12 s), because `notify_chat` notifies it too (`panes.rs:150`). With an
  empty panel that's cheap. With a diff in the panel it costs 130 ms, see the diff rows.
- **Full-window redraws** (all panes, as when a background agent streams) cost 6–9 ms with
  a few sessions, 8–13 ms with 600, and **130 ms with the 6,000-line diff open**. A
  background agent asks for one 4 times a second (`BACKGROUND_TICKS`, `main.rs:10025`), so
  any streaming agent plus an open diff means 34 of 53 frames are slow and the app runs at
  35% CPU.
- **Heavy terminal output** reaches the screen at only about 4 fps, and each update redraws
  the whole window (46 of 46 frames drew every pane).
- **Typing in Crew's chat** drops frames. Both runs reproduce it. The profile is below.
- The Arcade wasn't measured while playing, since the bench can't press keys. It costs
  nothing while paused.

**Profile: typing in Crew's chat.** A `sample` at 1 ms over the 11 s scene, main thread
only:

- About 730 samples were in `Window::draw`. Of those, 567 were in one cached `Pane`'s
  prepaint, and 397 of those were Taffy flexbox layout.
- Runo's own code (`RunoApp::render` and everything it calls) took under 60 samples.
- So each slow frame re-lays out a large element tree. It isn't Runo's code being slow.
- Typing redrew TopBar, Rail and Status 28 times. Those three are moved only by
  `motion`, since a notify on the app would have redrawn all seven panes, which suggests
  an animation that starts on a keystroke in Crew's composer.
- The likely suspects are the Crew chat's cards or the 50-row Threads panel, laid out
  again inside a list's prepaint.
- **Next step:** time each pane in `framestats` (wrap `Pane::render`/prepaint) to name
  the pane and the element.

### Idle (Runo covered, as in the background; "real" data; nothing running)

| What | Measured |
|---|---|
| Idle wakeups (`top` IDLEW, 30 s average) | **~109/s**. The code gives about 126/s; see the timer list below. |
| CPU | **2.9%** covered. **7%** with the window visible (bench `idle`, 2 fps). |
| Child processes in one idle minute | `lsof -nP -d cwd` **×~14** (about 0.3 s each, scans every process on the Mac); `lsof -iTCP -sTCP:LISTEN` ×2 (port scan, Browser on); `git rev-parse` / `rev-list` / `status` / `diff` (the 10 s `ship::scan` of the open session); `gh pr view` ×1 |
| Threads | 9 steady, plus short-lived `bg` threads |

The `sample` profile is almost all waits. What isn't is `posix_spawn` (the processes
above) and the CVDisplayLink thread.

Main-thread timers that keep running whether Runo is visible, hidden or idle:

| Timer | Period | Wakeups/s |
|---|---|---|
| `pump_browser` (`main.rs:2045`), even with no browser | 16 ms | 62.5 |
| `ui_rx` drain (`main.rs:1209`) | 30 ms | 33 |
| Event drain (`main.rs:1238`) | 50 ms | 20 |
| Menu bar tick (`menubar.rs:378`), even with the menu bar off | 250 ms | 4 |
| Dock badge (`main.rs:2031`), `health_tick` (`main.rs:2095`), approvals `read_dir` (`main.rs:2105`) | 500 ms each | 6 |
| Inboxes (`main.rs:2116`) | 2 s | 0.5 |
| Automations, then a full redraw (`main.rs:2133`) | 30 s | — |

Background threads add 100 wakeups a second per open terminal: the exit poll at
`pty.rs:156` runs every 10 ms. Each design canvas ever opened adds 4.7 a second
(`designdraw/view/agents.rs:20`). None of these loops checks whether the window is
visible or active.

### Startup (warm runs; the first run after a copy in brackets, the closest to cold without `sudo purge`)

| Data | Store opened | First frame | Responsive |
|---|---|---|---|
| real, 40 sessions | 6 ms | 222–267 ms | **245–285 ms** |
| big, 400 sessions | 50–53 ms (116) | 306–334 ms (403) | **357–384 ms (468)** |
| bench, ~600 small sessions | 28 ms (145) | 257–269 ms (525) | 278–285 ms (543) |

"Store opened" is measured from the start of `RunoApp::new`.

Startup is fine. The login shell is no longer waited on: the "login shell environment"
mark comes after "responsive" (#235 verified). About 140 ms passes before
`RunoApp::new`; gpui-kit's `components` init takes about 56 ms of it and creating the
window about 40 ms.

Only one part grows with use: every session file is read and fully parsed, every message
included, on the UI thread before the first frame (`main.rs:1351` → `store.rs:1125` →
`persist.rs:119-161`). That's about 0.12 ms per session warm.

Right after startup, `measure` also starts 3 git commands for every session, archived
ones included, with one OS thread per job (`main.rs:2141`, `ship.rs:355-368`). With 400
sessions that is about 1,200 git processes competing with the first seconds of use.

### Memory

| When | RSS |
|---|---|
| Bench start | 63 MB |
| After the scroll, type and stream scenes | 134 MB |
| From Settings, then the diff scenes | **409–454 MB** |
| From the file viewer on | back to 160–205 MB |
| Covered idle app (real data) | 52–187 MB |

Nothing kept growing in the time measured, but the roughly 300 MB jump while Settings
and the diff were shown needs a heap profile. A guess, not checked: the 4,000 unvirtualized
diff rows, or every Settings page's elements and images.

A one-hour streaming soak was not run. The code points to these long-run growth sources:

- every session's full message history stays in memory, and each save briefly holds a
  full second copy (`store.rs:1164`);
- each terminal keeps 10k lines of scrollback (`pty.rs:10`);
- a design canvas, once opened, is never freed, and keeps two polling loops
  (`designdraw/view/agents.rs:17`, `designdraw/view.rs:968`);
- `self.bg` starts an OS thread per job, with no limit.

Already bounded:

- the markdown cache (`KEEP_CHATS`=6, `chat_list.rs:21`);
- stderr, 200 lines (`main.rs:2523`);
- Runo's terminal output, 2,000 lines (`main.rs:2533`);
- Crew's `Seen`.

## Ranked findings

The table is ranked by expected user-visible gain. Effort: S is under a day, M is 1–3
days, L is a week or more.

| # | Issue | Evidence | Scenario | Likely fix | Effort | Risk | Expected gain |
|---|---|---|---|---|---|---|---|
| 1 | **Sending a message freezes the UI while git takes a checkpoint.** `read-tree HEAD` goes into a fresh temporary index, so `add -A` has no saved timestamps and must hash the whole worktree. Then `write-tree`, `rev-parse`, an index copy, 2× `commit-tree` and `update-ref`, all on the UI thread. | `turns.rs:155,211` → `git.rs:1776` (`checkpoint`), `git.rs:1736` (`snapshot_tree`). The same commands timed by hand: **140 ms** warm in this repo (398 files), **0.9–1.2 s** in a 10k-file repo; `git status` there takes 63 ms. | Every send, more so in big repos | Start from a copy of the real index, so timestamps are reused and only changed files are hashed. Run the checkpoint in `self.bg` and start the agent when it returns. | M | Med: undo depends on the checkpoint existing before the agent edits | Removes a 0.1–1 s freeze from every send |
| 2 | **Every full-window redraw lays out the whole diff.** `diff_rows` is parsed again each render; up to 4,000 rows are built, unvirtualized; `hidden.iter().find` runs per row (O(rows×folds)). | `main.rs:7943-7990`, `diff_view` `main.rs:7864`. Bench: **130 ms per frame** with a 6,000-line diff, and 34/53 frames slow while one background agent streams. | Changes or Review open while any agent runs; any notify | Cache the parsed rows by diff hash; draw them with `uniform_list`; precompute the fold map. | M | Low | 130 ms → under 10 ms on those frames; probably also most of the memory jump |
| 3 | **`notify_chat` redraws the Right pane on every chat frame.** That rebuilds `changes_tab` (where `s.touched()` makes a HashSet over all messages), the diff and the file tree. | `panes.rs:150`. Bench: Right drew 900/900 times during scrolling and about 236 times per stream scene. | Scrolling or streaming with the side panel open | Notify Right only when the event is for `self.peek`, the thread beside Crew; never during scroll. | S | Low | With #2, ends 130 ms frames while streaming with a diff open; less work on every scroll frame |
| 4 | **Background agents redraw the whole window 4×/s, and the 1 s tick redraws it while any turn runs.** | `main.rs:1337` (`BACKGROUND_TICKS`=5, `main.rs:10025`), `main.rs:1305-1314`. Bench: 1 background agent made every pane redraw about 50 times in 12 s, at 6–13 ms each (130 ms with a diff open). | Any agent running in the background, the common case with Crew | Notify only the Sidebar, Status and TopBar panes for background output and ticks; the running times already tick through `motion::ticking`. | S–M | Low | About 4 full redraws a second fewer per running agent |
| 5 | **Terminal output redraws the whole window, and only about 4 times a second.** `event_shown` returns `true` for `Pty` and `Line(Terminal)`. `pty_view` allocates a `String` per cell, joins each row, runs the link regex per row, and holds the parser lock throughout. | `main.rs:2424` (`_ => true`), `terminal.rs:769-787,876`. Bench: 4 fps, 46/46 frames drew every pane, 12.7 ms each, 21% CPU. | Builds, tests, logs in the terminal | Give the terminal its own cached view and notify only it. Build each row's text in one reused `String`. Rebuild rows only when the parser says they changed. | M | Low | Smooth terminal output at display rate, at a fraction of the CPU |
| 6 | **Typing in Crew's chat drops frames.** | Bench: p95 35 ms, max 105/136 ms, 26/192 and 24/191 slow in two runs. Profile: ~70% in Taffy layout inside one pane's prepaint; Runo's code under 10%. TopBar, Rail and Status drew 28 times. Contributing: `crew_cards_hidden` does a linear `state.session()` per message (`crew_view.rs:1031`, `store.rs:1178`). | Talking to Crew with many threads | First time each pane in `framestats` to find which one re-lays out. Then stop the keystroke-triggered motion, or cache the Threads list/cards; add a set of live session ids for `crew_cards_hidden`. | S to find, S–M to fix | Low | Crew's composer as smooth as the normal one (p95 3 ms) |
| 7 | **The idle app wakes about 109 times a second.** Polling loops never sleep and none stops when the window is hidden. | `main.rs:2045` (16 ms browser pump, which runs even with no browser), `main.rs:1209` (30 ms), `main.rs:1238` (50 ms), `menubar.rs:378` (250 ms, even with the menu bar off), 500 ms timers at `main.rs:2031,2095,2105`. Measured: 109 wakeups/s, 2.9% CPU covered, 7% visible. | Battery, all day | Start the browser pump only while a browser is shown. Wait on async channels (`smol::channel`) instead of polling `try_recv`. Skip the menu bar tick when it's off. Slow everything down when the window is inactive. | S (pump) + M (channels) | Low–Med: the 50 ms batching paces repaints | Idle wakeups 109 → under 10 a second; idle CPU near 0 |
| 8 | **Idle child processes:** `lsof -d cwd` once per worktree on each build-cleanup pass; a git `ship::scan` every 10 s (2 s while watching) even when hidden; `gh pr view` every 60 s; `gh` ×4 every 5 min; `lsof -iTCP` every 5 s with Browser on. | `buildclean.rs:304` (`free` → `cwds()` per job), `space_ui.rs:198`; `main.rs:1259,1302`; `main.rs:1260` → `ship.rs:840`; `github.rs:666-719`; `browser.rs:66,185`. Measured in one idle minute: about 14 `lsof -d cwd` at about 0.3 s each, 2 `lsof -iTCP`, git scans, `gh`. | Idle, battery, and background CPU for the rest of the system | Call `cwds()` once per pass, not per worktree. Skip scans and `gh` when the window isn't active (activation already refreshes). Don't notify unless something changed. Add an in-flight guard to `refresh()` (`main.rs:2311`). | S | Low | Most idle process spawns gone |
| 9 | **Turn end runs about 5 git commands on the UI thread** (`write_memory`): `is_repo`, `git::changes` (which reads every untracked file), `git log`, `git::exclude`, plus file writes. They stack when several threads finish at once. | `main.rs:2810` → `handoff.rs:512-541`. Not timed; from the `git status` and `diff` times above, an estimated 50–300 ms. | Every turn end, most with Crew | Do it in `self.bg`; cache `exclude` per repo. | S | Low | Removes a hitch at each turn end |
| 10 | **Hover handlers call `window.refresh()`**, which skips every pane cache, so all 7 panes rebuild on each enter or leave of a button, rail item or chat message. That includes messages sliding under a still pointer while scrolling. | `shell.rs:852` (`hover_wash`), `shell.rs:905` (`reveal_group`); used at `main.rs:4207,4243,5423,5628`. | Mouse movement; scrolling with the pointer over the chat | Capture `window.current_view()` while drawing and notify that view only. | S | Low | Hover costs a pane, not the window |
| 11 | **The "needs you" count is O(S²·A) and allocates.** `need()` → `approval_for` → `approval_session` scans every session for each approval. It runs per session, per sidebar row, three times on Home, in `attention()` every root frame, and every 500 ms for the Dock. `waiting_on` clones to test `is_some`; `format!` builds keys for lookups. | `shell.rs:1252,1345,1109`, `alerts.rs:602,1031-1047,1238`, `home.rs:260,530-541`, `assistant.rs:1055`, `main.rs:2037,4441,4888` | Many sessions with approvals waiting | Build a session → approval map once per frame; return a borrowed waiting kind; use typed keys. | M | Low | Sidebar and Home cost no longer grows with the square of the session count |
| 12 | **Pulsing dots and the working indicator redraw their whole pane about 30 times a second while any agent runs.** `chat_rows` walks every message for each motion frame. | `shell.rs:448` (`live_dot`), status-bar dot `main.rs:6014`, sidebar `main.rs:4452,4900`, `chat_list.rs:483` | Any agent running | Make the dot or indicator its own tiny entity, so motion notifies only it. | M | Low–Med | Smaller redraws while agents run |
| 13 | **Repeating `with_animation` outside the motion clock** runs at display rate and doesn't stop when the window is inactive: the assistant mark while anything waits on you, and the unsaved-file pulse. | `assistant.rs:1580-1585`, `fileview.rs:577-580`, `settings_ui.rs:2584`, `handoff.rs:773` | Idle with something waiting; editing a file | Use `.animated()` (`main.rs:778`), which follows the motion clock and pauses. | S | Low | Stops continuous root redraws in those states |
| 14 | **Saving clones the entire State** (every message) on the UI thread, up to every 500 ms. The save thread then serializes every session to find which ones changed. | `health.rs:277-281` → `store.rs:1164`; `persist.rs:78-90` | Long histories; costs more as history grows | Track dirty session ids, or keep `Arc<Session>`, and write only what changed. | M | Med | UI-thread save cost stops growing with history |
| 15 | **Home starts a git process (`git::is_repo`) every render.** Home redraws 4×/s while background agents run. | `home.rs:738,1057`; also `home.rs:548` (`is_dir`) | Home open while agents run | Use the cached `checks::is_repo` / `checks::is_dir`. | S | None | About 4 git processes a second fewer |
| 16 | **Startup parses every session's full history** before the first frame, and `measure` runs 3 git commands for every session, archived included, one thread each. | `persist.rs:119-161`; `main.rs:2141` → `ship.rs:355-368`. Measured: store 6 ms (40 sessions) → 50–116 ms (400). | Heavy users' startup and first seconds | Now: measure only visible, non-archived sessions through a pool of 2–4 (S). Later: load headers first and messages on open (L). | S / L | Low / Med (search, Home and Crew read messages) | 50–100 ms off startup at 400 sessions; no git burst right after launch |
| 17 | **The composer redoes setup work every root frame.** `caps::get` clones all of `Caps`; `selections()` clones a HashMap; `model_note` and `store::today()` run per model. | `caps.rs:119,765-781`, `store.rs:133`, `main.rs:6394` | Typing (still measured fine, p95 3 ms) | Keep caps in an `Arc`; cache controls per (provider, selections). | S–M | Low | Small; headroom on slow Macs |
| 18 | **The sidebar builds the project tree and then drops it** when "Needs you" or group-by-status is on. It also scans every session per project (lowercasing three strings each), runs `threads.iter().any` per session, and draws rows unvirtualized. | `main.rs:4781-5037,4967,5003`, `workspace.rs:108-118` | Many sessions | Decide the grouping first; group in one pass; virtualize later. | S–M | Low | Full redraws with 600 sessions take 8–13 ms now; this is part of that |
| 19 | **Settings clones `AppSettings` and `Discovered` every frame.** With a search typed, it builds every page each keystroke; it scans PATH with `canonicalize` per agent and runs `exists()` per row. | `settings_ui.rs:2996-2997,3077,578,2785` | Settings (page switch max 39 ms) | Cache sections by (settings version, query); move disk checks into discovery. | M | Low | Faster Settings search and page switches |
| 20 | **Small items:** | | | | | | |
| | • New Session stats every PATH folder for every agent each frame | `agent.rs:109-123`, `main.rs:7106`, `agent_pick.rs:74` | | Use `caps.installed` | S | | |
| | • The Files tab sorts the whole file list each render | `files.rs:33-39,70-77` | | Cache the tree | M | | |
| | • The terminal output panel joins 300 lines into markdown each render | `main.rs:8735,8762-8792` | | Cache by line count | S | | |
| | • The Arcade asks for animation frames without pausing when the window loses focus | `arcade.rs:2063` | | Pause on deactivation | S | | |
| | • The approvals `read_dir` runs every 500 ms even with no agent running | `approvals.rs:184-196` | | Skip when `handles` is empty | S | | |
| | • Automations trigger an unconditional full redraw every 30 s | `main.rs:2133` | | Notify only on change | S | | |
| | • Every terminal's exit poll runs every 10 ms | `pty.rs:139-156` | | Blocking `wait()` | S | | |
| | • `TurnLog::start` blocks on the writer flush, and resume reads up to 20 MB on the UI thread | `harness/log.rs:91,271` | | Move both to the writer thread | S | | |
| | • `focus_shell` notifies from inside render | `main.rs:9263` | | | S | | |
| | • `connections::all()` clones the whole list just to test `is_empty` | `connections_ui.rs:524` | | | S | | |
| | • Opening a diff (`open_change` → `load_diff`) and a file (`open_file`) read synchronously on the UI thread: 8–10 ms for the bench files, more on slow disks | `main.rs:2343,7918`, `fileview.rs:179-215` | | | S | | |

## Found while measuring

This ranks with #1–#3 for anyone working on Rust projects.

**Post-turn checks build every Rust worktree from scratch.** `proof.rs:110` adds a plain
`cargo test` for any folder with a `Cargo.toml`. It runs in the session's worktree with
no `CARGO_TARGET_DIR`, so each worktree compiles every dependency into its own `target/`.

What happened during this audit:

- The installed Runo (`/Applications/Runo.app`, pid 99061) started `cargo test` in this
  thread's worktree after a turn that changed only Markdown.
- It was still compiling after 15 minutes.
- It had written 1.5 GB to `target/` by then, on a disk with under 5 GB free.
- It ran alongside the user's other work, and alongside this audit's bench.

With Crew starting several threads, every thread repeats the full build.

| | |
|---|---|
| **Effort** | S–M |
| **Risk** | Low–Med (a shared target serializes builds through Cargo's lock) |
| **Expected gain** | Minutes of CPU and GBs of disk saved per thread |

Fixes:

- Point the checks at one target dir per project, for example `<repo>/target`. Either set
  `CARGO_TARGET_DIR`, or reuse the repo's own dir when the worktree is under
  `.runo-worktrees`.
- Skip the checks when the turn changed no source files: here it changed only docs.
- Optionally run the checks at a lower priority with `nice`, since the queue from #244
  already runs them one at a time.

## Quick wins (S effort, low risk)

0. Share one Cargo target dir per project for post-turn checks, and skip the checks for
   turns that touched no source files (see [Found while measuring](#found-while-measuring)).

1. Stop `notify_chat` from redrawing the Right pane unless the event is for the peeked
   thread (#3).
2. Start the 16 ms browser pump only while a browser is shown. That's more than half the
   idle wakeups (#7).
3. Move `write_memory` at turn end into `self.bg` (#9).
4. Call `buildclean::cwds()` once per cleanup pass. Skip `ship::scan`, `gh pr view` and
   GitHub polling while the window is inactive, and notify only when something changed
   (#8).
5. Notify only the hovered view in `hover_wash` / `reveal_group` instead of
   `window.refresh()` (#10).
6. Use `checks::is_repo` / `checks::is_dir` in Home's render (#15).
7. Switch the repeating `with_animation`s to `.animated()` (#13).
8. At startup, `measure` only non-archived sessions, through a small pool (#16, first
   half).
9. Skip the menu bar tick when the menu bar is off; skip the approvals poll when nothing
   runs (#7, #20).
10. Find the pane behind the Crew typing stalls by timing each pane in `framestats` (#6).
    Keep the extra bench scenes as a permanent part of `bench.rs` so these numbers can be
    rerun after each fix.

## Bigger projects

1. **Take the checkpoint off the UI thread and make it incremental** (#1, M). This is the
   largest freeze a user hits, up to a second on every send in a big repo.
2. **Virtualize and cache the diff view** (#2, M). It turns 130 ms frames into normal ones
   and probably accounts for most of the memory jump.
3. **Targeted notifies instead of whole-window redraws** (#4, #5, #12, M). Background
   output, ticks and terminal output should go only to the panes that show them. The
   terminal should become its own cached view, with row text built without per-cell
   allocations.
4. **Event-driven main loop** (#7, M). Replace the 30 ms and 50 ms `try_recv` polling with
   async channels, and pause or slow every periodic timer while the window is hidden.
5. **Incremental saves** (#14, M). Write only dirty sessions instead of cloning the whole
   State on the UI thread.
6. **Needs-you index** (#11, M). Compute attention once per change instead of O(S²) per
   frame.
7. **Lazy session history** (#16, second half, L). Load session headers at startup and
   messages on open. It caps startup time and memory as history grows.

## Checks of what's already done

| Change | Status | Evidence |
|---|---|---|
| #235, startup without the login shell | Works | The "login shell environment" mark arrives after "responsive"; responsive at 245–285 ms with real data. |
| #250, virtualized chat and markdown cache | Works | Scrolling a 300-message chat: 75 fps on a 75 Hz display, p95 2.8 ms, about 3 rows drawn per frame. |
| Cached panes | Work | Typing in the composer redrew the panes 4 times in 168 frames, p95 2.9 ms. Also typing with 600 sessions: p95 3.5 ms. |
| Cached panes: exceptions | See findings | The Right pane is still redrawn by `notify_chat` (#3); hover refreshes skip the caches (#10). |
| #244, low-priority post-turn checks | Queue works; builds are costly | The bench's turns run in a temporary folder without git, so no checks ran there. The installed Runo's check for this thread compiled a fresh debug target, see [Found while measuring](#found-while-measuring). |
