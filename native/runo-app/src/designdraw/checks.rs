//! Design checks: what makes a screen hard to read or use, found on the drawn page. The
//! page reports the facts (colours, sizes, what's clickable) and the rules live here.

use super::dom::{Box, Path};
use super::model::Device;
use serde::Deserialize;
use std::collections::HashSet;

/// Reads the facts the checks need from the page, as an expression.
pub(crate) const FACTS: &str = r#"
(() => {
  const SKIP = new Set(['SCRIPT', 'STYLE', 'LINK', 'META', 'TITLE', 'TEMPLATE', 'BR']);
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
  const rect = (el) => {
    const r = el.getBoundingClientRect();
    return { x: r.x + scrollX, y: r.y + scrollY, w: r.width, h: r.height };
  };
  // The colour behind an element: the first ancestor with a background colour. A picture
  // or gradient behind it can't be judged.
  const behind = (el) => {
    for (let e = el; e; e = e.parentElement) {
      const cs = getComputedStyle(e);
      if (cs.backgroundImage && cs.backgroundImage !== 'none') return null;
      const c = cs.backgroundColor;
      if (c && c !== 'transparent' && !/rgba\(.*,\s*0\)$/.test(c)) return c;
    }
    return 'rgb(255, 255, 255)';
  };
  const CLICK = 'a, button, input, select, textarea, [data-goto], [role="button"], [onclick]';
  const out = [];
  for (const el of document.body.querySelectorAll('*')) {
    if (SKIP.has(el.tagName) || out.length >= 800) continue;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    const r = rect(el);
    if (r.w === 0 || r.h === 0) continue;
    const own = Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim());
    const click = el.matches(CLICK) && !(el.parentElement && el.parentElement.closest(CLICK));
    const img = el.tagName === 'IMG';
    const raw = [];
    for (const k of ['color', 'background-color', 'background', 'border-color']) {
      const v = el.style.getPropertyValue(k);
      if (v && !v.includes('var(') && /#|rgb|hsl/.test(v)) raw.push(k);
    }
    if (!own && !click && !img && raw.length === 0) continue;
    const path = pathOf(el);
    if (!path) continue;
    const cls = Array.from(el.classList).slice(0, 2).join('.');
    out.push({
      path, label: el.tagName.toLowerCase() + (cls ? '.' + cls : ''), rect: r,
      text: own ? el.textContent.trim().slice(0, 40) : null,
      color: cs.color, background: own ? behind(el) : null,
      size: parseFloat(cs.fontSize) || 0, weight: parseInt(cs.fontWeight) || 400,
      click, inline: cs.display === 'inline', img,
      alt: img ? el.getAttribute('alt') : null, raw,
      goto: el.dataset.goto ? el.dataset.goto.replace(/\.html$/, '') : null,
    });
  }
  return { width: innerWidth, scroll: document.documentElement.scrollWidth, items: out };
})()
"#;

/// What the page reported.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(crate) struct Facts {
    pub width: f32,
    pub scroll: f32,
    pub items: Vec<Fact>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(crate) struct Fact {
    pub path: Path,
    pub label: String,
    pub rect: Box,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub size: f32,
    #[serde(default)]
    pub weight: u32,
    #[serde(default)]
    pub click: bool,
    #[serde(default)]
    pub inline: bool,
    #[serde(default)]
    pub img: bool,
    #[serde(default)]
    pub alt: Option<String>,
    #[serde(default)]
    pub raw: Vec<String>,
    #[serde(default)]
    pub goto: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Contrast,
    SmallText,
    TapTarget,
    OffTheme,
    NoAlt,
    Overflow,
    DeadLink,
}

impl Kind {
    /// Problems that stop someone using the screen, before ones of polish.
    pub(crate) fn serious(self) -> bool {
        matches!(
            self,
            Kind::Contrast | Kind::TapTarget | Kind::Overflow | Kind::DeadLink
        )
    }
}

/// One problem on a screen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Finding {
    pub kind: Kind,
    /// The element, or none for the whole screen.
    pub path: Option<Path>,
    pub rect: Option<Box>,
    pub text: String,
}

/// The most findings kept for one screen.
const MOST: usize = 40;

/// Check a screen.
pub(crate) fn review(facts: &Facts, device: Device, screens: &HashSet<String>) -> Vec<Finding> {
    let mut out = Vec::new();
    if facts.scroll > facts.width + 1. {
        out.push(Finding {
            kind: Kind::Overflow,
            path: None,
            rect: None,
            text: format!(
                "Wider than the screen: it scrolls sideways by {} px",
                (facts.scroll - facts.width).round()
            ),
        });
    }
    let min_tap = if device == Device::Desktop { 24. } else { 44. };
    let name = |f: &Fact| match &f.text {
        Some(t) if !t.is_empty() => format!("\u{201c}{}\u{201d}", short(t)),
        _ => f.label.clone(),
    };
    for f in &facts.items {
        let mut add = |kind: Kind, text: String| {
            out.push(Finding {
                kind,
                path: Some(f.path.clone()),
                rect: Some(f.rect.clone()),
                text,
            })
        };
        if f.text.is_some() {
            if let (Some(fg), Some(bg)) = (rgba(&f.color), f.background.as_deref().and_then(rgba)) {
                let ratio = contrast(over(fg, bg), bg);
                let large = f.size >= 24. || (f.size >= 18.66 && f.weight >= 700);
                let need = if large { 3. } else { 4.5 };
                if ratio < need {
                    add(
                        Kind::Contrast,
                        format!(
                            "{} is hard to read: contrast {:.1}:1, needs {need}:1",
                            name(f),
                            ratio
                        ),
                    );
                }
            }
            if f.size > 0. && f.size < 12. {
                add(
                    Kind::SmallText,
                    format!("{} is small text ({} px)", name(f), f.size.round()),
                );
            }
        }
        if f.click && !f.inline && (f.rect.w < min_tap - 0.5 || f.rect.h < min_tap - 0.5) {
            add(
                Kind::TapTarget,
                format!(
                    "{} is a small target ({}×{}, aim for {min_tap}×{min_tap})",
                    name(f),
                    f.rect.w.round(),
                    f.rect.h.round()
                ),
            );
        }
        if f.img && f.alt.is_none() {
            add(Kind::NoAlt, format!("{} has no alt text", f.label));
        }
        if !f.raw.is_empty() {
            // `background` set to a colour shows as both it and `background-color`.
            let raw: Vec<&str> = f
                .raw
                .iter()
                .map(String::as_str)
                .filter(|k| !(*k == "background" && f.raw.iter().any(|r| r == "background-color")))
                .collect();
            add(
                Kind::OffTheme,
                format!(
                    "{} sets its {} outside the theme",
                    name(f),
                    raw.join(" and ")
                ),
            );
        }
        if let Some(to) = f.goto.as_deref().filter(|g| !screens.contains(*g)) {
            add(
                Kind::DeadLink,
                format!("{} links to a screen that isn't there ({to})", name(f)),
            );
        }
    }
    out.sort_by_key(|f| !f.kind.serious());
    out.truncate(MOST);
    out
}

fn short(t: &str) -> String {
    let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 24 {
        format!("{}…", t.chars().take(23).collect::<String>())
    } else {
        t
    }
}

/// A CSS colour as red, green, blue (0 to 255) and alpha (0 to 1).
fn rgba(css: &str) -> Option<[f32; 4]> {
    let s = css.trim();
    if let Some(v) = crate::archdraw::model::parse_hex(s) {
        return Some([
            ((v >> 16) & 0xff) as f32,
            ((v >> 8) & 0xff) as f32,
            (v & 0xff) as f32,
            1.,
        ]);
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
    if parts.len() < 3 {
        return None;
    }
    Some([
        parts[0],
        parts[1],
        parts[2],
        parts.get(3).copied().unwrap_or(1.),
    ])
}

/// A see-through colour as it shows over another.
fn over(fg: [f32; 4], bg: [f32; 4]) -> [f32; 4] {
    let a = fg[3].clamp(0., 1.);
    [
        fg[0] * a + bg[0] * (1. - a),
        fg[1] * a + bg[1] * (1. - a),
        fg[2] * a + bg[2] * (1. - a),
        1.,
    ]
}

/// The WCAG contrast ratio between two colours, from 1 to 21.
fn contrast(a: [f32; 4], b: [f32; 4]) -> f32 {
    let lum = |c: [f32; 4]| {
        let ch = |v: f32| {
            let v = v / 255.;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The findings as a task for an agent to fix.
pub(crate) fn fix_task(screens: &[(String, String, Vec<Finding>)]) -> String {
    let mut out = String::from(
        "Fix these design check findings, keeping each screen's look and content otherwise. \
         Use the theme's CSS variables for colours and sizes. Elements are given as CSS \
         paths from <body>.\n",
    );
    for (id, name, found) in screens {
        if found.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{name} (design/screens/{id}.html):\n"));
        for f in found {
            let at = f
                .path
                .as_ref()
                .map(|p| {
                    let steps: Vec<String> =
                        p.iter().map(|i| format!(":nth-child({})", i + 1)).collect();
                    format!(" at `body > {}`", steps.join(" > "))
                })
                .unwrap_or_default();
            out.push_str(&format!("- {}{at}\n", f.text));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn fact(text: Option<&str>) -> Fact {
        Fact {
            path: vec![0, 1],
            label: "p.note".into(),
            rect: Box {
                x: 0.,
                y: 0.,
                w: 120.,
                h: 48.,
            },
            text: text.map(String::from),
            color: "rgb(17, 24, 39)".into(),
            background: Some("rgb(255, 255, 255)".into()),
            size: 16.,
            weight: 400,
            ..Default::default()
        }
    }

    fn kinds(facts: &Facts, device: Device) -> Vec<Kind> {
        let screens = HashSet::from(["home".to_string()]);
        review(facts, device, &screens)
            .into_iter()
            .map(|f| f.kind)
            .collect()
    }

    fn page(items: Vec<Fact>) -> Facts {
        Facts {
            width: 390.,
            scroll: 390.,
            items,
        }
    }

    #[test]
    fn contrast_is_measured_as_wcag_does() {
        let black = [0., 0., 0., 1.];
        let white = [255., 255., 255., 1.];
        assert!((contrast(black, white) - 21.).abs() < 0.01);
        assert!((contrast(white, white) - 1.).abs() < 0.01);
        // #767676 on white is the classic just-passing grey.
        let grey = rgba("#767676").unwrap();
        assert!(contrast(grey, white) >= 4.5);
        assert!(contrast(rgba("#777777").unwrap(), white) < 4.5);
        // Half see-through black over white is a mid grey.
        let half = over(rgba("rgba(0, 0, 0, 0.5)").unwrap(), white);
        assert!((half[0] - 127.5).abs() < 0.5);
    }

    #[test]
    fn readable_text_passes_and_faint_text_does_not() {
        assert!(kinds(&page(vec![fact(Some("Hello"))]), Device::Phone).is_empty());
        let mut faint = fact(Some("Terms apply"));
        faint.color = "rgb(200, 200, 200)".into();
        faint.size = 11.;
        assert_eq!(
            kinds(&page(vec![faint.clone()]), Device::Phone),
            vec![Kind::Contrast, Kind::SmallText]
        );
        // Big bold text needs less contrast.
        faint.color = "rgb(148, 148, 148)".into();
        faint.size = 28.;
        faint.weight = 700;
        assert!(kinds(&page(vec![faint]), Device::Phone).is_empty());
        // Text on a picture can't be judged.
        let mut on_photo = fact(Some("Sale"));
        on_photo.color = "rgb(250, 250, 250)".into();
        on_photo.background = None;
        assert!(kinds(&page(vec![on_photo]), Device::Phone).is_empty());
    }

    #[test]
    fn small_buttons_dead_links_and_the_rest_are_found() {
        let mut button = fact(None);
        button.click = true;
        button.rect.w = 32.;
        button.rect.h = 32.;
        button.goto = Some("missing".into());
        let mut img = fact(None);
        img.img = true;
        let mut raw = fact(None);
        raw.raw = vec!["background-color".into(), "background".into()];
        let mut facts = page(vec![button.clone(), img, raw]);
        facts.scroll = 420.;
        let found = kinds(&facts, Device::Phone);
        assert_eq!(found[0], Kind::Overflow);
        for k in [Kind::TapTarget, Kind::DeadLink, Kind::NoAlt, Kind::OffTheme] {
            assert!(found.contains(&k), "{k:?}");
        }
        let off = review(&facts, Device::Phone, &HashSet::new());
        let off = off.iter().find(|f| f.kind == Kind::OffTheme).unwrap();
        assert!(off
            .text
            .ends_with("sets its background-color outside the theme"));
        // The serious ones come first.
        assert!(found[..3].iter().all(|k| k.serious()));
        // 32 px is enough on a desktop, and a link inside a sentence is exempt.
        button.goto = Some("home".into());
        assert!(kinds(&page(vec![button.clone()]), Device::Desktop).is_empty());
        button.inline = true;
        assert!(kinds(&page(vec![button]), Device::Phone).is_empty());
    }

    #[test]
    fn findings_become_a_task() {
        let mut faint = fact(Some("Terms apply"));
        faint.color = "rgb(200, 200, 200)".into();
        let found = review(&page(vec![faint]), Device::Phone, &HashSet::new());
        let task = fix_task(&[("home".into(), "Home".into(), found)]);
        assert!(task.contains("Home (design/screens/home.html):"));
        assert!(task.contains("\u{201c}Terms apply\u{201d} is hard to read"));
        assert!(task.contains("`body > :nth-child(1) > :nth-child(2)`"));
    }
}
