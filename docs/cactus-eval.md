# Cactus Compute for Runo — read-only evaluation

Dated 2026-10-27. Research only; no code change.

## What Cactus is

Cactus Compute (YC-backed, [cactuscompute.com](https://cactuscompute.com/),
[github.com/cactus-compute](https://github.com/cactus-compute)) ships an on-device
inference stack aimed at phones, wearables and embedded hardware. Three pieces matter
here:

- **Cactus engine** — a C++ runtime with quantization (down to 2-bit), kernels, and
  bindings for Swift, Kotlin, Flutter, React Native, Python and **Rust**. A `cactus serve`
  command exposes an **OpenAI-compatible HTTP server**. Metal backend for Apple Silicon;
  ARM and Android supported. Install via `brew install cactus-compute/cactus/cactus`. It
  loads GGUF models (Qwen, Gemma, Llama, DeepSeek, Phi, Mistral, SmolLM, Needle).
- **Needle 3** — their own 121M-parameter automation foundation model, 8–29 MB on disk,
  shipped as a Laddered Simple Attention Network so you can pick depth (2–20 layers) at
  deploy. Does tool calling, structured extraction, and embeddings. Streams; emits JSON
  (`function_calls`, `reasoning`, `confidence`); supports JSON-schema tool specs
  (`--tools tools.json`). Apache-2.0 open weights.
- **Cactus Hybrid** — probes that score a local model's confidence and route the
  low-confidence slice to a frontier cloud model. Their Gemma-4-E2B hybrid reports
  parity with Gemini 3.1 Flash-Lite while sending only 15–55% of queries up.

### Pricing, licence, hardware

- **Engine licence is NOT permissive.** The `cactus` repo ships a custom licence: free
  for individuals, orgs under US$2M in both funding and revenue, educational institutions
  and non-profits — everyone else needs a paid commercial licence. Needle 3 weights are
  Apache-2.0. If Runo ever grows past the engine's revenue threshold, bundling it would
  require a licence negotiation. Flag for legal.
- Runs CPU-only; prefers Metal/NEON when present. Published RAM footprints range
  ~630 MB–1.4 GB depending on the GGUF loaded. Apple notarization/sandbox is a
  known-good path (Cactus already ships inside iOS apps), but a bundled binary engine
  adds entitlement and signing work in Runo's packaging.

### Benchmarks on code tasks

This is the key weakness for Runo. Published numbers:

| Benchmark | Needle 3 (121M) | Peers |
| --- | --- | --- |
| Mobile Actions (Cactus's own tool-call bench) | 86.0% | DeepSeek V4 Flash 88.4%, LFM2.5 1.2B 82.4%, Qwen3.5 0.8B 76.0% |
| BFCL v4 (broader tool calling, penalises bad calls) | 50.2% | LFM2.5 1.2B 62.0%, Qwen3.5 0.8B 56.8% |
| HumanEval | not published | — |
| SWE-bench | not published | — |

Cactus explicitly says Needle "gives up general chat ability to specialise in the
three things an on-device agent actually needs" (tool calls, extraction, embeddings).
A separate quantization study in their stack shows HumanEval collapsing from 54.88
(FP16 base) to 1.02 at CQ2 — i.e., aggressive quantization is incompatible with code
generation. Needle itself ships at 2-bit. Treat it as a tool-call/extraction model,
not a coding model.

Nothing on long-context performance or tokens/sec on consumer Macs is published in a
reproducible form; the headline claim is "sub-50 ms time-to-first-token" on-device.

### Features relevant to how Runo drives agents today

- Streaming output: **yes** (callback-based).
- Tool / function calling: **yes**, both Python decorator and JSON-schema modes.
- JSON mode / structured output: **yes**, grammar-constrained decode on Needle.
- Long context: **not disclosed**; mobile-first design suggests small windows.
- `stream-json` or Claude-Code-style headless CLI parity: **no** — the server is
  OpenAI-compatible, which is a different protocol from what Runo's agent CLIs use.

## Fit for Runo

Before scoring, note what the task's candidate sites actually do today — because
several of them are not LLM calls at all:

- `git::branch_slug` (`native/runo-app/src/git.rs:254`) — pure deterministic:
  takes the first non-blank line, filters stop-words, keeps up to 5 words / 40 chars.
- `crew::title_from` / `self_title` (`native/runo-app/src/crew.rs:1018`, `:341`) —
  deterministic: first line, cut to 60 chars at the last space.
- `chatctl::summary` (`native/runo-app/src/chatctl.rs:77`) — deterministic:
  formats git changes, recent commits and notes via `handoff::brief`. **The actual
  compaction summarisation is delegated to the agent CLI**, e.g. Claude's `/compact`.
- `modelcheck.rs` — probes the user's agent CLIs with a tiny prompt to classify
  errors; does not itself drive a model.
- `prwatch::ci_prompt` / `review_prompt` (via `ship.rs:348`, `:370`) —
  deterministic prompt builders handed to the user's agent CLI on wake.
- Crew routing (`parse_actions`, `within_limits`, `wake_prompts` in `crew.rs`) —
  deterministic parsers over the `runo-actions` fenced block. The intelligence
  (deciding to start a thread, writing a brief, picking who to wake) runs inside
  Crew's chat session, which is itself driven by the user's frontier agent CLI.

So the "small-model-wins" argument has to clear a higher bar: there is almost nothing
costly to replace, and inserting a local model would add latency, binary weight and a
licence to pure string functions.

| Candidate use | Fit | Why | Effort | Risk |
| --- | --- | --- | --- | --- |
| a) Primary coding agent (replace Claude Code / Codex / Grok Build) | **No** | No HumanEval/SWE-bench numbers; Needle is explicit about trading chat for tool calling; 2-bit quant known to break code gen. No `stream-json`, no resume, no CLI parity with the three agents Runo drives. | — | — |
| b) Crew's routing / coordination model | **No** | Crew doesn't call an LLM for routing — `parse_actions` and `wake_prompts` are deterministic. The reasoning (write brief, pick effort/model, decide to wake) happens in Crew's own chat session with a frontier CLI. Swapping Opus/Sonnet for a 121M-param local model on the hardest reasoning surface in Runo is a regression, not a win. | L | High |
| c-i) `branch_slug` / `title_from` | **No** | Already deterministic and good. "runo/fix-login-redirect" from a 20-line stop-word filter is not worth a 30-MB binary + Metal context + a licence. | S | Low value |
| c-ii) `chatctl::summary` for compaction | **No** | The *framing* is deterministic; the *summarisation* is already done by the agent CLI's `/compact`. There is no local LLM call to insert. | S | Medium (quality drop on compaction) |
| c-iii) `modelcheck.rs` post-turn error classifying | **No** | Classification is already regex over known CLI error strings. A model would introduce variance in exactly the diagnostic path that needs to be deterministic. | S | High |
| c-iv) `prwatch.rs` wake prompts | **No** | Deterministic templates; nothing to generate. | — | — |
| d) Embeddings for session / PR-comment / codebase search | **Maybe later** | Needle 3 does ship embeddings, Apache-2.0, tiny. Could enable semantic search over `sessions.rs` content and PR comments. But current search works, there's no user complaint, and Runo doesn't have a vector store today — this is new infrastructure, not a swap. | M | Low |
| e) Confidence-scored tool-call pre-check (via Cactus Hybrid pattern) | **Maybe later** | *If* Runo ever intercepts an agent's tool call before executing (e.g. "does this `rm -rf` match what the task asked?"), a local small model with confidence routing could pre-screen cheaply. Speculative; not on the roadmap. | L | Medium |

## Recommendation

**Start with: nothing. Open no PR today.**

Every concrete candidate the task names is either (a) deterministic string code where
an LLM would be a strict regression, or (b) a surface where the real LLM already runs
inside the user's agent CLI and is a frontier model by design — the exact place
substituting a 121M-param local model would hurt most.

**Skip (no fit today):**
- Primary coding agent — Cactus doesn't aim at this market and has no code benchmarks.
- Crew routing — routing is parse-based; swapping the chat's frontier CLI for Needle
  is a regression.
- Branch slugs, session titles, PR-watch prompts — already deterministic.

**Revisit when:**
- Cactus publishes HumanEval / SWE-bench numbers for a model you can actually bundle
  (not CQ2-quantized Needle). Until then, no evidence it's "close to Sonnet on code."
- Runo picks up a feature that genuinely wants an on-device LLM: semantic session
  search, PR-comment clustering, offline Crew answering when no network. Embeddings
  via Needle is the cleanest first step there — Apache-2.0 weights, HTTP server, well
  under 100 MB on disk.
- Cactus relaxes the engine licence, or Runo's distribution model makes a commercial
  Cactus licence trivial. Today's custom licence is a non-trivial legal checkbox.

**Sketch (do NOT build without a Crew decision):** if later we do want to try Needle
for embeddings on past sessions, the smallest shape is:

1. User-installed `cactus serve --model needle3 --embeddings` on 127.0.0.1 — Runo does
   not bundle the binary (sidesteps notarization and the engine licence).
2. A `trait Embedder { fn embed(&self, text: &str) -> Vec<f32>; }` with one impl that
   hits the local HTTP endpoint and a `None` impl when the daemon is down. Search
   stays text-first; embeddings augment, don't replace.
3. Index lives next to `.runo/` state; recompute on session close.

That's it — a single trait, one HTTP call, feature-flagged. If Crew wants that, it
can start a small thread. No PR here.

## Sources

- https://cactuscompute.com/
- https://github.com/cactus-compute/cactus
- https://github.com/cactus-compute/needle
- https://github.com/cactus-compute/cactus-hybrid
- https://docs.cactuscompute.com/v2.1.0/
- https://docs.cactuscompute.com/v1.14/docs/choose-sdk/
- https://www.mindstudio.ai/blog/needle3-vs-larger-models-benchmark
- https://www.mindstudio.ai/blog/needle-3-benchmarks-vs-larger-models
- https://betterstack.com/community/guides/ai/needle3-edge/
- https://news.ycombinator.com/item?id=44524544
- https://www.ycombinator.com/companies/cactus
