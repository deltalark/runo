//! The New project / Edit project popup: folder, name, color and icon.

use super::*;
use std::path::PathBuf;

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
    pub error: Option<String>,
}

impl RunoApp {
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
            error: None,
        });
        cx.notify();
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
            error: None,
        });
    }

    pub(crate) fn browse_project_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.first().cloned() {
                    let _ = this.update(cx, |this, cx| {
                        // The name follows the folder until the user types their own.
                        let typed = this.project_name.read(cx).value().trim().to_string();
                        if let Some(form) = this.project_form.as_mut() {
                            let previous = form
                                .path
                                .as_ref()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if form.auto_name && (typed.is_empty() || typed == previous) {
                                form.sync_name =
                                    path.file_name().map(|n| n.to_string_lossy().into_owned());
                            }
                            form.path = Some(path);
                            form.error = None;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Put a pending name into the name field; called from render, which has the window.
    pub(crate) fn sync_project_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.project_form.as_mut().and_then(|f| f.sync_name.take()) else {
            return;
        };
        self.project_name.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
    }

    pub(crate) fn submit_project_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.project_form.as_ref() else {
            return;
        };
        let name = self.project_name.read(cx).value().trim().to_string();
        let color = COLORS[form.color].0.to_string();
        let glyph = ICONS[form.icon].0.to_string();
        if let Some(pid) = form.editing {
            if let Some(p) = self.state.projects.iter_mut().find(|p| p.id == pid) {
                if !name.is_empty() {
                    p.name = name;
                }
                p.color = color;
                p.icon = glyph;
            }
            self.project_form = None;
            self.save();
            cx.notify();
            return;
        }
        let Some(path) = form.path.clone() else {
            if let Some(form) = self.project_form.as_mut() {
                form.error = Some("Choose a folder first.".into());
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
                if let Some(form) = self.project_form.as_mut() {
                    form.error = Some(format!("git init failed: {why}"));
                }
                cx.notify();
                return;
            }
        }
        match self.add_project(&path, name, color, glyph, start) {
            Ok(()) => self.project_form = None,
            Err(why) => {
                if let Some(form) = self.project_form.as_mut() {
                    form.error = Some(why);
                }
            }
        }
        cx.notify();
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
        let label = |text: &'static str| {
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
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
            .border_color(rgb(c_border()))
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

        let preview = div()
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
                    .border_color(rgb(c_border()))
                    .child(icon(ICONS[form.icon].1, 18., accent)),
            )
            .child(
                div()
                    .v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child(if editing {
                                "Edit project"
                            } else {
                                "New project"
                            }),
                    )
                    .child(div().text_xs().text_color(rgb(c_muted())).truncate().child(
                        if editing {
                            "Change how this project looks in the sidebar"
                        } else {
                            "Pick a folder for Runo's agents to work in"
                        },
                    )),
            );

        let card = div()
            .id("project-card")
            .w(px(520.))
            .max_w(relative(0.94))
            .p_6()
            .v_flex()
            .gap_4()
            .rounded_2xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(preview)
            .child(div().v_flex().gap_1p5().child(label("Folder")).child(folder))
            .child(
                div()
                    .v_flex()
                    .gap_1p5()
                    .child(label("Name"))
                    .child(Input::new(&self.project_name).bordered(true).h(px(34.))),
            )
            .child(div().v_flex().gap_1p5().child(label("Color")).child(swatches))
            .child(div().v_flex().gap_1p5().child(label("Icon")).child(glyphs))
            .when(!editing, |el| {
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
                            if editing { "Save" } else { "Add project" },
                            true,
                        )
                        .when(!editing && path_text.is_none(), |el| el.opacity(0.5))
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
