//! Handing the design off to be built, from the top of the Code tab: what's changed since
//! the last handoff, the stacks it's built with, and the Hand off button, which gives every
//! layer a lasting id, saves a version, and writes the spec and the tokens.

use super::figma::caption;
use super::*;
use crate::designdraw::spec::{self, Handoff, Mobile};
use crate::designdraw::versions::{self, Changes};

impl DesignCanvas {
    /// Read again the handoffs made and the project's stacks.
    pub(super) fn refresh_handoffs(&mut self) {
        self.handoffs = spec::list(&self.root);
        self.handoff_settings = spec::settings(&self.root);
    }

    /// What changed since the last handoff, worked out again only when files changed.
    fn pending(&self) -> Option<Changes> {
        let last = self.handoffs.first()?;
        let key = (self.files_changed, last.n);
        let mut cache = self.handoff_pending.borrow_mut();
        if cache.as_ref().map(|(k, _)| *k) != Some(key) {
            *cache = Some((key, versions::changes(&self.root, &last.version)));
        }
        cache.as_ref().map(|(_, c)| c.clone())
    }

    fn set_mobile(&mut self, mobile: Mobile, cx: &mut Context<Self>) {
        self.handoff_settings.mobile = mobile;
        if let Err(e) = spec::save_settings(&self.root, &self.handoff_settings) {
            self.say(format!("Couldn't save the stack: {e}"));
        }
        self.note_writes();
        cx.notify();
    }

    fn hand_off(&mut self, cx: &mut Context<Self>) {
        if self.handing_off {
            return;
        }
        if self.canvas.screens.is_empty() {
            self.say("Add a screen first: a handoff builds the design's screens.");
            cx.notify();
            return;
        }
        self.handing_off = true;
        let screens: Vec<(String, Device)> = self
            .canvas
            .screens
            .iter()
            .map(|s| (s.id.clone(), s.device))
            .collect();
        let comps: serde_json::Map<String, serde_json::Value> = self
            .library
            .iter()
            .map(|c| (c.id.clone(), serde_json::Value::Object(c.map())))
            .collect();
        let tree_js = dom::call(
            "codeTree",
            &[json!(Vec::<usize>::new()), serde_json::Value::Object(comps)],
        );
        let ids_js = dom::call("ensureIds", &[]);
        self.say("Handing off: reading every screen…");
        self.run(
            cx,
            move |r| {
                let mut out = Vec::new();
                for (id, device) in screens {
                    // Ids first, so the layout read below carries them.
                    let shot = match r.edit(&id, device, &ids_js) {
                        Ok(Some((_, shot))) => Ok(shot),
                        _ => r.shoot(&id, device),
                    };
                    let tree = r.eval(&id, device, &tree_js).ok().filter(|v| !v.is_null());
                    let png = shot.as_ref().ok().and_then(png_of);
                    out.push((
                        spec::Read {
                            id: id.clone(),
                            tree,
                            png,
                        },
                        shot.ok(),
                    ));
                }
                out
            },
            |this, out, cx| {
                let mut read = Vec::new();
                for (r, shot) in out {
                    if let Some(shot) = shot {
                        drop_shot(this.shots.insert(r.id.clone(), shot), cx);
                    }
                    read.push(r);
                }
                this.handing_off = false;
                this.finish_handoff(read, cx);
            },
        );
        cx.notify();
    }

    fn finish_handoff(&mut self, read: Vec<spec::Read>, cx: &mut Context<Self>) {
        self.note_writes();
        self.refresh_handoffs();
        let since = self.handoffs.first().cloned();
        let changes = since
            .as_ref()
            .map(|s| versions::changes(&self.root, &s.version));
        let n = since.as_ref().map_or(1, |s| s.n + 1);
        let title = format!("Handoff {n}");
        let who = self.who();
        let version = match versions::save(&self.root, &title, &who) {
            Ok(Some(v)) => v.id,
            Ok(None) => versions::list(&self.root)
                .first()
                .map(|v| v.id.clone())
                .unwrap_or_default(),
            Err(e) => {
                self.say(format!("Couldn't save a version for the handoff: {e}"));
                cx.notify();
                return;
            }
        };
        let h = Handoff {
            n,
            version,
            at: chrono::Local::now().format("%b %-d, %H:%M").to_string(),
            who,
            mobile: self.handoff_settings.mobile,
            to_build: 0,
        };
        match spec::write(
            &self.root,
            &h,
            since.as_ref(),
            changes.as_ref(),
            &self.canvas,
            read,
        ) {
            Ok(_) => {
                self.refresh_handoffs();
                let count = self.handoffs.first().map_or(0, |h| h.to_build);
                self.say(match count {
                    0 => format!("{title} is saved. Nothing changed since the last one."),
                    1 => format!("{title} is ready: 1 screen to build, in design/handoff/{n}/."),
                    c => format!("{title} is ready: {c} screens to build, in design/handoff/{n}/."),
                });
            }
            Err(e) => self.say(format!("Couldn't write the handoff: {e}")),
        }
        self.versions = versions::list(&self.root);
        self.note_writes();
        cx.notify();
    }

    /// The Hand off section, at the top of the Code tab.
    pub(super) fn handoff_section(&self, cx: &mut Context<Self>) -> Div {
        let last = self.handoffs.first();
        let status = match (last, self.pending()) {
            (None, _) => format!(
                "Not handed off yet. The first handoff asks for all {} screens.",
                self.canvas.screens.len()
            ),
            (Some(h), Some(c)) if c.none() => {
                format!("{} on {}. Nothing changed since.", h.title(), h.at)
            }
            (Some(h), Some(c)) => {
                let name = |id: &String| {
                    self.canvas
                        .screen(id)
                        .map_or_else(|| id.clone(), |s| s.name.clone())
                };
                let mut parts = Vec::new();
                if !c.changed.is_empty() {
                    let names: Vec<String> = c.changed.iter().map(name).collect();
                    parts.push(format!("changed {}", names.join(", ")));
                }
                if !c.added.is_empty() {
                    let names: Vec<String> = c.added.iter().map(name).collect();
                    parts.push(format!("added {}", names.join(", ")));
                }
                if !c.removed.is_empty() {
                    parts.push(format!("removed {}", c.removed.join(", ")));
                }
                if c.styles && parts.is_empty() {
                    parts.push("changed styles or components".into());
                }
                format!("Since {} on {}: {}.", h.title(), h.at, parts.join("; "))
            }
            (Some(h), None) => format!("{} on {}.", h.title(), h.at),
        };
        let mut mobile = div().flex().flex_wrap().gap(px(4.));
        for (i, m) in Mobile::ALL.into_iter().enumerate() {
            mobile = mobile.child(
                chip_btn(
                    ("design-handoff-mobile", i),
                    m.label(),
                    self.handoff_settings.mobile == m,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_mobile(m, cx))),
            );
        }
        let label = |t: &'static str| {
            div()
                .w(px(52.))
                .flex_none()
                .text_size(px(11.))
                .text_color(rgb(c_muted()))
                .child(t)
        };
        div()
            .v_flex()
            .gap(px(8.))
            .pt_3()
            .pb_3()
            .border_b_1()
            .border_color(rgb(c_border()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child("Hand off"),
                    )
                    .child(
                        text_btn(
                            "design-handoff",
                            if self.handing_off {
                                "Handing off…"
                            } else {
                                "Hand off"
                            },
                            Some(IconName::Upload),
                            true,
                        )
                        .tooltip(tip(
                            "Save a version and write what to build, with the tokens, to design/handoff/",
                        ))
                        .on_click(cx.listener(|this, _, _, cx| this.hand_off(cx))),
                    ),
            )
            .child(caption(status))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(label("Web"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(c_text()))
                            .child(spec::WEB_LABEL),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .child(label("Mobile").pt(px(4.)))
                    .child(mobile),
            )
    }
}

/// A screen's picture as a PNG, at the density it was drawn.
fn png_of(shot: &Shot) -> Option<Vec<u8>> {
    let size = shot.image.size(0);
    let (w, h) = (size.width.0 as u32, size.height.0 as u32);
    let density = w as f32 / shot.size.0.max(1.);
    let bytes = shot.image.as_bytes(0)?;
    let pic =
        crate::designdraw::export::picture(bytes, w, h, density, None, density.round() as u32)?;
    crate::designdraw::export::encode(&pic, crate::designdraw::export::Format::Png).ok()
}
