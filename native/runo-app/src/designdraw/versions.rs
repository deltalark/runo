//! Versions of the design: copies of its screens, styles, canvas and components kept in
//! `design/.versions/<id>/`, saved when the design is opened, every so often while it's
//! changed, and by name. A version's pictures are linked to the design's own, so keeping
//! many costs little, and its screens still draw as they were, with their own styles.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const DIR: &str = ".versions";
const FACTS: &str = "version.toml";
/// Versions saved on their own that are kept; named ones are kept for good.
const KEEP_AUTO: usize = 30;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct Version {
    #[serde(skip)]
    pub id: String,
    /// What it's called: empty for one saved on its own.
    #[serde(default)]
    pub name: String,
    /// When it was saved, as shown: "Oct 17, 18:04".
    pub at: String,
    /// What the design held, to tell when it's changed since.
    pub digest: String,
    #[serde(default)]
    pub who: String,
}

impl Version {
    pub(crate) fn title(&self) -> &str {
        if self.name.is_empty() {
            "Autosave"
        } else {
            &self.name
        }
    }
}

/// How a version differs from the design now, by screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Changes {
    pub changed: Vec<String>,
    /// Screens there now that the version didn't have.
    pub added: Vec<String>,
    /// Screens the version had that are gone now.
    pub removed: Vec<String>,
    /// The styles, the canvas or the components changed.
    pub styles: bool,
}

impl Changes {
    pub(crate) fn none(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty() && !self.styles
    }
}

fn dir(root: &Path) -> PathBuf {
    super::model::folder(root).join(DIR)
}

pub(crate) fn folder_of(root: &Path, id: &str) -> PathBuf {
    dir(root).join(id)
}

/// A screen's file as a version kept it.
pub(crate) fn screen_file(root: &Path, id: &str, screen: &str) -> PathBuf {
    folder_of(root, id)
        .join(super::model::SCREENS)
        .join(format!("{screen}.html"))
}

/// The design's files a version keeps, relative to the design folder.
fn tracked(design: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(at).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if let (Ok(rel), Ok(bytes)) = (p.strip_prefix(base), std::fs::read(&p)) {
                out.insert(rel.to_path_buf(), bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(design).into_iter().flatten().flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        if p.is_file() && (matches!(ext, "css" | "js") || p.ends_with("canvas.toml")) {
            if let Ok(bytes) = std::fs::read(&p) {
                out.insert(PathBuf::from(e.file_name()), bytes);
            }
        }
    }
    for sub in [super::model::SCREENS, "components"] {
        walk(design, &design.join(sub), &mut out);
    }
    out
}

fn digest(files: &BTreeMap<PathBuf, Vec<u8>>) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn read(dir: &Path) -> Option<Version> {
    let text = std::fs::read_to_string(dir.join(FACTS)).ok()?;
    let mut v: Version = toml::from_str(&text).ok()?;
    v.id = dir.file_name()?.to_str()?.to_string();
    Some(v)
}

/// Every version, newest first.
pub(crate) fn list(root: &Path) -> Vec<Version> {
    let mut out: Vec<Version> = std::fs::read_dir(dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read(&e.path()))
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

/// Save the design as a version called `name` (empty for one saved on its own). Nothing
/// is saved on its own when nothing changed since the last version; a name given then
/// names that version instead.
pub(crate) fn save(root: &Path, name: &str, who: &str) -> Result<Option<Version>, String> {
    let design = super::model::folder(root);
    let files = tracked(&design);
    if files.is_empty() {
        return Ok(None);
    }
    let sum = digest(&files);
    if let Some(mut last) = list(root).into_iter().next().filter(|v| v.digest == sum) {
        if name.is_empty() || last.name == name {
            return Ok(None);
        }
        if last.name.is_empty() {
            last.name = name.to_string();
            write_facts(&folder_of(root, &last.id), &last)?;
            return Ok(Some(last));
        }
    }
    let now = chrono::Local::now();
    let mut id = now.format("%Y%m%d-%H%M%S").to_string();
    while folder_of(root, &id).exists() {
        id.push('a');
    }
    let to = folder_of(root, &id);
    for (rel, bytes) in &files {
        let p = to.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
    }
    // Pictures stay where they are; the version links to them so its screens still draw.
    let assets = design.join("assets");
    for e in std::fs::read_dir(&assets).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_file() {
            let dest = to.join("assets").join(e.file_name());
            let _ = std::fs::create_dir_all(to.join("assets"));
            if std::fs::hard_link(&p, &dest).is_err() {
                let _ = std::fs::copy(&p, &dest);
            }
        }
    }
    let v = Version {
        id,
        name: name.to_string(),
        at: now.format("%b %-d, %H:%M").to_string(),
        digest: sum.clone(),
        who: who.to_string(),
    };
    write_facts(&to, &v)?;
    prune(root);
    Ok(Some(v))
}

fn write_facts(dir: &Path, v: &Version) -> Result<(), String> {
    let text = toml::to_string(v).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(FACTS), text).map_err(|e| e.to_string())
}

/// Name a version, or with an empty name make it an autosave again.
pub(crate) fn rename(root: &Path, id: &str, name: &str) -> Result<(), String> {
    let dir = folder_of(root, id);
    let mut v = read(&dir).ok_or("That version is gone.")?;
    v.name = name.trim().to_string();
    write_facts(&dir, &v)
}

/// Drop the oldest versions saved on their own, past the ones kept.
fn prune(root: &Path) {
    let autos: Vec<Version> = list(root)
        .into_iter()
        .filter(|v| v.name.is_empty())
        .collect();
    for v in autos.iter().skip(KEEP_AUTO) {
        let _ = std::fs::remove_dir_all(folder_of(root, &v.id));
    }
}

/// How version `id` differs from the design now.
pub(crate) fn changes(root: &Path, id: &str) -> Changes {
    let now = tracked(&super::model::folder(root));
    let then = tracked(&folder_of(root, id));
    let screen = |p: &PathBuf| -> Option<String> {
        p.starts_with(super::model::SCREENS)
            .then(|| p.file_stem()?.to_str().map(String::from))
            .flatten()
    };
    let mut out = Changes::default();
    for (p, bytes) in &now {
        match (screen(p), then.get(p)) {
            (Some(s), None) => out.added.push(s),
            (Some(s), Some(b)) if b != bytes => out.changed.push(s),
            (None, b) if b != Some(bytes) => out.styles = true,
            _ => {}
        }
    }
    for p in then.keys().filter(|p| !now.contains_key(*p)) {
        match screen(p) {
            Some(s) => out.removed.push(s),
            None => out.styles = true,
        }
    }
    out
}

/// Put the design back as version `id` had it. Pictures added since stay.
pub(crate) fn restore(root: &Path, id: &str) -> Result<(), String> {
    let design = super::model::folder(root);
    let then = tracked(&folder_of(root, id));
    if then.is_empty() {
        return Err("That version is empty.".into());
    }
    for p in tracked(&design).keys().filter(|p| !then.contains_key(*p)) {
        let _ = std::fs::remove_file(design.join(p));
    }
    for (rel, bytes) in &then {
        let p = design.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design() -> PathBuf {
        let root = std::env::temp_dir().join(format!("runo-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = super::super::model::folder(&root);
        std::fs::create_dir_all(d.join("screens")).unwrap();
        std::fs::write(d.join("tokens.css"), ":root{}").unwrap();
        std::fs::write(d.join("screens/home.html"), "<p>one</p>").unwrap();
        root
    }

    #[test]
    fn versions_keep_what_changed_and_bring_it_back() {
        let root = design();
        let d = super::super::model::folder(&root);
        let first = save(&root, "", "Ada").unwrap().unwrap();
        // Nothing changed: nothing saved, but a name names the last one.
        assert_eq!(save(&root, "", "Ada").unwrap(), None);
        assert_eq!(save(&root, "Kickoff", "Ada").unwrap().unwrap().id, first.id);
        std::fs::write(d.join("screens/home.html"), "<p>two</p>").unwrap();
        std::fs::write(d.join("screens/new.html"), "<p>new</p>").unwrap();
        let c = changes(&root, &first.id);
        assert_eq!(c.changed, ["home"]);
        assert_eq!(c.added, ["new"]);
        assert!(!c.styles && c.removed.is_empty());
        restore(&root, &first.id).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("screens/home.html")).unwrap(),
            "<p>one</p>"
        );
        assert!(!d.join("screens/new.html").exists());
        assert!(changes(&root, &first.id).none());
        assert_eq!(list(&root)[0].title(), "Kickoff");
        let _ = std::fs::remove_dir_all(&root);
    }
}
