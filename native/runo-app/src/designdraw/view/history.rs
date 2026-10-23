//! The History tab: versions of the design, saved on their own while it changes and by
//! name, what changed since each, a screen compared side by side with how it is now, and
//! putting the design back as a version had it.

use super::chrome::segment as mode;
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
    /// Comparing what an AI change did to the screen, with the screen as it was before.
    pub ai: bool,
    /// One picture over the other with a line between to slide, rather than side by side.
    pub slide: bool,
    /// Where the line is, from the left, as a share of the width.
    pub split: f32,
    /// Where the slider's picture was drawn, to place the line under the pointer.
    pub frame: Rc<Cell<Bounds<Pixels>>>,
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

    pub(super) fn who(&mut self) -> String {
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
        // A design as the last commit has it can be had back from git; a version of it
        // would only show up as a change.
        let dir = self.root.to_string_lossy();
        if crate::git::is_repo(&dir) && crate::git::changed_under(&dir, FOLDER).is_empty() {
            return;
        }
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
        self.compare_file(v.title().to_string(), &v.id, screen, false, cx);
    }

    /// Compare a screen with how it was before the AI change waiting for Keep or Undo.
    pub(super) fn compare_ai(&mut self, screen: String, cx: &mut Context<Self>) {
        let Some(v) = self.ai.as_ref().and_then(|ai| ai.version.clone()) else {
            return;
        };
        self.compare_file("Before AI".into(), &v, screen, true, cx);
    }

    /// Compare a screen with its file in version `version`, called `title`.
    fn compare_file(
        &mut self,
        title: String,
        version: &str,
        screen: String,
        ai: bool,
        cx: &mut Context<Self>,
    ) {
        let file = versions::screen_file(&self.root, version, &screen);
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
            title,
            screen: screen.clone(),
            name,
            old: None,
            drawn: !file.exists(),
            ai,
            slide: true,
            split: 0.5,
            frame: Rc::new(Cell::new(Bounds::default())),
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
            .border_color(divider())
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
                    .border_color(divider())
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
        let body = if c.slide && c.old.is_some() && now.is_some() {
            self.slider(c, (w * scale, h * scale), scale, cx)
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(40.))
                .child(frame(c.old.as_ref(), c.title.clone(), old_empty))
                .child(frame(now, "Now".into(), "Not on the design now"))
                .into_any_element()
        };
        let both = c.old.is_some() && now.is_some();
        let pending = c.ai && self.ai.as_ref().is_some_and(|ai| !ai.busy());
        let screen = c.screen.clone();
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
                        .pl_3()
                        // Clear of the Chats button over the canvas's corner.
                        .pr(px(112.))
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
                        .when(both, |d| {
                            d.child(
                                div()
                                    .p(px(2.))
                                    .flex()
                                    .gap(px(2.))
                                    .rounded(px(8.))
                                    .bg(tint(c_subtle()))
                                    .child(
                                        mode("design-compare-slide", "Slider", c.slide).on_click(
                                            cx.listener(|this, _, _, cx| this.set_slide(true, cx)),
                                        ),
                                    )
                                    .child(
                                        mode("design-compare-side", "Side by side", !c.slide)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_slide(false, cx)
                                            })),
                                    ),
                            )
                        })
                        .when(pending, |d| {
                            d.child(
                                text_btn(
                                    "design-compare-put-back",
                                    "Put this screen back",
                                    Some(IconName::Undo2),
                                    false,
                                )
                                .tooltip(tip(
                                    "Undo AI's change to this screen only, keeping the rest",
                                ))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.drop_compare(cx);
                                        this.reject_screen(&screen, cx);
                                    },
                                )),
                            )
                        })
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
                        .child(body),
                )
                .into_any_element(),
        )
    }

    fn set_slide(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(c) = self.comparing.as_mut() {
            c.slide = on;
        }
        cx.notify();
    }

    /// The screen as it was over how it is now, cut at a line that follows the pointer:
    /// left of it is before, right of it is now.
    fn slider(
        &self,
        c: &Compare,
        (w, h): (f32, f32),
        scale: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(old), Some(now)) = (c.old.as_ref(), self.shots.get(&c.screen)) else {
            return div().into_any_element();
        };
        let at = c.split.clamp(0., 1.) * w;
        let frame = c.frame.clone();
        let picture = |shot: &Shot| {
            img(ImageSource::Render(shot.image.clone()))
                .absolute()
                .top_0()
                .left_0()
                .w(px(w))
                .h(px(shot.size.1 * scale))
        };
        let tag = |text: &str, left: bool| {
            div()
                .absolute()
                .top(px(8.))
                .when(left, |d| d.left(px(8.)))
                .when(!left, |d| d.right(px(8.)))
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .bg(col(0x000000, 0.6))
                .text_size(px(11.))
                .text_color(rgb(0xffffff))
                .child(text.to_string())
        };
        div()
            .id("design-compare-slider")
            .relative()
            .w(px(w))
            .h(px(h))
            .rounded(px(10.))
            .overflow_hidden()
            .bg(rgb(0xffffff))
            .shadow_lg()
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                let b = frame.get();
                let x = f32::from(e.position.x - b.origin.x);
                if let Some(c) = this.comparing.as_mut() {
                    c.split = (x / w).clamp(0., 1.);
                    cx.notify();
                }
            }))
            .child({
                let frame = c.frame.clone();
                canvas(move |b, _, _| frame.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full()
            })
            .child(picture(now))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(at))
                    .h(px(h))
                    .overflow_hidden()
                    .child(picture(old)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(at - 1.))
                    .w(px(2.))
                    .h(px(h))
                    .bg(rgb(0xffffff))
                    .shadow_md(),
            )
            .child(
                div()
                    .absolute()
                    .top(px(h / 2. - 14.))
                    .left(px(at - 14.))
                    .size(px(28.))
                    .rounded_full()
                    .bg(rgb(0xffffff))
                    .shadow_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::ChevronsLeftRight, 14., c_text())),
            )
            .child(tag(&c.title, true))
            .child(tag("Now", false))
            .into_any_element()
    }
}
