//! Components: a piece of a screen kept once and reused as instances on any screen, each
//! with its variants. A component is a file in `design/components/`, one `<template>` per
//! variant; an instance is an element marked `data-component` and `data-variant`, so pushing
//! a change to the component updates every instance on every screen.

use std::path::{Path, PathBuf};

/// One variant of a component: its name and its HTML, the element an instance starts as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Variant {
    pub name: String,
    pub html: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Component {
    /// The file's name, and what instances are marked with.
    pub id: String,
    pub name: String,
    pub variants: Vec<Variant>,
}

/// The variant a new component starts with.
pub(crate) const DEFAULT: &str = "Default";

pub(crate) fn folder(root: &Path) -> PathBuf {
    super::model::folder(root).join("components")
}

fn file(root: &Path, id: &str) -> PathBuf {
    folder(root).join(format!("{id}.html"))
}

impl Component {
    pub(crate) fn variant(&self, name: &str) -> Option<&Variant> {
        self.variants.iter().find(|v| v.name == name)
    }

    /// The variants by name, as the page's `sync` call takes them.
    pub(crate) fn map(&self) -> serde_json::Map<String, serde_json::Value> {
        self.variants
            .iter()
            .map(|v| (v.name.clone(), v.html.clone().into()))
            .collect()
    }

    /// Set a variant's HTML, adding the variant when it's new.
    pub(crate) fn set(&mut self, name: &str, html: &str) {
        match self.variants.iter_mut().find(|v| v.name == name) {
            Some(v) => v.html = html.to_string(),
            None => self.variants.push(Variant {
                name: name.to_string(),
                html: html.to_string(),
            }),
        }
    }

    /// A name for one more variant: "Variant 2", "Variant 3", …
    pub(crate) fn next_variant(&self) -> String {
        (2..)
            .map(|n| format!("Variant {n}"))
            .find(|n| self.variant(n).is_none())
            .unwrap_or_default()
    }

    /// The component as its file holds it.
    pub(crate) fn write(&self) -> String {
        let mut out = format!(
            "<!-- component: {} -->\n<!-- Each template is one variant. Instances on the screens carry \
             data-component=\"{}\" and data-variant. -->\n",
            self.name.replace("--", "-"),
            self.id
        );
        for v in &self.variants {
            out.push_str(&format!(
                "<template data-variant=\"{}\">\n{}\n</template>\n",
                attr(&v.name),
                v.html.trim()
            ));
        }
        out
    }

    /// A component from its file.
    pub(crate) fn read(id: &str, text: &str) -> Component {
        let name = text
            .split_once("<!-- component:")
            .and_then(|(_, rest)| rest.split_once("-->"))
            .map(|(n, _)| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_string());
        let mut variants = Vec::new();
        let mut rest = text;
        while let Some(i) = rest.find("<template data-variant=\"") {
            let after = &rest[i + "<template data-variant=\"".len()..];
            let Some(q) = after.find('"') else {
                break;
            };
            let Some(open) = after[q..].find('>') else {
                break;
            };
            let body = &after[q + open + 1..];
            let Some(close) = body.find("</template>") else {
                break;
            };
            variants.push(Variant {
                name: unattr(&after[..q]),
                html: body[..close].trim().to_string(),
            });
            rest = &body[close..];
        }
        Component {
            id: id.to_string(),
            name,
            variants,
        }
    }
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

fn unattr(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&amp;", "&")
}

/// Every component of the design, by name.
pub(crate) fn load(root: &Path) -> Vec<Component> {
    let mut out: Vec<Component> = std::fs::read_dir(folder(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("html") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let text = std::fs::read_to_string(&path).ok()?;
            Some(Component::read(&id, &text))
        })
        .filter(|c| !c.variants.is_empty())
        .collect();
    out.sort_by_key(|c| c.name.to_lowercase());
    out
}

pub(crate) fn save(root: &Path, c: &Component) -> Result<(), String> {
    let path = file(root, &c.id);
    std::fs::create_dir_all(folder(root))
        .and_then(|_| std::fs::write(&path, c.write()))
        .map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

/// An id for a new component called `name`, not taken by any in `have`.
pub(crate) fn new_id(have: &[Component], name: &str) -> String {
    let base = match super::model::slug(name) {
        s if s.is_empty() => "component".to_string(),
        s => s,
    };
    let taken = |id: &str| have.iter().any(|c| c.id == id);
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap_or(base)
}

/// What to call a component made from an element: what it is when its tag says, else its
/// classes (leaving out ones that only lay things out), else its text, else its first
/// class or its tag.
pub(crate) fn name_for(tag: &str, classes: &[String], text: Option<&str>) -> String {
    const LAYOUT: [&str; 9] = [
        "grow", "row", "stack", "muted", "center", "flex", "col", "column", "wrap",
    ];
    let named = match tag {
        "button" => Some("Button"),
        "input" | "textarea" | "select" => Some("Input"),
        "a" => Some("Link"),
        "img" | "picture" => Some("Image"),
        "nav" => Some("Nav"),
        "header" => Some("Header"),
        "footer" => Some("Footer"),
        _ => None,
    };
    let first = classes.first().map(String::as_str).unwrap_or(tag);
    let classes: Vec<&str> = classes
        .iter()
        .map(String::as_str)
        .filter(|c| !LAYOUT.contains(c) && !c.starts_with("gap-") && !c.starts_with("p-"))
        .collect();
    let text = text
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.len() <= 24);
    let words = match (named, classes.is_empty(), text) {
        (Some(n), _, _) => n.to_string(),
        (None, false, _) => classes.join(" ").replace(['-', '_'], " "),
        (None, true, Some(t)) => t.to_string(),
        _ => first.to_string(),
    };
    let mut c = words.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => "Component".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_keep_their_variants_in_a_file() {
        let mut c = Component {
            id: "button".into(),
            name: "Button \"main\"".into(),
            variants: vec![Variant {
                name: DEFAULT.into(),
                html: "<button data-component=\"button\" data-variant=\"Default\">Go</button>"
                    .into(),
            }],
        };
        c.set("Hover \"on\"", "<button class=\"hover\">Go</button>");
        c.set(DEFAULT, "<button>Went</button>");
        let back = Component::read("button", &c.write());
        assert_eq!(back, c);
        assert_eq!(back.next_variant(), "Variant 2");
        assert_eq!(back.map()["Default"], "<button>Went</button>");
    }

    #[test]
    fn components_are_named_for_what_they_were_made_from() {
        assert_eq!(
            name_for("button", &["primary".into()], Some("Go")),
            "Button"
        );
        assert_eq!(
            name_for("div", &["card".into(), "list-row".into()], None),
            "Card list row"
        );
        assert_eq!(
            name_for("span", &["grow".into()], Some("Push notifications")),
            "Push notifications"
        );
        assert_eq!(name_for("div", &["row".into()], None), "Row");
        assert_eq!(name_for("div", &[], None), "Div");
        let have = vec![Component {
            id: "card".into(),
            name: "Card".into(),
            variants: Vec::new(),
        }];
        assert_eq!(new_id(&have, "Card"), "card-2");
        assert_eq!(new_id(&have, "Tab bar"), "tab-bar");
        assert_eq!(new_id(&have, "!!"), "component");
    }

    #[test]
    fn the_library_is_read_from_the_design() {
        let root = std::env::temp_dir().join(format!("runo-library-{}", std::process::id()));
        let c = Component {
            id: "chip".into(),
            name: "Chip".into(),
            variants: vec![Variant {
                name: DEFAULT.into(),
                html: "<span class=\"chip\">New</span>".into(),
            }],
        };
        save(&root, &c).unwrap();
        assert_eq!(load(&root), vec![c]);
        let _ = std::fs::remove_dir_all(root);
    }
}
