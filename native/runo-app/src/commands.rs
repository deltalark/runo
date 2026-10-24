//! Runo's commands: the rows of Quick open's Commands group. Quick open lists them, the
//! assistant tells its agent about them and can run any of them, and the guide's checks
//! make sure each one is written up, so all three always name the same things.

use super::*;

/// What a command does when it runs.
#[derive(Clone, Copy)]
pub(crate) enum Run {
    /// Dispatch an action, as its menu item or shortcut does.
    Action(fn() -> Box<dyn gpui_kit::Action>),
    /// Go to a place.
    View(View),
    Tour,
    Arcade,
    AddProject,
}

pub(crate) struct AppCommand {
    /// How the assistant names it in an `ACTION:` line.
    pub id: &'static str,
    pub label: &'static str,
    pub icon: IconName,
    pub keywords: &'static [&'static str],
    /// The guide article that explains it.
    pub guide: &'static str,
    pub run: Run,
}

const fn act(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    guide: &'static str,
    run: fn() -> Box<dyn gpui_kit::Action>,
) -> AppCommand {
    AppCommand {
        id,
        label,
        icon,
        keywords: &[],
        guide,
        run: Run::Action(run),
    }
}

const fn go(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    guide: &'static str,
    view: View,
) -> AppCommand {
    AppCommand {
        id,
        label,
        icon,
        keywords: &[],
        guide,
        run: Run::View(view),
    }
}

/// Every command, in the order Quick open lists them.
pub(crate) const COMMANDS: &[AppCommand] = &[
    act(
        "new-session",
        "New session",
        IconName::Plus,
        "new-session",
        || Box::new(NewSession),
    ),
    act(
        "settings",
        "Open settings",
        IconName::Settings,
        "settings",
        || Box::new(OpenSettings),
    ),
    act(
        "toggle-view",
        "Switch between Chat and CLI",
        IconName::SquareTerminal,
        "chat-cli",
        || Box::new(ToggleView),
    ),
    act(
        "panel",
        "Toggle the Changes panel",
        IconName::PanelRightOpen,
        "changes-review",
        || Box::new(TogglePanel),
    ),
    act(
        "toggle-terminal",
        "Toggle the Terminal",
        IconName::SquareTerminal,
        "files-terminal",
        || Box::new(ToggleTerminal),
    ),
    act(
        "new-terminal",
        "New terminal",
        IconName::SquareTerminal,
        "files-terminal",
        || Box::new(NewTerminal),
    ),
    act(
        "toggle-term-size",
        "Expand or shrink the Terminal",
        IconName::ChevronsUpDown,
        "files-terminal",
        || Box::new(ToggleTermSize),
    ),
    act(
        "focus-changes",
        "Show Changes",
        IconName::GitCompare,
        "changes-review",
        || Box::new(FocusChanges),
    ),
    act(
        "focus-files",
        "Show Files",
        IconName::FileText,
        "files-terminal",
        || Box::new(FocusFiles),
    ),
    act(
        "search-files",
        "Search in files",
        IconName::Search,
        "files-terminal",
        || Box::new(SearchFiles),
    ),
    act(
        "open-browser",
        "Open Browser",
        IconName::Globe,
        "browser",
        || Box::new(OpenBrowser),
    ),
    act(
        "sidebar",
        "Toggle the sidebar",
        IconName::PanelLeft,
        "navigation",
        || Box::new(ToggleSidebar),
    ),
    act(
        "stop",
        "Stop the running agent",
        IconName::CircleStop,
        "navigation",
        || Box::new(StopAgent),
    ),
    act("close-tab", "Close tab", IconName::X, "tabs", || {
        Box::new(CloseTab)
    }),
    act(
        "archive",
        "Archive session",
        IconName::Archive,
        "navigation",
        || Box::new(ArchiveSession),
    ),
    act("quit", "Quit Runo", IconName::LogOut, "quit-update", || {
        Box::new(Quit)
    }),
    go(
        "go-home",
        "Go to Home",
        IconName::House,
        "navigation",
        View::Home,
    ),
    go(
        "go-inbox",
        "Go to Inbox",
        IconName::Inbox,
        "approvals",
        View::Inbox,
    ),
    go(
        "go-sessions",
        "Go to Sessions",
        IconName::Layers,
        "navigation",
        View::AllSessions,
    ),
    go(
        "go-isolated",
        "Go to Isolated sessions",
        IconName::GitBranch,
        "worktrees",
        View::Worktrees,
    ),
    go(
        "go-history",
        "Go to Agent history",
        IconName::RotateCcwClock,
        "history",
        View::History,
    ),
    go(
        "go-review",
        "Go to Git: Changes",
        IconName::GitCompare,
        "git",
        View::Review,
    ),
    go(
        "go-git-history",
        "Go to Git: History",
        IconName::GitCommitHorizontal,
        "git",
        View::GitHistory,
    ),
    go(
        "go-branches",
        "Go to Git: Branches",
        IconName::GitBranch,
        "git",
        View::Branches,
    ),
    go(
        "go-reviews",
        "Go to Git: GitHub",
        IconName::GitPullRequest,
        "github",
        View::GitHub,
    ),
    go(
        "go-playbooks",
        "Go to Playbooks",
        IconName::BookOpen,
        "playbooks",
        View::Automations,
    ),
    go(
        "go-timeline",
        "Go to Timeline",
        IconName::Activity,
        "timeline",
        View::Activity,
    ),
    go(
        "go-design",
        "Go to Design",
        IconName::PanelsTopLeft,
        "design",
        View::Design,
    ),
    go(
        "go-architecture",
        "Go to Architecture",
        IconName::Network,
        "architecture",
        View::Architecture,
    ),
    go(
        "go-data",
        "Go to Data",
        IconName::Database,
        "data",
        View::Data,
    ),
    go(
        "go-notes",
        "Go to Project notes",
        IconName::NotebookPen,
        "notes",
        View::Notes,
    ),
    AppCommand {
        id: "add-project",
        label: "Add a project folder…",
        icon: IconName::FolderPlus,
        keywords: &["new project", "open folder", "clone repository"],
        guide: "projects",
        run: Run::AddProject,
    },
    AppCommand {
        id: "tour",
        label: "Take the tour",
        icon: IconName::Sparkles,
        keywords: &["welcome", "help", "guide", "onboarding"],
        guide: "tour",
        run: Run::Tour,
    },
    AppCommand {
        id: "arcade",
        label: "Open the Arcade",
        icon: IconName::Gamepad2,
        keywords: &["game", "play", "break", "snake", "2048", "flappy", "spark"],
        guide: "arcade",
        run: Run::Arcade,
    },
];

pub(crate) fn find(id: &str) -> Option<&'static AppCommand> {
    let id = id.trim().to_lowercase();
    COMMANDS
        .iter()
        .find(|c| c.id == id || c.label.to_lowercase() == id)
}

impl AppCommand {
    /// What Quick open does with the row: an action runs by itself, the rest jump.
    pub(crate) fn jump(&self) -> Jump {
        match self.run {
            Run::Action(_) => Jump::Action,
            Run::View(view) => Jump::View(view),
            Run::Tour => Jump::Tour,
            Run::Arcade => Jump::Arcade,
            Run::AddProject => Jump::AddProject,
        }
    }

    /// The keys that run it now, or "" when none do.
    pub(crate) fn keys(&self, cx: &App) -> String {
        match self.run {
            Run::Action(action) => keymap::label_for(cx, &*action()),
            Run::View(view) => match view {
                View::Home => keymap::label(cx, "go-home"),
                View::AllSessions => keymap::label(cx, "go-sessions"),
                View::Review => keymap::label(cx, "go-review"),
                View::GitHub => keymap::label(cx, "go-reviews"),
                View::Automations => keymap::label(cx, "go-playbooks"),
                View::History => keymap::label(cx, "go-history"),
                View::Design => keymap::label(cx, "go-design"),
                View::Architecture => keymap::label(cx, "go-architecture"),
                View::Data => keymap::label(cx, "go-data"),
                View::Notes => keymap::label(cx, "go-notes"),
                View::Inbox => keymap::label(cx, "go-inbox"),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }
}
