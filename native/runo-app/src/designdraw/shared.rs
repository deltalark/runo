//! Libraries shared across projects. A project publishes its components and styles to
//! Runo's data folder (`libraries/<id>/`), and any other project adds them: they're copied
//! into its `design/components/<id>/`, so its screens keep working with the library's
//! files gone, and instances carry `data-component="<id>/<component>"`. The library's
//! styles come along as one sheet scoped to its instances (`data-library`), so they
//! don't change the rest of the screen. Publishing again makes a new version, which the
//! projects that added it can take when they choose.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A library's facts, kept with it in `library.toml`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct Library {
    #[serde(skip)]
    pub id: String,
    pub name: String,
    pub version: u32,
    /// The project it's published from.
    pub from: String,
    /// How many components it has.
    #[serde(default)]
    pub components: usize,
}

const FACTS: &str = "library.toml";
/// The library's styles, scoped to its instances, in the project that added it.
pub(crate) const SHEET: &str = "library.css";

fn home() -> PathBuf {
    crate::store::data_dir().join("libraries")
}

fn read(dir: &Path) -> Option<Library> {
    let text = std::fs::read_to_string(dir.join(FACTS)).ok()?;
    let mut lib: Library = toml::from_str(&text).ok()?;
    lib.id = dir.file_name()?.to_str()?.to_string();
    Some(lib)
}

fn libraries_in(dir: &Path) -> Vec<Library> {
    let mut out: Vec<Library> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read(&e.path()))
        .collect();
    out.sort_by_key(|l| l.name.to_lowercase());
    out
}

/// Every library published from any project.
pub(crate) fn published() -> Vec<Library> {
    libraries_in(&home())
}

/// The libraries this project has added, at the version it took.
pub(crate) fn added(root: &Path) -> Vec<Library> {
    libraries_in(&super::library::folder(root))
}

/// Which project a library was published from, as written in its facts.
pub(crate) fn origin(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

/// Publish this project's own components and styles as the library `name`, or a new
/// version of it.
pub(crate) fn publish(root: &Path, name: &str) -> Result<Library, String> {
    let id = super::model::slug(name);
    if id.is_empty() {
        return Err("Give the library a name.".into());
    }
    let dir = home().join(&id);
    let was = read(&dir);
    if let Some(w) = was.as_ref().filter(|w| w.from != origin(root)) {
        return Err(format!(
            "There's already a library called {} from another project.",
            w.name
        ));
    }
    let own = super::library::folder(root);
    let comps = dir.join("components");
    let _ = std::fs::remove_dir_all(&comps);
    std::fs::create_dir_all(&comps).map_err(|e| e.to_string())?;
    let mut count = 0;
    for e in std::fs::read_dir(&own).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_file() && p.extension().and_then(|x| x.to_str()) == Some("html") {
            std::fs::copy(&p, comps.join(e.file_name())).map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    let design = super::model::folder(root);
    for sheet in ["tokens.css", "components.css"] {
        let text = std::fs::read_to_string(design.join(sheet)).unwrap_or_default();
        std::fs::write(dir.join(sheet), text).map_err(|e| e.to_string())?;
    }
    let lib = Library {
        id,
        name: name.trim().to_string(),
        version: was.map_or(1, |w| w.version + 1),
        from: origin(root),
        components: count,
    };
    write_facts(&dir, &lib)?;
    Ok(lib)
}

fn write_facts(dir: &Path, lib: &Library) -> Result<(), String> {
    let text = toml::to_string(lib).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(FACTS), text).map_err(|e| e.to_string())
}

/// Add the published library `id` to this project, or take its newest version.
pub(crate) fn add(root: &Path, id: &str) -> Result<Library, String> {
    let from = home().join(id);
    let lib = read(&from).ok_or_else(|| format!("The library {id} isn't published."))?;
    let to = super::library::folder(root).join(id);
    let _ = std::fs::remove_dir_all(&to);
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(from.join("components"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = e.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let mut c = super::library::Component::read(stem, &text);
        c.id = format!("{id}/{stem}");
        for v in &mut c.variants {
            v.html = adopt(id, &v.html);
        }
        std::fs::write(to.join(e.file_name()), c.write()).map_err(|e| e.to_string())?;
    }
    let css: String = ["tokens.css", "components.css"]
        .iter()
        .map(|s| std::fs::read_to_string(from.join(s)).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(to.join(SHEET), scoped(id, &css)).map_err(|e| e.to_string())?;
    write_facts(&to, &lib)?;
    Ok(lib)
}

/// Take the library `id` out of this project. Its instances stay as they are.
pub(crate) fn remove(root: &Path, id: &str) -> Result<(), String> {
    std::fs::remove_dir_all(super::library::folder(root).join(id)).map_err(|e| e.to_string())
}

/// Where a screen links the library's styles from.
pub(crate) fn sheet_href(id: &str) -> String {
    format!("../components/{id}/{SHEET}")
}

/// The library's styles, kept to its instances: its root variables become theirs, and
/// its rules reach only inside them.
pub(crate) fn scoped(id: &str, css: &str) -> String {
    format!(
        "/* The {id} library's styles, for its instances only. */\n@scope ([data-library=\"{id}\"]) {{\n{}\n}}\n",
        css.replace(":root", ":scope")
    )
}

/// A variant's HTML as the project that added library `id` keeps it: its instances, and
/// any inside it, name the library's components, and its outermost element is marked as
/// the library's.
pub(crate) fn adopt(id: &str, html: &str) -> String {
    let mut out = html.replace("data-component=\"", &format!("data-component=\"{id}/"));
    if let Some(start) = out.find('<') {
        let end = out[start + 1..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .map(|i| start + 1 + i);
        if let Some(end) = end {
            out.insert_str(end, &format!(" data-library=\"{id}\""));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{adopt, scoped};

    #[test]
    fn a_library_component_names_its_library() {
        let html = r#"<button class="button" data-component="button" data-variant="Default"><i data-component="icon"></i></button>"#;
        assert_eq!(
            adopt("brand", html),
            r#"<button data-library="brand" class="button" data-component="brand/button" data-variant="Default"><i data-component="brand/icon"></i></button>"#
        );
    }

    #[test]
    fn library_styles_stay_on_its_instances() {
        let css = scoped(
            "brand",
            ":root { --brand: red; }\n.button { color: var(--brand); }",
        );
        assert!(css.starts_with("/* The brand"));
        assert!(css.contains("@scope ([data-library=\"brand\"]) {\n:scope { --brand: red; }"));
    }
}
