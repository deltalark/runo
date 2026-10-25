# Context compaction and the context meter

A read-only check of the session menu's **Compact context** item (`native/runo-app/src/ctxmenu.rs:229`), the per-turn `… tokens · N% context` footer, and how Crew's long-lived conversation behaves near the limit. No app code was changed. Tested 2026-10-25 with Claude Code 2.1.310, Codex CLI 0.147.0, opencode 1.19.4 and Gemini CLI 0.65.0.

## TL;DR

1. **Claude Code auto-compacts in the headless mode Runo uses** (`-p --resume … --output-format stream-json`). The stream emits `system/status: "compacting"` and then `system/compact_boundary` with `trigger: "auto"`. Runo ignores both events, so the person sees a pause and then the meter drops, with nothing in between to explain it.
2. **A footer above 100% is a bug in the meter, not real overflow.** Two causes:
   - (a) **Claude:** a turn that ends without a `result` event (stopped, a message sent while it was working, or an error) keeps `window = 0`, and the meter falls back to 200k on 1M models.
   - (b) **Codex `exec --json` (the fallback path) and other providers:** they report *cumulative* usage, or Runo never learns their window.
3. **Crew:** at about 2.7k tokens a turn, the observed Crew session is at 297k of 1M after 94 turns. Claude will auto-compact it at around 1M. At that point Crew's **rules and action protocol** may survive only as a summary, because they are sent once in the first user message. So may the **full thread board**, because later turns send only what changed. Runo's own state (threads, decisions, reports) is safe on disk.
4. **Recommendation:** keep the item and rename it **Compact now**. Fix the meter. Handle `compact_boundary` by showing a divider and re-sending Crew's rules and full board. For providers that don't compact themselves, add an automatic Runo compact at about 85% of the window.
5. **Side finding (bigger than the meter):** Claude's `total_cost_usd` is the *session's running total*, but Runo stores it per turn and **adds the turns up**. Crew session #3 really cost about **$16**, and Runo's sum says **$589**. This affects session cost, daily spend limits and thread `budget_usd`.

---

## 1. Auto-compaction per agent

### Claude Code: tested for real

Setup: a throwaway git repo in `/tmp/cc-compact-test` with 14 random-text files of about 100 KB each. Runs used Runo's exact flags (`agent.rs:663-673`):
`claude -p … --output-format stream-json --verbose --include-partial-messages --permission-mode acceptEdits --model haiku`.

**Run 1:** read the files with no overrides. The model resolved to `claude-haiku-5-5`, and `result.modelUsage[…].contextWindow` reported `1000000`. The context grew in steps of about 20k tokens per call, to **767,613 tokens over 42 API calls with no compaction**. That fits a threshold near the 1M window. The run stopped at the $1.50 budget, so the 1M limit itself was never reached.

**Run 2:** `--resume <id>` (as Runo does) with `--autocompact 100000`, the CLI's own flag for the auto-compact window. The prompt was "Reply with just OK." Stream excerpt:

```json
{"type":"system","subtype":"status","status":"compacting","compact_output_chars":0,...}
{"type":"system","subtype":"status","status":null,"compact_result":"success",...}
{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"auto","pre_tokens":768344,"post_tokens":10301,"cumulative_dropped_tokens":758043,"duration_ms":10629,"preserved_segment":{...}}}
{"type":"user","message":{"content":[{"type":"text","text":"This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion ...\nSummary:\n1. Primary Request and Intent: ..."}]}}
{"type":"assistant", ... usage → 29,198 tokens of context}
{"type":"result","subtype":"success","result":"OK",...}
```

The session went from 768k to 10k tokens, and the turn still succeeded.

**Run 3:** `-p "/compact keep only the file list" --resume <id>`. This is what Runo's menu item sends (`chatctl.rs:183`). It produced the same `compacting` → `compact_boundary` sequence with `trigger: "manual"` (29,202 → 8,705 tokens), plus a replayed `<local-command-stdout>Compacted </local-command-stdout>` user message. The `result` has **all-zero usage and no `modelUsage`**, so Runo records no usage for that turn, and the meter keeps showing the pre-compact figure until the next turn.

Total cost of the three runs: $1.54.

**Controls in the 2.1.310 binary and `--help`:**

| Control | Effect |
|---|---|
| `--autocompact <auto\|tokens>` / `CLAUDE_CODE_AUTO_COMPACT_WINDOW` / `autoCompactWindow` setting | Auto-compact window, 100k–1M tokens |
| `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` | Threshold as a % of the window |
| `autoCompactEnabled` setting / `DISABLE_AUTO_COMPACT=1` | Turns auto-compact off (manual `/compact` still works) |
| `DISABLE_COMPACT=1` | Turns off auto **and** manual `/compact` |
| `CLAUDE_CODE_BLOCKING_LIMIT_OVERRIDE`, `CLAUDE_CODE_MAX_CONTEXT_TOKENS` | Hard limits |

Runo sets none of these, so Claude sessions in Runo auto-compact by default.

**What Runo does with these events:** nothing. `stream.rs` has no `compact` handling. The `system` subtypes `status` and `compact_boundary` fall through the match, and the summary user message has no `tool_result` blocks, so it is dropped. Runo does not know a compaction happened. That matters for Crew (§3).

### Other providers

Claude was tested live. Codex was tested live for its usage fields only (it wasn't pushed to the limit), and its config keys were checked in the installed 0.147.0 binary. opencode's keys were also checked in its binary (1.19.4). Gemini is installed but not signed in here. The rest come from vendor docs and changelogs. **"Headless?" is unconfirmed in docs for every provider except Claude.**

| Provider | Auto-compacts? | Threshold / setting / disable | Manual command | At the limit | Stream event Runo could read | Source |
|---|---|---|---|---|---|---|
| **claude** | **Yes, tested headless with `--resume`** | `--autocompact`, `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`; off with `DISABLE_AUTO_COMPACT` / `autoCompactEnabled:false` | `/compact [focus]`, **works as the `-p` prompt** (tested) | Compacts silently and the turn continues | `system/status:"compacting"`, `system/compact_boundary {trigger, pre_tokens, post_tokens}` | tested, §1 |
| **codex** | Yes, before or during a turn | `model_auto_compact_token_limit`, `model_auto_compact_token_limit_scope`, `compact_prompt`, `experimental_compact_prompt_file` (all in the binary); no off switch documented | `/compact`; app-server `thread/compact/start` with a `contextCompaction` item (in the binary) | Compacts; falls back to dropping the oldest items (third-party docs) | `context_compacted` / `contextCompaction` (app-server); `exec --json`: unconfirmed | binary strings; developers.openai.com codex config reference |
| **opencode** | Yes | `compaction.auto` (default true), `compaction.buffer`, `compaction.prune`, `compaction.keep.tokens`; `OPENCODE_DISABLE_AUTOCOMPACT` and `OPENCODE_DISABLE_PRUNE` (in the binary) | `/compact` (TUI), `POST /api/session/<id>/compact` | Compacts and retries once, then errors; `run --format json` may report an error and exit 1 even when it recovers (issue #39573) | `session.compacted` on the bus, not in `run --format json` | opencode.ai/v2/docs/compaction |
| **gemini** | Yes, before each turn | `model.compressionThreshold` (default 0.5 of the window) | `/compress` (Runo maps it) | Unconfirmed | Internal `ChatCompressed`; not among the stream-json types | gemini-cli docs/reference/configuration.md |
| **cursor** | Yes, near 90% (staff forum post) | No setting | `/summarize` (alias `/compress`) | Summary plus a pointer to the full history file | Unconfirmed | cursor.com/changelog/1-6 |
| **copilot** | Yes, in the background from ~80%; pauses at ~95% | SDK `infiniteSessions.*`; no CLI setting to disable | `/compact` | Waits for compaction to finish | Unconfirmed | docs.github.com …/copilot-cli/context-management |
| **amp** | Yes, at 90% (since the May 2026 rebuild; Handoff removed) | No setting documented | None documented | Summarizes and continues | Unconfirmed | ampcode.com/news/neo |
| **droid** | Yes; can be turned off (May 2026) | `compactionTokenLimit` (per model, 250k for BYOK), `compactionTokenLimitPerModel`, `compactionModel` | `/compress [prompt]`, `/handoff` | Compacts instead of failing (since Sep 2026); a `PreCompact` hook can block it | None documented in stream-json | docs.factory.com/cli/configuration/settings |
| **qwen** | Yes | `context.autoCompactThreshold`, default 0.85 | `/compress` (Runo maps it), `/compress-fast` | Has a hard tier; what happens there is unconfirmed | Unconfirmed | qwenlm.github.io/qwen-code-docs …/settings |
| **grok** | Yes | `[session] auto_compact_threshold_percent`, default 85 (third-party guide) | `/compact [instructions]` | Unconfirmed | OTel `grok_code.compaction` only | tokenade.net (third-party) |

**Result: every provider Runo supports compacts on its own in its interactive CLI.** Only Claude is proven to do it in Runo's headless mode. Only Claude's native compaction is reachable from Runo today (`compacts_itself`). Codex's app-server RPC and Gemini/Qwen `/compress` could be used too, once tested headless.

How Runo maps these today: `chatctl.rs:17-34` parses `/clear`, `/new`, `/compact` (and `/compress` for gemini and qwen) typed in the composer. `compacts_itself()` (`chatctl.rs:37`) is true only for `claude`. For every other provider, Compact uses Runo's summary handoff (`chatctl.rs:189-219`): it clears `agent_session`, and the next turn starts a fresh CLI conversation that opens with the summary. That discards the CLI's own session, including its native `/compact`, its cache, and anything the summary misses.

---

## 2. The context meter

**Where it's computed:**
- Label: `main.rs:1153 usage_label()` gives `pct = usage.context() / window`, with `window = usage.window` or **200,000 when it is 0**. It is shown at `main.rs:5713`.
- `store.rs:119 Usage::context()` = `input + cached + output`.
- Parsing (`stream.rs`):
  - Claude `assistant` events (l.152-158) set `input` and `cached` from the **latest API call**, which is correct.
  - Claude `result` (l.236-255) sets `output` (the **turn total**) and `window = max(modelUsage[*].contextWindow)`.
  - Codex `exec` `turn.completed` (l.765-775), Gemini/Cursor `result` (l.884-901), Droid `completion` (l.919-929), opencode `step_finish` (l.868-881), Copilot `assistant.usage` (l.978-984).
  - Codex app-server: `harness/codex.rs:552-568` reads `tokenUsage.last` and `modelContextWindow`, which is correct.

**Why it exceeds 100%: evidence from `~/.runo/sessions`.** Every turn over 100% has `window: 0`:

| Session | Provider | Context tokens | Window stored | Shown | How the turn ended |
|---|---|---|---|---|---|
| 30 "Smooth window scrolling" | claude (1M) | 322,813 | 0 → 200k | 161% | next message "sent while working" |
| 34 "Review X post vs Runo" | claude (1M) | 305,527 | 0 → 200k | 153% | "Claude Code isn't signed in" error |
| 55 "Perf: redraw cost" | claude (1M) | 248,615 | 0 → 200k | 124% | next message "sent while working" |
| 8 (codex exec) | codex | 341,214 | 0 → 200k | 171% | completed; cumulative usage |

Of 257 Claude turns with usage, 195 have `window: 1000000` and 53 have `0`. Those 53 are exactly the turns that never got a `result` line (their `output` is also 0). For example, "419k tokens · 42% context" is the same kind of session on a turn that did finish.

Causes, ranked:

1. **Wrong window (main cause).** `window` is learned only from Claude's final `result.modelUsage`. Stopped, interrupted or failed turns keep 0, and the label uses 200k. For every provider except Claude and the Codex app-server, `window` is *always* 0, so their meter always assumes 200k (gpt-5.x and Gemini have 272k–1M+).
2. **Cumulative usage (Codex `exec --json`).** Tested: turn 1 `turn.completed.usage` = `{input 30966, cached 15104, output 146}`. After `exec resume` with "Reply OK", turn 2 = `{input 46588, cached 30208, output 151}`, which is turn 1 plus one call. It is a **running total for the whole thread**, so the meter only ever grows. The app-server path, used when available (`agent.rs:1761`), is correct. Gemini/Cursor `result.stats` and Droid `completion.usage` are per-run totals across every API call in the turn, so they over-count multi-step turns in the same way. This comes from their docs and could not be run here.
3. **Output tokens counted.** On Claude, `output` comes from `result.usage.output_tokens`, which is the *whole turn's* output (8,806 in run 1, versus 8 on the last call), including thinking. This is minor, and next turn's input already contains the visible part of it.
4. **Cache-read counted as context: this is correct.** `cache_read + cache_creation + input` *is* the prompt size of a call. It is not double-counting, as long as it comes from one call and not a turn sum.

**Correct calculation** (per provider, using the **last** model call of the turn):

```
context = input_tokens + cache_read_input_tokens + cache_creation_input_tokens   // last call's prompt
          (+ that call's output_tokens, optionally: it joins the next prompt)
pct     = context / window
```

- **Claude:** take it from the last top-level `assistant` event (already done), or from `result.usage.iterations[-1]`. Do *not* use `result.usage`, `result.modelUsage` or `total_cost_usd` for context: they are turn or session totals. `window` = `result.modelUsage[model].contextWindow`. **Persist the last known window on the session (keyed by model)** and use it when a turn has no `result`. Default by model id: a `[1m]` suffix or a 1M-capable model means 1,000,000, else 200,000.
- **Codex:** `exec`: diff consecutive `turn.completed` totals, or prefer the app-server's `tokenUsage.last` and `modelContextWindow` (already done there).
- **opencode:** the last `step_finish` `tokens.input + cache.read + cache.write` (already done).
- **Gemini, Cursor, Droid, Copilot:** per-run totals over-count. Use per-call events if the stream has them; otherwise show tokens without a % (or label it "≈"), rather than a wrong percentage.
- **Window per provider/model:**

  | Provider / model | Window |
  |---|---|
  | Claude Opus/Sonnet with 1M, Haiku 5.5 | 1,000,000 (observed `contextWindow: 1000000`) |
  | Other Claude | 200,000 |
  | Codex | `modelContextWindow` from the app-server (1,000,000 in the stored sessions). `exec` doesn't report it. |
  | Others | Use the model's published window. Unknown means no %. |

- **Clamp the display at 100%** only as a guard, never as the fix.

**Related: the cost double count.** `stream.rs:245` stores `total_cost_usd` as the turn's cost. The tests show that it is the **session's running total** on `--resume` ($1.4894 → $1.5427 → $1.5450 over three resumes, where the last turn alone was a ~$0.002 compact). `health.rs:88 session_cost`, `menubar.rs:216`, `main.rs:6045` and `shell.rs:1367 record_spend` all **sum** it per turn. Crew session #3's per-turn values climb 0.35 → 16.00 (real spend ≈ $16), while the sum is **$589**. Fix: store `cost_cents = total_cost_usd − previous total for this agent_session` (reset when the CLI session changes), or keep the latest total instead of summing.

---

## 3. Crew

How Crew's turns are built (`crew_ui.rs:187-259 chat_preface`):
- **First turn only** (`first = !has_conversation()`): `crew::rules(...)` (`crew.rs:1070`). This is the whole action protocol (`start_thread`, `message_thread`, `read_thread`, `resolve_thread` JSON), the limits, the roster and the models.
- **Every turn:** `## Threads`, holding only **changes** since the last turn (`crew.rs:1051 board_changes`, compared against `canvas_context`). Also `## Slots`, `## From threads` (thread reports) and `## The person says`.
- The CLI conversation is resumed every turn. For Claude, Crew is sent with no `--append-system-prompt`, so the rules live only in message 1.

Observed: Crew session #3 (claude, 1M) has 94 turns and 504 messages, and is at **297,551 tokens**. It grows by about 2.7k tokens a turn (45k → 290k), so it reaches 1M after another ~250 turns. On a 200k-window model it would compact after about 55 turns.

**What happens near the limit:**
- **Claude:** the CLI auto-compacts silently (shown in §1). The summary keeps "primary request, key concepts, …" plus a preserved recent segment. Message 1, which holds Crew's rules, is summarized. The exact JSON action syntax, the limits and the roster may be paraphrased or lost, so Crew may stop emitting valid actions. Because later turns send only board *diffs*, the full list of open threads and pending decisions exists only in the summary. Runo doesn't know a compaction happened, so it doesn't re-send either.
- **Other providers as Crew:** they have no Runo-side automatic policy. At the limit they hit whatever their CLI does (§1 table). If the person uses Runo's Compact, `agent_session` is cleared and `has_conversation()` becomes false, so the next turn re-sends the rules and the **full** board. That path is safe.

**What's actually lost vs safe:**
- **Safe, because it is persisted by Runo:** threads, their status, checklists, reports, decisions (`ThreadInfo`), and `read_thread`.
- **Lost or degraded:** the protocol rules, the full board snapshot, and the conversation with the person (earlier asks and preferences, which survive only as a summary).

**Recommendation for Crew (ranked):**
1. **Let the CLI auto-compact, and make Crew compaction-proof.** Put `crew::rules` in `--append-system-prompt` for Claude, where it is cached and survives compaction. On `compact_boundary`, clear `canvas_context` so the next turn sends the **full** board, and re-send the rules for providers without a system-prompt flag. This is cheap and keeps the conversation with the person.
2. For providers that don't compact themselves, have **Runo compact at about 85% of the window** with its summary. That path already re-sends the rules and the full board.
3. *Not recommended:* a fresh conversation every Crew turn. It is robust, but it loses the thread of the person's conversation and pays the full rules and board every turn (on Claude the system prompt still caches). Keep it as a fallback if (1) proves flaky.
4. *Not recommended:* never compacting Crew. Claude will do it anyway unless `DISABLE_AUTO_COMPACT` is set, and then the session simply errors at the limit.

---

## 4. Recommendations for the menu item and meter (ranked)

1. **Fix the meter.** Persist the window per session and model, default by model, never fall back to 200k for 1M models, and diff or avoid cumulative usage (Codex exec, Gemini/Cursor/Droid). The formula is in §2. Also fix the **cost summing**: this is the most user-visible money bug.
2. **Handle Claude's compaction events.** Map `status: "compacting"` to a "Compacting…" step, and `compact_boundary` to a `COMPACTED` divider using `pre_tokens → post_tokens`, setting the meter to `post_tokens`. For Crew, also reset the board and re-send the rules (§3).
3. **Keep the menu item, renamed "Compact now".** It is useful for every provider: for Claude it triggers the real `/compact` (tested to work headless), and for others it is the only way to shrink. Don't hide it for Claude: auto-compact only fires near the limit, and compacting early with a focus ("/compact keep the API design") is a real use. Its tooltip can say "Claude also compacts on its own near the limit."
4. **Add an automatic policy for providers that don't compact themselves.** When the corrected meter passes about 85%, run Runo's summary compact before the next turn, with a notice. Prefer the CLI's native compaction where Runo can trigger it (Codex app-server, opencode, Gemini `/compress`) over Runo's summary, because it keeps the CLI session and cache.
5. Leave **Clear context** as is.

### Test artefacts
Reproduce with the commands in §1. The test files were generated in `/tmp/cc-compact-test`, and the session data came from `~/.runo/sessions/{3,8,30,34,55}/session.json`.
