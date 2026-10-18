//! Text styles: named type (size, weight, line height, letter spacing) applied as a class.
//! The kit's own are classes `components.css` always has; ones made in Design are kept at the
//! end of the project's `components.css`, one rule a line, so agents and people can use them.

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

/// Where the styles made in Design start in `components.css`.
const MARK: &str = "/* Text styles made in Design. */";

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
        .chain(own(css))
        .collect()
}

/// The text styles made in Design, from lines like `/* Card title */ .card-title { … }`.
pub(crate) fn own(css: &str) -> Vec<TextStyle> {
    let Some((_, rest)) = css.split_once(MARK) else {
        return Vec::new();
    };
    rest.lines()
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

/// `css` with a style added at the end, in the Design section.
pub(crate) fn add(css: &str, style: &TextStyle, decls: &[(&str, String)]) -> String {
    let mut out = css.trim_end().to_string();
    if !out.contains(MARK) {
        out.push_str("\n\n");
        out.push_str(MARK);
    }
    let body: Vec<String> = decls
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| format!("{k}: {};", v.trim()))
        .collect();
    out.push_str(&format!(
        "\n/* {} */ .{} {{ {} }}\n",
        style.name.replace("*/", ""),
        style.class,
        body.join(" ")
    ));
    out
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
            &s,
            &[
                ("font-size", "18px".into()),
                ("font-weight", "600".into()),
                ("line-height", "".into()),
            ],
        );
        assert!(css.contains(".text-18-semibold { font-size: 18px; font-weight: 600; }"));
        assert_eq!(own(&css), vec![s.clone()]);
        assert_eq!(class_for(&css, "18 Semibold"), "text-18-semibold-2");
        assert_eq!(class_for(&css, "Card"), "card-2");
        let more = add(
            &css,
            &TextStyle {
                name: "Label".into(),
                class: "label".into(),
            },
            &[("font-size", "13px".into())],
        );
        assert_eq!(more.matches(MARK).count(), 1);
        assert_eq!(own(&more).len(), 2);
        assert_eq!(all(&more).last().unwrap().name, "Label");
    }
}
