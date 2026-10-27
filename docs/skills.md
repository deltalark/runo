# Skills

Runo has two things it calls a "skill":

1. **User-authored skills (`SKILL.md`)** — folders with a `SKILL.md` file under
   `~/.claude/skills`, `~/.agents/skills`, `~/.codex/skills` or `.claude/skills` in a
   project. Each one is a short markdown prompt the agent loads on demand. This is
   Anthropic's definition of the word; Runo discovers them in Settings → Skills.
2. **Built-in skills** — small MCP tools Runo ships. Each one works in every agent
   Runo supports (Claude Code, Codex, Gemini CLI, Cursor, Antigravity, Grok, Qwen
   Code, OpenCode, Amp, Droid, …) with no extra wiring per CLI, because they ride
   on the same memory server every agent already loads.

The first built-in skill is **image generation**. More are planned: PDF reading,
web search, charts.

## Turning a built-in skill on

Settings → Skills shows a **Built-in skills** section at the top. Flip the switch
for the one you want. Each one with its own setup (an API key, a backend choice)
reveals its own section once it is on.

Enabled tools are added to Runo's no-ask list, so the agent can call them without a
permission prompt. A disabled skill still answers in band: the agent gets a short
"turned off in Runo Settings > Skills" reply, no popup lands on you.

## Image generation

**Tool exposed:** `generate_image(prompt, size?, style?, n?)`.

**What it does:** given a text prompt, calls the chosen backend, saves each PNG under
the session's `.runo/attachments/generated/` folder and returns the paths. The chat
image viewer picks them up automatically; click to open the lightbox.

### Backends

| Backend | What to set | Where keys come from | Notes |
| --- | --- | --- | --- |
| OpenAI | `gpt-image-1` or `dall-e-3` | <https://platform.openai.com/api-keys> | Default; cost reported per image (fallback ~$0.04). |
| Stability | Engine (`sd3`, `sd3.5`, `core`) | <https://platform.stability.ai/account/keys> | Multipart form, PNG back. Aspect ratio derived from size. |
| Replicate | Model id (`owner/name` or version hash) | <https://replicate.com/account/api-tokens> | Poll-based; up to two minutes per batch. |
| Local | A shell command | — | Runs on this computer. Prompt on stdin, output path as `$1`, size as `$2`. Example: `sdcli --steps 30 --out`. Works with any wrapper around Stable Diffusion / ComfyUI / Draw Things CLI. |

### Keys

API keys live in `settings.json`, which Runo writes 0600 (owner-read only). Any
field whose name says `key` is masked in diagnostic bundles by `applog::hide_secrets`.
No key ever ends up in a log line.

### Daily cost ceiling

Settings → Skills → Image generation → **Daily cost limit** (cents). 0 means no
cap. The server adds up what each backend reports; a hit returns a short refusal.

## Adding a new built-in skill

New skills live in `native/runo-app/src/skills/`. One file per skill, plus one line
in `skills/mod.rs::all()`. The shape is:

```rust
pub struct MySkill;

impl BuiltinSkill for MySkill {
    fn id(&self) -> &'static str { "my-skill" }
    fn name(&self) -> &'static str { "My skill" }
    fn description(&self) -> &'static str { "What it does for the agent." }
    fn tools(&self) -> Vec<Value> { vec![/* MCP tool schemas */] }
    fn call(
        &self,
        dir: &str,
        settings: &SkillSettings,
        tool: &str,
        args: &Value,
    ) -> Value { /* carry out the tool */ }
}
```

Then:

1. Append it to `all()` in `skills/mod.rs`.
2. Append every tool name it exposes to `ALL_TOOL_NAMES` so the agent harness
   lets its tool through without a per-call prompt.
3. If it needs per-skill settings, add a `MySkillSettings` struct and reference
   it from `SkillSettings`.
4. In `settings_ui.rs::builtin_skill_sections`, add a `match` arm that returns
   the skill's Settings section when its switch is on.
5. Add tests next to the skill. The pattern of the image skill's tests (parsing
   request shapes, mapping to each backend's request body, parsing fixture
   responses, path handling) covers the ground to copy from.

Runo picks skills up automatically; no changes elsewhere are needed.

## Where skills run

Built-in skills run **inside the Runo app process** on this computer. The MCP
transport is Runo's own memory server (`runo --mcp-memory`), which agents start
per turn and connect to over stdio. One wiring, every agent. A session running on
a Cove VM or a remote server reaches the server through the connection relay in
the same way the memory tools do, so skills work from anywhere a session runs.

Image generation itself sends the prompt to the chosen backend over HTTPS (via
`curl`), so **the prompt leaves this computer** for every hosted backend. The
Local backend keeps everything on the computer.
