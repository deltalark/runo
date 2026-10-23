//! Finding sessions: the search, filters and sort on the Sessions page, matches in past
//! conversations for it and for quick open, and the base branch and branch name of a new
//! session.

use super::*;
use std::sync::Arc;

/// Which archived sessions the Sessions page lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Archived {
    #[default]
    Include,
    Hide,
    Only,
}

impl Archived {
    const ALL: [Self; 3] = [Self::Include, Self::Hide, Self::Only];

    fn label(self) -> &'static str {
        match self {
            Self::Include => "Live and archived",
            Self::Hide => "Live only",
            Self::Only => "Archived only",
        }
    }
}

/// The order of the Sessions page. Pinned sessions stay first and archived ones last.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Sort {
    /// Latest activity first.
    #[default]
    Recent,
    /// Newest first.
    Created,
    /// By title, A to Z.
    Name,
}

impl Sort {
    const ALL: [Self; 3] = [Self::Recent, Self::Created, Self::Name];

    fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Created => "Created",
            Self::Name => "Name",
        }
    }
}

/// The Sessions page's filters and sort.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct ListFilter {
    pub project: Option<u64>,
    pub agent: Option<String>,
    pub status: Option<Status>,
    pub archived: Archived,
    pub sort: Sort,
}

impl ListFilter {
    fn is_default(&self) -> bool {
        self.project.is_none()
            && self.agent.is_none()
            && self.status.is_none()
            && self.archived == Archived::Include
    }
}

const STATUSES: [(Status, &str); 5] = [
    (Status::Working, "Working"),
    (Status::Done, "Done"),
    (Status::Failed, "Failed"),
    (Status::Stopped, "Stopped"),
    (Status::Idle, "Idle"),
];

/// Sessions whose conversation mentions a search, with a snippet around the first mention.
#[derive(Default)]
pub(crate) struct Hits {
    /// The search these hits answer.
    pub query: String,
    pub found: Vec<(u64, String)>,
    /// Bumped by each search, so an older one finishing late is dropped.
    asked: u64,
}

impl Hits {
    /// The snippet for `id`, when these hits answer `query`.
    pub fn snippet(&self, query: &str, id: u64) -> Option<&str> {
        (self.query == query)
            .then(|| self.found.iter().find(|(s, _)| *s == id))
            .flatten()
            .map(|(_, snippet)| snippet.as_str())
    }
}

/// A project, its default branch and its branches (`true` when only on the remote).
pub(crate) type Branches = (u64, String, Vec<(String, bool)>);

/// Applies a filter menu's pick.
type Pick = Box<dyn Fn(&mut ListFilter, usize)>;

/// Every session's messages, shared with background searches.
type Corpus = Arc<Vec<(u64, Vec<String>)>>;

/// State behind finding sessions.
pub(crate) struct Finder {
    pub query: Entity<InputState>,
    pub filter: ListFilter,
    pub list_hits: Hits,
    pub palette_hits: Hits,
    pub home_hits: Hits,
    /// The messages as of the last search, and what they looked like then.
    corpus: Option<(u64, Corpus)>,
}

impl Finder {
    pub fn new(window: &mut Window, cx: &mut Context<RunoApp>) -> Self {
        Self {
            query: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Search titles, branches and conversations")
            }),
            filter: ListFilter::default(),
            list_hits: Hits::default(),
            palette_hits: Hits::default(),
            home_hits: Hits::default(),
            corpus: None,
        }
    }
}

/// Where a search's hits go.
#[derive(Clone, Copy)]
pub(crate) enum HitsFor {
    List,
    Palette,
    Home,
}

/// Sessions to list on the Sessions page (or only isolated ones), filtered and in order.
pub(crate) fn filtered<'a>(
    state: &'a State,
    filter: &ListFilter,
    query: &str,
    hits: &Hits,
    isolated_only: bool,
) -> Vec<&'a Session> {
    let needle = query.trim().to_lowercase();
    let project_name = |s: &Session| {
        state
            .projects
            .iter()
            .find(|p| p.id == s.project_id)
            .map(|p| p.name.as_str())
            .unwrap_or("")
    };
    let mut list: Vec<&Session> = state
        .sessions
        .iter()
        // Chats asked from the Design or Architecture canvas live on that canvas.
        .filter(|s| s.canvas.is_empty())
        .filter(|s| !isolated_only || s.isolated)
        .filter(|s| filter.project.is_none_or(|p| s.project_id == p))
        .filter(|s| filter.agent.as_ref().is_none_or(|a| &s.provider == a))
        .filter(|s| filter.status.is_none_or(|st| s.status == st))
        .filter(|s| match filter.archived {
            Archived::Include => true,
            Archived::Hide => !s.archived,
            Archived::Only => s.archived,
        })
        .filter(|s| {
            needle.is_empty()
                || [
                    s.title.as_str(),
                    s.branch.as_str(),
                    project_name(s),
                    agent::provider(&s.provider).name,
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
                || hits.snippet(query.trim(), s.id).is_some()
        })
        .collect();
    // The latest activity of each session: its newest entry in the activity log.
    let last: HashMap<u64, usize> = state
        .activity
        .iter()
        .enumerate()
        .map(|(i, a)| (a.session, i))
        .collect();
    let group = |s: &Session| (s.archived, !s.pinned);
    match filter.sort {
        Sort::Recent => list.sort_by_key(|s| {
            (
                group(s),
                std::cmp::Reverse((s.status == Status::Working, last.get(&s.id), s.id)),
            )
        }),
        Sort::Created => list.sort_by_key(|s| (group(s), std::cmp::Reverse(s.id))),
        Sort::Name => list.sort_by_cached_key(|s| (group(s), s.title.to_lowercase(), s.id)),
    }
    list
}

/// Sessions whose messages contain `query` (ignoring case), each with a one-line snippet
/// around its first mention, in corpus order and at most `limit`.
pub(crate) fn search_corpus(
    corpus: &[(u64, Vec<String>)],
    query: &str,
    limit: usize,
) -> Vec<(u64, String)> {
    let needle = query.trim().to_lowercase();
    if needle.chars().count() < 2 {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (id, texts) in corpus {
        if let Some(snippet) = texts.iter().find_map(|t| snippet(t, &needle)) {
            found.push((*id, snippet));
            if found.len() >= limit {
                break;
            }
        }
    }
    found
}

/// A short window of `text` around the first place it contains `needle` (lowercase).
fn snippet(text: &str, needle: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(needle)?;
    // Lowercasing can change byte lengths outside ASCII; cut from the lowercase copy then.
    let source = if lower.len() == text.len() && text.is_char_boundary(at) {
        text
    } else {
        lower.as_str()
    };
    let mut start = at.saturating_sub(40);
    while !source.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + needle.len() + 80).min(source.len());
    while !source.is_char_boundary(end) {
        end += 1;
    }
    let body = source[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!(
        "{}{body}{}",
        if start > 0 { "…" } else { "" },
        if end < source.len() { "…" } else { "" }
    ))
}

/// A cheap fingerprint of every session's messages, to tell when the corpus is stale.
fn corpus_key(state: &State) -> u64 {
    state
        .sessions
        .iter()
        .fold(state.sessions.len() as u64, |key, s| {
            let tail = s.messages.last().map(|m| m.text.len()).unwrap_or(0);
            key.wrapping_mul(31)
                .wrapping_add(s.id ^ (s.messages.len() as u64) << 20 ^ tail as u64)
        })
}

impl RunoApp {
    /// Look through past conversations for `query` off the UI thread; the hits land in the
    /// Sessions page's, quick open's or Home's results.
    pub(crate) fn search_transcripts(&mut self, query: String, into: HitsFor) {
        let query = query.trim().to_string();
        let key = corpus_key(&self.state);
        let corpus = match &self.finder.corpus {
            Some((k, corpus)) if *k == key => corpus.clone(),
            _ => {
                let corpus: Corpus = Arc::new(
                    self.state
                        .sessions
                        .iter()
                        .map(|s| (s.id, s.messages.iter().map(|m| m.text.clone()).collect()))
                        .collect(),
                );
                self.finder.corpus = Some((key, corpus.clone()));
                corpus
            }
        };
        let hits = match into {
            HitsFor::List => &mut self.finder.list_hits,
            HitsFor::Palette => &mut self.finder.palette_hits,
            HitsFor::Home => &mut self.finder.home_hits,
        };
        hits.asked += 1;
        let asked = hits.asked;
        if query.chars().count() < 2 {
            hits.query = query;
            hits.found.clear();
            return;
        }
        self.bg(
            move || {
                let found = search_corpus(&corpus, &query, 200);
                (query, found)
            },
            move |this, (query, found), cx| {
                let hits = match into {
                    HitsFor::List => &mut this.finder.list_hits,
                    HitsFor::Palette => &mut this.finder.palette_hits,
                    HitsFor::Home => &mut this.finder.home_hits,
                };
                if hits.asked == asked {
                    hits.query = query;
                    hits.found = found;
                    cx.notify();
                }
            },
        );
    }

    /// The Sessions page's search box and filter menus.
    pub(crate) fn list_toolbar(&self, isolated_only: bool, cx: &mut Context<Self>) -> Div {
        let f = &self.finder.filter;
        let project = f
            .project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "All projects".into());
        let agent = f
            .agent
            .as_deref()
            .map(|a| agent::provider(a).name.to_string())
            .unwrap_or_else(|| "All agents".into());
        let status = STATUSES
            .iter()
            .find(|(s, _)| Some(*s) == f.status)
            .map(|(_, l)| l.to_string())
            .unwrap_or_else(|| "Any status".into());
        let mut menus = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(self.filter_chip("sl-project", project, f.project.is_some(), cx))
            .child(self.filter_chip("sl-agent", agent, f.agent.is_some(), cx))
            .child(self.filter_chip("sl-status", status, f.status.is_some(), cx));
        if !isolated_only {
            menus = menus.child(self.filter_chip(
                "sl-archived",
                f.archived.label().to_string(),
                f.archived != Archived::Include,
                cx,
            ));
        }
        menus = menus.child(self.filter_chip(
            "sl-sort",
            format!("Sort: {}", f.sort.label()),
            false,
            cx,
        ));
        if !f.is_default() {
            menus = menus.child(
                div()
                    .id("sl-clear")
                    .text_xs()
                    .text_color(rgb(c_blue()))
                    .cursor_pointer()
                    .child("Clear filters")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let sort = this.finder.filter.sort;
                        this.finder.filter = ListFilter {
                            sort,
                            ..Default::default()
                        };
                        cx.notify();
                    })),
            );
        }
        div()
            .v_flex()
            .gap_2()
            .child(
                Input::new(&self.finder.query)
                    .bordered(true)
                    .prefix(icon(IconName::Search, 13., c_muted()))
                    .h(px(34.)),
            )
            .child(menus)
    }

    /// A filter's chip, with its menu under it while open.
    fn filter_chip(
        &self,
        key: &'static str,
        label: String,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.menu.as_deref() == Some(key);
        div()
            .relative()
            .child(
                self.chip((key, 0), label, active || open)
                    .gap_1()
                    .child(icon(IconName::ChevronDown, 12., c_muted()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = if this.menu.as_deref() == Some(key) {
                            None
                        } else {
                            Some(key.to_string())
                        };
                        cx.notify();
                    })),
            )
            .when(open, |el| el.child(self.filter_menu(key, cx)))
            .into_any_element()
    }

    fn filter_menu(&self, key: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let f = &self.finder.filter;
        let rows: Vec<(String, bool)>;
        let pick: Pick;
        match key {
            "sl-project" => {
                let ids: Vec<Option<u64>> = std::iter::once(None)
                    .chain(self.state.projects.iter().map(|p| Some(p.id)))
                    .collect();
                rows = ids
                    .iter()
                    .map(|id| {
                        let name = id
                            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| "All projects".into());
                        (name, *id == f.project)
                    })
                    .collect();
                pick = Box::new(move |f, i| f.project = ids[i]);
            }
            "sl-agent" => {
                // Agents that have sessions, in the usual agent order.
                let ids: Vec<Option<String>> = std::iter::once(None)
                    .chain(
                        agent::PROVIDERS
                            .iter()
                            .filter(|p| self.state.sessions.iter().any(|s| s.provider == p.id))
                            .map(|p| Some(p.id.to_string())),
                    )
                    .collect();
                rows = ids
                    .iter()
                    .map(|id| {
                        let name = id
                            .as_deref()
                            .map(|a| agent::provider(a).name.to_string())
                            .unwrap_or_else(|| "All agents".into());
                        (name, *id == f.agent)
                    })
                    .collect();
                pick = Box::new(move |f, i| f.agent = ids[i].clone());
            }
            "sl-status" => {
                let ids: Vec<Option<Status>> = std::iter::once(None)
                    .chain(STATUSES.iter().map(|(s, _)| Some(*s)))
                    .collect();
                rows = std::iter::once("Any status")
                    .chain(STATUSES.iter().map(|(_, l)| *l))
                    .zip(&ids)
                    .map(|(l, id)| (l.to_string(), *id == f.status))
                    .collect();
                pick = Box::new(move |f, i| f.status = ids[i]);
            }
            "sl-archived" => {
                rows = Archived::ALL
                    .iter()
                    .map(|a| (a.label().to_string(), *a == f.archived))
                    .collect();
                pick = Box::new(|f, i| f.archived = Archived::ALL[i]);
            }
            _ => {
                rows = Sort::ALL
                    .iter()
                    .map(|s| (s.label().to_string(), *s == f.sort))
                    .collect();
                pick = Box::new(|f, i| f.sort = Sort::ALL[i]);
            }
        }
        let pick = std::rc::Rc::new(pick);
        let mut list = div().v_flex().p_1().min_w(px(200.));
        for (i, (label, on)) in rows.into_iter().enumerate() {
            let pick = pick.clone();
            list = list.child(menu_row((key, i), label, String::new(), on, None).on_click(
                cx.listener(move |this, _, _, cx| {
                    pick(&mut this.finder.filter, i);
                    this.menu = None;
                    cx.notify();
                }),
            ));
        }
        drop_menu(key, list, cx)
    }

    /// Quick open's sessions, archived ones included and marked, then sessions whose
    /// conversation mentions what was typed.
    pub(crate) fn palette_sessions(&self) -> Vec<(CommandGroup, Vec<Jump>)> {
        let project_name = |pid: u64| {
            self.state
                .projects
                .iter()
                .find(|p| p.id == pid)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        };
        let mut sessions = CommandGroup::new().label("Sessions");
        let mut session_jumps = Vec::new();
        let mut ordered: Vec<&Session> = self.state.sessions.iter().collect();
        ordered.sort_by_key(|s| s.archived);
        for session in ordered {
            let project = project_name(session.project_id);
            let label = if session.archived {
                format!("{}  ·  {project}  ·  archived", session.title)
            } else {
                format!("{}  ·  {project}", session.title)
            };
            sessions = sessions.item(
                CommandItem::new()
                    .label(label)
                    .icon(if session.archived {
                        IconName::Archive
                    } else {
                        IconName::MessageSquare
                    })
                    .keywords([session.branch.clone(), session.provider.clone()]),
            );
            session_jumps.push(Jump::Session(session.id));
        }

        let hits = &self.finder.palette_hits;
        let mut said = CommandGroup::new().label("In conversations");
        let mut said_jumps = Vec::new();
        for (id, snippet) in hits.found.iter().take(50) {
            let Some(session) = self.state.session(*id) else {
                continue;
            };
            said = said.item(
                CommandItem::new()
                    .label(format!("{}  ·  {snippet}", session.title))
                    .icon(IconName::Search)
                    // Shown for the search that found it, whatever its label says.
                    .keywords([hits.query.clone()]),
            );
            said_jumps.push(Jump::Session(*id));
        }
        vec![(sessions, session_jumps), (said, said_jumps)]
    }

    /// Load the branches of the New session form's project in the background, once per
    /// project, for the base branch menu.
    pub(crate) fn sync_new_branches(&mut self) {
        if self.view != View::NewSession {
            return;
        }
        let Some(project) = self
            .new_project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
        else {
            return;
        };
        if self
            .new_branches
            .as_ref()
            .is_some_and(|(id, ..)| *id == project.id)
        {
            return;
        }
        let (pid, path) = (project.id, project.path.clone());
        self.new_branches = Some((pid, String::new(), Vec::new()));
        self.new_base = None;
        self.bg(
            move || {
                let default = git::default_branch(&path);
                let mut list = git::branches(&path);
                // The default branch leads the list.
                if let Some(pos) = list.iter().position(|(n, _)| *n == default) {
                    let entry = list.remove(pos);
                    list.insert(0, entry);
                }
                (default, list)
            },
            move |this, (default, list), cx| {
                if this
                    .new_branches
                    .as_ref()
                    .is_some_and(|(id, ..)| *id == pid)
                {
                    this.new_branches = Some((pid, default, list));
                    cx.notify();
                }
            },
        );
    }

    /// The branch a new isolated session gets when none was typed: a slug of its task,
    /// else `fallback`, with a number added when that name is in use.
    pub(crate) fn task_branch(
        &self,
        project: &Project,
        task: &str,
        fallback: String,
        has_branch: impl Fn(&str) -> bool,
    ) -> String {
        let wanted = Some(git::branch_slug(task))
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback);
        git::unique_branch(&wanted, |name| {
            has_branch(name)
                || git::worktree_path(&project.path, name).exists()
                || self
                    .state
                    .sessions
                    .iter()
                    .any(|s| s.project_id == project.id && s.branch == name)
        })
    }

    /// Whether the New session form's project has a local branch `name`, from its listing.
    pub(crate) fn listed_branch(&self, name: &str) -> bool {
        self.new_branches
            .as_ref()
            .is_some_and(|(_, _, list)| list.iter().any(|(n, remote)| !remote && n == name))
    }

    /// The default branch of the New session form's project, once listed.
    pub(crate) fn listed_default(&self) -> &str {
        self.new_branches
            .as_ref()
            .map(|(_, default, _)| default.as_str())
            .unwrap_or("")
    }

    /// The New session form's base branch chip and its menu.
    pub(crate) fn base_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let key = "new-base";
        let default = self.listed_default();
        let open = self.menu.as_deref() == Some(key);
        let shown = self.new_base.as_deref().unwrap_or(default);
        let mut el = div().relative().flex_none().child(
            self.chip(
                (key, 0),
                if shown.is_empty() {
                    "From the current branch".to_string()
                } else {
                    format!("From {shown}")
                },
                self.new_base.is_some() || open,
            )
            .h(px(36.))
            .gap_1()
            .child(icon(IconName::GitBranch, 13., c_muted()))
            .child(icon(IconName::ChevronDown, 12., c_muted()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.menu = if this.menu.as_deref() == Some(key) {
                    None
                } else {
                    Some(key.to_string())
                };
                cx.notify();
            })),
        );
        if open {
            let list_of = self
                .new_branches
                .as_ref()
                .map(|(_, _, l)| l.clone())
                .unwrap_or_default();
            let mut list = div().v_flex().p_1().min_w(px(240.));
            if list_of.is_empty() {
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(rgb(c_muted()))
                        .child("Loading branches…"),
                );
            }
            for (i, (name, remote)) in list_of.into_iter().enumerate() {
                let picked = name == shown;
                let is_default = name == default;
                let desc = match (is_default, remote) {
                    (true, _) => "Default branch",
                    (false, true) => "On the remote only",
                    _ => "",
                };
                list = list.child(
                    menu_row((key, i), name.clone(), desc.to_string(), picked, None).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.new_base = (!is_default).then(|| name.clone());
                            this.menu = None;
                            cx.notify();
                        }),
                    ),
                );
            }
            el = el.child(drop_menu(key, list, cx));
        }
        el.into_any_element()
    }
}

/// A menu opening downward from the chip above it, closed by clicking elsewhere.
pub(crate) fn drop_menu(
    key: &str,
    list: impl IntoElement,
    cx: &mut Context<RunoApp>,
) -> AnyElement {
    div()
        .absolute()
        .top(px(34.))
        .left_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            }))
                            .child(menu_box(key, list)),
                    ),
            )
            .with_priority(2),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn session(id: u64, project: u64, title: &str, provider: &str) -> Session {
        Session {
            id,
            project_id: project,
            title: title.into(),
            provider: provider.into(),
            branch: format!("runo/session-{id}"),
            ..Default::default()
        }
    }

    fn state() -> State {
        let mut state = State::default();
        for (id, name) in [(1, "web"), (2, "api")] {
            state.projects.push(Project {
                id,
                name: name.into(),
                path: format!("/{name}"),
                ..Default::default()
            });
        }
        state.sessions = vec![
            session(10, 1, "Fix login", "claude"),
            session(11, 2, "Add metrics", "codex"),
            session(12, 1, "Bump deps", "claude"),
            session(13, 2, "Old idea", "gemini"),
        ];
        state.sessions[1].status = Status::Failed;
        state.sessions[3].archived = true;
        state.sessions[2].pinned = true;
        state.sessions[0].messages = vec![
            Message::new(Role::User, "The redirect after LOGIN goes to /home"),
            Message::new(Role::Agent, "Fixed the redirect in auth.rs"),
        ];
        state
    }

    fn ids(list: Vec<&Session>) -> Vec<u64> {
        list.iter().map(|s| s.id).collect()
    }

    #[test]
    fn filters_narrow_the_list() {
        let state = state();
        let none = Hits::default();
        let all = ListFilter::default();
        // Pinned first, archived last.
        assert_eq!(
            ids(filtered(&state, &all, "", &none, false)),
            [12, 11, 10, 13]
        );
        let web = ListFilter {
            project: Some(1),
            ..Default::default()
        };
        assert_eq!(ids(filtered(&state, &web, "", &none, false)), [12, 10]);
        let codex = ListFilter {
            agent: Some("codex".into()),
            ..Default::default()
        };
        assert_eq!(ids(filtered(&state, &codex, "", &none, false)), [11]);
        let failed = ListFilter {
            status: Some(Status::Failed),
            ..Default::default()
        };
        assert_eq!(ids(filtered(&state, &failed, "", &none, false)), [11]);
        let live = ListFilter {
            archived: Archived::Hide,
            ..Default::default()
        };
        assert_eq!(ids(filtered(&state, &live, "", &none, false)), [12, 11, 10]);
        let gone = ListFilter {
            archived: Archived::Only,
            ..Default::default()
        };
        assert_eq!(ids(filtered(&state, &gone, "", &none, false)), [13]);
    }

    #[test]
    fn sorts_keep_pinned_first_and_archived_last() {
        let mut state = state();
        let none = Hits::default();
        let by = |sort| ListFilter {
            sort,
            ..Default::default()
        };
        assert_eq!(
            ids(filtered(&state, &by(Sort::Name), "", &none, false)),
            [12, 11, 10, 13]
        );
        state.sessions[2].pinned = false;
        assert_eq!(
            ids(filtered(&state, &by(Sort::Name), "", &none, false)),
            [11, 12, 10, 13]
        );
        assert_eq!(
            ids(filtered(&state, &by(Sort::Created), "", &none, false)),
            [12, 11, 10, 13]
        );
        // Recent follows the activity log; sessions without activity go after, newest first.
        state.log(10, "Started");
        state.log(11, "Started");
        state.log(10, "Finished");
        assert_eq!(
            ids(filtered(&state, &by(Sort::Recent), "", &none, false)),
            [10, 11, 12, 13]
        );
    }

    #[test]
    fn search_matches_titles_projects_and_conversations() {
        let state = state();
        let all = ListFilter::default();
        let none = Hits::default();
        assert_eq!(ids(filtered(&state, &all, "metrics", &none, false)), [11]);
        assert_eq!(ids(filtered(&state, &all, "API", &none, false)), [11, 13]);
        assert_eq!(ids(filtered(&state, &all, "codex", &none, false)), [11]);
        assert!(filtered(&state, &all, "redirect", &none, false).is_empty());
        let corpus: Vec<(u64, Vec<String>)> = state
            .sessions
            .iter()
            .map(|s| (s.id, s.messages.iter().map(|m| m.text.clone()).collect()))
            .collect();
        let found = search_corpus(&corpus, "login goes", 10);
        assert_eq!(
            found,
            [(10, "The redirect after LOGIN goes to /home".to_string())]
        );
        let hits = Hits {
            query: "redirect".into(),
            found: search_corpus(&corpus, "redirect", 10),
            asked: 1,
        };
        assert_eq!(ids(filtered(&state, &all, "redirect", &hits, false)), [10]);
        // Hits for another search don't count.
        assert!(filtered(&state, &all, "redirec", &hits, false).is_empty());
        assert!(search_corpus(&corpus, "x", 10).is_empty());
    }

    #[test]
    fn snippets_cut_around_the_match() {
        let long = format!("{} needle {}", "a ".repeat(60), "b ".repeat(80));
        let s = snippet(&long, "needle").unwrap();
        assert!(s.starts_with('…') && s.ends_with('…'));
        assert!(s.contains("needle"));
        assert!(s.chars().count() < 140);
        // Text whose lowercase form changes length still cuts on character boundaries.
        let odd = "İİİ Straße NEEDLE ü";
        assert!(snippet(odd, "needle").unwrap().contains("needle"));
        assert_eq!(snippet("nothing here", "needle"), None);
    }
}
