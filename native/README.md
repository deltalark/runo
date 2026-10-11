# Runo

Native desktop app for running coding agents (Claude Code, Codex, OpenCode) in parallel, each in its own git worktree. Written in Rust on [GPUI Kit](https://gpui-kit.com/); targets macOS, Windows and Linux.

```bash
cargo run -p runo --release
```

## What works

- **Projects:** add any git repository; projects and sessions persist in `~/.runo/state.json` (override with `RUNO_HOME`).
- **Sessions:** start an agent on a task in a new isolated worktree (`<repo-parent>/.runo-worktrees/<repo>/<branch>`) or in the project directory. Output streams live into the conversation. Follow-up messages continue the Claude Code conversation. Stop and Delete (which removes the worktree) are in the session header.
- **Changes:** real `git` status and diffs for the session's worktree, with +/- counts.
- **Files:** browse and preview the worktree's files.
- **Terminal:** run shell commands in the session's directory with streamed output. This is a command runner, not a full PTY.
- **Browser:** opens URLs in the system browser. There is no embedded browser yet.

Agents run with `claude -p --permission-mode acceptEdits`, `codex exec --full-auto` and `opencode run`; they need to be installed and signed in.

## Not built yet

Interactive PTY terminal, embedded browser, CLI control surface, remote/SSH worktrees, PR integration, auto-scroll in the conversation.
