//! Layout grids and guides: columns, rows or a square grid drawn over a screen, lines to
//! line things up against, and snapping to both while drawing, resizing and moving.

use super::figma::{caption, mini, sec};
use super::*;

/// How close, in screen pixels, an edge must come to a line to snap to it.
const SNAP: f32 = 6.;

/// The grid colour, as design tools draw it.
const GRID: u32 = 0xff4d6d;
const GUIDE: u32 = 0xf43f5e;

impl DesignCanvas {
    /// How tall a screen is drawn: its whole page, or its device when not drawn yet.
    pub(super) fn board_size(&self, s: &Screen) -> (f32, f32) {
        let (w, h) = s.device.size();
        (
            w,
            self.shots.get(&s.id).map_or(h, |shot| shot.size.1.max(h)),
        )
    }

    /// The lines on screen `id` an edge can snap to, down (`vertical`) or across: its
    /// guides, its grid's edges, and its own sides and middle.
    fn snap_lines(&self, id: &str, vertical: bool) -> Vec<f32> {
        let Some(s) = self.canvas.screen(id) else {
            return vec![];
        };
        let (w, h) = self.board_size(s);
        let span = if vertical { w } else { h };
        let mut out = vec![0., span / 2., span];
        out.extend(
            s.guides
                .iter()
                .filter(|g| g.vertical == vertical)
                .map(|g| g.at),
        );
        if let Some(grid) = s.grid.as_ref().filter(|_| self.show_grids) {
            match (grid.kind.as_str(), vertical) {
                ("columns", true) | ("rows", false) => {
                    let span = if vertical { w } else { s.device.size().1 };
                    for (a, len) in grid.bands(span) {
                        out.extend([a, a + len]);
                    }
                }
                ("square", _) if grid.size >= 4. => {
                    let mut v = 0.;
                    while v <= span {
                        out.push(v);
                        v += grid.size;
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// How far to shift a thing whose edges sit at `edges` (page units, down or across) so
    /// the nearest one lies on a line; nothing when none is close.
    pub(super) fn snap(&self, id: &str, vertical: bool, edges: &[f32]) -> f32 {
        let reach = SNAP / self.zoom;
        let mut best: Option<f32> = None;
        for line in self.snap_lines(id, vertical) {
            for e in edges {
                let d = line - e;
                if d.abs() <= reach && best.is_none_or(|b| d.abs() < b.abs()) {
                    best = Some(d);
                }
            }
        }
        best.unwrap_or(0.)
    }

    /// A point in canvas units moved onto the nearest lines of screen `id`.
    pub(super) fn snap_point(&self, id: &str, w: Pt) -> Pt {
        let Some(s) = self.canvas.screen(id) else {
            return w;
        };
        let (x, y) = (w.0 - s.x, w.1 - s.y);
        (
            w.0 + self.snap(id, true, &[x]),
            w.1 + self.snap(id, false, &[y]),
        )
    }

    /// Whether the picked element floats, so dragging it moves it rather than reordering it.
    pub(super) fn floating(&self) -> bool {
        self.element
            .as_ref()
            .is_some_and(|e| matches!(e.style("position").trim(), "absolute" | "fixed"))
    }

    /// Move the floating picked element by (dx, dy), keeping the sides it's anchored to.
    pub(super) fn move_floating(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        let Some(el) = &self.element else {
            return;
        };
        let px_of = |k: &str| {
            el.inline
                .get(k)
                .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
        };
        let mut map = serde_json::Map::new();
        let mut put = |k: &str, v: f32| {
            map.insert(k.into(), json!(format!("{}px", v.round())));
        };
        match (px_of("left"), px_of("right")) {
            (None, Some(r)) => put("right", r - dx),
            (l, _) => put("left", l.unwrap_or(0.) + dx),
        }
        match (px_of("top"), px_of("bottom")) {
            (None, Some(b)) => put("bottom", b - dy),
            (t, _) => put("top", t.unwrap_or(0.) + dy),
        }
        if dx.abs() + dy.abs() > 0.5 {
            self.op("styles", &[serde_json::Value::Object(map)], After::Same, cx);
        }
    }

    /// Change the picked screen's grid and guides, and keep it.
    fn with_screen(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Screen)) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        if let Some(s) = self.canvas.screen_mut(&id) {
            f(s);
        }
        self.save_canvas();
        cx.notify();
    }

    /// The grids and guides drawn over every screen.
    pub(super) fn grid_marks(&self) -> Vec<AnyElement> {
        let z = self.zoom;
        let mut out = vec![];
        for s in &self.canvas.screens {
            let (w, h) = self.board_size(s);
            let (x0, y0) = self.screen_pt((s.x, s.y));
            if let Some(g) = s.grid.as_ref().filter(|_| self.show_grids) {
                match g.kind.as_str() {
                    "columns" => {
                        for (a, len) in g.bands(w) {
                            out.push(
                                div()
                                    .absolute()
                                    .left(px(x0 + a * z))
                                    .top(px(y0))
                                    .w(px(len * z))
                                    .h(px(h * z))
                                    .bg(col(GRID, 0.1))
                                    .into_any_element(),
                            );
                        }
                    }
                    "rows" => {
                        for (a, len) in g.bands(s.device.size().1) {
                            out.push(
                                div()
                                    .absolute()
                                    .left(px(x0))
                                    .top(px(y0 + a * z))
                                    .w(px(w * z))
                                    .h(px(len * z))
                                    .bg(col(GRID, 0.1))
                                    .into_any_element(),
                            );
                        }
                    }
                    "square" if g.size * z >= 3. => {
                        let mut v = g.size;
                        while v < w {
                            out.push(line(x0 + v * z, y0, 1., h * z, col(GRID, 0.18)));
                            v += g.size;
                        }
                        let mut v = g.size;
                        while v < h {
                            out.push(line(x0, y0 + v * z, w * z, 1., col(GRID, 0.18)));
                            v += g.size;
                        }
                    }
                    _ => {}
                }
            }
            for (i, g) in s.guides.iter().enumerate() {
                let at = self.guide_at(&s.id, i).unwrap_or(g.at);
                out.push(if g.vertical {
                    line(x0 + at * z, y0, 1., h * z, rgb(GUIDE))
                } else {
                    line(x0, y0 + at * z, w * z, 1., rgb(GUIDE))
                });
            }
            if let Drag::Guide { id, index, at, .. } = &self.drag {
                if *id == s.id {
                    if let Some(g) = s.guides.get(*index) {
                        let (lx, ly) = if g.vertical {
                            (x0 + at * z + 4., y0 + 4.)
                        } else {
                            (x0 + 4., y0 + at * z + 4.)
                        };
                        out.push(
                            div()
                                .absolute()
                                .left(px(lx))
                                .top(px(ly))
                                .px_1()
                                .rounded_sm()
                                .bg(rgb(GUIDE))
                                .text_size(px(10.5))
                                .text_color(rgb(0xffffff))
                                .child(format!("{}", at.round()))
                                .into_any_element(),
                        );
                    }
                }
            }
        }
        out
    }

    /// Where guide `index` of screen `id` is while it's being dragged.
    fn guide_at(&self, id: &str, index: usize) -> Option<f32> {
        match &self.drag {
            Drag::Guide {
                id: d,
                index: i,
                at,
                ..
            } if d == id && *i == index => Some(*at),
            _ => None,
        }
    }

    /// The guide under the pointer on the picked screen, to drag.
    pub(super) fn guide_hit(&self, w: Pt) -> Option<(String, usize, bool)> {
        let id = self.screen.as_ref()?;
        let s = self.canvas.screen(id)?;
        let (bw, bh) = self.board_size(s);
        let (x, y) = (w.0 - s.x, w.1 - s.y);
        if x < -4. || y < -4. || x > bw + 4. || y > bh + 4. {
            return None;
        }
        let reach = 4. / self.zoom;
        s.guides.iter().enumerate().find_map(|(i, g)| {
            let near = if g.vertical {
                (x - g.at).abs() <= reach
            } else {
                (y - g.at).abs() <= reach
            };
            near.then(|| (id.clone(), i, g.vertical))
        })
    }

    /// Drop a dragged guide: where it was let go, or off the screen to remove it.
    pub(super) fn drop_guide(&mut self, id: String, index: usize, at: f32, cx: &mut Context<Self>) {
        let Some(s) = self.canvas.screen(&id) else {
            return;
        };
        let (w, h) = self.board_size(s);
        self.with_screen(cx, |s| {
            let Some(g) = s.guides.get_mut(index) else {
                return;
            };
            let span = if g.vertical { w } else { h };
            if at < 0. || at > span {
                s.guides.remove(index);
            } else {
                g.at = at.round();
            }
        });
    }

    /// The Layout grid and Guides sections for the picked screen.
    pub(super) fn grid_sections(&self, s: &Screen, cx: &mut Context<Self>) -> Div {
        let kind = s.grid.as_ref().map(|g| g.kind.clone()).unwrap_or_default();
        let mut kinds = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, k)) in [
            ("None", ""),
            ("Columns", "columns"),
            ("Rows", "rows"),
            ("Square", "square"),
        ]
        .into_iter()
        .enumerate()
        {
            kinds = kinds.child(
                chip_btn(("design-grid-kind", i), label, kind == k).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.show_grids = true;
                        this.with_screen(cx, |s| {
                            s.grid = (!k.is_empty()).then(|| Grid {
                                kind: k.into(),
                                ..s.grid.clone().unwrap_or_else(Grid::columns)
                            });
                        })
                    },
                )),
            );
        }
        let mut grid = sec(
            "Layout grid",
            Some(
                mini(
                    "design-grid-show",
                    if self.show_grids {
                        IconName::Eye
                    } else {
                        IconName::EyeOff
                    },
                    "Show grids (⇧G)",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_grids = !this.show_grids;
                    cx.notify();
                }))
                .into_any_element(),
            ),
        )
        .child(kinds);
        if let Some(g) = &s.grid {
            let chips = |id: &'static str,
                         label: &'static str,
                         values: &[f32],
                         now: f32,
                         set: fn(&mut Grid, f32),
                         cx: &mut Context<Self>| {
                let mut row = div().flex().flex_wrap().gap(px(4.));
                for (i, v) in values.iter().copied().enumerate() {
                    row = row.child(
                        chip_btn((id, i), format!("{v}"), (now - v).abs() < 0.01).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.with_screen(cx, |s| {
                                    if let Some(g) = s.grid.as_mut() {
                                        set(g, v);
                                    }
                                })
                            }),
                        ),
                    );
                }
                label_row(label, row)
            };
            if g.kind == "square" {
                grid = grid.child(chips(
                    "design-grid-size",
                    "Size",
                    &[4., 8., 16., 24.],
                    g.size,
                    |g, v| g.size = v,
                    cx,
                ));
            } else {
                grid = grid
                    .child(chips(
                        "design-grid-count",
                        if g.kind == "rows" { "Rows" } else { "Columns" },
                        &[2., 3., 4., 6., 8., 12.],
                        g.count as f32,
                        |g, v| g.count = v as u32,
                        cx,
                    ))
                    .child(chips(
                        "design-grid-gutter",
                        "Gutter",
                        &[8., 16., 24., 32.],
                        g.gutter,
                        |g, v| g.gutter = v,
                        cx,
                    ))
                    .child(chips(
                        "design-grid-margin",
                        "Margin",
                        &[0., 16., 24., 32., 64.],
                        g.margin,
                        |g, v| g.margin = v,
                        cx,
                    ));
            }
        }
        let (w, h) = self.board_size(s);
        let add = |id: &'static str, label: &'static str, vertical: bool, at: f32| {
            text_btn(id, label, Some(IconName::Plus), false).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.with_screen(cx, |s| s.guides.push(Guide { vertical, at }))
                },
            ))
        };
        let mut guides = sec("Guides", None).child(
            div()
                .flex()
                .gap_1()
                .child(add("design-guide-v", "Down", true, (w / 2.).round()))
                .child(add(
                    "design-guide-h",
                    "Across",
                    false,
                    (h.min(s.device.size().1) / 2.).round(),
                )),
        );
        if s.guides.is_empty() {
            guides = guides.child(caption(
                "Drag a guide on the screen to move it, or off the screen to remove it. Shapes, sizes and floating layers snap to guides and the grid.",
            ));
        }
        for (i, g) in s.guides.iter().enumerate() {
            guides = guides.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .child(div().flex_1().text_color(rgb(c_text())).child(format!(
                        "{} {}px",
                        if g.vertical { "Down at" } else { "Across at" },
                        g.at
                    )))
                    .child(
                        mini(("design-guide-del", i), IconName::Trash, "Remove", false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.with_screen(cx, |s| {
                                    if i < s.guides.len() {
                                        s.guides.remove(i);
                                    }
                                })
                            }),
                        ),
                    ),
            );
        }
        div().v_flex().child(grid).child(guides)
    }
}

fn line(x: f32, y: f32, w: f32, h: f32, color: Rgba) -> AnyElement {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .bg(color)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use crate::designdraw::model::Grid;

    #[test]
    fn columns_share_the_width_between_margins_and_gutters() {
        let g = Grid::columns();
        // 390 wide: 16 each side and three 16 gutters leave 4 columns of 77.5.
        let bands = g.bands(390.);
        assert_eq!(bands.len(), 4);
        assert_eq!(bands[0], (16., 77.5));
        assert!((bands[3].0 + bands[3].1 - 374.).abs() < 0.01);
        let tight = Grid {
            count: 12,
            gutter: 40.,
            ..Grid::columns()
        };
        assert!(tight.bands(390.).is_empty());
    }
}
