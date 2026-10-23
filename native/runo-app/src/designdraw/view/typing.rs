//! Editing a text layer where it is: the editor sits on the layer in the layer's own size,
//! weight and colours, the screen takes each change as it's typed, and the whole edit
//! undoes as one step.

use super::*;
use crate::designdraw::fields;

/// Text being typed into a layer.
pub(super) struct Typing {
    screen: String,
    path: dom::Path,
    /// The latest text, and the text last sent to the screen.
    last: String,
    sent: String,
    /// An edit is on its way, so the next waits for it.
    busy: bool,
    /// The editor closed; typing ends once its last edit lands.
    done: bool,
    /// The first edit has made the undo step.
    stepped: bool,
}

/// A CSS colour as `0xRRGGBB`, or nothing for a transparent one.
fn colour(css: &str) -> Option<u32> {
    let hex = fields::hex(css)?;
    u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()
}

/// The colour of a screen's picture at a point on the page, as `0xRRGGBB`.
pub(super) fn pixel(image: &RenderImage, size: (f32, f32), x: f32, y: f32) -> Option<u32> {
    let s = image.size(0);
    let (w, h) = (s.width.0 as f32, s.height.0 as f32);
    if size.0 <= 0. || size.1 <= 0. || x < 0. || y < 0. {
        return None;
    }
    let (ix, iy) = ((x * w / size.0) as usize, (y * h / size.1) as usize);
    if ix >= w as usize || iy >= h as usize {
        return None;
    }
    let p = image
        .as_bytes(0)?
        .get((iy * w as usize + ix) * 4..)?
        .get(..4)?;
    Some(((p[2] as u32) << 16) | ((p[1] as u32) << 8) | p[0] as u32)
}

/// A computed length in pixels, or 0.
fn length(el: &Element, k: &str) -> f32 {
    el.style(k)
        .trim_end_matches("px")
        .trim()
        .parse()
        .unwrap_or(0.)
}

impl DesignCanvas {
    /// What was typed so far: send it to the screen when nothing else is on its way.
    pub(super) fn typed(&mut self, v: String, cx: &mut Context<Self>) {
        let Some(el) = self.element.as_ref().filter(|e| e.text.is_some()) else {
            return;
        };
        let Some(screen) = self.screen.clone() else {
            return;
        };
        let t = self.typing.get_or_insert_with(|| Typing {
            screen,
            path: el.path.clone(),
            sent: el.text.clone().unwrap_or_default(),
            last: String::new(),
            busy: false,
            done: false,
            stepped: false,
        });
        t.last = v;
        self.send_typed(cx);
    }

    /// The editor closed with `v` in it.
    pub(super) fn end_typing(&mut self, v: String, cx: &mut Context<Self>) {
        if self.typing.is_none() {
            self.set_text(v, cx);
            return;
        }
        if let Some(t) = self.typing.as_mut() {
            t.last = v;
            t.done = true;
        }
        self.send_typed(cx);
        if self.typing.as_ref().is_some_and(|t| !t.busy) {
            self.typing = None;
        }
    }

    fn send_typed(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.typing.as_mut().filter(|t| !t.busy && t.last != t.sent) else {
            return;
        };
        if self.screen.as_ref() != Some(&t.screen) {
            return;
        }
        t.busy = true;
        t.sent = t.last.clone();
        let js = dom::op(&t.path, "text", &[json!(t.last)]);
        self.edit(js, After::Same, cx);
    }

    /// Whether an edit landing now is more typing into a layer that already has its undo
    /// step.
    pub(super) fn typing_merges(&mut self) -> bool {
        self.typing
            .as_mut()
            .is_some_and(|t| std::mem::replace(&mut t.stepped, true))
    }

    /// An edit has landed, or failed: send what was typed since.
    pub(super) fn typing_landed(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.typing.as_mut() else {
            return;
        };
        t.busy = false;
        if t.last != t.sent {
            self.send_typed(cx);
        } else if t.done {
            self.typing = None;
        }
    }

    /// The editor over a text layer, set in the layer's own type and colours so typing
    /// looks like the result.
    pub(super) fn text_editor(&self, el: &Element, at: Pt, size: Pt) -> AnyElement {
        let z = self.zoom;
        let (x, y) = at;
        let fs = length(el, "font-size").max(8.) * z;
        let fg = colour(&el.style("color")).unwrap_or(0x111111);
        // The layer's own fill, else what's drawn behind it.
        let bg = colour(&el.style("background-color")).or_else(|| {
            let shot = self.shots.get(self.screen.as_ref()?)?;
            pixel(&shot.image, shot.size, el.rect.x + 1., el.rect.y + 1.)
        });
        let centred = el.style("text-align") == "center"
            || (el.style("display").contains("flex") && el.style("justify-content") == "center");
        let weight = el.style("font-weight").parse::<f32>().unwrap_or(400.);
        // Sized to what's typed, so centred text stays put while it grows.
        let shown = match &self.typing {
            Some(t) => t.last.as_str(),
            None => el.text.as_deref().unwrap_or(""),
        };
        let chars = shown.chars().count().max(1) as f32;
        let pad = |k| length(el, k) * z;
        let (pl, pr) = (pad("padding-left"), pad("padding-right"));
        // Wide enough for the text: a small layer's editor grows, about its middle when
        // the text is centred.
        let text_w = chars * fs * 0.72 + fs * 2.;
        // Text that wraps keeps the layer's width and scrolls in the editor.
        let wraps = size.1 > (length(el, "line-height").max(fs / z * 1.2) * z) * 1.6;
        let w = if wraps {
            size.0
        } else {
            size.0.max(text_w + pl + pr)
        };
        let x = if centred { x - (w - size.0) / 2. } else { x };
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .min_h(px(size.1))
            .pl(px(pl))
            .pr(px(pr))
            .flex()
            .items_center()
            .when(centred, |d| d.justify_center())
            .rounded(px(length(el, "border-top-left-radius") * z))
            .bg(rgb(bg.unwrap_or(0xffffff)))
            .border_1()
            .border_color(rgb(c_blue()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .when(centred, |d| d.w(px(text_w)))
                    .when(!centred, |d| d.flex_1())
                    .max_w_full()
                    .child(
                        Input::new(&self.f_inline)
                            .appearance(false)
                            .h(px((fs * 1.5).max(18.)))
                            .text_size(px(fs))
                            .text_color(rgb(fg))
                            .font_weight(FontWeight(weight)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn page_colours_read_as_hex_numbers() {
        assert_eq!(colour("rgb(255, 255, 255)"), Some(0xffffff));
        assert_eq!(colour("rgba(28, 25, 23, 1)"), Some(0x1c1917));
        assert_eq!(colour("rgba(0, 0, 0, 0)"), None);
    }
}
