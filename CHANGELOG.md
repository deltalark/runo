# Changelog

All notable changes to Runo are documented here.

## [Unreleased]

### Added

- Phone bridge: a token-protected API for a phone app (coming next) to see every session and what it needs, allow or deny permission requests, approve plans, answer questions, send follow-ups and stop runs. Turn it on in Settings → Phone or run `runo --phone`. Each paired phone has its own key and can be removed on its own. It listens on your Tailscale address, or only on this computer, unless you choose your local network.
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
- Local agent turns keep running when Runo closes or crashes; Runo follows them again on its next start instead of marking them Interrupted.
- Limits in Settings → General: a card with Stop when an agent goes quiet for too long, spending limits per session and per day that hold new turns (with Send anyway), and a cap on agents running at once with a visible "Waiting for a slot".
- A log file per launch and crash reports in `~/.runo/logs`, with a card on the next start to view or report a crash.
- Runo checks GitHub for a newer release and offers to download it; nothing installs on its own. A release workflow builds signed and notarized macOS zips and disk images when the signing secrets are set.
- Each session is saved in its own file under `~/.runo/sessions`, only when it changed and off the UI thread. The old `state.json` is split on first start and kept as `state.json.v1-backup`.
- Chat context control: /compact (Claude Code's own, else a fresh conversation seeded with a summary) and /clear, also in the session's right-click menu. Fork a session from any turn, with the files as they were then, and edit and resend any sent message, which rewinds the files and conversation or sends it as a fork.
- The / menu lists the agent's own commands besides playbooks: its built-ins, custom commands (.claude/commands, ~/.codex/prompts, .gemini/commands and the equivalents of other CLIs) and Claude Code skills.
- Sessions page: search over titles, branches, projects and past conversations (matches show a snippet), filters for project, agent, status and archived, and sort by recent, created or name. Quick open lists archived sessions (marked) and sessions whose conversation mentions what you type.
- New session: pick the base branch (local or remote) and get a branch named after the task, such as `runo/fix-login-redirect`, numbered when taken.
- New project can clone a repository from a URL into a folder you choose, with git's progress and errors shown, then adds it.
- Design mode: collect design references per project (pasted or picked images, captures of the preview) with notes, keep design notes in `docs/design.md`, see the app's dev server beside them, and start a session with the picked references attached.
- Architecture mode: a map of the project's packages, dependencies and module imports read from its manifests and code, architecture notes and decisions kept in `docs/architecture.md` with a generated Mermaid diagram, and agent sessions that start with both attached.
- Reviews refreshes every five minutes in the background, filters pull requests and issues by state, author and review request, loads past 30 with Load more, and lets you pick the agent for sessions started there.
- Review this PR: the agent writes its review to `.runo/review.json`, and Post review shows it for confirmation, then posts it to GitHub with its line comments.
- Playbook schedules can run at a time of day on chosen weekdays, or on a cron expression. Runs missed while Runo was closed catch up once on launch (Settings can turn this off).
- On macOS, scheduled playbooks can run while Runo is closed, from a launchd job: each runs in a new worktree after the project's setup command, with the agent's settings and the playbook's permissions, and shows up as a session when Runo opens.
- Settings → Providers and each agent's page list every copy of its CLI on this computer (PATH, nvm, ~/.local/bin, Homebrew, npm, Bun, Cargo, Runo's own) with version and path. Pick the one Runo runs, Use newest when an older copy comes first, Update the one in use, or remove a stray link after a confirmation that names it.
- Each agent's page lists its models from the CLI's own list, Codex's hidden and retiring ones included with their dates, with Use as default and a Test that tries each model with a one-line prompt and marks it works, not available on your account, or needs a newer CLI. Codex can also save the default into ~/.codex/config.toml (off by default; only the model line changes, after a backup).
- A turn that fails because the agent is signed out, the model isn't available, or the CLI is too old for the model offers the fix under it: Sign in (runs the agent's login in the terminal panel and checks again), Retry with a model that works, Pick a model, Use the newer copy, Update or Install for Runo, and make the working model the default for the project or every session. Playbook runs get the same card.
- The composer's model menu marks models that need a newer CLI or are being retired, and a session on a retiring model offers a one-click switch to its replacement.

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
- Claude Code asks for permission while its turn runs: Allow once, Always allow or Deny in the corner card or the Inbox, and the turn carries on. Requests from servers wait for a reconnecting Runo (Settings → Chat → Wait for permission answers).
- The CLI view works for sessions on a Cove VM or server, syncing the files and conversation there and back.
- The worktree setup command runs in a terminal tab, so its prompts can be answered; the agent starts when it exits.
- The Browser tab lists dev servers running where a remote session runs and forwards one to this machine to open it.
- Terminal: dim and strikethrough text are drawn (vt100 is now vendored with both attributes).
- Cursor, Gemini CLI and Droid get Runo's memory server and shared MCP servers through their project config file, merged with yours, kept out of git and removed after each turn. Grok has no project-level MCP config, so it still gets none.
- Settings → MCP adds and edits Runo's own servers in a form (name, command or URL, environment) instead of a JSON file.
- The CLI view and terminals pass middle and right clicks, drags and the wheel to programs that turn on mouse reporting, at the cell under the pointer; Shift keeps local selection.

- A repository with no commits no longer fails to start an isolated session: Runo's git init makes a first empty commit, and otherwise the session runs in the project folder with a note.
- Quit (Cmd+Q, the menu and Cmd+K) warns about running agents, the same as closing the window.
- Agents: OpenCode and Codex errors show their message instead of nothing or raw JSON, a turn that reports an error counts as failed, and expired OpenCode and signed-out Gemini sign-ins get the sign-in hint.
- Gemini: the session's mode applies in new worktrees (Runo marks the folder trusted), and follow-ups resume the conversation by its id.
- Codex: commands, edits and reasoning show while they run, and reply text streams when the CLI sends updates.
- Notifications are on by default, cover turns waiting for you, open the session when clicked, and work on Windows. Permission requests can wait indefinitely.
- Agents that can't resume (such as Grok) keep a one-line summary of earlier requests; Copilot sessions no longer pick up each other's conversation.
- Tests pass on macOS.
- Playbooks started in Runo wait for their worktree and run the project's own setup command there, instead of possibly running the setup in your home folder.
- Playbooks run while Runo is closed use the agent's settings, run the setup command first, edit in every agent's edit mode, and let Claude Code run commands when allowed.
- Codex: no Approval chip in Chat, where `codex exec` ignored it; warnings such as missing model metadata show once in the chat, and a model your account can't use points to the model chip.
- Sign-in is checked ahead of time: Settings → Providers and New session show signed-out agents, and a turn on one shows the sign-in hint before it starts.
- Spending limits, the spend shown and each session's context meter say that only Claude Code and OpenCode report a cost.
- Architecture reads Swift, Maven, Gradle, Ruby, .NET, PHP and Dart manifests.
- A CI workflow checks formatting, Clippy and the tests on macOS and Linux for every push and pull request.
- Agents: a model an outdated Codex rejects ("not supported when using Codex with a ChatGPT account", "requires a newer version of Codex") is reported as a version problem, not blamed on the account, when a newer Codex wrote its model list.
