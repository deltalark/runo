//! Editing a picture: how it fits its box, which part of it shows and how close, its colour
//! filters, and swapping it for another picture in the design.

use super::figma::{caption, mini};
use super::*;

/// A picture's colour adjustments, as CSS filter functions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Adjust {
    pub brightness: f32,
    pub contrast: f32,
    pub saturate: f32,
    pub grayscale: f32,
    pub sepia: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            brightness: 1.,
            contrast: 1.,
            saturate: 1.,
            grayscale: 0.,
            sepia: 0.,
        }
    }
}

/// Ready-made looks, as filters.
pub(crate) const LOOKS: [(&str, Adjust); 6] = [
    (
        "Original",
        Adjust {
            brightness: 1.,
            contrast: 1.,
            saturate: 1.,
            grayscale: 0.,
            sepia: 0.,
        },
    ),
    (
        "Vivid",
        Adjust {
            brightness: 1.,
            contrast: 1.1,
            saturate: 1.5,
            grayscale: 0.,
            sepia: 0.,
        },
    ),
    (
        "Warm",
        Adjust {
            brightness: 1.05,
            contrast: 1.,
            saturate: 1.2,
            grayscale: 0.,
            sepia: 0.3,
        },
    ),
    (
        "Faded",
        Adjust {
            brightness: 1.1,
            contrast: 0.85,
            saturate: 0.8,
            grayscale: 0.,
            sepia: 0.,
        },
    ),
    (
        "Mono",
        Adjust {
            brightness: 1.,
            contrast: 1.1,
            saturate: 1.,
            grayscale: 1.,
            sepia: 0.,
        },
    ),
    (
        "Sepia",
        Adjust {
            brightness: 1.,
            contrast: 1.,
            saturate: 1.,
            grayscale: 0.,
            sepia: 0.8,
        },
    ),
];

impl Adjust {
    /// Read the adjustments from a `filter` value; anything else in it is dropped.
    pub(crate) fn parse(filter: &str) -> Self {
        let mut a = Adjust::default();
        for part in filter.split(')') {
            let Some((name, v)) = part.trim().split_once('(') else {
                continue;
            };
            let v = v.trim();
            let n = match v.strip_suffix('%') {
                Some(p) => p.trim().parse::<f32>().ok().map(|p| p / 100.),
                None => v.parse::<f32>().ok(),
            };
            let Some(n) = n else {
                continue;
            };
            match name.trim() {
                "brightness" => a.brightness = n,
                "contrast" => a.contrast = n,
                "saturate" => a.saturate = n,
                "grayscale" => a.grayscale = n,
                "sepia" => a.sepia = n,
                _ => {}
            }
        }
        a
    }

    /// As a `filter` value, leaving out what's unchanged; empty when nothing is.
    pub(crate) fn css(&self) -> String {
        let r = |v: f32| (v * 100.).round() / 100.;
        let mut out = vec![];
        for (name, v, plain) in [
            ("brightness", self.brightness, 1.),
            ("contrast", self.contrast, 1.),
            ("saturate", self.saturate, 1.),
            ("grayscale", self.grayscale, 0.),
            ("sepia", self.sepia, 0.),
        ] {
            if (v - plain).abs() > 0.001 {
                out.push(format!("{name}({})", r(v)));
            }
        }
        out.join(" ")
    }
}

/// How close an image is cropped, from its `object-view-box`: 1 shows all of it.
pub(crate) fn zoom_of(view_box: &str) -> f32 {
    let Some(inner) = view_box
        .trim()
        .strip_prefix("inset(")
        .and_then(|v| v.strip_suffix(')'))
    else {
        return 1.;
    };
    let n: Vec<f32> = inner
        .split_whitespace()
        .filter_map(|v| v.trim_end_matches('%').parse().ok())
        .collect();
    match n.as_slice() {
        [t, r, b, l] => {
            let kept = (100. - l - r).min(100. - t - b).max(1.);
            100. / kept
        }
        _ => 1.,
    }
}

/// The `object-view-box` that shows `1 / zoom` of an image, around the focus (0 to 1 across
/// and down).
pub(crate) fn view_box(zoom: f32, focus: (f32, f32)) -> String {
    if zoom <= 1.001 {
        return String::new();
    }
    let slack = (1. - 1. / zoom) * 100.;
    let p = |v: f32| (v * 10.).round() / 10.;
    format!(
        "inset({}% {}% {}% {}%)",
        p(slack * focus.1),
        p(slack * (1. - focus.0)),
        p(slack * (1. - focus.1)),
        p(slack * focus.0)
    )
}

/// Where a picture's focus is, from a position like "left top" or "50% 0%".
pub(crate) fn focus_of(position: &str) -> (f32, f32) {
    let mut f = (0.5, 0.5);
    let words: Vec<&str> = position.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        match *w {
            "left" => f.0 = 0.,
            "right" => f.0 = 1.,
            "top" => f.1 = 0.,
            "bottom" => f.1 = 1.,
            "center" => {}
            v => {
                if let Some(p) = v.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()) {
                    if i == 0 {
                        f.0 = p / 100.
                    } else {
                        f.1 = p / 100.
                    }
                }
            }
        }
    }
    f
}

impl DesignCanvas {
    /// Fit, focus, crop, looks and swapping, for an element showing a picture.
    pub(super) fn picture_controls(&self, el: &Element, cx: &mut Context<Self>) -> Div {
        let image = el.tag == "img";
        let (fit_key, pos_key) = if image {
            ("object-fit", "object-position")
        } else {
            ("background-size", "background-position")
        };
        let fit_now = el.style(fit_key);
        let fits: &[(&str, &str)] = if image {
            &[("Fill", "cover"), ("Fit", "contain"), ("Stretch", "fill")]
        } else {
            &[
                ("Fill", "cover"),
                ("Fit", "contain"),
                ("Stretch", "100% 100%"),
                ("Tile", "auto"),
            ]
        };
        let mut fit_row = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, value)) in fits.iter().enumerate() {
            let value: &'static str = value;
            let on =
                fit_now.trim() == value || (image && value == "fill" && fit_now.trim().is_empty());
            fit_row = fit_row.child(chip_btn(("design-pic-fit", i), *label, on).on_click(
                cx.listener(move |this, _, _, cx| {
                    if image {
                        this.set_style("object-fit", value, cx);
                    } else {
                        let tile = if value == "auto" {
                            "repeat"
                        } else {
                            "no-repeat"
                        };
                        this.set_styles(
                            &[("background-size", value), ("background-repeat", tile)],
                            cx,
                        );
                    }
                }),
            ));
        }

        // Which part stays in view, on a 3 × 3 grid.
        let focus = focus_of(&el.style(pos_key));
        let zoom = if image {
            zoom_of(&el.style("object-view-box"))
        } else {
            1.
        };
        let mut grid = div().grid().grid_cols(3).gap(px(3.)).w(px(60.));
        for (i, (fx, fy)) in [0., 0.5, 1.]
            .into_iter()
            .flat_map(|y| [0., 0.5, 1.].map(|x| (x, y)))
            .enumerate()
        {
            let on = (focus.0 - fx).abs() < 0.01 && (focus.1 - fy).abs() < 0.01;
            let pos = format!("{}% {}%", fx * 100., fy * 100.);
            grid = grid.child(
                div()
                    .id(("design-pic-focus", i))
                    .size(px(18.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(rgb(if on { c_blue() } else { c_border() }))
                    .when(on, |el| el.bg(rgb(c_blue())))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if image {
                            let vb = view_box(zoom, (fx, fy));
                            this.set_styles(
                                &[("object-position", pos.as_str()), ("object-view-box", &vb)],
                                cx,
                            );
                        } else {
                            this.set_style("background-position", &pos, cx);
                        }
                    })),
            );
        }
        let mut zooms = div().flex().flex_wrap().gap(px(4.));
        if image {
            for (i, z) in [1., 1.25, 1.5, 2., 3.].into_iter().enumerate() {
                let on = (zoom - z).abs() < 0.02;
                zooms = zooms.child(
                    chip_btn(("design-pic-zoom", i), format!("{}%", z * 100.), on).on_click(
                        cx.listener(move |this, _, _, cx| {
                            let vb = view_box(z, focus);
                            this.set_styles(&[("object-view-box", &vb)], cx);
                        }),
                    ),
                );
            }
        }

        // Looks, and fine adjustments a step at a time.
        let now = Adjust::parse(&el.style("filter"));
        let mut looks = div().flex().flex_wrap().gap(px(4.));
        for (i, (label, a)) in LOOKS.iter().enumerate() {
            let css = a.css();
            looks = looks.child(
                chip_btn(("design-pic-look", i), *label, now == *a).on_click(
                    cx.listener(move |this, _, _, cx| this.set_style("filter", &css, cx)),
                ),
            );
        }
        let stepper = |i: usize, label: &'static str, v: f32, set: fn(&mut Adjust, f32)| {
            let step = |d: f32, id: &'static str| {
                mini(
                    (id, i),
                    if d < 0. {
                        IconName::Minus
                    } else {
                        IconName::Plus
                    },
                    label,
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Some(el) = &this.element else {
                        return;
                    };
                    let mut a = Adjust::parse(&el.style("filter"));
                    set(&mut a, (v + d).clamp(0., 3.));
                    this.set_style("filter", &a.css(), cx);
                }))
            };
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(11.5))
                .child(div().flex_1().text_color(rgb(c_muted())).child(label))
                .child(step(-0.1, "design-pic-less"))
                .child(
                    div()
                        .w(px(40.))
                        .text_center()
                        .text_color(rgb(c_text()))
                        .child(format!("{}%", (v * 100.).round())),
                )
                .child(step(0.1, "design-pic-more"))
        };
        let adjust = div()
            .v_flex()
            .gap(px(2.))
            .child(stepper(0, "Brightness", now.brightness, |a, v| {
                a.brightness = v
            }))
            .child(stepper(1, "Contrast", now.contrast, |a, v| a.contrast = v))
            .child(stepper(2, "Saturation", now.saturate, |a, v| {
                a.saturate = v
            }));

        let mut out = div()
            .v_flex()
            .gap(px(6.))
            .child(caption("Fit"))
            .child(fit_row)
            .child(caption(if image { "Focus and crop" } else { "Focus" }))
            .child(div().flex().gap(px(10.)).child(grid).child(zooms))
            .child(caption("Look"))
            .child(looks)
            .child(adjust);
        if el.children > 0 && !image {
            out = out.child(caption(
                "Looks change everything in this box, its text too.",
            ));
        }
        out
    }

    /// The pictures already in the design, to swap the picked one for in a click.
    pub(super) fn picture_library(&self, cx: &mut Context<Self>) -> Option<Div> {
        let dir = self.root.join(FOLDER).join(ASSETS);
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| crate::turns::is_image(p))
            .collect();
        if files.is_empty() {
            return None;
        }
        files.sort();
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (i, f) in files.into_iter().take(24).enumerate() {
            let name = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let url = format!("../{ASSETS}/{name}");
            row = row.child(
                div()
                    .id(("design-pic-swap", i))
                    .size(px(38.))
                    .rounded(px(4.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(divider())
                    .cursor_pointer()
                    .hover(|el| el.border_color(rgb(c_blue())))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(name.clone()).build(window, cx)
                    })
                    .child(img(f).size_full().object_fit(ObjectFit::Cover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.op("picture", &[json!(url)], After::Same, cx)
                    })),
            );
        }
        Some(
            div()
                .v_flex()
                .gap(px(6.))
                .child(caption("Swap for"))
                .child(row),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{focus_of, view_box, zoom_of, Adjust, LOOKS};

    #[test]
    fn picture_looks_read_back_as_they_were_set() {
        for (_, a) in LOOKS {
            assert_eq!(Adjust::parse(&a.css()), a);
        }
        assert_eq!(LOOKS[0].1.css(), "");
        let a = Adjust::parse("blur(4px) brightness(120%) grayscale(1)");
        assert_eq!((a.brightness, a.grayscale), (1.2, 1.));
    }

    #[test]
    fn crops_keep_the_focus_in_view() {
        assert_eq!(view_box(1., (0.5, 0.5)), "");
        assert_eq!(view_box(2., (0.5, 0.5)), "inset(25% 25% 25% 25%)");
        assert_eq!(view_box(2., (0., 0.)), "inset(0% 50% 50% 0%)");
        assert!((zoom_of("inset(25% 25% 25% 25%)") - 2.).abs() < 0.01);
        assert_eq!(zoom_of(""), 1.);
        assert_eq!(focus_of("left top"), (0., 0.));
        assert_eq!(focus_of("100% 50%"), (1., 0.5));
        assert_eq!(focus_of(""), (0.5, 0.5));
    }
}
