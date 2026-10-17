//! The History tab: versions of the design, saved on their own while it changes and by
//! name, what changed since each, a screen compared side by side with how it is now, and
//! putting the design back as a version had it.

use super::figma::caption;
use super::*;
use crate::designdraw::notes;
use crate::designdraw::versions::{self, Changes, Version};
use gpui_kit::component::Sizable;

/// How often a version is saved on its own while the design changes.
const AUTOSAVE: Duration = Duration::from_secs(20 * 60);

/// A screen being compared with how a version had it.
pub(super) struct Compare {
    pub title: String,
    pub screen: String,
    pub name: String,
    /// The version's screen, once drawn; None while drawing, or when it didn't have it.
    pub old: Option<Shot>,
    pub drawn: bool,
}

impl DesignCanvas {
    pub(super) fn make_version_field(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let f = cx.new(|cx| InputState::new(window, cx).placeholder("Name this version"));
        subs.push(cx.subscribe_in(&f, window, |this, _, ev, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.save_version(window, cx);
            }
        }));
        f
    }

    fn who(&mut self) -> String {
        if self.author.is_empty() {
            self.author = notes::author(&self.root);
        }
        self.author.clone()
    }

    /// Save a version on its own when it's been a while since the last.
    pub(super) fn autosave(&mut self) {
        if self.version_due.is_some_and(|t| t.elapsed() < AUTOSAVE) {
            return;
        }
        self.version_due = Some(Instant::now());
        let who = self.who();
        if let Ok(Some(_)) = versions::save(&self.root, "", &who) {
            self.versions = versions::list(&self.root);
        }
    }

    fn save_version(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.f_version.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.say("Give the version a name.");
            cx.notify();
            return;
        }
        let who = self.who();
        match versions::save(&self.root, &name, &who) {
            Ok(_) => {
                self.say(format!("Saved the design as {name}."));
                self.f_version
                    .update(cx, |s, cx| s.set_value("", window, cx));
            }
            Err(e) => self.say(format!("Couldn't save the version: {e}")),
        }
        self.versions = versions::list(&self.root);
        cx.notify();
    }

    /// Show what changed since a version, or hide it.
    fn open_version(&mut self, id: String, cx: &mut Context<Self>) {
        if self.version_open.as_ref().is_some_and(|(v, _)| *v == id) {
            self.version_open = None;
        } else {
            let changes = versions::changes(&self.root, &id);
            self.version_open = Some((id, changes));
        }
        cx.notify();
    }

    fn name_version(&mut self, id: &str, cx: &mut Context<Self>) {
        let name = self.f_version.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.say("Type a name above first.");
        } else if let Err(e) = versions::rename(&self.root, id, &name) {
            self.say(e);
        }
        self.versions = versions::list(&self.root);
        cx.notify();
    }

    /// Put the design back as a version had it, saving how it is now first.
    fn restore_version(&mut self, v: Version, cx: &mut Context<Self>) {
        let who = self.who();
        let _ = versions::save(&self.root, &format!("Before restoring {}", v.title()), &who);
        if let Err(e) = versions::restore(&self.root, &v.id) {
            self.say(format!("Couldn't restore it: {e}"));
            cx.notify();
            return;
        }
        self.undo.clear();
        self.redo.clear();
        self.library = crate::designdraw::library::load(&self.root);
        self.versions = versions::list(&self.root);
        self.version_open = None;
        self.comparing = None;
        // Read the files again and draw what changed.
        self.check_files(cx);
        self.say(format!(
            "Restored {} from {}. How it was is saved as a version too.",
            v.title(),
            v.at
        ));
        cx.notify();
    }

    /// Compare a screen with how version `id` had it.
    fn compare(&mut self, v: &Version, screen: String, cx: &mut Context<Self>) {
        let file = versions::screen_file(&self.root, &v.id, &screen);
        let name = self
            .canvas
            .screen(&screen)
            .map_or_else(|| screen.clone(), |s| s.name.clone());
        let device = self
            .canvas
            .screen(&screen)
            .map(|s| s.device)
            .or_else(|| {
                let html = std::fs::read_to_string(&file).ok()?;
                Some(device_of(&html))
            })
            .unwrap_or(Device::Phone);
        self.drop_compare(cx);
        self.comparing = Some(Compare {
            title: v.title().to_string(),
            screen: screen.clone(),
            name,
            old: None,
            drawn: !file.exists(),
        });
        if !file.exists() {
            cx.notify();
            return;
        }
        self.run(
            cx,
            move |r| r.shoot_file(&file, device),
            move |this, out, cx| match this.comparing.as_mut() {
                Some(c) if c.screen == screen => {
                    c.old = out.ok();
                    c.drawn = true;
                    cx.notify();
                }
                _ => drop_shot(out.ok(), cx),
            },
        );
        cx.notify();
    }

    pub(super) fn drop_compare(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.comparing.take() {
            drop_shot(c.old, cx);
        }
    }

    // ---- What's drawn

    /// The left panel's History tab.
    pub(super) fn history_list(&self, cx: &mut Context<Self>) -> Div {
        let save = div()
            .px_2()
            .pt_2()
            .pb_2()
            .v_flex()
            .gap(px(6.))
            .border_b_1()
            .border_color(rgb(c_border()))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
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
                            .child(
                                Input::new(&self.f_version)
                                    .appearance(false)
                                    .small()
                                    .h(px(24.)),
                            ),
                    )
                    .child(
                        text_btn("design-version-save", "Save", None, false).on_click(
                            cx.listener(|this, _, window, cx| this.save_version(window, cx)),
                        ),
                    ),
            )
            .child(caption(
                "Versions save on their own as you work. Name one to keep it for good.",
            ));
        let mut list = div().v_flex().px_1().pb_3().child(save);
        if self.versions.is_empty() {
            return list.child(div().px_2().pt_2().child(caption("No versions yet.")));
        }
        for (i, v) in self.versions.iter().enumerate() {
            let open = self
                .version_open
                .as_ref()
                .filter(|(id, _)| *id == v.id)
                .map(|(_, c)| c);
            let id = v.id.clone();
            list = list.child(
                div()
                    .id(("design-version", i))
                    .px_2()
                    .py(px(6.))
                    .v_flex()
                    .gap(px(1.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .when(open.is_some(), |el| el.bg(tint(c_subtle())))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_version(id.clone(), cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().size(px(7.)).flex_none().rounded_full().bg(rgb(
                                if v.name.is_empty() {
                                    c_muted()
                                } else {
                                    c_blue()
                                },
                            )))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(if v.name.is_empty() {
                                        FontWeight::NORMAL
                                    } else {
                                        FontWeight::SEMIBOLD
                                    })
                                    .text_color(rgb(c_text()))
                                    .child(v.title().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(13.))
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(if v.who.is_empty() {
                                v.at.clone()
                            } else {
                                format!("{} · {}", v.at, v.who)
                            }),
                    ),
            );
            if let Some(c) = open {
                list = list.child(self.version_details(i, v, c, cx));
            }
        }
        list
    }

    /// What changed since a version, and what can be done with it.
    fn version_details(&self, i: usize, v: &Version, c: &Changes, cx: &mut Context<Self>) -> Div {
        let mut out = div().pl(px(21.)).pr_2().pb_2().v_flex().gap(px(4.));
        if c.none() {
            out = out.child(caption("The design is the same as this version."));
        }
        let name = |id: &str| {
            self.canvas
                .screen(id)
                .map_or_else(|| id.to_string(), |s| s.name.clone())
        };
        let groups = [
            ("Changed since", &c.changed),
            ("Added since", &c.added),
            ("Gone since", &c.removed),
        ];
        let mut n = 0;
        for (label, screens) in groups {
            if screens.is_empty() {
                continue;
            }
            out = out.child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgb(c_muted()))
                    .child(label),
            );
            for s in screens {
                n += 1;
                let (v2, s2) = (v.clone(), s.clone());
                out = out.child(
                    div()
                        .id(("design-version-screen", i * 1000 + n))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(c_blue()))
                        .tooltip(tip("Compare it with how it is now"))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.compare(&v2, s2.clone(), cx)),
                        )
                        .child(icon(IconName::Frame, 11., c_blue()))
                        .child(name(s)),
                );
            }
        }
        if c.styles {
            out = out.child(caption("Styles or components changed too."));
        }
        let (v2, id) = (v.clone(), v.id.clone());
        out.child(
            div()
                .pt_1()
                .flex()
                .gap(px(4.))
                .when(!c.none(), |el| {
                    el.child(
                        text_btn(("design-version-restore", i), "Restore", None, false)
                            .tooltip(tip("Put the design back as this version had it"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.restore_version(v2.clone(), cx)
                            })),
                    )
                })
                .child(
                    text_btn(("design-version-name", i), "Name it", None, false)
                        .tooltip(tip("Name this version with what's typed above"))
                        .on_click(cx.listener(move |this, _, _, cx| this.name_version(&id, cx))),
                ),
        )
    }

    /// A screen as a version had it beside how it is now, over the canvas.
    pub(super) fn compare_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.comparing.as_ref()?;
        let (vw, vh) = self.view_size();
        let now = self.shots.get(&c.screen);
        let device = self
            .canvas
            .screen(&c.screen)
            .map(|s| s.device)
            .unwrap_or(Device::Phone);
        let (w, h) = device.size();
        let scale = ((vh - 110.) / h).min((vw / 2. - 60.) / w).min(1.);
        let frame = |shot: Option<&Shot>, label: String, empty: &str| {
            let picture = match shot {
                Some(s) => div()
                    .w(px(w * scale))
                    .h(px(h * scale))
                    .rounded(px(10.))
                    .overflow_hidden()
                    .bg(rgb(0xffffff))
                    .shadow_lg()
                    .child(
                        img(ImageSource::Render(s.image.clone()))
                            .w(px(w * scale))
                            .h(px(s.size.1 * scale)),
                    ),
                None => div()
                    .w(px(w * scale))
                    .h(px(h * scale))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(c_border()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(caption(empty.to_string())),
            };
            div()
                .v_flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_secondary()))
                        .child(label),
                )
                .child(picture)
        };
        let old_empty = if c.drawn {
            "Not in this version"
        } else {
            "Drawing…"
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .v_flex()
                .bg(rgb(c_canvas()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(40.))
                        .w_full()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .text_color(rgb(c_text()))
                                .child(format!("{}  ·  {} and now", c.name, c.title)),
                        )
                        .child(
                            icon_btn(
                                "design-compare-close",
                                IconName::Close,
                                "Close (Esc)",
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.drop_compare(cx);
                                cx.notify();
                            })),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(40.))
                        .child(frame(c.old.as_ref(), c.title.clone(), old_empty))
                        .child(frame(now, "Now".into(), "Not on the design now")),
                )
                .into_any_element(),
        )
    }
}
