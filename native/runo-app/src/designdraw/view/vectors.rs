//! Vector editing: joining, subtracting, intersecting or excluding shapes, turning a box
//! into a path, and moving, adding and removing a path's points.

use super::figma::{caption, sec};
use super::*;
use crate::designdraw::vector::{self, Contour, Op, Outline};

/// A path whose points are being edited: its screen and layer, where its units start on
/// the page, and its contours in those units.
pub(super) struct PointEdit {
    pub id: String,
    pub path: dom::Path,
    pub origin: Pt,
    pub contours: Vec<Contour>,
}

/// How close, in screen pixels, a press must be to a point to take it.
const HIT: f32 = 7.;

/// An svg for contours in page units, with the given fill and stroke, and where its top-left
/// corner goes.
fn svg_for(name: &str, contours: &[Contour], fill: &str, stroke: &str) -> Option<(String, Pt)> {
    let b = vector::bounds(contours)?;
    let (w, h) = ((b[2] - b[0]).ceil().max(1.), (b[3] - b[1]).ceil().max(1.));
    let d = vector::to_d(contours, [b[0], b[1]]);
    let fill = match fill.trim() {
        "" | "none" | "transparent" | "rgba(0, 0, 0, 0)" => "var(--surface-2)",
        f => f,
    };
    let stroke = match stroke.trim() {
        "" | "none" => String::new(),
        s => format!("stroke:{s};stroke-width:2px;"),
    };
    Some((
        format!(
            "<svg data-name=\"{name}\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"none\" \
             style=\"fill:{fill};{stroke}overflow:visible\"><path d=\"{d}\" \
             vector-effect=\"non-scaling-stroke\" fill-rule=\"evenodd\" stroke-linejoin=\"round\"/></svg>"
        ),
        (b[0] as f32, b[1] as f32),
    ))
}

/// Whether an element is a shape that can be combined: a drawing, or an empty box.
fn is_shape(el: &Element) -> bool {
    el.tag == "svg"
        || (el.tag == "div"
            && el.children == 0
            && el.text.as_deref().is_none_or(|t| t.trim().is_empty()))
}

impl DesignCanvas {
    /// Boolean buttons for several shapes, or Flatten and Edit points for one.
    pub(super) fn vector_tools(&self, el: &Element, cx: &mut Context<Self>) -> Option<Div> {
        let all: Vec<&Element> = std::iter::once(el).chain(&self.also).collect();
        if !all.iter().all(|e| is_shape(e)) {
            return None;
        }
        let mut row = div().flex().flex_wrap().gap(px(4.));
        if all.len() > 1 {
            for (i, op) in Op::ALL.into_iter().enumerate() {
                row = row.child(
                    text_btn(("design-bool", i), op.label(), None, false)
                        .on_click(cx.listener(move |this, _, _, cx| this.combine(Some(op), cx))),
                );
            }
            return Some(
                sec("Boolean", None)
                    .child(row)
                    .child(caption("Subtract takes the others out of the first one.")),
            );
        }
        row = row.child(
            text_btn("design-flatten", "Flatten", None, false)
                .on_click(cx.listener(|this, _, _, cx| this.combine(None, cx))),
        );
        if el.tag == "svg" {
            let editing = self.points.is_some();
            row = row.child(
                text_btn("design-points", "Edit points", None, editing).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if editing {
                            this.points = None;
                            cx.notify();
                        } else {
                            this.start_points(cx);
                        }
                    },
                )),
            );
        }
        Some(sec("Vector path", None).child(row).when(self.points.is_some(), |s| {
            s.child(caption(
                "Drag a point to move it, drag a small dot to add one, Alt-click a point to remove it. Enter or Escape to finish.",
            ))
        }))
    }

    /// Combine the picked shapes with `op`, or with none turn the one picked into a path.
    pub(super) fn combine(&mut self, op: Option<Op>, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        let Some(id) = self.screen.clone() else {
            return;
        };
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let js = dom::call("outlines", &[json!(paths)]);
        let (id2, paths2) = (id.clone(), paths.clone());
        self.run(
            cx,
            move |r| r.eval(&id, device, &js),
            move |this, out, cx| {
                let outlines: Vec<Outline> = out
                    .ok()
                    .and_then(|v| serde_json::from_value::<Vec<Option<Outline>>>(v).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .flatten()
                    .collect();
                let Some(first) = outlines.first() else {
                    return;
                };
                let shapes: Vec<Vec<Contour>> = outlines.iter().map(|o| o.contours()).collect();
                let result = vector::combine(op.unwrap_or(Op::Union), &shapes);
                let name = op.map_or("Vector", |o| o.label());
                let Some((html, (x, y))) = svg_for(name, &result, &first.fill, &first.stroke)
                else {
                    this.say("Those shapes don't overlap, so nothing is left.");
                    cx.notify();
                    return;
                };
                this.replace_shapes(id2, device, paths2, (x, y), html, cx);
            },
        );
    }

    fn replace_shapes(
        &mut self,
        id: String,
        device: Device,
        paths: Vec<dom::Path>,
        at: Pt,
        html: String,
        cx: &mut Context<Self>,
    ) {
        let js = dom::call(
            "replaceShapes",
            &[json!(paths), json!(at.0), json!(at.1), json!(html)],
        );
        let id2 = id.clone();
        self.points = None;
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

    /// Start editing the picked drawing's points.
    pub(super) fn start_points(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(el)) = (self.screen.clone(), self.element.clone()) else {
            return;
        };
        if el.tag != "svg" {
            return;
        }
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let js = dom::call("outlines", &[json!([el.path.clone()])]);
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.eval(&id, device, &js),
            move |this, out, cx| {
                let o: Option<Outline> = out
                    .ok()
                    .and_then(|v| serde_json::from_value::<Vec<Option<Outline>>>(v).ok())
                    .and_then(|mut v| v.pop().flatten());
                let Some(o) = o.filter(|o| o.kind == "path") else {
                    return;
                };
                let (ox, oy) = (o.rect.x as f64, o.rect.y as f64);
                let contours = o
                    .contours()
                    .into_iter()
                    .map(|c| Contour {
                        pts: c.pts.iter().map(|p| [p[0] - ox, p[1] - oy]).collect(),
                        closed: c.closed,
                    })
                    .collect();
                this.points = Some(PointEdit {
                    id: id2,
                    path: el.path,
                    origin: (o.rect.x, o.rect.y),
                    contours,
                });
                cx.notify();
            },
        );
    }

    /// The points being edited, and the dots between them, in canvas units: (contour,
    /// index, point, is a dot between).
    fn point_marks(&self) -> Vec<(usize, usize, Pt, bool)> {
        let Some(e) = &self.points else {
            return vec![];
        };
        let Some(s) = self.canvas.screen(&e.id) else {
            return vec![];
        };
        let (bx, by) = (s.x + e.origin.0, s.y + e.origin.1);
        let mut out = vec![];
        for (ci, c) in e.contours.iter().enumerate() {
            let n = c.pts.len();
            for (pi, p) in c.pts.iter().enumerate() {
                let moved = self
                    .dragged_point(ci, pi)
                    .unwrap_or((p[0] as f32, p[1] as f32));
                out.push((ci, pi, (bx + moved.0, by + moved.1), false));
                let next = if pi + 1 < n {
                    Some(pi + 1)
                } else if c.closed {
                    Some(0)
                } else {
                    None
                };
                // A dot halfway along each side, to add a point there. Too many dots on a
                // finely drawn curve get in the way, so they're left off.
                if let Some(q) = next.filter(|_| n <= 40) {
                    let q = c.pts[q];
                    out.push((
                        ci,
                        pi + 1,
                        (
                            bx + ((p[0] + q[0]) / 2.) as f32,
                            by + ((p[1] + q[1]) / 2.) as f32,
                        ),
                        true,
                    ));
                }
            }
        }
        out
    }

    /// Where point `pi` of contour `ci` is while dragged, in the path's units.
    fn dragged_point(&self, ci: usize, pi: usize) -> Option<(f32, f32)> {
        let Drag::Point {
            contour,
            index,
            from,
            at,
        } = &self.drag
        else {
            return None;
        };
        let e = self.points.as_ref()?;
        if (*contour, *index) != (ci, pi) {
            return None;
        }
        let p = e.contours.get(ci)?.pts.get(pi)?;
        Some((p[0] as f32 + at.0 - from.0, p[1] as f32 + at.1 - from.1))
    }

    /// A press while points are edited: take a point, add one at a dot, or remove one with
    /// Alt. Gives whether it was taken; a press elsewhere finishes editing.
    pub(super) fn points_down(&mut self, w: Pt, alt: bool, cx: &mut Context<Self>) -> bool {
        if self.points.is_none() {
            return false;
        }
        let reach = HIT / self.zoom;
        let hit = self
            .point_marks()
            .into_iter()
            .filter(|(_, _, p, _)| (p.0 - w.0).abs() <= reach && (p.1 - w.1).abs() <= reach)
            .min_by_key(|(_, _, _, dot)| *dot);
        let Some((ci, pi, _, dot)) = hit else {
            self.points = None;
            cx.notify();
            return false;
        };
        let Some(e) = self.points.as_mut() else {
            return false;
        };
        let c = &mut e.contours[ci];
        if alt && !dot {
            if c.pts.len() > 2 {
                c.pts.remove(pi);
                self.commit_points(cx);
            }
            return true;
        }
        if dot {
            let n = c.pts.len();
            let (a, b) = (c.pts[pi - 1], c.pts[pi % n]);
            c.pts.insert(pi, [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]);
        }
        self.drag = Drag::Point {
            contour: ci,
            index: pi,
            from: w,
            at: w,
        };
        cx.notify();
        true
    }

    /// Let go of a dragged point.
    pub(super) fn drop_point(
        &mut self,
        ci: usize,
        pi: usize,
        from: Pt,
        at: Pt,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self
            .points
            .as_mut()
            .and_then(|e| e.contours.get_mut(ci))
            .and_then(|c| c.pts.get_mut(pi))
        {
            p[0] += (at.0 - from.0) as f64;
            p[1] += (at.1 - from.1) as f64;
        }
        self.commit_points(cx);
    }

    /// Write the edited points back to the drawing, keeping its box tight around them.
    fn commit_points(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.points.as_mut() else {
            return;
        };
        let Some(b) = vector::bounds(&e.contours) else {
            return;
        };
        let (w, h) = ((b[2] - b[0]).ceil().max(1.), (b[3] - b[1]).ceil().max(1.));
        let d = vector::to_d(&e.contours, [b[0], b[1]]);
        for c in &mut e.contours {
            for p in &mut c.pts {
                *p = [p[0] - b[0], p[1] - b[1]];
            }
        }
        e.origin = (e.origin.0 + b[0] as f32, e.origin.1 + b[1] as f32);
        let (id, path) = (e.id.clone(), e.path.clone());
        if self.screen.as_deref() != Some(id.as_str()) {
            return;
        }
        self.edit(
            dom::op(
                &path,
                "reshape",
                &[json!(b[0]), json!(b[1]), json!(w), json!(h), json!(d)],
            ),
            After::Path(path.clone()),
            cx,
        );
    }

    /// The edited path's sides as canvas-pixel lines, for painting while a point moves.
    pub(super) fn point_lines(&self) -> Vec<(Vec<Pt>, bool)> {
        let Some(e) = &self.points else {
            return vec![];
        };
        let Some(s) = self.canvas.screen(&e.id) else {
            return vec![];
        };
        let (bx, by) = (s.x + e.origin.0, s.y + e.origin.1);
        e.contours
            .iter()
            .enumerate()
            .map(|(ci, c)| {
                let pts = c
                    .pts
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        let m = self
                            .dragged_point(ci, pi)
                            .unwrap_or((p[0] as f32, p[1] as f32));
                        self.screen_pt((bx + m.0, by + m.1))
                    })
                    .collect();
                (pts, c.closed)
            })
            .collect()
    }

    /// The handles for the points being edited.
    pub(super) fn point_handles(&self) -> Vec<AnyElement> {
        self.point_marks()
            .into_iter()
            .map(|(_, _, p, dot)| {
                let (x, y) = self.screen_pt(p);
                let s = if dot { 6. } else { 9. };
                div()
                    .absolute()
                    .left(px(x - s / 2.))
                    .top(px(y - s / 2.))
                    .size(px(s))
                    .when(dot, |d| d.rounded_full().bg(col(c_blue(), 0.5)))
                    .when(!dot, |d| {
                        d.rounded(px(2.))
                            .border_1()
                            .border_color(rgb(c_blue()))
                            .bg(rgb(0xffffff))
                    })
                    .into_any_element()
            })
            .collect()
    }
}
