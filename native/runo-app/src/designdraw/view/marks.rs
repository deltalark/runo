//! Draw to ask: with the marker, circle or scribble on screens to show AI what you mean.
//! The next ask takes a picture of each marked screen with the marks on it, and names the
//! layers under them, so "make this bigger" or "a button here" needs no selecting.

use super::*;

/// The marker's colour: apart from selection blue, so a mark never reads as a selection.
const MARK: u32 = 0xe8368f;
/// How thick a mark is, in screen pixels on the canvas and CSS pixels in its picture.
const THICK: f32 = 3.;
/// The most layers named for one mark.
const MOST_LAYERS: usize = 3;

/// A stroke on a screen, in the screen's CSS pixels.
pub(super) struct Mark {
    pub screen: String,
    pub pts: Vec<Pt>,
}

impl DesignCanvas {
    /// Turn the marker on or off. On, a drag on a screen draws a mark.
    pub(super) fn toggle_marking(&mut self, cx: &mut Context<Self>) {
        let on = !self.marking;
        self.set_shape(None, cx);
        self.marking = on;
        if on {
            self.hand = false;
            self.commenting = false;
            self.comment_open = None;
        }
        cx.notify();
    }

    /// Start a mark where the marker went down on a screen.
    pub(super) fn mark_down(&mut self, w: Pt, cx: &mut Context<Self>) -> bool {
        if !self.marking {
            return false;
        }
        let Some((id, false)) = self.screen_at(w) else {
            return false;
        };
        self.drag = Drag::Mark { id, pts: vec![w] };
        cx.notify();
        true
    }

    /// Keep the mark just drawn, in its screen's pixels. A click is a dot.
    pub(super) fn mark_up(&mut self, id: String, pts: Vec<Pt>, cx: &mut Context<Self>) {
        let Some(s) = self.canvas.screen(&id) else {
            return;
        };
        let mut pts: Vec<Pt> = pts.iter().map(|p| (p.0 - s.x, p.1 - s.y)).collect();
        if pts.len() == 1 {
            pts.push((pts[0].0 + 0.5, pts[0].1 + 0.5));
        }
        self.marks.push(Mark { screen: id, pts });
        cx.notify();
    }

    pub(super) fn clear_marks(&mut self, cx: &mut Context<Self>) {
        self.marks.clear();
        cx.notify();
    }

    /// Every mark, and the one being drawn, as canvas-pixel points to paint.
    pub(super) fn mark_strokes(&self) -> Vec<Vec<Pt>> {
        let mut out: Vec<Vec<Pt>> = self
            .marks
            .iter()
            .filter_map(|m| {
                let s = self.canvas.screen(&m.screen)?;
                Some(
                    m.pts
                        .iter()
                        .map(|p| self.screen_pt((s.x + p.0, s.y + p.1)))
                        .collect(),
                )
            })
            .collect();
        if let Drag::Mark { pts, .. } = &self.drag {
            out.push(pts.iter().map(|p| self.screen_pt(*p)).collect());
        }
        out
    }

    /// The line over the canvas while marking: what to do, and how many marks wait.
    pub(super) fn mark_hint(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.marking && self.marks.is_empty() {
            return None;
        }
        let n = self.marks.len();
        let text = match n {
            0 => "Draw on a screen to show AI what you mean, then say what to do.".to_string(),
            1 => "1 mark. Draw more, or say what to do in the ask bar.".to_string(),
            n => format!("{n} marks. Draw more, or say what to do in the ask bar."),
        };
        Some(
            div()
                .absolute()
                .top(px(12.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("design-mark-hint")
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .px_3()
                        .py(px(6.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_full()
                        .border_1()
                        .border_color(float_edge())
                        .bg(float(c_raised()))
                        .shadow_md()
                        .text_size(px(12.))
                        .text_color(rgb(c_text()))
                        .child(icon(IconName::Highlighter, 13., MARK))
                        .child(text)
                        .when(n > 0, |d| {
                            d.child(
                                div()
                                    .id("design-mark-clear")
                                    .text_color(rgb(c_blue()))
                                    .cursor_pointer()
                                    .child("Clear")
                                    .on_click(cx.listener(|this, _, _, cx| this.clear_marks(cx))),
                            )
                        })
                        .when(self.marking, |d| {
                            d.child(
                                div()
                                    .id("design-mark-done")
                                    .text_color(rgb(c_blue()))
                                    .cursor_pointer()
                                    .child("Done")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_marking(cx)),
                                    ),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    /// Take the marks for an ask: a picture of each marked screen with its marks on it, by
    /// screen, and the lines telling the agent what was marked. Empty with no marks.
    pub(super) fn take_marks(&mut self) -> (Vec<(String, String)>, String) {
        let marks = std::mem::take(&mut self.marks);
        self.marking = false;
        let mut screens: Vec<&str> = Vec::new();
        for m in &marks {
            if !screens.contains(&m.screen.as_str()) {
                screens.push(&m.screen);
            }
        }
        let mut pictures = Vec::new();
        let mut text = String::new();
        for id in screens {
            let Some(s) = self.canvas.screen(id) else {
                continue;
            };
            let here: Vec<&Mark> = marks.iter().filter(|m| m.screen == id).collect();
            let picture = self.save_marked_picture(id, &here);
            text.push_str(&format!(
                "- {} ({id}), {} mark{}{}:\n",
                s.name,
                here.len(),
                if here.len() == 1 { "" } else { "s" },
                picture
                    .as_deref()
                    .map(|p| format!(", in the picture at {p}"))
                    .unwrap_or_default()
            ));
            let boxes = self.shots.get(id).map(|s| &s.boxes[..]).unwrap_or_default();
            for (i, m) in here.iter().enumerate() {
                let area = bounds(&m.pts);
                let shape = if area.w < 12. && area.h < 12. {
                    "a dot"
                } else if closed(&m.pts) {
                    "a ring"
                } else {
                    "a stroke"
                };
                let layers = under_mark(boxes, &area);
                text.push_str(&format!(
                    "  {}. {shape} at x {}, y {}, {} × {} px{}\n",
                    i + 1,
                    area.x.round(),
                    area.y.round(),
                    area.w.round(),
                    area.h.round(),
                    if layers.is_empty() {
                        String::new()
                    } else {
                        format!(", over {}", layers.join(", "))
                    }
                ));
            }
            if let Some(p) = picture {
                pictures.push((id.to_string(), p));
            }
        }
        if text.is_empty() {
            return (pictures, text);
        }
        let intro = "\nThe person drew on the design to show what they mean. The pink marks in \
                     the attached pictures are theirs, not part of the design: read \"this\", \
                     \"here\" and \"these\" in the ask as what they marked.\n";
        (pictures, format!("{intro}{text}"))
    }

    /// Save a picture of screen `id` with `marks` drawn on it, and say where.
    fn save_marked_picture(&self, id: &str, marks: &[&Mark]) -> Option<String> {
        let shot = self.shots.get(id)?;
        let mut picture = self.picture_of(id)?;
        let scale = picture.width() as f32 / shot.size.0.max(1.);
        let r = (THICK * scale / 2.).max(1.);
        for m in marks {
            let pts: Vec<Pt> = m.pts.iter().map(|p| (p.0 * scale, p.1 * scale)).collect();
            stroke(&mut picture, &pts, r, MARK);
        }
        self.write_picture(id, "marked", picture)
    }
}

/// The box around points.
fn bounds(pts: &[Pt]) -> dom::Box {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    dom::Box {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0.),
        h: (y1 - y0).max(0.),
    }
}

/// Whether a stroke comes back near where it started, around something.
fn closed(pts: &[Pt]) -> bool {
    let b = bounds(pts);
    let (a, z) = (pts[0], pts[pts.len() - 1]);
    let gap = ((a.0 - z.0).powi(2) + (a.1 - z.1).powi(2)).sqrt();
    b.w.min(b.h) > 16. && gap < b.w.max(b.h) * 0.3
}

/// The layers a mark is about: those it takes in, else the one under its middle.
fn under_mark(boxes: &[dom::Bx], area: &dom::Box) -> Vec<String> {
    let mut paths = dom::inside(boxes, area);
    if paths.is_empty() {
        paths.extend(
            dom::under(boxes, area.x + area.w / 2., area.y + area.h / 2., None)
                .filter(|b| !b.path.is_empty())
                .map(|b| b.path.clone()),
        );
    }
    paths
        .iter()
        .take(MOST_LAYERS)
        .filter_map(|p| boxes.iter().find(|b| &b.path == p))
        .map(|b| match &b.key {
            Some(k) => format!("<{}> \"{k}\" at {:?}", b.tag, b.path),
            None => format!("<{}> at {:?}", b.tag, b.path),
        })
        .collect()
}

/// Draw a round-ended line through `pts`, `r` pixels either side, in `color`.
fn stroke(img: &mut image::RgbaImage, pts: &[Pt], r: f32, color: u32) {
    let px = image::Rgba([(color >> 16) as u8, (color >> 8) as u8, color as u8, 255]);
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut dot = |c: Pt| {
        let (x0, x1) = ((c.0 - r).floor() as i32, (c.0 + r).ceil() as i32);
        let (y0, y1) = ((c.1 - r).floor() as i32, (c.1 + r).ceil() as i32);
        for y in y0.max(0)..=y1.min(h - 1) {
            for x in x0.max(0)..=x1.min(w - 1) {
                let (dx, dy) = (x as f32 + 0.5 - c.0, y as f32 + 0.5 - c.1);
                if dx * dx + dy * dy <= r * r {
                    img.put_pixel(x as u32, y as u32, px);
                }
            }
        }
    };
    for pair in pts.windows(2) {
        let (a, z) = (pair[0], pair[1]);
        let steps = ((z.0 - a.0).abs().max((z.1 - a.1).abs()) / (r / 2.).max(0.5)).ceil() as usize;
        for i in 0..=steps.max(1) {
            let t = i as f32 / steps.max(1) as f32;
            dot((a.0 + (z.0 - a.0) * t, a.1 + (z.1 - a.1) * t));
        }
    }
}

/// Paint marks on the canvas.
pub(super) fn paint_marks(
    strokes: &[Vec<Pt>],
    tf: &dyn Fn(Pt) -> Point<Pixels>,
    window: &mut Window,
) {
    for pts in strokes.iter().filter(|p| p.len() >= 2) {
        let mut p = PathBuilder::stroke(px(THICK));
        p.move_to(tf(pts[0]));
        for q in &pts[1..] {
            p.line_to(tf(*q));
        }
        if let Ok(path) = p.build() {
            window.paint_path(path, col(MARK, 0.9));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn bx(path: &[usize], x: f32, y: f32, w: f32, h: f32, key: &str) -> dom::Bx {
        dom::Bx {
            path: path.to_vec(),
            rect: dom::Box { x, y, w, h },
            row: false,
            tag: "div".into(),
            key: Some(key.into()),
        }
    }

    #[test]
    fn a_ring_names_what_it_takes_in_and_a_stroke_what_it_is_over() {
        let boxes = vec![
            bx(&[0], 0., 0., 400., 800., "screen"),
            bx(&[0, 0], 20., 40., 100., 30., "Title"),
            bx(&[0, 1], 20., 100., 360., 48., "Get started"),
        ];
        let ring = [(10., 30.), (130., 30.), (130., 80.), (10., 80.), (14., 34.)];
        assert!(closed(&ring));
        let named = under_mark(&boxes, &bounds(&ring));
        assert_eq!(named, vec!["<div> \"Title\" at [0, 0]".to_string()]);
        let line = [(50., 120.), (90., 124.)];
        assert!(!closed(&line));
        assert_eq!(
            under_mark(&boxes, &bounds(&line)),
            vec!["<div> \"Get started\" at [0, 1]".to_string()]
        );
    }

    #[test]
    fn strokes_are_drawn_thick_and_stay_inside_the_picture() {
        let mut img = image::RgbaImage::new(20, 20);
        stroke(&mut img, &[(-5., 10.), (25., 10.)], 2., MARK);
        assert_eq!(img.get_pixel(10, 10).0, [0xe8, 0x36, 0x8f, 255]);
        assert_eq!(img.get_pixel(0, 10).0[3], 255);
        assert_eq!(img.get_pixel(10, 4).0[3], 0);
    }
}
