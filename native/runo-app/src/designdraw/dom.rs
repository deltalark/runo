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
    'justify-content', 'width', 'height', 'opacity', 'line-height', 'letter-spacing',
    'border-top-width', 'border-top-color', 'border-top-style', 'border-top-left-radius',
    'box-shadow', 'flex-wrap'];
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
      transition: el.dataset.transition || null, speed: el.dataset.speed || null,
      children: kids(el).length,
      component: el.dataset.component || null, variant: el.dataset.variant || null,
    };
  };
  const tree = () => {
    const out = [];
    const walk = (el, depth) => {
      for (const c of kids(el)) {
        if (out.length >= 600) return;
        const t = ownText(c);
        out.push({ path: pathOf(c), label: label(c), depth, hidden: c.hidden,
          text: t ? t.slice(0, 40) : null, component: c.dataset.component || null });
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
    .map((el) => ({ path: pathOf(el), goto: el.dataset.goto.replace(/\.html$/, ''), rect: rect(el),
      transition: el.dataset.transition || '', speed: el.dataset.speed || '' }))
    .filter((l) => l.path);
  const html = () => '<!doctype html>\n' + document.documentElement.outerHTML + '\n';
  const parse = (s) => {
    const t = document.createElement('template');
    t.innerHTML = s.trim();
    return t.content.firstElementChild;
  };
  const parseAll = (s) => {
    const t = document.createElement('template');
    t.innerHTML = s.trim();
    return Array.from(t.content.children);
  };
  // Paths in the order their elements appear on the page.
  const cmp = (a, b) => {
    for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] - b[i];
    return a.length - b.length;
  };
  const inOrder = (ps) => ps.slice().sort(cmp).map(at).filter(Boolean);
  // What an instance keeps when its component changes: its text, and the links and sources
  // of its elements, as long as it's still built the same way.
  const texts = (el) => {
    const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const out = [];
    while (w.nextNode()) if (w.currentNode.data.trim()) out.push(w.currentNode);
    return out;
  };
  const OWN = ['src', 'alt', 'href', 'placeholder', 'value', 'data-goto', 'data-transition',
    'data-speed'];
  const keep = (old, n) => {
    const a = [old, ...old.querySelectorAll('*')];
    const b = [n, ...n.querySelectorAll('*')];
    if (a.length === b.length && a.every((e, i) => e.tagName === b[i].tagName)) {
      a.forEach((e, i) => {
        for (const k of OWN) if (e.hasAttribute(k)) b[i].setAttribute(k, e.getAttribute(k));
      });
    }
    const ta = texts(old);
    const tb = texts(n);
    if (ta.length === tb.length) ta.forEach((t, i) => { tb[i].data = t.data; });
  };
  const instances = (id) => Array.from(document.querySelectorAll('[data-component]'))
    .filter((el) => el.dataset.component === id);
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
    infos: (ps) => ps.map(at).filter(Boolean).map(info),
    // The same change to several elements, last on the page first, so changing one
    // doesn't move the ones still to come.
    opAll: (ps, name, ...args) => {
      for (const el of inOrder(ps).reverse()) ops[name](el, ...args);
      return true;
    },
    copy: (ps) => inOrder(ps).map((el) => el.outerHTML),
    // Pieces of HTML added inside the element at `p` (at its end), or after it.
    paste: (p, htmls, inside) => {
      const el = p.length ? at(p) : document.body;
      if (!el) return null;
      const nodes = htmls.flatMap(parseAll);
      if (inside) el.append(...nodes); else el.after(...nodes);
      return nodes.length;
    },
    // Wrap elements in a new one, put where the first of them was.
    group: (ps, html) => {
      const els = inOrder(ps);
      const g = parse(html);
      if (!els.length || !g) return null;
      els[0].before(g);
      g.append(...els);
      return true;
    },
    // Make an element an instance of a component, or with no id, no longer one.
    mark: (p, id, variant) => {
      const el = at(p);
      if (!el) return null;
      if (id) { el.dataset.component = id; el.dataset.variant = variant; }
      else { delete el.dataset.component; delete el.dataset.variant; }
      return true;
    },
    // Put a variant's HTML in place of the instance at `p`, keeping its text with `own`.
    swap: (p, html, own) => {
      const el = at(p);
      const n = parse(html);
      if (!el || !n) return null;
      if (own) keep(el, n);
      el.replaceWith(n);
      return true;
    },
    // Bring every instance of a component up to date with its variants, by name.
    sync: (id, variants) => {
      const names = Object.keys(variants);
      let changed = 0;
      for (const el of instances(id)) {
        if (!el.isConnected || !names.length) continue;
        const n = parse(variants[el.dataset.variant] ?? variants[names[0]]);
        if (!n) continue;
        keep(el, n);
        if (n.outerHTML !== el.outerHTML) { el.replaceWith(n); changed++; }
      }
      return changed || null;
    },
    renameVariant: (id, from, to) => {
      let changed = 0;
      for (const el of instances(id)) {
        if (el.dataset.variant === from) { el.dataset.variant = to; changed++; }
      }
      return changed || null;
    },
    // Put each element's children where it was.
    ungroup: (ps) => {
      for (const el of inOrder(ps).reverse()) {
        if (el.children.length) el.replaceWith(...el.childNodes);
      }
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
    /// How the prototype moves to `goto`, and how fast: `data-transition`, `data-speed`.
    #[serde(default)]
    pub transition: Option<String>,
    #[serde(default)]
    pub speed: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub children: usize,
    /// The component it's an instance of, and which variant.
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
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
    /// The component it's an instance of.
    #[serde(default)]
    pub component: Option<String>,
}

/// An element on the page: where it sits, and whether its children run across.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Bx {
    pub path: Path,
    pub rect: Box,
    /// Its children run left to right (a flex row or a grid).
    #[serde(default)]
    pub row: bool,
    #[serde(default)]
    pub tag: String,
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

/// Paths in page order, without repeats or anything inside another of them: changing an
/// element changes what's inside it too.
pub(crate) fn outermost(paths: &[Path]) -> Vec<Path> {
    let mut out: Vec<Path> = paths
        .iter()
        .filter(|p| {
            !p.is_empty()
                && !paths
                    .iter()
                    .any(|q| q.len() < p.len() && p.starts_with(q) && !q.is_empty())
        })
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Where the copies land when each element is duplicated in place: just after its
/// original, with the copies before it pushing it along.
pub(crate) fn duplicated(paths: &[Path]) -> Vec<Path> {
    let paths = outermost(paths);
    paths
        .iter()
        .map(|p| {
            let mut out = p.clone();
            for m in &paths {
                let d = m.len() - 1;
                if m != p && p.len() > d && p[..d] == m[..d] && p[d] > m[d] {
                    out[d] += 1;
                }
            }
            if let Some(last) = out.last_mut() {
                *last += 1;
            }
            out
        })
        .collect()
}

/// The outermost elements lying wholly inside a box, as a drag across a screen picks them.
pub(crate) fn inside(boxes: &[Bx], area: &Box) -> Vec<Path> {
    let within: Vec<Path> = boxes
        .iter()
        .filter(|b| {
            !b.path.is_empty()
                && b.rect.x >= area.x
                && b.rect.y >= area.y
                && b.rect.x + b.rect.w <= area.x + area.w
                && b.rect.y + b.rect.h <= area.y + area.h
        })
        .map(|b| b.path.clone())
        .collect();
    outermost(&within)
}

/// Whether boxes sit side by side rather than one above another, to lay them out the way
/// they already are.
pub(crate) fn side_by_side(rects: &[Box]) -> bool {
    if rects.len() < 2 {
        return false;
    }
    let span = |lo: fn(&Box) -> f32, hi: fn(&Box) -> f32| {
        let a = rects.iter().map(lo).fold(f32::MAX, f32::min);
        let b = rects.iter().map(hi).fold(f32::MIN, f32::max);
        b - a
    };
    let wide = span(|r| r.x, |r| r.x + r.w);
    let tall = span(|r| r.y, |r| r.y + r.h);
    let widest = rects.iter().map(|r| r.w).fold(0., f32::max);
    let tallest = rects.iter().map(|r| r.h).fold(0., f32::max);
    wide - widest > tall - tallest
}

/// An element that opens another screen.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Link {
    pub goto: String,
    pub rect: Box,
    /// Where the linked element is on its screen.
    #[serde(default)]
    pub path: Path,
    /// `data-transition` and `data-speed`, empty for the platform's own.
    #[serde(default)]
    pub transition: String,
    #[serde(default)]
    pub speed: String,
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
    fn a_selection_keeps_its_outermost_elements_in_page_order() {
        let got = outermost(&[vec![2], vec![0, 1], vec![2, 3], vec![0, 1], vec![]]);
        assert_eq!(got, vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn copies_land_after_their_originals() {
        // Two siblings: the first copy pushes the second original along.
        assert_eq!(
            duplicated(&[vec![0, 1], vec![0, 3]]),
            vec![vec![0, 2], vec![0, 5]]
        );
        // A copy of an earlier element shifts a later one's container.
        assert_eq!(
            duplicated(&[vec![0], vec![1, 2]]),
            vec![vec![1], vec![2, 3]]
        );
    }

    #[test]
    fn a_drag_box_picks_what_is_wholly_inside_it() {
        let b = |path: Vec<usize>, x, y, w, h| Bx {
            path,
            rect: Box { x, y, w, h },
            ..Default::default()
        };
        let boxes = vec![
            b(vec![0], 0., 0., 390., 800.),
            b(vec![0, 0], 10., 10., 100., 40.),
            b(vec![0, 0, 0], 12., 12., 20., 20.),
            b(vec![0, 1], 10., 60., 100., 40.),
            b(vec![0, 2], 10., 110., 100., 40.),
        ];
        let area = Box {
            x: 0.,
            y: 0.,
            w: 200.,
            h: 105.,
        };
        assert_eq!(inside(&boxes, &area), vec![vec![0, 0], vec![0, 1]]);
    }

    #[test]
    fn layout_follows_how_things_sit() {
        let r = |x, y| Box {
            x,
            y,
            w: 50.,
            h: 20.,
        };
        assert!(side_by_side(&[r(0., 0.), r(60., 0.), r(120., 2.)]));
        assert!(!side_by_side(&[r(0., 0.), r(0., 30.), r(4., 60.)]));
        assert!(!side_by_side(&[r(0., 0.)]));
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
