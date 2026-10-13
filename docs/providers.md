# Provider setup

Runo drives the agent CLIs you already have installed and signed in. It does not sell or proxy tokens.

## Supported agents

Install at least one, sign in, and make sure it is on your `PATH` (or point Runo at it with Locate… in Settings):

| Agent | Command | Output Runo reads | Follow-ups resume with |
| --- | --- | --- | --- |
| Claude Code | `claude` | `-p --output-format stream-json` | `--session-id <uuid>` on the first turn, then `--resume <uuid>` |
| Codex | `codex` | `exec --json` | `exec … resume <thread id>` |
| Gemini CLI | `gemini` | `--output-format stream-json` when its help lists it, else text | `--session-id <uuid>` when its help lists it, then `--resume <uuid>` |
| Qwen Code | `qwen` | `stream-json` when its help lists it, else text | `--session-id <uuid>`, then `--resume <uuid>` |
| OpenCode | `opencode` | `run --format json` | `-s <session id>` |
| Cursor | `cursor-agent` (or `agent`) | `-p --output-format stream-json` | `--resume <chat id>` |
| Copilot | `copilot` | `--output-format json` when its help lists it, else text | `--session-id <uuid>` when its help lists it, else the id from `~/.copilot/session-state`, then `--resume <id>` |
| Amp | `amp` | `--stream-json` | `threads continue <thread id>` |
| Droid | `droid` | `exec -o stream-json` | `-s <session id>` |
| Grok | `grok` | text | Can't resume; a summary of earlier requests and the recent conversation go in each prompt |

Settings → Providers lists which are installed, their versions and paths, and whether each is signed in. If you install or update a CLI while Runo is open, use Re-detect. Agents that aren't installed can be installed from there with the vendor's installer.

## Sign-in

Runo checks each installed agent's sign-in in the background when it starts and when Settings or New session opens, and again after every turn:

- An API key the CLI reads (such as `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` or `GEMINI_API_KEY`), set in your login shell or in the agent's Environment setting, counts as signed in.
- Codex and Cursor are asked directly (`codex login status`, `cursor-agent status`).
- Claude Code: its login in the macOS keychain, or `~/.claude/.credentials.json` elsewhere.
- Gemini CLI: `~/.gemini/oauth_creds.json` or `~/.gemini/.env`. OpenCode: `~/.local/share/opencode/auth.json`.
- Copilot, Amp, Droid, Qwen Code and Grok are only checked for their API keys; without one Runo can't tell, and says nothing.

A signed-out agent is marked in New session and in Settings → Providers, and a turn sent to it shows how to sign in before it starts. A turn that fails because of the sign-in shows the same hint.

## Per-agent settings

Each installed agent has its own page under Settings → Providers: a default model, the executable, extra arguments and environment variables, plus agent-specific options (Claude Code's allowed and denied tools, appended system prompt and fallback model; Codex's config profile; OpenCode's agent, variant and thinking; extra directories where the CLI takes them).

## Permission levels

The composer's Permission chip offers Runo's three levels where the CLI can hold to them:

| Agent | Auto edit | Plan only | Full access |
| --- | --- | --- | --- |
| Claude Code | `--permission-mode acceptEdits` | `plan` | `bypassPermissions` |
| Codex | `--sandbox workspace-write` | `read-only` | `danger-full-access` |
| Gemini CLI, Qwen Code | `--approval-mode auto_edit` | `plan` (or `default` on older CLIs) | `yolo` |
| Copilot | `--allow-all-tools --deny-tool shell` | also `--deny-tool write` | `--allow-all-tools` |
| OpenCode | its default agent | `--agent plan` | `--auto` |
| Droid | `--auto medium` | read-only (no `--auto`) | `--auto high` |
| Cursor | not offered (edits come with commands) | without `--force` | `--force` |
| Amp | its default | not offered (no read-only mode) | `--dangerously-allow-all` |
| Grok | not offered | not offered | always |

Codex has no Approval chip: `codex exec` never stops to ask, so an approval policy changes nothing in Chat. The policy in your Codex config still applies in the CLI view.

## MCP servers and memory

Runo's memory server and the MCP servers you share reach every agent except Grok: by flag or environment for Claude Code, Codex, OpenCode, Qwen Code, Copilot and Amp, and through their project config file for Cursor, Gemini CLI and Droid (merged with yours, kept out of git and removed after each turn). Grok has no project-level MCP config, so it gets none.

## Cost and spending limits

Only Claude Code and OpenCode report what a turn cost. The spend in the top bar and on Home, and the spending limits in Settings → General, count only those; turns of the other agents count as $0, so the limits don't hold them. A session on an agent that reports no cost says so next to its context meter.

## Images

Images can be attached for Claude Code, Codex, OpenCode, Gemini CLI and Amp, and for Copilot once its help lists `--attachment`.
