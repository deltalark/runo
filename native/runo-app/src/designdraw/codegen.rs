//! Writing a screen, or a layer of it, as code for an app: React (TSX that keeps the
//! design's classes, with its components as React components whose props are their
//! variant properties) or SwiftUI (stacks, text and shapes with the layout and styles the
//! page computed, its colours named after the design's tokens). No AI: the same design
//! always gives the same code.

use super::codemap::{CodeMap, Link, Target};
use super::library::{self, Component};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

/// A piece of the page as `tree` gives it: an element or a run of text.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub(crate) enum Node {
    Text {
        text: String,
    },
    El {
        tag: String,
        #[serde(default)]
        attrs: Vec<(String, String)>,
        #[serde(default)]
        kids: Vec<Node>,
        /// The styles the page computed, for SwiftUI.
        #[serde(default)]
        s: HashMap<String, String>,
        #[serde(default)]
        w: f32,
        #[serde(default)]
        h: f32,
        #[serde(default = "yes")]
        shown: bool,
        /// It spans the width inside its parent.
        #[serde(default)]
        fill: bool,
        /// Its size is set on it.
        #[serde(default)]
        sized: bool,
    },
}

fn yes() -> bool {
    true
}

/// What `tree` gives back.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Tree {
    pub node: Node,
    /// Each component's variants as trees, by id and variant name.
    #[serde(default)]
    pub comps: HashMap<String, HashMap<String, Node>>,
}

impl Node {
    fn attr(&self, k: &str) -> Option<&str> {
        match self {
            Node::El { attrs, .. } => attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()),
            Node::Text { .. } => None,
        }
    }

    /// Its text, when that's all it holds.
    fn only_text(&self) -> Option<String> {
        match self {
            Node::Text { text } => Some(text.clone()),
            Node::El { kids, .. } => {
                let mut out = String::new();
                for k in kids {
                    match k {
                        Node::Text { text } => out.push_str(text),
                        _ => return None,
                    }
                }
                Some(out.trim().to_string()).filter(|t| !t.is_empty())
            }
        }
    }
}

/// `name` as a type name: "sign in screen" gives "SignInScreen".
pub(crate) fn type_name(name: &str) -> String {
    let out: String = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect();
    match out.chars().next() {
        Some(c) if c.is_ascii_digit() => format!("Screen{out}"),
        Some(_) => out,
        None => "Screen".into(),
    }
}

/// `name` in camelCase: "on-primary" gives "onPrimary".
fn camel(name: &str) -> String {
    let t = type_name(name);
    let mut c = t.chars();
    c.next()
        .map(|f| f.to_lowercase().chain(c).collect())
        .unwrap_or_default()
}

// ---- React

const VOID: [&str; 8] = ["img", "input", "br", "hr", "meta", "link", "source", "wbr"];

/// An attribute as JSX names it.
fn jsx_attr(k: &str, svg: bool) -> String {
    match k {
        "class" => "className".into(),
        "for" => "htmlFor".into(),
        "tabindex" => "tabIndex".into(),
        "readonly" => "readOnly".into(),
        "maxlength" => "maxLength".into(),
        "autocomplete" => "autoComplete".into(),
        "contenteditable" => "contentEditable".into(),
        "colspan" => "colSpan".into(),
        "viewbox" => "viewBox".into(),
        k if k.starts_with("data-") || k.starts_with("aria-") => k.into(),
        k if svg && k.contains('-') => camel(k),
        k => k.into(),
    }
}

/// A style attribute as a JSX style object.
fn jsx_style(css: &str) -> String {
    let parts: Vec<String> = css
        .split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let (k, v) = (k.trim(), v.trim());
            (!k.is_empty()).then(|| {
                let key = if k.starts_with("--") {
                    format!("{k:?}")
                } else {
                    camel(k)
                };
                format!("{key}: {v:?}")
            })
        })
        .collect();
    format!("{{{{ {} }}}}", parts.join(", "))
}

fn jsx_text(t: &str) -> String {
    if t.contains(['{', '}', '<', '>']) {
        format!("{{{t:?}}}")
    } else {
        t.to_string()
    }
}

struct React<'a> {
    lib: &'a [Component],
    /// The components used, by id.
    used: BTreeMap<String, String>,
    map: &'a CodeMap,
    /// The app's own components used, by where they're imported from.
    imports: BTreeMap<String, BTreeSet<String>>,
}

impl React<'_> {
    /// The props an instance is written with, from its variant's properties.
    fn props(&self, id: &str, variant: &str) -> String {
        let c = self.lib.iter().find(|c| c.id == id);
        let axes = c.map(Component::axes).unwrap_or_default();
        if axes.is_empty() {
            return match c.and_then(|c| c.variants.first()) {
                Some(first) if first.name != variant => format!(" variant={variant:?}"),
                _ => String::new(),
            };
        }
        library::props(variant)
            .into_iter()
            .filter(|(k, v)| {
                axes.iter()
                    .any(|(a, vals)| a == k && vals.first() != Some(v))
            })
            .map(|(k, v)| format!(" {}={v:?}", camel(&k)))
            .collect()
    }

    fn jsx(&mut self, n: &Node, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        let Node::El {
            tag, attrs, kids, ..
        } = n
        else {
            if let Some(t) = n.only_text() {
                let _ = writeln!(out, "{pad}{}", jsx_text(t.trim()));
            }
            return;
        };
        if let Some(id) = n.attr("data-component") {
            if let Some(link) = self.map.get(id, Target::Web) {
                self.mapped(n, id, link, &pad, out);
                return;
            }
            let name = type_name(id.rsplit('/').next().unwrap_or(id));
            self.used.insert(id.to_string(), name.clone());
            let props = self.props(id, n.attr("data-variant").unwrap_or_default());
            let _ = match n.only_text() {
                Some(t) => writeln!(out, "{pad}<{name}{props}>{}</{name}>", jsx_text(&t)),
                None => writeln!(out, "{pad}<{name}{props} />"),
            };
            return;
        }
        let svg = tag == "svg" || tag == "path";
        let mut open = format!("{pad}<{tag}");
        for (k, v) in attrs {
            if k == "data-library" || k == "data-variant" || k == "data-rid" {
                continue;
            }
            let k2 = jsx_attr(k, svg);
            if k == "style" {
                let _ = write!(open, " style={}", jsx_style(v));
            } else if v.is_empty() && !k.starts_with("data-") {
                let _ = write!(open, " {k2}");
            } else {
                let _ = write!(open, " {k2}={v:?}");
            }
        }
        if VOID.contains(&tag.as_str()) || kids.is_empty() && svg {
            let _ = writeln!(out, "{open} />");
        } else if let Some(t) = n.only_text() {
            let _ = writeln!(out, "{open}>{}</{tag}>", jsx_text(&t));
        } else if kids.is_empty() {
            let _ = writeln!(out, "{open}></{tag}>");
        } else {
            let _ = writeln!(out, "{open}>");
            for k in kids {
                self.jsx(k, depth + 1, out);
            }
            let _ = writeln!(out, "{pad}</{tag}>");
        }
    }

    /// An instance of a component the app has: that component, with the variant's
    /// properties as its props.
    fn mapped(&mut self, n: &Node, id: &str, link: &Link, pad: &str, out: &mut String) {
        let c = self.lib.iter().find(|c| c.id == id);
        let props: String = link
            .args(c, n.attr("data-variant").unwrap_or_default(), false)
            .into_iter()
            .map(|(k, v)| format!(" {k}={v:?}"))
            .collect();
        let name = &link.name;
        if !link.import.is_empty() {
            // `Card.Header` is imported as `Card`.
            let root = name.split('.').next().unwrap_or(name).to_string();
            self.imports
                .entry(link.import.clone())
                .or_default()
                .insert(root);
        }
        let _ = match n.only_text() {
            Some(t) => writeln!(out, "{pad}<{name}{props}>{}</{name}>", jsx_text(&t)),
            None => writeln!(out, "{pad}<{name}{props} />"),
        };
    }

    /// A component as a React component: its variant picked by its props, its text by
    /// what an instance holds.
    fn component(&mut self, c: &Component, trees: &HashMap<String, Node>, name: &str) -> String {
        let axes = c.axes();
        let mut out = String::new();
        let (sig, key) = if axes.is_empty() {
            let first = c
                .variants
                .first()
                .map(|v| v.name.clone())
                .unwrap_or_default();
            (
                format!("{{ variant = {first:?}, children }}: {{ variant?: string; children?: React.ReactNode }}"),
                "variant".to_string(),
            )
        } else {
            let args: Vec<String> = axes
                .iter()
                .map(|(k, vals)| format!("{} = {:?}", camel(k), vals[0]))
                .collect();
            let types: Vec<String> = axes
                .iter()
                .map(|(k, vals)| {
                    let opts: Vec<String> = vals.iter().map(|v| format!("{v:?}")).collect();
                    format!("{}?: {}", camel(k), opts.join(" | "))
                })
                .collect();
            let key: Vec<String> = axes
                .iter()
                .map(|(k, _)| format!("{k}=${{{}}}", camel(k)))
                .collect();
            (
                format!(
                    "{{ {}, children }}: {{ {}; children?: React.ReactNode }}",
                    args.join(", "),
                    types.join("; ")
                ),
                format!("`{}`", key.join(", ")),
            )
        };
        let _ = writeln!(out, "export function {name}({sig}) {{");
        let _ = writeln!(out, "  switch ({key}) {{");
        for (i, v) in c.variants.iter().enumerate() {
            let Some(tree) = trees.get(&v.name) else {
                continue;
            };
            let label = if i + 1 == c.variants.len() {
                "    default:".to_string()
            } else {
                format!("    case {:?}:", v.name)
            };
            let _ = writeln!(out, "{label}");
            let mut body = String::new();
            // The variant's own text is what's shown unless the instance says otherwise.
            match tree.only_text() {
                Some(t) => {
                    let mut stripped = tree.clone();
                    if let Node::El { kids, .. } = &mut stripped {
                        kids.clear();
                    }
                    self.jsx(&strip_component(&stripped), 4, &mut body);
                    let line = body
                        .trim_end()
                        .trim_end_matches("/>")
                        .trim_end()
                        .to_string();
                    let tag = match tree {
                        Node::El { tag, .. } => tag.as_str(),
                        _ => "span",
                    };
                    let open = line.trim_end_matches(&format!("></{tag}>")).to_string();
                    body = format!("{open}>{{children ?? {:?}}}</{tag}>\n", t);
                }
                None => self.jsx(&strip_component(tree), 4, &mut body),
            }
            let _ = write!(out, "      return (\n{body}      );\n");
        }
        let _ = writeln!(out, "  }}\n}}");
        out
    }
}

/// A variant's tree without its own instance marking, so it's written as markup.
fn strip_component(n: &Node) -> Node {
    let mut n = n.clone();
    if let Node::El { attrs, .. } = &mut n {
        attrs.retain(|(k, _)| k != "data-component" && k != "data-variant" && k != "data-library");
    }
    n
}

/// The layer as a React component called `name`, with the components it uses after it.
/// Components the app has, by `map`, are used as they are.
pub(crate) fn react(tree: &Tree, name: &str, lib: &[Component], map: &CodeMap) -> String {
    let mut r = React {
        lib,
        used: BTreeMap::new(),
        map,
        imports: BTreeMap::new(),
    };
    let mut body = String::new();
    // A whole screen is its body's children, in a fragment.
    match &tree.node {
        Node::El { tag, kids, .. } if tag == "body" => {
            body.push_str("    <>\n");
            for k in kids {
                r.jsx(k, 3, &mut body);
            }
            body.push_str("    </>\n");
        }
        n => r.jsx(n, 2, &mut body),
    }
    let mut out = String::new();
    for (from, names) in &r.imports {
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let _ = writeln!(out, "import {{ {} }} from {from:?};", names.join(", "));
    }
    if !r.imports.is_empty() {
        out.push('\n');
    }
    out.push_str(
        "// The design's styles: copy design/tokens.css and design/components.css next to this file.\n\
         import \"./tokens.css\";\nimport \"./components.css\";\n\n",
    );
    let _ = write!(
        out,
        "export default function {name}() {{\n  return (\n{body}  );\n}}\n"
    );
    // Components, and the ones they use in turn.
    let mut done: Vec<String> = Vec::new();
    while let Some((id, cname)) = r
        .used
        .iter()
        .find(|(id, _)| !done.contains(id))
        .map(|(a, b)| (a.clone(), b.clone()))
    {
        done.push(id.clone());
        let (Some(c), Some(trees)) = (lib.iter().find(|c| c.id == id), tree.comps.get(&id)) else {
            continue;
        };
        let code = r.component(c, trees, &cname);
        let _ = write!(out, "\n{code}");
    }
    out
}

// ---- SwiftUI

/// A colour the page computed, "rgb(37, 99, 235)" or "rgba(…, 0.5)": its channels and
/// how see-through it is.
fn rgba(v: &str) -> Option<([u8; 3], f32)> {
    let inner = v
        .trim()
        .strip_prefix("rgba(")
        .or_else(|| v.trim().strip_prefix("rgb("))?;
    let nums: Vec<f32> = inner
        .trim_end_matches(')')
        .split([',', ' ', '/'])
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    let a = nums.get(3).copied().unwrap_or(1.);
    (nums.len() >= 3 && a > 0.).then(|| ([nums[0] as u8, nums[1] as u8, nums[2] as u8], a))
}

/// The design's colour tokens: their Swift names and colours.
pub(crate) fn token_colors(tokens_css: &str) -> Vec<(String, [u8; 3])> {
    // Only the light theme, the first :root block.
    let block = tokens_css
        .split_once(":root")
        .and_then(|(_, rest)| rest.split_once('{'))
        .and_then(|(_, rest)| rest.split_once('}'))
        .map(|(b, _)| b)
        .unwrap_or("");
    let mut raw: Vec<(String, String)> = Vec::new();
    for d in block.split(';') {
        if let Some((k, v)) = d.split_once(':') {
            if let Some(k) = k.trim().strip_prefix("--") {
                raw.push((k.to_string(), v.trim().to_string()));
            }
        }
    }
    let hex = |v: &str| -> Option<[u8; 3]> {
        let h = v.strip_prefix('#')?;
        let h = if h.len() == 3 {
            h.chars().flat_map(|c| [c, c]).collect::<String>()
        } else {
            h.to_string()
        };
        let n = u32::from_str_radix(h.get(..6)?, 16).ok()?;
        Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
    };
    raw.iter()
        .filter_map(|(k, v)| {
            let c = hex(v).or_else(|| {
                // One step through another token: "var(--brand)".
                let other = v.strip_prefix("var(--")?.trim_end_matches(')');
                raw.iter()
                    .find(|(k2, _)| k2 == other)
                    .and_then(|(_, v2)| hex(v2))
            })?;
            Some((camel(k), c))
        })
        .collect()
}

struct Swift<'a> {
    tokens: &'a [(String, [u8; 3])],
    used: BTreeMap<String, [u8; 3]>,
    /// The app's own components, when the app is built with SwiftUI.
    map: Option<&'a CodeMap>,
    lib: &'a [Component],
}

/// What a colour is for, to pick the token that names it best when several match.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Fill,
    Ink,
}

fn num(v: f32) -> String {
    let r = (v * 10.).round() / 10.;
    if r.fract() == 0. {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn px(v: Option<&String>) -> f32 {
    v.and_then(|s| s.trim().trim_end_matches("px").parse().ok())
        .unwrap_or(0.)
}

type Style = HashMap<String, String>;

fn get<'a>(s: &'a Style, k: &str) -> &'a str {
    s.get(k).map(String::as_str).unwrap_or("")
}

impl Swift<'_> {
    fn color(&mut self, v: &str, role: Role) -> Option<String> {
        let (c, a) = rgba(v)?;
        // Of the tokens with this colour, the one whose name fits what it colours.
        let fits = |name: &str| {
            let n = name.to_lowercase();
            let fill = ["surface", "background", "bg", "canvas", "card"];
            let ink = ["text", "on", "ink", "fg", "foreground", "muted"];
            let words: &[&str] = if role == Role::Fill { &fill } else { &ink };
            words.iter().any(|w| n.starts_with(w))
        };
        let same: Vec<&(String, [u8; 3])> = self.tokens.iter().filter(|(_, t)| *t == c).collect();
        let pick = same
            .iter()
            .find(|(n, _)| fits(n))
            .or_else(|| same.first())
            .copied();
        let base = match pick {
            Some((name, t)) => {
                self.used.insert(name.clone(), *t);
                format!("Theme.{name}")
            }
            None => rgb_color(c),
        };
        Some(if a < 0.999 {
            format!("{base}.opacity({})", num(a))
        } else {
            base
        })
    }

    /// How text set in `s` looks: its font, colour and alignment.
    fn text_mods(&mut self, s: &Style, out: &mut Vec<String>) {
        let size = px(s.get("font-size"));
        let weight = match get(s, "font-weight").parse::<u32>().ok() {
            Some(w) if w >= 800 => ".heavy",
            Some(w) if w >= 700 => ".bold",
            Some(w) if w >= 600 => ".semibold",
            Some(w) if w >= 500 => ".medium",
            Some(w) if w <= 300 => ".light",
            _ => ".regular",
        };
        out.push(format!(
            ".font(.system(size: {}, weight: {weight}))",
            num(size.max(1.))
        ));
        if let Some(c) = s.get("color").and_then(|c| self.color(c, Role::Ink)) {
            out.push(format!(".foregroundStyle({c})"));
        }
        match get(s, "text-align") {
            "center" => out.push(".multilineTextAlignment(.center)".into()),
            "right" | "end" => out.push(".multilineTextAlignment(.trailing)".into()),
            _ => {}
        }
    }

    /// The modifiers for an element's box: padding, size, fill, border, shadow.
    fn box_mods(&mut self, n: &Node, out: &mut Vec<String>) {
        let Node::El {
            tag,
            s,
            w,
            h,
            fill,
            sized,
            kids,
            ..
        } = n
        else {
            return;
        };
        let pads: Vec<f32> = ["top", "right", "bottom", "left"]
            .iter()
            .map(|k| px(s.get(&format!("padding-{k}"))))
            .collect();
        if pads.iter().any(|p| *p > 0.) {
            if pads.iter().all(|p| *p == pads[0]) {
                out.push(format!(".padding({})", num(pads[0])));
            } else {
                out.push(format!(
                    ".padding(EdgeInsets(top: {}, leading: {}, bottom: {}, trailing: {}))",
                    num(pads[0]),
                    num(pads[3]),
                    num(pads[2]),
                    num(pads[1])
                ));
            }
        }
        // Boxes with nothing in their flow keep their size; controls their height.
        let empty = kids.iter().all(is_floating) && tag != "img";
        let control = matches!(tag.as_str(), "button" | "input" | "select" | "textarea");
        let row = get(s, "display").contains("flex") && get(s, "flex-direction").starts_with("row");
        let along = if row {
            get(s, "justify-content")
        } else {
            get(s, "align-items")
        };
        let align = match along {
            "center" => ".center",
            "flex-end" | "end" | "right" => ".trailing",
            _ => ".leading",
        };
        let width = if *fill {
            Some("maxWidth: .infinity, ".to_string())
        } else if *sized || empty {
            Some(format!("width: {}, ", num(*w)))
        } else {
            None
        };
        let height = if *sized || empty {
            Some(format!("height: {}, ", num(*h)))
        } else if control {
            Some(format!("minHeight: {}, ", num(*h)))
        } else {
            None
        };
        if width.is_some() || height.is_some() {
            let args = format!(
                "{}{}alignment: {align}",
                width.unwrap_or_default(),
                height.unwrap_or_default()
            );
            out.push(format!(".frame({args})"));
        }
        let radius = px(s.get("border-top-left-radius"));
        let shape = if radius > 0. {
            format!(
                "RoundedRectangle(cornerRadius: {})",
                num(radius.min(w.min(*h) / 2.).max(0.))
            )
        } else {
            "Rectangle()".into()
        };
        let image = get(s, "background-image");
        let gradient: Vec<String> = if image.contains("gradient") {
            image
                .split("rgb")
                .skip(1)
                .filter_map(|c| {
                    let end = c.find(')')?;
                    self.color(&format!("rgb{}", &c[..=end]), Role::Fill)
                })
                .collect()
        } else {
            Vec::new()
        };
        let bg = get(s, "background-color").to_string();
        if gradient.len() >= 2 {
            out.push(format!(
                ".background(LinearGradient(colors: [{}], startPoint: .topLeading, endPoint: .bottomTrailing), in: {shape})",
                gradient.join(", ")
            ));
        } else if let Some(c) = self.color(&bg, Role::Fill) {
            out.push(format!(".background({c}, in: {shape})"));
        }
        let bw = px(s.get("border-top-width"));
        let line = get(s, "border-top-color").to_string();
        if bw > 0. && get(s, "border-top-style") != "none" && line != bg {
            if let Some(c) = self.color(&line, Role::Ink) {
                out.push(format!(
                    ".overlay({shape}.stroke({c}, lineWidth: {}))",
                    num(bw)
                ));
            }
        }
        if !matches!(get(s, "box-shadow"), "" | "none") {
            out.push(".shadow(color: .black.opacity(0.08), radius: 8, y: 2)".into());
        }
        let opacity: f32 = get(s, "opacity").parse().unwrap_or(1.);
        if opacity < 0.999 {
            out.push(format!(".opacity({})", num(opacity)));
        }
    }

    fn lines(out: &mut String, depth: usize, head: &str, mods: &[String]) {
        let pad = "    ".repeat(depth);
        let _ = writeln!(out, "{pad}{head}");
        for m in mods {
            let _ = writeln!(out, "{pad}    {m}");
        }
    }

    /// One piece of the page as SwiftUI. `parent` styles bare text.
    fn write(&mut self, n: &Node, parent: &Style, depth: usize, out: &mut String) {
        let Node::El {
            tag,
            s,
            kids,
            shown,
            ..
        } = n
        else {
            if let Some(t) = n.only_text() {
                let mut mods = Vec::new();
                self.text_mods(parent, &mut mods);
                Self::lines(out, depth, &format!("Text({:?})", t.trim()), &mods);
            }
            return;
        };
        if !shown || get(s, "visibility") == "hidden" {
            return;
        }
        let pad = "    ".repeat(depth);
        let link = n
            .attr("data-component")
            .and_then(|id| Some((id, self.map?.get(id, Target::Mobile)?)));
        if let Some((id, link)) = link {
            let c = self.lib.iter().find(|c| c.id == id);
            let args: Vec<String> = link
                .args(c, n.attr("data-variant").unwrap_or_default(), true)
                .into_iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            let name = &link.name;
            let _ = match n.only_text() {
                Some(t) if args.is_empty() => {
                    writeln!(out, "{pad}{name} {{ Text({:?}) }}", t.trim())
                }
                Some(t) => writeln!(
                    out,
                    "{pad}{name}({}) {{ Text({:?}) }}",
                    args.join(", "),
                    t.trim()
                ),
                None => writeln!(out, "{pad}{name}({})", args.join(", ")),
            };
            return;
        }
        let action = tag == "button" || tag == "a" || n.attr("data-goto").is_some();
        let classes = n.attr("class").unwrap_or_default();
        let mut mods = Vec::new();
        // Leaves: text, pictures, fields, icons and drawings.
        let leaf = if let Some(t) = n.only_text().filter(|_| !action && tag != "input") {
            let t = if get(s, "text-transform") == "uppercase" {
                t.to_uppercase()
            } else {
                t
            };
            self.text_mods(s, &mut mods);
            Some(format!("Text({t:?})"))
        } else if tag == "img" {
            let src = n.attr("src").unwrap_or_default();
            let file = src.rsplit('/').next().unwrap_or(src);
            let stem = file.split('.').next().unwrap_or(file);
            mods.push(".resizable()".into());
            mods.push(if get(s, "object-fit") == "contain" {
                ".scaledToFit()".into()
            } else {
                ".scaledToFill()".into()
            });
            Some(format!("Image({stem:?})"))
        } else if matches!(tag.as_str(), "input" | "textarea") {
            let hint = n.attr("placeholder").unwrap_or_default();
            self.text_mods(s, &mut mods);
            Some(format!("TextField({hint:?}, text: .constant(\"\"))"))
        } else if let Some(icon) = classes
            .split_whitespace()
            .find_map(|c| c.strip_prefix("i-"))
        {
            if let Some(c) = s
                .get("background-color")
                .and_then(|c| self.color(c, Role::Ink))
            {
                mods.push(format!(".foregroundStyle({c})"));
            }
            // The design's icons are Lucide's; SF Symbols mostly share their names.
            Some(format!("Image(systemName: {icon:?})"))
        } else if tag == "svg" {
            let name = camel(n.attr("data-name").unwrap_or("drawing"));
            Some(format!(
                "Image({name:?}) // A drawing: export it as an SVG asset"
            ))
        } else {
            None
        };
        if let Some(head) = leaf {
            let mut boxed = Vec::new();
            self.box_mods(n, &mut boxed);
            // An icon is drawn by its shape, not filled in.
            boxed.retain(|m| {
                !(m.starts_with(".background(") && head.starts_with("Image(systemName"))
            });
            mods.extend(boxed);
            if tag == "img" {
                mods.push(".clipped()".into());
            }
            Self::lines(out, depth, &head, &mods);
            return;
        }
        // A box: what flows in it, in a stack the way it's laid out, and what floats over.
        let display = get(s, "display");
        let row = display.contains("flex") && get(s, "flex-direction").starts_with("row");
        let grid_cols = display
            .contains("grid")
            .then(|| get(s, "grid-template-columns").split_whitespace().count())
            .filter(|c| *c > 1);
        let gap = px(s.get(if row { "column-gap" } else { "row-gap" }));
        let between = get(s, "justify-content") == "space-between";
        let flow: Vec<&Node> = kids.iter().filter(|k| !is_floating(k)).collect();
        let floating: Vec<&Node> = kids.iter().filter(|k| is_floating(k)).collect();
        let mut d = depth;
        if action {
            let _ = writeln!(out, "{pad}Button(action: {{}}) {{");
            d += 1;
        }
        if !floating.is_empty() {
            let _ = writeln!(out, "{}ZStack(alignment: .topLeading) {{", "    ".repeat(d));
            d += 1;
        }
        let stack = if let Some(cols) = grid_cols {
            format!(
                "LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: {g}), count: {cols}), spacing: {g})",
                g = num(px(s.get("row-gap")))
            )
        } else if row {
            let a = match get(s, "align-items") {
                "flex-start" | "start" => ".top",
                "flex-end" | "end" => ".bottom",
                "baseline" => ".firstTextBaseline",
                _ => ".center",
            };
            format!("HStack(alignment: {a}, spacing: {})", num(gap))
        } else {
            let a = match get(s, "align-items") {
                "center" => ".center",
                "flex-end" | "end" => ".trailing",
                _ if display.contains("grid") => ".center",
                _ => ".leading",
            };
            format!("VStack(alignment: {a}, spacing: {})", num(gap))
        };
        let inner_pad = "    ".repeat(d);
        if flow.is_empty() {
            let _ = writeln!(out, "{inner_pad}Color.clear");
        } else {
            let _ = writeln!(out, "{inner_pad}{stack} {{");
            for (i, k) in flow.iter().enumerate() {
                if between && row && i > 0 {
                    let _ = writeln!(out, "{inner_pad}    Spacer(minLength: 0)");
                }
                self.write(k, s, d + 1, out);
            }
            let _ = writeln!(out, "{inner_pad}}}");
        }
        if !floating.is_empty() {
            for f in floating {
                let Node::El { s: fs, .. } = f else {
                    continue;
                };
                self.write(f, s, d, out);
                let _ = writeln!(
                    out,
                    "{inner_pad}    .offset(x: {}, y: {})",
                    num(px(fs.get("left"))),
                    num(px(fs.get("top")))
                );
            }
            d -= 1;
            let _ = writeln!(out, "{}}}", "    ".repeat(d));
        }
        self.box_mods(n, &mut mods);
        for m in &mods {
            let _ = writeln!(out, "{}    {m}", "    ".repeat(d));
        }
        if action {
            let _ = writeln!(out, "{pad}}}");
            let _ = writeln!(out, "{pad}    .buttonStyle(.plain)");
        }
    }
}

/// A colour as SwiftUI writes it.
fn rgb_color(c: [u8; 3]) -> String {
    let unit = |v: u8| (v as f32 / 255. * 1000.).round() / 1000.;
    format!(
        "Color(red: {}, green: {}, blue: {})",
        unit(c[0]),
        unit(c[1]),
        unit(c[2])
    )
}

fn is_floating(n: &Node) -> bool {
    matches!(n, Node::El { s, .. } if s.get("position").is_some_and(|p| p == "absolute" || p == "fixed"))
}

/// The layer as a SwiftUI view called `name`, and the theme its colours come from. With
/// `map`, components the app has are used as they are.
pub(crate) fn swiftui(
    tree: &Tree,
    name: &str,
    tokens_css: &str,
    lib: &[Component],
    map: Option<&CodeMap>,
) -> (String, String) {
    let tokens = token_colors(tokens_css);
    let mut sw = Swift {
        tokens: &tokens,
        used: BTreeMap::new(),
        map,
        lib,
    };
    let mut body = String::new();
    let screen = matches!(&tree.node, Node::El { tag, .. } if tag == "body");
    if screen {
        body.push_str("        ScrollView {\n");
        sw.write(&tree.node, &Style::new(), 3, &mut body);
        body.push_str("        }\n");
    } else {
        sw.write(&tree.node, &Style::new(), 2, &mut body);
    }
    let view = format!(
        "import SwiftUI\n\nstruct {name}: View {{\n    var body: some View {{\n{body}    }}\n}}\n\n#Preview {{\n    {name}()\n}}\n"
    );
    let mut theme = String::from(
        "import SwiftUI\n\n/// The design's colours, from design/tokens.css.\nenum Theme {\n",
    );
    for (n, c) in &sw.used {
        let _ = writeln!(theme, "    static let {n} = {}", rgb_color(*c));
    }
    theme.push_str("}\n");
    (view, theme)
}

#[cfg(test)]
mod tests {
    use super::{react, swiftui, token_colors, type_name, CodeMap, Node, Tree};
    use crate::designdraw::library::{Component, Variant};
    use std::collections::HashMap;

    fn tree(json: serde_json::Value) -> Tree {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn names_become_type_names() {
        assert_eq!(type_name("sign in"), "SignIn");
        assert_eq!(type_name("screen-2"), "Screen2");
        assert_eq!(type_name("2fa"), "Screen2fa");
    }

    #[test]
    fn react_keeps_the_markup_and_makes_components() {
        let t = tree(serde_json::json!({
            "node": {"t": "el", "tag": "main", "attrs": [["class", "screen stack"], ["style", "gap: 8px; --x: 1"]], "kids": [
                {"t": "el", "tag": "h1", "attrs": [], "kids": [{"t": "text", "text": "Hi {you}"}]},
                {"t": "el", "tag": "img", "attrs": [["src", "../assets/a.jpg"], ["alt", ""]], "kids": []},
                {"t": "el", "tag": "button", "attrs": [["class", "button"], ["data-component", "button"], ["data-variant", "State=Pressed"]], "kids": [{"t": "text", "text": "Buy"}]}
            ]},
            "comps": {"button": {
                "State=Default": {"t": "el", "tag": "button", "attrs": [["class", "button"], ["data-component", "button"]], "kids": [{"t": "text", "text": "Go"}]},
                "State=Pressed": {"t": "el", "tag": "button", "attrs": [["class", "button"], ["style", "opacity: 0.6"]], "kids": [{"t": "text", "text": "Go"}]}
            }}
        }));
        let lib = vec![Component {
            id: "button".into(),
            name: "Button".into(),
            variants: vec![
                Variant {
                    name: "State=Default".into(),
                    html: String::new(),
                },
                Variant {
                    name: "State=Pressed".into(),
                    html: String::new(),
                },
            ],
        }];
        let code = react(&t, "Home", &lib, &CodeMap::default());
        assert!(code.contains("export default function Home()"), "{code}");
        assert!(
            code.contains(r#"<main className="screen stack" style={{ gap: "8px", "--x": "1" }}>"#),
            "{code}"
        );
        assert!(code.contains(r#"<h1>{"Hi {you}"}</h1>"#), "{code}");
        assert!(
            code.contains(r#"<img src="../assets/a.jpg" alt />"#),
            "{code}"
        );
        assert!(
            code.contains(r#"<Button state="Pressed">Buy</Button>"#),
            "{code}"
        );
        assert!(
            code.contains(r#"export function Button({ state = "Default", children }"#),
            "{code}"
        );
        assert!(code.contains(r#"case "State=Default":"#), "{code}");
        assert!(
            code.contains(r#"<button className="button">{children ?? "Go"}</button>"#),
            "{code}"
        );

        // With the app's own Button mapped, that's used, imported, and not written out.
        let mut map = CodeMap::default();
        map.set(
            "button",
            crate::designdraw::codemap::Target::Web,
            "@/components/ui/button",
            "Button",
        );
        let code = react(&t, "Home", &lib, &map);
        assert!(
            code.starts_with("import { Button } from \"@/components/ui/button\";\n"),
            "{code}"
        );
        assert!(
            code.contains(r#"<Button state="pressed">Buy</Button>"#),
            "{code}"
        );
        assert!(!code.contains("export function Button"), "{code}");
        let (view, _) = swiftui(
            &t,
            "HomeView",
            "",
            &lib,
            Some(&{
                let mut m = CodeMap::default();
                m.set(
                    "button",
                    crate::designdraw::codemap::Target::Mobile,
                    "",
                    "PrimaryButton",
                );
                m
            }),
        );
        assert!(
            view.contains(r#"PrimaryButton(state: .pressed) { Text("Buy") }"#),
            "{view}"
        );
    }

    #[test]
    fn swiftui_lays_out_stacks_with_the_theme() {
        let s = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let t = Tree {
            node: Node::El {
                tag: "div".into(),
                attrs: vec![],
                s: s(&[
                    ("display", "flex"),
                    ("flex-direction", "row"),
                    ("column-gap", "12px"),
                    ("justify-content", "space-between"),
                    ("background-color", "rgb(37, 99, 235)"),
                    ("border-top-left-radius", "16px"),
                    ("padding-top", "8px"),
                    ("padding-right", "8px"),
                    ("padding-bottom", "8px"),
                    ("padding-left", "8px"),
                ]),
                w: 300.,
                h: 60.,
                shown: true,
                fill: true,
                sized: false,
                kids: vec![
                    Node::El {
                        tag: "strong".into(),
                        attrs: vec![],
                        s: s(&[
                            ("font-size", "16px"),
                            ("font-weight", "600"),
                            ("color", "rgb(255, 255, 255)"),
                        ]),
                        w: 40.,
                        h: 20.,
                        shown: true,
                        fill: false,
                        sized: false,
                        kids: vec![Node::Text {
                            text: "Title".into(),
                        }],
                    },
                    Node::El {
                        tag: "span".into(),
                        attrs: vec![],
                        s: s(&[("font-size", "12px"), ("color", "rgba(255, 255, 255, 0.7)")]),
                        w: 40.,
                        h: 20.,
                        shown: true,
                        fill: false,
                        sized: false,
                        kids: vec![Node::Text {
                            text: "Note".into(),
                        }],
                    },
                ],
            },
            comps: HashMap::new(),
        };
        let (view, theme) = swiftui(
            &t,
            "Card",
            ":root {\n  --primary: #2563eb;\n  --on-primary: #fff;\n}\n",
            &[],
            None,
        );
        assert!(view.contains("struct Card: View"), "{view}");
        assert!(
            view.contains("HStack(alignment: .center, spacing: 12) {"),
            "{view}"
        );
        assert!(view.contains("Spacer(minLength: 0)"), "{view}");
        assert!(view.contains(r#"Text("Title")"#), "{view}");
        assert!(
            view.contains(".font(.system(size: 16, weight: .semibold))"),
            "{view}"
        );
        assert!(view.contains(".foregroundStyle(Theme.onPrimary)"), "{view}");
        assert!(
            view.contains(".foregroundStyle(Theme.onPrimary.opacity(0.7))"),
            "{view}"
        );
        assert!(view.contains(".padding(8)"), "{view}");
        assert!(
            view.contains(".background(Theme.primary, in: RoundedRectangle(cornerRadius: 16))"),
            "{view}"
        );
        assert!(
            theme.contains("static let primary = Color(red: 0.145, green: 0.388, blue: 0.922)"),
            "{theme}"
        );
    }

    #[test]
    fn tokens_are_read_from_the_light_theme() {
        let t = token_colors(":root { --brand: #7c3aed; --primary: var(--brand); --space: 8px; }\n@media (prefers-color-scheme: dark) { :root { --brand: #000; } }");
        assert_eq!(
            t,
            vec![
                ("brand".to_string(), [0x7c, 0x3a, 0xed]),
                ("primary".to_string(), [0x7c, 0x3a, 0xed])
            ]
        );
    }
}
