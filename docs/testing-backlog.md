# Testing backlog

These are things that are built but not yet verified end to end. Runo's cloud test environment is Linux under Xvfb, with only the real Claude Code CLI installed. Codex and npm are fakes, and Gemini is a wrapper around Claude Code. Anything that needs another agent CLI, another operating system, a desktop compositor or a real account is listed here.

Each item says what to do and what should happen. Tick it off with the date and the version tested.

## 1. Agents other than Claude Code

Their flags come from each vendor's documentation, checked on 2026-10-11. On 2026-10-13 the real Codex, OpenCode and Gemini CLIs were run on a Mac (main 0e95854): Codex chat across two turns works, and the Gemini and OpenCode flags are accepted. Real Gemini and OpenCode turns are still untested because both were signed out.

| Agent | What to test | Expected |
| --- | --- | --- |
| ✓ Codex (2026-10-13, macOS, main 0e95854) | Chat: two turns, then ask "what did I ask first?" | The second turn runs `codex exec … resume <id>` and remembers the first |
| Codex | A model the account can't use, such as one with no metadata | Codex's warning shows once above the reply, and the failure points to the model chip and Settings → Codex |
| Codex | CLI view: switch Chat → CLI → Chat, do one turn in the CLI | The TUI opens with `codex resume <id>` on the same conversation, and the CLI turn appears in chat under the divider |
| Codex | First chat turn in a new session | Runo finds the conversation id from `~/.codex/sessions` (newest rollout in the folder) |
| Gemini CLI | Chat: two turns | The first turn sends `--session-id <uuid>` and the second `--resume <uuid>` (Gemini 0.46 takes the id directly), and the context is kept |
| Gemini CLI | CLI view | `gemini --resume <uuid>` opens the same chat. Coming back shows "Used Gemini's CLI", and the next chat turn knows what happened in the CLI |
| Qwen Code | Same two checks as Gemini | Same results (`--session-id` / `--resume`) |
| Cursor CLI | Chat with `--output-format json` | The reply text shows, `session_id` is saved, and the next turn sends `--resume <id>` |
| Cursor CLI | CLI view | `cursor-agent --resume <id>` opens the same chat |
| Copilot CLI | Chat | After the first turn, the session id is picked up from the newest folder in `~/.copilot/session-state/`, and the next turn sends `--resume <id>` |
| Copilot CLI | Two Copilot sessions running at once | Each keeps its own id. The newest-folder lookup could pick the wrong one, so watch for that |
| Copilot CLI | CLI view | `copilot --resume <id>` opens the same session |
| Amp | Chat with `--stream-json` | Text and tool steps show, the thread id (`T-…`) is saved, and the next turn runs `amp threads continue <id> -x …` |
| Amp | CLI view | `amp threads continue <id>` opens the same thread |
| Droid | Chat with `droid exec -o json` | The reply text shows, `session_id` is saved, and the next turn sends `-s <id>` |
| Droid | CLI view | `droid --resume <id>` opens the same session |
| OpenCode | Chat with `run --format json` | Text from `{"type":"text","part":{"text":…}}` shows (this event shape is inferred from the source), `sessionID` is saved, and the next turn sends `-s <id>` |
| OpenCode | CLI view | `opencode -s <id>` opens the same session |
| Grok CLI | Chat | Not wired for resume yet: its docs don't say how to learn the session id. Context is still sent in the prompt. Check that it still works |
| All of the above | Model, effort and permission chips | Each chip changes the run as expected |
| Cursor | Model menu | Filled from `agent models`; check that the output format parses into ids and names |
| Copilot, Gemini, Qwen, Amp, Droid, Grok | Model menu | Filled from the `--model` choices in each CLI's help, where the help lists them. Agents whose help lists none show only Default |
| Claude Code on macOS (native and npm installs) | Model menu | "Default · <model>" plus versioned aliases and pinned versions; `/model` runs without creating a transcript |
| All of the above | Switching agents mid-session (handoff dialog) | Cost shown, brief carried over, new agent continues the task |
| All of the above | Memory server | Every agent but Grok gets it: by flag or environment for Claude Code, Codex, OpenCode, Qwen Code, Copilot and Amp, and through the project config file for Cursor, Gemini CLI and Droid. Check that each can call `read_memory`, and that Grok runs without it |
| All of the above | Sign-in check, signed in and signed out | Settings → Providers says Signed in or how to sign in, New session marks a signed-out agent, and a turn on one shows the hint before it starts. Codex and Cursor ask their CLI; Claude Code reads the macOS keychain; Copilot, Amp, Droid, Qwen Code and Grok are judged by API keys only |
| All but Claude Code and OpenCode | Spending limits | These report no cost: the session's context meter says so, and Settings and the top bar's spend say the limits don't count them |

## 2. CLI view (Chat ↔ CLI)

Tested with real Claude Code only: Chat → CLI → Chat → CLI, `/cost` unchanged by switching, CLI turns imported, `/exit` returns to Chat.

| What to test | Expected |
| --- | --- |
| Real keyboard typing and pasting a long prompt into the Claude Code TUI | Nothing is dropped. In automated tests, very fast synthetic typing lost text |
| Mouse in the TUI (clicking, selecting, scrolling inside the agent UI) | Clicks, drags and the wheel reach a TUI that turns on mouse reporting, at the cell under the pointer; Shift keeps Runo's own selection |
| Resizing the window and toggling the side panels while in the CLI | The TUI reflows to the new size |
| Switching to Chat while the CLI shows a permission prompt or is mid-answer | The switch is refused with a notice, and nothing is lost |
| Quitting Runo while the CLI is busy, then reopening | The CLI reopens on the same conversation, and the turns made before quitting are imported |
| Default view set to CLI, then creating a new Claude Code or Codex session | It opens in the CLI with the task as the first prompt |
| A queued chat prompt while the CLI is open | It is queued, then sent as soon as you switch back to Chat |
| IME and non-Latin input in the TUI | Characters arrive correctly |
| macOS: Cmd+C and Cmd+V in the CLI | Copies the screen and pastes |
| Ctrl+Shift+M from inside the CLI | Switches to Chat |

## 3. Operating systems and look

| Area | What to test | Expected |
| --- | --- | --- |
| Glass on macOS | Off, Subtle, Frosted and Clear | The desktop shows through, blurred |
| Glass on Windows 11 | The same four styles | Acrylic blur. Xvfb in the cloud cannot show transparency |
| Glass on Linux (KDE and GNOME, X11 and Wayland) | The same four styles | Transparent where the compositor supports it, and a clean opaque fallback where it doesn't |
| macOS title bar | Traffic lights, the 80px left inset, and tabs starting at the main column | No overlap |
| Windows | PTY shells (ConPTY), `pwsh`, paths with `\`, agent CLIs on PATH | Terminals and agents run |
| HiDPI and multiple monitors | Moving the window between screens | Text stays crisp and the terminal grid resizes |
| Light theme | The tour, switch dialog, CLI banner, popups and settings tree | They read well in light mode |

## 4. First-run tour

| What to test | Expected |
| --- | --- |
| A true first install on macOS and Windows | The tour starts once, and Skip or Done stops it for good |
| Small windows (1024×700) | Cards stay on screen and spotlights line up |
| Replay from Settings › General and from the palette | It starts again from step 1 |

## 5. Popups, updates and notifications

| What to test | Expected |
| --- | --- |
| Real `claude update`, `opencode upgrade` and `cursor update`, and npm for the others | "Update now" upgrades the CLI, and the version refreshes. Only a fake npm has been used |
| An update check with no network | No popup and no error spam |
| Real updates through the card, for each agent (npm with `--loglevel http`, `claude update`, `opencode upgrade`, `cursor update`) | The bar and the current line keep moving, then "Updated" shows and the card fades out. Only fake updaters printing percentages and npm lines have been tested |
| Codex owned by Orca, plus an nvm copy (Anish's Mac) | "Update needs you" names Orca, offers "Switch to 0.149.1", and the switch then updates the nvm copy with nvm's own npm and `--prefix`. Simulated in the cloud with fake files only |
| Homebrew formula and cask installs (`brew upgrade`, `brew upgrade --cask`) | The card runs brew and shows its output |
| Updating while the agent runs in a session | The card asks first and offers "Update anyway" |
| A copy earlier on PATH shadowing the updated one | After a "successful" update, the card says Runo still runs the old copy and offers to switch |
| A real npm failure (EACCES, ENOTEMPTY, EEXIST, offline) | The card turns into "Update failed" with a plain-words reason, Try again and View log. ENOTEMPTY names the leftover folder to delete |
| Desktop notifications on Linux and Windows (macOS checked 2026-10-13) | Notifications appear when an agent finishes or fails |
| Keep the computer awake (caffeinate, systemd-inhibit) | The machine doesn't sleep while agents run, and sleeps again afterwards |
| Permission popups from three sessions at once | Each answer reaches the right session |

## 6. Git and GitHub

| What to test | Expected |
| --- | --- |
| Commit, push and Create PR on a real GitHub repo with `gh` signed in | A PR opens with the message |
| The Pull requests page with a real account | PRs and runs list, and links open |
| The worktree setup command and `.worktreeinclude` | The setup runs first, and ignored files are copied in |
| A playbook started from the Playbooks page in a project with its own setup command | The project's command runs in the new worktree's terminal tab once the worktree exists, then the agent starts; nothing runs in the home folder |
| Racing agents on one task with real CLIs | Each agent runs in its own worktree, and picking a winner works |
| Deleting a session together with its worktree | The worktree and branch are removed |

## 7. Memory

| What to test | Expected |
| --- | --- |
| Codex with the memory MCP server (`-c mcp_servers.runo_memory…`) | Codex can call `read_memory`, `search_history` and `save_note`. Only Claude Code has been verified |
| A large history (hundreds of sessions) | `search_history` stays fast and returns at most 20 results |

## 8. Installing agents from Runo

Tested in the cloud with a fake `npm` only: the confirm card, the "Installing…" state, a CLI that lands off PATH (`~/.npm-global/bin`) being found and saved as the agent's binary, and a failed npm install showing its error. No vendor installer has been run for real.

| What to test | Expected |
| --- | --- |
| Claude Code on macOS and Linux (`curl -fsSL https://claude.ai/install.sh \| bash`) and Windows (`irm https://claude.ai/install.ps1 \| iex`) | Installs, Runo finds `claude` (often in `~/.local/bin`), and the card switches to Installed |
| Cursor (`curl https://cursor.com/install -fsS \| bash`) | The CLI is found even when the installer names it `agent` instead of `cursor-agent` |
| Droid with and without npm, and on Windows | npm installs `droid`; without npm the Factory script runs |
| OpenCode without npm on macOS and Linux | The `opencode.ai/install` script runs and `~/.opencode/bin/opencode` is found |
| Grok CLI install script | Installs and is found. On Windows the card says to use WSL |
| Codex, Gemini, Copilot, Amp and Qwen through real npm | Each installs globally and is detected |
| npm with a system prefix (EACCES) | The install card fails with a hint to run `npm config set prefix ~/.npm-global` |
| Progress during a real install | An "Installing" card shows the bar and the installer's output, then "Installed" with the sign-in hint, and fades out |
| No Node.js at all | The card explains that Node.js is needed instead of offering Install |
| An installer that asks a question on stdin | It does not hang Runo; the install fails with a message |
| A desktop launch on macOS (Finder/Dock, so PATH is minimal) | The newly installed CLI is still found, through the saved binary path |
| Signing in after install | The info line tells you to run the CLI once in a terminal; check that a first chat turn works afterwards |

## 9. Playbooks while Runo is closed (macOS)

| What to test | Expected |
| --- | --- |
| A scheduled playbook that comes due with Runo closed and "Run while Runo is closed" on | The launchd job runs it in a new worktree after the project's setup command, and it shows up as a session when Runo opens |
| The agent's settings (model, effort, extra arguments, Codex profile, Claude Code allowed tools) | The closed-Runo run passes the same flags as a run in Runo |
| A playbook allowed to edit, on Gemini, OpenCode, Droid or Copilot | It edits in the agent's edit mode instead of running read-only |
| A Claude Code playbook allowed to run commands, such as running the tests | It runs them (`bypassPermissions`, with web tools denied when the network isn't allowed); without commands it runs with `acceptEdits` |
| A read-only playbook on Amp | The run fails with a note that Amp has no read-only mode |
