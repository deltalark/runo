//! Spacing handles: on a selected auto layout, the gaps between its children and its
//! padding show as pink bands on the canvas, and dragging one sets that space, as in
//! Figma. And Tidy up, which lines up floating layers evenly.

use super::*;

/// Figma's spacing pink.
const PINK: u32 = 0xf24e6b;
/// The thinnest a band is drawn, in screen pixels, so a zero space can still be grabbed.
const GRAB: f32 = 6.;

/// One space a handle sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Space {
    /// The CSS property: `gap`, or a side's padding.
    pub key: &'static str,
    /// Dragged up and down, rather than left and right.
    pub vertical: bool,
    /// Which way of dragging makes it bigger: 1 or -1.
    pub sign: f32,
}

/// A band to drag, in the screen's CSS pixels, and what it sets.
#[derive(Debug, PartialEq)]
pub(super) struct Band {
    pub rect: dom::Box,
    pub space: Space,
}

/// The bands of a flex container `el`: its padding on each side, and the gap between each
/// pair of children next to each other along its direction.
pub(super) fn bands(el: &dom::Element, kids: &[dom::Box]) -> Vec<Band> {
    let r = &el.rect;
    let v = |k: &str| {
        el.computed
            .get(k)
            .and_then(|s| s.trim().strip_suffix("px")?.parse::<f32>().ok())
            .unwrap_or(0.)
    };
    let (pt, pr, pb, pl) = (
        v("padding-top"),
        v("padding-right"),
        v("padding-bottom"),
        v("padding-left"),
    );
    let side = |key, vertical, sign, rect| Band {
        rect,
        space: Space {
            key,
            vertical,
            sign,
        },
    };
    let mut out = vec![
        side(
            "padding-top",
            true,
            1.,
            dom::Box {
                x: r.x + pl,
                y: r.y,
                w: r.w - pl - pr,
                h: pt,
            },
        ),
        side(
            "padding-bottom",
            true,
            -1.,
            dom::Box {
                x: r.x + pl,
                y: r.y + r.h - pb,
                w: r.w - pl - pr,
                h: pb,
            },
        ),
        side(
            "padding-left",
            false,
            1.,
            dom::Box {
                x: r.x,
                y: r.y + pt,
                w: pl,
                h: r.h - pt - pb,
            },
        ),
        side(
            "padding-right",
            false,
            -1.,
            dom::Box {
                x: r.x + r.w - pr,
                y: r.y + pt,
                w: pr,
                h: r.h - pt - pb,
            },
        ),
    ];
    let row = !el
        .computed
        .get("flex-direction")
        .is_some_and(|d| d.starts_with("column"));
    let mut kids: Vec<&dom::Box> = kids.iter().filter(|k| k.w > 0. && k.h > 0.).collect();
    kids.sort_by(|a, b| {
        let (x, y) = if row { (a.x, b.x) } else { (a.y, b.y) };
        x.total_cmp(&y)
    });
    for pair in kids.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let rect = if row {
            // Only children side by side: a wrapped row starts again at the left.
            if b.x < a.x + a.w - 0.5 {
                continue;
            }
            dom::Box {
                x: a.x + a.w,
                y: r.y + pt,
                w: b.x - a.x - a.w,
                h: r.h - pt - pb,
            }
        } else {
            if b.y < a.y + a.h - 0.5 {
                continue;
            }
            dom::Box {
                x: r.x + pl,
                y: a.y + a.h,
                w: r.w - pl - pr,
                h: b.y - a.y - a.h,
            }
        };
        // Each gap drags the same `gap`: one band for the first is enough to set it, but
        // all show, as the space they are.
        out.push(side("gap", !row, 1., rect));
    }
    out.retain(|b| b.rect.w >= 0. && b.rect.h >= 0.);
    out
}

/// What a drag of `by` CSS pixels along the band's axis makes the space, from `start`:
/// whole pixels, or steps of `step` with Shift.
pub(super) fn dragged(space: Space, start: f32, by: f32, step: Option<f32>) -> f32 {
    let v = (start + by * space.sign).max(0.);
    match step.filter(|s| *s > 0.) {
        Some(s) => (v / s).round() * s,
        None => v.round(),
    }
}

impl DesignCanvas {
    /// The selected auto layout's children, as their boxes on the screen.
    fn kid_boxes(&self, id: &str, el: &dom::Element) -> Vec<dom::Box> {
        self.shots
            .get(id)
            .map(|s| {
                s.boxes
                    .iter()
                    .filter(|b| b.path.len() == el.path.len() + 1 && b.path.starts_with(&el.path))
                    .map(|b| b.rect.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The bands of the one selected layer, when it lays out what it holds.
    fn selected_bands(&self) -> Option<(String, Vec<Band>)> {
        let el = self.element.as_ref().filter(|e| e.is_flex())?;
        if !self.also.is_empty() || self.ai.is_some() || self.playing.is_some() {
            return None;
        }
        let id = self.screen.clone()?;
        let kids = self.kid_boxes(&id, el);
        Some((id, bands(el, &kids)))
    }

    /// Start dragging the band under `w`, in canvas units. Gives whether one was there.
    pub(super) fn spacing_down(&mut self, w: Pt, cx: &mut Context<Self>) -> bool {
        let Some((id, bands)) = self.selected_bands() else {
            return false;
        };
        let Some(s) = self.canvas.screen(&id) else {
            return false;
        };
        let (x, y) = (w.0 - s.x, w.1 - s.y);
        let slack = GRAB / 2. / self.zoom;
        let hit = bands.into_iter().find(|b| {
            let r = &b.rect;
            let (sx, sy) = if b.space.vertical {
                (0., slack.max(r.h / 2.) - r.h / 2.)
            } else {
                (slack.max(r.w / 2.) - r.w / 2., 0.)
            };
            x >= r.x - sx && x <= r.x + r.w + sx && y >= r.y - sy && y <= r.y + r.h + sy
        });
        let Some(b) = hit else {
            return false;
        };
        let start = self
            .element
            .as_ref()
            .and_then(|e| e.computed.get(b.space.key))
            .and_then(|v| v.trim().strip_suffix("px")?.parse::<f32>().ok())
            .unwrap_or(0.);
        self.drag = Drag::Spacing {
            space: b.space,
            from: w,
            start,
            now: start,
        };
        cx.notify();
        true
    }

    /// Set the space dragged, as one edit.
    pub(super) fn spacing_up(
        &mut self,
        space: Space,
        start: f32,
        now: f32,
        cx: &mut Context<Self>,
    ) {
        if (now - start).abs() < 0.5 {
            return;
        }
        let value = format!("{}px", now.round());
        if space.key == "gap" {
            // A gap set one way shouldn't leave the other axis's gap behind.
            self.set_styles(&[("gap", &value)], cx);
        } else {
            self.set_style(space.key, &value, cx);
        }
    }

    /// The bands over the selected auto layout, and while dragging, the space as it'll be.
    pub(super) fn spacing_marks(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let Some((id, bands)) = self.selected_bands() else {
            return out;
        };
        let Some(s) = self.canvas.screen(&id) else {
            return out;
        };
        let z = self.zoom;
        let dragging = match &self.drag {
            Drag::Spacing { space, now, .. } => Some((*space, *now)),
            Drag::None => None,
            _ => return out,
        };
        for (i, b) in bands.iter().enumerate() {
            let r = &b.rect;
            let (x, y) = self.screen_pt((s.x + r.x, s.y + r.y));
            let (mut w, mut h) = (r.w * z, r.h * z);
            let (mut x, mut y) = (x, y);
            if b.space.vertical && h < GRAB {
                y -= (GRAB - h) / 2.;
                h = GRAB;
            }
            if !b.space.vertical && w < GRAB {
                x -= (GRAB - w) / 2.;
                w = GRAB;
            }
            let on = dragging.is_some_and(|(sp, _)| sp == b.space);
            let mid = div()
                .absolute()
                .when(b.space.vertical, |d| {
                    d.left(px(w / 2. - 8.))
                        .top(px(h / 2. - 1.))
                        .w(px(16.))
                        .h(px(2.))
                })
                .when(!b.space.vertical, |d| {
                    d.left(px(w / 2. - 1.))
                        .top(px(h / 2. - 8.))
                        .w(px(2.))
                        .h(px(16.))
                })
                .bg(rgb(PINK));
            out.push(
                div()
                    .id(("design-space", i))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h))
                    .when(b.space.vertical, |d| d.cursor_row_resize())
                    .when(!b.space.vertical, |d| d.cursor_col_resize())
                    .when(on, |d| d.bg(col(PINK, 0.25)))
                    .hover(|d| d.bg(col(PINK, 0.18)))
                    .group("space")
                    .child(mid.invisible().group_hover("space", |d| d.visible()))
                    .tooltip(tip(match b.space.key {
                        "gap" => "Drag to change the gap",
                        _ => "Drag to change the padding",
                    }))
                    .into_any_element(),
            );
        }
        if let (Some((space, now)), Drag::Spacing { from, .. }) = (dragging, &self.drag) {
            let (fx, fy) = self.screen_pt(*from);
            let word = match space.key {
                "gap" => "Gap",
                _ => "Padding",
            };
            out.push(
                div()
                    .absolute()
                    .left(px(fx + 12.))
                    .top(px(fy + 12.))
                    .px_1()
                    .rounded_sm()
                    .bg(rgb(PINK))
                    .text_size(px(10.5))
                    .text_color(rgb(0xffffff))
                    .child(format!("{word} {}", now.round()))
                    .into_any_element(),
            );
        }
        let _ = cx;
        out
    }

    /// Line the selected floating layers up evenly, in a row or a column, whichever way
    /// they spread more.
    pub(super) fn tidy_up(&mut self, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if paths.len() < 2 {
            return;
        }
        let Some(id) = self.screen.clone() else {
            return;
        };
        let Some(device) = self.canvas.screen(&id).map(|s| s.device) else {
            return;
        };
        let js = dom::call("tidy", &[json!(paths)]);
        let id2 = id.clone();
        self.run(
            cx,
            move |r| r.edit(&id, device, &js),
            move |this, out, cx| match out {
                Ok(Some((before, shot))) => this.landed(id2, before, shot, After::Paths(paths), cx),
                Ok(None) => {
                    this.say("Tidy up lines up floating layers. These are laid out already.");
                    cx.notify();
                }
                Err(e) => this.say(e),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn row(direction: &str) -> dom::Element {
        let mut el = dom::Element {
            rect: dom::Box {
                x: 0.,
                y: 0.,
                w: 300.,
                h: 60.,
            },
            ..Default::default()
        };
        for (k, v) in [
            ("display", "flex"),
            ("flex-direction", direction),
            ("padding-top", "10px"),
            ("padding-bottom", "10px"),
            ("padding-left", "16px"),
            ("padding-right", "16px"),
            ("gap", "8px"),
        ] {
            el.computed.insert(k.into(), v.into());
        }
        el
    }

    fn b(x: f32, y: f32, w: f32, h: f32) -> dom::Box {
        dom::Box { x, y, w, h }
    }

    #[test]
    fn a_row_has_its_padding_and_a_gap_between_each_pair() {
        let kids = [
            b(16., 10., 100., 40.),
            b(124., 10., 60., 40.),
            b(192., 10., 0., 0.),
        ];
        let found = bands(&row("row"), &kids);
        let keys: Vec<&str> = found.iter().map(|b| b.space.key).collect();
        assert_eq!(
            keys,
            [
                "padding-top",
                "padding-bottom",
                "padding-left",
                "padding-right",
                "gap"
            ]
        );
        let gap = &found[4];
        assert_eq!(gap.rect, b(116., 10., 8., 40.));
        assert!(!gap.space.vertical);
        assert_eq!(found[1].rect, b(16., 50., 268., 10.));
        assert_eq!(found[1].space.sign, -1.);
    }

    #[test]
    fn a_column_drags_its_gap_up_and_down_and_skips_wrapped_rows() {
        let kids = [b(16., 10., 268., 20.), b(16., 38., 268., 12.)];
        let found = bands(&row("column"), &kids);
        assert_eq!(found[4].rect, b(16., 30., 268., 8.));
        assert!(found[4].space.vertical);
        let wrapped = [b(16., 10., 200., 20.), b(16., 30., 100., 20.)];
        assert_eq!(bands(&row("row"), &wrapped).len(), 4);
    }

    #[test]
    fn drags_round_and_stop_at_zero() {
        let s = Space {
            key: "padding-bottom",
            vertical: true,
            sign: -1.,
        };
        assert_eq!(dragged(s, 10., -5.4, None), 15.);
        assert_eq!(dragged(s, 10., 30., None), 0.);
        assert_eq!(dragged(s, 10., -9., Some(8.)), 16.);
    }
}
