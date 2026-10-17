//! Building by hand: the Insert section's parts library, icons to put in and swap, pictures
//! from files, and overlays (sheets, dialogs) a tap opens and closes.

use super::figma::{caption, sec};
use super::*;
use crate::designdraw::parts::PARTS;
use gpui_kit::component::Sizable;
use std::sync::OnceLock;

/// The kit's icons by name, as `icons.css` has them.
pub(super) fn kit_icons() -> &'static [String] {
    static ICONS: OnceLock<Vec<String>> = OnceLock::new();
    ICONS.get_or_init(|| {
        ICONS_CSS
            .lines()
            .filter_map(|l| l.strip_prefix(".i-")?.split_once(' ').map(|(n, _)| n))
            .map(String::from)
            .collect()
    })
}

/// Whether an element shows a picture: an image, or a box that stands in for one.
fn takes_picture(el: &Element) -> bool {
    el.tag == "img"
        || el
            .label
            .split('.')
            .skip(1)
            .any(|c| matches!(c, "image" | "hero" | "avatar" | "illustration"))
}

/// What a picture chosen from a file goes into.
#[derive(Clone, Copy)]
enum PictureFor {
    /// A new picture on the screen.
    New,
    /// The picked element: its source, or its fill.
    Picked,
}

impl DesignCanvas {
    pub(super) fn make_icon_field(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let f = cx.new(|cx| InputState::new(window, cx).placeholder("Find an icon"));
        subs.push(cx.subscribe_in(&f, window, |this, state, ev, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.icon_query = state.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        }));
        f
    }

    /// Put `html` where that kind of part goes on the picked screen, and pick it.
    fn place(&mut self, html: &str, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let js = dom::call("place", &[json!(html)]);
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.edit_giving(&id, device, &js),
            move |this, out, cx| match out {
                Ok(Some((before, shot, path))) => {
                    let after = match serde_json::from_value(path) {
                        Ok(p) => After::Path(p),
                        Err(_) => After::Clear,
                    };
                    this.landed(id2, before, shot, after, cx)
                }
                Ok(None) => {}
                Err(e) => this.say(e),
            },
        );
    }

    /// Put a part on the picked screen: next to or inside what's picked, else where that
    /// kind of part goes (bars on top, tab bars and overlays over the screen).
    pub(super) fn insert_part(&mut self, html: &str, cx: &mut Context<Self>) {
        let frame = [
            "<nav class=\"tabbar",
            "<div class=\"overlay",
            "<button class=\"fab",
            "<header class=\"bar",
        ]
        .iter()
        .any(|k| html.starts_with(k));
        if self.element.is_some() && !frame {
            self.insert(html, cx);
        } else if self.screen.is_some() {
            self.place(html, cx);
        } else {
            self.say("Pick a screen first, then insert onto it.");
            cx.notify();
        }
    }

    /// The Insert section's body: every part, by kind.
    pub(super) fn parts_library(&self, cx: &mut Context<Self>) -> Div {
        let mut out = div().v_flex().gap(px(8.));
        let mut n = 0usize;
        for (kind, parts) in PARTS.iter() {
            let mut row = div().flex().flex_wrap().gap(px(4.));
            for (label, html) in parts.iter() {
                let html: &'static str = html;
                row = row.child(
                    chip_btn(("design-part", n), *label, false)
                        .on_click(cx.listener(move |this, _, _, cx| this.insert_part(html, cx))),
                );
                n += 1;
            }
            out = out.child(
                div()
                    .v_flex()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(c_muted()))
                            .child(*kind),
                    )
                    .child(row),
            );
        }
        out
    }

    /// The Icon section, for an icon or something holding one: the kit's icons to swap it
    /// for.
    pub(super) fn icon_section(&self, el: &Element, cx: &mut Context<Self>) -> Option<Div> {
        let now = el.icon.clone()?;
        let q = &self.icon_query;
        let mut grid = div().flex().flex_wrap().gap(px(2.));
        for (i, name) in kit_icons().iter().enumerate() {
            if !q.is_empty() && !name.contains(q.as_str()) {
                continue;
            }
            let Some(glyph) = crate::archdraw::view::icon_named(name) else {
                continue;
            };
            let name2 = name.clone();
            grid = grid.child(
                div()
                    .id(("design-icon", i))
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .when(*name == now, |el| el.bg(tint(c_selected())))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .tooltip(tip(name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.op("icon", &[json!(name2)], After::Same, cx)
                    }))
                    .child(icon(
                        glyph,
                        14.,
                        if *name == now {
                            c_blue()
                        } else {
                            c_secondary()
                        },
                    )),
            );
        }
        Some(
            sec("Icon", None)
                .child(Input::new(&self.f_icon).small().prefix(icon(
                    IconName::Search,
                    12.,
                    c_muted(),
                )))
                .child(
                    div()
                        .id("design-icon-grid")
                        .max_h(px(196.))
                        .overflow_y_scroll()
                        .child(grid),
                ),
        )
    }

    /// The Picture section, for an image, a hero, an avatar or a picture: a file to show
    /// in it.
    pub(super) fn picture_section(&self, el: &Element, cx: &mut Context<Self>) -> Option<Div> {
        if !takes_picture(el) {
            return None;
        }
        let has = el.src.is_some() || el.inline.contains_key("background-image");
        let mut row = div().flex().gap_1().child(
            text_btn(
                "design-picture-pick",
                if has {
                    "Replace picture"
                } else {
                    "Choose picture"
                },
                Some(IconName::Image),
                false,
            )
            .on_click(cx.listener(|this, _, _, cx| this.choose_picture(PictureFor::Picked, cx))),
        );
        if has {
            row = row.child(
                text_btn("design-picture-clear", "Remove", None, false).on_click(
                    cx.listener(|this, _, _, cx| this.op("picture", &[json!("")], After::Same, cx)),
                ),
            );
        }
        Some(sec("Picture", None).child(row))
    }

    /// The Image tool: a picture from a file, onto the screen.
    pub(super) fn image_tool(&mut self, cx: &mut Context<Self>) {
        self.choose_picture(PictureFor::New, cx);
    }

    fn choose_picture(&mut self, to: PictureFor, cx: &mut Context<Self>) {
        if self.screen.is_none() {
            self.say("Pick a screen first, then add a picture to it.");
            cx.notify();
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a picture".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().find(|p| crate::turns::is_image(p)) else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.use_picture(&path, to, cx));
        })
        .detach();
    }

    /// Pictures dropped on the canvas: the first fills what's picked when it takes a
    /// picture, else each goes onto the picked screen.
    pub(super) fn drop_pictures(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        let pictures: Vec<_> = paths.iter().filter(|p| crate::turns::is_image(p)).collect();
        let takes = self.element.as_ref().is_some_and(takes_picture);
        match pictures.as_slice() {
            [] => {}
            [one, ..] if takes => self.use_picture(one, PictureFor::Picked, cx),
            all => {
                for p in all {
                    self.use_picture(p, PictureFor::New, cx);
                }
            }
        }
    }

    fn use_picture(&mut self, path: &std::path::Path, to: PictureFor, cx: &mut Context<Self>) {
        let url = match add_picture(&self.root, path) {
            Ok(url) => url,
            Err(e) => {
                self.say(e);
                cx.notify();
                return;
            }
        };
        match to {
            PictureFor::Picked if self.element.is_some() => {
                self.op("picture", &[json!(url)], After::Same, cx)
            }
            _ => {
                let html = format!(
                    "<div class=\"image\" style=\"background-image: url(&quot;{url}&quot;); \
                     background-size: cover; background-position: center\"></div>"
                );
                self.insert_part(&html, cx);
            }
        }
    }

    // ---- Overlays

    /// The overlays on the picked screen (sheets, dialogs), by layer.
    fn overlays(&self) -> Vec<&Layer> {
        self.layers
            .iter()
            .filter(|l| l.label.split('.').skip(1).any(|c| c == "overlay"))
            .collect()
    }

    /// Prototype choices for an element: the overlays a tap on it opens or closes.
    pub(super) fn overlay_section(&self, el: &Element, cx: &mut Context<Self>) -> Option<Div> {
        let overlays = self.overlays();
        if overlays.is_empty() || el.label.split('.').skip(1).any(|c| c == "overlay") {
            return None;
        }
        let mut row = div().flex().flex_wrap().gap(px(4.));
        row = row.child(
            chip_btn(("design-show", 0usize), "Nothing", el.show.is_none()).on_click(cx.listener(
                |this, _, _, cx| this.op("attr", &[json!("data-show"), json!("")], After::Same, cx),
            )),
        );
        for (i, o) in overlays.iter().enumerate() {
            let inside = el.path.starts_with(&o.path);
            let inner = self
                .layers
                .iter()
                .find(|l| l.path.starts_with(&o.path) && l.depth == o.depth + 1)
                .map(|l| l.label.as_str())
                .unwrap_or_default();
            let title = match o.name.as_deref() {
                Some(n) => n.to_string(),
                None if inner.contains(".sheet") => "sheet".into(),
                None if inner.contains(".modal") => "dialog".into(),
                None => Self::layer_title(o),
            };
            let label = if inside {
                format!("Close {title}")
            } else {
                format!("Open {title}")
            };
            let on = el
                .show
                .as_deref()
                .is_some_and(|s| o.id.as_deref().is_some_and(|id| s == format!("#{id}")));
            let target = o.path.clone();
            row = row.child(
                chip_btn(("design-show", i + 1), label, on).on_click(cx.listener(
                    move |this, _, _, cx| this.op("showTarget", &[json!(target)], After::Same, cx),
                )),
            );
        }
        Some(
            sec("On tap, also", None)
                .child(caption(
                    "Open a sheet or dialog on this screen, or close the one it's in. It starts hidden; Present shows it on tap.",
                ))
                .child(row),
        )
    }
}
