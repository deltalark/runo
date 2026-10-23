//! Shapes drawn by hand: rectangles, ellipses, lines and arrows dragged out on a screen, and
//! paths placed point by point with the pen. Each lands as a layer floating where it was
//! drawn, inside whatever box holds that spot.

use super::figma::{caption, sec};
use super::*;

/// What the shape tool draws.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pen,
}

impl Shape {
    pub(super) fn name(self) -> &'static str {
        match self {
            Shape::Rect => "Rectangle",
            Shape::Ellipse => "Ellipse",
            Shape::Line => "Line",
            Shape::Arrow => "Arrow",
            Shape::Pen => "Path",
        }
    }
}

/// How close, in screen pixels, a pen click must come to the first point to close the path.
const CLOSE: f32 = 8.;

/// With shift held, a box becomes a square and a line snaps to the nearest 45°.
pub(super) fn constrain(shape: Shape, from: Pt, at: Pt) -> Pt {
    let (dx, dy) = (at.0 - from.0, at.1 - from.1);
    match shape {
        Shape::Rect | Shape::Ellipse => {
            let s = dx.abs().max(dy.abs());
            (from.0 + s * dx.signum(), from.1 + s * dy.signum())
        }
        _ => {
            let len = (dx * dx + dy * dy).sqrt();
            let step = std::f32::consts::FRAC_PI_4;
            let a = (dy.atan2(dx) / step).round() * step;
            (from.0 + len * a.cos(), from.1 + len * a.sin())
        }
    }
}

/// The markup for a shape through `pts`, in page units, and the top-left corner it sits at.
fn markup(shape: Shape, pts: &[Pt], closed: bool) -> (String, Pt) {
    let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        (l, t, r, b) = (l.min(p.0), t.min(p.1), r.max(p.0), b.max(p.1));
    }
    let name = shape.name();
    match shape {
        Shape::Rect | Shape::Ellipse => {
            // A click without a drag gives a shape of the usual size.
            let (w, h) = if r - l < 3. && b - t < 3. {
                (100., 100.)
            } else {
                ((r - l).round().max(1.), (b - t).round().max(1.))
            };
            let round = if shape == Shape::Ellipse {
                ";border-radius:50%"
            } else {
                ""
            };
            (
                format!(
                    "<div data-name=\"{name}\" style=\"width:{w}px;height:{h}px;\
                     background:var(--surface-2){round}\"></div>"
                ),
                (l.round(), t.round()),
            )
        }
        _ => {
            // Lines are drawn in an svg sized to their bounds, with room for the stroke and
            // an arrow's head.
            let pad = if shape == Shape::Arrow { 8. } else { 2. };
            let (ox, oy) = ((l - pad).round(), (t - pad).round());
            let (w, h) = ((r + pad).round() - ox, (b + pad).round() - oy);
            let pt = |p: &Pt| format!("{} {}", fmt(p.0 - ox), fmt(p.1 - oy));
            let mut d = String::new();
            for (i, p) in pts.iter().enumerate() {
                d.push_str(if i == 0 { "M" } else { " L" });
                d.push_str(&pt(p));
            }
            if closed {
                d.push_str(" Z");
            }
            if let (Shape::Arrow, [.., a, z]) = (shape, pts) {
                let (left, right) = head(*a, *z, 10.);
                d.push_str(&format!(" M{} L{} L{}", pt(&left), pt(z), pt(&right)));
            }
            // Fill and stroke are styles on the svg, so the inspector edits them as it does
            // any other style, and the path takes them on.
            let fill = if closed { "var(--surface-2)" } else { "none" };
            (
                format!(
                    "<svg data-name=\"{name}\" width=\"{w}\" height=\"{h}\" \
                     viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"none\" style=\"fill:{fill};stroke:var(--text);\
                     stroke-width:2px;overflow:visible\"><path d=\"{d}\" \
                     vector-effect=\"non-scaling-stroke\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></svg>"
                ),
                (ox, oy),
            )
        }
    }
}

fn fmt(v: f32) -> String {
    let v = (v * 10.).round() / 10.;
    format!("{v}")
}

/// The two ends of an arrow's head at `z`, pointing away from `a`.
fn head(a: Pt, z: Pt, size: f32) -> (Pt, Pt) {
    let (x, y) = (z.0 - a.0, z.1 - a.1);
    let l = (x * x + y * y).sqrt().max(0.001);
    let (ux, uy) = (x / l, y / l);
    (
        (
            z.0 - ux * size - uy * size * 0.6,
            z.1 - uy * size + ux * size * 0.6,
        ),
        (
            z.0 - ux * size + uy * size * 0.6,
            z.1 - uy * size - ux * size * 0.6,
        ),
    )
}

/// Paint a shape being drawn: its points, in canvas pixels, as a line.
pub(super) fn paint_sketch(
    pts: &[Pt],
    arrow: bool,
    tf: &dyn Fn(Pt) -> Point<Pixels>,
    window: &mut Window,
) {
    let color = col(c_blue(), 1.);
    if pts.len() >= 2 {
        let mut p = PathBuilder::stroke(px(1.5));
        p.move_to(tf(pts[0]));
        for q in &pts[1..] {
            p.line_to(tf(*q));
        }
        if arrow {
            let (a, z) = (pts[pts.len() - 2], pts[pts.len() - 1]);
            let (left, right) = head(a, z, 10.);
            p.move_to(tf(left));
            p.line_to(tf(z));
            p.line_to(tf(right));
        }
        if let Ok(path) = p.build() {
            window.paint_path(path, color);
        }
    }
}

impl DesignCanvas {
    /// Fill and stroke for a drawing.
    pub(super) fn vector_section(&self, el: &dom::Element, cx: &mut Context<Self>) -> Div {
        sec("Vector", None)
            .child(caption("Fill"))
            .child(self.choice_row("fill", el, &[("None", "none")], cx))
            .child(self.token_swatches("fill", cx))
            .child(caption("Stroke"))
            .child(self.token_swatches("stroke", cx))
            .child(self.choice_row(
                "stroke-width",
                el,
                &[
                    ("1", "1px"),
                    ("2", "2px"),
                    ("3", "3px"),
                    ("4", "4px"),
                    ("6", "6px"),
                ],
                cx,
            ))
    }

    /// Pick a shape tool, or none for the move tool.
    pub(super) fn set_shape(&mut self, shape: Option<Shape>, cx: &mut Context<Self>) {
        self.finish_pen(false, cx);
        self.shape = shape;
        self.marking = false;
        if shape.is_some() {
            self.hand = false;
            self.commenting = false;
            self.comment_open = None;
            self.new_open = false;
        }
        cx.notify();
    }

    /// A press with a shape tool. Gives whether it was taken.
    pub(super) fn shape_down(&mut self, w: Pt, clicks: usize, cx: &mut Context<Self>) -> bool {
        let Some(shape) = self.shape else {
            return false;
        };
        if shape == Shape::Pen {
            if let Some((_, pts)) = &mut self.pen {
                let first = pts[0];
                let near = (w.0 - first.0).abs().max((w.1 - first.1).abs()) * self.zoom < CLOSE;
                if near && pts.len() >= 3 {
                    self.finish_pen(true, cx);
                } else if clicks >= 2 {
                    self.finish_pen(false, cx);
                } else {
                    let id = self.pen.as_ref().map(|p| p.0.clone()).unwrap_or_default();
                    let w = self.snap_point(&id, w);
                    if let Some((_, pts)) = &mut self.pen {
                        pts.push(w);
                    }
                }
                cx.notify();
                return true;
            }
        }
        match self.screen_at(w) {
            Some((id, false)) => {
                self.select_screen(Some(id.clone()));
                let w = self.snap_point(&id, w);
                if shape == Shape::Pen {
                    self.pen = Some((id, vec![w]));
                } else {
                    self.drag = Drag::Draw {
                        id,
                        from: w,
                        at: w,
                        shape,
                    };
                }
            }
            _ => self.say("Draw on a screen."),
        }
        cx.notify();
        true
    }

    /// Finish the path the pen is placing. A path needs two points; with fewer it's
    /// dropped.
    pub(super) fn finish_pen(&mut self, closed: bool, cx: &mut Context<Self>) {
        let Some((id, pts)) = self.pen.take() else {
            return;
        };
        self.pen_at = None;
        if pts.len() >= 2 {
            self.draw_shape(&id, Shape::Pen, &pts, closed, cx);
        }
    }

    /// Put a shape through `pts`, in canvas units, on screen `id`, and pick it.
    pub(super) fn draw_shape(
        &mut self,
        id: &str,
        shape: Shape,
        pts: &[Pt],
        closed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((sx, sy, device)) = self.canvas.screen(id).map(|s| (s.x, s.y, s.device)) else {
            return;
        };
        let local: Vec<Pt> = pts.iter().map(|p| (p.0 - sx, p.1 - sy)).collect();
        let (html, (x, y)) = markup(shape, &local, closed);
        let js = dom::call("drawShape", &[json!(x), json!(y), json!(html)]);
        // One shape at a time: after drawing, the move tool is back, as in other design tools.
        self.shape = None;
        let id = id.to_string();
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

    /// The shape being drawn, as canvas-pixel points to paint as a line, and whether it ends
    /// in an arrow's head. Boxes are drawn by `shape_marks` instead.
    pub(super) fn sketch(&self) -> Option<(Vec<Pt>, bool)> {
        if let Some((_, pts)) = &self.pen {
            let mut all: Vec<Pt> = pts.iter().map(|p| self.screen_pt(*p)).collect();
            if let Some(at) = self.pen_at {
                all.push(self.screen_pt(at));
            }
            return Some((all, false));
        }
        match &self.drag {
            Drag::Draw {
                from, at, shape, ..
            } if matches!(shape, Shape::Line | Shape::Arrow) => Some((
                vec![self.screen_pt(*from), self.screen_pt(*at)],
                *shape == Shape::Arrow,
            )),
            _ => None,
        }
    }

    /// The box being dragged out for a rectangle or ellipse, and the pen's points.
    pub(super) fn shape_marks(&self) -> Vec<AnyElement> {
        let z = self.zoom;
        let mut out = vec![];
        if let Drag::Draw {
            from, at, shape, ..
        } = &self.drag
        {
            if matches!(shape, Shape::Rect | Shape::Ellipse) {
                let (x, y) = self.screen_pt((from.0.min(at.0), from.1.min(at.1)));
                let (w, h) = ((at.0 - from.0).abs() * z, (at.1 - from.1).abs() * z);
                out.push(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(w))
                        .h(px(h))
                        .when(*shape == Shape::Ellipse, |d| d.rounded_full())
                        .border_1()
                        .border_color(rgb(c_blue()))
                        .bg(col(c_blue(), 0.12))
                        .into_any_element(),
                );
                // The size as it's drawn, under the box.
                out.push(
                    div()
                        .absolute()
                        .left(px(x + w / 2. - 40.))
                        .top(px(y + h + 6.))
                        .w(px(80.))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_1()
                                .rounded(px(3.))
                                .bg(rgb(c_blue()))
                                .text_size(px(10.5))
                                .text_color(rgb(0xffffff))
                                .child(format!(
                                    "{} × {}",
                                    (at.0 - from.0).abs().round(),
                                    (at.1 - from.1).abs().round()
                                )),
                        )
                        .into_any_element(),
                );
            }
        }
        if let Some((_, pts)) = &self.pen {
            for (i, p) in pts.iter().enumerate() {
                let (x, y) = self.screen_pt(*p);
                let s = if i == 0 { 9. } else { 7. };
                out.push(
                    div()
                        .absolute()
                        .left(px(x - s / 2.))
                        .top(px(y - s / 2.))
                        .size(px(s))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(c_blue()))
                        .bg(rgb(0xffffff))
                        .into_any_element(),
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{constrain, markup, Shape};

    #[test]
    fn shapes_are_drawn_where_they_were_dragged() {
        let (html, at) = markup(Shape::Rect, &[(40., 60.), (10., 20.)], false);
        assert_eq!(at, (10., 20.));
        assert!(html.contains("width:30px;height:40px"));
        // A click without a drag gives the usual size.
        let (html, _) = markup(Shape::Ellipse, &[(5., 5.), (5., 5.)], false);
        assert!(html.contains("width:100px") && html.contains("border-radius:50%"));
        let (html, at) = markup(Shape::Pen, &[(10., 10.), (30., 10.), (20., 30.)], true);
        assert_eq!(at, (8., 8.));
        assert!(html.contains("d=\"M2 2 L22 2 L12 22 Z\""));
        assert!(html.contains("fill:var(--surface-2)"));
        let (html, _) = markup(Shape::Arrow, &[(0., 0.), (100., 0.)], false);
        assert!(html.contains("fill:none") && html.matches('M').count() == 2);
        // Shift makes a square and snaps a line to 45°.
        assert_eq!(constrain(Shape::Rect, (0., 0.), (10., -4.)), (10., -10.));
        let (x, y) = constrain(Shape::Line, (0., 0.), (10., 1.));
        assert!((x - 10.05).abs() < 0.01 && y.abs() < 0.001);
    }
}
