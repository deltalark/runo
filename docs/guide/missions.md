---
title: Missions
keywords: mission, missions, goal, plan, planner, parts, several agents, many agents, at once, parallel, split the work, board, mission branch, orchestrate, team of agents
shortcuts: go-missions
open: missions
---
A mission is one goal that several agents work on at once. Runo plans it, runs each part in its own worktree, checks every part, and merges them into one branch you ship like any other.

**Start one.** Open **Missions** on the rail ({go-missions}). Describe the goal, pick the project, the agents that may work on it (the first one writes the plan), how many parts run at once, and whether another vendor's agent reviews each part. **Plan mission** asks the planner to read the project and split the goal into parts. It only reads; nothing changes yet.

**Check the plan.** Each part shows its title, what its agent will be told, the parts it waits for and the files it expects to touch. Click a part's agent to switch it, or the cross to leave the part out. Type what should change and **Plan again** for a new plan, or **Start mission**.

**The board.** Parts move through Waiting, Working, Checking and Done. A part starts as its own session, branched from the mission branch, once the parts it waits for are merged, so it builds on their work. Click a card to open its session and talk to its agent like any other.

**When a part is done.** Its checks run (see Proof of done); failures go back to its agent for at least two rounds. With the second opinion on, another vendor's agent reviews it and its notes go back once. Then Runo commits the part and merges it into the mission branch, one part at a time; a conflict goes back to the part's agent to resolve. A part that still fails shows **Needs you** with **Try again** and **Skip**. Only the parts that wait on it are held up.

**Shipping.** When every part is in, the checks run once more on the mission branch, in the mission's own session, and its agent fixes what broke between the parts. Then the mission is **Ready to ship**: **Review and ship** opens the mission branch in Review, where you create the pull request or merge it. **Pause** stops new parts from starting; **Delete** removes the mission and keeps its sessions.
