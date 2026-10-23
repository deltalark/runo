---
title: Cove VMs
keywords: cove, vm, sign in, login, not logged in, virtual machine, sandbox, isolate, linux, vfkit, safe, untrusted, install cove
open: settings:connections
---
Cove runs a session in its own Linux VM, away from your files and keys, on this Mac or on your own server.

1. **Settings → Connections → Cove**: **Install** sets it up in one step and starts it at login.
2. Pick **Cove VM** under **Runs on** when you start a session. **Make this the project's default** runs the project's next sessions there, Crew's threads included.
3. With **Let agents use Cove** on, agents can also start scratch VMs to run commands, try untrusted code or run heavy jobs.

To use a Cove server elsewhere, run `cove pair` on it and add it under **Run Cove on a server**.

**Signing agents in.** A VM doesn't have your Mac's logins (Claude Code keeps its login in the Mac's keychain). Sign in once per Cove: under the connection in **Settings → Connections**, press **Sign in** next to the agent, or **Sign in on …** on a turn that failed because it wasn't signed in. The agent's own login runs in a terminal in a small VM, with a link you can open on any device. Cove then keeps that login for every VM on that server, and saves the tokens agents refresh. **Sign out** removes it from the server and its VMs. This works for Claude Code, Codex, Gemini, OpenCode and Qwen Code, and needs Cove with `cove agents` on both this Mac and the server.
