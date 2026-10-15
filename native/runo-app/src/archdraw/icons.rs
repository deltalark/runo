//! A searchable library of logos and tech icons for diagrams: Simple Icons (brand logos
//! such as Docker, Node.js or Postgres) and Devicon (languages and tools). Nothing is
//! bundled: each set's index is downloaded the first time someone searches, and each icon
//! when it's first shown, then kept in Runo's data folder so they work offline. An icon
//! placed on a diagram is copied into `architecture/icons/` in the repository, so the
//! diagram carries it everywhere.

use super::model::FOLDER;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const SIMPLE_ICONS: &str = "16.35.0";
const DEVICON: &str = "2.16.0";

/// Where an icon comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Set {
    Logos,
    Devicon,
}

impl Set {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Set::Logos => "Simple Icons",
            Set::Devicon => "Devicon",
        }
    }

    fn dir(self) -> &'static str {
        match self {
            Set::Logos => "simple-icons",
            Set::Devicon => "devicon",
        }
    }
}

/// One icon in the library.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Icon {
    pub set: Set,
    /// The file name in its set, without `.svg`.
    pub slug: String,
    pub title: String,
    /// Other names and tags, lowercased, for search.
    pub words: Vec<String>,
    /// The brand colour of a one-colour logo, as `rrggbb`.
    pub color: Option<String>,
}

/// Every icon of both sets.
#[derive(Debug, Default)]
pub(crate) struct Library {
    pub icons: Vec<Icon>,
}

impl Library {
    /// The icons that best match a search, best first.
    pub(crate) fn search(&self, query: &str, limit: usize) -> Vec<&Icon> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let squashed: String = q.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let mut hits: Vec<(u8, usize, &Icon)> = self
            .icons
            .iter()
            .filter_map(|i| {
                let t = i.title.to_lowercase();
                let rank = if t == q || i.slug == squashed {
                    0
                } else if t.starts_with(&q)
                    || (!squashed.is_empty() && i.slug.starts_with(&squashed))
                {
                    1
                } else if t.split([' ', '-', '.', '/']).any(|w| w.starts_with(&q)) {
                    2
                } else if i.words.iter().any(|w| w == &q) {
                    3
                } else if t.contains(&q) {
                    4
                } else if i.words.iter().any(|w| w.starts_with(&q)) {
                    5
                } else {
                    return None;
                };
                // The same name in both sets: Devicon's full-colour logo first.
                let set = usize::from(i.set == Set::Logos);
                Some((rank, t.len() * 2 + set, i))
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        hits.into_iter().take(limit).map(|h| h.2).collect()
    }
}

/// Runo's own copies of indexes and icons.
pub(crate) fn cache_dir() -> PathBuf {
    crate::store::data_dir().join("icons")
}

/// The places a file of a set can be downloaded from, in order: a CDN, then GitHub.
fn urls(set: Set, path: &str) -> Vec<String> {
    match set {
        Set::Logos => vec![
            format!("https://cdn.jsdelivr.net/npm/simple-icons@{SIMPLE_ICONS}/{path}"),
            format!(
                "https://raw.githubusercontent.com/simple-icons/simple-icons/{SIMPLE_ICONS}/{path}"
            ),
        ],
        Set::Devicon => vec![
            format!("https://cdn.jsdelivr.net/gh/devicons/devicon@v{DEVICON}/{path}"),
            format!("https://raw.githubusercontent.com/devicons/devicon/v{DEVICON}/{path}"),
        ],
    }
}

/// Download a file, trying each place in turn.
fn download(urls: &[String]) -> Result<Vec<u8>, String> {
    let mut why = String::from("no address");
    for url in urls {
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "30", url])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|_| "curl isn't available".to_string())?;
        if out.status.success() && !out.stdout.is_empty() {
            return Ok(out.stdout);
        }
        why = String::from_utf8_lossy(&out.stderr).trim().to_string();
    }
    Err(why)
}

/// A file of a set from the cache, downloading it the first time.
fn cached(set: Set, path: &str, cache: &str) -> Result<Vec<u8>, String> {
    let file = cache_dir().join(set.dir()).join(cache);
    if let Ok(bytes) = std::fs::read(&file) {
        if !bytes.is_empty() {
            return Ok(bytes);
        }
    }
    let bytes = download(&urls(set, path))?;
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&file, &bytes);
    Ok(bytes)
}

/// Read both sets' indexes, downloading them the first time. Blocks; call it off the UI
/// thread. Fails only if neither set can be had.
pub(crate) fn load() -> Result<Library, String> {
    let mut icons = Vec::new();
    let mut errors = Vec::new();
    match cached(Set::Logos, "data/simple-icons.json", "index.json") {
        Ok(bytes) => icons.extend(parse_simple_icons(&bytes)),
        Err(e) => errors.push(e),
    }
    match cached(Set::Devicon, "devicon.json", "index.json") {
        Ok(bytes) => icons.extend(parse_devicon(&bytes)),
        Err(e) => errors.push(e),
    }
    if icons.is_empty() {
        return Err(format!(
            "Couldn't download the icon library. {}",
            errors.first().cloned().unwrap_or_default()
        ));
    }
    Ok(Library { icons })
}

#[derive(Deserialize)]
struct SimpleIcon {
    title: String,
    #[serde(default)]
    hex: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    aliases: Option<SimpleAliases>,
}

#[derive(Deserialize)]
struct SimpleAliases {
    #[serde(default)]
    aka: Vec<String>,
}

/// Simple Icons' index: a list, or `{ "icons": [...] }` in older releases.
pub(crate) fn parse_simple_icons(bytes: &[u8]) -> Vec<Icon> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Index {
        List(Vec<SimpleIcon>),
        Wrapped { icons: Vec<SimpleIcon> },
    }
    let list = match serde_json::from_slice::<Index>(bytes) {
        Ok(Index::List(l)) | Ok(Index::Wrapped { icons: l }) => l,
        Err(_) => return Vec::new(),
    };
    list.into_iter()
        .map(|i| Icon {
            set: Set::Logos,
            slug: i.slug.clone().unwrap_or_else(|| title_to_slug(&i.title)),
            words: i
                .aliases
                .map(|a| a.aka.iter().map(|w| w.to_lowercase()).collect())
                .unwrap_or_default(),
            color: (i.hex.len() == 6).then_some(i.hex),
            title: i.title,
        })
        .filter(|i| !i.slug.is_empty())
        .collect()
}

#[derive(Deserialize)]
struct DevIcon {
    name: String,
    #[serde(default)]
    altnames: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    versions: DevVersions,
}

#[derive(Deserialize, Default)]
struct DevVersions {
    #[serde(default)]
    svg: Vec<String>,
}

/// Devicon's index. Each icon is kept under the file of its best version: the full-colour
/// original where there is one.
pub(crate) fn parse_devicon(bytes: &[u8]) -> Vec<Icon> {
    let Ok(list) = serde_json::from_slice::<Vec<DevIcon>>(bytes) else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|i| {
            let v = ["original", "plain", "original-wordmark", "plain-wordmark"]
                .into_iter()
                .find(|v| i.versions.svg.iter().any(|s| s == v))
                .map(String::from)
                .or_else(|| i.versions.svg.first().cloned())?;
            let title = i
                .altnames
                .first()
                .filter(|a| a.len() > i.name.len())
                .cloned()
                .unwrap_or_else(|| i.name.clone());
            // Devicon names are lowercase file names: "docker" reads as "Docker".
            let title = if title.chars().any(|c| c.is_uppercase()) {
                title
            } else {
                let mut c = title.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            };
            let mut words: Vec<String> = i.altnames.iter().map(|w| w.to_lowercase()).collect();
            words.extend(i.tags.iter().map(|w| w.to_lowercase()));
            words.push(i.name.to_lowercase());
            Some(Icon {
                set: Set::Devicon,
                slug: format!("{}/{}-{v}", i.name, i.name),
                title,
                words,
                color: None,
            })
        })
        .collect()
}

/// Simple Icons' file name for a title: "Node.js" is `nodedotjs`, "C++" is `cplusplus`.
pub(crate) fn title_to_slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        match c {
            '+' => out.push_str("plus"),
            '.' => out.push_str("dot"),
            '&' => out.push_str("and"),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'đ' => out.push('d'),
            'ħ' => out.push('h'),
            'ı' => out.push('i'),
            'ĸ' => out.push('k'),
            'ŀ' | 'ł' => out.push('l'),
            'ŧ' => out.push('t'),
            'ø' => out.push('o'),
            c if c.is_ascii_alphanumeric() => out.push(c),
            c => {
                if let Some(base) = fold(c) {
                    out.push(base);
                }
            }
        }
    }
    out
}

/// The plain letter under an accented Latin one.
fn fold(c: char) -> Option<char> {
    const TABLE: [(&str, char); 13] = [
        ("àáâãäåāăą", 'a'),
        ("çćĉċč", 'c'),
        ("ď", 'd'),
        ("èéêëēĕėęě", 'e'),
        ("ìíîïĩīĭį", 'i'),
        ("ñńņňŉ", 'n'),
        ("òóôõöōŏő", 'o'),
        ("ŕŗř", 'r'),
        ("śŝşšș", 's'),
        ("ţťț", 't'),
        ("ùúûüũūŭůűų", 'u'),
        ("ýÿŷ", 'y'),
        ("žźż", 'z'),
    ];
    TABLE
        .iter()
        .find(|(set, _)| set.contains(c))
        .map(|(_, base)| *base)
}

/// Where an icon's picture is kept, downloading it the first time. One-colour logos are
/// kept in their brand colour, lightened when too dark to see on a dark canvas. Blocks.
pub(crate) fn fetch(icon: &Icon) -> Result<PathBuf, String> {
    let name = icon.slug.replace('/', "__");
    let file = cache_dir().join(icon.set.dir()).join(format!("{name}.svg"));
    if file.exists() {
        return Ok(file);
    }
    let path = match icon.set {
        Set::Logos => format!("icons/{}.svg", icon.slug),
        Set::Devicon => format!("icons/{}.svg", icon.slug),
    };
    let bytes = download(&urls(icon.set, &path))?;
    let mut text = String::from_utf8(bytes).map_err(|_| "The icon isn't an SVG.".to_string())?;
    if !text.contains("<svg") {
        return Err("The icon isn't an SVG.".into());
    }
    if let Some(hex) = &icon.color {
        text = text.replacen("<svg ", &format!("<svg fill=\"#{}\" ", visible(hex)), 1);
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file)
}

/// A brand colour, or a mid grey when it's nearly black and would vanish on a dark canvas.
fn visible(hex: &str) -> String {
    let v = u32::from_str_radix(hex, 16).unwrap_or(0);
    let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    let lum = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
    if lum < 40. {
        "8b95a5".into()
    } else {
        hex.to_lowercase()
    }
}

/// Whether a file can go on a diagram as a picture.
pub(crate) fn is_picture(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "svg" | "png" | "jpg" | "jpeg" | "webp" | "gif"
        )
    })
}

/// Copy a picture into the diagram's `icons/` folder and return its path there, relative
/// to the diagram folder. The same picture placed twice is kept once.
pub(crate) fn add_to_project(root: &Path, file: &Path, name: &str) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("Couldn't read the picture: {e}"))?;
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "svg".into());
    let stem: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let stem = if stem.is_empty() { "icon".into() } else { stem };
    let dir = root.join(FOLDER).join("icons");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    for i in 1.. {
        let fname = if i == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{i}.{ext}")
        };
        let target = dir.join(&fname);
        match std::fs::read(&target) {
            Ok(existing) if existing == bytes => return Ok(format!("icons/{fname}")),
            Ok(_) => continue,
            Err(_) => {
                std::fs::write(&target, &bytes)
                    .map_err(|e| format!("Couldn't save {}: {e}", target.display()))?;
                return Ok(format!("icons/{fname}"));
            }
        }
    }
    unreachable!()
}

/// Swap each shape's picture for a data URI, so an export carries its pictures inside it.
/// A picture whose file is gone is left off.
pub(crate) fn inline_pictures(root: &Path, doc: &mut super::model::Doc) {
    for e in &mut doc.elements {
        if !e.image.is_empty() {
            e.image = data_uri(root, &e.image).unwrap_or_default();
        }
    }
}

/// A picture as a data URI, for exports that must carry it inside the file.
pub(crate) fn data_uri(root: &Path, image: &str) -> Option<String> {
    use base64::Engine;
    let path = root.join(FOLDER).join(image);
    let bytes = std::fs::read(&path).ok()?;
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/svg+xml",
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn titles_become_simple_icons_file_names() {
        assert_eq!(title_to_slug("Node.js"), "nodedotjs");
        assert_eq!(title_to_slug("C++"), "cplusplus");
        assert_eq!(title_to_slug("Docker"), "docker");
        assert_eq!(title_to_slug("Amazon S3"), "amazons3");
        assert_eq!(title_to_slug("Pokémon"), "pokemon");
        assert_eq!(title_to_slug("AT&T"), "atandt");
    }

    #[test]
    fn indexes_are_read_and_searched() {
        let si = br#"[
            {"title": "Docker", "hex": "2496ED", "source": "x"},
            {"title": "Node.js", "hex": "5FA04E", "source": "x", "aliases": {"aka": ["nodejs"]}},
            {"title": "Node-RED", "hex": "8F0000", "source": "x"},
            {"title": "GitHub", "hex": "181717", "source": "x", "slug": "github"}
        ]"#;
        let dev = br#"[
            {"name": "docker", "altnames": [], "tags": ["container"],
             "versions": {"svg": ["plain", "original"]}},
            {"name": "postgresql", "altnames": ["Postgres"], "tags": ["database"],
             "versions": {"svg": ["plain"]}}
        ]"#;
        let mut icons = parse_simple_icons(si);
        icons.extend(parse_devicon(dev));
        let lib = Library { icons };
        let node = lib.search("node", 5);
        assert_eq!(node[0].title, "Node.js");
        assert_eq!(node[0].slug, "nodedotjs");
        let docker = lib.search("Docker", 5);
        assert_eq!(docker.len(), 2);
        assert_eq!(docker[0].slug, "docker/docker-original");
        assert_eq!(docker[1].set, Set::Logos);
        // Tags and other names find icons too.
        assert_eq!(
            lib.search("database", 5)[0].slug,
            "postgresql/postgresql-plain"
        );
        assert_eq!(lib.search("nodejs", 5)[0].title, "Node.js");
        assert!(lib.search("  ", 5).is_empty());
    }

    #[test]
    fn dark_logos_stay_visible() {
        assert_eq!(visible("181717"), "8b95a5");
        assert_eq!(visible("2496ED"), "2496ed");
    }

    #[test]
    fn pictures_are_copied_into_the_diagram_once() {
        let root = std::env::temp_dir().join(format!("runo-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("Docker logo.svg");
        std::fs::write(&src, "<svg/>").unwrap();
        let a = add_to_project(&root, &src, "Docker").unwrap();
        let b = add_to_project(&root, &src, "Docker").unwrap();
        assert_eq!(a, "icons/docker.svg");
        assert_eq!(a, b);
        let other = root.join("other.svg");
        std::fs::write(&other, "<svg></svg>").unwrap();
        assert_eq!(
            add_to_project(&root, &other, "Docker").unwrap(),
            "icons/docker-2.svg"
        );
        assert!(data_uri(&root, &a)
            .unwrap()
            .starts_with("data:image/svg+xml;base64,"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exports_carry_their_pictures() {
        use super::super::editor::Editor;
        use super::super::io;
        let root = std::env::temp_dir().join(format!("runo-icons-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("docker.svg");
        std::fs::write(&src, "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\"/></svg>").unwrap();
        let image = add_to_project(&root, &src, "Docker").unwrap();
        let mut ed = Editor::new(super::super::model::Doc::default());
        let id = ed.add_icon("Docker", &image, (200., 200.));
        assert_eq!(ed.kind_of(&id), super::super::model::Kind::Icon);
        let mut doc = ed.doc.clone();
        inline_pictures(&root, &mut doc);
        let view = doc.views[0].clone();
        let svg = io::svg(&doc, &view);
        assert!(
            svg.contains("<image href=\"data:image/svg+xml;base64,"),
            "{svg}"
        );
        assert!(io::png(&doc, &view).is_ok());
        let drawio = io::drawio(&doc);
        assert!(drawio.contains("shape=image;"));
        assert!(drawio.contains("image=data:image/svg+xml,"));
        let back = io::import_drawio(&drawio).unwrap();
        assert!(back
            .elements
            .iter()
            .any(|e| e.kind == super::super::model::Kind::Icon && e.name == "Docker"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
