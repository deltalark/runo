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
  // Markup as it's copied or shown, without the layer ids that tie it to this screen.
  const noRid = (h) => h.replace(/ data-rid="[^"]*"/g, '');
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
    'box-shadow', 'flex-wrap', 'position', 'top', 'left', 'right', 'bottom', 'z-index', 'background-image',
    'fill', 'stroke', 'stroke-width', 'filter', 'object-fit', 'object-position', 'object-view-box',
    'background-size', 'background-position', 'background-repeat', 'rotate', 'overflow'];
  // The one icon an element is or holds, as an icon button or a tab does.
  const iconEl = (el) => {
    const all = el.matches('.icon') ? [el] : Array.from(el.querySelectorAll('.icon'));
    return all.length === 1 ? all[0] : null;
  };
  const iconOf = (el) => {
    const i = iconEl(el);
    const c = i && Array.from(i.classList).find((c) => c.startsWith('i-'));
    return c ? c.slice(2) : null;
  };
  // The styles written for an element, from the rules that match it and its own style,
  // as a developer would copy them; the variables they use, and its markup.
  // Declarations as written, shorthands kept whole: split the text at semicolons that
  // aren't inside brackets.
  const written = (style) => {
    const out = [];
    let depth = 0, from = 0;
    const t = style.cssText;
    const take = (end) => {
      const d = t.slice(from, end), i = d.indexOf(':');
      if (i > 0) out.push([d.slice(0, i).trim(), d.slice(i + 1).trim()]);
      from = end + 1;
    };
    for (let i = 0; i < t.length; i++) {
      if (t[i] === '(') depth++;
      else if (t[i] === ')') depth--;
      else if (t[i] === ';' && depth === 0) take(i);
    }
    take(t.length);
    return out;
  };
  // Linked style sheets can't be read from a file page, so their text comes along.
  const code = (el, text) => {
    const decls = new Map();
    const visit = (rules) => {
      for (const r of rules) {
        if (!r.selectorText) {
          if (r.cssRules && (!r.media || matchMedia(r.media.mediaText).matches)) visit(r.cssRules);
          continue;
        }
        if (/^\s*(\*|html\b|body\b|:root\b)/.test(r.selectorText) && !el.matches('body')) continue;
        let hit = false;
        try { hit = el.matches(r.selectorText); } catch (e) { hit = false; }
        if (!hit) continue;
        for (const [k, v] of written(r.style)) if (!k.startsWith('--')) decls.set(k, v);
      }
    };
    for (const sheet of document.styleSheets) { try { visit(sheet.cssRules); } catch (e) {} }
    if (text) { try { const s = new CSSStyleSheet(); s.replaceSync(text); visit(s.cssRules); } catch (e) {} }
    for (const [k, v] of written(el.style)) decls.set(k, v);
    const css = Array.from(decls, ([k, v]) => ({ k, v }));
    const root = getComputedStyle(document.documentElement);
    const names = new Set();
    for (const d of css) for (const m of d.v.matchAll(/var\((--[\w-]+)/g)) names.add(m[1]);
    const vars = Array.from(names, (k) => ({ k, v: root.getPropertyValue(k).trim() }));
    let html = noRid(el.outerHTML);
    if (html.length > 4000) html = html.slice(0, 4000) + '\n…';
    return { css, vars, html };
  };
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
      ease: el.dataset.ease || null, trigger: el.dataset.trigger || null,
      delay: el.dataset.delay || null,
      children: kids(el).length,
      component: el.dataset.component || null, variant: el.dataset.variant || null,
      name: el.dataset.name || null, locked: el.hasAttribute('data-locked'),
      icon: iconOf(el), src: el.getAttribute('src'),
      show: el.dataset.show || null,
    };
  };
  const tree = () => {
    const out = [];
    const walk = (el, depth) => {
      for (const c of kids(el)) {
        if (out.length >= 600) return;
        const t = ownText(c);
        out.push({ path: pathOf(c), label: label(c), depth, hidden: c.hidden,
          text: t ? t.slice(0, 40) : null, component: c.dataset.component || null,
          name: c.dataset.name || null, locked: c.hasAttribute('data-locked'), id: c.id || null });
        // A drawing is one layer: what it's drawn with stays out of the list.
        if (!(c instanceof SVGElement)) walk(c, depth + 1);
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
      if (el.closest('[data-locked]') || el.ownerSVGElement) continue;
      const path = pathOf(el);
      if (!path) continue;
      const row = (cs.display.includes('flex') && cs.flexDirection.startsWith('row'))
        || cs.display.includes('grid') || cs.display === 'inline';
      // What it's called, so Smart animate can find it on the next screen.
      const t = ownText(el);
      const key = el.dataset.name || (el.id ? '#' + el.id : null)
        || (t ? el.tagName.toLowerCase() + ':' + t.slice(0, 60) : null);
      out.push({ path, rect: rect(el), row, tag: el.tagName.toLowerCase(), key });
    }
    return out;
  };
  const links = () => Array.from(document.querySelectorAll('[data-goto]'))
    .map((el) => ({ path: pathOf(el), goto: el.dataset.goto.replace(/\.html$/, ''), rect: rect(el),
      transition: el.dataset.transition || '', speed: el.dataset.speed || '',
      ease: el.dataset.ease || '', trigger: el.dataset.trigger || '',
      delay: Number(el.dataset.delay) || 0 }))
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
    'data-speed', 'data-rid'];
  const keep = (old, n) => {
    const a = [old, ...old.querySelectorAll('*')];
    const b = [n, ...n.querySelectorAll('*')];
    if (a.length === b.length && a.every((e, i) => e.tagName === b[i].tagName)) {
      a.forEach((e, i) => {
        for (const k of OWN) if (e.hasAttribute(k)) b[i].setAttribute(k, e.getAttribute(k));
      });
    }
    if (old.dataset.rid) n.dataset.rid = old.dataset.rid;
    const ta = texts(old);
    const tb = texts(n);
    if (ta.length === tb.length) ta.forEach((t, i) => { tb[i].data = t.data; });
  };
  const instances = (id) => Array.from(document.querySelectorAll('[data-component]'))
    .filter((el) => el.dataset.component === id);
  // An element's own style as written, declaration by declaration, keeping shorthands set
  // to a variable whole.
  const ownStyle = (el) => {
    const out = [];
    let depth = 0, quote = null, cur = '';
    for (const c of el.getAttribute('style') || '') {
      if (quote) { if (c === quote) quote = null; }
      else if (c === '"' || c === "'") quote = c;
      else if (c === '(') depth++;
      else if (c === ')') depth--;
      else if (c === ';' && depth === 0) { out.push(cur); cur = ''; continue; }
      cur += c;
    }
    out.push(cur);
    return out.map((d) => {
      const i = d.indexOf(':');
      return i < 0 ? null : [d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim()];
    }).filter((d) => d && d[0] && d[1]);
  };
  // What stays on an element when its look becomes a style: where it is and how big.
  const PLACE = /^(width|height|min-|max-|margin|top|left|right|bottom|inset|position|z-index|flex$|flex-grow|flex-shrink|flex-basis|align-self|order|grid-column|grid-row)/;
  const ops = {
    text: (el, v) => { el.textContent = v; },
    // Give an element the style `cls` in place of any of `drop`, and let go of its own
    // values for what the style sets, so the style shows.
    look: (el, cls, props, drop) => {
      for (const c of drop) if (c !== cls) el.classList.remove(c);
      if (cls) el.classList.add(cls);
      for (const k of props) el.style.removeProperty(k);
      if (!el.getAttribute('style')) el.removeAttribute('style');
      if (!el.getAttribute('class')) el.removeAttribute('class');
    },
    // Take an element out of the style `cls`, keeping how it looks: what the style set
    // becomes its own, under anything it set itself.
    detach: (el, cls, decls) => {
      el.classList.remove(cls);
      const own = new Set(ownStyle(el).map((d) => d[0]));
      for (const [k, v] of decls) if (!own.has(k)) el.style.setProperty(k, v);
      if (!el.getAttribute('class')) el.removeAttribute('class');
    },
    // Where an element sits: in the flow, floating over what holds it where it is now,
    // stuck to the top as the screen scrolls, or pinned to the screen.
    position: (el, mode) => {
      for (const k of ['position', 'top', 'left', 'right', 'bottom', 'z-index']) el.style.removeProperty(k);
      const parent = el.parentElement;
      const wide = el.getBoundingClientRect().width;
      if (mode === 'absolute' && parent) {
        if (getComputedStyle(parent).position === 'static') parent.style.setProperty('position', 'relative');
        const r = el.getBoundingClientRect();
        const pr = parent.getBoundingClientRect();
        el.style.setProperty('position', 'absolute');
        el.style.setProperty('left', Math.round(r.left - pr.left - parent.clientLeft) + 'px');
        el.style.setProperty('top', Math.round(r.top - pr.top - parent.clientTop) + 'px');
        el.style.setProperty('z-index', '1');
      } else if (mode === 'sticky') {
        el.style.setProperty('position', 'sticky');
        el.style.setProperty('top', '0px');
        el.style.setProperty('z-index', '2');
      } else if (mode === 'fixed') {
        const r = el.getBoundingClientRect();
        el.style.setProperty('position', 'fixed');
        el.style.setProperty('left', Math.round(r.left) + 'px');
        el.style.setProperty('top', Math.round(r.top) + 'px');
        el.style.setProperty('z-index', '3');
      }
      // Out of the flow it would shrink to its content, so it keeps the width it had.
      if ((mode === 'absolute' || mode === 'fixed') && !el.style.width) {
        el.style.setProperty('width', Math.round(wide) + 'px');
      }
      if (!el.getAttribute('style')) el.removeAttribute('style');
    },
    // Line it up in what holds it, as design tools' align buttons do: a floating element
    // moves to that edge or the middle; one in an auto layout sits there across the layout,
    // or is pushed there along it by auto margins.
    align: (el, side) => {
      const horiz = side === 'left' || side === 'hcenter' || side === 'right';
      const how = getComputedStyle(el).position;
      if (how === 'absolute' || how === 'fixed') {
        const p = how === 'fixed' ? null : el.offsetParent || el.parentElement;
        const r = el.getBoundingClientRect();
        const w = p ? p.clientWidth : innerWidth;
        const h = p ? p.clientHeight : innerHeight;
        const put = (k, v, other) => {
          el.style.setProperty(k, Math.round(v) + 'px');
          el.style.removeProperty(other);
        };
        if (side === 'left') put('left', 0, 'right');
        else if (side === 'right') put('right', 0, 'left');
        else if (side === 'hcenter') put('left', (w - r.width) / 2, 'right');
        else if (side === 'top') put('top', 0, 'bottom');
        else if (side === 'bottom') put('bottom', 0, 'top');
        else put('top', (h - r.height) / 2, 'bottom');
        return;
      }
      const ps = el.parentElement ? getComputedStyle(el.parentElement) : null;
      const flex = !!ps && ps.display.includes('flex');
      const row = flex && !ps.flexDirection.startsWith('column');
      const at = { left: 'flex-start', top: 'flex-start', hcenter: 'center', vcenter: 'center',
        right: 'flex-end', bottom: 'flex-end' }[side];
      if (flex && horiz !== row) {
        el.style.setProperty('align-self', at);
      } else if (horiz || flex) {
        const [a, b] = horiz ? ['margin-left', 'margin-right'] : ['margin-top', 'margin-bottom'];
        const auto = (k, on) => on ? el.style.setProperty(k, 'auto') : el.style.removeProperty(k);
        auto(a, at !== 'flex-start');
        auto(b, at !== 'flex-end');
      }
      if (!el.getAttribute('style')) el.removeAttribute('style');
    },
    // Which edges of what holds it a floating or pinned element keeps its distance to, so
    // it stays in its corner when that changes size. Sticky ones stick at the top or bottom.
    anchor: (el, side) => {
      const other = { left: 'right', right: 'left', top: 'bottom', bottom: 'top' }[side];
      if (!other) return;
      const set = (v) => {
        el.style.setProperty(side, Math.round(v) + 'px');
        el.style.removeProperty(other);
      };
      const how = getComputedStyle(el).position;
      if (how === 'sticky') return set(0);
      const r = el.getBoundingClientRect();
      const p = how === 'fixed' ? null : el.offsetParent || el.parentElement;
      const box = p ? p.getBoundingClientRect() : { left: 0, top: 0 };
      const w = p ? p.clientWidth : innerWidth;
      const h = p ? p.clientHeight : innerHeight;
      const x = r.left - box.left - (p ? p.clientLeft : 0);
      const y = r.top - box.top - (p ? p.clientTop : 0);
      set({ left: x, right: w - x - r.width, top: y, bottom: h - y - r.height }[side]);
    },
    // A drawing's new outline: its path data and size, moved by (dx, dy).
    reshape: (el, dx, dy, w, h, d) => {
      const q = el.querySelector('path');
      if (q) q.setAttribute('d', d);
      el.setAttribute('width', w); el.setAttribute('height', h);
      if (el.style.width) el.style.width = w + 'px';
      if (el.style.height) el.style.height = h + 'px';
      el.setAttribute('viewBox', '0 0 ' + w + ' ' + h);
      const move = (k, by) => { const v = parseFloat(el.style[k]); el.style[k] = Math.round((isNaN(v) ? 0 : v) + by) + 'px'; };
      if (el.style.right && !el.style.left) move('right', -dx); else move('left', dx);
      if (el.style.bottom && !el.style.top) move('bottom', -dy); else move('top', dy);
    },
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
    lock: (el) => { el.toggleAttribute('data-locked'); },
    // A tap on `el` opens or closes the overlay at `p`, which gets an id to be named by
    // and starts hidden.
    showTarget: (el, p) => {
      const o = at(p);
      if (!o) return;
      if (!o.id) {
        let n = 1;
        while (document.getElementById('overlay-' + n)) n++;
        o.id = 'overlay-' + n;
      }
      el.dataset.show = '#' + o.id;
      if (!o.contains(el)) o.hidden = true;
    },
    icon: (el, name) => {
      const i = iconEl(el);
      if (!i) return;
      for (const c of Array.from(i.classList)) if (c.startsWith('i-')) i.classList.remove(c);
      i.classList.add('i-' + name);
    },
    // A picture: an <img> shows it, anything else is filled with it. An empty url takes
    // it away.
    picture: (el, url) => {
      if (el.tagName === 'IMG') { if (url) el.setAttribute('src', url); else el.removeAttribute('src'); return; }
      if (url) {
        el.style.setProperty('background-image', 'url("' + url + '")');
        el.style.setProperty('background-size', 'cover');
        el.style.setProperty('background-position', 'center');
      } else {
        for (const k of ['background-image', 'background-size', 'background-position']) el.style.removeProperty(k);
        if (!el.getAttribute('style')) el.removeAttribute('style');
      }
    },
    // A text style is a class: the one given replaces any of `all`, and the type set on the
    // element itself goes, so the style shows.
    textStyle: (el, cls, all) => {
      for (const c of all) el.classList.remove(c);
      if (cls) el.classList.add(cls);
      for (const k of ['font-size', 'font-weight', 'line-height', 'letter-spacing']) el.style.removeProperty(k);
      if (!el.getAttribute('style')) el.removeAttribute('style');
      if (!el.getAttribute('class')) el.removeAttribute('class');
    },
    remove: (el) => { el.remove(); },
    duplicate: (el) => { el.after(el.cloneNode(true)); },
    // A copy dropped before or after the element at `target`, as Alt-drag leaves one.
    copyTo: (el, target, after) => {
      const t = at(target);
      if (!t || el.contains(t)) return;
      const c = el.cloneNode(true);
      if (after) t.after(c); else t.before(c);
    },
    // A copy just after the element, restyled by `map`: Alt-drag on a floating layer.
    copyStyled: (el, map) => {
      const c = el.cloneNode(true);
      el.after(c);
      ops.styles(c, map);
    },
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
    // An agent's new layers: never with an id copied from another layer.
    insertNew: (el, s, inside) => ops.insert(el, noRid(s), inside),
    moveBy: (el, sel, after) => {
      const t = document.querySelector(sel);
      if (!t || t === el || el.contains(t)) return;
      if (after) t.after(el); else t.before(el);
    },
    swapTo: (el, html) => {
      const n = parse(html);
      if (!n) return;
      keep(el, n);
      el.replaceWith(n);
    },
  };
  // The layers an agent names by a CSS selector, as Design's own layers: not Design's
  // helpers, and not inside a locked layer.
  const matching = (sel) => Array.from(document.querySelectorAll(sel))
    .filter((el) => !SKIP.has(el.tagName) && el !== document.documentElement);
  return {
    pick: (x, y) => {
      // A point below the fold is scrolled into view first: only what shows can be hit.
      if (y - scrollY < 0 || y - scrollY >= innerHeight) scrollTo(0, Math.max(0, y - innerHeight / 2));
      let el = document.elementFromPoint(x - scrollX, y - scrollY);
      while (el && el.ownerSVGElement) el = el.ownerSVGElement;
      // A locked element can't be picked on the canvas: what holds it is picked instead.
      while (el && el.closest('[data-locked]')) el = el.closest('[data-locked]').parentElement;
      if (!el || el === document.documentElement || el === document.body) return null;
      return info(el);
    },
    info: (p) => { const el = at(p); return el ? info(el) : null; },
    code: (p, text) => { const el = at(p); return el ? code(el, text) : null; },
    // Put a drawn shape at a point on the page, in the box that holds that spot, floating
    // where it was drawn. Gives back its path.
    // Each shape's outline as Design draws it: an svg's path data and where its units sit
    // on the page, or a box with its corner radius; and the first one's fill and stroke.
    // The layer at `p` (or the whole screen) as a tree for writing it as code: each
    // element's tag, attributes and what's inside, and how it's laid out and styled; and
    // the variants of `comps` ({id: {variant: html}}) as trees, for their code.
    codeTree: (p, comps) => {
      const STY = ['display', 'flex-direction', 'flex-wrap', 'row-gap', 'column-gap',
        'align-items', 'justify-content', 'align-self', 'flex-grow', 'padding-top',
        'padding-right', 'padding-bottom', 'padding-left', 'background-color',
        'background-image', 'color', 'font-size', 'font-weight', 'font-style', 'line-height',
        'text-align', 'text-transform', 'letter-spacing', 'border-top-left-radius',
        'border-top-width', 'border-top-style', 'border-top-color', 'box-shadow', 'opacity',
        'position', 'left', 'top', 'grid-template-columns', 'object-fit', 'overflow',
        'text-decoration-line', 'visibility'];
      const nodeOf = (el, styled) => {
        const kids = [];
        for (const c of el.childNodes) {
          if (c.nodeType === 3) {
            const t = c.textContent.replace(/\s+/g, ' ');
            if (t.trim()) kids.push({ t: 'text', text: t });
          } else if (c.nodeType === 1 && !SKIP.has(c.tagName)) {
            kids.push(nodeOf(c, styled));
          }
        }
        const out = { t: 'el', tag: el.tagName.toLowerCase(), kids,
          attrs: Array.from(el.attributes, (a) => [a.name, a.value]) };
        if (styled && el.getBoundingClientRect) {
          const cs = getComputedStyle(el), r = el.getBoundingClientRect();
          out.s = {};
          for (const k of STY) out.s[k] = cs.getPropertyValue(k);
          out.w = r.width; out.h = r.height;
          out.shown = cs.display !== 'none' && !el.hidden;
          const par = el.parentElement;
          if (par) {
            const ps = getComputedStyle(par), pr = par.getBoundingClientRect();
            const inner = pr.width - parseFloat(ps.paddingLeft) - parseFloat(ps.paddingRight);
            out.fill = Math.abs(r.width - inner) < 1.5;
          }
          out.sized = !!(el.style.width || el.style.height);
        }
        return out;
      };
      const el = p && p.length ? at(p) : document.body;
      if (!el) return null;
      const parsed = {};
      for (const [id, vs] of Object.entries(comps || {})) {
        parsed[id] = {};
        for (const [v, html] of Object.entries(vs)) {
          const t = document.createElement('template');
          t.innerHTML = html;
          const first = t.content.firstElementChild;
          if (first) parsed[id][v] = nodeOf(first, false);
        }
      }
      return { node: nodeOf(el, true), comps: parsed };
    },
    // Give every layer an id of its own that stays with it through edits, so the code built
    // from it can name it: ones without, and copies that came with another's, get a new one.
    ensureIds: () => {
      const seen = new Set();
      let changed = 0;
      const fresh = () => {
        let id;
        do id = Math.random().toString(36).slice(2, 10); while (seen.has(id) || id.length < 6);
        return id;
      };
      for (const el of document.body.querySelectorAll('*')) {
        if (SKIP.has(el.tagName) || el.closest('svg') !== null && el.tagName !== 'svg') continue;
        const id = el.dataset.rid;
        if (id && !seen.has(id)) { seen.add(id); continue; }
        const n = fresh();
        seen.add(n);
        el.dataset.rid = n;
        changed++;
      }
      return changed || null;
    },
    // Link a stylesheet into the page unless it's linked already.
    useSheet: (href) => {
      const have = Array.from(document.querySelectorAll('link[rel="stylesheet"]'));
      if (have.some((l) => l.getAttribute('href') === href)) return false;
      const l = document.createElement('link');
      l.rel = 'stylesheet';
      l.href = href;
      document.head.appendChild(l);
      return true;
    },
    // Show only the elements at `ps`, on a page of colour `bg`; with no `bg`, everything again.
    solo: (ps, bg) => {
      const html = document.documentElement, body = document.body;
      if (bg) html.style.setProperty('background', bg, 'important');
      else html.style.removeProperty('background');
      body.style.visibility = bg ? 'hidden' : '';
      for (const p of ps) { const el = at(p); if (el) el.style.visibility = bg ? 'visible' : ''; }
      if (!html.getAttribute('style')) html.removeAttribute('style');
      if (!body.getAttribute('style')) body.removeAttribute('style');
      return true;
    },
    // Hide the elements at `ps`, or show them again, to draw a screen without them.
    hide: (ps, on) => {
      for (const p of ps) { const el = at(p); if (el) el.style.visibility = on ? 'hidden' : ''; }
      return true;
    },
    outlines: (ps) => ps.map((p) => {
      const el = at(p);
      if (!el) return null;
      const r = rect(el), cs = getComputedStyle(el);
      if (el.tagName.toLowerCase() === 'svg') {
        const vb = el.viewBox && el.viewBox.baseVal;
        const sx = vb && vb.width ? r.w / vb.width : 1, sy = vb && vb.height ? r.h / vb.height : 1;
        const d = Array.from(el.querySelectorAll('path')).map((q) => q.getAttribute('d') || '').join(' ');
        return { kind: 'path', d, x: r.x - (vb ? vb.x * sx : 0), y: r.y - (vb ? vb.y * sy : 0), sx, sy, rect: r,
          fill: el.style.fill || cs.fill, stroke: el.style.stroke || 'none' };
      }
      const radius = parseFloat(cs.borderTopLeftRadius) || 0;
      const round = cs.borderTopLeftRadius.endsWith('%') && parseFloat(cs.borderTopLeftRadius) >= 50;
      return { kind: round ? 'ellipse' : 'rect', radius: Math.min(radius, r.w / 2, r.h / 2), rect: r,
        fill: el.style.background || el.style.backgroundColor || cs.backgroundColor, stroke: 'none' };
    }),
    // Put `s` where the shapes at `ps` were, in the first one's box, floating at page point
    // (x, y), and take the shapes away. Gives back its path.
    replaceShapes: (ps, x, y, s) => {
      const n = parse(s);
      const els = ps.map(at).filter(Boolean);
      if (!n || !els.length) return null;
      const parent = els[0].parentElement;
      if (getComputedStyle(parent).position === 'static') parent.style.position = 'relative';
      const r = parent.getBoundingClientRect();
      n.style.position = 'absolute';
      n.style.left = Math.round(x - scrollX - r.left - parent.clientLeft + parent.scrollLeft) + 'px';
      n.style.top = Math.round(y - scrollY - r.top - parent.clientTop + parent.scrollTop) + 'px';
      els[0].before(n);
      for (const e of els) e.remove();
      return pathOf(n);
    },
    drawShape: (x, y, s) => {
      const n = parse(s);
      if (!n) return null;
      if (y - scrollY < 0 || y - scrollY >= innerHeight) scrollTo(0, Math.max(0, y - innerHeight / 2));
      const LEAF = new Set(['IMG', 'INPUT', 'TEXTAREA', 'SELECT', 'BUTTON', 'P', 'SPAN', 'A',
        'LABEL', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'I', 'B', 'STRONG', 'EM', 'SMALL', 'VIDEO']);
      let el = document.elementFromPoint(x - scrollX, y - scrollY) || document.body;
      // Not inside a component's copy, a locked layer, text, a picture or a drawing.
      for (;;) {
        const fixed = el.closest('[data-component], [data-locked]');
        if (fixed && fixed !== document.body) { el = fixed.parentElement; continue; }
        if (el instanceof SVGElement || LEAF.has(el.tagName) || el.matches('.icon')) { el = el.parentElement; continue; }
        break;
      }
      if (!el || el === document.documentElement) el = document.body;
      if (getComputedStyle(el).position === 'static') el.style.position = 'relative';
      const r = el.getBoundingClientRect();
      n.style.position = 'absolute';
      n.style.left = Math.round(x - scrollX - r.left - el.clientLeft + el.scrollLeft) + 'px';
      n.style.top = Math.round(y - scrollY - r.top - el.clientTop + el.scrollTop) + 'px';
      el.appendChild(n);
      return pathOf(n);
    },
    // Put a part where it belongs on a screen with nothing picked: bars at the top, tab
    // bars, overlays and floating buttons at the end, anything else at the end of the
    // screen's content. Gives back its path.
    place: (s) => {
      const n = parse(s);
      if (!n) return null;
      const frame = n.matches('.tabbar, .overlay, .fab, .toast');
      const main = document.querySelector('main.screen, .screen, main');
      if (n.matches('header.bar, .bar')) document.body.prepend(n);
      else if (frame || !main) {
        const tab = document.querySelector('.tabbar');
        if (tab && !n.matches('.tabbar')) tab.before(n); else document.body.appendChild(n);
      } else main.appendChild(n);
      return pathOf(n);
    },
    tree, links, html, boxes,
    size: () => ({ w: document.documentElement.scrollWidth, h: document.documentElement.scrollHeight }),
    op: (p, name, ...args) => {
      const el = p.length ? at(p) : document.body;
      if (!el) return null;
      ops[name](el, ...args);
      return true;
    },
    infos: (ps) => ps.map(at).filter(Boolean).map(info),
    // Every layer's inline style, by path, but a locked layer's.
    inlineStyles: () => Array.from(document.body.querySelectorAll('[style]'))
      .filter((el) => !SKIP.has(el.tagName) && !el.closest('[data-locked]'))
      .map((el) => [pathOf(el), el.getAttribute('style')])
      .filter(([p]) => p),
    // Write each [path, style] given.
    setStyles: (list) => {
      for (const [p, s] of list) { const el = at(p); if (el) el.setAttribute('style', s); }
      return true;
    },
    // What an agent's selector matches, before changing it: how many, and each one's
    // component and variant; or why it can't be changed.
    find: (sel, scope) => {
      let els;
      try { els = matching(sel); } catch (e) { return { error: `"${sel}" isn't a CSS selector.` }; }
      if (!els.length) return { error: `Nothing on this screen matches "${sel}".` };
      const locked = els.filter((el) => el.closest('[data-locked]')).length;
      if (locked) return { error: `${locked} of the ${els.length} layers matching "${sel}" are locked by the person, so they stay as they are.` };
      if (scope) {
        const within = scope.map(at).filter(Boolean);
        const out = els.filter((el) => !within.some((w) => w.contains(el))).length;
        if (out) return { error: `${out} of the ${els.length} layers matching "${sel}" are outside what the person selected, and this ask is about the selection only. Narrow the selector to the selected layers or what's inside them.` };
      }
      return { n: els.length, components: els.map((el) => [el.dataset.component || null, el.dataset.variant || null]) };
    },
    // A CSS selector an agent can name each layer at `ps` by: its id, or its place.
    selectors: (ps) => ps.map(at).filter(Boolean).map((el) => {
      if (el.dataset.rid) return `[data-rid="${el.dataset.rid}"]`;
      const steps = [];
      for (let e = el; e && e !== document.body; e = e.parentElement) {
        steps.unshift(`${e.tagName.toLowerCase()}:nth-child(${Array.from(e.parentElement.children).indexOf(e) + 1})`);
      }
      return ['body', ...steps].join(' > ');
    }),
    // Run the operation `name` on every layer `sel` matches, last on the page first.
    agent: (sel, name, args) => {
      const els = matching(sel).filter((el) => !el.closest('[data-locked]'));
      if (!els.length || !ops[name]) return null;
      for (const el of els.slice().reverse()) ops[name](el, ...args);
      return { n: els.length, boxes: els.filter((el) => el.isConnected).map(rect) };
    },
    // Swap each matching instance for the HTML given for it, in page order.
    agentSwap: (sel, htmls) => {
      const els = matching(sel).filter((el) => !el.closest('[data-locked]'));
      const now = [];
      els.forEach((el, i) => {
        if (!htmls[i]) return;
        const next = el.nextSibling, parent = el.parentNode;
        ops.swapTo(el, htmls[i]);
        const n = next ? next.previousElementSibling : parent.lastElementChild;
        if (n) now.push(n);
      });
      return now.length ? { n: now.length, boxes: now.map(rect) } : null;
    },
    // Move how an element looks into the style `cls`, giving it that class: its own style
    // goes, but for where it is and how big. Gives back what moved, as [property, value].
    takeLook: (p, cls) => {
      const el = at(p);
      if (!el) return null;
      const all = ownStyle(el);
      const moved = all.filter(([k]) => !PLACE.test(k));
      el.removeAttribute('style');
      for (const [k, v] of all) if (PLACE.test(k)) el.style.setProperty(k, v);
      el.classList.add(cls);
      return moved;
    },
    // The same change to several elements, last on the page first, so changing one
    // doesn't move the ones still to come.
    // Line floating layers up evenly in a row or a column, whichever way they spread more:
    // their order kept, the space between each the average space between them now, and
    // lined up with the first across. Null when fewer than two of them float.
    tidy: (ps) => {
      const els = ps.map(at).filter((el) => el && getComputedStyle(el).position === 'absolute');
      if (els.length < 2) return null;
      const rs = els.map((el) => [el, el.getBoundingClientRect()]);
      const xs = rs.map(([, r]) => r.left + r.width / 2), ys = rs.map(([, r]) => r.top + r.height / 2);
      const row = Math.max(...xs) - Math.min(...xs) >= Math.max(...ys) - Math.min(...ys);
      const [a, z, size] = row ? ['left', 'right', 'width'] : ['top', 'bottom', 'height'];
      const cross = row ? 'top' : 'left';
      rs.sort((p, q) => p[1][a] - q[1][a]);
      const total = rs.reduce((n, [, r]) => n + r[size], 0);
      const span = rs[rs.length - 1][1][z] - rs[0][1][a];
      const gap = Math.max(0, Math.round((span - total) / (rs.length - 1)));
      let at0 = rs[0][1][a];
      const line = rs[0][1][cross];
      for (const [el, r] of rs) {
        const move = (k, by) => {
          const now = parseFloat(getComputedStyle(el)[k]) || 0;
          el.style.setProperty(k, Math.round(now + by) + 'px');
          el.style.removeProperty({ left: 'right', top: 'bottom' }[k]);
        };
        move(a, at0 - r[a]);
        move(cross, line - r[cross]);
        at0 += r[size] + gap;
      }
      return true;
    },
    opAll: (ps, name, ...args) => {
      for (const el of inOrder(ps).reverse()) ops[name](el, ...args);
      return true;
    },
    copy: (ps) => inOrder(ps).map((el) => noRid(el.outerHTML)),
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
      else if (el.dataset.rid) n.dataset.rid = el.dataset.rid;
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
    renameVariants: (id, map) => {
      let changed = 0;
      for (const el of instances(id)) {
        const to = map[el.dataset.variant];
        if (to) { el.dataset.variant = to; changed++; }
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
    pub ease: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub delay: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub children: usize,
    /// The component it's an instance of, and which variant.
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    /// The one icon it is or holds, by name ("heart" for `i-heart`).
    #[serde(default)]
    pub icon: Option<String>,
    /// An image's `src`.
    #[serde(default)]
    pub src: Option<String>,
    /// What a tap on it shows or hides (`data-show`).
    #[serde(default)]
    pub show: Option<String>,
    /// Its name in the layers panel (`data-name`), and whether it's locked.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub locked: bool,
    /// Every class it has.
    #[serde(default)]
    pub classes: Vec<String>,
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

    /// Whether new pieces go inside it rather than after it, unless the person says.
    pub(crate) fn is_container(&self) -> bool {
        self.can_hold() && (self.children > 0 || self.is_flex())
    }

    /// Whether pieces can go inside it at all, even while it's empty.
    pub(crate) fn can_hold(&self) -> bool {
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
        )
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
    /// What it was named in the layers panel (`data-name`).
    #[serde(default)]
    pub name: Option<String>,
    /// Locked (`data-locked`): it can't be picked on the canvas.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub id: Option<String>,
}

/// An element as code: the declarations written for it, the variables they use with their
/// values, and its markup.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Code {
    #[serde(default)]
    pub css: Vec<Decl>,
    #[serde(default)]
    pub vars: Vec<Decl>,
    #[serde(default)]
    pub html: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Decl {
    pub k: String,
    pub v: String,
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
    /// Its name, id or text: the same key on two screens is the same layer.
    #[serde(default)]
    pub key: Option<String>,
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

/// Where a copy lands when dropped before or after the element at `to`: nothing is taken
/// out, so only the place beside `to` matters.
pub(crate) fn copied_path(to: &Path, after: bool) -> Path {
    let mut t = to.clone();
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
    /// `data-ease`, how the move eases; empty for ease out.
    #[serde(default)]
    pub ease: String,
    /// `data-trigger` and `data-delay`: "delay" follows the link by itself after `delay`
    /// milliseconds; empty for a click.
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub delay: u64,
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
