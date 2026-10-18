//! Handoffs: what's handed to the code that builds the design. Each one is kept in
//! `design/handoff/<n>/`: `spec.json`, which says what to build (every screen, how it's
//! laid out and what changed since the last handoff), a picture of each screen to build,
//! and `handoff.toml`. The tokens go to `design/handoff/tokens/` for the web and for the
//! mobile stack the project picked.

use super::model::Device;
use super::tokens;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DIR: &str = "handoff";
const FACTS: &str = "handoff.toml";
const SETTINGS: &str = "settings.toml";

/// What phone and tablet screens are built with. Desktop screens are built for the web,
/// with React and Tailwind.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Mobile {
    #[default]
    Flutter,
    ReactNative,
    #[serde(rename = "swiftui")]
    SwiftUi,
}

impl Mobile {
    pub(crate) const ALL: [Mobile; 3] = [Mobile::Flutter, Mobile::ReactNative, Mobile::SwiftUi];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Mobile::Flutter => "Flutter",
            Mobile::ReactNative => "React Native",
            Mobile::SwiftUi => "SwiftUI",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Mobile::Flutter => "flutter",
            Mobile::ReactNative => "react-native",
            Mobile::SwiftUi => "swiftui",
        }
    }

    /// The theme file its tokens are written to.
    pub(crate) fn theme_file(self) -> &'static str {
        match self {
            Mobile::Flutter => "app_theme.dart",
            Mobile::ReactNative => "theme.ts",
            Mobile::SwiftUi => "Theme.swift",
        }
    }
}

/// The stack the web is built with.
pub(crate) const WEB: &str = "react-tailwind";
pub(crate) const WEB_LABEL: &str = "React + Tailwind";
const WEB_THEME: &str = "tailwind.css";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct Settings {
    #[serde(default)]
    pub mobile: Mobile,
}

/// A handoff, as `handoff.toml` keeps it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct Handoff {
    pub n: u32,
    /// The version saved for it, to tell what changed by the next one.
    pub version: String,
    pub at: String,
    #[serde(default)]
    pub who: String,
    pub mobile: Mobile,
    /// How many screens it asks to build: those added or changed.
    #[serde(default)]
    pub to_build: usize,
}

impl Handoff {
    pub(crate) fn title(&self) -> String {
        format!("Handoff {}", self.n)
    }
}

/// How a screen stands since the last handoff.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Change {
    Added,
    Changed,
    Unchanged,
}

/// A screen as the spec gives it.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Screen {
    pub id: String,
    pub name: String,
    pub device: &'static str,
    /// "web" or "mobile", from its device.
    pub target: &'static str,
    pub change: Change,
    /// Its file, from the project's root.
    pub file: String,
    /// Its picture, from the handoff's folder, when it's to be built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shot: Option<String>,
    /// How it's laid out, as Design reads it, when it's to be built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<serde_json::Value>,
}

/// What's read from each screen to hand it off: its layout and its picture.
pub(crate) struct Read {
    pub id: String,
    pub tree: Option<serde_json::Value>,
    pub png: Option<Vec<u8>>,
}

pub(crate) fn dir(root: &Path) -> PathBuf {
    super::model::folder(root).join(DIR)
}

fn folder_of(root: &Path, n: u32) -> PathBuf {
    dir(root).join(n.to_string())
}

pub(crate) fn tokens_dir(root: &Path) -> PathBuf {
    dir(root).join("tokens")
}

pub(crate) fn settings(root: &Path) -> Settings {
    std::fs::read_to_string(dir(root).join(SETTINGS))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn save_settings(root: &Path, s: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(dir(root)).map_err(|e| e.to_string())?;
    let text = toml::to_string(s).map_err(|e| e.to_string())?;
    std::fs::write(dir(root).join(SETTINGS), text).map_err(|e| e.to_string())
}

/// Every handoff, newest first.
pub(crate) fn list(root: &Path) -> Vec<Handoff> {
    let mut out: Vec<Handoff> = std::fs::read_dir(dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join(FACTS)).ok())
        .filter_map(|t| toml::from_str(&t).ok())
        .collect();
    out.sort_by_key(|h| std::cmp::Reverse(h.n));
    out
}

/// Write the tokens for the web and for `mobile`, giving the files' names.
pub(crate) fn write_tokens(root: &Path, mobile: Mobile) -> Result<Vec<String>, String> {
    let css = std::fs::read_to_string(super::model::folder(root).join("tokens.css"))
        .map_err(|e| format!("Couldn't read tokens.css: {e}"))?;
    let all = tokens::read(&css);
    let dir = tokens_dir(root);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let files = [
        ("tokens.json", tokens::dtcg(&all)),
        (WEB_THEME, tokens::tailwind(&all)),
        (
            mobile.theme_file(),
            match mobile {
                Mobile::Flutter => tokens::flutter(&all),
                Mobile::ReactNative => tokens::react_native(&all),
                Mobile::SwiftUi => tokens::swiftui(&all),
            },
        ),
    ];
    for (name, text) in &files {
        std::fs::write(dir.join(name), text).map_err(|e| e.to_string())?;
    }
    Ok(files.iter().map(|(n, _)| n.to_string()).collect())
}

fn target(d: Device) -> &'static str {
    match d {
        Device::Desktop => "web",
        _ => "mobile",
    }
}

/// Write handoff `h`: its spec, the pictures of the screens to build, its facts and the
/// tokens. `since` is the handoff before, and `changes` what changed since its version
/// (None for the first). Gives the handoff's folder.
pub(crate) fn write(
    root: &Path,
    h: &Handoff,
    since: Option<&Handoff>,
    changes: Option<&super::versions::Changes>,
    canvas: &super::model::Canvas,
    read: Vec<Read>,
) -> Result<PathBuf, String> {
    let folder = folder_of(root, h.n);
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("shots")).map_err(|e| e.to_string())?;
    let mut screens = Vec::new();
    for s in &canvas.screens {
        let change = match changes {
            None => Change::Added,
            Some(c) if c.added.contains(&s.id) => Change::Added,
            Some(c) if c.changed.contains(&s.id) => Change::Changed,
            Some(_) => Change::Unchanged,
        };
        let r = read.iter().find(|r| r.id == s.id);
        let build = change != Change::Unchanged;
        let mut shot = None;
        if let Some(png) = r.and_then(|r| r.png.as_ref()).filter(|_| build) {
            let name = format!("shots/{}.png", s.id);
            std::fs::write(folder.join(&name), png).map_err(|e| e.to_string())?;
            shot = Some(name);
        }
        screens.push(Screen {
            id: s.id.clone(),
            name: s.name.clone(),
            device: s.device.label(),
            target: target(s.device),
            change,
            file: format!(
                "{}/{}/{}.html",
                super::model::FOLDER,
                super::model::SCREENS,
                s.id
            ),
            shot,
            tree: r.and_then(|r| r.tree.clone()).filter(|_| build),
        });
    }
    let building: Vec<&str> = screens
        .iter()
        .filter(|s| s.change != Change::Unchanged)
        .map(|s| s.id.as_str())
        .collect();
    let components: Vec<serde_json::Value> = super::library::load(root)
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "name": c.name,
                "library": super::library::library_of(&c.id),
                "variants": c.variants.iter().map(|v| v.name.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let comments: Vec<serde_json::Value> = super::notes::load(root)
        .into_iter()
        .filter(|c| !c.resolved && building.contains(&c.screen.as_str()))
        .map(|c| {
            serde_json::json!({
                "screen": c.screen, "x": c.x, "y": c.y, "author": c.author, "text": c.text,
            })
        })
        .collect();
    let token_files = write_tokens(root, h.mobile)?;
    let spec = serde_json::json!({
        "handoff": h.n,
        "version": h.version,
        "at": h.at,
        "by": h.who,
        "since": since.map(|s| serde_json::json!({ "handoff": s.n, "version": s.version })),
        "targets": {
            "web": { "stack": WEB, "devices": ["Desktop"] },
            "mobile": { "stack": h.mobile.key(), "devices": ["Phone", "Tablet"] },
        },
        "tokens": token_files
            .iter()
            .map(|f| format!("../tokens/{f}"))
            .collect::<Vec<_>>(),
        "screens": screens,
        "removed": changes.map(|c| c.removed.clone()).unwrap_or_default(),
        "components": components,
        "comments": comments,
    });
    let text = serde_json::to_string_pretty(&spec).map_err(|e| e.to_string())? + "\n";
    std::fs::write(folder.join("spec.json"), text).map_err(|e| e.to_string())?;
    let facts = Handoff {
        to_build: building.len(),
        ..h.clone()
    };
    let text = toml::to_string(&facts).map_err(|e| e.to_string())?;
    std::fs::write(folder.join(FACTS), text).map_err(|e| e.to_string())?;
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handoff_lists_what_to_build_and_writes_the_tokens() {
        let root = std::env::temp_dir().join(format!("runo-spec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = super::super::model::folder(&root);
        std::fs::create_dir_all(d.join("screens")).unwrap();
        std::fs::write(
            d.join("tokens.css"),
            ":root { --primary: #2563eb; --radius: 12px; }",
        )
        .unwrap();
        let canvas = super::super::model::Canvas {
            screens: vec![
                super::super::model::Screen {
                    id: "home".into(),
                    name: "Home".into(),
                    device: Device::Phone,
                    ..Default::default()
                },
                super::super::model::Screen {
                    id: "admin".into(),
                    name: "Admin".into(),
                    device: Device::Desktop,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let changes = super::super::versions::Changes {
            changed: vec!["home".into()],
            ..Default::default()
        };
        let h = Handoff {
            n: 2,
            version: "v2".into(),
            mobile: Mobile::ReactNative,
            ..Default::default()
        };
        let before = Handoff {
            n: 1,
            version: "v1".into(),
            ..Default::default()
        };
        let read = vec![Read {
            id: "home".into(),
            tree: Some(serde_json::json!({ "t": "el" })),
            png: Some(vec![1, 2, 3]),
        }];
        let folder = write(&root, &h, Some(&before), Some(&changes), &canvas, read).unwrap();
        let spec: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("spec.json")).unwrap())
                .unwrap();
        assert_eq!(spec["since"]["version"], "v1");
        assert_eq!(spec["targets"]["mobile"]["stack"], "react-native");
        assert_eq!(spec["screens"][0]["change"], "changed");
        assert_eq!(spec["screens"][0]["target"], "mobile");
        assert_eq!(spec["screens"][0]["shot"], "shots/home.png");
        assert_eq!(spec["screens"][1]["change"], "unchanged");
        assert_eq!(spec["screens"][1]["target"], "web");
        assert!(spec["screens"][1].get("tree").is_none());
        assert!(tokens_dir(&root).join("theme.ts").exists());
        assert!(tokens_dir(&root).join("tailwind.css").exists());
        let listed = list(&root);
        assert_eq!(listed[0].to_build, 1);
        assert_eq!(listed[0].title(), "Handoff 2");
        let _ = std::fs::remove_dir_all(&root);
    }
}
