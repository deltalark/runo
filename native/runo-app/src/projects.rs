//! The New project / Edit project popup: folder, name, color and icon.

use super::*;
use std::path::{Path, PathBuf};

/// Accent colors a project can carry in the sidebar. The first entry is "no color".
pub(crate) const COLORS: &[(&str, u32)] = &[
    ("", 0),
    ("blue", 0x3b82f6),
    ("violet", 0x8b5cf6),
    ("pink", 0xec4899),
    ("red", 0xef4444),
    ("orange", 0xf97316),
    ("amber", 0xeab308),
    ("green", 0x22c55e),
    ("teal", 0x14b8a6),
];

/// Icons a project can carry. The first entry is the default folder.
pub(crate) const ICONS: &[(&str, IconName)] = &[
    ("folder", IconName::Folder),
    ("code", IconName::Code),
    ("globe", IconName::Globe),
    ("server", IconName::Server),
    ("database", IconName::Database),
    ("phone", IconName::Smartphone),
    ("terminal", IconName::Terminal),
    ("bot", IconName::Bot),
    ("brain", IconName::Brain),
    ("rocket", IconName::Rocket),
    ("box", IconName::Box),
    ("layers", IconName::Layers),
    ("cloud", IconName::Cloud),
    ("cpu", IconName::Cpu),
    ("flask", IconName::FlaskConical),
    ("game", IconName::Gamepad2),
    ("palette", IconName::Palette),
    ("book", IconName::BookOpen),
    ("briefcase", IconName::Briefcase),
    ("zap", IconName::Zap),
];

/// The project's accent color, if it has one.
pub(crate) fn color_of(project: &Project) -> Option<u32> {
    COLORS
        .iter()
        .find(|(key, _)| !key.is_empty() && *key == project.color)
        .map(|(_, hex)| *hex)
}

/// The project's icon, or the plain folder.
pub(crate) fn icon_of(project: &Project) -> IconName {
    ICONS
        .iter()
        .find(|(key, _)| *key == project.icon)
        .map(|(_, glyph)| *glyph)
        .unwrap_or(IconName::Folder)
}

/// The popup's state while it is open.
pub(crate) struct ProjectForm {
    /// Some when editing an existing project; its folder can't change.
    pub editing: Option<u64>,
    pub path: Option<PathBuf>,
    pub color: usize,
    pub icon: usize,
    /// Run `git init` in a folder that isn't a repository yet.
    pub git_init: bool,
    /// Open a new session in the project right after adding it.
    pub start_session: bool,
    /// The name follows the folder until the user types their own.
    pub auto_name: bool,
    /// A name to put in the field on the next render (picking a folder happens off the window).
    pub sync_name: Option<String>,
    /// Whether putting `sync_name` in also moves the focus to the name field. Not while
    /// the person is typing a path, or the rest of it would land in the name.
    pub focus_name: bool,
    /// The name Runo last filled in from a folder, so it keeps following the folder
    /// while the path is typed, until the person types a name of their own.
    pub filled_name: String,
    /// The setup command to put in its field on the next render.
    pub sync_setup: Option<String>,
    /// The checks to put in their field on the next render.
    pub sync_checks: Option<String>,
    /// Failing checks go back to the agent on their own this many times in a row.
    pub fix_rounds: u32,
    pub error: Option<String>,
    /// A folder path to put in the path field on the next render (after Browse).
    pub sync_path: Option<String>,
    /// What is wrong with the folder path typed in, or a pointer to the field when the
    /// folder picker didn't give a folder.
    pub path_error: Option<String>,
    /// Clone a repository from a URL instead of picking a folder.
    pub clone: bool,
    /// The folder the clone goes into, as a new folder named after the repository.
    pub parent: Option<PathBuf>,
    /// The clone is running; its latest progress line.
    pub cloning: Option<String>,
    /// Start with no code, for Design or Architecture: a new folder under `~/Runo/`.
    pub blank: Option<Mode>,
    /// Work through Crew rather than directly in sessions (Labs).
    pub crew: bool,
}

/// Where projects started without code get their folder: `~/Runo/`.
pub(crate) fn blank_parent() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join("Runo"))
}

/// A folder name for a project name: lowercase letters, digits and dashes.
pub(crate) fn folder_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// The folder a typed or pasted path names: `~` is the home folder, surrounding quotes
/// are dropped, and the path must be a full path to a folder that exists.
pub(crate) fn resolve_folder(typed: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let text = typed.trim();
    let text = text
        .strip_prefix(['"', '\''])
        .and_then(|t| t.strip_suffix(['"', '\'']))
        .unwrap_or(text)
        .trim();
    if text.is_empty() {
        return Err("Type or paste the folder's path, or choose it with Browse….".into());
    }
    let path = match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => {
            let Some(home) = home else {
                return Err("Could not find your home folder; type the full path.".into());
            };
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(text),
    };
    if !path.is_absolute() {
        return Err("Type the full path, such as ~/code/app.".into());
    }
    // Drop a trailing slash and "." parts so the project's path reads cleanly.
    let path: PathBuf = path.components().collect();
    if !path.exists() {
        return Err(format!("{} doesn't exist.", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("{} is a file, not a folder.", path.display()));
    }
    Ok(path)
}

/// The home folder, for `~` in typed paths.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Where clones go unless another folder is picked: beside the newest project, else home.
fn clone_parent(projects: &[Project]) -> Option<PathBuf> {
    projects
        .last()
        .and_then(|p| std::path::Path::new(&p.path).parent().map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
        })
}

/// The Remove project dialog while it is open.
pub(crate) struct RemoveProject {
    pub id: u64,
    /// Also delete the worktrees Runo made for this project's sessions.
    pub worktrees: bool,
    /// Also delete the project's folder from disk; needs the name typed to confirm.
    pub folder: bool,
    pub error: Option<String>,
}

impl RunoApp {
    /// Ask before taking a project out of Runo.
    pub(crate) fn ask_remove_project(
        &mut self,
        pid: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.demo_guard(cx) {
            return;
        }
        self.remove_project = Some(RemoveProject {
            id: pid,
            worktrees: false,
            folder: false,
            error: None,
        });
        self.remove_name
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Take the project out of Runo: its sessions stop and leave the sidebar. Files on disk
    /// are only deleted when the dialog asked for that.
    pub(crate) fn remove_project(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.remove_project.as_ref() else {
            return;
        };
        let (pid, worktrees, folder) = (form.id, form.worktrees, form.folder);
        let Some(project) = self.state.projects.iter().find(|p| p.id == pid).cloned() else {
            self.remove_project = None;
            return;
        };
        if folder && self.remove_name.read(cx).value().trim() != project.name {
            if let Some(form) = self.remove_project.as_mut() {
                form.error = Some(format!(
                    "Type {} to confirm deleting the folder.",
                    project.name
                ));
            }
            cx.notify();
            return;
        }
        let ids: Vec<u64> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == pid)
            .map(|s| s.id)
            .collect();
        let mut failed = Vec::new();
        for id in &ids {
            for target in [Target::Session(*id), Target::Terminal(*id)] {
                if let Some(handle) = self.handles.remove(&target) {
                    handle.kill();
                }
            }
            let Some(session) = self.state.session(*id).cloned() else {
                continue;
            };
            crate::connections::cleanup_session(&session);
            if worktrees && session.isolated && session.dir != project.path {
                if let Err(error) = git::remove_worktree(&project.path, &session.dir) {
                    failed.push(error);
                }
            }
            self.terminal.remove(id);
            self.stats.remove(id);
        }
        self.state.sessions.retain(|s| s.project_id != pid);
        self.prune_shells();
        self.tabs.retain(|t| !ids.contains(t));
        self.state.notes.remove(&pid);
        self.state.notes_meta.remove(&pid);
        // Playbooks tied to the project now run wherever they are started, by hand.
        for a in self
            .state
            .automations
            .iter_mut()
            .filter(|a| a.project_id == pid)
        {
            a.project_id = 0;
            a.every_minutes = 0;
            a.cron.clear();
        }
        self.state.projects.retain(|p| p.id != pid);
        self.expanded.retain(|p| *p != pid);
        if self.project == Some(pid) {
            self.project = None;
        }
        for slot in [
            &mut self.new_project,
            &mut self.notes_project,
            &mut self.auto_project,
        ] {
            if *slot == Some(pid) {
                *slot = None;
            }
        }
        if self.new_project.is_none() {
            self.new_project = self.state.projects.first().map(|p| p.id);
        }
        if self.selected.is_some_and(|id| ids.contains(&id)) {
            match self.tabs.last().copied() {
                Some(next) => self.select(next),
                // Nothing left open: stay in Sessions, on the list of sessions.
                None => {
                    self.selected = None;
                    if self.view == View::Session {
                        self.view = if self.state.projects.is_empty() {
                            View::Home
                        } else {
                            View::AllSessions
                        };
                    }
                }
            }
        }
        let mut message = format!(
            "Removed {} from Runo. Its files are still on disk.",
            project.name
        );
        if folder {
            match std::fs::remove_dir_all(&project.path) {
                Ok(()) => message = format!("Removed {} and deleted its folder.", project.name),
                Err(error) => failed.push(format!("{}: {error}", project.path)),
            }
        }
        if failed.is_empty() {
            self.info = Some((message, std::time::Instant::now()));
        } else {
            self.fail(
                "project",
                format!(
                    "Removed {} from Runo, but some files could not be deleted: {}",
                    project.name,
                    failed.join("; ")
                ),
            );
        }
        self.remove_project = None;
        self.save();
        self.refresh();
        cx.notify();
    }

    /// The Remove project dialog.
    pub(crate) fn remove_project_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.remove_project.as_ref()?;
        let project = self.state.projects.iter().find(|p| p.id == form.id)?;
        let sessions: Vec<&Session> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == form.id)
            .collect();
        let running = sessions
            .iter()
            .filter(|s| self.handles.contains_key(&Target::Session(s.id)))
            .count();
        let isolated = sessions
            .iter()
            .filter(|s| s.isolated && s.dir != project.path)
            .count();
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
        let mut what = vec![if sessions.is_empty() {
            "It has no sessions.".to_string()
        } else {
            format!(
                "Its {} leave the sidebar{}.",
                plural(sessions.len(), "session"),
                if running > 0 {
                    format!(", and {} stop", plural(running, "running agent"))
                } else {
                    String::new()
                }
            )
        }];
        if self.state.notes.contains_key(&form.id) {
            what.push("Its project notes are removed.".into());
        }
        what.push("Nothing on disk changes unless you tick an option below.".into());
        let check = |id: &'static str,
                     on: bool,
                     title: String,
                     desc: String,
                     danger: bool,
                     set: fn(&mut RemoveProject, bool)| {
            let tick = if danger { c_red() } else { c_blue() };
            div()
                .id(id)
                .flex()
                .items_start()
                .gap_2()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(f) = this.remove_project.as_mut() {
                        set(f, !on);
                        f.error = None;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .mt(px(1.))
                        .size(px(16.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(if on { tick } else { c_border() }))
                        .when(on, |el| el.bg(rgb(tick)))
                        .when(on, |el| el.child(icon(IconName::Check, 11., 0xffffff))),
                )
                .child(
                    div()
                        .v_flex()
                        .child(
                            div()
                                .text_size(px(12.5))
                                .when(danger && on, |el| el.text_color(rgb(c_red())))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(desc),
                        ),
                )
        };
        let folder_on = form.folder;
        let name_matches = self.remove_name.read(cx).value().trim() == project.name;
        let can_remove = !folder_on || name_matches;
        let card = div()
            .id("remove-card")
            .w(px(500.))
            .max_w(relative(0.94))
            .p_6()
            .v_flex()
            .gap_4()
            .rounded(px(crate::shell::ISLAND_R))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .bg(rgb(settings::mix(c_red(), c_canvas(), 0.12)))
                            .child(icon(IconName::FolderMinus, 18., c_red())),
                    )
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Remove {}?", project.name)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(rgb(c_muted()))
                                    .font_family(self.mono_font.clone())
                                    .child(project.path.clone()),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(c_secondary()))
                    .child(what.join(" ")),
            )
            .child(
                div()
                    .v_flex()
                    .gap_3()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .when(isolated > 0, |el| {
                        el.child(check(
                            "remove-worktrees",
                            form.worktrees,
                            format!("Also delete {}", plural(isolated, "worktree")),
                            "The isolated copies Runo made for these sessions, with any uncommitted work in them. Branches stay.".into(),
                            false,
                            |f, v| f.worktrees = v,
                        ))
                    })
                    .child(check(
                        "remove-folder",
                        folder_on,
                        "Also delete the project folder from disk".into(),
                        "Permanently deletes the folder and everything in it. This can't be undone.".into(),
                        true,
                        |f, v| f.folder = v,
                    ))
                    .when(folder_on, |el| {
                        el.child(
                            div()
                                .v_flex()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(rgb(c_secondary()))
                                        .child(format!("Type {} to confirm", project.name)),
                                )
                                .child(Input::new(&self.remove_name).bordered(true).h(px(32.))),
                        )
                    }),
            )
            .children(form.error.clone().map(|why| {
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_red()))
                    .child(why)
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("remove-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remove_project = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "remove-go",
                            if folder_on {
                                "Remove and delete folder"
                            } else {
                                "Remove from Runo"
                            },
                            false,
                        )
                        .border_color(rgb(c_red()))
                        .text_color(rgb(c_red()))
                        .when(!can_remove, |el| el.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.remove_project(cx))),
                    ),
            );
        Some(
            fade(
                div()
                    .id("remove-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(backdrop)
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.remove_project = None;
                        cx.notify();
                    }))
                    .child(rise(card, "remove-card-rise", 300)),
                "remove-backdrop-fade",
                200,
            )
            .into_any_element(),
        )
    }

    /// Open the New project popup.
    pub(crate) fn pick_project(&mut self, cx: &mut Context<Self>) {
        self.project_form = Some(ProjectForm {
            editing: None,
            path: None,
            color: 0,
            icon: 0,
            git_init: false,
            start_session: true,
            auto_name: true,
            sync_name: Some(String::new()),
            focus_name: true,
            filled_name: String::new(),
            sync_setup: Some(String::new()),
            sync_checks: Some(String::new()),
            fix_rounds: 0,
            error: None,
            sync_path: Some(String::new()),
            path_error: None,
            clone: false,
            parent: clone_parent(&self.state.projects),
            cloning: None,
            blank: None,
            crew: false,
        });
        cx.notify();
    }

    /// Open the New project popup set to start without code, for Design or Architecture.
    pub(crate) fn start_project(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.pick_project(cx);
        let glyph = if mode == Mode::Design {
            "palette"
        } else {
            "layers"
        };
        if let Some(form) = self.project_form.as_mut() {
            form.blank = Some(mode);
            form.start_session = false;
            form.icon = ICONS.iter().position(|(k, _)| *k == glyph).unwrap_or(0);
        }
    }

    /// Open the popup to change an existing project's name, color or icon.
    pub(crate) fn edit_project(&mut self, pid: u64) {
        let Some(p) = self.state.projects.iter().find(|p| p.id == pid) else {
            return;
        };
        self.project_form = Some(ProjectForm {
            editing: Some(pid),
            path: Some(PathBuf::from(&p.path)),
            color: COLORS.iter().position(|(k, _)| *k == p.color).unwrap_or(0),
            icon: ICONS.iter().position(|(k, _)| *k == p.icon).unwrap_or(0),
            git_init: false,
            start_session: false,
            auto_name: false,
            sync_name: Some(p.name.clone()),
            focus_name: true,
            filled_name: String::new(),
            sync_setup: Some(p.setup.clone()),
            sync_checks: Some(p.checks.clone()),
            fix_rounds: p.fix_rounds,
            error: None,
            sync_path: None,
            path_error: None,
            clone: false,
            parent: None,
            cloning: None,
            blank: None,
            crew: crate::crew::is_crew(p),
        });
    }

    /// Point a project whose folder moved at its new place. Sessions that worked in the
    /// project folder itself follow it; isolated ones keep their own worktrees.
    pub(crate) fn relink_project(&mut self, pid: u64, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose the project's folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.first().cloned() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let new = path.to_string_lossy().into_owned();
                if this
                    .state
                    .projects
                    .iter()
                    .any(|p| p.path == new && p.id != pid)
                {
                    this.fail("project", "That folder is already a project.".into());
                    cx.notify();
                    return;
                }
                let Some(project) = this.state.projects.iter_mut().find(|p| p.id == pid) else {
                    return;
                };
                let old = std::mem::replace(&mut project.path, new.clone());
                let name = project.name.clone();
                for s in this
                    .state
                    .sessions
                    .iter_mut()
                    .filter(|s| s.project_id == pid && s.dir == old)
                {
                    s.dir = new.clone();
                }
                this.info = Some((
                    format!("{name} now points at {new}."),
                    std::time::Instant::now(),
                ));
                this.save();
                this.refresh();
                cx.notify();
            });
        })
        .detach();
    }

    /// Pick the folder a clone goes into.
    pub(crate) fn browse_clone_parent(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Clone into".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.first().cloned() {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(form) = this.project_form.as_mut() {
                            form.parent = Some(path);
                            form.error = None;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Clone the form's URL in the background, showing git's progress, then add the clone
    /// as the project.
    fn start_clone(&mut self, cx: &mut Context<Self>) {
        let url = self.clone_url.read(cx).value().trim().to_string();
        let Some(form) = self.project_form.as_mut() else {
            return;
        };
        let name = git::clone_dir_name(&url);
        let why = if url.is_empty() || name.is_empty() {
            Some("Enter the repository's URL.")
        } else if form.parent.is_none() {
            Some("Choose where to clone it.")
        } else {
            None
        };
        if let Some(why) = why {
            form.error = Some(why.into());
            cx.notify();
            return;
        }
        let dest = form.parent.clone().unwrap_or_default().join(&name);
        form.error = None;
        form.cloning = Some(format!("Cloning into {}…", dest.display()));
        let tx = self.ui_tx.clone();
        std::thread::spawn(move || {
            let progress = tx.clone();
            let result = git::clone(&url, &dest, |line| {
                let line = line.to_string();
                let _ = progress.send(Box::new(move |this: &mut RunoApp, _| {
                    if let Some(form) = this.project_form.as_mut().filter(|f| f.cloning.is_some()) {
                        form.cloning = Some(line);
                    }
                }));
            });
            let _ = tx.send(Box::new(move |this: &mut RunoApp, cx| {
                this.clone_finished(dest, result, cx)
            }));
        });
        cx.notify();
    }

    fn clone_finished(
        &mut self,
        dest: PathBuf,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        match (result, self.project_form.as_mut()) {
            (Ok(()), Some(form)) => {
                form.cloning = None;
                form.clone = false;
                form.path = Some(dest);
                self.submit_project_form(cx);
            }
            // The popup was closed while cloning: add the project as it is.
            (Ok(()), None) => {
                let name = dest
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match self.add_project(&dest, String::new(), String::new(), String::new(), false) {
                    Ok(()) => {
                        self.info = Some((
                            format!("Cloned and added {name}."),
                            std::time::Instant::now(),
                        ))
                    }
                    Err(why) => self.fail("project", why),
                }
            }
            (Err(why), Some(form)) => {
                crate::applog::warn(
                    "git",
                    format!("git clone failed: {}", crate::applog::gist(&why)),
                );
                form.cloning = None;
                form.error = Some(format!("Could not clone: {why}"));
            }
            (Err(why), None) => self.fail("project", format!("Could not clone: {why}")),
        }
        cx.notify();
    }

    pub(crate) fn browse_project_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let picked = match rx.await {
                Ok(Ok(Some(paths))) => Ok(paths.first().cloned()),
                Ok(Ok(None)) => Ok(None),
                // No picker could be shown, such as on Linux without a desktop portal.
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err("the picker closed".to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                match picked {
                    Ok(Some(path)) => {
                        if let Some(form) = this.project_form.as_mut() {
                            form.sync_path = Some(path.to_string_lossy().into_owned());
                        }
                        this.use_project_folder(path, true, cx);
                    }
                    Ok(None) => {
                        if let Some(form) = this.project_form.as_mut().filter(|f| f.path.is_none())
                        {
                            form.path_error = Some(
                                "No folder was chosen. You can also type or paste its path above."
                                    .into(),
                            );
                        }
                    }
                    Err(why) => {
                        crate::applog::info(
                            "project",
                            format!("the folder picker didn't open: {why}"),
                        );
                        if let Some(form) = this.project_form.as_mut() {
                            form.path_error = Some(
                                "Couldn't open a folder picker here. Type or paste the folder's path above instead."
                                    .into(),
                            );
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Take `path` as the new project's folder; the name follows the folder until the
    /// user types their own.
    fn use_project_folder(&mut self, path: PathBuf, focus: bool, cx: &mut Context<Self>) {
        let typed = self.project_name.read(cx).value().trim().to_string();
        let Some(form) = self.project_form.as_mut() else {
            return;
        };
        if form.auto_name && (typed.is_empty() || typed == form.filled_name) {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            form.filled_name = name.clone();
            form.sync_name = Some(name);
            form.focus_name = focus;
        }
        form.path = Some(path);
        form.error = None;
        form.path_error = None;
    }

    /// The path field changed: a folder that exists is used right away. What's wrong
    /// with any other path is said when the person tries to add it.
    pub(crate) fn project_path_typed(&mut self, cx: &mut Context<Self>) {
        let typed = self.project_path.read(cx).value().to_string();
        let name = self.project_name.read(cx).value().trim().to_string();
        let Some(form) = self.project_form.as_mut().filter(|f| f.editing.is_none()) else {
            return;
        };
        form.path_error = None;
        match resolve_folder(&typed, home_dir().as_deref()) {
            Ok(path) => {
                if form.path.as_ref() != Some(&path) {
                    self.use_project_folder(path, false, cx);
                }
            }
            Err(_) => {
                form.path = None;
                // A name filled in from a folder along the way goes with it.
                if form.auto_name && !name.is_empty() && name == form.filled_name {
                    form.filled_name.clear();
                    form.sync_name = Some(String::new());
                    form.focus_name = false;
                }
            }
        }
        cx.notify();
    }

    /// Put a pending name into the name field; called from render, which has the window.
    pub(crate) fn sync_project_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.project_form.as_mut().and_then(|f| f.sync_path.take()) {
            self.project_path
                .update(cx, |input, cx| input.set_value(path, window, cx));
        }
        if let Some(setup) = self.project_form.as_mut().and_then(|f| f.sync_setup.take()) {
            self.project_setup
                .update(cx, |input, cx| input.set_value(setup, window, cx));
        }
        if let Some(checks) = self
            .project_form
            .as_mut()
            .and_then(|f| f.sync_checks.take())
        {
            self.project_checks
                .update(cx, |input, cx| input.set_value(checks, window, cx));
        }
        let Some((name, focus)) = self
            .project_form
            .as_mut()
            .and_then(|f| Some((f.sync_name.take()?, f.focus_name)))
        else {
            return;
        };
        self.project_name.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            if focus {
                input.focus(window, cx);
            }
        });
    }

    pub(crate) fn submit_project_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.project_form.as_ref() else {
            return;
        };
        let name = self.project_name.read(cx).value().trim().to_string();
        let setup = self.project_setup.read(cx).value().trim().to_string();
        let checks = self.project_checks.read(cx).value().trim().to_string();
        let fix_rounds = form.fix_rounds;
        let crew = form.crew;
        let color = COLORS[form.color].0.to_string();
        let glyph = ICONS[form.icon].0.to_string();
        if let Some(pid) = form.editing {
            if let Some(p) = self.state.projects.iter_mut().find(|p| p.id == pid) {
                if !name.is_empty() {
                    p.name = name;
                }
                p.color = color;
                p.icon = glyph;
                p.setup = setup;
                p.checks = checks;
                p.fix_rounds = fix_rounds;
            }
            self.set_project_mode(pid, crew);
            self.project_form = None;
            self.save();
            cx.notify();
            return;
        }
        if form.cloning.is_some() {
            return;
        }
        if form.clone {
            self.start_clone(cx);
            return;
        }
        if let Some(mode) = form.blank {
            self.create_blank_project(mode, name, color, glyph, cx);
            return;
        }
        let Some(path) = form.path.clone() else {
            let typed = self.project_path.read(cx).value().to_string();
            let why = match resolve_folder(&typed, home_dir().as_deref()) {
                Ok(_) => "Choose a folder first.".to_string(),
                Err(why) => why,
            };
            if let Some(form) = self.project_form.as_mut() {
                form.path_error = Some(why);
            }
            cx.notify();
            return;
        };
        let (git_init, start) = (form.git_init, form.start_session);
        let path_str = path.to_string_lossy().into_owned();
        if git_init && !git::is_repo(&path_str) {
            let out = std::process::Command::new("git")
                .arg("init")
                .current_dir(&path)
                .output();
            let failed = match out {
                Ok(out) if out.status.success() => None,
                Ok(out) => Some(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                Err(e) => Some(e.to_string()),
            };
            if let Some(why) = failed {
                crate::applog::warn(
                    "git",
                    format!("git init failed: {}", crate::applog::gist(&why)),
                );
                if let Some(form) = self.project_form.as_mut() {
                    form.error = Some(format!("git init failed: {why}"));
                }
                cx.notify();
                return;
            }
            // Worktrees need a commit to branch from. Without a git identity this fails,
            // and the first sessions run in the folder until something is committed.
            let _ = git::initial_commit(&path_str);
        }
        match self.add_project(&path, name, color, glyph, start) {
            Ok(()) => {
                self.project_form = None;
                if let Some(p) = self.state.projects.last_mut() {
                    p.setup = setup;
                    p.checks = checks;
                    p.fix_rounds = fix_rounds;
                    if crew {
                        p.mode = crate::crew::MODE_CREW.into();
                    }
                }
                self.save();
            }
            Err(why) => {
                crate::applog::warn("project", format!("could not add the project: {why}"));
                if let Some(form) = self.project_form.as_mut() {
                    form.error = Some(why);
                }
            }
        }
        cx.notify();
    }

    /// Make `~/Runo/<name>/` with git and add it as a project that has no code yet, then
    /// open its Design or Architecture page.
    fn create_blank_project(
        &mut self,
        mode: Mode,
        name: String,
        color: String,
        glyph: String,
        cx: &mut Context<Self>,
    ) {
        let slug = folder_name(&name);
        let fail = |this: &mut Self, why: String, cx: &mut Context<Self>| {
            crate::applog::warn("project", format!("could not create the project: {why}"));
            if let Some(form) = this.project_form.as_mut() {
                form.error = Some(why);
            }
            cx.notify();
        };
        if slug.is_empty() {
            return fail(self, "Give the project a name.".into(), cx);
        }
        let Some(parent) = blank_parent() else {
            return fail(self, "Could not find your home folder.".into(), cx);
        };
        let dir = parent.join(&slug);
        let taken = std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some());
        if taken {
            return fail(
                self,
                format!("{} already exists. Pick another name.", dir.display()),
                cx,
            );
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return fail(self, format!("Could not create {}: {e}", dir.display()), cx);
        }
        let path_str = dir.to_string_lossy().into_owned();
        if !git::is_repo(&path_str) {
            let out = std::process::Command::new("git")
                .arg("init")
                .current_dir(&dir)
                .output();
            if out.is_ok_and(|o| o.status.success()) {
                // Sessions branch from a commit once code is added here.
                let _ = git::initial_commit(&path_str);
            }
        }
        match self.add_project(&dir, name, color, glyph, false) {
            Ok(()) => {
                self.project_form = None;
                let id = self.state.projects.last().map(|p| p.id);
                self.project = id;
                self.home_all = false;
                self.view = if mode == Mode::Design {
                    View::Design
                } else {
                    View::Architecture
                };
                self.save();
                cx.notify();
            }
            Err(why) => fail(self, why, cx),
        }
    }

    pub(crate) fn project_modal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.project_form.as_ref()?;
        let editing = form.editing.is_some();
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.55)
        } else {
            hsla(0., 0., 0., 0.3)
        };
        let accent = match COLORS[form.color].1 {
            0 => c_secondary(),
            hex => hex,
        };
        let checks = form.blank.is_none().then(|| self.checks_field(cx));
        let mode_choice = form.blank.is_none().then(|| {
            let on = form.crew;
            let choice = |id: &'static str, title: &'static str, crew: bool| {
                shell::seg_choice(id, format!("project-mode-{}", crew == on), crew == on)
                    .flex_1()
                    .child(title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(f) = this.project_form.as_mut() {
                            f.crew = crew;
                        }
                        cx.notify();
                    }))
            };
            div()
                .v_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_secondary()))
                        .child("How you work here"),
                )
                .child(
                    shell::seg_track()
                        .child(choice("project-mode-crew", "Crew", true))
                        .child(choice("project-mode-direct", "Direct", false)),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(if on {
                            "You talk to Crew. It answers, and starts threads for the work, each an ordinary session. Switch any time from the project menu."
                        } else {
                            "You start and run sessions yourself, as Runo always has. Switch to Crew any time from the project menu."
                        }),
                )
        });
        let label = |text: &'static str| {
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(c_secondary()))
                .child(text)
        };
        let path_text = form.path.as_ref().map(|p| p.to_string_lossy().into_owned());
        let is_git = path_text.as_deref().is_some_and(git::is_repo);

        let folder = div()
            .id("project-folder")
            .h(px(40.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .bg(tint(c_subtle()))
            .when(!editing, |el| {
                el.cursor_pointer()
                    .hover(|el| el.border_color(rgb(c_muted())))
                    .on_click(cx.listener(|this, _, _, cx| this.browse_project_folder(cx)))
            })
            .child(icon(IconName::FolderOpen, 15., c_muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .font_family(self.mono_font.clone())
                    .text_color(rgb(if path_text.is_some() {
                        c_text()
                    } else {
                        c_muted()
                    }))
                    .child(
                        path_text
                            .clone()
                            .unwrap_or_else(|| "No folder chosen".into()),
                    ),
            )
            .when(!editing, |el| {
                el.child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_blue()))
                        .child(if path_text.is_some() {
                            "Change…"
                        } else {
                            "Browse…"
                        }),
                )
            });

        // A new project's folder can be typed or pasted, as well as picked with Browse.
        let path_error = form.path_error.clone();
        let folder_entry = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.project_path)
                        .bordered(true)
                        .h(px(34.))
                        .font_family(self.mono_font.clone())
                        .text_size(px(12.5)),
                ),
            )
            .child(
                self.button("project-browse", "Browse…", false)
                    .h(px(34.))
                    .on_click(cx.listener(|this, _, _, cx| this.browse_project_folder(cx))),
            );
        let typed_path = !self.project_path.read(cx).value().trim().is_empty();

        let clone_mode = form.clone && !editing;
        let blank = form.blank.filter(|_| !editing);
        let cloning = form.cloning.clone();
        let mut source = div()
            .flex()
            .items_center()
            .p(px(3.))
            .gap(px(2.))
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .bg(rgb(c_canvas()));
        for (j, (value, start, glyph, text)) in [
            (false, None, IconName::FolderOpen, "Folder"),
            (true, None, IconName::GitBranch, "Clone"),
            (
                false,
                Some(Mode::Design),
                IconName::PanelsTopLeft,
                "Design an app",
            ),
            (
                false,
                Some(Mode::Architecture),
                IconName::Network,
                "Architecture",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let on = clone_mode == value && blank == start;
            source = source.child(
                div()
                    .id(("project-source", j))
                    .flex_1()
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |el| {
                        el.bg(tint(c_selected()))
                            .text_color(rgb(c_blue()))
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!on, |el| {
                        el.text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(f) = this.project_form.as_mut().filter(|f| f.cloning.is_none())
                        {
                            f.clone = value;
                            f.blank = start;
                            f.start_session = start.is_none();
                            f.error = None;
                        }
                        cx.notify();
                    }))
                    .child(icon(glyph, 12., if on { c_blue() } else { c_muted() }))
                    .child(text),
            );
        }
        let url = self.clone_url.read(cx).value().trim().to_string();
        let repo_name = git::clone_dir_name(&url);
        let parent_text = form
            .parent
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let clone_box = div()
            .v_flex()
            .gap_1p5()
            .child(label("Repository URL"))
            .child(Input::new(&self.clone_url).bordered(true).h(px(34.)))
            .child(label("Clone into"))
            .child(
                div()
                    .id("project-clone-parent")
                    .h(px(40.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .bg(tint(c_subtle()))
                    .cursor_pointer()
                    .hover(|el| el.border_color(rgb(c_muted())))
                    .on_click(cx.listener(|this, _, _, cx| this.browse_clone_parent(cx)))
                    .child(icon(IconName::FolderOpen, 15., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .font_family(self.mono_font.clone())
                            .child(
                                parent_text
                                    .clone()
                                    .unwrap_or_else(|| "No folder chosen".into()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_blue()))
                            .child("Change…"),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .truncate()
                    .child(match (&parent_text, repo_name.is_empty()) {
                        (Some(parent), false) => format!(
                            "Creates {}",
                            std::path::Path::new(parent).join(&repo_name).display()
                        ),
                        _ => "Clones into a new folder named after the repository.".into(),
                    }),
            )
            .when_some(cloning.clone(), |el, line| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(tint(c_subtle()))
                        .child(icon(IconName::LoaderCircle, 13., c_blue()).animated(
                            "clone-spin",
                            Animation::new(Duration::from_millis(900)).repeat(),
                            |i, d| i.rotate(percentage(d)),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.5))
                                .font_family(self.mono_font.clone())
                                .text_color(rgb(c_secondary()))
                                .child(line),
                        ),
                )
            });

        let typed = self.project_name.read(cx).value().trim().to_string();
        let blank_box = div()
            .v_flex()
            .gap_1p5()
            .child(label("Folder"))
            .child(
                div()
                    .h(px(40.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .bg(tint(c_subtle()))
                    .child(icon(IconName::FolderPlus, 15., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .font_family(self.mono_font.clone())
                            .child(match (blank_parent(), folder_name(&typed)) {
                                (Some(parent), slug) if !slug.is_empty() => {
                                    parent.join(slug).display().to_string()
                                }
                                (Some(parent), _) => format!("{}/…", parent.display()),
                                (None, _) => "~/Runo/…".into(),
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("A new folder with git and no code yet. Sessions you start here add the code to this same project."),
            );

        let mut swatches = div().flex().flex_wrap().gap_2();
        for (i, (key, hex)) in COLORS.iter().enumerate() {
            let on = form.color == i;
            swatches = swatches.child(
                div()
                    .id(("project-color", i))
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .border_color(if on {
                        rgb(c_text()).into()
                    } else {
                        hsla(0., 0., 0., 0.)
                    })
                    .tooltip(move |window, cx| {
                        let tip: SharedString = if key.is_empty() {
                            "No color".into()
                        } else {
                            let mut s = key.to_string();
                            s[..1].make_ascii_uppercase();
                            s.into()
                        };
                        gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(f) = this.project_form.as_mut() {
                            f.color = i;
                        }
                        cx.notify();
                    }))
                    .child(if *hex == 0 {
                        div()
                            .size(px(16.))
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(c_muted()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::Minus, 10., c_muted()))
                    } else {
                        div().size(px(16.)).rounded_full().bg(rgb(*hex))
                    }),
            );
        }

        let mut glyphs = div().flex().flex_wrap().gap_1();
        for (i, (_, glyph)) in ICONS.iter().enumerate() {
            let on = form.icon == i;
            glyphs = glyphs.child(
                div()
                    .id(("project-icon", i))
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .border_1()
                    .border_color(if on {
                        rgb(accent).into()
                    } else {
                        hsla(0., 0., 0., 0.)
                    })
                    .when(on, |el| el.bg(tint(c_subtle())))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(f) = this.project_form.as_mut() {
                            f.icon = i;
                        }
                        cx.notify();
                    }))
                    .child(icon(*glyph, 15., if on { accent } else { c_secondary() })),
            );
        }

        let check = |id: &'static str,
                     on: bool,
                     title: &'static str,
                     desc: String,
                     set: fn(&mut ProjectForm, bool)| {
            div()
                .id(id)
                .flex()
                .items_start()
                .gap_2()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(f) = this.project_form.as_mut() {
                        set(f, !on);
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .mt(px(1.))
                        .size(px(16.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(if on { c_blue() } else { c_border() }))
                        .when(on, |el| el.bg(rgb(c_blue())))
                        .when(on, |el| el.child(icon(IconName::Check, 11., 0xffffff))),
                )
                .child(
                    div()
                        .v_flex()
                        .child(div().text_size(px(12.5)).child(title))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(c_muted()))
                                .child(desc),
                        ),
                )
        };

        let preview =
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(36.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_lg()
                        .bg(tint(c_subtle()))
                        .border_1()
                        .border_color(divider())
                        .child(icon(ICONS[form.icon].1, 18., accent)),
                )
                .child(
                    div()
                        .v_flex()
                        .min_w_0()
                        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(
                            if editing {
                                "Edit project"
                            } else {
                                "New project"
                            },
                        ))
                        .child(div().text_xs().text_color(rgb(c_muted())).truncate().child(
                            match (editing, blank) {
                                (true, _) => {
                                    "Change how this project looks and how its work is checked"
                                }
                                (false, Some(Mode::Design)) => {
                                    "Start with design references and notes, before any code"
                                }
                                (false, Some(_)) => {
                                    "Sketch the system and its decisions, before any code"
                                }
                                (false, None) => "Pick a folder for Runo's agents to work in",
                            },
                        )),
                );

        let card = div()
            .id("project-card")
            .w(px(520.))
            .max_w(relative(0.94))
            .max_h(relative(0.92))
            .overflow_y_scroll()
            .p_6()
            .v_flex()
            .gap_4()
            .rounded(px(crate::shell::ISLAND_R))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(preview)
            .when(!editing, |el| el.child(source))
            .when(clone_mode, |el| el.child(clone_box))
            .when(blank.is_some(), |el| el.child(blank_box))
            .when(!clone_mode && blank.is_none(), |el| {
                el.child(
                    div()
                        .v_flex()
                        .gap_1p5()
                        .child(label("Folder"))
                        .map(|el| {
                            if editing {
                                el.child(folder)
                            } else {
                                el.child(folder_entry)
                            }
                        })
                        .children(path_error.map(|why| {
                            div()
                                .id("project-path-error")
                                .text_size(px(11.5))
                                .text_color(rgb(c_red()))
                                .child(why)
                        })),
                )
            })
            .child(
                div()
                    .v_flex()
                    .gap_1p5()
                    .child(label("Name"))
                    .child(Input::new(&self.project_name).bordered(true).h(px(34.))),
            )
            .when(blank.is_none(), |el| el.child(
                div()
                    .v_flex()
                    .gap_1p5()
                    .child(label("Worktree setup command"))
                    .child(Input::new(&self.project_setup).bordered(true).h(px(34.)))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child("Runs in each new worktree before its agent starts, such as npm install. Empty uses the one in Settings."),
                    ),
            ))
            .children(checks)
            .children(mode_choice)
            .child(div().v_flex().gap_1p5().child(label("Color")).child(swatches))
            .child(div().v_flex().gap_1p5().child(label("Icon")).child(glyphs))
            .when(!editing && blank.is_none(), |el| {
                el.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .pt_1()
                        .when(path_text.is_some() && !is_git, |el| {
                            el.child(check(
                                "project-git",
                                form.git_init,
                                "Initialize a Git repository",
                                "This folder isn't a repository yet. Git lets sessions run in isolated worktrees and shows changes.".into(),
                                |f, v| f.git_init = v,
                            ))
                        })
                        .child(check(
                            "project-start",
                            form.start_session,
                            "Start a session",
                            "Open a new session in this project right away.".into(),
                            |f, v| f.start_session = v,
                        )),
                )
            })
            .children(form.error.clone().map(|why| {
                div()
                    .text_size(px(12.))
                    .text_color(rgb(c_red()))
                    .child(why)
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("project-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_form = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "project-save",
                            if editing {
                                "Save"
                            } else if cloning.is_some() {
                                "Cloning…"
                            } else if clone_mode {
                                "Clone and add"
                            } else if blank.is_some() {
                                "Create project"
                            } else {
                                "Add project"
                            },
                            true,
                        )
                        .when(
                            !editing
                                && (cloning.is_some()
                                    || if clone_mode {
                                        repo_name.is_empty()
                                    } else if blank.is_some() {
                                        folder_name(&typed).is_empty()
                                    } else {
                                        path_text.is_none() && !typed_path
                                    }),
                            |el| el.opacity(0.5),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.submit_project_form(cx))),
                    ),
            );
        Some(
            fade(
                div()
                    .id("project-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(backdrop)
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_form = None;
                        cx.notify();
                    }))
                    .child(rise(card, "project-card-rise", 300)),
                "project-backdrop-fade",
                200,
            )
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn typed_folder_paths_are_checked() {
        let tmp = std::env::temp_dir().join(format!("runo-path-test-{}", std::process::id()));
        let dir = tmp.join("app");
        std::fs::create_dir_all(&dir).unwrap();
        let file = tmp.join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let home = Some(tmp.as_path());
        assert_eq!(resolve_folder("~/app", home), Ok(dir.clone()));
        assert_eq!(resolve_folder("  ~/app/  ", home), Ok(dir.clone()));
        assert_eq!(resolve_folder("~", home), Ok(tmp.clone()));
        let quoted = format!("'{}'", dir.display());
        assert_eq!(resolve_folder(&quoted, home), Ok(dir.clone()));
        assert!(resolve_folder("~/missing", home)
            .unwrap_err()
            .ends_with("doesn't exist."));
        assert!(resolve_folder("~/notes.txt", home)
            .unwrap_err()
            .contains("is a file, not a folder"));
        assert!(resolve_folder("app", home)
            .unwrap_err()
            .contains("full path"));
        assert!(
            resolve_folder("~other/app", home).is_err(),
            "no other users' homes"
        );
        assert!(resolve_folder("   ", home).is_err());
        assert!(resolve_folder("~/app", None)
            .unwrap_err()
            .contains("home folder"));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn clones_go_beside_the_newest_project() {
        let projects = vec![
            Project {
                path: "/code/old/app".into(),
                ..Default::default()
            },
            Project {
                path: "/work/api".into(),
                ..Default::default()
            },
        ];
        assert_eq!(clone_parent(&projects), Some(PathBuf::from("/work")));
    }

    #[test]
    fn projects_without_code_get_a_plain_folder_name() {
        assert_eq!(folder_name("Habit tracker"), "habit-tracker");
        assert_eq!(folder_name("  Acme: Checkout v2! "), "acme-checkout-v2");
        assert_eq!(folder_name("!!!"), "");
    }

    #[test]
    fn project_look_falls_back_to_plain_folder() {
        let mut p = Project {
            id: 1,
            name: "x".into(),
            path: "/x".into(),
            ..Default::default()
        };
        assert_eq!(color_of(&p), None);
        assert_eq!(icon_of(&p), IconName::Folder);
        p.color = "teal".into();
        p.icon = "rocket".into();
        assert_eq!(color_of(&p), Some(0x14b8a6));
        assert_eq!(icon_of(&p), IconName::Rocket);
        p.color = "nonsense".into();
        assert_eq!(color_of(&p), None);
    }
}
