# Runo

Run Claude Code, Codex and OpenCode in parallel, each in its own git worktree, with diffs, a terminal and per-agent controls in one native app.

Runo is written in Rust on [GPUI Kit](https://gpui-kit.com/) and targets macOS, Windows and Linux.

## Run it

Install and sign in to at least one agent CLI first (see [provider setup](docs/providers.md)).

```bash
cargo run -p runo-native --release
```

## What it does

- **Projects and sessions.** Add any local folder. Start a session on a task with an agent of your choice. Git repositories can use an isolated worktree per session so agents never collide.
- **Live output.** The agent's output streams into the conversation as it works. Text is selectable and copyable.
- **Changes.** Real `git` status and diffs for each session's worktree.
- **Agent controls.** The composer offers every option the installed CLI reports: model, effort or reasoning, permission or sandbox mode, speed tier, approvals and agents. Options are detected from the CLIs, not hard-coded.
- **Settings.** Themes, accent colors, per-agent launch options, and read-only views of detected MCP servers and skills.

Projects and sessions are stored in `~/.runo/state.json` and settings in `~/.runo/settings.json` (set `RUNO_HOME` to move them).

## Status

Early. Not built yet: an interactive terminal, an embedded browser, structured tool-step output from agents, and remote machines.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security reports: [SECURITY.md](SECURITY.md).
