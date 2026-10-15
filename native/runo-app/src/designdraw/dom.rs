//! What the canvas asks of a screen's page: find the element under a point, describe it,
//! list its layers and links, change it, and give the HTML back to save.
//!
//! Elements are named by their path of child indexes from `<body>`, which stays valid
//! between calls as long as nothing else changes the page.

use serde::Deserialize;
use serde_json::{json, Value};

/// An element's path of child indexes from `<body>`.
pub(crate) type Path = Vec<usize>;

/// Installed once per page before any other call; every call starts with it, so a reload
/// just installs it again.
pub(crate) const HELPER: &str = r#"
if (!window.__runo) window.__runo = (() => {
  const SKIP = new Set(['SCRIPT', 'STYLE', 'LINK', 'META', 'TITLE', 'TEMPLATE']);
  const kids = (el) => Array.from(el.children).filter((c) => !SKIP.has(c.tagName));
  const pathOf = (el) => {
    const p = [];
    while (el && el !== document.body) {
      const parent = el.parentElement;
      if (!parent) return null;
      p.unshift(kids(parent).indexOf(el));
      el = parent;
    }
    return el === document.body ? p : null;
  };
  const at = (p) => {
    let el = document.body;
    for (const i of p) { el = kids(el)[i]; if (!el) return null; }
    return el;
  };
  const label = (el) => {
    const cls = Array.from(el.classList).slice(0, 2).join('.');
    return el.tagName.toLowerCase() + (cls ? '.' + cls : '');
  };
  const ownText = (el) => kids(el).length === 0 ? el.textContent.trim() : null;
  const rect = (el) => {
    const r = el.getBoundingClientRect();
    return { x: r.x + scrollX, y: r.y + scrollY, w: r.width, h: r.height };
  };
  const STYLES = ['color', 'background-color', 'font-size', 'font-weight', 'text-align',
    'padding-top', 'padding-right', 'padding-bottom', 'padding-left', 'margin-top',
    'margin-bottom', 'gap', 'border-radius', 'display', 'flex-direction', 'align-items',
    'justify-content', 'width', 'height', 'opacity'];
  const info = (el) => {
    const cs = getComputedStyle(el);
    const computed = {};
    for (const k of STYLES) computed[k] = cs.getPropertyValue(k);
    const inline = {};
    for (let i = 0; i < el.style.length; i++) {
      const k = el.style[i];
      inline[k] = el.style.getPropertyValue(k);
    }
    return {
      path: pathOf(el), label: label(el), tag: el.tagName.toLowerCase(),
      text: ownText(el), classes: Array.from(el.classList), rect: rect(el),
      computed, inline, goto: el.dataset.goto || null, hidden: el.hidden,
      children: kids(el).length,
    };
  };
  const tree = () => {
    const out = [];
    const walk = (el, depth) => {
      for (const c of kids(el)) {
        if (out.length >= 600) return;
        const t = ownText(c);
        out.push({ path: pathOf(c), label: label(c), depth, hidden: c.hidden,
          text: t ? t.slice(0, 40) : null });
        walk(c, depth + 1);
      }
    };
    walk(document.body, 0);
    return out;
  };
  const links = () => Array.from(document.querySelectorAll('[data-goto]'))
    .map((el) => ({ path: pathOf(el), goto: el.dataset.goto.replace(/\.html$/, ''), rect: rect(el) }))
    .filter((l) => l.path);
  const html = () => '<!doctype html>\n' + document.documentElement.outerHTML + '\n';
  const parse = (s) => {
    const t = document.createElement('template');
    t.innerHTML = s.trim();
    return t.content.firstElementChild;
  };
  const ops = {
    text: (el, v) => { el.textContent = v; },
    style: (el, k, v) => { if (v) el.style.setProperty(k, v); else el.style.removeProperty(k);
      if (!el.getAttribute('style')) el.removeAttribute('style'); },
    attr: (el, k, v) => { if (v === null || v === '') el.removeAttribute(k); else el.setAttribute(k, v); },
    hide: (el) => { el.hidden = !el.hidden; },
    remove: (el) => { el.remove(); },
    duplicate: (el) => { el.after(el.cloneNode(true)); },
    move: (el, d) => {
      const sib = d < 0 ? el.previousElementSibling : el.nextElementSibling;
      if (sib) { if (d < 0) sib.before(el); else sib.after(el); }
    },
    moveTo: (el, target, after) => {
      const t = at(target);
      if (!t || t === el || el.contains(t)) return;
      if (after) t.after(el); else t.before(el);
    },
    insert: (el, s, inside) => {
      const n = parse(s);
      if (!n) return;
      if (inside) el.appendChild(n); else el.after(n);
    },
  };
  return {
    pick: (x, y) => {
      // A point below the fold is scrolled into view first: only what shows can be hit.
      if (y - scrollY < 0 || y - scrollY >= innerHeight) scrollTo(0, Math.max(0, y - innerHeight / 2));
      let el = document.elementFromPoint(x - scrollX, y - scrollY);
      if (!el || el === document.documentElement || el === document.body) return null;
      return info(el);
    },
    info: (p) => { const el = at(p); return el ? info(el) : null; },
    tree, links, html,
    size: () => ({ w: document.documentElement.scrollWidth, h: document.documentElement.scrollHeight }),
    op: (p, name, ...args) => {
      const el = p.length ? at(p) : document.body;
      if (!el) return null;
      ops[name](el, ...args);
      return true;
    },
  };
})();
"#;

/// A call on the helper, as an expression to evaluate.
pub(crate) fn call(method: &str, args: &[Value]) -> String {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    format!("{HELPER}\nwindow.__runo.{method}({})", args.join(", "))
}

pub(crate) fn pick(x: f32, y: f32) -> String {
    call("pick", &[json!(x), json!(y)])
}

pub(crate) fn info(path: &Path) -> String {
    call("info", &[json!(path)])
}

/// A change to the element at `path`.
pub(crate) fn op(path: &Path, name: &str, args: &[Value]) -> String {
    let mut all = vec![json!(path), json!(name)];
    all.extend_from_slice(args);
    call("op", &all)
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(crate) struct Box {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[cfg(test)]
impl Box {
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.w && y <= self.y + self.h
    }
}

/// An element as the inspector shows it.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Element {
    pub path: Path,
    pub label: String,
    pub tag: String,
    /// The text, when the element holds only text.
    pub text: Option<String>,
    pub rect: Box,
    /// Computed values of the styles the inspector edits.
    #[serde(default)]
    pub computed: std::collections::HashMap<String, String>,
    /// Styles set on the element itself.
    #[serde(default)]
    pub inline: std::collections::HashMap<String, String>,
    pub goto: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub children: usize,
}

impl Element {
    /// The value the inspector shows for a style: the element's own, else the computed.
    pub(crate) fn style(&self, key: &str) -> String {
        self.inline
            .get(key)
            .or_else(|| self.computed.get(key))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn is_flex(&self) -> bool {
        self.computed
            .get("display")
            .is_some_and(|d| d.contains("flex"))
    }

    /// Whether new pieces go inside it rather than after it.
    pub(crate) fn is_container(&self) -> bool {
        matches!(
            self.tag.as_str(),
            "main"
                | "section"
                | "div"
                | "ul"
                | "ol"
                | "nav"
                | "header"
                | "footer"
                | "aside"
                | "form"
        ) && (self.children > 0 || self.is_flex())
    }
}

/// One row of the layers panel.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Layer {
    pub path: Path,
    pub label: String,
    pub depth: usize,
    #[serde(default)]
    pub hidden: bool,
    pub text: Option<String>,
}

/// An element that opens another screen.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Link {
    pub goto: String,
    pub rect: Box,
}

/// The parent's path, or None at the top.
pub(crate) fn parent(path: &Path) -> Option<Path> {
    (!path.is_empty()).then(|| path[..path.len() - 1].to_vec())
}

/// A CSS colour as RGB, for `rgb(…)`, `rgba(…)` with any alpha, and `#rrggbb`.
#[cfg(test)]
pub(crate) fn rgb_of(css: &str) -> Option<u32> {
    let s = css.trim();
    if let Some(hex) = crate::archdraw::model::parse_hex(s) {
        return Some(hex);
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .trim_end_matches(')');
    let parts: Vec<f32> = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    if parts.len() < 3 || parts.get(3).is_some_and(|a| *a == 0.) {
        return None;
    }
    Some(((parts[0] as u32) << 16) | ((parts[1] as u32) << 8) | parts[2] as u32)
}

/// A pixel length as a number: `12px` gives 12.
#[cfg(test)]
pub(crate) fn px(css: &str) -> Option<f32> {
    css.trim().strip_suffix("px")?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_quote_their_arguments() {
        let js = op(&vec![0, 2], "text", &[json!("Say \"hi\"\n")]);
        assert!(js.ends_with(r#"window.__runo.op([0,2], "text", "Say \"hi\"\n")"#));
        assert!(pick(10., 20.5).ends_with("window.__runo.pick(10.0, 20.5)"));
    }

    #[test]
    fn elements_read_from_the_page() {
        let v = json!({
            "path": [1, 0], "label": "button.button.primary", "tag": "button", "text": "Go",
            "classes": ["button", "primary"], "rect": {"x": 1, "y": 2, "w": 30, "h": 10},
            "computed": {"display": "inline-flex", "color": "rgb(255, 255, 255)"},
            "inline": {"color": "var(--primary)"}, "goto": null, "hidden": false, "children": 0
        });
        let e: Element = serde_json::from_value(v).unwrap();
        assert_eq!(e.style("color"), "var(--primary)");
        assert!(e.is_flex());
        assert!(!e.is_container());
        assert!(e.rect.contains(10., 5.));
        assert_eq!(parent(&e.path), Some(vec![1]));
        assert_eq!(parent(&vec![]), None);
    }

    #[test]
    fn css_values_are_read() {
        assert_eq!(rgb_of("rgb(37, 99, 235)"), Some(0x2563eb));
        assert_eq!(rgb_of("rgba(0, 0, 0, 0)"), None);
        assert_eq!(rgb_of("#ff0000"), Some(0xff0000));
        assert_eq!(px("24px"), Some(24.));
        assert_eq!(px("normal"), None);
    }
}
