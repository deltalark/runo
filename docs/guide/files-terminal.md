---
title: Files and Terminal
keywords: files, editor, terminal, shell, pty, quick commands, save file, run command
shortcuts: save-file, toggle-terminal, new-terminal, toggle-term-size, focus-files, search-files
---
**Files** lets you browse and edit the worktree, with syntax highlighting, rendered Markdown, images and CSV tables. {focus-files} opens the tab, {search-files} focuses the search box, and {save-file} saves.

**Terminal** is a real shell in the session's folder, with several tabs. {toggle-terminal} shows it and focuses it, or hides the panel when the terminal already has focus. {new-terminal} opens another shell tab; {toggle-term-size} expands or shrinks the panel. Drag its top edge to make it taller. On Windows a new tab starts PowerShell with `-NoProfile`, so a profile that hangs cannot leave the tab blank, and profile customizations are not loaded.

**Run** lists your quick commands (Settings → Terminal → Quick commands). Each one runs in its own tab and shows whether it passed and how long it took. Running it again reuses that tab. When a run fails, **Ask agent to fix** sends the end of its output to the session's agent.

**Output** appears when Runo itself has printed something for the session, such as a worktree setup or a push.
