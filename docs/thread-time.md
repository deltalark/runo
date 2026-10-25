# Where Crew threads' time goes (24–25 Oct 2026)

A measurement of Crew threads #22–#59, read-only. In one line: **the agent spends about
half of a thread's working time on Rust builds and tests on this Mac (a quarter of all
working time is waiting on other threads' builds or re-running what CI also runs), 14%
waiting for CI and 24% generating.** Crew's guess that builds dominate holds for time
inside a turn. On top of that come time spent queued, waiting on the person, and finished
threads sitting idle, none of which the "Working" timer counts.

## Data and method

- **Turn boundaries**: `~/.runo/logs/runo-*.log` (`turn started`, `finished/failed/stopped
  after Ns`, `[proof]` check runs, thread creation, Runo restarts).
- **Inside each turn**: Claude Code's own transcript for the thread's conversation,
  `~/.claude/projects/<worktree>/<agent_session>.jsonl`. It has a timestamp on every API
  request, every streamed response block (with `usage`) and every tool result.
  `~/.runo/sessions/<id>/turn/out` holds only the latest turn, so it wasn't used.
- **Model time** is API request → last block of its response, added up per request.
  Tools that ran while the response was still streaming cost nothing extra. **Tool time**
  is response end → next request, charged to the slowest tool in that batch (the one being
  waited on). Waiting for a background command's notification (`run_in_background`,
  `gh pr checks --watch` in the background) is charged to that command's kind.
- Bash commands are sorted by what they waited on: `cargo build/check/run`, `cargo test`,
  `cargo clippy/fmt`, sleep loops on `pgrep cargo` (**waiting for other threads' cargo**),
  loops on the thread's own build log, `gh pr checks --watch` / `gh run` polling (**CI**),
  loops on `df` (**disk full**), launching the app with orca or screenshots (**app
  testing**), git, gh, and other shell.
- **Between turns**: post-turn checks (proof start → pass/fail), **waiting for an answer**
  (the turn before called `ask_decision` or `message_crew`), **Runo closed** (the gap spans
  a Runo restart), **idle** (anything else over 30 s: the thread had finished and nobody
  had written to it yet), Runo overhead (gaps of 30 s or less).
- Out of scope: #25 (a nodot session, not a Crew thread). #37 (one 28 s turn that failed
  on the false sign-in error, then deleted) and #43 (1 min) are in the totals but have no
  row. #54–#56 were stopped by the person after 67 min.

## Per thread (minutes)

"Working" is the sum of turn wall-clock time: the time the "Working" timer counts.
The columns from Model to Overhead add up to Working. The last five columns are outside
any turn.

| # | Thread | Turns (failed) | Steps | **Working** | Model | Local cargo | Wait other cargo | CI wait | Disk-full wait | App testing | git/gh | Read/edit | Other tools | Overhead | Queue before start | Checks after turn | Wait for answer | Idle between turns | Runo closed |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 22 | Add Tetris game | 3 (0) | 38 | **19** | 6 | 12 | 0 | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 1 | 29 | 1 | 0 |
| 23 | Crew chat scroll lock | 1 (0) | 127 | **40** | 10 | 17 | 0 | 0 | 0 | 12 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 24 | Sync, merge, release | 11 (2) | 164 | **86** | 15 | 20 | 0 | 33 | 0 | 0 | 2 | 0 | 1 | 14 | 0 | 23 | 0 | 35 | 895 |
| 26 | App startup time | 1 (0) | 119 | **138** | 15 | 75 | 0 | 0 | 31 | 1 | 0 | 2 | 14 | 0 | 0 | 0 | 0 | 0 | 0 |
| 27 | Thread pane scroll fix | 2 (0) | 26 | **31** | 3 | 27 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | 0 | 3 | 11 | 0 | 0 |
| 28 | Playful Arcade redesign | 4 (0) | 79 | **63** | 16 | 14 | 21 | 0 | 0 | 1 | 0 | 0 | 9 | 0 | 0 | 7 | 9 | 82 | 0 |
| 29 | Dock icon badge count | 2 (0) | 82 | **45** | 8 | 33 | 0 | 0 | 0 | 1 | 0 | 0 | 1 | 0 | 0 | 3 | 7 | 0 | 0 |
| 30 | Smooth window scrolling | 12 (1) | 330 | **182** | 45 | 42 | 80 | 5 | 0 | 0 | 1 | 1 | 5 | 3 | 0 | 20 | 7 | 493 | 297 |
| 31 | Highlight Crew in sidebar | 4 (0) | 114 | **91** | 9 | 54 | 0 | 0 | 0 | 9 | 6 | 10 | 2 | 1 | 0 | 1 | 107 | 0 | 0 |
| 32 | Runo memory usage | 3 (0) | 81 | **45** | 10 | 32 | 0 | 0 | 0 | 1 | 0 | 1 | 0 | 0 | 0 | 3 | 33 | 0 | 0 |
| 33 | Shared cargo target dir | 4 (1) | 47 | **38** | 5 | 17 | 15 | 0 | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 2 | 58 | 0 | 0 |
| 34 | Review X post vs Runo | 8 (1) | 215 | **69** | 24 | 38 | 0 | 0 | 0 | 0 | 0 | 1 | 1 | 5 | 0 | 1 | 3 | 24 | 0 |
| 35 | Clickable thread references | 4 (0) | 113 | **51** | 11 | 38 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| 36 | False signed-out errors | 20 (17) | 113 | **44** | 9 | 2 | 0 | 30 | 0 | 0 | 1 | 0 | 0 | 2 | 0 | 0 | 1 | 65 | 8 |
| 38 | Stunning working indicator | 3 (0) | 134 | **45** | 16 | 22 | 0 | 0 | 0 | 3 | 0 | 0 | 0 | 3 | 0 | 0 | 11 | 1 | 0 |
| 39 | Auto-clean worktree builds | 5 (0) | 136 | **46** | 15 | 22 | 0 | 1 | 0 | 0 | 0 | 0 | 5 | 2 | 0 | 1 | 5 | 0 | 780 |
| 40 | Low-priority background checks | 2 (1) | 38 | **39** | 5 | 19 | 0 | 0 | 0 | 0 | 0 | 2 | 1 | 13 | 0 | 0 | 1 | 0 | 0 |
| 41 | Refine Arcade: pro + fun | 5 (0) | 85 | **27** | 11 | 3 | 3 | 4 | 0 | 0 | 1 | 1 | 0 | 4 | 0 | 15 | 0 | 412 | 130 |
| 42 | Thread message chip fix | 4 (0) | 68 | **14** | 5 | 0 | 0 | 8 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 | 1 | 5 | 0 |
| 44 | Align Crew route bar | 1 (0) | 16 | **6** | 1 | 0 | 0 | 5 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 45 | Sidebar redesign mockups | 3 (0) | 106 | **36** | 13 | 15 | 0 | 6 | 0 | 1 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 3 | 0 |
| 46 | Image viewer on click | 1 (0) | 57 | **26** | 18 | 3 | 0 | 5 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 47 | Compact usage indicator | 1 (0) | 30 | **10** | 3 | 0 | 0 | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 48 | Release v0.5.7 | 3 (0) | 69 | **79** | 4 | 1 | 0 | 73 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | 25 | 0 |
| 49 | Fix bar backgrounds width | 2 (0) | 92 | **28** | 8 | 7 | 0 | 6 | 0 | 0 | 0 | 6 | 0 | 0 | 0 | 15 | 0 | 101 | 0 |
| 50 | Performance audit | 2 (0) | 114 | **44** | 15 | 15 | 0 | 0 | 0 | 9 | 0 | 3 | 3 | 0 | 0 | 0 | 2 | 0 | 0 |
| 51 | Borderless sidebar sections | 1 (0) | 21 | **8** | 2 | 0 | 0 | 7 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 52 | Assistant badge and input | 1 (0) | 66 | **14** | 6 | 2 | 0 | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 53 | Working indicator stutter | 1 (0) | 52 | **15** | 10 | 0 | 0 | 4 | 0 | 0 | 0 | 0 | 1 | 0 | 3 | 0 | 0 | 0 | 0 |
| 54 | Perf: no UI freezes | 1 (0) | 86 | **67** | 11 | 0 | 47 | 0 | 0 | 0 | 0 | 0 | 0 | 8 | 0 | 0 | 0 | 0 | 0 |
| 55 | Perf: redraw cost | 1 (0) | 166 | **67** | 19 | 11 | 26 | 9 | 0 | 0 | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 0 | 0 |
| 56 | Perf: idle cost | 1 (0) | 141 | **67** | 18 | 2 | 10 | 6 | 0 | 29 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 |
| 57 | Scale limits to hardware | 1 (0) | 42 | **12** | 6 | 0 | 0 | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 46 | 0 | 0 | 0 | 0 |
| 58 | Validate context compaction | 1 (0) | 49 | **12** | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 5 | 0 | 0 | 33 | 0 | 0 | 0 | 0 |
| 59 | Chat image overlap glitch | 1 (0) | 57 | **11** | 6 | 0 | 0 | 5 | 0 | 0 | 0 | 0 | 0 | 0 | 33 | 0 | 0 | 0 | 0 |
|  | **All 35 threads** | 120 (23) | 3273 | **1616** | **386** | **542** | **203** | **227** | **31** | **66** | **15** | **37** | **48** | **61** | **116** | **97** | **287** | **1248** | **2110** |

### Totals

| Inside turns (the timer) | min | share of Working |
|---|---:|---:|
| Local cargo: build/check 168, test 157, clippy 39, backgrounded builds 106, polling own build log 73 | 542 | 34% |
| Waiting for *other threads'* cargo (`until ! pgrep cargo; do sleep …`) | 203 | 13% |
| Model (generating, incl. thinking) | 386 | 24% |
| CI (`gh pr checks --watch`, `gh run` polling) | 227 | 14% |
| Launching and testing the app (orca, screenshots) | 66 | 4% |
| Disk-full waits (`until df …`) | 31 | 2% |
| Reads/edits 37, git/gh 15, other shell and unknown background 48 | 100 | 6% |
| Overhead: CLI start/stop, killed turns' tails | 61 | 4% |
| **Working, total** | **1616 (27 h)** | |

Per thread, Working is a median of 40 min (mean 46). From the thread starting to its
last turn ending, not counting time Runo was closed, the median is 54 min (mean 96).

| Outside turns | min |
|---|---:|
| Idle: thread finished, waiting for its next prompt (mostly #30 493, #41 412, #49 101, #28 82, #36 65; overnight or the person away) | 1248 |
| Waiting for an answer after `ask_decision` / `message_crew` | 287 |
| Queued for a slot before the first turn (#57 46, #58 33, #59 33) | 116 |
| Post-turn checks (proof.rs) running between turns | 97 |
| Runo closed or restarting across a gap | 2110 |
| Runo overhead between turns (gaps of 30 s or less) | 6 |

## Answers to the specific questions

**What the "Working · 1:28:41" timer measures.** The current turn only:
`crew_view.rs` `thread_row` shows `elapsed(turn.started.elapsed())` from `self.turns`. A
turn that has to wait for a slot is put in `session.queue` before it starts (`turns.rs`
→ `hold_turn`), so the timer doesn't count queue time, gaps or earlier turns. It runs past
an hour because single turns hold builds and CI waits inline: 19 of 122 turns ran over
30 min. The longest were #26 at 138 min (release builds and benchmarks on a full disk), #30
at 94, and #54–#56 at 67 each (stopped by the person, after waiting 10–47 min for each
other's cargo).

**Model time.** 3,273 API requests (steps), 94 per thread, a median of 28 per turn. Each
step took a median of 4.7 s (mean 7.1 s, p90 12.3 s). Every request ran at
`effort: medium`, with adaptive thinking. Latency follows output tokens (thinking
included), at about 12 s per 1k: steps under 1k tokens took a median of 4.3 s, 2k+ took
23 s, 10k+ took 131 s. Steps over 2k output tokens are 5% of steps but 93 of the 386 model
minutes. **Context size barely matters**: the median step took 3.9 s under 50k tokens of
context, 4.6–5.0 s from 50k to 250k, and 5.8 s at 300–350k. The time per output token
didn't rise with context.

**Context "past 100%".** This is a display bug, not real pressure. 82% of the usage events
Runo records have `window: 0`, and `usage_label` (`main.rs`) then divides by 200,000. The
model's window is 1,000,000, which the other 18% of events report. The largest context
seen was 373k (#30), which is 37%.

**Failed and retried turns.** 23 of 120 turns failed. 20 of them failed as "the agent isn't
signed in" (17 in #36, plus #37 and #33). That was the false sign-in detection #36 was
fixing (PR #242): the CLI's own output that mentioned "401" or login was read as an auth
error. The 20 failures cost only about 20 min of turn time, but each one stopped a turn
partway through, and #37 was given up entirely. The other 3 were killed when the disk
filled (#40 lost 36 min: its turn died at 02:34 with `No space left on device`). Six more
turns couldn't start at all because the disk was full (02:33–02:34 on the 25th).

**Waiting for Crew (MAX_WAKES).** MAX_WAKES barely mattered. Threads woke Crew 15 times
over the two days, in runs of 2, 1, 3, 1, 1, 5, 1 and 1 between the person's messages.
The limit of 4 was reached once (02:11–02:37 on the 25th). #39's next message, at 02:39,
went to the inbox and reached Crew 2.8 min later, only because the person happened to
write. Of the 20 thread messages that went through, 17 were in Crew's next prompt within
3 min. (Wake prompts reach Crew under a "## The person says" heading, so Crew is told the
person said the thread's words. Small, but worth fixing.) The real problems:
- `message_crew` takes **one message per turn**. Three second messages got "You already
  sent Crew a message this turn" (#38, #41 twice). Both of #33's messages never reached
  Crew at all, and #33 then sat for 58 min.
- **`ask_decision` doesn't wake Crew**, by design (`crew_tick` only wakes on `Message`
  events). It waits for the person. The 16 decisions waited from under a minute up to 36
  min before the next turn, and that time is most of the 287 min in "Wait for answer".
- Crew's **ReadThread** results go into `crew.inbox` and only arrive with the person's next
  message (13 deliveries, `crew_ui.rs:607`). This slows Crew's follow-ups, not the threads.
  #35's 00:47 message (with no wakes used since the person's last message) never came
  through as a wake. It only reached Crew 242 min later, inside a ReadThread result.

**Slot queue.** About 2 hours in total, almost all of it at the end of the 25th, when 3
slots were held by #54–#56 (busy waiting on each other's cargo) and #57–#59 waited 33–46
min to start. The timer doesn't count this, and the thread card shows "Waiting for a slot".

**Cargo lock.** Cargo's own "Blocking waiting for file lock" showed up in 8 cargo runs
(35 min). The much bigger wait is threads **polling until no cargo runs anywhere on the
machine** (`until ! pgrep -f cargo`): 203 min. That rule came from the disk-full episodes
(Crew's "no builds until it's your turn"). It serializes every thread behind every other
thread's build and behind Runo's own post-turn `cargo test`. 111 of the 250 cargo runs also
used `-j 2`, which slows each build further.

**Post-turn checks (proof.rs).** There were 57 check runs (43 in the foreground, 14 in the
background) and **35 failed** (61%), mostly from a full disk or a second build in a
worktree's own `target/` instead of the shared one. 10 waited for another session's
checks. The checks took 97 min between turns, and their failures started 11 extra
"Runo ran the project's checks… one failed" turns (63 min).

**Held turns.** No spending-limit or plan-limit holds were logged on these days. The only
holds were the false sign-in failures above.

## Top 5 real causes

1. **Local cargo work: 542 min (34% of Working).** 250 cargo runs, about 7 per thread. A
   `cargo test` took a median of 102 s (mean 187 s, 19 over 5 min), `cargo build/check` a
   median of 41 s (mean 138 s). Most threads ran clippy and the full test suite locally even
   though CI runs both on every PR.
2. **Waiting for other threads' cargo: 203 min (13%).** `until ! pgrep cargo` loops and
   backgrounded waits for them. #30 waited 80 min, #54 47, #55 26, #28 21, #33 15.
3. **Model generation: 386 min (24%).** Already at medium effort. Driven by output and
   thinking tokens, not by context. Big-output steps (over 2k tokens) are 5% of steps but
   a quarter of model time.
4. **Waiting for CI inside the turn: 227 min (14%).** Every PR thread sat on
   `gh pr checks --watch` for about 5–8 min. The release thread #48 waited 73 min for
   release workflows, and #24 waited 33.
5. **Things the timer doesn't show: ~500 min of waiting while a thread wasn't finished.**
   Decisions waiting for the person (287), the slot queue (116) and post-turn checks (97),
   plus 63 min of fix turns those checks started. Separately, the 1,248 min of "idle" is
   threads that had finished, waiting for someone to write to them. It isn't slowness.

The disk filling up repeatedly is behind several of these: 31 min of explicit waits, a
killed 36-min turn, 6 turns that couldn't start, many of the 35 failed checks, and the
serial-build rule in cause 2.

## Fixes, ranked by time saved

Savings are for these two days' 35 threads (27 h of Working time), estimated from the
numbers above.

1. **Run tests and clippy in CI, not locally. ~250 min.** Tell threads (in the Crew thread
   preamble) to run `cargo check` locally at most, then push and let CI run tests and
   clippy. Keep local `cargo test` for runs where the thread needs the result before it
   can continue. Removes most of the 157 min of local tests, the 39 min of local clippy and
   the backgrounded test/clippy runs.
2. **Drop the "no cargo anywhere" wait. ~150–200 min, and it frees slots.** With disk
   cleanup in place (#39's worktree cleaning), let threads build in parallel without
   polling `pgrep cargo`. Leave cargo's own target lock to keep two builds out of the same
   target. Stop forcing `-j 2` unless memory is short.
3. **Don't wait for CI inside the turn. ~200 min of Working, and slots free sooner.** After
   pushing, the thread ends its turn with the PR link. Runo (or Crew) watches CI and wakes
   the thread only if it fails. A thread watching CI holds one of the 3 slots for nothing,
   which is what queued #57–#59 for 33–46 min.
4. **Make post-turn checks cheap and not fail on the environment. ~100–160 min.** Run
   proof.rs checks with the shared `CARGO_TARGET_DIR`. Prefer `cargo check` or CI status
   over a full `cargo test`. Treat ENOSPC or a killed process as "couldn't check" rather
   than "failed", so it doesn't start a fix turn. That covers 97 min of checks and 63 min
   of fix turns, and 61% of runs failed.
5. **Fix the slot queue and the timer display. ~116 min of perceived waiting.** The queue
   already isn't counted by the timer. Showing "Queued · 33m" separately on the card, and
   freeing slots per fixes 2–3, makes the rest visible. Consider not counting threads that
   only wait on CI against the slot cap.
6. **Let decisions reach Crew. Up to ~287 min.** Wake Crew on `ask_decision` too, or let
   Crew answer the ones it can (e.g. "which version to tag"). Also accept more than one
   `message_crew` per turn, or queue the extra messages instead of dropping them (#33 lost
   both of its messages). Raising MAX_WAKES from 4 to around 8 is cheap but would have
   saved under 3 min here, because the limit was reached only once.
7. **Lower effort for mechanical threads. ~20–40 min.** Release and merge threads (#24,
   #43, #48) and typo or docs threads could run at low effort. Model time is only 24% and
   these threads are a small part of it, so this ranks low. Smaller briefs save cost but
   not time, because context size barely affects step latency.
8. **Fix the context % label. 0 min, but it removes a false alarm.** Use the model's real
   window (1M) when `window` is 0, instead of 200k.

Already fixed: the false sign-in failures (#36, PR #242) and worktree build cleanup (#39).
Both caused retries and disk-full stalls on these days.
