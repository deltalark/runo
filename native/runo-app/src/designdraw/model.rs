//! Designed screens, kept as real HTML in `design/` so what's approved is what gets built.
//!
//! `screens/<id>.html` are the screens, `tokens.css` the theme every screen shares,
//! `components.css` the building blocks, `prototype.js` the links between screens, and
//! `canvas.toml` where each screen sits on the canvas and which device it's for.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The folder in the project that holds the design.
pub(crate) const FOLDER: &str = "design";
const SCREENS: &str = "screens";
const CANVAS: &str = "canvas.toml";
pub(crate) const TOKENS: &str = "tokens.css";
const COMPONENTS: &str = "components.css";
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
        (TOKENS, TOKENS_CSS),
        (COMPONENTS, COMPONENTS_CSS),
        (PROTOTYPE, PROTOTYPE_JS),
    ] {
        let file = dir.join(name);
        if !file.exists() {
            std::fs::write(file, text)?;
        }
    }
    Ok(())
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
pub(crate) const TEMPLATES: [(&str, &str); 7] = [
    ("Blank", "An empty screen with a title"),
    ("Sign in", "Email, password and a primary button"),
    ("List", "A header, search and a list of rows"),
    ("Detail", "A hero, facts and actions for one item"),
    ("Settings", "Grouped rows with switches"),
    ("Dashboard", "Stat cards and a recent list"),
    ("Empty state", "Nothing here yet, with one action"),
];

/// A new screen's HTML.
pub(crate) fn template(kind: &str, name: &str, device: Device) -> String {
    let body = match kind {
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
        "Detail" => r##"<header class="bar"><a class="link" href="#">Back</a></header>
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

/// Pieces the inspector can insert, as HTML using `components.css`.
pub(crate) const BLOCKS: [(&str, &str); 10] = [
    ("Heading", "<h2>Heading</h2>"),
    ("Text", "<p>Some text.</p>"),
    ("Button", "<button class=\"button primary\">Button</button>"),
    ("Input", "<label class=\"field\"><span>Label</span><input placeholder=\"Placeholder\"></label>"),
    ("Image", "<div class=\"image\"></div>"),
    ("Card", "<div class=\"card stack gap-sm\"><strong>Card title</strong><p class=\"muted\">Card text.</p></div>"),
    ("Row", "<div class=\"row\"><div class=\"avatar\">A</div><div class=\"grow\"><strong>Title</strong><p class=\"muted\">Subtitle</p></div></div>"),
    ("Stack", "<div class=\"stack gap-sm\"><p>First</p><p>Second</p></div>"),
    ("Divider", "<hr class=\"divider\">"),
    ("Tab bar", "<nav class=\"tabbar\"><a class=\"tab on\">Home</a><a class=\"tab\">Search</a><a class=\"tab\">Profile</a></nav>"),
];

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

const TOKENS_CSS: &str = r#"/* The design's theme. Every screen uses these, so changing one restyles them all. */
:root {
  --primary: #2563eb;
  --on-primary: #ffffff;
  --bg: #ffffff;
  --surface: #f4f5f7;
  --text: #0f172a;
  --muted: #5b6577;
  --border: #e2e8f0;
  --radius: 12px;
  --space: 8px;
  --font: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
}

@media (prefers-color-scheme: dark) {
  :root {
    --on-primary: #ffffff;
    --bg: #0b0d12;
    --surface: #161a22;
    --text: #e8ebf1;
    --muted: #8b95a7;
    --border: #262c38;
  }
}
"#;

const COMPONENTS_CSS: &str = r#"/* Building blocks the screens share. Prefer these classes and the tokens over raw values. */
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

const PROTOTYPE_JS: &str = r#"// Clicking an element with data-goto="other-screen" opens that screen, so the design
// can be clicked through in any browser.
document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-goto]');
  if (!el) return;
  e.preventDefault();
  location.href = el.dataset.goto.replace(/\.html$/, '') + '.html';
});
"#;

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
