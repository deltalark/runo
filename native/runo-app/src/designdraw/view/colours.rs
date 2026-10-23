//! The colour picker design tools have: a square of shades for the hue, a hue strip and
//! an opacity strip, the theme's colours and the ones used last, and an eyedropper that
//! takes a colour from anywhere on the screens.

use super::figma::caption;
use super::*;

const PICKER_W: f32 = 240.;
const SQUARE_H: f32 = 168.;
const STRIP_H: f32 = 12.;
/// How many colours the picker remembers as used last.
const RECENT: usize = 8;

/// A colour as hue (0–360), saturation, value and alpha (0–1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Hsva {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: f32,
}

impl Hsva {
    pub(super) fn from_rgb(rgb: u32, a: f32) -> Self {
        let (r, g, b) = (
            ((rgb >> 16) & 0xff) as f32 / 255.,
            ((rgb >> 8) & 0xff) as f32 / 255.,
            (rgb & 0xff) as f32 / 255.,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0. {
            0.
        } else if max == r {
            60. * ((g - b) / d).rem_euclid(6.)
        } else if max == g {
            60. * ((b - r) / d + 2.)
        } else {
            60. * ((r - g) / d + 4.)
        };
        let s = if max == 0. { 0. } else { d / max };
        Self { h, s, v: max, a }
    }

    pub(super) fn rgb(&self) -> u32 {
        let c = self.v * self.s;
        let x = c * (1. - ((self.h / 60.).rem_euclid(2.) - 1.).abs());
        let m = self.v - c;
        let (r, g, b) = match (self.h.rem_euclid(360.) / 60.) as u32 {
            0 => (c, x, 0.),
            1 => (x, c, 0.),
            2 => (0., c, x),
            3 => (0., x, c),
            4 => (x, 0., c),
            _ => (c, 0., x),
        };
        let to = |v: f32| (((v + m) * 255.).round().clamp(0., 255.)) as u32;
        (to(r) << 16) | (to(g) << 8) | to(b)
    }

    /// As CSS: `#rrggbb` when opaque, else `rgba(…)`.
    pub(super) fn css(&self) -> String {
        let c = self.rgb();
        if self.a >= 0.999 {
            format!("#{c:06x}")
        } else {
            format!(
                "rgba({}, {}, {}, {})",
                (c >> 16) & 0xff,
                (c >> 8) & 0xff,
                c & 0xff,
                (self.a * 100.).round() / 100.
            )
        }
    }
}

/// A CSS colour as RGB and alpha: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(…)` and `rgba(…)`,
/// with commas or spaces.
pub(super) fn parse_css(css: &str) -> Option<(u32, f32)> {
    let s = css.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let full: String = match hex.len() {
            3 | 4 => hex.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => hex.to_string(),
            _ => return None,
        };
        let n = u32::from_str_radix(&full[..6], 16).ok()?;
        let a = match full.get(6..8) {
            Some(a) => u8::from_str_radix(a, 16).ok()? as f32 / 255.,
            None => 1.,
        };
        return Some((n, a));
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .collect();
    let channel = |p: &str| {
        p.trim()
            .parse::<f32>()
            .ok()
            .map(|v| v.round().clamp(0., 255.) as u32)
    };
    let (r, g, b) = (
        channel(parts.first()?)?,
        channel(parts.get(1)?)?,
        channel(parts.get(2)?)?,
    );
    let a = match parts.get(3) {
        Some(p) if p.ends_with('%') => p.trim_end_matches('%').parse::<f32>().ok()? / 100.,
        Some(p) => p.parse::<f32>().ok()?,
        None => 1.,
    };
    Some(((r << 16) | (g << 8) | b, a.clamp(0., 1.)))
}

/// The style each colour field sets.
pub(super) fn property(field: &str) -> &'static str {
    match field {
        "color" => "color",
        "stroke" => "border-color",
        _ => "background-color",
    }
}

/// The part of the picker being dragged.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Square,
    Hue,
    Alpha,
}

/// The colour picker open on one of the colour fields.
pub(super) struct Picker {
    /// The field it's open on: "fill", "color" or "stroke".
    pub field: &'static str,
    /// The layer it's open for; picking another hides it.
    pub path: dom::Path,
    pub colour: Hsva,
    pub dragging: Option<Part>,
    /// Where each part was last drawn, in window pixels, for the pointer.
    pub square: Rc<Cell<Bounds<Pixels>>>,
    pub hue: Rc<Cell<Bounds<Pixels>>>,
    pub alpha: Rc<Cell<Bounds<Pixels>>>,
}

/// The colour of the pixel at page point `(x, y)` of a screen's picture, `size` being the
/// page size it shows. The picture's bytes are BGRA, as GPUI keeps them.
pub(super) fn pixel(image: &RenderImage, size: (f32, f32), x: f32, y: f32) -> Option<u32> {
    let px_size = image.size(0);
    let (w, h) = (px_size.width.0 as f32, px_size.height.0 as f32);
    if size.0 <= 0. || size.1 <= 0. || x < 0. || y < 0. {
        return None;
    }
    let (ix, iy) = ((x * w / size.0) as usize, (y * h / size.1) as usize);
    if ix >= w as usize || iy >= h as usize {
        return None;
    }
    let bytes = image.as_bytes(0)?;
    let i = (iy * w as usize + ix) * 4;
    let p = bytes.get(i..i + 4)?;
    Some(((p[2] as u32) << 16) | ((p[1] as u32) << 8) | p[0] as u32)
}

/// A strip or square remembering where it was drawn.
fn track(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |b, _, _| cell.set(b), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// How far along `b` a point is, across and down, each 0–1.
fn along(b: Bounds<Pixels>, p: Point<Pixels>) -> (f32, f32) {
    let w = f32::from(b.size.width).max(1.);
    let h = f32::from(b.size.height).max(1.);
    (
        (f32::from(p.x - b.origin.x) / w).clamp(0., 1.),
        (f32::from(p.y - b.origin.y) / h).clamp(0., 1.),
    )
}

fn hsla_of(rgb_: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(rgb_).into();
    c.a = a;
    c
}

impl DesignCanvas {
    /// Open the picker on colour field `field` of the picked layer, or close it if it's
    /// open there already.
    pub(super) fn open_picker(&mut self, field: &'static str, cx: &mut Context<Self>) {
        let path = self
            .element
            .as_ref()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        if self
            .picker
            .as_ref()
            .is_some_and(|p| p.field == field && p.path == path)
        {
            self.picker = None;
            cx.notify();
            return;
        }
        let now = self
            .element
            .as_ref()
            .map(|e| e.style(property(field)))
            .unwrap_or_default();
        let (c, a) = parse_css(&now).unwrap_or((0xffffff, 1.));
        self.picker = Some(Picker {
            field,
            path,
            colour: Hsva::from_rgb(c, a),
            dragging: None,
            square: Rc::default(),
            hue: Rc::default(),
            alpha: Rc::default(),
        });
        cx.notify();
    }

    /// The colour shown for colour field `field`: the picker's while it's open there.
    pub(super) fn picking(&self, field: &str) -> Option<u32> {
        self.picker
            .as_ref()
            .filter(|p| p.field == field && self.element.as_ref().is_some_and(|e| e.path == p.path))
            .map(|p| p.colour.rgb())
    }

    /// Follow the pointer on the part of the picker being dragged. True when it was.
    pub(super) fn picker_drag(&mut self, p: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(pk) = self.picker.as_mut() else {
            return false;
        };
        let Some(part) = pk.dragging else {
            return false;
        };
        match part {
            Part::Square => {
                let (x, y) = along(pk.square.get(), p);
                pk.colour.s = x;
                pk.colour.v = 1. - y;
            }
            Part::Hue => pk.colour.h = along(pk.hue.get(), p).0 * 359.9,
            Part::Alpha => pk.colour.a = (along(pk.alpha.get(), p).0 * 100.).round() / 100.,
        }
        cx.notify();
        true
    }

    /// Let go of the picker: set the colour on the picked layer. True when it was held.
    pub(super) fn picker_drop(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pk) = self.picker.as_mut() else {
            return false;
        };
        if pk.dragging.take().is_none() {
            return false;
        }
        let (field, css) = (pk.field, pk.colour.css());
        self.use_colour(property(field), &css, cx);
        true
    }

    /// Set a colour on the picked layer, and remember it among the ones used last.
    pub(super) fn use_colour(&mut self, prop: &'static str, css: &str, cx: &mut Context<Self>) {
        self.recent_colours.retain(|c| c != css);
        self.recent_colours.insert(0, css.to_string());
        self.recent_colours.truncate(RECENT);
        self.set_style(prop, css, cx);
    }

    /// Turn the eyedropper on or off.
    pub(super) fn toggle_eyedropper(&mut self, cx: &mut Context<Self>) {
        self.eyedropper = !self.eyedropper && self.element.is_some();
        self.eye_at = None;
        cx.notify();
    }

    /// The colour on the canvas under a point, in canvas units.
    pub(super) fn colour_at(&self, w: Pt) -> Option<u32> {
        let (id, title) = self.screen_at(w)?;
        if title {
            return None;
        }
        let s = self.canvas.screen(&id)?;
        let shot = self.shots.get(&id)?;
        pixel(&shot.image, shot.size, w.0 - s.x, w.1 - s.y)
    }

    /// A click with the eyedropper: the colour under it goes on the picked layer, in the
    /// field the picker is open on, else its text colour for text and its fill otherwise.
    pub(super) fn eyedrop(&mut self, w: Pt, cx: &mut Context<Self>) {
        self.eyedropper = false;
        self.eye_at = None;
        let Some(c) = self.colour_at(w) else {
            cx.notify();
            return;
        };
        let field = match &self.picker {
            Some(p) => p.field,
            None if self.element.as_ref().is_some_and(|e| e.text.is_some()) => "color",
            None => "fill",
        };
        if let Some(p) = self.picker.as_mut() {
            p.colour = Hsva::from_rgb(c, 1.);
        }
        self.use_colour(property(field), &format!("#{c:06x}"), cx);
    }

    /// The loupe that follows the pointer with the eyedropper: the colour under it, and its
    /// hex.
    pub(super) fn eyedropper_loupe(&self) -> Option<AnyElement> {
        let (at, c) = self.eye_at?;
        Some(
            div()
                .absolute()
                .left(px(at.0 + 14.))
                .top(px(at.1 + 14.))
                .flex()
                .items_center()
                .gap(px(6.))
                .p(px(4.))
                .pr(px(8.))
                .rounded_full()
                .bg(float(c_canvas()))
                .border_1()
                .border_color(divider())
                .shadow_lg()
                .child(
                    div()
                        .size(px(22.))
                        .rounded_full()
                        .border_2()
                        .border_color(rgb(0xffffff))
                        .bg(rgb(c)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .font_family("monospace")
                        .text_color(rgb(c_text()))
                        .child(format!("#{c:06X}")),
                )
                .into_any_element(),
        )
    }

    /// The picker itself, floating over the canvas beside the right panel.
    pub(super) fn colour_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pk = self.picker.as_ref()?;
        if self.element.as_ref()?.path != pk.path || !self.also.is_empty() {
            return None;
        }
        let c = pk.colour;
        let pure = Hsva {
            s: 1.,
            v: 1.,
            a: 1.,
            ..c
        }
        .rgb();
        let now = c.rgb();
        let knob = |size: f32| {
            div()
                .absolute()
                .size(px(size))
                .rounded_full()
                .border_2()
                .border_color(rgb(0xffffff))
                .shadow_md()
        };
        let title = match pk.field {
            "color" => "Text colour",
            "stroke" => "Stroke",
            _ => "Fill",
        };
        let inner = PICKER_W - 24.;
        let start = |part: Part| {
            cx.listener(move |this: &mut Self, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                if let Some(p) = this.picker.as_mut() {
                    p.dragging = Some(part);
                }
                this.picker_drag(e.position, cx);
            })
        };
        // The hue strip: six bands, each a gradient between two of the primaries.
        let mut hue = div()
            .id("design-picker-hue")
            .relative()
            .h(px(STRIP_H))
            .w_full()
            .flex()
            .rounded_full()
            .overflow_hidden()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, start(Part::Hue))
            .child(track(pk.hue.clone()));
        let stops = [
            0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xff0000,
        ];
        for pair in stops.windows(2) {
            hue = hue.child(div().flex_1().h_full().bg(linear_gradient(
                90.,
                linear_color_stop(rgb(pair[0]), 0.),
                linear_color_stop(rgb(pair[1]), 1.),
            )));
        }
        let hue = div().relative().child(hue).child(
            knob(14.)
                .top(px(-1.))
                .left(px(c.h / 360. * inner - 7.))
                .bg(rgb(pure)),
        );
        let alpha = div()
            .relative()
            .child(
                div()
                    .id("design-picker-alpha")
                    .relative()
                    .h(px(STRIP_H))
                    .w_full()
                    .rounded_full()
                    .overflow_hidden()
                    .cursor_pointer()
                    .bg(rgb(0xd4d4d8))
                    .on_mouse_down(MouseButton::Left, start(Part::Alpha))
                    .child(track(pk.alpha.clone()))
                    .child(div().absolute().size_full().bg(linear_gradient(
                        90.,
                        linear_color_stop(hsla_of(now, 0.), 0.),
                        linear_color_stop(hsla_of(now, 1.), 1.),
                    ))),
            )
            .child(
                knob(14.)
                    .top(px(-1.))
                    .left(px(c.a * (inner - 48.) - 7.))
                    .bg(rgb(now)),
            );
        let square = div()
            .id("design-picker-square")
            .relative()
            .w_full()
            .h(px(SQUARE_H))
            .rounded(px(6.))
            .overflow_hidden()
            .cursor_crosshair()
            .bg(rgb(pure))
            .on_mouse_down(MouseButton::Left, start(Part::Square))
            .child(track(pk.square.clone()))
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(hsla_of(0xffffff, 1.), 0.),
                linear_color_stop(hsla_of(0xffffff, 0.), 1.),
            )))
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(hsla_of(0x000000, 0.), 0.),
                linear_color_stop(hsla_of(0x000000, 1.), 1.),
            )))
            .child(
                knob(16.)
                    .left(px(c.s * inner - 8.))
                    .top(px((1. - c.v) * SQUARE_H - 8.))
                    .bg(rgb(now)),
            );
        let value = |label: &'static str, v: String| {
            div()
                .flex_1()
                .min_w_0()
                .h(px(26.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(5.))
                .bg(tint(c_subtle()))
                .text_size(px(11.5))
                .child(div().text_color(rgb(c_muted())).child(label))
                .child(div().text_color(rgb(c_text())).truncate().child(v))
        };
        let values = div()
            .flex()
            .gap(px(4.))
            .child(value("#", format!("{now:06X}")).flex_grow(1.))
            .child(
                value("R", ((now >> 16) & 0xff).to_string())
                    .flex_none()
                    .w(px(42.)),
            )
            .child(
                value("G", ((now >> 8) & 0xff).to_string())
                    .flex_none()
                    .w(px(42.)),
            )
            .child(value("B", (now & 0xff).to_string()).flex_none().w(px(42.)));
        let opacity = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().child(alpha))
            .child(
                div()
                    .w(px(40.))
                    .text_right()
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .child(format!("{}%", (c.a * 100.).round())),
            );
        let field = pk.field;
        let prop = property(field);
        let mut recent = div().flex().flex_wrap().gap(px(4.));
        for (i, css) in self.recent_colours.iter().enumerate() {
            let Some((rgb_, a)) = parse_css(css) else {
                continue;
            };
            let css = css.clone();
            recent = recent.child(
                div()
                    .id(("design-picker-recent", i))
                    .size(px(18.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(hsla_of(rgb_, a))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(p) = this.picker.as_mut() {
                            p.colour = Hsva::from_rgb(rgb_, a);
                        }
                        this.use_colour(prop, &css, cx);
                    })),
            );
        }
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .child(
                        icon_btn(
                            "design-picker-eyedropper",
                            IconName::Pipette,
                            "Pick a colour from the canvas (I)",
                            self.eyedropper,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_eyedropper(cx))),
                    )
                    .child(
                        icon_btn("design-picker-close", IconName::Close, "Close", false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.picker = None;
                                cx.notify();
                            }),
                        ),
                    ),
            );
        Some(
            div()
                .id("design-colour-picker")
                .absolute()
                .top(px(12.))
                .right(px(12.))
                .w(px(PICKER_W))
                .p(px(12.))
                .v_flex()
                .gap(px(10.))
                .rounded_lg()
                .border_1()
                .border_color(divider())
                .bg(float(c_canvas()))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(head)
                .child(square)
                .child(hue)
                .child(opacity)
                .child(values)
                .child(caption("Theme"))
                .child(self.token_swatches(prop, cx))
                .when(!self.recent_colours.is_empty(), |el| {
                    el.child(caption("Used last")).child(recent)
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn colours_go_to_hsv_and_back() {
        for c in [
            0x000000, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0x1c1917, 0xd93a1e, 0x7c3aed,
        ] {
            assert_eq!(Hsva::from_rgb(c, 1.).rgb(), c, "{c:06x}");
        }
        let red = Hsva::from_rgb(0xff0000, 1.);
        assert_eq!((red.h, red.s, red.v), (0., 1., 1.));
        let blue = Hsva::from_rgb(0x0000ff, 1.);
        assert_eq!(blue.h, 240.);
    }

    #[test]
    fn css_colours_are_read_in_every_form_the_page_gives() {
        assert_eq!(parse_css("#d93a1e"), Some((0xd93a1e, 1.)));
        assert_eq!(parse_css("#fff"), Some((0xffffff, 1.)));
        assert_eq!(parse_css("rgb(217, 58, 30)"), Some((0xd93a1e, 1.)));
        assert_eq!(parse_css("rgba(0, 0, 0, 0.5)"), Some((0, 0.5)));
        assert_eq!(parse_css("rgb(0 0 0 / 25%)"), Some((0, 0.25)));
        assert_eq!(
            parse_css("#00000080").map(|(c, a)| (c, (a * 100.).round())),
            Some((0, 50.))
        );
        assert_eq!(parse_css("var(--brand)"), None);
    }

    #[test]
    fn a_colour_is_written_as_hex_unless_see_through() {
        assert_eq!(Hsva::from_rgb(0xd93a1e, 1.).css(), "#d93a1e");
        assert_eq!(
            Hsva::from_rgb(0xd93a1e, 0.5).css(),
            "rgba(217, 58, 30, 0.5)"
        );
    }

    #[test]
    fn the_eyedropper_reads_the_picture_at_page_scale() {
        // A 4×2 picture of a 2×1 page: each page pixel is 2×2 picture pixels. BGRA.
        let mut bytes = Vec::new();
        for i in 0..8 {
            let bgra = if i % 4 < 2 {
                [0x30, 0x20, 0x10, 0xff]
            } else {
                [0x03, 0x02, 0x01, 0xff]
            };
            bytes.extend(bgra);
        }
        let img = image::RgbaImage::from_raw(4, 2, bytes).unwrap();
        let pic = RenderImage::new(vec![image::Frame::new(img)]);
        assert_eq!(pixel(&pic, (2., 1.), 0.2, 0.5), Some(0x102030));
        assert_eq!(pixel(&pic, (2., 1.), 1.5, 0.5), Some(0x010203));
        assert_eq!(pixel(&pic, (2., 1.), 2.5, 0.5), None);
    }
}
