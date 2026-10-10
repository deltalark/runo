# Runo native design

Runo is a developer workspace for several coding agents working concurrently. The unit of work is a **run**: one provider, one task, one checkout, one transcript, and a set of terminals and changes. A project groups runs. A Git worktree isolates each active run.

## Visual direction

The selected direction is a calm, light developer workspace. The references establish the three-column rhythm, but Runo uses its own wider, clearly grouped global sidebar and stronger hierarchy. The default view is one run, its changed files, a persistent composer, and an optional terminal dock. Diffs and browser pages open as tabs rather than crowding the conversation.

- Canvas `#ffffff`; global sidebar `#f6f8fa`; session sidebar `#fbfcfd`; selected row `#eaf2ff`; border `#e2e7ed`.
- Primary text `#1d2735`; secondary `#647286`; subdued `#8d9aac`.
- Action/focus blue `#1767e8`; success `#258a5b`; attention `#b6771b`; error `#c95358`.
- UI font: platform system sans; code, branches, paths, diffs and terminal: platform monospace. App title 18 px; pane title 15 px; body 13 px; metadata 11–12 px.
- 4 px spacing grid, 36–40 px navigation rows, 1 px structural dividers, 6–8 px radii. No dashboard card grid, decorative gradients or oversized hero copy.
- Global sidebar has distinct **Workspace**, **Create** and **Projects** groups with 16–24 px between groups and a quiet rule beside each group label. Icons align in a fixed column; labels have strong contrast. Settings is anchored at the bottom. The selected item has a tinted background and blue text.
- Run status uses a dot plus a word; no meaning is carried by color alone.

The images in [workspace.png](design/workspace.png) and [secondary-views.png](design/secondary-views.png) are visual concepts generated with the imagegen skill. Text and data in them are illustrative. This specification is the implementation source of truth.

## Layout

At wide desktop sizes, the content area has three stable zones:

1. **Global sidebar (216–232 px):** clearly separated Workspace, Create and Projects groups; Search, Inbox, Notes, Automations, Projects and Settings. It can collapse to icons at medium widths.
2. **Project sidebar (260–280 px):** Sessions / Explorer / Changes tabs, local search, New run, compact session rows showing provider, task, status, branch and time. It follows the current project.
3. **Work canvas (flexible):** tab strip across top, selected run transcript, file/diff, terminal, browser, worktrees or settings page. A prompt composer anchors the run view. A terminal dock can open beneath it.

Optional split panes are user initiated. The default run view stays single-column. The window uses host titlebar controls; content chrome works on macOS, Windows and Linux.

At 1100–1399 px, the global sidebar narrows and the terminal may collapse. At 800–1099 px, the project sidebar is collapsible. Below 800 px, show one work pane and a clear project switcher. Every primary action works by keyboard and pointer; shortcut labels use the platform modifier.

## Primary flows

### Open a project

Show recent projects with path, branch and availability. Opening a project restores its selected run, open tabs, split and terminal layout. A missing folder shows a repair action. No provider sign-in blocks project opening.

### Start a run

One focused form asks for provider, task, checkout, branch/worktree name and permission mode. Creating an isolated worktree is the suggested path when another run already owns the current checkout. The final button shows the actual branch and folder. Validation handles branch collisions, dirty source, missing provider CLI, missing auth and permission errors before launch. A launched run opens in the work area and appears under its worktree in the sidebar.

### Follow a run

The run tab shows provider/model, plain-language status, branch and worktree. The transcript groups user prompts, agent text, collapsible tool calls, approval requests and artifacts. A prompt composer stays visible while output streams. Follow-up behavior and permission mode are visible in the composer. A finished run exposes Review changes, Open terminal and Continue.

### Review changes

A changed-file list gives status and additions/deletions. A selected file opens a readable unified diff, with split mode, whitespace, stage/unstage and context-to-chat actions. Review comments remain drafts until submitted. Worktree identity is always visible above destructive actions.

### Use terminal and browser

Terminals are scoped to a run or project, with tabs, process status, cwd, search and split. Browser tabs are also scoped and show URL, navigation state and the owning run. Both can be docked or opened as main tabs. Closing a view does not silently stop a process.

### Manage worktrees

A compact table shows branch, path, run owner, dirty state, changed-file count and last activity. Creation and deletion preview their effects. Deletion shows active sessions and uncommitted files and requires explicit confirmation.

## Full screen map

| Area | Screens and states |
| --- | --- |
| Workspace | Empty project, active run, waiting for input, completed run, failed/disconnected run, multiple runs, split view |
| Project navigation | Recent projects, active/pinned/archived runs, branch groups, search/filter |
| Files | Explorer, editor, quick open, binary/large file, conflict |
| Source control | Changes list, unified/split diff, commit, branch, worktree management |
| Tools | Terminal, browser, CLI/command palette, global search |
| Coordination | Inbox, notes, automations, resident agents |
| Settings | General, Appearance, Keybindings, Chat, Providers, Connections, MCP, Skills, Notifications, Archive |
| Cross-cutting | New run, approval, error detail, confirmation, provider sign-in, offline/remote states |

The broader behavior for each screen is recorded in [docs/product-design.md](../docs/product-design.md). The native implementation should preserve these routes while keeping the compact work surface as its default.

## Reusable components

- Navigation item, tab, run row, branch group, status token, provider label.
- Text input, multiline composer, action button, segmented choice, menu, tooltip.
- Transcript turn, tool activity row, approval card, artifact link.
- File row, diff hunk/line, terminal tab, browser tab/address bar.
- Table header/row, settings row, switch, empty state, inline error.

Each interactive component needs rest, hover, focus, active, disabled and busy states as applicable. Streaming output never animates layout; reduced motion removes nonessential transitions. The interface supports both light and dark themes through semantic color tokens; the light theme is the first implementation target.

## GPUI Kit implementation

Use GPUI Kit 0.7.1 with retained entities for inputs and run/session state. The window renders a navigation shell, then feature views. Keep Git/worktree operations, agent processes, PTYs, browser embedding and persistence behind native Rust services so rendering never blocks on them. The initial native build is an interactive UI preview with illustrative runs, clearly marked as preview data. It must not imply that a sample agent is actually running. Production service wiring is a separate milestone and is described in `native/README.md`.
