//! The code map: which component in the app's code each design component is, for the web
//! and for the mobile stack, kept in `design/code-map.toml`. Code written from the design
//! uses the mapped component, with the variant's properties as its props, instead of
//! writing the component out again.
//!
//! ```toml
//! [components.button.web]
//! import = "@/components/ui/button"
//! name = "Button"
//! props = { State = "variant" }      # a variant property under another prop name
//! values = { Pressed = "active" }    # a value written another way
//! ```

use super::library::{self, Component};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FILE: &str = "code-map.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Web,
    Mobile,
}

impl Target {
    pub(crate) fn of(device: super::model::Device) -> Target {
        match device {
            super::model::Device::Desktop => Target::Web,
            _ => Target::Mobile,
        }
    }
}

/// One component in the app's code.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct Link {
    /// Where it's imported from: a module path, a package, or a file.
    #[serde(default)]
    pub import: String,
    pub name: String,
    /// A variant property's prop name, when it isn't the property's own name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, String>,
    /// A value as the code writes it, when it isn't the value in lower case.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<Link>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile: Option<Link>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct CodeMap {
    #[serde(default)]
    pub components: BTreeMap<String, Entry>,
}

pub(crate) fn path(root: &Path) -> PathBuf {
    super::model::folder(root).join(FILE)
}

/// "On Surface" as a prop: "onSurface".
fn camel(s: &str) -> String {
    super::tokens::ident(s)
}

/// "Extra Large" as a value: "extra-large".
fn kebab(s: &str) -> String {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

impl Link {
    pub(crate) fn prop(&self, property: &str) -> String {
        self.props
            .get(property)
            .cloned()
            .unwrap_or_else(|| camel(property))
    }

    /// A value as the code writes it: "extra-large" for the web, `.extraLarge` in Swift.
    pub(crate) fn value(&self, v: &str, swift: bool) -> String {
        match self.values.get(v) {
            Some(own) => own.clone(),
            None if swift => format!(".{}", camel(v)),
            None => kebab(v),
        }
    }

    /// The props an instance of `c` in `variant` is written with: the properties that
    /// differ from the component's first variant.
    pub(crate) fn args(
        &self,
        c: Option<&Component>,
        variant: &str,
        swift: bool,
    ) -> Vec<(String, String)> {
        let axes = c.map(Component::axes).unwrap_or_default();
        if axes.is_empty() {
            return match c.and_then(|c| c.variants.first()) {
                Some(first) if first.name != variant => {
                    vec![(self.prop("variant"), self.value(variant, swift))]
                }
                _ => Vec::new(),
            };
        }
        library::props(variant)
            .into_iter()
            .filter(|(k, v)| {
                axes.iter()
                    .any(|(a, vals)| a == k && vals.first() != Some(v))
            })
            .map(|(k, v)| (self.prop(&k), self.value(&v, swift)))
            .collect()
    }
}

impl CodeMap {
    pub(crate) fn load(root: &Path) -> CodeMap {
        std::fs::read_to_string(path(root))
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, root: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path(root), text).map_err(|e| e.to_string())
    }

    pub(crate) fn get(&self, id: &str, t: Target) -> Option<&Link> {
        let e = self.components.get(id)?;
        match t {
            Target::Web => e.web.as_ref(),
            Target::Mobile => e.mobile.as_ref(),
        }
        .filter(|l| !l.name.trim().is_empty())
    }

    /// Map component `id` to `name` from `import`, keeping its prop and value names; an
    /// empty name takes the mapping away.
    pub(crate) fn set(&mut self, id: &str, t: Target, import: &str, name: &str) {
        let e = self.components.entry(id.to_string()).or_default();
        let slot = match t {
            Target::Web => &mut e.web,
            Target::Mobile => &mut e.mobile,
        };
        if name.trim().is_empty() {
            *slot = None;
        } else {
            let link = slot.get_or_insert_with(Link::default);
            link.import = import.trim().to_string();
            link.name = name.trim().to_string();
        }
        if e.web.is_none() && e.mobile.is_none() {
            self.components.remove(id);
        }
    }

    /// The mapping as the handoff spec gives it.
    pub(crate) fn spec(&self, id: &str) -> Option<serde_json::Value> {
        let e = self.components.get(id)?;
        serde_json::to_value(e).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::designdraw::library::Variant;

    fn button() -> Component {
        Component {
            id: "button".into(),
            name: "Button".into(),
            variants: [
                "Kind=Primary, Size=Medium",
                "Kind=Ghost Link, Size=Medium",
                "Kind=Primary, Size=Large",
            ]
            .iter()
            .map(|n| Variant {
                name: n.to_string(),
                html: "<button>Go</button>".into(),
            })
            .collect(),
        }
    }

    #[test]
    fn variants_become_props_of_the_mapped_component() {
        let mut map = CodeMap::default();
        map.set("button", Target::Web, "@/components/ui/button", "Button");
        let link = map.get("button", Target::Web).unwrap().clone();
        let c = button();
        assert_eq!(
            link.args(Some(&c), "Kind=Primary, Size=Medium", false),
            vec![]
        );
        assert_eq!(
            link.args(Some(&c), "Kind=Ghost Link, Size=Medium", false),
            vec![("kind".to_string(), "ghost-link".to_string())]
        );
        assert_eq!(
            link.args(Some(&c), "Kind=Primary, Size=Large", true),
            vec![("size".to_string(), ".large".to_string())]
        );
        let mut own = link.clone();
        own.props.insert("Kind".into(), "variant".into());
        own.values.insert("Ghost Link".into(), "link".into());
        assert_eq!(
            own.args(Some(&c), "Kind=Ghost Link, Size=Medium", false),
            vec![("variant".to_string(), "link".to_string())]
        );
        assert!(map.get("button", Target::Mobile).is_none());
        // Its own names stay when the import changes, and an empty name unmaps it.
        map.components.get_mut("button").unwrap().web = Some(own);
        map.set("button", Target::Web, "@/ui/button", "Button");
        assert_eq!(
            map.get("button", Target::Web).unwrap().props["Kind"],
            "variant"
        );
        map.set("button", Target::Web, "", "");
        assert!(map.components.is_empty());
    }

    #[test]
    fn the_map_is_kept_as_toml() {
        let mut map = CodeMap::default();
        map.set("card", Target::Mobile, "package:app/card.dart", "AppCard");
        let text = toml::to_string(&map).unwrap();
        assert!(text.contains("[components.card.mobile]"));
        assert_eq!(toml::from_str::<CodeMap>(&text).unwrap(), map);
    }
}
