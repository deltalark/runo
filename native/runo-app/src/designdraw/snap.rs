//! Raw values snapped to the theme: a colour, text size, corner radius or spacing written as
//! a plain value that is, or is all but, one of the design's tokens is written as that
//! token instead, so it follows the theme when the theme changes, dark mode included.
//! Values close to no token stay as they are, for the checks to point out.

use super::tokens::{self, Group, Token, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How far apart two colours can be, in red, green and blue, and still be the same token.
const NEAR_COLOR: f32 = 10.;
/// How far apart two lengths can be, in px, and still be the same token.
const NEAR_PX: f32 = 1.;

/// Properties whose whole value is a colour.
const COLORS: &[&str] = &[
    "color",
    "background",
    "background-color",
    "border-color",
    "outline-color",
    "fill",
    "stroke",
    "caret-color",
    "accent-color",
];
/// Shorthands with a colour among their parts: `border: 1px solid #ddd`.
const WITH_COLOR: &[&str] = &[
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "outline",
];
/// Properties made of spacing lengths.
const SPACINGS: &[&str] = &[
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "gap",
    "row-gap",
    "column-gap",
];

/// The design's tokens, as raw values are matched against them.
pub(crate) struct Theme {
    tokens: Vec<Token>,
    /// The base spacing unit, `--space`, that other spacings are multiples of.
    space: Option<f32>,
}

impl Theme {
    pub(crate) fn new(css: &str) -> Self {
        let tokens = tokens::read(css);
        let space = tokens
            .iter()
            .find(|t| t.group == Group::Spacing && (t.name == "space" || t.name == "spacing"))
            .and_then(|t| match t.light {
                Value::Px(n) if n > 0. => Some(n),
                _ => None,
            });
        Theme { tokens, space }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The colour token nearest `c` for `prop`, within reach.
    fn color(&self, prop: &str, c: [u8; 4]) -> Option<&Token> {
        if c[3] == 0 {
            return None;
        }
        let far = |t: &Token| match t.light {
            Value::Color(v) if v[3].abs_diff(c[3]) <= 3 => Some(
                (0..3)
                    .map(|i| (v[i] as f32 - c[i] as f32).powi(2))
                    .sum::<f32>()
                    .sqrt(),
            ),
            _ => None,
        };
        let best = self
            .tokens
            .iter()
            .filter_map(|t| Some((t, far(t)?)))
            .filter(|(_, d)| *d <= NEAR_COLOR)
            .min_by(|a, b| a.1.total_cmp(&b.1))?
            .1;
        // Of the tokens that close, the one whose name fits the property: a white background
        // is the background, white text is text on a colour.
        let fits: &[&str] = if prop.starts_with("background") {
            &["bg", "background", "surface"]
        } else if prop.starts_with("border") || prop.starts_with("outline") {
            &["border", "outline", "divider"]
        } else if prop == "color" || prop == "fill" {
            &["text", "on-", "muted", "fg"]
        } else {
            &[]
        };
        let near: Vec<&Token> = self
            .tokens
            .iter()
            .filter(|t| far(t).is_some_and(|d| d <= best + 0.01))
            .collect();
        near.iter()
            .find(|t| fits.iter().any(|f| t.name.contains(f)))
            .or(near.first())
            .copied()
    }

    /// The token of `group` within reach of `n` px.
    fn px(&self, group: Group, n: f32) -> Option<&Token> {
        self.tokens
            .iter()
            .filter(|t| t.group == group)
            .filter_map(|t| match t.light {
                Value::Px(v) if (v - n).abs() <= NEAR_PX => Some((t, (v - n).abs())),
                _ => None,
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(t, _)| t)
    }

    /// A spacing of `n` px as a token, or as a whole or half multiple of `--space`.
    fn spacing(&self, n: f32) -> Option<String> {
        if let Some(t) = self.px(Group::Spacing, n) {
            return Some(var(t));
        }
        let unit = self.space?;
        let times = (n / unit * 2.).round() / 2.;
        if times <= 0. || (times * unit - n).abs() > 0.5 {
            return None;
        }
        Some(if times == 1. {
            "var(--space)".into()
        } else {
            format!("calc(var(--space) * {})", num(times))
        })
    }
}

fn var(t: &Token) -> String {
    format!("var(--{})", t.name)
}

fn num(n: f32) -> String {
    let s = format!("{n:.1}");
    s.strip_suffix(".0").map_or(s.clone(), String::from)
}

/// A length in px, not zero: `16px`, `1rem`.
fn px_of(v: &str) -> Option<f32> {
    match tokens::length(v)? {
        (n, true) if n != 0. => Some(n),
        _ => None,
    }
}

/// `value` of `prop` as tokens, when any of it snaps.
pub(crate) fn value(theme: &Theme, prop: &str, value: &str) -> Option<String> {
    let v = value.trim();
    if v.contains("var(") || v.contains('!') {
        return None;
    }
    let prop = prop.trim().to_ascii_lowercase();
    let p = prop.as_str();
    if COLORS.contains(&p) {
        return theme.color(p, tokens::color(v)?).map(var);
    }
    if p == "font-size" {
        return theme.px(Group::FontSize, px_of(v)?).map(var);
    }
    if p == "border-radius" {
        return theme.px(Group::Radius, px_of(v)?).map(var);
    }
    let parts = tokens::split_top(v, ' ');
    let parts: Vec<&str> = parts
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let each: Vec<Option<String>> = if WITH_COLOR.contains(&p) {
        parts
            .iter()
            .map(|s| theme.color(p, tokens::color(s)?).map(var))
            .collect()
    } else if SPACINGS.contains(&p) {
        parts.iter().map(|s| theme.spacing(px_of(s)?)).collect()
    } else {
        return None;
    };
    if each.iter().all(Option::is_none) {
        return None;
    }
    Some(
        parts
            .iter()
            .zip(each)
            .map(|(was, now)| now.unwrap_or_else(|| was.to_string()))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A `style` attribute's declarations with what snaps written as tokens, and how many
/// values snapped; `None` when nothing did.
pub(crate) fn style(theme: &Theme, css: &str) -> Option<(String, usize)> {
    let mut n = 0;
    let mut out = Vec::new();
    for d in tokens::split_top(css, ';') {
        let d = d.trim();
        if d.is_empty() {
            continue;
        }
        match d.split_once(':') {
            Some((k, v)) => match value(theme, k, v) {
                Some(now) => {
                    n += 1;
                    out.push(format!("{}: {now}", k.trim()));
                }
                None => out.push(d.to_string()),
            },
            None => out.push(d.to_string()),
        }
    }
    (n > 0).then(|| (out.join("; "), n))
}

/// A screen's markup with its inline styles snapped, and how many values snapped. Style
/// attributes found just as they are in `was` are left alone: they were there before.
pub(crate) fn html(theme: &Theme, html: &str, was: &str) -> (String, usize) {
    let mut out = String::with_capacity(html.len());
    let mut n = 0;
    let mut rest = html;
    while let Some(i) = rest.find(" style=\"") {
        let start = i + " style=\"".len();
        let Some(len) = rest[start..].find('"') else {
            break;
        };
        out.push_str(&rest[..start]);
        let attr = &rest[start..start + len];
        let snapped = (!was.contains(&format!(" style=\"{attr}\"")))
            .then(|| style(theme, &attr.replace("&quot;", "\"")))
            .flatten();
        match snapped {
            Some((css, k)) => {
                n += k;
                out.push_str(&css.replace('"', "&quot;"));
            }
            None => out.push_str(attr),
        }
        rest = &rest[start + len..];
    }
    out.push_str(rest);
    (out, n)
}

/// Snap the screens in `dir` that changed since `before`, a snapshot of the files by
/// path, keeping what each had before as it was. Gives how many values snapped.
pub(crate) fn screens(dir: &Path, before: &HashMap<PathBuf, Vec<u8>>, theme: &Theme) -> usize {
    if theme.is_empty() {
        return 0;
    }
    let Ok(files) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for file in files.flatten().map(|e| e.path()) {
        if file.extension().is_none_or(|e| e != "html") {
            continue;
        }
        let Ok(now) = std::fs::read_to_string(&file) else {
            continue;
        };
        let was = before
            .get(&file)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        if now == was {
            continue;
        }
        let (out, k) = html(theme, &now, &was);
        if k > 0 && std::fs::write(&file, out).is_ok() {
            n += k;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    const CSS: &str = ":root {
      --primary: #D9381A; --on-primary: #ffffff; --bg: #ffffff; --text: #1c1917;
      --border: #ebe6e0; --radius: 14px; --radius-sm: calc(var(--radius) * 0.5);
      --space: 8px; --text-sm: 14px; --text-md: 16px;
    }";

    #[test]
    fn colours_snap_to_the_nearest_token_that_fits() {
        let t = Theme::new(CSS);
        assert_eq!(
            value(&t, "color", "#d9381a").as_deref(),
            Some("var(--primary)")
        );
        assert_eq!(
            value(&t, "color", "rgb(218, 58, 28)").as_deref(),
            Some("var(--primary)")
        );
        assert_eq!(
            value(&t, "background", "#fff").as_deref(),
            Some("var(--bg)")
        );
        assert_eq!(
            value(&t, "color", "white").as_deref(),
            Some("var(--on-primary)")
        );
        assert_eq!(
            value(&t, "border", "1px solid #ebe6e1").as_deref(),
            Some("1px solid var(--border)")
        );
        // Far from every token, or see-through: left for the checks.
        assert_eq!(value(&t, "color", "#2563eb"), None);
        assert_eq!(value(&t, "background", "transparent"), None);
        assert_eq!(value(&t, "color", "var(--text)"), None);
    }

    #[test]
    fn sizes_snap_to_their_group() {
        let t = Theme::new(CSS);
        assert_eq!(
            value(&t, "font-size", "16px").as_deref(),
            Some("var(--text-md)")
        );
        assert_eq!(
            value(&t, "font-size", "15px").as_deref(),
            Some("var(--text-sm)")
        );
        assert_eq!(value(&t, "font-size", "20px"), None);
        assert_eq!(
            value(&t, "border-radius", "7px").as_deref(),
            Some("var(--radius-sm)")
        );
        assert_eq!(value(&t, "width", "16px"), None);
    }

    #[test]
    fn spacing_snaps_to_multiples_of_the_unit() {
        let t = Theme::new(CSS);
        assert_eq!(value(&t, "gap", "8px").as_deref(), Some("var(--space)"));
        assert_eq!(
            value(&t, "padding", "0 16px 12px 13px").as_deref(),
            Some("0 calc(var(--space) * 2) calc(var(--space) * 1.5) 13px")
        );
        assert_eq!(value(&t, "margin", "0"), None);
    }

    #[test]
    fn styles_and_markup() {
        let t = Theme::new(CSS);
        assert_eq!(
            style(&t, "width: 44px; padding: 16px;color:#1c1917"),
            Some((
                "width: 44px; padding: calc(var(--space) * 2); color: var(--text)".into(),
                2
            ))
        );
        assert_eq!(style(&t, "width: 44px"), None);
        let was = r#"<p style="color: #1c1917">a</p>"#;
        let now = r#"<p style="color: #1c1917">a</p><div style="background: url(&quot;x.svg&quot;); gap: 16px">b</div>"#;
        let (out, n) = html(&t, now, was);
        assert_eq!(n, 1);
        assert_eq!(
            out,
            r#"<p style="color: #1c1917">a</p><div style="background: url(&quot;x.svg&quot;); gap: calc(var(--space) * 2)">b</div>"#
        );
    }

    #[test]
    fn only_changed_screens_snap() {
        let dir = std::env::temp_dir().join(format!("runo-snap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.html"), dir.join("b.html"));
        let old = r#"<p style="color: #1c1917">a</p>"#;
        std::fs::write(&a, old).unwrap();
        std::fs::write(&b, old).unwrap();
        let before: HashMap<PathBuf, Vec<u8>> =
            [(a.clone(), old.into()), (b.clone(), old.into())].into();
        let new = r#"<p style="color: #1c1917">a</p><h1 style="font-size: 17px">b</h1>"#;
        std::fs::write(&b, new).unwrap();
        assert_eq!(screens(&dir, &before, &Theme::new(CSS)), 1);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), old);
        assert_eq!(
            std::fs::read_to_string(&b).unwrap(),
            r#"<p style="color: #1c1917">a</p><h1 style="font-size: var(--text-md)">b</h1>"#
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
