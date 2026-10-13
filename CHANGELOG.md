# Changelog

All notable changes to Runo are documented here.

## [Unreleased]

### Added

- Connections: run sessions on this machine, in a Cove VM (local or on a server) or on any SSH server, and move a session between them from its header. Files, uncommitted changes and the agent's conversation are synced over the connection before and after every turn, and terminals open there.
- Turns on a Cove server or SSH server keep running when your computer sleeps, shuts down, loses the network or closes Runo. Runo follows the agent's output there, reconnects after drops, and picks the turn back up on its next start: the reply replays and the work and conversation come back.
- Settings → Connections with Test, VM size, network and idle stop for Cove, forwarded keys, optional GitHub access, and a list of this machine's Cove VMs.
- Agents can use connections themselves through Runo's MCP server (`list_connections`, `run_on`) and Cove's own tools.
- Playbooks as shareable `.playbook.md` files: install from a git repo, link, folder or the clipboard, export, update from the source, inputs asked before a run, and declared permissions.
- Native desktop client in pure Rust for macOS, with Windows and Linux also targeted.
- Projects and sessions with isolated git worktrees, live agent output, real git diffs, a file browser and a command terminal.
- Claude Code, Codex and OpenCode support. Models, permission and sandbox modes, effort and reasoning levels, speed tiers and agents are discovered from each installed CLI.
- Per-agent settings: default model, executable, extra arguments, environment variables and agent-specific options.
- Settings screen with search, light and dark themes, accent colors and colorblind-friendly diff colors.
- Desktop notifications when an agent finishes (macOS and Linux).

### Fixed

- Agents: permission modes now match what each CLI can do. Plan is read-only everywhere it is offered, Grok runs in full access only, and Amp, Copilot and Cursor respect Edit vs Full.
- Agents: Runo's memory server and shared MCP servers reach every CLI that takes servers by flag or env (Claude, Codex, OpenCode, Qwen, Copilot, Amp), and local servers are skipped for sandboxed or remote runs.
- Agents: Codex, Cursor, Droid, Gemini, Qwen, Copilot and OpenCode turns stream as structured JSON, with live steps, conversation ids, resume and token usage.
- Images: drafts and attachments belong to each session, queued prompts keep their images, files live in `.runo/attachments` in the worktree, and the attach button only shows for agents that read images.
- Queue: Send now and Retry, and the queue drains in order after a failure, a stop or a restart; a prompt sent during setup is queued.
- Turns: Stop ends the agent's whole process group, a first-turn stop keeps the conversation, and the split pane has a Stop button.
- Turns: a sign-in failure, even with exit code 0, shows how to sign in instead of an empty reply.
- Questions: an agent asking several questions gets an answer to each, and the alert stays after an error.
- CLI view: turns typed in the CLI are kept on quit, and the CLI no longer opens over a running chat turn.
- Handoff and history: recaps work for more agents without changing the conversation, past conversations load in the background without duplicates, and a resumed one shows its earlier turns.
