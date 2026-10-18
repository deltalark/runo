//! What Design keeps when AI changes the design, checked after every AI turn rather than
//! only asked for in its brief, so an AI change follows the same rules as one made by
//! hand: layers keep the ids a handoff gave them, libraries added from other projects stay
//! as they were published, and a component mapped to the app's code keeps its variants.

use super::codemap::CodeMap;
use super::library::{self, Component};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// What was put back after an AI change.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Kept {
    /// Layer ids put back on the layers that lost them, or taken off copies.
    pub ids: usize,
    /// Libraries whose files AI changed, put back as published.
    pub libraries: Vec<String>,
    /// Variants of mapped components AI renamed or removed, added back: "Button: Pressed".
    pub variants: Vec<String>,
}

impl Kept {
    /// One line for the person, when anything was put back.
    pub(crate) fn line(&self) -> Option<String> {
        let mut parts = Vec::new();
        match self.ids {
            0 => {}
            1 => parts.push("put back 1 layer id the handoff relies on".to_string()),
            n => parts.push(format!("put back {n} layer ids the handoff relies on")),
        }
        if !self.libraries.is_empty() {
            parts.push(format!(
                "undid changes to the {} {}, which are used as published",
                self.libraries.join(", "),
                if self.libraries.len() == 1 {
                    "library"
                } else {
                    "libraries"
                }
            ));
        }
        if !self.variants.is_empty() {
            parts.push(format!(
                "kept the variants the app's code uses ({})",
                self.variants.join(", ")
            ));
        }
        (!parts.is_empty()).then(|| format!("Design {}.", parts.join("; ")))
    }
}

/// Put back what an AI change shouldn't have changed, comparing the design folder with
/// `before`, the files as they were when it was asked.
pub(crate) fn keep(root: &Path, before: &HashMap<PathBuf, Vec<u8>>, map: &CodeMap) -> Kept {
    let mut kept = Kept {
        libraries: keep_libraries(root, before),
        ..Kept::default()
    };
    let screens = super::model::folder(root).join("screens");
    for (path, was) in before {
        if path.parent() != Some(screens.as_path()) || path.extension().is_none_or(|e| e != "html")
        {
            continue;
        }
        let (Ok(was), Ok(now)) = (std::str::from_utf8(was), std::fs::read_to_string(path)) else {
            continue;
        };
        let (fixed, n) = keep_ids(was, &now);
        if n > 0 && std::fs::write(path, fixed).is_ok() {
            kept.ids += n;
        }
    }
    kept.variants = keep_variants(root, before, map);
    kept
}

/// Put every file of an added library back as it was, by library.
fn keep_libraries(root: &Path, before: &HashMap<PathBuf, Vec<u8>>) -> Vec<String> {
    let comps = library::folder(root);
    let lib_of = |p: &Path| -> Option<String> {
        let rel = p.strip_prefix(&comps).ok()?;
        let mut parts = rel.components();
        let first = parts.next()?.as_os_str().to_str()?.to_string();
        parts.next().map(|_| first)
    };
    let mut libs: Vec<String> = Vec::new();
    let mut touched = |lib: String| {
        if !libs.contains(&lib) {
            libs.push(lib);
        }
    };
    let mut now = Vec::new();
    for e in std::fs::read_dir(&comps).into_iter().flatten().flatten() {
        if e.path().is_dir() {
            files(&e.path(), &mut now);
        }
    }
    for p in now {
        if !before.contains_key(&p) {
            if let Some(lib) = lib_of(&p) {
                if std::fs::remove_file(&p).is_ok() {
                    touched(lib);
                }
            }
        }
    }
    for (p, bytes) in before {
        let Some(lib) = lib_of(p) else {
            continue;
        };
        if std::fs::read(p).ok().as_ref() != Some(bytes) {
            let put = p
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|_| std::fs::write(p, bytes));
            if put.is_ok() {
                touched(lib);
            }
        }
    }
    libs.sort();
    libs
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Add back the variants AI renamed or removed from a component mapped to the app's code,
/// since the code uses them by name.
fn keep_variants(root: &Path, before: &HashMap<PathBuf, Vec<u8>>, map: &CodeMap) -> Vec<String> {
    let mut out = Vec::new();
    for id in map.components.keys() {
        let path = library::folder(root).join(format!("{id}.html"));
        let Some(was) = before
            .get(&path)
            .and_then(|b| std::str::from_utf8(b).ok())
            .map(|t| Component::read(id, t))
        else {
            continue;
        };
        let mut now = match std::fs::read_to_string(&path) {
            Ok(t) => Component::read(id, &t),
            Err(_) => Component {
                variants: Vec::new(),
                ..was.clone()
            },
        };
        let lost = missing_variants(&was, &now);
        if lost.is_empty() {
            continue;
        }
        for v in &lost {
            now.set(&v.name, &v.html);
            out.push(format!("{}: {}", was.name, v.name));
        }
        if library::save(root, &now).is_err() {
            out.truncate(out.len() - lost.len());
        }
    }
    out
}

fn missing_variants(was: &Component, now: &Component) -> Vec<library::Variant> {
    was.variants
        .iter()
        .filter(|v| now.variant(&v.name).is_none())
        .cloned()
        .collect()
}

/// An opening tag in a screen's HTML.
struct Tag {
    /// Where its name ends, to add an attribute after it.
    name_end: usize,
    name: String,
    /// Each attribute's name, value, and where it is, with the space before it.
    attrs: Vec<(String, String, Range<usize>)>,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, v, _)| v.as_str())
    }

    /// What a layer is recognised by when its id is gone: its tag, name, classes, and the
    /// component it's an instance of.
    fn looks(&self) -> (String, Option<&str>, Option<&str>, Option<&str>) {
        (
            self.name.clone(),
            self.attr("data-name"),
            self.attr("class"),
            self.attr("data-component"),
        )
    }
}

/// The opening tags of `html`, outside comments, scripts and styles.
fn tags(html: &str) -> Vec<Tag> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = html[i..].find('<') {
        i += off;
        if html[i..].starts_with("<!--") {
            i = html[i..].find("-->").map_or(b.len(), |e| i + e + 3);
            continue;
        }
        if !b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
            j += 1;
        }
        let name = html[start..j].to_ascii_lowercase();
        let name_end = j;
        let mut attrs = Vec::new();
        loop {
            let space = j;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() || b[j] == b'>' || b[j] == b'/' && b.get(j + 1) == Some(&b'>') {
                break;
            }
            if b[j] == b'/' {
                j += 1;
                continue;
            }
            let n0 = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && !b"=>/".contains(&b[j]) {
                j += 1;
            }
            let attr = html[n0..j].to_ascii_lowercase();
            let mut value = String::new();
            if b.get(j) == Some(&b'=') {
                j += 1;
                match b.get(j) {
                    Some(&q @ (b'"' | b'\'')) => {
                        let close = html[j + 1..].find(q as char).map_or(b.len(), |e| j + 1 + e);
                        value = html[j + 1..close].to_string();
                        j = (close + 1).min(b.len());
                    }
                    _ => {
                        let v0 = j;
                        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                            j += 1;
                        }
                        value = html[v0..j].to_string();
                    }
                }
            }
            if j == n0 {
                j += 1;
                continue;
            }
            attrs.push((attr, value, space..j));
        }
        let end = html[j..].find('>').map_or(b.len(), |e| j + e + 1);
        if name == "script" || name == "style" {
            let close = format!("</{name}");
            i = html[end..]
                .to_ascii_lowercase()
                .find(&close)
                .map_or(b.len(), |e| end + e);
        } else {
            i = end;
        }
        out.push(Tag {
            name_end,
            name,
            attrs,
        });
    }
    out
}

/// Keep the layer ids a screen had before an AI change: take ids off copies, keeping the
/// first, and put each lost id back on the one layer that looks like the layer it was on.
/// Returns the screen's HTML, and how many ids were fixed.
pub(crate) fn keep_ids(before: &str, now: &str) -> (String, usize) {
    let had: Vec<Tag> = tags(before)
        .into_iter()
        .filter(|t| t.attr("data-rid").is_some())
        .collect();
    if had.is_empty() {
        return (now.to_string(), 0);
    }
    let tags = tags(now);
    // Edits by where they go: a range to take out, or text to put in.
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut seen = HashSet::new();
    let mut bare: Vec<&Tag> = Vec::new();
    for t in &tags {
        match t.attrs.iter().find(|(n, _, _)| n == "data-rid") {
            Some((_, rid, at)) if !seen.insert(rid.clone()) => {
                edits.push((at.clone(), String::new()));
                bare.push(t);
            }
            Some(_) => {}
            None => bare.push(t),
        }
    }
    // Lost ids, and the layers without one, by how they look.
    let mut lost: HashMap<_, Vec<&str>> = HashMap::new();
    for t in &had {
        let rid = t.attr("data-rid").unwrap_or_default();
        if !seen.contains(rid) {
            lost.entry(t.looks()).or_default().push(rid);
        }
    }
    let mut like: HashMap<_, Vec<&Tag>> = HashMap::new();
    for t in bare {
        like.entry(t.looks()).or_default().push(t);
    }
    for (looks, rids) in &lost {
        // Only where it's plain which layer each id was on.
        if let Some(ts) = like.get(looks).filter(|ts| ts.len() == rids.len()) {
            for (t, rid) in ts.iter().zip(rids) {
                edits.push((t.name_end..t.name_end, format!(" data-rid=\"{rid}\"")));
            }
        }
    }
    let n = edits.len();
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = now.to_string();
    for (r, text) in edits {
        out.replace_range(r, &text);
    }
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_ids_go_back_and_copies_lose_theirs() {
        let before = r#"<body><div data-rid="a1" class="card" data-name="Card"><h2 data-rid="b2">Hi</h2><button data-rid="c3" class="btn">Go</button></div></body>"#;
        // AI rewrote the card: the heading lost its id, the button was copied with its id,
        // and a new paragraph came in without one.
        let now = r#"<body><div data-rid="a1" class="card" data-name="Card"><h2 class="">Hello</h2><p>New</p><button data-rid="c3" class="btn">Go</button><button data-rid="c3" class="btn">Again</button></div></body>"#;
        let (fixed, n) = keep_ids(before, now);
        assert_eq!(
            n, 1,
            "only the copy: the heading's class changed, so it's unclear"
        );
        assert_eq!(fixed.matches("data-rid=\"c3\"").count(), 1);
        let now = r#"<body><div data-rid="a1" class="card" data-name="Card"><h2>Hello</h2><p>New</p></div></body>"#;
        let (fixed, n) = keep_ids(before, now);
        assert_eq!(n, 1);
        assert!(fixed.contains(r#"<h2 data-rid="b2">Hello</h2>"#));
        assert!(
            fixed.contains("<p>New</p>"),
            "a new layer gets its id at handoff"
        );
    }

    #[test]
    fn unclear_layers_are_left_alone() {
        let before = r#"<li data-rid="x1">A</li><li data-rid="x2">B</li>"#;
        let now = r#"<li>A</li><li>B</li><li>C</li>"#;
        assert_eq!(keep_ids(before, now), (now.to_string(), 0));
        // A screen without ids yet, before any handoff, is left as it is.
        assert_eq!(keep_ids("<li>A</li>", "<li>B</li>").1, 0);
    }

    #[test]
    fn scripts_styles_and_comments_are_not_layers() {
        let html = r#"<style>a<b{}</style><!-- <i data-rid="z"> --><script>if(a<b){}</script><b data-rid='q' hidden>x</b>"#;
        let t = tags(html);
        let names: Vec<&str> = t.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["style", "script", "b"]);
        assert_eq!(t[2].attr("data-rid"), Some("q"));
        assert_eq!(t[2].attr("hidden"), Some(""));
    }

    #[test]
    fn libraries_and_mapped_variants_are_put_back() {
        let dir = std::env::temp_dir().join(format!("runo-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let comps = library::folder(&dir);
        let lib = comps.join("brand");
        std::fs::create_dir_all(&lib).unwrap();
        let mut button = Component {
            id: "button".into(),
            name: "Button".into(),
            variants: Vec::new(),
        };
        button.set("State=Default", "<button>Go</button>");
        button.set("State=Pressed", "<button class=\"down\">Go</button>");
        library::save(&dir, &button).unwrap();
        std::fs::write(lib.join("chip.html"), "<template>chip</template>").unwrap();
        let mut before = HashMap::new();
        for p in [comps.join("button.html"), lib.join("chip.html")] {
            before.insert(p.clone(), std::fs::read(&p).unwrap());
        }
        // AI edits the library, adds a file to it, and renames a mapped variant.
        std::fs::write(lib.join("chip.html"), "changed").unwrap();
        std::fs::write(lib.join("new.html"), "new").unwrap();
        let mut renamed = button.clone();
        renamed.variants[1].name = "State=Active".into();
        library::save(&dir, &renamed).unwrap();
        let mut map = CodeMap::default();
        map.set(
            "button",
            super::super::codemap::Target::Web,
            "@/ui",
            "Button",
        );
        let kept = keep(&dir, &before, &map);
        assert_eq!(kept.libraries, ["brand"]);
        assert_eq!(kept.variants, ["Button: State=Pressed"]);
        assert_eq!(
            std::fs::read_to_string(lib.join("chip.html")).unwrap(),
            "<template>chip</template>"
        );
        assert!(!lib.join("new.html").exists());
        let now = &library::load(&dir)[0];
        assert!(now.variant("State=Active").is_some() && now.variant("State=Pressed").is_some());
        assert!(kept.line().unwrap().contains("brand library"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
