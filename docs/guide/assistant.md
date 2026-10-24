---
title: The Runo assistant
keywords: assistant, avatar, ask runo, what can the assistant do, help, runo, what needs me, do it for me, slash commands, mention, nudge, remember, chat history, briefing, command deck
shortcuts: ask-runo
open: settings:general
---
The Runo mark in the corner answers questions about Runo and about what is happening in it. Press {ask-runo} or click it, then ask how to do something, ask what needs you, or tell it what to do.

It knows all of Runo: every menu, command and shortcut, and every setting with what it is set to now, read from the app itself, so it always matches the version you have. It also knows your projects, sessions and how each is doing, permission requests and questions waiting on you, your playbooks and the clash radar. It can offer to:

- start an agent on a task, or race several on it, each in its own worktree
- run a playbook
- send a message to a session, or stop its agent
- allow or deny a permission request once
- open a session, switch project or go to any place in the app
- run any command in Quick open, such as quitting Runo or opening the Terminal
- change any switch or choice in Settings, or press a button there such as **Check now** for updates

Each offer is a card that says exactly what it will do; nothing happens until you click **Run** or **Go**, and **×** dismisses it. Follow-ups like "do that" or "and the other one?" work, as it remembers the conversation until you start a new chat.

Answers stream in as they are written; **Stop** keeps what is there so far.

Type `/` for shortcuts that answer straight away, without an agent:

- `/status`: what's running, what's waiting on you and what finished since you last looked
- `/new <task>` and `/race <task>`: a card to start an agent, or race your agents, on the task
- `/playbook <name>`: a card to run a playbook
- `/stop`: a card for each running agent
- `/approve`: a card for each permission request waiting on you
- `/remember <something>` keeps a preference for later chats in this project, `/remember` alone lists what it keeps, and `/forget` drops one
- `/clear` starts a new chat, and `/help` lists these

Type `@` to name a session or project; the assistant gets that session's recent conversation with your question, so "what did @fix-login do?" works.

While the assistant is closed, the mark speaks up when something is ready for you: a run you haven't looked at finished, a race is done and ready to compare, or the clash radar found two lines of work changing the same lines. Each nudge has one button for the next step; **Later** hides it until something new happens. Turn nudges off with **Nudges** in Settings.

Choose which agent answers, and its model, in **Settings → General → Runo assistant** (**Assistant agent** and **Assistant model**). The agent starts out the same as your default agent for new sessions, and the model starts on a fast default (Haiku on Claude). The line under the input shows which one is answering.

A new chat opens on a briefing: what needs you, what's running, what's ready to review, an estimate of today's spend, the clash radar and what it remembers. Click a tile to go there. The expand button in the header turns the panel into the command deck, a bigger view in the middle of the window; click outside it to put it back in the corner.

Chats are kept between restarts. Today's chat reopens where you left it, and a new day starts on the briefing. The clock button in the header lists earlier chats to pick up or delete, and everything it remembers, each with **×** to forget it. When you say something like "always use worktrees for Codex", it can offer a card to remember that, and it follows what it remembers in every chat in that project.

Turn it on or off in **Settings → General → Runo assistant**. With **Guide only** it answers from this guide without an agent and can't see or do anything in the app.
