//! The inspector's typed fields: which styles each one reads and sets, how a value is shown
//! in it, and how what's typed becomes CSS.

use super::dom::Element;

/// What kind of value a field holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    /// A length; a bare number is pixels.
    Px,
    /// Opacity, shown and typed as a percentage.
    Percent,
    /// A colour: hex, a theme token by name, or any CSS colour.
    Color,
}

/// A field: its id, the style it shows, the styles it sets, and its unit.
pub(crate) struct Field {
    pub id: &'static str,
    pub reads: &'static str,
    pub sets: &'static [&'static str],
    pub unit: Unit,
}

pub(crate) const FIELDS: [Field; 18] = [
    px("w", "width", &["width"]),
    px("h", "height", &["height"]),
    px("gap", "gap", &["gap"]),
    px("pad-x", "padding-left", &["padding-left", "padding-right"]),
    px("pad-y", "padding-top", &["padding-top", "padding-bottom"]),
    px("radius", "border-top-left-radius", &["border-radius"]),
    Field {
        id: "opacity",
        reads: "opacity",
        sets: &["opacity"],
        unit: Unit::Percent,
    },
    px("font-size", "font-size", &["font-size"]),
    px("line-height", "line-height", &["line-height"]),
    px("letter", "letter-spacing", &["letter-spacing"]),
    color("fill", "background-color", &["background-color"]),
    color("color", "color", &["color"]),
    Field {
        id: "stroke",
        reads: "border-top-color",
        sets: &["border-color"],
        unit: Unit::Color,
    },
    px("stroke-w", "border-top-width", &["border-width"]),
    px("x", "left", &["left"]),
    px("y", "top", &["top"]),
    px("r", "right", &["right"]),
    px("b", "bottom", &["bottom"]),
];

const fn px(id: &'static str, reads: &'static str, sets: &'static [&'static str]) -> Field {
    Field {
        id,
        reads,
        sets,
        unit: Unit::Px,
    }
}

const fn color(id: &'static str, reads: &'static str, sets: &'static [&'static str]) -> Field {
    Field {
        id,
        reads,
        sets,
        unit: Unit::Color,
    }
}

pub(crate) fn field(id: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.id == id)
}

/// The colour tokens of the theme, by the name a person types for them.
const TOKENS: [&str; 7] = [
    "primary",
    "on-primary",
    "bg",
    "surface",
    "text",
    "muted",
    "border",
];

/// What a field shows for an element: its own value when that's a plain number or colour,
/// else what the page worked out, so a field always shows the size things really are.
pub(crate) fn display(f: &Field, el: &Element) -> String {
    // A shorthand set with a variable leaves its longhands empty here.
    let own = el
        .inline
        .get(f.reads)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(v) = own.as_ref().filter(|_| matches!(f.unit, Unit::Color)) {
        if let Some(name) = token_of(v) {
            return name;
        }
    }
    let v = own
        .filter(|v| !v.starts_with("var(") && !v.starts_with("calc("))
        .or_else(|| el.computed.get(f.reads).cloned())
        .unwrap_or_default();
    let v = v.trim();
    match f.unit {
        Unit::Px => match v {
            "" | "auto" | "normal" | "none" => "Auto".into(),
            "fit-content" => "Hug".into(),
            "100%" if f.id == "w" || f.id == "h" => "Fill".into(),
            _ => match v
                .strip_suffix("px")
                .and_then(|n| n.trim().parse::<f32>().ok())
            {
                Some(n) => number(n),
                None => v.to_string(),
            },
        },
        Unit::Percent => match v.parse::<f32>() {
            Ok(n) => format!("{}%", number(n * 100.)),
            Err(_) => "100%".into(),
        },
        Unit::Color => hex(v).unwrap_or_default(),
    }
}

/// A number without a trailing `.0`, and to at most two places.
fn number(n: f32) -> String {
    let r = (n * 100.).round() / 100.;
    if r.fract() == 0. {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// `var(--primary)` as `primary`: a variable by its name.
fn token_of(v: &str) -> Option<String> {
    let name = v.strip_prefix("var(--")?.strip_suffix(')')?;
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .then(|| name.to_string())
}

/// A CSS colour as `#RRGGBB`, or nothing for a transparent one.
pub(crate) fn hex(css: &str) -> Option<String> {
    let s = css.trim();
    if let Some(h) = s.strip_prefix('#') {
        return Some(format!("#{}", h.to_uppercase()));
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .trim_end_matches(')');
    let parts: Vec<f32> = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    if parts.len() < 3 || parts.get(3).is_some_and(|a| *a == 0.) {
        return None;
    }
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        parts[0] as u8, parts[1] as u8, parts[2] as u8
    ))
}

/// What's typed in a field as CSS, or `None` when it can't be read. An empty value removes
/// the element's own style.
pub(crate) fn parse(f: &Field, typed: &str) -> Option<String> {
    let t = typed.trim();
    let lower = t.to_lowercase();
    if t.is_empty() || lower == "auto" {
        return Some(String::new());
    }
    // A design variable, by its name.
    if let Some(name) = t.strip_prefix("--").filter(|n| !n.is_empty()) {
        return Some(format!("var(--{name})"));
    }
    match f.unit {
        Unit::Px => {
            match lower.as_str() {
                "hug" if f.id == "w" || f.id == "h" => return Some("fit-content".into()),
                "fill" if f.id == "w" || f.id == "h" => return Some("100%".into()),
                _ => {}
            }
            if let Ok(n) = t.parse::<f32>() {
                return (n >= 0. || f.id == "letter").then(|| format!("{}px", number(n)));
            }
            // Any other CSS length or expression goes in as it is.
            t.chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '.' || c.is_alphabetic())
                .then(|| t.to_string())
        }
        Unit::Percent => {
            let n: f32 = t.trim_end_matches('%').trim().parse().ok()?;
            Some(number((n / 100.).clamp(0., 1.)))
        }
        Unit::Color => {
            let bare = lower.trim_start_matches("--");
            if TOKENS.contains(&bare) {
                return Some(format!("var(--{bare})"));
            }
            let h = t.trim_start_matches('#');
            if matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(format!("#{}", h.to_lowercase()));
            }
            Some(t.to_string())
        }
    }
}

/// The styles to set for what's typed in a field, as name and value.
pub(crate) fn styles(f: &Field, value: &str, el: &Element) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> =
        f.sets.iter().map(|k| (*k, value.to_string())).collect();
    // A stroke needs a style to show, and a colour unless it has one.
    if (f.id == "stroke-w" || f.id == "stroke") && !value.is_empty() {
        if el.style("border-top-style").is_empty() || el.style("border-top-style") == "none" {
            out.push(("border-style", "solid".into()));
        }
        if f.id == "stroke-w" && !el.inline.contains_key("border-top-color") {
            out.push(("border-color", "var(--border)".into()));
        }
        if f.id == "stroke" && display(field("stroke-w").unwrap(), el) == "0" {
            out.push(("border-width", "1px".into()));
        }
    }
    out
}

/// Where children sit in a flex container, as a cell of a 3 × 3 grid: (column, row) from
/// the top left, whichever way the container runs.
pub(crate) fn cell(el: &Element) -> (usize, usize) {
    let pos = |v: &str| match v {
        "center" => 1,
        "flex-end" | "end" => 2,
        _ => 0,
    };
    let main = pos(&el.style("justify-content"));
    let cross = pos(&el.style("align-items"));
    if row(el) {
        (main, cross)
    } else {
        (cross, main)
    }
}

/// Whether a flex container runs across.
pub(crate) fn row(el: &Element) -> bool {
    !el.style("flex-direction").starts_with("column")
}

/// The `justify-content` and `align-items` that put children in a cell of the grid.
pub(crate) fn align_for(col: usize, line: usize, across: bool) -> (&'static str, &'static str) {
    const AT: [&str; 3] = ["flex-start", "center", "flex-end"];
    if across {
        (AT[col], AT[line])
    } else {
        (AT[line], AT[col])
    }
}

/// Shadows offered under Effects.
pub(crate) const SHADOWS: [(&str, &str); 4] = [
    ("None", ""),
    ("Small", "0 1px 3px rgba(0, 0, 0, 0.12)"),
    ("Medium", "0 4px 12px rgba(0, 0, 0, 0.14)"),
    ("Large", "0 12px 32px rgba(0, 0, 0, 0.18)"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn el(inline: &[(&str, &str)], computed: &[(&str, &str)]) -> Element {
        let map = |kv: &[(&str, &str)]| -> HashMap<String, String> {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        Element {
            inline: map(inline),
            computed: map(computed),
            ..Default::default()
        }
    }

    #[test]
    fn fields_show_plain_numbers_and_colours() {
        let e = el(
            &[("gap", "var(--space)"), ("color", "var(--primary)")],
            &[
                ("width", "342.5px"),
                ("gap", "8px"),
                ("opacity", "0.5"),
                ("background-color", "rgba(0, 0, 0, 0)"),
                ("color", "rgb(37, 99, 235)"),
                ("line-height", "normal"),
            ],
        );
        let show = |id| display(field(id).unwrap(), &e);
        assert_eq!(show("w"), "342.5");
        assert_eq!(show("gap"), "8");
        assert_eq!(show("opacity"), "50%");
        assert_eq!(show("fill"), "");
        assert_eq!(show("color"), "primary");
        assert_eq!(show("line-height"), "Auto");
        let hug = el(&[("width", "fit-content")], &[("width", "120px")]);
        assert_eq!(display(field("w").unwrap(), &hug), "Hug");
    }

    #[test]
    fn what_is_typed_becomes_css() {
        let p = |id, t| parse(field(id).unwrap(), t);
        assert_eq!(p("w", "120"), Some("120px".into()));
        assert_eq!(p("w", "fill"), Some("100%".into()));
        assert_eq!(p("w", "50%"), Some("50%".into()));
        assert_eq!(p("w", ""), Some(String::new()));
        assert_eq!(p("w", "-4"), None);
        assert_eq!(p("letter", "-0.5"), Some("-0.5px".into()));
        assert_eq!(p("opacity", "40%"), Some("0.4".into()));
        assert_eq!(p("opacity", "250"), Some("1".into()));
        assert_eq!(p("fill", "FF0000"), Some("#ff0000".into()));
        assert_eq!(p("fill", "primary"), Some("var(--primary)".into()));
        assert_eq!(p("fill", "--muted"), Some("var(--muted)".into()));
    }

    #[test]
    fn a_stroke_gets_a_style_and_colour() {
        let e = el(&[], &[("border-top-style", "none")]);
        let got = styles(field("stroke-w").unwrap(), "2px", &e);
        assert!(got.contains(&("border-width", "2px".into())));
        assert!(got.contains(&("border-style", "solid".into())));
        assert!(got.contains(&("border-color", "var(--border)".into())));
    }

    #[test]
    fn alignment_reads_and_writes_the_same_cell() {
        for across in [true, false] {
            for c in 0..3 {
                for r in 0..3 {
                    let (j, a) = align_for(c, r, across);
                    let e = el(
                        &[
                            ("justify-content", j),
                            ("align-items", a),
                            ("flex-direction", if across { "row" } else { "column" }),
                        ],
                        &[],
                    );
                    assert_eq!(cell(&e), (c, r));
                }
            }
        }
    }

    #[test]
    fn colours_read_as_hex() {
        assert_eq!(hex("rgb(37, 99, 235)"), Some("#2563EB".into()));
        assert_eq!(hex("rgba(0, 0, 0, 0)"), None);
        assert_eq!(hex("#abc"), Some("#ABC".into()));
    }
}
