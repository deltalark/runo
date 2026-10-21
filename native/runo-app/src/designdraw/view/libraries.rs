//! The Assets tab's libraries: publishing this project's components for other projects,
//! and adding, updating or removing the libraries other projects published.

use super::*;
use crate::designdraw::library;
use crate::designdraw::shared::{self, Library};
use gpui_kit::component::Sizable;

impl DesignCanvas {
    pub(super) fn make_library_field(
        root: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let own = shared::published()
            .into_iter()
            .find(|l| l.from == shared::origin(root))
            .map(|l| l.name);
        let name = own.unwrap_or_else(|| {
            let stem = root
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Library");
            let mut c = stem.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        });
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Library name")
                .default_value(name)
        })
    }

    /// Read again which libraries are published and which this project has.
    pub(super) fn refresh_libraries(&mut self) {
        self.published = shared::published();
        self.added_libs = shared::added(&self.root);
    }

    /// This project's own library, when it published one.
    fn own_library(&self) -> Option<&Library> {
        let me = shared::origin(&self.root);
        self.published.iter().find(|l| l.from == me)
    }

    fn publish_library(&mut self, cx: &mut Context<Self>) {
        let name = self.f_lib.read(cx).value().trim().to_string();
        if !self
            .library
            .iter()
            .any(|c| library::library_of(&c.id).is_none())
        {
            self.say("Make a component first: a library shares this project's components.");
            cx.notify();
            return;
        }
        match shared::publish(&self.root, &name) {
            Ok(lib) => {
                let what = if lib.version == 1 {
                    format!(
                        "Published {} as a library. Other projects can add it from Assets.",
                        lib.name
                    )
                } else {
                    format!(
                        "Published version {} of {}. Projects that use it can update.",
                        lib.version, lib.name
                    )
                };
                self.say(what);
            }
            Err(e) => self.say(e),
        }
        self.refresh_libraries();
        cx.notify();
    }

    /// Add library `id`, or bring it up to its newest version and its instances with it.
    fn add_library(&mut self, id: &str, cx: &mut Context<Self>) {
        let dir = library::folder(&self.root).join(id);
        let before: HashMap<PathBuf, Vec<u8>> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Some((e.path(), std::fs::read(e.path()).ok()?)))
            .collect();
        let had = self.added_libs.iter().any(|l| l.id == id);
        match shared::add(&self.root, id) {
            Ok(lib) => {
                if had {
                    // Its styles changed too: draw every screen again.
                    self.check_files(cx);
                    self.say(format!(
                        "Updated {} to version {}, and its instances with it.",
                        lib.name, lib.version
                    ));
                    self.sync_changed_components(&before, cx);
                } else {
                    self.note_writes();
                    self.library = library::load(&self.root);
                    self.say(format!(
                        "Added {}. Its components are in Assets, ready for any screen.",
                        lib.name
                    ));
                }
            }
            Err(e) => self.say(e),
        }
        self.refresh_libraries();
        cx.notify();
    }

    fn remove_library(&mut self, id: &str, cx: &mut Context<Self>) {
        let name = self
            .added_libs
            .iter()
            .find(|l| l.id == id)
            .map_or(id.to_string(), |l| l.name.clone());
        match shared::remove(&self.root, id) {
            Ok(()) => {
                self.note_writes();
                self.library = library::load(&self.root);
                self.say(format!(
                    "Removed {name}. Its instances stay on the screens as they are."
                ));
            }
            Err(e) => self.say(format!("Couldn't remove {name}: {e}")),
        }
        self.refresh_libraries();
        cx.notify();
    }

    /// Before an instance of library `lib` goes on the selected screen, link the library's
    /// styles there.
    pub(super) fn use_library_sheet(&mut self, lib: &str, cx: &mut Context<Self>) {
        let href = shared::sheet_href(lib);
        self.edit(dom::call("useSheet", &[json!(href)]), After::Same, cx);
    }

    /// Publish this project's library, and the others to add or update.
    pub(super) fn libraries_section(&self, cx: &mut Context<Self>) -> Div {
        let own = self.own_library();
        let mut out = div()
            .v_flex()
            .gap(px(6.))
            .px_2()
            .pt_2()
            .pb_2()
            .border_b_1()
            .border_color(divider())
            .child(components::asset_heading("Libraries").px_0().pt_0())
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(26.))
                            .px(px(6.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .bg(tint(c_subtle()))
                            .child(Input::new(&self.f_lib).appearance(false).small().h(px(24.))),
                    )
                    .child(
                        text_btn(
                            "design-lib-publish",
                            if own.is_some() {
                                "Publish update"
                            } else {
                                "Publish"
                            },
                            Some(IconName::Upload),
                            false,
                        )
                        .tooltip(tip(
                            "Share this project's components and styles with your other projects",
                        ))
                        .on_click(cx.listener(|this, _, _, cx| this.publish_library(cx))),
                    ),
            );
        if let Some(own) = own {
            out = out.child(figma::caption(format!(
                "Published as {}, version {}, with {} components.",
                own.name, own.version, own.components
            )));
        }
        let me = shared::origin(&self.root);
        let others: Vec<&Library> = self.published.iter().filter(|l| l.from != me).collect();
        if others.is_empty() {
            return out.child(figma::caption(
                "Libraries other projects publish show here, to add their components.",
            ));
        }
        for (i, lib) in others.into_iter().enumerate() {
            let have = self.added_libs.iter().find(|a| a.id == lib.id);
            let (id, id2) = (lib.id.clone(), lib.id.clone());
            let action = match have {
                None => Some(("Add", false)),
                Some(a) if a.version < lib.version => Some(("Update", true)),
                Some(_) => None,
            };
            let note = match have {
                Some(a) if a.version < lib.version => {
                    format!("Version {} is ready (have {})", lib.version, a.version)
                }
                Some(a) => format!("Added, version {}", a.version),
                None if lib.components == 1 => "1 component".to_string(),
                None => format!("{} components", lib.components),
            };
            out = out.child(
                div()
                    .id(("design-lib", i))
                    .group("design-lib")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon(IconName::BookOpen, 12., components::PURPLE))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(c_text()))
                                    .child(lib.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(rgb(if action == Some(("Update", true)) {
                                        c_blue()
                                    } else {
                                        c_muted()
                                    }))
                                    .child(note),
                            ),
                    )
                    .children(action.map(|(label, accent)| {
                        text_btn(("design-lib-add", i), label, None, accent)
                            .on_click(cx.listener(move |this, _, _, cx| this.add_library(&id, cx)))
                    }))
                    .when(have.is_some(), |row| {
                        row.child(
                            figma::mini(
                                ("design-lib-remove", i),
                                IconName::Close,
                                "Remove the library from this project",
                                false,
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_library(&id2, cx)),
                            ),
                        )
                    }),
            );
        }
        out
    }
}
