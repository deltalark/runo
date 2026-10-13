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
- Landing work from the Changes panel: update a session from main (rebase or merge, with conflicts handed to the agent), merge into main, pull on the main branch, and clean up the worktree and branch afterwards. New worktrees start from the freshly fetched default branch.
- Per-file staging, "Commit N staged", and no commits while the agent is mid-turn. Commits, worktree creation, `.worktreeinclude` copies and refreshes run off the UI thread, and the Changes panel refreshes on focus and while idle.
- Create PR asks for title, body and base, saves the PR link, reuses an existing PR, never pushes the default branch, and logs only after it worked. Each session shows its PR's checks and reviews with Fix CI, Address review and Merge PR; Review this PR checks out the PR branch.
- GitHub page lists load and fail independently per project.
- Undo restores the agent's commits, file modes and symlinks, keeps your staging and your own edits, saves a snapshot so the turn can be redone, and tells the agent what was undone.
- Races: Keep this one stops the other racers, removes their worktrees and branches after confirming, then offers to merge or open a PR; racer counts include committed work and open that racer's diff against main.
- Deleting a session prunes its checkpoints and merged branch; branches like `a/b` and `a-b` no longer share a folder. Each project can have its own worktree setup command.
- Changes handles non-ASCII file names, shows "Binary file changed" and notes how many diff lines are hidden. Diff notes on removed lines say they refer to the old version and stay until the follow-up starts.
- Files tab is a folder tree with search in files, without the silent 2,000-file cap.
- The editor asks before dropping unsaved edits, keeps them across session switches, notices files changed on disk, opens non-UTF-8 files read-only and reports failed saves.
- Terminal: drag to select and copy, find in scrollback, Cmd/Ctrl-click links, mouse reporting for full-screen programs, italic text and a scrollback-length setting.
- Browser panel: Cmd/Ctrl-C copies the page selection, new tabs and popups open in place, console errors can be sent to the agent, downloads are saved, Linux browsers are found on PATH or via `CHROME_PATH`, and the dev-server list shows every listening local port.
- The login shell's whole environment (API keys and the like) now reaches agents when Runo is opened from the Dock or a launcher.
