//! Designed screens, kept as real HTML in `design/` so what's approved is what gets built.
//!
//! `screens/<id>.html` are the screens, `tokens.css` the theme every screen shares,
//! `components.css` the building blocks, `prototype.js` the links between screens, and
//! `canvas.toml` where each screen sits on the canvas and which device it's for.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The folder in the project that holds the design.
pub(crate) const FOLDER: &str = "design";
pub(crate) const SCREENS: &str = "screens";
const CANVAS: &str = "canvas.toml";
pub(crate) const TOKENS: &str = "tokens.css";
pub(crate) const COMPONENTS: &str = "components.css";
const ICONS: &str = "icons.css";
const PROTOTYPE: &str = "prototype.js";

/// The frame a screen is designed for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Device {
    #[default]
    Phone,
    Tablet,
    Desktop,
}

impl Device {
    pub(crate) const ALL: [Device; 3] = [Device::Phone, Device::Tablet, Device::Desktop];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Device::Phone => "Phone",
            Device::Tablet => "Tablet",
            Device::Desktop => "Desktop",
        }
    }

    /// The CSS size of the viewport.
    pub(crate) fn size(self) -> (f32, f32) {
        match self {
            Device::Phone => (390., 844.),
            Device::Tablet => (834., 1194.),
            Device::Desktop => (1440., 900.),
        }
    }
}

/// A screen's place on the canvas.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Screen {
    /// The file stem in `screens/`.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub device: Device,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    /// The layout grid drawn over it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Grid>,
    /// Lines to line things up against, across or down the screen.
    #[serde(default, rename = "guide", skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<Guide>,
}

/// A layout grid: columns or rows with gutters between and margins at the sides, or a
/// square grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Grid {
    /// "columns", "rows" or "square".
    pub kind: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub gutter: f32,
    #[serde(default)]
    pub margin: f32,
    /// The square grid's step.
    #[serde(default)]
    pub size: f32,
}

impl Grid {
    pub(crate) fn columns() -> Self {
        Self {
            kind: "columns".into(),
            count: 4,
            gutter: 16.,
            margin: 16.,
            size: 8.,
        }
    }

    /// The bands of a columns or rows grid across `span`, as (start, length).
    pub(crate) fn bands(&self, span: f32) -> Vec<(f32, f32)> {
        let n = self.count.max(1) as f32;
        let each = (span - 2. * self.margin - (n - 1.) * self.gutter) / n;
        if each <= 0. {
            return vec![];
        }
        (0..self.count.max(1))
            .map(|i| (self.margin + i as f32 * (each + self.gutter), each))
            .collect()
    }
}

/// A guide: a line at `at` pixels from the left (vertical) or the top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Guide {
    pub vertical: bool,
    pub at: f32,
}

/// Everything about the canvas that isn't in the screens themselves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Canvas {
    #[serde(default, rename = "screen", skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<Screen>,
    /// The platform the prototype moves like: "ios" (the default) or "android".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub platform: String,
}

impl Canvas {
    pub(crate) fn screen(&self, id: &str) -> Option<&Screen> {
        self.screens.iter().find(|s| s.id == id)
    }

    pub(crate) fn screen_mut(&mut self, id: &str) -> Option<&mut Screen> {
        self.screens.iter_mut().find(|s| s.id == id)
    }

    /// A file stem from `name` that no screen uses yet.
    pub(crate) fn new_id(&self, name: &str) -> String {
        let base = slug(name);
        let base = if base.is_empty() {
            "screen".into()
        } else {
            base
        };
        let mut id = base.clone();
        let mut n = 2;
        while self.screen(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    /// Where a new screen goes: to the right of the others, on the top row.
    pub(crate) fn next_spot(&self) -> (f32, f32) {
        let right = self
            .screens
            .iter()
            .map(|s| s.x + s.device.size().0)
            .fold(f32::NEG_INFINITY, f32::max);
        if right.is_finite() {
            (
                right + GAP,
                self.screens.iter().map(|s| s.y).fold(f32::MAX, f32::min),
            )
        } else {
            (0., 0.)
        }
    }

    /// Lay the screens out along the flow: each screen to the right of the first screen
    /// that links to it, branches stacked below, screens nothing links to after.
    pub(crate) fn arrange(&mut self, links: &[(String, String)], heights: &dyn Fn(&str) -> f32) {
        let ids: Vec<String> = self.screens.iter().map(|s| s.id.clone()).collect();
        let linked: std::collections::HashSet<&str> = links.iter().map(|l| l.1.as_str()).collect();
        let mut column: std::collections::HashMap<String, usize> = Default::default();
        let mut order: Vec<String> = Vec::new();
        let roots: Vec<&String> = ids
            .iter()
            .filter(|id| !linked.contains(id.as_str()))
            .chain(ids.iter().filter(|id| linked.contains(id.as_str())))
            .collect();
        for root in roots {
            if column.contains_key(root) {
                continue;
            }
            column.insert(root.clone(), 0);
            order.push(root.clone());
            let mut i = order.len() - 1;
            while i < order.len() {
                let from = order[i].clone();
                let c = column[&from];
                for (a, b) in links {
                    if *a == from && ids.contains(b) && !column.contains_key(b) {
                        column.insert(b.clone(), c + 1);
                        order.push(b.clone());
                    }
                }
                i += 1;
            }
        }
        let columns = column.values().copied().max().unwrap_or(0) + 1;
        let mut widths = vec![0f32; columns];
        for id in &order {
            if let Some(s) = self.screen(id) {
                let c = column[id];
                widths[c] = widths[c].max(s.device.size().0);
            }
        }
        let mut lefts = vec![0f32; columns];
        for c in 1..columns {
            lefts[c] = lefts[c - 1] + widths[c - 1] + GAP * 1.5;
        }
        let mut tops = vec![0f32; columns];
        for id in &order {
            let c = column[id];
            let h = heights(id);
            if let Some(s) = self.screen_mut(id) {
                s.x = lefts[c];
                s.y = tops[c];
                tops[c] += h + GAP * 1.5;
            }
        }
    }
}

/// The space between screens on the canvas.
pub(crate) const GAP: f32 = 80.;

pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

pub(crate) fn folder(root: &Path) -> PathBuf {
    root.join(FOLDER)
}

/// Where pictures put on screens are kept, next to `screens/`.
pub(crate) const ASSETS: &str = "assets";

/// Copy a picture into the design's assets, under a name not taken yet, and give the URL a
/// screen uses for it: `../assets/<name>`.
pub(crate) fn add_picture(root: &Path, from: &Path) -> Result<String, String> {
    let dir = folder(root).join(ASSETS);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = slug(&from.file_stem().unwrap_or_default().to_string_lossy());
    let stem = if stem.is_empty() {
        "picture".to_string()
    } else {
        stem
    };
    let ext = from
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "png".into());
    let name = (1..)
        .map(|n| match n {
            1 => format!("{stem}.{ext}"),
            n => format!("{stem}-{n}.{ext}"),
        })
        .find(|n| !dir.join(n).exists())
        .unwrap_or_default();
    std::fs::copy(from, dir.join(&name))
        .map_err(|e| format!("Couldn't copy {}: {e}", from.display()))?;
    Ok(format!("../{ASSETS}/{name}"))
}

pub(crate) fn screen_path(root: &Path, id: &str) -> PathBuf {
    folder(root).join(SCREENS).join(format!("{id}.html"))
}

/// The canvas, with screens added that are in `screens/` but not on it yet (written by an
/// agent or by hand) and screens dropped whose file is gone.
pub(crate) fn load(root: &Path) -> Result<Canvas, String> {
    let file = folder(root).join(CANVAS);
    let mut canvas: Canvas = match std::fs::read_to_string(&file) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{FOLDER}/{CANVAS}: {e}"))?,
        Err(_) => Canvas::default(),
    };
    let dir = folder(root).join(SCREENS);
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension().is_some_and(|x| x == "html"))
                        .then(|| p.file_stem()?.to_str().map(String::from))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    canvas.screens.retain(|s| files.contains(&s.id));
    for id in files {
        if canvas.screen(&id).is_none() {
            let text = std::fs::read_to_string(dir.join(format!("{id}.html"))).unwrap_or_default();
            let (x, y) = canvas.next_spot();
            canvas.screens.push(Screen {
                name: title_of(&text).unwrap_or_else(|| id.replace('-', " ")),
                device: device_of(&text),
                id,
                x,
                y,
                ..Default::default()
            });
        }
    }
    Ok(canvas)
}

pub(crate) fn save(root: &Path, canvas: &Canvas) -> std::io::Result<()> {
    let dir = folder(root);
    std::fs::create_dir_all(&dir)?;
    let text = toml::to_string_pretty(canvas).unwrap_or_default();
    let file = dir.join(CANVAS);
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(file, text)?;
    }
    Ok(())
}

/// Write the shared files a new design starts with, keeping any that exist.
pub(crate) fn seed(root: &Path) -> std::io::Result<()> {
    let dir = folder(root);
    std::fs::create_dir_all(dir.join(SCREENS))?;
    for (name, text) in [
        (TOKENS, TOKENS_CSS.to_string()),
        (COMPONENTS, COMPONENTS_CSS.to_string()),
        (ICONS, ICONS_CSS.to_string()),
        (PROTOTYPE, prototype_js()),
    ] {
        let file = dir.join(name);
        let was = std::fs::read_to_string(&file).ok();
        let old = match name {
            PROTOTYPE => {
                was.as_deref() == Some(PROTOTYPE_JS_V1)
                    || was.as_deref() == Some(prototype_js_v2().as_str())
            }
            COMPONENTS => was.as_deref() == Some(COMPONENTS_CSS_V1),
            _ => false,
        };
        if was.is_none() || old {
            std::fs::write(file, text)?;
        } else if name == TOKENS {
            // A theme from before the type scale, shadows and status colours gets them,
            // keeping every value it has.
            let was = was.unwrap_or_default();
            let now = with_missing_tokens(&was);
            if now != was {
                std::fs::write(file, now)?;
            }
        }
    }
    Ok(())
}

/// `css` with the tokens a new theme has that it lacks, added to its `:root`.
fn with_missing_tokens(css: &str) -> String {
    let Some((start, end)) = root_block(TOKENS_CSS) else {
        return css.to_string();
    };
    let mut out = css.to_string();
    for line in TOKENS_CSS[start..end].lines() {
        let Some((name, value)) = line.trim().trim_end_matches(';').split_once(':') else {
            continue;
        };
        if var(&out, name.trim()).is_none() {
            out = set_var(&out, name.trim(), value.trim());
        }
    }
    out
}

/// The `<title>` of a screen.
pub(crate) fn title_of(html: &str) -> Option<String> {
    let start = html.find("<title>")? + 7;
    let end = html[start..].find("</title>")? + start;
    let t = html[start..end].trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// The device a screen says it's for, in `<meta name="runo:device">`.
pub(crate) fn device_of(html: &str) -> Device {
    let Some(i) = html.find("name=\"runo:device\"") else {
        return Device::Phone;
    };
    let rest = &html[i..html.len().min(i + 80)];
    if rest.contains("desktop") {
        Device::Desktop
    } else if rest.contains("tablet") {
        Device::Tablet
    } else {
        Device::Phone
    }
}

/// Screens this one links to with `data-goto`.
#[cfg(test)]
pub(crate) fn links_of(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("data-goto=\"") {
        rest = &rest[i + 11..];
        if let Some(end) = rest.find('"') {
            let id = rest[..end].trim().trim_end_matches(".html").to_string();
            if !id.is_empty() && !out.contains(&id) {
                out.push(id);
            }
            rest = &rest[end..];
        }
    }
    out
}

/// Replace the `<title>` of a screen.
pub(crate) fn with_title(html: &str, title: &str) -> String {
    let title = esc(title);
    match (html.find("<title>"), html.find("</title>")) {
        (Some(a), Some(b)) if a < b => format!("{}<title>{title}{}", &html[..a], &html[b..]),
        _ => html.replacen("<head>", &format!("<head>\n<title>{title}</title>"), 1),
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---- Theme

/// The part of `tokens.css` the theme panel edits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Theme {
    pub primary: u32,
    pub font: String,
    pub radius: f32,
    pub space: f32,
}

/// Fonts offered in the theme panel, as CSS font stacks.
pub(crate) const FONTS: [(&str, &str); 5] = [
    (
        "System",
        "-apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, sans-serif",
    ),
    (
        "Inter",
        "Inter, -apple-system, \"Segoe UI\", Roboto, sans-serif",
    ),
    (
        "Rounded",
        "\"SF Pro Rounded\", \"Nunito\", ui-rounded, sans-serif",
    ),
    ("Serif", "Georgia, \"Times New Roman\", serif"),
    ("Mono", "ui-monospace, \"SF Mono\", Menlo, monospace"),
];

/// Primary colours offered in the theme panel.
pub(crate) const PRIMARIES: [u32; 10] = [
    0x2563eb, 0x4f46e5, 0x7c3aed, 0xdb2777, 0xdc2626, 0xea580c, 0xca8a04, 0x16a34a, 0x0d9488,
    0x0f172a,
];

/// The value of a CSS custom property in the first `:root` block.
pub(crate) fn var(css: &str, name: &str) -> Option<String> {
    let block = root_block(css)?;
    let key = format!("{name}:");
    let i = css[block.0..block.1].find(&key)? + block.0 + key.len();
    let end = css[i..block.1].find(';').map(|e| e + i).unwrap_or(block.1);
    Some(css[i..end].trim().to_string())
}

/// `css` with a custom property in the first `:root` block set, added if missing.
pub(crate) fn set_var(css: &str, name: &str, value: &str) -> String {
    let Some((start, end)) = root_block(css) else {
        return format!(":root {{\n  {name}: {value};\n}}\n{css}");
    };
    let key = format!("{name}:");
    if let Some(i) = css[start..end].find(&key).map(|i| i + start) {
        let v = i + key.len();
        let stop = css[v..end].find(';').map(|e| e + v).unwrap_or(end);
        format!("{}{key} {value}{}", &css[..i], &css[stop..])
    } else {
        format!("{}  {name}: {value};\n{}", &css[..end], &css[end..])
    }
}

/// A design variable: a custom property of the theme, its value, and its value in dark mode
/// when that differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Var {
    pub name: String,
    pub light: String,
    pub dark: Option<String>,
}

/// The custom properties set in a block of declarations, in order.
fn decls(block: &str) -> Vec<(String, String)> {
    block
        .split(';')
        .filter_map(|d| {
            let d = d.trim();
            let d = d.rsplit_once("*/").map(|(_, r)| r.trim()).unwrap_or(d);
            let (k, v) = d.split_once(':')?;
            k.trim()
                .starts_with("--")
                .then(|| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Every variable of the theme, with its dark value.
pub(crate) fn vars(css: &str) -> Vec<Var> {
    let dark = dark_block(css)
        .map(|(a, b)| decls(&css[a..b]))
        .unwrap_or_default();
    root_block(css)
        .map(|(a, b)| decls(&css[a..b]))
        .unwrap_or_default()
        .into_iter()
        .map(|(name, light)| Var {
            dark: dark
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone()),
            name,
            light,
        })
        .collect()
}

/// The byte range inside the braces of the `:root` that dark mode sets.
fn dark_block(css: &str) -> Option<(usize, usize)> {
    let at = css.find("prefers-color-scheme: dark")?;
    let root = css[at..].find(":root")? + at;
    let open = css[root..].find('{')? + root + 1;
    let close = css[open..].find('}')? + open;
    Some((open, close))
}

/// `css` with a variable's dark mode value set, or taken out so it's the same as light.
pub(crate) fn set_dark_var(css: &str, name: &str, value: &str) -> String {
    let value = value.trim();
    let Some((start, end)) = dark_block(css) else {
        if value.is_empty() {
            return css.to_string();
        }
        return format!(
            "{}\n\n@media (prefers-color-scheme: dark) {{\n  :root {{\n    {name}: {value};\n  }}\n}}\n",
            css.trim_end()
        );
    };
    let key = format!("{name}:");
    match css[start..end].find(&key).map(|i| i + start) {
        Some(i) => {
            let stop = css[i..end].find(';').map(|e| e + i + 1).unwrap_or(end);
            if value.is_empty() {
                let line = css[..i].rfind('\n').map(|n| n + 1).unwrap_or(i);
                let after = if css[stop..].starts_with('\n') {
                    stop + 1
                } else {
                    stop
                };
                format!("{}{}", &css[..line], &css[after..])
            } else {
                format!("{}{key} {value};{}", &css[..i], &css[stop..])
            }
        }
        None if value.is_empty() => css.to_string(),
        None => {
            let line = css[..end].rfind('\n').map(|n| n + 1).unwrap_or(end);
            format!("{}    {name}: {value};\n{}", &css[..line], &css[line..])
        }
    }
}

/// `text` with a variable renamed wherever it's set or used.
pub(crate) fn rename_var(text: &str, from: &str, to: &str) -> String {
    text.replace(&format!("{from}:"), &format!("{to}:"))
        .replace(&format!("var({from})"), &format!("var({to})"))
        .replace(&format!("var({from},"), &format!("var({to},"))
}

/// `css` without a variable, in light and dark.
pub(crate) fn remove_var(css: &str, name: &str) -> String {
    css.lines()
        .filter(|l| !l.trim_start().starts_with(&format!("{name}:")))
        .collect::<Vec<_>>()
        .join("\n")
        + if css.ends_with('\n') { "\n" } else { "" }
}

/// The byte range inside the braces of the first `:root {`.
fn root_block(css: &str) -> Option<(usize, usize)> {
    let at = css.find(":root")?;
    let open = css[at..].find('{')? + at + 1;
    let close = css[open..].find('}')? + open;
    Some((open, close))
}

impl Theme {
    pub(crate) fn read(css: &str) -> Theme {
        let px = |name: &str, d: f32| {
            var(css, name)
                .and_then(|v| v.trim_end_matches("px").trim().parse().ok())
                .unwrap_or(d)
        };
        Theme {
            primary: var(css, "--primary")
                .and_then(|v| crate::archdraw::model::parse_hex(&v))
                .unwrap_or(0x2563eb),
            font: var(css, "--font")
                .and_then(|v| FONTS.iter().find(|f| f.1 == v).map(|f| f.0.to_string()))
                .unwrap_or_else(|| "Custom".into()),
            radius: px("--radius", 12.),
            space: px("--space", 8.),
        }
    }

    /// `css` with this theme written into it.
    pub(crate) fn write(&self, css: &str) -> String {
        let mut out = set_var(css, "--primary", &crate::archdraw::model::hex(self.primary));
        if let Some(stack) = FONTS.iter().find(|f| f.0 == self.font) {
            out = set_var(&out, "--font", stack.1);
        }
        out = set_var(&out, "--radius", &format!("{}px", self.radius));
        set_var(&out, "--space", &format!("{}px", self.space))
    }
}

// ---- Templates

/// Screens a new one can start from.
pub(crate) const TEMPLATES: [(&str, &str); 15] = [
    ("Blank", "An empty screen with a title"),
    ("Onboarding", "A picture, a promise and Get started"),
    ("Sign in", "Email, password and a primary button"),
    ("Home", "Greeting, chips, a carousel and tabs"),
    ("List", "A header, search and a list of rows"),
    ("Detail", "A hero, facts and actions for one item"),
    ("Shop", "Search, categories and a product grid"),
    ("Checkout", "Items, totals and a pay button"),
    ("Profile", "Avatar, stats and account rows"),
    ("Chat", "Messages and a box to write in"),
    ("Notifications", "Grouped updates with icons"),
    ("Search", "Recent searches and results"),
    ("Settings", "Grouped rows with switches"),
    ("Dashboard", "Stat cards and a recent list"),
    ("Empty state", "Nothing here yet, with one action"),
];

/// A screen that sits over a tab bar: the screen's content grows so the bar stays at the
/// bottom, whatever the content's height.
fn with_tabs(top: &str, content: &str, on: usize) -> String {
    let tabs = [
        ("house", "Home"),
        ("search", "Search"),
        ("heart", "Saved"),
        ("user", "Profile"),
    ];
    let tabs: String = tabs
        .iter()
        .enumerate()
        .map(|(i, (icon, label))| {
            format!(
                "<a class=\"tab{}\"><i class=\"icon i-{icon}\"></i>{label}</a>",
                if i == on { " on" } else { "" }
            )
        })
        .collect();
    format!(
        "<div class=\"stack\" style=\"min-height: 100vh\">\n{top}<main class=\"screen stack gap-lg grow\">\n{content}\n</main>\n<nav class=\"tabbar\">{tabs}</nav>\n</div>"
    )
}

/// A new screen's HTML.
pub(crate) fn template(kind: &str, name: &str, device: Device) -> String {
    let body = match kind {
        "Onboarding" => r##"<main class="screen center">
  <div class="stack gap-lg center-text">
    <div class="image dusk w-full"></div>
    <div class="stack gap-sm">
      <h1>Plan less, live more</h1>
      <p class="muted">Everything you need for the week, in one calm place.</p>
    </div>
    <div class="cluster" style="justify-content: center"><span class="dot"></span><span class="dot" style="opacity: 0.3"></span><span class="dot" style="opacity: 0.3"></span></div>
    <button class="button primary block lg">Get started</button>
    <a class="link">I already have an account</a>
  </div>
</main>"##
            .to_string(),
        "Home" => with_tabs(
            "<header class=\"bar plain\"><div class=\"stack\"><span class=\"small muted\">Good morning</span><strong class=\"bar-title\">Alex</strong></div><div class=\"avatar\">A</div></header>\n",
            r##"  <div class="search-box"><i class="icon i-search"></i><input class="search" type="search" placeholder="Search"></div>
  <div class="scroll-x"><span class="chip on">All</span><span class="chip">Popular</span><span class="chip">New</span><span class="chip">Nearby</span></div>
  <section class="section">
    <div class="section-head"><h3>For you</h3><a class="link small">See all</a></div>
    <div class="scroll-x">
      <div class="stack gap-xs" style="width: 220px"><div class="image cool"></div><strong>Morning run</strong><span class="small muted">25 min · Easy</span></div>
      <div class="stack gap-xs" style="width: 220px"><div class="image warm"></div><strong>Sunset yoga</strong><span class="small muted">40 min · Calm</span></div>
    </div>
  </section>
  <section class="section">
    <h3>Today</h3>
    <ul class="list card">
      <li class="row"><span class="icon-tile"><i class="icon i-calendar"></i></span><div class="grow"><strong>Team sync</strong><p class="small muted">10:00</p></div><i class="icon i-chevron-right"></i></li>
      <li class="row"><span class="icon-tile"><i class="icon i-shopping-bag"></i></span><div class="grow"><strong>Groceries</strong><p class="small muted">6 items</p></div><i class="icon i-chevron-right"></i></li>
    </ul>
  </section>"##,
            0,
        ),
        "Shop" => with_tabs(
            "<header class=\"bar plain\"><h1>Shop</h1><button class=\"icon-button filled\"><i class=\"icon i-shopping-cart\"></i></button></header>\n",
            r##"  <div class="search-box"><i class="icon i-search"></i><input class="search" type="search" placeholder="Search products"></div>
  <div class="scroll-x"><span class="chip on">All</span><span class="chip">Shoes</span><span class="chip">Bags</span><span class="chip">Hats</span></div>
  <div class="grid-2">
    <div class="stack gap-xs"><div class="image square warm"></div><strong>Canvas tote</strong><span class="muted">$38</span></div>
    <div class="stack gap-xs"><div class="image square cool"></div><strong>Trail runner</strong><span class="muted">$120</span></div>
    <div class="stack gap-xs"><div class="image square green"></div><strong>Wool cap</strong><span class="muted">$24</span></div>
    <div class="stack gap-xs"><div class="image square dusk"></div><strong>Day pack</strong><span class="muted">$86</span></div>
  </div>"##,
            0,
        ),
        "Checkout" => r##"<header class="bar"><button class="icon-button" data-goto="back"><i class="icon i-chevron-left"></i></button><strong class="bar-title">Checkout</strong><span style="width: 44px"></span></header>
<main class="screen stack gap-lg">
  <ul class="list">
    <li class="row"><div class="image square warm" style="width: 56px"></div><div class="grow"><strong>Canvas tote</strong><p class="small muted">Qty 1</p></div><strong>$38</strong></li>
    <li class="row"><div class="image square cool" style="width: 56px"></div><div class="grow"><strong>Trail runner</strong><p class="small muted">Qty 1</p></div><strong>$120</strong></li>
  </ul>
  <div class="card stack gap-sm">
    <div class="row-between"><span class="muted">Subtotal</span><span>$158</span></div>
    <div class="row-between"><span class="muted">Delivery</span><span>Free</span></div>
    <hr class="divider">
    <div class="row-between"><strong>Total</strong><strong>$158</strong></div>
  </div>
  <div class="card list"><div class="row"><i class="icon i-credit-card"></i><span class="grow">Visa ending 4242</span><a class="link small">Change</a></div></div>
  <button class="button primary block lg">Pay $158</button>
</main>"##
            .to_string(),
        "Profile" => with_tabs(
            "<header class=\"bar plain\"><span></span><button class=\"icon-button\"><i class=\"icon i-settings\"></i></button></header>\n",
            r##"  <div class="stack gap-sm center-text"><div class="avatar lg">AL</div><h2>Alex Lee</h2><p class="muted">alex@example.com</p></div>
  <div class="grid-3">
    <div class="card stat center-text"><span class="value">24</span><span class="small muted">Trips</span></div>
    <div class="card stat center-text"><span class="value">9</span><span class="small muted">Saved</span></div>
    <div class="card stat center-text"><span class="value">4.9</span><span class="small muted">Rating</span></div>
  </div>
  <ul class="list card">
    <li class="row"><i class="icon i-user"></i><span class="grow">Personal details</span><i class="icon i-chevron-right"></i></li>
    <li class="row"><i class="icon i-credit-card"></i><span class="grow">Payments</span><i class="icon i-chevron-right"></i></li>
    <li class="row"><i class="icon i-bell"></i><span class="grow">Notifications</span><i class="icon i-chevron-right"></i></li>
    <li class="row"><i class="icon i-log-out"></i><span class="grow">Sign out</span></li>
  </ul>"##,
            3,
        ),
        "Chat" => r##"<div class="stack" style="min-height: 100vh">
<header class="bar"><button class="icon-button" data-goto="back"><i class="icon i-chevron-left"></i></button><div class="cluster grow"><div class="avatar sm">S</div><strong>Sam</strong></div><button class="icon-button"><i class="icon i-phone"></i></button></header>
<main class="screen stack gap-sm grow">
  <p class="tiny muted center-text">Today 09:41</p>
  <div class="card" style="align-self: flex-start; max-width: 75%">Are we still on for tonight?</div>
  <div class="card accent" style="align-self: flex-end; max-width: 75%">Yes! 7pm at the usual place.</div>
  <div class="card" style="align-self: flex-start; max-width: 75%">Perfect, see you there.</div>
</main>
<div class="bar" style="border-top: 1px solid var(--border); border-bottom: none"><input class="input grow" placeholder="Message"><button class="icon-button filled"><i class="icon i-send"></i></button></div>
</div>"##
            .to_string(),
        "Notifications" => r##"<header class="bar"><strong class="bar-title">Notifications</strong><a class="link small">Mark all read</a></header>
<main class="screen stack gap-lg">
  <section class="section">
    <h3 class="muted">Today</h3>
    <ul class="list">
      <li class="row"><span class="icon-tile"><i class="icon i-heart"></i></span><div class="grow"><strong>Sam liked your post</strong><p class="small muted">2 min ago</p></div><span class="dot"></span></li>
      <li class="row"><span class="icon-tile"><i class="icon i-message-circle"></i></span><div class="grow"><strong>New comment</strong><p class="small muted">1 h ago</p></div><span class="dot"></span></li>
    </ul>
  </section>
  <section class="section">
    <h3 class="muted">Earlier</h3>
    <ul class="list">
      <li class="row"><span class="icon-tile"><i class="icon i-gift"></i></span><div class="grow"><strong>You earned a reward</strong><p class="small muted">Yesterday</p></div></li>
    </ul>
  </section>
</main>"##
            .to_string(),
        "Search" => with_tabs(
            "",
            r##"  <div class="search-box"><i class="icon i-search"></i><input class="search" type="search" placeholder="Search" value="coffee"></div>
  <section class="section">
    <h3 class="muted">Recent</h3>
    <div class="cluster"><span class="chip">Brunch</span><span class="chip">Parks</span><span class="chip">Bookshops</span></div>
  </section>
  <section class="section">
    <h3>Results</h3>
    <ul class="list">
      <li class="row"><div class="image square warm" style="width: 48px"></div><div class="grow"><strong>Corner Roasters</strong><p class="small muted">0.4 km · Open now</p></div><span class="badge">4.8</span></li>
      <li class="row"><div class="image square green" style="width: 48px"></div><div class="grow"><strong>Leaf &amp; Bean</strong><p class="small muted">1.1 km · Open now</p></div><span class="badge">4.6</span></li>
    </ul>
  </section>"##,
            1,
        ),
        "Sign in" => r##"<main class="screen center">
  <div class="stack gap-lg">
    <div class="stack gap-sm">
      <h1>Welcome back</h1>
      <p class="muted">Sign in to continue.</p>
    </div>
    <label class="field"><span>Email</span><input type="email" placeholder="you@example.com"></label>
    <label class="field"><span>Password</span><input type="password" placeholder="••••••••"></label>
    <button class="button primary">Sign in</button>
    <a class="link" href="#">Forgot your password?</a>
  </div>
</main>"##
            .to_string(),
        "List" => r##"<header class="bar"><h2>Items</h2><button class="button ghost">Add</button></header>
<main class="screen stack gap-md">
  <input class="search" type="search" placeholder="Search">
  <ul class="list">
    <li class="row"><div class="avatar">A</div><div class="grow"><strong>First item</strong><p class="muted">A short description</p></div><span class="badge">New</span></li>
    <li class="row"><div class="avatar">B</div><div class="grow"><strong>Second item</strong><p class="muted">A short description</p></div></li>
    <li class="row"><div class="avatar">C</div><div class="grow"><strong>Third item</strong><p class="muted">A short description</p></div></li>
  </ul>
</main>"##
            .to_string(),
        "Detail" => r##"<header class="bar"><button class="icon-button" data-goto="back"><i class="icon i-chevron-left"></i></button><strong class="bar-title">Details</strong><button class="icon-button"><i class="icon i-heart"></i></button></header>
<main class="screen stack gap-lg">
  <div class="hero"></div>
  <div class="stack gap-sm">
    <h1>Item name</h1>
    <p class="muted">A sentence or two about this item and why it matters.</p>
  </div>
  <div class="card stack gap-sm">
    <div class="row-between"><span class="muted">Status</span><strong>Active</strong></div>
    <div class="row-between"><span class="muted">Updated</span><strong>Today</strong></div>
  </div>
  <button class="button primary">Main action</button>
</main>"##
            .to_string(),
        "Settings" => r##"<header class="bar"><h2>Settings</h2></header>
<main class="screen stack gap-lg">
  <section class="stack gap-sm">
    <h3 class="muted">Account</h3>
    <div class="card list">
      <div class="row"><span class="grow">Name</span><span class="muted">Alex</span></div>
      <div class="row"><span class="grow">Email</span><span class="muted">alex@example.com</span></div>
    </div>
  </section>
  <section class="stack gap-sm">
    <h3 class="muted">Notifications</h3>
    <div class="card list">
      <div class="row"><span class="grow">Push notifications</span><span class="switch on"></span></div>
      <div class="row"><span class="grow">Email updates</span><span class="switch"></span></div>
    </div>
  </section>
</main>"##
            .to_string(),
        "Dashboard" => r##"<header class="bar"><h2>Overview</h2><div class="avatar">A</div></header>
<main class="screen stack gap-lg">
  <div class="grid">
    <div class="card stack gap-xs"><span class="muted">Revenue</span><strong class="big">$12.4k</strong></div>
    <div class="card stack gap-xs"><span class="muted">Orders</span><strong class="big">318</strong></div>
    <div class="card stack gap-xs"><span class="muted">Customers</span><strong class="big">1,204</strong></div>
    <div class="card stack gap-xs"><span class="muted">Refunds</span><strong class="big">4</strong></div>
  </div>
  <section class="stack gap-sm">
    <h3>Recent</h3>
    <ul class="list card">
      <li class="row"><span class="grow">Order #1042</span><span class="badge">Paid</span></li>
      <li class="row"><span class="grow">Order #1041</span><span class="badge">Paid</span></li>
      <li class="row"><span class="grow">Order #1040</span><span class="badge">Refunded</span></li>
    </ul>
  </section>
</main>"##
            .to_string(),
        "Empty state" => r##"<main class="screen center">
  <div class="stack gap-md center-text">
    <div class="illustration"></div>
    <h2>Nothing here yet</h2>
    <p class="muted">When you add something, it shows up here.</p>
    <button class="button primary">Add the first one</button>
  </div>
</main>"##
            .to_string(),
        _ => format!(
            "<header class=\"bar\"><h2>{}</h2></header>\n<main class=\"screen stack gap-md\">\n  <p class=\"muted\">Start designing.</p>\n</main>",
            esc(name)
        ),
    };
    let device = match device {
        Device::Phone => "phone",
        Device::Tablet => "tablet",
        Device::Desktop => "desktop",
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta name=\"runo:device\" content=\"{device}\">\n<title>{}</title>\n<link rel=\"stylesheet\" href=\"../tokens.css\">\n<link rel=\"stylesheet\" href=\"../components.css\">\n<script src=\"../prototype.js\" defer></script>\n</head>\n<body>\n{body}\n</body>\n</html>\n",
        esc(name)
    )
}

/// What `data-goto` says to go back to the screen before.
pub(crate) const BACK: &str = "back";

/// Colours the inspector offers, as the tokens they come from.
pub(crate) const COLOR_TOKENS: [&str; 7] = [
    "--primary",
    "--on-primary",
    "--bg",
    "--surface",
    "--text",
    "--muted",
    "--border",
];

/// The theme a new design starts with.
pub(crate) const TOKENS_CSS: &str = include_str!("assets/tokens.css");

/// The building blocks a new design starts with.
pub(crate) const COMPONENTS_CSS: &str = include_str!("assets/components.css");

/// Line icons for the screens, imported by `components.css`.
pub(crate) const ICONS_CSS: &str = include_str!("assets/icons.css");

/// What `components.css` was before it grew icons, a type scale and more parts, replaced
/// when a design still has it unchanged.
const COMPONENTS_CSS_V1: &str = r#"/* Building blocks the screens share. Prefer these classes and the tokens over raw values. */
* { box-sizing: border-box; }
html, body { margin: 0; background: var(--bg); color: var(--text); font-family: var(--font); font-size: 16px; line-height: 1.45; }
h1, h2, h3, p { margin: 0; }
h1 { font-size: 30px; line-height: 1.15; }
h2 { font-size: 22px; line-height: 1.2; }
h3 { font-size: 15px; }
.muted { color: var(--muted); }
.screen { padding: calc(var(--space) * 3); }
.center { min-height: 100vh; display: flex; flex-direction: column; justify-content: center; }
.center-text { text-align: center; align-items: center; }
.stack { display: flex; flex-direction: column; }
.gap-xs { gap: calc(var(--space) * 0.5); }
.gap-sm { gap: var(--space); }
.gap-md { gap: calc(var(--space) * 2); }
.gap-lg { gap: calc(var(--space) * 3); }
.grow { flex: 1; min-width: 0; }
.bar { display: flex; align-items: center; justify-content: space-between; padding: calc(var(--space) * 2) calc(var(--space) * 3); border-bottom: 1px solid var(--border); }
.button { appearance: none; border: 1px solid var(--border); background: var(--surface); color: var(--text); font: inherit; font-weight: 600; padding: calc(var(--space) * 1.5) calc(var(--space) * 2.5); border-radius: var(--radius); cursor: pointer; }
.button.primary { background: var(--primary); border-color: var(--primary); color: var(--on-primary); }
.button.ghost { background: transparent; border-color: transparent; color: var(--primary); }
.link { color: var(--primary); text-decoration: none; font-weight: 500; }
.field { display: flex; flex-direction: column; gap: calc(var(--space) * 0.75); font-size: 14px; font-weight: 500; }
.field input, .search { font: inherit; font-weight: 400; padding: calc(var(--space) * 1.5); border: 1px solid var(--border); border-radius: var(--radius); background: var(--bg); color: var(--text); width: 100%; }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: calc(var(--space) * 2); }
.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
.row { display: flex; align-items: center; gap: calc(var(--space) * 1.5); padding: calc(var(--space) * 1.5) 0; border-bottom: 1px solid var(--border); }
.card.list .row, .list.card .row { padding: calc(var(--space) * 1.5) 0; }
.row:last-child { border-bottom: none; }
.row-between { display: flex; justify-content: space-between; align-items: center; }
.avatar { width: 40px; height: 40px; border-radius: 50%; background: var(--primary); color: var(--on-primary); display: grid; place-items: center; font-weight: 600; flex: none; }
.badge { font-size: 12px; font-weight: 600; padding: 2px 8px; border-radius: 999px; background: color-mix(in srgb, var(--primary) 14%, transparent); color: var(--primary); }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: calc(var(--space) * 1.5); }
.big { font-size: 24px; }
.hero, .image { aspect-ratio: 16 / 10; border-radius: var(--radius); background: linear-gradient(135deg, color-mix(in srgb, var(--primary) 35%, var(--surface)), var(--surface)); }
.illustration { width: 120px; height: 120px; border-radius: 50%; background: color-mix(in srgb, var(--primary) 16%, var(--surface)); }
.switch { width: 44px; height: 26px; border-radius: 999px; background: var(--border); position: relative; flex: none; }
.switch::after { content: ""; position: absolute; top: 3px; left: 3px; width: 20px; height: 20px; border-radius: 50%; background: #fff; }
.switch.on { background: var(--primary); }
.switch.on::after { left: 21px; }
.divider { border: none; border-top: 1px solid var(--border); margin: 0; }
.tabbar { position: sticky; bottom: 0; display: flex; justify-content: space-around; padding: var(--space); border-top: 1px solid var(--border); background: var(--bg); }
.tab { font-size: 12px; color: var(--muted); padding: var(--space); }
.tab.on { color: var(--primary); font-weight: 600; }
[data-goto] { cursor: pointer; }
"#;

/// What `prototype.js` was before controls worked, replaced when a design still has it.
const PROTOTYPE_JS_V1: &str = r#"// Clicking an element with data-goto="other-screen" opens that screen, so the design
// can be clicked through in any browser.
document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-goto]');
  if (!el) return;
  e.preventDefault();
  location.href = el.dataset.goto.replace(/\.html$/, '') + '.html';
});
"#;

/// The controls' behaviour, in `prototype.js` and run in Play for designs without it.
pub(crate) const UI_JS: &str = r#"// Controls work when clicked: a .switch flips, a .tab (or a child of [data-select])
// becomes the one that's on, [data-toggle] toggles a class on itself, and [data-show]
// shows or hides what its selector names, for menus, accordions and sheets.
(() => {
  if (window.__runoUi) return;
  window.__runoUi = true;
  document.addEventListener('click', (e) => {
    const t = e.target;
    if (!(t instanceof Element)) return;
    const sw = t.closest('.switch');
    if (sw) {
      sw.classList.toggle('on');
      sw.setAttribute('aria-checked', String(sw.classList.contains('on')));
    }
    const pick = t.closest('.tab, [role="tab"], [data-select] > *');
    if (pick && pick.parentElement) {
      for (const s of pick.parentElement.children) s.classList.toggle('on', s === pick);
    }
    const tg = t.closest('[data-toggle]');
    if (tg) tg.classList.toggle(tg.dataset.toggle || 'on');
    const sh = t.closest('[data-show]');
    if (sh) {
      for (const el of document.querySelectorAll(sh.dataset.show)) el.hidden = !el.hidden;
    }
  });
})();
"#;

/// What `prototype.js` was before links could go back, replaced when a design still has it.
fn prototype_js_v2() -> String {
    prototype_js().replace(
        "  if (el.dataset.goto === 'back') { history.back(); return; }\n",
        "",
    )
}

/// Clicking through links, then the controls' behaviour.
fn prototype_js() -> String {
    format!(
        r#"// Clicking an element with data-goto="other-screen" opens that screen, so the design
// can be clicked through in any browser.
document.addEventListener('click', (e) => {{
  const el = e.target.closest('[data-goto]');
  if (!el) return;
  e.preventDefault();
  if (el.dataset.goto === 'back') {{ history.back(); return; }}
  location.href = el.dataset.goto.replace(/\.html$/, '') + '.html';
}});

{UI_JS}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-design-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_template_uses_classes_the_kit_has() {
        for (kind, _) in TEMPLATES {
            let html = template(kind, kind, Device::Phone);
            assert_eq!(
                super::super::parts::missing_classes(&html),
                Vec::<String>::new(),
                "{kind}"
            );
        }
    }

    #[test]
    fn pictures_are_copied_into_the_design_under_free_names() {
        let root = temp("pictures");
        let from = root.join("My Photo.PNG");
        std::fs::write(&from, b"png").unwrap();
        assert_eq!(add_picture(&root, &from).unwrap(), "../assets/my-photo.png");
        assert_eq!(
            add_picture(&root, &from).unwrap(),
            "../assets/my-photo-2.png"
        );
        assert!(folder(&root).join("assets/my-photo-2.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_old_prototype_script_is_upgraded_but_an_edited_one_is_kept() {
        let root = temp("proto");
        seed(&root).unwrap();
        let file = folder(&root).join(PROTOTYPE);
        assert!(std::fs::read_to_string(&file).unwrap().contains("__runoUi"));
        std::fs::write(&file, PROTOTYPE_JS_V1).unwrap();
        seed(&root).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("__runoUi"));
        assert_ne!(prototype_js_v2(), prototype_js());
        std::fs::write(&file, prototype_js_v2()).unwrap();
        seed(&root).unwrap();
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("history.back()"));
        std::fs::write(&file, "// mine").unwrap();
        seed(&root).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "// mine");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_old_design_gets_icons_and_the_new_tokens_but_keeps_its_own() {
        let root = temp("upgrade");
        let dir = folder(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(COMPONENTS), COMPONENTS_CSS_V1).unwrap();
        std::fs::write(
            dir.join(TOKENS),
            ":root {\n  --primary: #db2777;\n  --radius: 4px;\n}\n",
        )
        .unwrap();
        seed(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(COMPONENTS)).unwrap(),
            COMPONENTS_CSS
        );
        assert!(dir.join(ICONS).exists());
        let tokens = std::fs::read_to_string(dir.join(TOKENS)).unwrap();
        assert_eq!(var(&tokens, "--primary").as_deref(), Some("#db2777"));
        assert_eq!(var(&tokens, "--radius").as_deref(), Some("4px"));
        assert_eq!(var(&tokens, "--danger").as_deref(), Some("#dc2626"));
        assert!(var(&tokens, "--text-xl").is_some());
        // Components someone changed are theirs.
        std::fs::write(dir.join(COMPONENTS), "/* mine */").unwrap();
        seed(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(COMPONENTS)).unwrap(),
            "/* mine */"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn screens_on_disk_join_the_canvas() {
        let root = temp("load");
        seed(&root).unwrap();
        let html = template("Sign in", "Sign in", Device::Phone);
        std::fs::write(screen_path(&root, "sign-in"), &html).unwrap();
        std::fs::write(
            screen_path(&root, "dash"),
            template("Dashboard", "Overview", Device::Desktop),
        )
        .unwrap();
        let mut canvas = load(&root).unwrap();
        assert_eq!(canvas.screens.len(), 2);
        let dash = canvas.screen("dash").unwrap();
        assert_eq!(dash.name, "Overview");
        assert_eq!(dash.device, Device::Desktop);
        canvas.screen_mut("dash").unwrap().x = 900.;
        save(&root, &canvas).unwrap();
        std::fs::remove_file(screen_path(&root, "sign-in")).unwrap();
        let again = load(&root).unwrap();
        assert_eq!(again.screens.len(), 1);
        assert_eq!(again.screens[0].x, 900.);
        assert!(folder(&root).join(TOKENS).exists());
    }

    #[test]
    fn ids_titles_and_links() {
        let mut canvas = Canvas::default();
        assert_eq!(canvas.new_id("Sign in!"), "sign-in");
        canvas.screens.push(Screen {
            id: "sign-in".into(),
            ..Default::default()
        });
        assert_eq!(canvas.new_id("Sign in"), "sign-in-2");
        let html = template("Blank", "Home & away", Device::Phone);
        assert_eq!(title_of(&html).as_deref(), Some("Home &amp; away"));
        let renamed = with_title(&html, "Profile");
        assert_eq!(title_of(&renamed).as_deref(), Some("Profile"));
        let page = r#"<a data-goto="list">x</a><button data-goto="detail.html">y</button><a data-goto="list">z</a>"#;
        assert_eq!(links_of(page), vec!["list", "detail"]);
    }

    #[test]
    fn theme_reads_and_writes_tokens() {
        let mut theme = Theme::read(TOKENS_CSS);
        assert_eq!(theme.primary, 0x2563eb);
        assert_eq!(theme.font, "System");
        assert_eq!(theme.radius, 12.);
        theme.primary = 0xdb2777;
        theme.font = "Serif".into();
        theme.radius = 4.;
        let css = theme.write(TOKENS_CSS);
        let back = Theme::read(&css);
        assert_eq!(back, theme);
        // The dark block keeps its own values.
        assert!(css.contains("--bg: #0b0d12"));
        assert_eq!(set_var("a{}", "--x", "1"), ":root {\n  --x: 1;\n}\na{}");
        assert!(set_var(":root {\n}", "--y", "2px").contains("--y: 2px;"));
    }

    #[test]
    fn variables_have_a_light_and_a_dark_value() {
        let v = vars(TOKENS_CSS);
        let bg = v.iter().find(|v| v.name == "--bg").unwrap();
        assert_eq!(
            (bg.light.as_str(), bg.dark.as_deref()),
            ("#ffffff", Some("#0b0d12"))
        );
        let p = v.iter().find(|v| v.name == "--primary").unwrap();
        assert_eq!(p.dark, None);
        let css = set_dark_var(TOKENS_CSS, "--primary", "#60a5fa");
        let css = set_dark_var(&css, "--bg", "#000000");
        let css = set_dark_var(&css, "--border", "");
        let v = vars(&css);
        let get = |n: &str| v.iter().find(|v| v.name == n).unwrap().clone();
        assert_eq!(get("--primary").dark.as_deref(), Some("#60a5fa"));
        assert_eq!(get("--primary").light, "#2563eb");
        assert_eq!(get("--bg").dark.as_deref(), Some("#000000"));
        assert_eq!(get("--border").dark, None);
        assert_eq!(get("--border").light, "#e2e8f0");
        let plain = set_dark_var(":root {\n  --x: red;\n}\n", "--x", "blue");
        assert_eq!(vars(&plain)[0].dark.as_deref(), Some("blue"));
        let renamed = rename_var(&css, "--primary", "--brand");
        assert!(renamed.contains("--brand: #2563eb;"));
        assert!(renamed.contains("var(--brand) 12%"));
        assert!(renamed.contains("--primary-soft:"));
        let gone = remove_var(&css, "--primary");
        assert!(vars(&gone).iter().all(|v| v.name != "--primary"));
        assert!(gone.contains("--primary-soft"));
    }

    #[test]
    fn arrange_follows_the_flow() {
        let mut canvas = Canvas::default();
        for id in ["detail", "list", "login", "about"] {
            canvas.screens.push(Screen {
                id: id.into(),
                ..Default::default()
            });
        }
        let links = vec![
            ("login".to_string(), "list".to_string()),
            ("list".to_string(), "detail".to_string()),
        ];
        canvas.arrange(&links, &|_| 844.);
        let x = |id: &str| canvas.screen(id).unwrap().x;
        assert!(x("login") < x("list") && x("list") < x("detail"));
        assert_eq!(x("about"), x("login"));
        assert!(canvas.screen("about").unwrap().y > canvas.screen("login").unwrap().y);
    }
}
