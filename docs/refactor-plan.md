# Core app refactor plan

How to close the gaps in `docs/arch-review-core.md` (PR #268) in small, safe steps.
Paths are relative to `native/runo-app/src/`.

## Rules for every slice
- No behaviour change. A slice is either a pure refactor (move, rename, extract) or one
  targeted fix, with a test.
- At most a few hundred lines moved or changed per PR. One feature group per PR.
- Rebase on the latest `main` first. Check `gh pr list` and avoid files that open PRs
  are changing; if a slice must touch one of them (usually `main.rs` for the struct),
  keep that part to a few lines.
- Run `cargo check` locally; CI runs the tests and lints.
- Don't add new `prefix_*` fields to `RunoApp`. Put new state in the feature's struct.

## Done
| # | Slice | Gap | PR |
|---|---|---|---|
| 1 | Poison-tolerant locks in `browser_engine.rs` and `harness/host.rs`; this plan | 9 | refactor step 1 |

## Next, in order
Each line is one PR. Items marked *waits* depend on another PR merging first.

1. **File editor state → `FileEditor` sub-struct** (gap 1). `file_*` fields (9) become
   `self.file.*`. Touches `files.rs`, `fileview.rs`, `shell.rs`, `idle.rs` and the
   struct in `main.rs`. Mechanical.
2. **Browser tab state → `BrowserTab` sub-struct** (gap 1). `browser_*` fields (7),
   used in `browser.rs`, `shell.rs`, `main.rs`.
3. **Durable saves** (gap 8). *Waits for #272*, which replaces `persist::write_text` and
   `store::write_json` with `store::write_private`. Then add `file.sync_all()` before
   the rename there, plus a best-effort fsync of the folder on unix. One function.
4. **`move_aside` can't lose a file** (gap 13). If the rename fails, copy instead, and if
   that fails too, don't save over it. `store.rs`, *after #272/#276/#277/#270 merge*.
5. **`format` version on `state.json`, `session.json` and `settings.json`** (gap 6).
   `#[serde(default)] format: u32`; writing it is the only change. Reading code that
   acts on it comes in a later PR. Touches `store.rs` and `settings.rs`, so it waits
   for the open security PRs.
6. **Playbook editor state → `PlaybookEditor`** (`pb_*`, 9 fields; `playbooks.rs`,
   `pbshare.rs`, `workspace.rs`, `turns.rs`). Wait until `turns.rs` is quiet.
7. **Diff view → `DiffView`** (`diff_*`, 8 fields; `diff_list.rs`, `ship.rs`).
8. **Terminal dock → `TerminalDock`** (`term_*`, 8 fields).
9. **PR state → `PrState`** (`pr_*`, 8 fields; `github.rs`, `prwatch.rs`, `ship.rs`).
10. **Git place → `GitPlace`** (`git_*`, ≈45 fields). Split into 3–4 PRs by sub-area
    (tabs and branches, history, folder messages, scans).
11. **Targeted notifies** (gap 2a). Add a dirty-parts set; switch the hot paths one at a
    time (stream events, PTY output, composer typing, scan results) to
    `notify_parts`. One path per PR, with a debug assert that a part was marked.
12. **Save only dirty sessions** (gap 3). Per-session generation counter set in
    `session_mut`; keep a full write on quit and on a slow timer.
13. **`bg` rework** (gap 5): keyed jobs and generation tokens first; panic capture only
    after checking `panic = "unwind"` against the vendored GPUI.
14. Later, needs a design note first: lazy session messages (gap 4, after 5), provider
    adapters (gap 10), settings table (gap 11), check detectors (gap 12).

Once a sub-struct has no readers outside its own files, it can become an `Entity<_>`
with its own `Render` (gap 2b). Do that as its own PR, not in the same PR as the move.

## Notes
- Release builds use `panic = "abort"`, so a poisoned lock can't occur there today. The
  lock fixes matter for tests and for gap 5, which will catch panics in `bg` jobs.
- The review found saves already atomic and `serde(default)` coverage already thorough.
  No work is needed there beyond items 3–5.
