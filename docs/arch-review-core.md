# Architecture review: Runo core app (`native/runo-app`)

Reviewed 2026-10-25 at `5edcf99` (v0.5.9). Read-only: no code changed and nothing built.
Paths are relative to `native/runo-app/src/` unless noted. The counts are grep-based
estimates, accurate to within a few percent.

## At a glance

| Measure | Value |
|---|---|
| Source files in `src/` | 133 (132k lines) |
| `main.rs` | 10,089 lines, 128 `mod` declarations |
| `RunoApp` fields | **318** (`main.rs:255`–`776`) |
| `impl RunoApp` blocks | **81, in 71 files** |
| Methods on `RunoApp` | about 1,400 (`fn` at impl indent in those files; 106 in `main.rs`) |
| Files with `use super::*` / `use crate::*` | 77 |
| `cx.notify()` calls | 776, all on `RunoApp` (only 14 targeted `notify_parts`/`notify_chat`) |
| `self.bg(...)` call sites | about 778 |
| `save()` call sites | 141, all marking `save_due` |
| `Entity<…>` view types | `RunoApp`, `Pane`×8, `Ticker`, `Arcade`, `menubar_panel::Panel`, plus widget states (`InputState`×43, `TextareaState`×12) |
| `~/.runo` on this machine | `state.json` 92 KB, `sessions/` 83 MB (57 sessions; the biggest `session.json` is 765 KB), `agents/` 338 MB |

The app is reliable in practice. Saves are atomic, files that fail to load are moved
aside, few `unwrap`s reach UI code, and frame-cost work (the virtual chat list, cached
panes, batched events) has clearly paid off. The structural problem is that almost
everything is one object, `RunoApp`. Every feature reads and writes its fields directly,
and one `notify` redraws everything. Most of the gaps below follow from that.

---

## Ranked gaps

### 1. `RunoApp` is a god object (318 fields, ~1,400 methods, 71 files) — **High**
- **Evidence:** the struct is at `main.rs:255`–`776`. Field prefixes show at least 15
  features flattened into it: `git_*` (≈45 fields, `main.rs:256`–`300`), `pr_*`, `pb_*`
  (playbooks), `auto_*`, `file_*`, `diff_*`, `term_*`, `browser_*`, `settings_*`,
  `peek_*`, `arcade*`, `tour*`, `notes_*`, `new_*`. `impl RunoApp` appears in 71 files
  (for example `alerts.rs:630`, `gitplace.rs:60`, `ship.rs:353`, `settings_ui.rs:2000`/`3966`,
  `shell.rs:1227`/`3025`/`3073`/`3496`). 77 files glob-import the crate root.
- **Impact (developers):** module boundaries exist only as file names. Any feature can
  read or change any other feature's state, so it is hard to tell what owns a field or
  what a change will break. There are no import cycles only because there are no real
  imports: every module is effectively one module. Compile units are large, and any
  change to the struct recompiles the whole crate. New contributors have to learn the
  whole object.
- **Fix:** move features into owned sub-structs, then into entities. Start with the
  largest prefixes: `GitPlace { tab, branches, history, … }`, `PrState`,
  `PlaybookEditor`, `FileEditor`, `TerminalDock`. A step like `self.git_tab` →
  `self.git.tab` is mechanical. Once a group is self-contained, make it an `Entity<_>`
  with its own `Render` (see gap 2). Don't add new `git_*`-style fields to `RunoApp`.
- **Effort:** L (incremental, one feature per PR). **Risk:** low per step if done as
  mechanical renames. The risk is merge churn with parallel threads, so sequence the
  steps.

### 2. One notify redraws every pane; most of the UI is still one tree — **High**
- **Evidence:** `panes.rs:7`–`10` says it outright: every `cx.notify()` on the app
  "draws all of them again". Each `Pane` observes the whole app (`panes.rs:215`,
  `cx.observe(&app, |_, _, cx| cx.notify())`). Pane renders call back into `RunoApp`
  (`Pane::render` → `draw_part`, `panes.rs:303`). There are 776 `cx.notify()` calls
  and only 14 targeted ones (`notify_parts`, `notify_chat`, `notify_background` at
  `panes.rs:263`–`293`). Only 6 types implement `Render` (`main.rs:8957`, `panes.rs:87`
  and `195`, `arcade.rs:2059`, `menubar_panel.rs:871`, `shell.rs:1209`). Settings, Git,
  Data, Crew, Home and the right panel are functions on `RunoApp`, rendered inside
  whichever pane holds them.
- **Impact (users):** typing in the composer, a git scan finishing or a stream event
  each rebuild the sidebar, top bar, rail, right panel and chat at once. The cache only
  helps when *something other than the app* changed. On a large session (765 KB here)
  or a busy right panel, this costs frame time on every keystroke and event.
- **Fix:** (a) In the short term, replace bare `cx.notify()` in hot paths (stream
  events, PTY output, composer input, scan results) with `notify_parts(&[…])`, and stop
  the `Pane` observer from redrawing on every app notify. Needs a "dirty parts" set on
  `RunoApp` that `notify` paths fill. (b) In the longer term, the entities from gap 1
  own their state and notify only themselves.
- **Effort:** M (a), L (b). **Risk:** medium. A missed notify leaves a stale pane, so
  add a debug assert or bench check that each state change marks a part.

### 3. Every save clones all of state, including every message of every session — **High**
- **Evidence:** `save()` only sets `save_due` (`main.rs:2179`). Every 500 ms
  `health_tick` → `save_now` → `State::save` (`health.rs:276`–`280`, `620`), which runs
  `persist::queue(&data_dir(), self.clone())` (`store.rs:1189`). That is a deep clone
  of all sessions with all messages, on the UI thread. The writer thread then renders
  every session to pretty JSON and hashes it to find what changed (`persist.rs:78`–`91`,
  `render` at `persist.rs:29` goes through `serde_json::Value`, so it allocates twice).
- **Impact (users):** cost grows with *total* history, not with what changed. During a
  streaming turn, `save_due` is set constantly, so the UI thread clones megabytes
  (83 MB of sessions here, with the loaded messages) twice a second. This is a likely
  source of hitches that grow as people use Runo longer.
- **Fix:** keep a per-session dirty flag or generation counter (set wherever
  `session_mut` writes). Queue only the dirty sessions plus the index, and clone just
  those. Render sessions with `serde_json::to_string_pretty` directly (keys are already
  in struct order) and skip the hash when the session is clean. Optionally store
  `messages` behind `Arc<Vec<Message>>` so a clone is cheap.
- **Effort:** M. **Risk:** medium. A missed dirty mark means lost data, so keep a full
  write on quit and on a slow timer as a safety net.

### 4. Session history grows without limit, and all of it loads at startup — **High**
- **Evidence:** `Session.messages: Vec<Message>` (`store.rs:408`) is never trimmed. A
  grep finds no truncation, archive or pagination. `persist::read` (`persist.rs:119`–
  `160`) reads and parses every `session.json` synchronously at startup, including
  archived sessions. Session lookup is a linear `Vec` scan (`store.rs` `session()` /
  `session_mut()`, 142 call sites).
- **Impact (users):** startup time, memory, and the save cost in gap 3 all grow with
  lifetime usage. On this machine there are 57 sessions, one of them 765 KB of JSON.
  Heavy users will feel it within months.
- **Fix:** split each session into a small header (title, status, project, flags,
  usage) that stays in `state.json` or the index, and messages that load lazily when a
  session is opened. Messages can be stored append-only (`messages.jsonl`) so a save
  appends instead of rewriting. Use a `HashMap<u64, usize>` index for lookups.
  Archived sessions never load their messages.
- **Effort:** L. **Risk:** medium-high. This is a format migration (see gap 6), so it
  needs a reader for the old layout.

### 5. `bg` spawns an unbounded OS thread per job, with no cancellation or panic path — **High**
- **Evidence:** `ship.rs:355`–`368`: each call is `std::thread::spawn`. With about 778
  call sites, git scans, PR polls and file searches can stack up threads during bursts
  (window focus, project switches). There is no token or generation, so a stale result
  is applied unless the `done` closure checks for that by hand (`apply_scan` checks
  `selected_file == asked`, `ship.rs:378`; many callers don't). Release builds use
  `panic = "abort"` (root `Cargo.toml:25`). A panic in *any* `bg` worker, or in a
  poisoned `lock().unwrap()` (27 in `browser_engine.rs`, e.g. `:551`, `:569`, `:576`),
  kills the whole app. Up to 2 s of unsaved state is lost (`persist.rs:17`, `MAX_DELAY`).
- **Impact (users):** occasional stale UI (results for the previous project or branch),
  thread spikes, and a hard crash for a bug in background code that should only have
  failed one job. On quit, in-flight jobs are dropped silently. That is fine for reads,
  but some `bg` jobs write (git commit, PR create), and the user isn't told those were
  cut off.
- **Fix:** route `bg` through `cx.background_executor()` or a small fixed pool with a
  per-kind key (`bg_keyed("scan", …)` drops or ignores superseded jobs). Hand out a
  generation token that `done` checks automatically. Wrap `work()` in `catch_unwind`
  inside `bg` and report through `fail()`. That requires `panic = "unwind"`, or at least
  isolating the bg pool. For writes, keep a "jobs in flight" count that `before_quit`
  (`main.rs:2957`) waits on or warns about.
- **Effort:** M. **Risk:** medium. Changing the panic strategy affects binary size and
  every crate, so check the vendored GPUI first.

### 6. No schema version on any persisted file — **Medium**
- **Evidence:** there is no `version` or `schema` field in `state.json`, `session.json`
  or `settings.json` (grep of `store.rs`, `persist.rs`, `settings.rs`). Migrations run
  implicitly on load (`migrate_check_level`, `store.rs:1121`, keyed on
  `check_level.is_none()`). Compatibility relies on `#[serde(default)]`, which is
  thorough (`AppSettings` and `Project` are container-level or per-field defaults,
  `settings.rs:11`/`63`, `store.rs:47`–`109`).
- **Impact:** forward compatibility is fine, but a *downgrade* silently drops unknown
  fields on the next save. The person runs an older Runo, and the new data is gone.
  Every new migration has to invent its own "is this old?" heuristic, and gap 4 can't
  be done safely without a version.
- **Fix:** add `#[serde(default)] format: u32` to the index, sessions and settings. Run
  migrations in order from `format` up to `CURRENT`. If `format > CURRENT`, open
  read-only or keep a copy (`*.json.newer`) before writing.
- **Effort:** S. **Risk:** low.

### 7. Blocking work on the UI thread in render and input paths — **Medium**
- **Evidence:**
  - `project_chooser` → `chooser_summary` does `std::fs::read_dir` while rendering
    (`shell.rs:3085`, `3199`).
  - Pasting an image calls `save_image` → `attachments_dir` → `exclude_runo_folder`.
    That runs `git rev-parse` as a subprocess, reads and writes `info/exclude`, and
    writes the image, all synchronously in `attach` (`turns.rs:1150`–`1160`,
    `2604`–`2640`).
  - `settings::AppSettings::save` writes `settings.json` synchronously
    (`settings.rs:407`).
  - The quit path blocks for up to 5 s (`store.rs` `flush`), which is acceptable.
- **Impact (users):** a hitch when opening the chooser on a slow or network drive, and
  a visible pause when pasting a large image into a repo on a cold disk.
- **Fix:** cache chooser summaries and refresh them in `bg`. Move `exclude_runo_folder`
  and the file write into `bg`, adding the attachment chip right away with a pending
  state. Send settings saves through the existing `persist` writer.
- **Effort:** S. **Risk:** low.

### 8. Saves are atomic but not durable — **Medium**
- **Evidence:** `persist::write_text` (`persist.rs:42`) and `store::write_json`
  (`store.rs:1089`) do write-then-rename with no `File::sync_all` before the rename and
  no directory fsync. `move_aside` (`store.rs:1071`) keeps corrupt files, which is
  good, but `read_json_or_backup` then starts from defaults with no user-visible
  notice beyond the log.
- **Impact (users):** after a power loss or kernel panic on macOS/APFS, a renamed file
  can be zero-length. The next start moves it aside and the person sees empty
  sessions or settings with no explanation.
- **Fix:** `sync_all()` on the temp file before the rename (cheap after gap 3 cuts
  write volume). When a load falls back to `move_aside`, raise a `notice` ("Couldn't
  read your settings; kept a copy at …") rather than only logging it.
- **Effort:** S. **Risk:** low.

### 9. Global mutable statics for state shared across threads — **Medium**
- **Evidence:** about 30 `static Mutex`/`RwLock`/`OnceLock<Mutex>` instances:
  `dock.rs:8` (`SHOWN`), `phone.rs:433` (`BRIDGE` holding a `JoinHandle`) and `:508`,
  `connections.rs:688`/`777`/`786`/`1184`/`3115`, `cove.rs:106`/`319`/`374`/`435`,
  `mcp.rs:241`–`242`, `caps.rs:66`/`69`, `planlimit.rs:158`, `alerts.rs:18`,
  `browser_engine.rs:95`. Most use poison-tolerant locking
  (`unwrap_or_else(|e| e.into_inner())`, 23 places), but 29 use `lock().unwrap()`.
- **Impact (developers):** hidden global state is invisible to the render model. A
  change to these statics doesn't notify anything, so it relies on polling ticks
  (`health_tick`, `wake::pace`). They also block test isolation (`store.rs:1742`
  needs an `ENV_LOCK`) and could deadlock if a lock is held across a `bg` boundary.
- **Fix:** make the poison-tolerant pattern universal (a `lock()` helper). Move statics
  that the UI reads into GPUI `Global`s, or into entities updated through `ui_tx`, so
  changes notify instead of being polled. Keep true process-wide caches (`caps`,
  `copies`) as they are.
- **Effort:** M. **Risk:** low-medium.

### 10. Providers are data-driven in name only — **Medium**
- **Evidence:** `PROVIDERS` derefs to the signed manifest (`agent.rs:21`–`30`,
  `registry.rs:22`–`27`, `agents.json`), and capability gates are manifest facts
  (`agent.rs:33`–`45`). But about 80 hard-coded match arms on provider ids remain
  (`"claude" =>` ×21, `"codex" =>` ×19, `"opencode"` ×13, `"cursor"` ×8, …) across
  20 files: `agent.rs:1290`/`1410`/`1515`/`1534`, `usage.rs:166`, `caps.rs`, `mcp.rs`,
  `signin.rs`, `modelcheck.rs`, `history.rs`, `settings_ui.rs`, `proof.rs` and others.
- **Impact (developers):** adding a provider in the manifest gets a row in the picker.
  Making it work fully (sign-in, usage, history import, MCP config, model check) means
  finding the matches across 20 files, and the compiler won't point to them because
  they match strings with `_ =>` fallbacks. `docs/providers.md` should list them, and
  it can't stay accurate.
- **Fix:** add a `trait ProviderAdapter` (or a struct of fn pointers) per provider in
  `agent/<id>.rs`, with methods for argv, stream parsing, usage, sign-in, MCP config
  path and history path. Have `registry` map id → adapter. Make the match arms go
  through the adapter. The manifest stays the source of feature gates.
- **Effort:** M-L. **Risk:** medium, because stream parsing is subtle. Move one
  provider per PR, behind the existing tests.

### 11. Adding a setting touches 3–4 places; adding a view touches `RunoApp` — **Low**
- **Evidence:** `AppSettings` has about 83 public fields (`settings.rs:64`). A new one
  needs the field, its default, a row in `settings_ui.rs` (4,255 lines), and the search
  index or `settings_areas` (`settings_ui.rs:2061`–`2098`). New views follow the
  existing pattern: fields on `RunoApp`, an `impl RunoApp` render fn, a `View`/`Mode`
  variant and a `Part` routing case.
- **Impact (developers):** it's easy to add a setting that search can't find, or a view
  that redraws with everything (gap 2).
- **Fix:** a declarative `SettingDef { key, title, keywords, page, kind }` table that
  drives both the row and the search. A "new view" template that is an `Entity` with
  its own state.
- **Effort:** M. **Risk:** low.

### 12. Check-language detection is a hard-coded chain — **Low**
- **Evidence:** `proof::detect_at` (`proof.rs:212`–`275`) has `if has("package.json")`
  … `Cargo.toml` … `go.mod` … `pyproject.toml`/`setup.py`/`requirements.txt` …
  `Makefile`. There's nothing for Swift, Gradle/Maven, .NET, Ruby or mixed monorepos
  (it stops at the root folder). `migrate` and `commands` live in the same file.
- **Impact:** projects in those languages get no checks, or only `make`. A monorepo
  with `apps/web/package.json` gets nothing.
- **Fix:** a `const DETECTORS: &[Detector { marker, quick, full }]` table, plus an
  optional per-project `.runo/checks.toml` override. Scan one level of workspace
  members.
- **Effort:** S-M. **Risk:** low.

### 13. Errors swallowed with `let _ =` — **Low**
- **Evidence:** 438 `let _ =` across the crate. Most are deliberate (channel sends after
  close, best-effort cleanup), but some hide real failures: `move_aside`'s `rename`
  (`store.rs:1078`; if it fails, the corrupt file is overwritten by defaults on the next
  save), `create_dir_all` before saves (`persist.rs:44`, `store.rs:1091`), and
  `remove_session`'s removals (`persist.rs:110`–`116`). User-facing errors go through
  `fail()` (`main.rs:2183`) into a single `notice` slot, so a second error overwrites
  the first.
- **Impact:** rare data loss in the corrupt-file path, and lost error messages when
  two things fail together.
- **Fix:** in `move_aside`, if the rename fails, copy the file, and if that fails too,
  refuse to save that file. Make `notice` a small queue. Audit `let _ =` on `fs::` calls
  only (a grep-sized task).
- **Effort:** S. **Risk:** low.

### 14. Smaller structural issues — **Low**
- `main.rs` holds 128 `mod` lines plus app wiring, keymaps, `quit` (`main.rs:9970`),
  `plain_files` (`main.rs:1133`) and the palette builder (`main.rs:3265`). It could be
  split into `app/mod.rs` (struct and new), `app/render.rs` and `app/palette.rs`.
- `quit` exists twice: `main.rs:9970` and `menubar.rs:320`. Check that they agree, or
  have one call the other.
- `RefCell` fields on `RunoApp` (`chat_rows`, `peek_rows`, `md_cache`, `diff_rows`,
  `output_md`; `main.rs:344`, `461`, `463`, `692`, `695`) let render mutate caches
  through `&self`. It works, but it's a sign these belong to their own entities.
- Names are mostly clear and consistent in plain English. Where they aren't, it's
  because of flattening (`git_folder_msg_set` / `git_folder_msg_kept`, `new_more_agents`,
  `peek_follow` vs `chat_follow`). Moving fields into sub-structs (gap 1) fixes this
  for free.
- Dead code is well controlled: only 9 `allow(dead_code|unused)`, all platform-gated.
  No action needed.

---

## What's already good (keep it)
- Atomic temp-and-rename writes everywhere. Sessions are written first, then the
  index, so a new session is never listed before its file exists (`persist.rs:75`–`77`).
  Orphaned session files are found again (`persist.rs:133`–`145`).
- Corrupt files are moved aside, not deleted (`store.rs:1071`). Interrupted turns are
  marked failed on load (`store.rs:1158`–`1170`).
- The save writer debounces with a max delay (`persist.rs:15`–`17`) and flushes on quit
  (`main.rs:2963`).
- The virtual chat list (`chat_list.rs`) and markdown cache keep long chats cheap to
  draw. Frame stats per pane exist (`panes.rs:65`).
- There are few panicking `unwrap`/`expect` in UI code. Most of the remaining ones are
  "checked above" invariants (`shell.rs:1434`, `crew_ui.rs:742`).
- `serde(default)` coverage on persisted structs is thorough.

## Top 5 recommendations
1. **Save only what changed** (gap 3): per-session dirty flags, clone and serialize only
   dirty sessions. Small (M) change, largest everyday performance gain, and it also
   makes adding fsync (gap 8) cheap.
2. **Add a `format` version to every persisted file** (gap 6), *before* any format
   change. It's an S change and makes gap 4 safe.
3. **Rework `bg`** (gap 5): an executor or pool, keyed and superseding jobs, automatic
   stale-result checks, and panic capture so a background bug fails one job instead
   of aborting the app.
4. **Targeted notifies for hot paths** (gap 2a): stream events, PTY output, composer
   typing and scan results should redraw only their pane. This gets most of the
   rendering win without the full entity split.
5. **Start dismantling `RunoApp`** (gaps 1 and 2b): one feature per PR into an owned
   sub-struct, then an entity. Git (≈45 fields) first, then PR, playbooks, the file
   editor and the terminal. Pair it with lazy-loaded session messages (gap 4) once
   versioning is in place.
