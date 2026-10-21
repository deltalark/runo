# Missions (backlog)

Not in the app. Missions were built, then taken out before the first release so Runo ships uncluttered. This is the design to come back to. The full implementation (`mission.rs`, `mission_ui.rs`, `mission_view.rs` and its hooks in proof, review, ship and turns) is in the history at commit e9c53f2.

When it returns, the plan is to make it part of Crew rather than its own page: Crew proposes a mission for a goal too big for one thread, its parts run as Crew threads, and the plan and progress show as a card in Crew's chat.

## How it worked

A mission is one goal that several agents work on at once. Runo plans it, runs each part in its own worktree, checks every part, and merges them into one branch you ship like any other.

**Start one.** Open **Missions** on the rail ({go-missions}). Describe the goal, pick the project, the agents that may work on it (the first one writes the plan), how many parts run at once, and whether another vendor's agent reviews each part. **Plan mission** asks the planner to read the project and split the goal into parts. It only reads; nothing changes yet.

**Check the plan.** Each part shows its title, what its agent will be told, the parts it waits for and the files it expects to touch. Click a part's agent to switch it, or the cross to leave the part out. Type what should change and **Plan again** for a new plan, or **Start mission**.

**The board.** Parts are laid out in waves: a wave starts once the waves before it are merged, and every part in a wave runs at once, each as its own session branched from the mission branch, so it builds on their work. Each card shows its agent, where it is in Work, Checks, Review and Merge, and what its agent is doing right now. The activity beside the board lists what happened: parts starting, checks passing or going back for a fix, second opinions, conflicts and merges; hide it with the activity button in the header. Click a card to open its session and talk to its agent like any other; **Mission board** at the top of that session brings you back.

**When a part is done.** Its checks run (see Proof of done); failures go back to its agent for at least two rounds. With the second opinion on, another vendor's agent reviews it and its notes go back once. Then Runo commits the part and merges it into the mission branch, one part at a time; a conflict goes back to the part's agent to resolve. A part that still fails shows **Needs you** with **Try again** and **Skip**. Only the parts that wait on it are held up.

**Shipping.** When every part is in, the checks run once more on the mission branch, in the mission's own session, and its agent fixes what broke between the parts. Then the mission is **Ready to ship**: **Review and ship** opens the mission branch in Review, where you create the pull request or merge it. **Pause** stops new parts from starting; **Delete** asks once more, then removes the mission and keeps its sessions. On the New mission page, Enter plans the goal (Shift+Enter for a new line), and Enter under a plan plans it again with what you typed.
