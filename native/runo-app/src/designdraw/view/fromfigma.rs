//! Import from Figma, in the Add a screen panel: paste a link to a frame or a page, and
//! its frames come in as screens next to the others.

use super::figma::caption;
use super::*;
use crate::designdraw::figmafile;
use crate::settings::{self, AppSettings};
use gpui_kit::component::Sizable;

impl DesignCanvas {
    /// The link box, and the token box for when Settings has none.
    pub(super) fn make_figma_fields(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        let mut make = |masked: bool, hint: &'static str| {
            let f = cx.new(|cx| InputState::new(window, cx).placeholder(hint).masked(masked));
            subs.push(cx.subscribe_in(&f, window, |this, _, ev, _, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.import_figma(cx);
                }
            }));
            f
        };
        let link = make(false, "Link to a Figma frame or page");
        let token = make(true, "Figma access token");
        (link, token)
    }

    fn figma_token(cx: &App) -> String {
        cx.global::<AppSettings>().figma_token.trim().to_string()
    }

    /// The token from Settings, or the one just typed here, which is saved to Settings.
    fn take_figma_token(&self, cx: &mut Context<Self>) -> String {
        let saved = Self::figma_token(cx);
        if !saved.is_empty() {
            return saved;
        }
        let typed = self.f_figma_token.read(cx).value().trim().to_string();
        if !typed.is_empty() {
            let v = typed.clone();
            settings::update(cx, move |s| s.figma_token = v);
        }
        typed
    }

    /// Bring in the frames the link points at, off the UI thread.
    pub(super) fn import_figma(&mut self, cx: &mut Context<Self>) {
        if self.figma_busy {
            return;
        }
        let url = self.f_figma.read(cx).value().trim().to_string();
        if url.is_empty() {
            self.say("Paste a link to a Figma frame or page first.");
            cx.notify();
            return;
        }
        let token = self.take_figma_token(cx);
        if token.is_empty() {
            self.say("Add a Figma access token first.");
            cx.notify();
            return;
        }
        if let Err(e) = seed(&self.root) {
            self.say(format!("Couldn't start the design: {e}"));
            return;
        }
        self.figma_busy = true;
        cx.notify();
        let design = folder(&self.root);
        cx.spawn(async move |this, cx| {
            let got = cx
                .background_spawn(async move { figmafile::import(&url, &token, &design) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.figma_busy = false;
                match got {
                    Ok(frames) => this.add_imported(frames, cx),
                    Err(e) => this.say(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Write the imported frames as screens, each on the page for its device.
    fn add_imported(&mut self, frames: Vec<figmafile::Imported>, cx: &mut Context<Self>) {
        let mut ids = Vec::new();
        for f in frames {
            let id = self.canvas.new_id(&f.name);
            let html = page(&f.name, f.device, &f.body);
            if std::fs::write(screen_path(&self.root, &id), html).is_err() {
                continue;
            }
            let page = self.page_for(f.device);
            let (x, y) = self
                .canvas
                .next_spot_on(page.as_deref().unwrap_or(f.device.label()));
            self.canvas.screens.push(Screen {
                id: id.clone(),
                name: f.name,
                device: f.device,
                x,
                y,
                page,
                ..Default::default()
            });
            ids.push(id);
        }
        let Some(first) = ids.first().cloned() else {
            self.say("Couldn't write the imported screens.");
            return;
        };
        self.tokens = std::fs::read_to_string(folder(&self.root).join(TOKENS)).unwrap_or_default();
        self.save_canvas();
        self.new_open = false;
        for id in &ids {
            self.draw(id, cx);
        }
        self.select_screen(Some(first.clone()));
        // Bring the first one into view.
        if let Some(s) = self.canvas.screen(&first).cloned() {
            let (vw, vh) = self.view_size();
            let (w, h) = s.device.size();
            self.pan = (
                vw / 2. - (s.x + w / 2.) * self.zoom,
                vh / 2. - (s.y + h / 2.) * self.zoom,
            );
        }
        self.say(match ids.len() {
            1 => "Imported 1 screen from Figma.".to_string(),
            n => format!("Imported {n} screens from Figma."),
        });
    }

    /// The frame tool's menu item for importing: it shows the form in the Add a screen
    /// panel on the right.
    pub(super) fn figma_menu_item(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("design-figma-item")
            .mx_1()
            .mb_1()
            .px_2()
            .py(px(6.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())))
            .child(icon(IconName::Download, 14., c_secondary()))
            .child(
                div()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_text()))
                            .child("Import from Figma"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child("Bring in the frames a Figma link points at"),
                    ),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.new_open = false;
                this.select_screen(None);
                this.panel = Panel::Design;
                this.say("Paste a Figma link under Import from Figma, on the right.");
                cx.notify();
            }))
    }

    /// The link, a token when Settings has none, and Import.
    pub(super) fn figma_import_form(&self, cx: &mut Context<Self>) -> Div {
        let has_token = !Self::figma_token(cx).is_empty();
        let field = |f: &Entity<InputState>| {
            div()
                .h(px(30.))
                .pl(px(8.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .bg(tint(c_subtle()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(f).appearance(false).small().h(px(28.))),
                )
        };
        let mut out = div().v_flex().gap(px(6.)).child(field(&self.f_figma));
        if !has_token {
            out = out.child(field(&self.f_figma_token)).child(caption(
                "Make a personal access token in Figma under Settings > Security, with read access to files. Runo keeps it in its settings on this computer.",
            ));
        }
        out.child(
            div().flex().child(
                text_btn(
                    "design-figma-import",
                    if self.figma_busy {
                        "Importing…"
                    } else {
                        "Import frames"
                    },
                    Some(IconName::Download),
                    true,
                )
                .when(!self.figma_busy, |b| {
                    b.on_click(cx.listener(|this, _, _, cx| this.import_figma(cx)))
                }),
            ),
        )
        .child(caption(
            "Auto layout comes in as flex, so it stays editable. Pictures and icons come along.",
        ))
    }
}
