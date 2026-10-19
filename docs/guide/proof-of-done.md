---
title: Proof of done
keywords: checks, tests, run my tests, after each turn, lint, build, verify, ci, fix it, fix until green, second opinion, review, codex, verdict
---
Runo checks an agent's work after every turn that changes files, so you don't have to read every diff to know if it's any good.

**Set up checks.** Right-click a project and choose **Edit project…**. Under **Checks after each turn**, put one command per line, such as `npm test`, `npm run lint` or `cargo test`. **Suggest** fills them in from the project's `package.json`, `Cargo.toml`, `go.mod`, `pyproject.toml` or `Makefile`. A turn that changed files without any checks offers **Set up checks** under it.

**The verdict.** When a turn ends, the checks run in the session's folder, one after another, and a card under the reply says whether they passed. A failed check shows the end of its output. **Run again** re-runs them, for example after you fixed something yourself. Sessions on a connection skip the checks for now.

**Fix it.** On a red card, **Fix it** sends the failures to the agent as the next message. To have Runo do that on its own, pick how many rounds under **Fix failures on their own** in the project's popup. It stops when the checks pass, when the rounds run out, or when the same failure comes back twice.

**Second opinion.** **Second opinion** asks an agent from another vendor (Codex for a Claude Code session, Claude Code for the others, otherwise any other installed agent) to review the turn's diff without editing anything. Its comments land as diff notes on the Changes panel, named after the agent. Delete the ones you disagree with and send the rest with **Send notes**.
