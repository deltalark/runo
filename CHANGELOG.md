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
- Inbox lists agents waiting on a permission, plan or question, with Allow, Approve and answers inline; the bell, rail, Home and sidebar share one "Needs you" rule and the same status words.
- Quick open no longer opens the wrong item, and adds projects, Add project, split, close tab, settings pages and the open session's files from any mode.
- The layout comes back after a restart: mode, split, window size and place, right-panel tab, open projects and sidebar width; a panel you hid stays hidden.
- Tabs scroll, list in an overflow menu and drag into a new order. New shortcuts for close, next and previous tab, Inbox, split, message box, archive and the context menu (Shift+F10); arrow keys move through the sidebar, Home and Inbox.
- In a terminal, Ctrl+Shift shortcuts and Esc twice get you back to Runo. Esc no longer switches modes.
- Top bar: the project switcher handles empty projects, the Agents pill lists running agents to open or stop, spend shows today, the branch and breadcrumb are clickable, and Runs on shows the session's real connection and moves it.
- Home follows the top-bar project with an All projects toggle and Add project. Reviews follows it too and badges only review requests and failed checks. A project whose folder is gone offers Relink or Remove.
- App menu with Quit, and the window reopens from the Dock on macOS. Closing the last tab stays in Sessions, the tour starts in Sessions, existing users get a one-time What's new card, shortcut hints follow your keybindings, and blank session names are refused.

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
