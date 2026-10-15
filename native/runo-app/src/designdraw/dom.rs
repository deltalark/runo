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
  // Every element shown, with where it sits and which way its children run, so the canvas
  // can find what's under the pointer and where a dragged element would land.
  const boxes = () => {
    const out = [];
    for (const el of document.body.querySelectorAll('*')) {
      if (SKIP.has(el.tagName) || out.length >= 1500) continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || cs.visibility === 'hidden') continue;
      const path = pathOf(el);
      if (!path) continue;
      const row = (cs.display.includes('flex') && cs.flexDirection.startsWith('row'))
        || cs.display.includes('grid') || cs.display === 'inline';
      out.push({ path, rect: rect(el), row, tag: el.tagName.toLowerCase() });
    }
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
    styles: (el, map) => {
      for (const [k, v] of Object.entries(map)) {
        if (v) el.style.setProperty(k, v); else el.style.removeProperty(k);
      }
      if (!el.getAttribute('style')) el.removeAttribute('style');
    },
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
    tree, links, html, boxes,
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

/// An element on the page: where it sits, and whether its children run across.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Bx {
    pub path: Path,
    pub rect: Box,
    /// Its children run left to right (a flex row or a grid).
    #[serde(default)]
    pub row: bool,
}

/// The deepest element under a point, leaving out `skip` and what's inside it.
pub(crate) fn under<'a>(boxes: &'a [Bx], x: f32, y: f32, skip: Option<&Path>) -> Option<&'a Bx> {
    boxes
        .iter()
        .filter(|b| {
            x >= b.rect.x && y >= b.rect.y && x <= b.rect.x + b.rect.w && y <= b.rect.y + b.rect.h
        })
        .filter(|b| skip.is_none_or(|s| !b.path.starts_with(s)))
        .max_by_key(|b| b.path.len())
}

/// A measured distance: from, to, and how far in CSS pixels.
pub(crate) type Gap = ((f32, f32), (f32, f32), f32);

/// The distances to show between two elements, as lines with their length: to the edges of
/// the one around the other, else across the gaps between them.
pub(crate) fn spacing(a: &Box, b: &Box) -> Vec<Gap> {
    let inside = |i: &Box, o: &Box| {
        i.x >= o.x && i.y >= o.y && i.x + i.w <= o.x + o.w && i.y + i.h <= o.y + o.h
    };
    let mut out = Vec::new();
    let (i, o) = if inside(a, b) {
        (a, b)
    } else if inside(b, a) {
        (b, a)
    } else {
        // Side by side or one above the other: the gaps between them.
        let ym = (a.y.max(b.y) + (a.y + a.h).min(b.y + b.h)) / 2.;
        let ym = if a.y.max(b.y) <= (a.y + a.h).min(b.y + b.h) {
            ym
        } else {
            a.y + a.h / 2.
        };
        let xm = (a.x.max(b.x) + (a.x + a.w).min(b.x + b.w)) / 2.;
        let xm = if a.x.max(b.x) <= (a.x + a.w).min(b.x + b.w) {
            xm
        } else {
            a.x + a.w / 2.
        };
        if a.x + a.w <= b.x {
            out.push(((a.x + a.w, ym), (b.x, ym), b.x - a.x - a.w));
        } else if b.x + b.w <= a.x {
            out.push(((b.x + b.w, ym), (a.x, ym), a.x - b.x - b.w));
        }
        if a.y + a.h <= b.y {
            out.push(((xm, a.y + a.h), (xm, b.y), b.y - a.y - a.h));
        } else if b.y + b.h <= a.y {
            out.push(((xm, b.y + b.h), (xm, a.y), a.y - b.y - b.h));
        }
        return out;
    };
    let (xm, ym) = (i.x + i.w / 2., i.y + i.h / 2.);
    out.push(((o.x, ym), (i.x, ym), i.x - o.x));
    out.push(((i.x + i.w, ym), (o.x + o.w, ym), o.x + o.w - i.x - i.w));
    out.push(((xm, o.y), (xm, i.y), i.y - o.y));
    out.push(((xm, i.y + i.h), (xm, o.y + o.h), o.y + o.h - i.y - i.h));
    out.retain(|l| l.2 > 0.5);
    out
}

/// Where an element at `from` ends up after moving it before or after the one at `to`.
pub(crate) fn moved_path(from: &Path, to: &Path, after: bool) -> Path {
    let mut t = to.clone();
    // Taking the element out shifts its later siblings, and their insides, up by one.
    let depth = from.len() - 1;
    if t.len() > depth && t[..depth] == from[..depth] && t[depth] > from[depth] {
        t[depth] -= 1;
    }
    if after {
        if let Some(last) = t.last_mut() {
            *last += 1;
        }
    }
    t
}

/// An element that opens another screen.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Link {
    pub goto: String,
    pub rect: Box,
    /// Where the linked element is on its screen.
    #[serde(default)]
    pub path: Path,
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
    fn moves_land_where_dropped() {
        // Down past a later sibling: the gap it leaves shifts the target up.
        assert_eq!(moved_path(&vec![0, 1], &vec![0, 3], true), vec![0, 3]);
        assert_eq!(moved_path(&vec![0, 1], &vec![0, 3], false), vec![0, 2]);
        // Up before an earlier sibling.
        assert_eq!(moved_path(&vec![0, 3], &vec![0, 1], false), vec![0, 1]);
        // Into another container after the old place.
        assert_eq!(moved_path(&vec![1], &vec![2, 0], false), vec![1, 0]);
        assert_eq!(moved_path(&vec![2, 0], &vec![1], true), vec![2]);
    }

    #[test]
    fn the_deepest_element_is_under_the_pointer() {
        let b = |path: Vec<usize>, x, y, w, h| Bx {
            path,
            rect: Box { x, y, w, h },
            ..Default::default()
        };
        let boxes = vec![
            b(vec![0], 0., 0., 100., 100.),
            b(vec![0, 1], 10., 10., 20., 20.),
        ];
        assert_eq!(under(&boxes, 15., 15., None).unwrap().path, vec![0, 1]);
        assert_eq!(under(&boxes, 50., 50., None).unwrap().path, vec![0]);
        assert_eq!(
            under(&boxes, 15., 15., Some(&vec![0, 1])).unwrap().path,
            vec![0]
        );
        assert!(under(&boxes, 150., 15., None).is_none());
    }

    #[test]
    fn spacing_is_measured_inside_and_between() {
        let r = |x, y, w, h| Box { x, y, w, h };
        let inner = spacing(&r(10., 20., 30., 10.), &r(0., 0., 100., 50.));
        let d: Vec<f32> = inner.iter().map(|l| l.2).collect();
        assert_eq!(d, vec![10., 60., 20., 20.]);
        let beside = spacing(&r(0., 0., 10., 10.), &r(30., 0., 10., 10.));
        assert_eq!(beside.len(), 1);
        assert_eq!(beside[0].2, 20.);
        let below = spacing(&r(0., 0., 10., 10.), &r(0., 16., 10., 10.));
        assert_eq!(below[0].2, 6.);
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
