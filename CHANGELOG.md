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
- Connections sign agents in: API keys forward even when set only in your login shell, Codex and Claude (Linux) login files are copied, and Test checks each agent's sign-in without booting a VM.
- Agent CLIs install on SSH and Cove servers for every agent (npm into a private Node, or the vendor's script, with sudo-aware apt) and follow the version you run locally.
- Sync problems after a remote turn show as a warning card with Restore kept changes.
- Moving a session hands the agent a brief when its conversation can't follow, and non-git folders can't be moved to remote connections.
- Detached Claude Code turns stop at the first refused command, so you can allow it and retry.
- Runo's own MCP tools reach sessions in Cove VMs and on servers through a relay, and shared MCP secrets stay out of remote command lines.
- Remote turns work on macOS hosts without setsid or GNU tail, and a turn that never starts ends instead of waiting forever.
- Backups to origin are off by default, never made for public repositories, leave out untracked files unless asked, and are deleted with the session; leftover copies, job folders and scratch VMs are cleaned up.
- Playbooks are held to their permissions on every turn and agent, with a warning where an agent can't be held, and agents ask before starting one.
- Scheduled playbooks run on the project's default connection with its setup command, never overlap, note missed runs, keep a set number of sessions and record a run history.
- Playbook updates show what changes and aren't pre-ticked when you edited your copy; input labels and defaults are editable, and `{{ key }}` works with spaces.
- Runo keeps its own list of MCP servers for every agent, and Settings can start a new skill.
- Agent installs can be cancelled, stop after 15 minutes, and offer to sign in when done.
