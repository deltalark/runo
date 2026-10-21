---
title: Cove VMs
keywords: cove, vm, virtual machine, sandbox, isolate, linux, vfkit, safe, untrusted, install cove
open: settings:connections
---
Cove runs a session in its own Linux VM, away from your files and keys, on this Mac or on your own server.

1. **Settings → Connections → Cove**: **Install** sets it up in one step and starts it at login.
2. Pick **Cove VM** under **Runs on** when you start a session. **Make this the project's default** runs the project's next sessions there, Crew's threads included.
3. With **Let agents use Cove** on, agents can also start scratch VMs to run commands, try untrusted code or run heavy jobs.

To use a Cove server elsewhere, run `cove pair` on it and add it under **Run Cove on a server**.
