---
title: Isolated worktrees
keywords: worktree, isolated, branch, copy, collide, parallel, setup command, worktreeinclude, env
open: settings:worktrees
---
A session can run in its own git branch and folder under `.runo-worktrees/`, so agents working at the same time never overwrite each other.

- A setup command (such as `npm install`) runs in each new worktree before the agent starts. Set it in **Settings → General** or in a project's Edit dialog.
- Git-ignored files listed in `.worktreeinclude`, such as `.env`, are copied in.
- **Go to Isolated sessions** in quick open lists every session that has its own worktree.
