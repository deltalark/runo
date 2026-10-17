//! Styles: named looks applied as a class. Text styles set type (size, weight, line height,
//! letter spacing); the kit's own are classes `components.css` always has. Styles set
//! everything else an element looks like: fill, stroke, corners, shadow, padding, layout.
//! Ones made in Design are kept at the end of the project's `components.css`, a section for
//! each kind and one rule a line, so agents and people can use them too.

use std::path::{Path, PathBuf};

/// A text style: what it's called and the class that applies it. The class is empty for
/// Body, the page's own type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextStyle {
    pub name: String,
    pub class: String,
}

/// The kit's text styles, as `components.css` defines them.
pub(crate) const KIT: [(&str, &str); 6] = [
    ("Display", "display"),
    ("Title", "big"),
    ("Body", ""),
    ("Small", "small"),
    ("Tiny", "tiny"),
    ("Eyebrow", "eyebrow"),
];

/// The kinds of style made in Design, each kept in its own section of `components.css`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Text,
    Look,
}

impl Kind {
    /// The line its section starts with.
    fn mark(self) -> &'static str {
        match self {
            Kind::Text => "/* Text styles made in Design. */",
            Kind::Look => "/* Styles made in Design. */",
        }
    }
}

const KINDS: [Kind; 2] = [Kind::Text, Kind::Look];

/// Where `kind`'s section runs in `css`, after its mark line, up to the next section.
fn section(css: &str, kind: Kind) -> Option<(usize, usize)> {
    let start = css.find(kind.mark())? + kind.mark().len();
    let end = KINDS
        .iter()
        .filter(|k| **k != kind)
        .filter_map(|k| css[start..].find(k.mark()).map(|i| start + i))
        .min()
        .unwrap_or(css.len());
    Some((start, end))
}

/// The type a text style sets.
pub(crate) const PROPS: [&str; 4] = ["font-size", "font-weight", "line-height", "letter-spacing"];

pub(crate) fn css_path(root: &Path) -> PathBuf {
    super::model::folder(root).join("components.css")
}

/// Every text style: the kit's, then the project's own.
pub(crate) fn all(css: &str) -> Vec<TextStyle> {
    KIT.iter()
        .map(|(n, c)| TextStyle {
            name: n.to_string(),
            class: c.to_string(),
        })
        .chain(own(css, Kind::Text))
        .collect()
}

/// The styles of a kind made in Design, from lines like `/* Card title */ .card-title { … }`.
pub(crate) fn own(css: &str, kind: Kind) -> Vec<TextStyle> {
    let Some((start, end)) = section(css, kind) else {
        return Vec::new();
    };
    css[start..end]
        .lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix("/*")?;
            let (name, rule) = l.split_once("*/")?;
            let class = rule.trim().strip_prefix('.')?.split_once('{')?.0.trim();
            Some(TextStyle {
                name: name.trim().to_string(),
                class: class.to_string(),
            })
        })
        .collect()
}

/// A class for a new style called `name`, not used in `css` yet.
pub(crate) fn class_for(css: &str, name: &str) -> String {
    let base = match super::model::slug(name) {
        s if s.is_empty() => "text-style".to_string(),
        s if s.starts_with(|c: char| c.is_ascii_digit()) => format!("text-{s}"),
        s => s,
    };
    let used = |c: &str| {
        css.contains(&format!(".{c} "))
            || css.contains(&format!(".{c}{{"))
            || css.contains(&format!(".{c},"))
            || css.contains(&format!(".{c}:"))
    };
    if !used(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !used(c))
        .unwrap_or(base)
}

/// What a style made from an element is called: "18 Semibold".
pub(crate) fn name_for(size: &str, weight: &str) -> String {
    let size = size.trim().trim_end_matches("px");
    let size = size
        .parse::<f32>()
        .map(|s| format!("{}", s.round()))
        .unwrap_or_else(|_| size.to_string());
    let weight = match weight.trim() {
        "100" => "Thin",
        "200" => "Extra light",
        "300" => "Light",
        "400" | "normal" => "Regular",
        "500" => "Medium",
        "600" => "Semibold",
        "700" | "bold" => "Bold",
        "800" => "Extra bold",
        "900" => "Black",
        w => w,
    };
    format!("{size} {weight}")
}

/// One rule's line: `/* Name */ .class { k: v; … }`.
fn rule_line(style: &TextStyle, decls: &[(String, String)]) -> String {
    let body: Vec<String> = decls
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| format!("{k}: {};", v.trim()))
        .collect();
    format!(
        "/* {} */ .{} {{ {} }}",
        style.name.replace("*/", ""),
        style.class,
        body.join(" ")
    )
}

/// `css` with a style added at the end of its kind's section, which it starts if need be.
pub(crate) fn add(css: &str, kind: Kind, style: &TextStyle, decls: &[(&str, String)]) -> String {
    let decls: Vec<(String, String)> = decls
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let line = rule_line(style, &decls);
    match section(css, kind) {
        Some((_, end)) => {
            let head = css[..end].trim_end();
            let tail = css[end..].trim_start();
            if tail.is_empty() {
                format!("{head}\n{line}\n")
            } else {
                format!("{head}\n{line}\n\n{tail}")
            }
        }
        None => format!("{}\n\n{}\n{line}\n", css.trim_end(), kind.mark()),
    }
}

/// Where the line of the rule for `class` made in Design is, and the line.
fn rule_at(css: &str, class: &str) -> Option<(usize, usize, TextStyle)> {
    for kind in KINDS {
        let Some((start, end)) = section(css, kind) else {
            continue;
        };
        let mut at = start;
        for l in css[start..end].split_inclusive('\n') {
            let line = l.trim_end_matches('\n');
            if let Some(s) = own(&format!("{}\n{line}", kind.mark()), kind).pop() {
                if s.class == class {
                    return Some((at, at + line.len(), s));
                }
            }
            at += l.len();
        }
    }
    None
}

/// What the rule for `class` made in Design sets, in order.
pub(crate) fn decls(css: &str, class: &str) -> Vec<(String, String)> {
    let Some((a, b, _)) = rule_at(css, class) else {
        return Vec::new();
    };
    let line = &css[a..b];
    let Some(body) = line
        .split_once('{')
        .and_then(|(_, r)| r.rsplit_once('}'))
        .map(|(b, _)| b)
    else {
        return Vec::new();
    };
    split_decls(body)
}

/// `k: v; k: v` as pairs, keeping `;` and `:` inside brackets and quotes, as in
/// `url("data:image/png;base64,…")`.
pub(crate) fn split_decls(body: &str) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut cur) = (0i32, None::<char>, String::new());
    for c in body.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ';') if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    parts.push(cur);
    parts
        .iter()
        .filter_map(|p| {
            let (k, v) = p.split_once(':')?;
            let (k, v) = (k.trim(), v.trim());
            (!k.is_empty() && !v.is_empty()).then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// `css` with the rule for `class` setting `decls`: each one replaces what the rule set for
/// that property, or is added at its end. Nothing changes if there's no such rule.
pub(crate) fn merge(css: &str, class: &str, more: &[(String, String)]) -> String {
    let Some((a, b, style)) = rule_at(css, class) else {
        return css.to_string();
    };
    let mut now = decls(css, class);
    for (k, v) in more {
        // A shorthand replaces its longhands, and a longhand the shorthand it's part of.
        now.retain(|(o, _)| {
            o != k && !o.starts_with(&format!("{k}-")) && !k.starts_with(&format!("{o}-"))
        });
        now.push((k.clone(), v.clone()));
    }
    format!("{}{}{}", &css[..a], rule_line(&style, &now), &css[b..])
}

/// `css` with the style for `class` called `name`.
pub(crate) fn rename(css: &str, class: &str, name: &str) -> String {
    let Some((a, b, _)) = rule_at(css, class) else {
        return css.to_string();
    };
    let style = TextStyle {
        name: name.to_string(),
        class: class.to_string(),
    };
    format!(
        "{}{}{}",
        &css[..a],
        rule_line(&style, &decls(css, class)),
        &css[b..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_styles_made_in_design_are_kept_in_the_css() {
        let css = ".card { padding: 8px; }\n.big { font-size: 28px; }\n";
        assert_eq!(all(css).len(), KIT.len());
        let name = name_for("18px", "600");
        assert_eq!(name, "18 Semibold");
        let class = class_for(css, &name);
        assert_eq!(class, "text-18-semibold");
        let s = TextStyle { name, class };
        let css = add(
            css,
            Kind::Text,
            &s,
            &[
                ("font-size", "18px".into()),
                ("font-weight", "600".into()),
                ("line-height", "".into()),
            ],
        );
        assert!(css.contains(".text-18-semibold { font-size: 18px; font-weight: 600; }"));
        assert_eq!(own(&css, Kind::Text), vec![s.clone()]);
        assert_eq!(class_for(&css, "18 Semibold"), "text-18-semibold-2");
        assert_eq!(class_for(&css, "Card"), "card-2");
        let more = add(
            &css,
            Kind::Text,
            &TextStyle {
                name: "Label".into(),
                class: "label".into(),
            },
            &[("font-size", "13px".into())],
        );
        assert_eq!(more.matches(Kind::Text.mark()).count(), 1);
        assert_eq!(own(&more, Kind::Text).len(), 2);
        assert_eq!(all(&more).last().unwrap().name, "Label");
    }

    #[test]
    fn looks_keep_their_own_section_and_can_change() {
        let text = TextStyle {
            name: "Label".into(),
            class: "label".into(),
        };
        let css = add(
            ".card { padding: 8px; }",
            Kind::Text,
            &text,
            &[("font-size", "13px".into())],
        );
        let card = TextStyle {
            name: "Dish card".into(),
            class: "dish-card".into(),
        };
        let css = add(
            &css,
            Kind::Look,
            &card,
            &[
                ("background", "var(--surface)".into()),
                ("border-radius", "var(--radius)".into()),
                ("padding-left", "16px".into()),
            ],
        );
        // A text style made after a look still goes in the text section.
        let small = TextStyle {
            name: "Tiny label".into(),
            class: "tiny-label".into(),
        };
        let css = add(&css, Kind::Text, &small, &[("font-size", "11px".into())]);
        assert_eq!(own(&css, Kind::Text), vec![text, small]);
        assert_eq!(own(&css, Kind::Look), vec![card.clone()]);
        assert!(css.find(".tiny-label").unwrap() < css.find(Kind::Look.mark()).unwrap());

        let css = merge(
            &css,
            "dish-card",
            &[
                ("padding".into(), "20px".into()),
                ("box-shadow".into(), "var(--shadow)".into()),
            ],
        );
        assert_eq!(
            decls(&css, "dish-card"),
            vec![
                ("background".to_string(), "var(--surface)".to_string()),
                ("border-radius".into(), "var(--radius)".into()),
                ("padding".into(), "20px".into()),
                ("box-shadow".into(), "var(--shadow)".into()),
            ]
        );
        let css = rename(&css, "dish-card", "Menu row");
        assert_eq!(own(&css, Kind::Look)[0].name, "Menu row");
        assert_eq!(decls(&css, "dish-card").len(), 4);
        assert_eq!(
            split_decls("background-image: url(\"a;b:c.png\"); color: red"),
            vec![
                (
                    "background-image".to_string(),
                    "url(\"a;b:c.png\")".to_string()
                ),
                ("color".into(), "red".into()),
            ]
        );
    }
}
