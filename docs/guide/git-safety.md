---
title: Clash radar, time machine and the seatbelt
keywords: clash radar, radar, overlap, conflict coming, time machine, go back, snapshot, seatbelt, secret, token, key, env, explain, error, reviewable, split, reviewer's guide, stacked
open: git-history
---
- **Clash radar**: at the top of **Branches**, Runo test-merges every session's work, your folder and the main branch in memory, and lists the pairs that change the same lines before anyone merges. Nothing in any folder is touched.
- **Time machine**: in **History**, it shows every step the folder took (commits, switches, syncs, go backs) and every snapshot taken before an agent turn, by day. Click a moment twice to go back to it. **Undo** takes that back too.
- **Seatbelt**: before a sync, a session's push or a pull request sends commits, Runo looks for passwords, tokens, key files, `.env` files and leftover clash markers, and asks before sending them. While you commit, it also points out huge files and debugging leftovers.
- **Explain**: when git stops with an error or the folder is in a puzzling state, a card at the top of the Git place says what happened in plain words, with the step that gets out of it. **What git said** shows the raw message.
- **Make it reviewable**: on a branch in **Branches**, the agent can write a reviewer's guide, or plan a split into smaller pull requests, each stacked on the one before.
