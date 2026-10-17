//! Motion in the prototype: how a move eases, Smart animate, which carries layers that
//! are on both screens from where they were to where they land, and links that follow
//! themselves after a delay.

use super::*;

/// How a move eases from start to end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Ease {
    /// Fast, then settling: the usual for things coming on screen.
    #[default]
    Out,
    InOut,
    Linear,
    /// Goes a little past the end and settles back.
    Spring,
}

/// The eases a link can pick, with their `data-ease` values.
pub(crate) const EASES: [(&str, &str); 4] = [
    ("Ease out", ""),
    ("Ease in and out", "in-out"),
    ("Linear", "linear"),
    ("Spring", "spring"),
];

/// How long a link can wait before it follows itself, as `data-delay`.
pub(crate) const DELAYS: [(&str, &str); 4] = [
    ("0.3s", "300"),
    ("0.8s", "800"),
    ("1.5s", "1500"),
    ("3s", "3000"),
];

impl Ease {
    pub(crate) fn of(value: &str) -> Self {
        match value {
            "in-out" => Ease::InOut,
            "linear" => Ease::Linear,
            "spring" => Ease::Spring,
            _ => Ease::Out,
        }
    }

    /// How far along the move is at time `t`, both from 0 to 1.
    pub(crate) fn at(self, t: f32) -> f32 {
        let t = t.clamp(0., 1.);
        match self {
            Ease::Out => 1. - (1. - t).powi(3),
            Ease::InOut if t < 0.5 => 4. * t * t * t,
            Ease::InOut => 1. - (-2. * t + 2.).powi(3) / 2.,
            Ease::Linear => t,
            Ease::Spring => {
                let (c1, c3) = (1.70158, 2.70158);
                1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
            }
        }
    }
}

/// The layers on both screens, by name, id or text, that Smart animate carries across:
/// where each is on the screen being left and where it lands. A layer inside one that is
/// carried goes with it.
pub(crate) fn pairs(old: &[dom::Bx], new: &[dom::Bx]) -> Vec<(dom::Box, dom::Box)> {
    fn count(boxes: &[dom::Bx]) -> HashMap<&str, usize> {
        let mut n: HashMap<&str, usize> = HashMap::new();
        for b in boxes {
            if let Some(k) = &b.key {
                *n.entry(k.as_str()).or_default() += 1;
            }
        }
        n
    }
    let (in_old, in_new) = (count(old), count(new));
    let mut carried: Vec<&dom::Path> = vec![];
    let mut out = vec![];
    for b in new {
        let Some(k) = b.key.as_deref() else {
            continue;
        };
        if in_old.get(k) != Some(&1) || in_new.get(k) != Some(&1) {
            continue;
        }
        if carried.iter().any(|p| b.path.starts_with(p)) {
            continue;
        }
        let Some(a) = old.iter().find(|o| o.key.as_deref() == Some(k)) else {
            continue;
        };
        carried.push(&b.path);
        let same = (a.rect.x - b.rect.x).abs() < 0.5
            && (a.rect.y - b.rect.y).abs() < 0.5
            && (a.rect.w - b.rect.w).abs() < 0.5
            && (a.rect.h - b.rect.h).abs() < 0.5;
        if !same && out.len() < 40 {
            out.push((a.rect.clone(), b.rect.clone()));
        }
    }
    out
}

impl DesignCanvas {
    /// The layers Smart animate moves from screen `from` to screen `to`, drawn at `scale`
    /// over the new screen as it fades in.
    pub(super) fn smart_layers(
        &self,
        from: &str,
        to: &str,
        how: Move,
        scale: f32,
        width: f32,
    ) -> Vec<AnyElement> {
        let shot = |id: &str| self.play_shots.get(id).or_else(|| self.shots.get(id));
        let (Some(old), Some(new)) = (shot(from), shot(to)) else {
            return vec![];
        };
        let tall = new.size.1 * scale;
        let n = self.play_n;
        pairs(&old.boxes, &new.boxes)
            .into_iter()
            .enumerate()
            .map(|(i, (a, b))| {
                let lerp = move |x: f32, y: f32, d: f32| (x + (y - x) * d) * scale;
                div()
                    .absolute()
                    .left(px(a.x * scale))
                    .top(px(a.y * scale))
                    .w(px(a.w * scale))
                    .h(px(a.h * scale))
                    .overflow_hidden()
                    .child(
                        img(ImageSource::Render(new.image.clone()))
                            .absolute()
                            .left(px(-b.x * scale))
                            .top(px(-b.y * scale))
                            .w(px(width))
                            .h(px(tall)),
                    )
                    .with_animation(
                        ("design-smart", n * 64 + i),
                        how.animation(),
                        move |el, d| {
                            el.left(px(lerp(a.x, b.x, d)))
                                .top(px(lerp(a.y, b.y, d)))
                                .w(px(lerp(a.w, b.w, d).max(0.)))
                                .h(px(lerp(a.h, b.h, d).max(0.)))
                        },
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// When the screen playing has a link that follows itself after a delay, follow it then,
    /// unless Present has moved on by that time.
    pub(super) fn arm_delay(&mut self, cx: &mut Context<Self>) {
        self.play_visit += 1;
        let Some(id) = self.playing.clone() else {
            return;
        };
        let shot = self.play_shots.get(&id).or_else(|| self.shots.get(&id));
        let Some(link) = shot
            .and_then(|s| s.links.iter().find(|l| l.trigger == "delay"))
            .cloned()
        else {
            return;
        };
        let moving = self.play_move.as_ref().map_or(0, |m| m.1.ms);
        let wait = Duration::from_millis(link.delay + moving);
        let visit = self.play_visit;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                if this.play_visit == visit && this.playing.as_deref() == Some(id.as_str()) {
                    this.play_go(&link, cx);
                }
            });
        })
        .detach();
    }

    /// What starts the selected element's link, and how long it waits.
    pub(super) fn trigger_rows(&self, el: &dom::Element, cx: &mut Context<Self>) -> Div {
        let delayed = el.trigger.as_deref() == Some("delay");
        let mut kinds = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, value)) in [("On click", ""), ("After delay", "delay")]
            .into_iter()
            .enumerate()
        {
            let on = delayed == (value == "delay");
            let has_delay = el.delay.is_some();
            kinds = kinds.child(
                chip_btn(("design-trigger", i), label, on).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.op(
                            "attr",
                            &[json!("data-trigger"), json!(value)],
                            After::Same,
                            cx,
                        );
                        if value == "delay" && !has_delay {
                            this.op(
                                "attr",
                                &[json!("data-delay"), json!("800")],
                                After::Same,
                                cx,
                            );
                        }
                    },
                )),
            );
        }
        let mut rows = div().v_flex().gap(px(6.)).child(kinds);
        if delayed {
            let now = el.delay.clone().unwrap_or_default();
            let mut waits = div().flex().flex_wrap().gap(px(4.));
            for (i, (label, value)) in DELAYS.iter().enumerate() {
                let value: &'static str = value;
                waits = waits.child(
                    chip_btn(("design-delay", i), *label, now == value).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.op(
                                "attr",
                                &[json!("data-delay"), json!(value)],
                                After::Same,
                                cx,
                            )
                        },
                    )),
                );
            }
            rows = rows.child(waits);
        }
        rows
    }

    /// How the selected element's link eases.
    pub(super) fn ease_row(&self, el: &dom::Element, cx: &mut Context<Self>) -> Div {
        let now = el.ease.clone().unwrap_or_default();
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, value)) in EASES.iter().enumerate() {
            let value: &'static str = value;
            row = row.child(chip_btn(("design-ease", i), *label, now == value).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.op("attr", &[json!("data-ease"), json!(value)], After::Same, cx)
                }),
            ));
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::{pairs, Ease};
    use crate::designdraw::dom::{Box, Bx};

    fn bx(path: Vec<usize>, key: &str, x: f32, y: f32) -> Bx {
        Bx {
            path,
            rect: Box {
                x,
                y,
                w: 50.,
                h: 20.,
            },
            key: Some(key.into()),
            ..Default::default()
        }
    }

    #[test]
    fn smart_animate_carries_layers_found_on_both_screens() {
        let old = vec![
            bx(vec![0], "Card", 0., 100.),
            bx(vec![0, 0], "p:Title", 10., 110.),
            bx(vec![1], "Logo", 0., 0.),
            bx(vec![2], "Gone", 0., 300.),
        ];
        let new = vec![
            bx(vec![3], "Card", 0., 40.),
            bx(vec![3, 0], "p:Title", 10., 50.),
            bx(vec![0], "Logo", 0., 0.),
            bx(vec![1], "Fresh", 0., 300.),
        ];
        let moved = pairs(&old, &new);
        // The card moves, its title goes with it, the logo stays put, and the rest fades.
        assert_eq!(moved.len(), 1);
        assert_eq!((moved[0].0.y, moved[0].1.y), (100., 40.));
    }

    #[test]
    fn eases_start_at_zero_and_end_at_one() {
        for e in [Ease::Out, Ease::InOut, Ease::Linear, Ease::Spring] {
            assert!(e.at(0.).abs() < 1e-4 && (e.at(1.) - 1.).abs() < 1e-4);
        }
        assert!(Ease::Spring.at(0.8) > 1.);
        assert_eq!(Ease::of("in-out"), Ease::InOut);
    }
}
