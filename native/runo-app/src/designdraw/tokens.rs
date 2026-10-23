//! The design's tokens, read from `tokens.css` and written for the code that builds it: a
//! `tokens.json` in the design tokens (DTCG) format, a Tailwind theme for the web, and a
//! theme for the mobile stack the project picked. Values are worked out here, through
//! `var()`, `calc()` and `color-mix()`, so every output holds the same plain numbers.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// What's at the top of every file written from the tokens.
const HEADER: &str =
    "Written by Runo from design/tokens.css. Don't edit it: hand off again to update it.";

/// A token's worked-out value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// Red, green, blue and alpha, 0 to 255.
    Color([u8; 4]),
    Px(f32),
    Number(f32),
    Fonts(Vec<String>),
    Shadow(Vec<Shadow>),
    /// Anything else, as written.
    Raw(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: [u8; 4],
    pub inset: bool,
}

/// What a token is for, from its value and its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    Color,
    FontSize,
    Spacing,
    Radius,
    Size,
    FontFamily,
    Shadow,
    Number,
    Other,
}

impl Group {
    /// The group's key in tokens.json.
    fn key(self) -> &'static str {
        match self {
            Group::Color => "color",
            Group::FontSize => "fontSize",
            Group::Spacing => "spacing",
            Group::Radius => "radius",
            Group::Size => "size",
            Group::FontFamily => "fontFamily",
            Group::Shadow => "shadow",
            Group::Number => "number",
            Group::Other => "other",
        }
    }

    /// The start of a token's name that the group already says.
    fn prefix(self) -> &'static str {
        match self {
            Group::FontSize => "text",
            Group::Spacing => "space",
            Group::Radius => "radius",
            Group::FontFamily => "font",
            Group::Shadow => "shadow",
            _ => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Token {
    /// As named in the CSS, without `--`: "on-primary", "radius-sm".
    pub name: String,
    pub group: Group,
    pub light: Value,
    /// Its value in the dark theme, when that's different.
    pub dark: Option<Value>,
}

impl Token {
    /// Its name within its group: "radius-sm" is "sm" among the radii, and "radius" alone
    /// is "base".
    pub(crate) fn short(&self) -> String {
        let p = self.group.prefix();
        if p.is_empty() {
            return self.name.clone();
        }
        if self.name == p {
            return "base".into();
        }
        self.name
            .strip_prefix(p)
            .and_then(|r| r.strip_prefix('-'))
            .map_or_else(|| self.name.clone(), String::from)
    }
}

// ---- Reading the CSS

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        rest = rest[i + 2..]
            .find("*/")
            .map_or("", |j| &rest[i + 2 + j + 2..]);
    }
    out.push_str(rest);
    out
}

/// The rules at the top of `css`: each selector and what's between its braces.
fn rules(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = css.as_bytes();
    let (mut i, mut start) = (0, 0);
    while i < b.len() {
        if b[i] == b'{' {
            let sel = css[start..i].trim().to_string();
            let mut depth = 1;
            let mut j = i + 1;
            while j < b.len() && depth > 0 {
                match b[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            out.push((sel, css[i + 1..j.saturating_sub(1)].to_string()));
            i = j;
            start = j;
        } else if b[i] == b';' && css[start..i].trim_start().starts_with('@') {
            // An at-rule with no block, such as @import.
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    out
}

fn custom_props(body: &str, into: &mut Props) {
    for d in split_top(body, ';') {
        if let Some((k, v)) = d.split_once(':') {
            if let Some(k) = k.trim().strip_prefix("--") {
                let v = v.trim().trim_end_matches("!important").trim();
                into.retain(|(k2, _)| k2 != k);
                into.push((k.to_string(), v.to_string()));
            }
        }
    }
}

fn is_dark_selector(sel: &str) -> bool {
    sel.contains("data-theme=\"dark\"") || sel.contains("data-theme='dark'") || sel == ".dark"
}

/// The custom properties of the light theme and the dark one, in the order written.
/// Custom properties, by name and as written.
type Props = Vec<(String, String)>;

fn themes(css: &str) -> (Props, Props) {
    let css = strip_comments(css);
    let (mut light, mut dark) = (Vec::new(), Vec::new());
    for (sel, body) in rules(&css) {
        if sel.starts_with("@media") {
            if sel.replace(' ', "").contains("prefers-color-scheme:dark") {
                for (inner, b) in rules(&body) {
                    if inner.contains(":root") || inner == "html" {
                        custom_props(&b, &mut dark);
                    }
                }
            }
        } else if is_dark_selector(&sel) {
            custom_props(&body, &mut dark);
        } else if sel
            .split(',')
            .any(|s| s.trim() == ":root" || s.trim() == "html")
        {
            custom_props(&body, &mut light);
        }
    }
    (light, dark)
}

/// `s` split at `sep`, but not inside brackets or quotes.
pub(crate) fn split_top(s: &str, sep: char) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    let (mut depth, mut quote) = (0i32, None);
    for c in s.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, _) if c == sep && depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

// ---- Working out values

/// `raw` with every `var()` replaced by what it stands for in `env`.
fn expand(raw: &str, env: &BTreeMap<String, String>, depth: usize) -> String {
    if depth > 12 {
        return raw.to_string();
    }
    let Some(i) = raw.find("var(") else {
        return raw.to_string();
    };
    // Find the var()'s closing bracket.
    let mut d = 0;
    let mut end = raw.len();
    for (j, c) in raw[i + 3..].char_indices() {
        match c {
            '(' => d += 1,
            ')' => {
                d -= 1;
                if d == 0 {
                    end = i + 3 + j;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = &raw[i + 4..end.min(raw.len())];
    let (name, fallback) = match inner.split_once(',') {
        Some((n, f)) => (n.trim(), Some(f.trim())),
        None => (inner.trim(), None),
    };
    let with = env
        .get(name.trim_start_matches("--"))
        .map(String::as_str)
        .or(fallback)
        .unwrap_or("");
    let replaced = format!(
        "{}{}{}",
        &raw[..i],
        expand(with, env, depth + 1),
        raw.get(end + 1..).unwrap_or("")
    );
    expand(&replaced, env, depth + 1)
}

fn hex(v: &str) -> Option<[u8; 4]> {
    let h = v.strip_prefix('#')?;
    let h: String = if h.len() <= 4 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// A number from CSS, with a percentage as a fraction when `pct_of` is given.
fn channel(s: &str, pct_of: f32) -> Option<f32> {
    match s.strip_suffix('%') {
        Some(p) => p.trim().parse::<f32>().ok().map(|p| p / 100. * pct_of),
        None => s.trim().parse().ok(),
    }
}

pub(crate) fn color(v: &str) -> Option<[u8; 4]> {
    let v = v.trim();
    match v.to_ascii_lowercase().as_str() {
        "transparent" => return Some([0, 0, 0, 0]),
        "white" => return Some([255, 255, 255, 255]),
        "black" => return Some([0, 0, 0, 255]),
        _ => {}
    }
    if v.starts_with('#') {
        return hex(v);
    }
    if let Some(inner) = v
        .strip_prefix("rgba(")
        .or_else(|| v.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner
            .split([',', ' ', '/'])
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() < 3 {
            return None;
        }
        let c = |i: usize| channel(parts[i], 255.).map(|n| n.round().clamp(0., 255.) as u8);
        let a = parts.get(3).map_or(Some(1.), |a| channel(a, 1.))?;
        return Some([
            c(0)?,
            c(1)?,
            c(2)?,
            (a * 255.).round().clamp(0., 255.) as u8,
        ]);
    }
    if let Some(inner) = v
        .strip_prefix("color-mix(")
        .and_then(|r| r.strip_suffix(')'))
    {
        return color_mix(inner);
    }
    None
}

/// `color-mix(in srgb, a p%, b q%)`, mixed as CSS mixes it: with premultiplied alpha.
fn color_mix(inner: &str) -> Option<[u8; 4]> {
    let args = split_top(inner, ',');
    if args.len() != 3 || !args[0].replace(' ', "").starts_with("insrgb") {
        return None;
    }
    let part = |s: &str| -> Option<([u8; 4], Option<f32>)> {
        let s = s.trim();
        // The percentage, if any, comes last after a space outside brackets.
        let words = split_top(s, ' ');
        match words.last() {
            Some(w) if w.ends_with('%') && words.len() > 1 => {
                let pct = channel(w, 1.)?;
                let col = &s[..s.len() - w.len()];
                Some((color(col)?, Some(pct)))
            }
            _ => Some((color(s)?, None)),
        }
    };
    let (a, pa) = part(&args[1])?;
    let (b, pb) = part(&args[2])?;
    let (pa, pb) = match (pa, pb) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, 1. - x),
        (None, Some(y)) => (1. - y, y),
        (None, None) => (0.5, 0.5),
    };
    let sum = pa + pb;
    if sum <= 0. {
        return None;
    }
    let (pa, pb) = (pa / sum, pb / sum);
    let (aa, ab) = (a[3] as f32 / 255., b[3] as f32 / 255.);
    let alpha = aa * pa + ab * pb;
    let mut out = [0u8; 4];
    for i in 0..3 {
        out[i] = if alpha > 0. {
            ((a[i] as f32 * aa * pa + b[i] as f32 * ab * pb) / alpha)
                .round()
                .clamp(0., 255.) as u8
        } else {
            0
        };
    }
    out[3] = (alpha * sum.min(1.) * 255.).round() as u8;
    Some(out)
}

/// A length or number, with `calc()` worked out: px, rem (16px) and plain numbers.
pub(crate) fn length(v: &str) -> Option<(f32, bool)> {
    let v = v.trim();
    if let Some(inner) = v.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) {
        let mut p = Calc {
            s: inner.as_bytes(),
            i: 0,
        };
        let out = p.sum()?;
        p.ws();
        return (p.i == p.s.len()).then_some(out);
    }
    let mut p = Calc {
        s: v.as_bytes(),
        i: 0,
    };
    let out = p.atom()?;
    p.ws();
    (p.i == p.s.len()).then_some(out)
}

/// Arithmetic as `calc()` does it, on px and plain numbers.
struct Calc<'a> {
    s: &'a [u8],
    i: usize,
}

impl Calc<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }

    fn sum(&mut self) -> Option<(f32, bool)> {
        let mut acc = self.product()?;
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b'+') | Some(b'-') => {
                    let minus = self.s[self.i] == b'-';
                    self.i += 1;
                    let (n, px) = self.product()?;
                    acc = (if minus { acc.0 - n } else { acc.0 + n }, acc.1 || px);
                }
                _ => return Some(acc),
            }
        }
    }

    fn product(&mut self) -> Option<(f32, bool)> {
        let mut acc = self.atom()?;
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b'*') => {
                    self.i += 1;
                    let (n, px) = self.atom()?;
                    acc = (acc.0 * n, acc.1 || px);
                }
                Some(b'/') => {
                    self.i += 1;
                    let (n, _) = self.atom()?;
                    acc = (acc.0 / n, acc.1);
                }
                _ => return Some(acc),
            }
        }
    }

    fn atom(&mut self) -> Option<(f32, bool)> {
        self.ws();
        if self.s.get(self.i) == Some(&b'(') {
            self.i += 1;
            let out = self.sum()?;
            self.ws();
            (self.s.get(self.i) == Some(&b')')).then(|| self.i += 1)?;
            return Some(out);
        }
        if self.s[self.i..].starts_with(b"calc(") {
            self.i += 4;
            return self.atom();
        }
        let start = self.i;
        while self
            .s
            .get(self.i)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'.' || (*c == b'-' && self.i == start))
        {
            self.i += 1;
        }
        let n: f32 = std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()?;
        let unit_start = self.i;
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        match &self.s[unit_start..self.i] {
            b"px" => Some((n, true)),
            b"rem" | b"em" => Some((n * 16., true)),
            b"" => Some((n, false)),
            _ => None,
        }
    }
}

fn shadow(v: &str) -> Option<Vec<Shadow>> {
    let mut out = Vec::new();
    for layer in split_top(v, ',') {
        let mut lens = Vec::new();
        let (mut col, mut inset) = (None, false);
        for w in split_top(&layer, ' ') {
            if w == "inset" {
                inset = true;
            } else if let Some((n, _)) = length(&w) {
                lens.push(n);
            } else {
                col = Some(color(&w)?);
            }
        }
        if lens.len() < 2 {
            return None;
        }
        out.push(Shadow {
            x: lens[0],
            y: lens[1],
            blur: lens.get(2).copied().unwrap_or(0.),
            spread: lens.get(3).copied().unwrap_or(0.),
            color: col.unwrap_or([0, 0, 0, 255]),
            inset,
        });
    }
    (!out.is_empty()).then_some(out)
}

fn fonts(v: &str) -> Vec<String> {
    split_top(v, ',')
        .into_iter()
        .map(|f| f.trim_matches(['"', '\'']).to_string())
        .collect()
}

fn value(name: &str, raw: &str) -> Value {
    if let Some(c) = color(raw) {
        return Value::Color(c);
    }
    if let Some((n, px)) = length(raw) {
        return if px || n == 0. {
            Value::Px(n)
        } else {
            Value::Number(n)
        };
    }
    if let Some(s) = shadow(raw) {
        return Value::Shadow(s);
    }
    if name == "font" || (name.starts_with("font-") && !name.contains("weight")) {
        return Value::Fonts(fonts(raw));
    }
    Value::Raw(raw.to_string())
}

fn group(name: &str, v: &Value) -> Group {
    let starts = |p: &str| name == p || name.starts_with(&format!("{p}-"));
    match v {
        Value::Color(_) => Group::Color,
        Value::Shadow(_) => Group::Shadow,
        Value::Fonts(_) => Group::FontFamily,
        Value::Px(_) if starts("text") || starts("font-size") => Group::FontSize,
        Value::Px(_) if starts("space") || starts("spacing") => Group::Spacing,
        Value::Px(_) if starts("radius") => Group::Radius,
        Value::Px(_) => Group::Size,
        Value::Number(_) => Group::Number,
        Value::Raw(_) => Group::Other,
    }
}

/// The design's tokens, worked out for the light theme and the dark one.
pub(crate) fn read(css: &str) -> Vec<Token> {
    let (light, dark) = themes(css);
    let env_light: BTreeMap<String, String> = light.iter().cloned().collect();
    let mut env_dark = env_light.clone();
    env_dark.extend(dark.iter().cloned());
    light
        .iter()
        .map(|(name, raw)| {
            let lv = value(name, &expand(raw, &env_light, 0));
            let raw_dark = env_dark.get(name).unwrap_or(raw);
            let dv = value(name, &expand(raw_dark, &env_dark, 0));
            Token {
                name: name.clone(),
                group: group(name, &lv),
                dark: (dv != lv).then_some(dv),
                light: lv,
            }
        })
        .collect()
}

// ---- Writing them out

fn hex_of(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

fn rgba_css(c: [u8; 4]) -> String {
    if c[3] == 255 {
        hex_of(c)
    } else {
        format!(
            "rgb({} {} {} / {})",
            c[0],
            c[1],
            c[2],
            num(c[3] as f32 / 255.)
        )
    }
}

fn num(n: f32) -> String {
    let r = (n * 1000.).round() / 1000.;
    if r.fract() == 0. {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn shadow_css(s: &[Shadow]) -> String {
    s.iter()
        .map(|l| {
            format!(
                "{}{}px {}px {}px {}px {}",
                if l.inset { "inset " } else { "" },
                num(l.x),
                num(l.y),
                num(l.blur),
                num(l.spread),
                rgba_css(l.color)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn css_value(v: &Value) -> String {
    match v {
        Value::Color(c) => rgba_css(*c),
        Value::Px(n) => format!("{}px", num(*n)),
        Value::Number(n) => num(*n),
        Value::Fonts(f) => f
            .iter()
            .map(|f| {
                if f.contains(' ') {
                    format!("\"{f}\"")
                } else {
                    f.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Shadow(s) => shadow_css(s),
        Value::Raw(r) => r.clone(),
    }
}

fn dtcg_value(v: &Value) -> serde_json::Value {
    use serde_json::json;
    let dim = |n: f32| json!({ "value": n, "unit": "px" });
    match v {
        Value::Color(c) => json!(hex_of(*c)),
        Value::Px(n) => dim(*n),
        Value::Number(n) => json!(n),
        Value::Fonts(f) => json!(f),
        Value::Shadow(s) => serde_json::Value::Array(
            s.iter()
                .map(|l| {
                    json!({
                        "color": hex_of(l.color),
                        "offsetX": dim(l.x),
                        "offsetY": dim(l.y),
                        "blur": dim(l.blur),
                        "spread": dim(l.spread),
                        "inset": l.inset,
                    })
                })
                .collect(),
        ),
        Value::Raw(r) => json!(r),
    }
}

fn dtcg_type(g: Group) -> Option<&'static str> {
    Some(match g {
        Group::Color => "color",
        Group::FontSize | Group::Spacing | Group::Radius | Group::Size => "dimension",
        Group::FontFamily => "fontFamily",
        Group::Shadow => "shadow",
        Group::Number => "number",
        Group::Other => return None,
    })
}

/// tokens.json, in the design tokens format: one group per kind of token, with the dark
/// theme's value kept beside the light one.
pub(crate) fn dtcg(tokens: &[Token]) -> String {
    let mut root = serde_json::Map::new();
    root.insert("$description".into(), HEADER.into());
    for t in tokens {
        let mut entry = serde_json::Map::new();
        if let Some(ty) = dtcg_type(t.group) {
            entry.insert("$type".into(), ty.into());
        }
        entry.insert("$value".into(), dtcg_value(&t.light));
        if let Some(d) = &t.dark {
            entry.insert(
                "$extensions".into(),
                serde_json::json!({ "com.runo.modes": { "dark": dtcg_value(d) } }),
            );
        }
        let group = root
            .entry(t.group.key())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let Some(g) = group.as_object_mut() {
            g.insert(t.short(), serde_json::Value::Object(entry));
        }
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap_or_default() + "\n"
}

/// A Tailwind (v4) theme: the tokens as CSS variables, light and dark, and `@theme` names
/// for them, so `bg-primary`, `rounded-lg`, `text-sm` and `shadow-lg` use the design's.
pub(crate) fn tailwind(tokens: &[Token]) -> String {
    let mut out = format!("/* {HEADER} */\n\n:root {{\n");
    for t in tokens {
        let _ = writeln!(out, "  --{}: {};", t.name, css_value(&t.light));
    }
    out.push_str("}\n");
    let dark: Vec<&Token> = tokens.iter().filter(|t| t.dark.is_some()).collect();
    if !dark.is_empty() {
        let mut block = String::new();
        for t in &dark {
            if let Some(d) = &t.dark {
                let _ = writeln!(block, "  --{}: {};", t.name, css_value(d));
            }
        }
        let _ = write!(
            out,
            "\n@media (prefers-color-scheme: dark) {{\n  :root {{\n{}  }}\n}}\n\n.dark {{\n{block}}}\n",
            block.lines().map(|l| format!("  {l}\n")).collect::<String>()
        );
    }
    out.push_str("\n@theme inline {\n");
    for t in tokens {
        let short = t.short();
        let name = match t.group {
            Group::Color => format!("color-{}", t.name),
            Group::FontSize => format!("text-{short}"),
            Group::Radius => format!("radius-{short}"),
            Group::Shadow => format!("shadow-{short}"),
            Group::FontFamily if short == "base" => "font-sans".into(),
            Group::FontFamily => format!("font-{short}"),
            Group::Spacing if short == "base" => "spacing".into(),
            Group::Spacing => format!("spacing-{short}"),
            _ => continue,
        };
        let _ = writeln!(out, "  --{name}: var(--{});", t.name);
    }
    out.push_str("}\n");
    out
}

/// A name as code calls it: "on-primary" is "onPrimary", and "2xl", which can't start a
/// name, is "xl2".
pub(crate) fn ident(name: &str) -> String {
    let mut words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    if let Some(first) = words.first_mut() {
        let digits: String = first.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            *first = format!("{}{digits}", &first[digits.len()..]);
            if first.chars().all(|c| c.is_ascii_digit()) {
                *first = format!("n{first}");
            }
        }
    }
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        let mut c = w.chars();
        if let Some(f) = c.next() {
            if i == 0 {
                out.extend(f.to_lowercase());
            } else {
                out.extend(f.to_uppercase());
            }
            out.push_str(c.as_str());
        }
    }
    if out.is_empty() {
        "token".into()
    } else {
        out
    }
}

fn of_group(tokens: &[Token], g: Group) -> Vec<&Token> {
    tokens.iter().filter(|t| t.group == g).collect()
}

fn px_of(v: &Value) -> f32 {
    match v {
        Value::Px(n) | Value::Number(n) => *n,
        _ => 0.,
    }
}

fn swift_color(c: [u8; 4]) -> String {
    format!(
        "Color(.sRGB, red: {}, green: {}, blue: {}, opacity: {})",
        num(c[0] as f32 / 255.),
        num(c[1] as f32 / 255.),
        num(c[2] as f32 / 255.),
        num(c[3] as f32 / 255.)
    )
}

/// The tokens for SwiftUI: colours that follow the light or dark appearance, and the
/// sizes as CGFloat.
pub(crate) fn swiftui(tokens: &[Token]) -> String {
    let mut out = format!(
        "// {HEADER}\n\nimport SwiftUI\n#if canImport(UIKit)\nimport UIKit\n#endif\n\nprivate func adaptive(_ light: Color, _ dark: Color) -> Color {{\n#if canImport(UIKit)\n    Color(UIColor {{ $0.userInterfaceStyle == .dark ? UIColor(dark) : UIColor(light) }})\n#else\n    light\n#endif\n}}\n\nstruct ThemeShadow {{\n    let color: Color\n    let radius: CGFloat\n    let x: CGFloat\n    let y: CGFloat\n}}\n\nenum Theme {{\n"
    );
    let colors = of_group(tokens, Group::Color);
    if !colors.is_empty() {
        out.push_str("    enum Colors {\n");
        for t in colors {
            let Value::Color(l) = t.light else { continue };
            let v = match &t.dark {
                Some(Value::Color(d)) => {
                    format!("adaptive({}, {})", swift_color(l), swift_color(*d))
                }
                _ => swift_color(l),
            };
            let _ = writeln!(out, "        static let {} = {v}", ident(&t.name));
        }
        out.push_str("    }\n");
    }
    for (g, name) in [
        (Group::Spacing, "Spacing"),
        (Group::Radius, "Radius"),
        (Group::FontSize, "FontSize"),
        (Group::Size, "Size"),
    ] {
        let list = of_group(tokens, g);
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(out, "    enum {name} {{");
        for t in list {
            let _ = writeln!(
                out,
                "        static let {}: CGFloat = {}",
                ident(&t.short()),
                num(px_of(&t.light))
            );
        }
        out.push_str("    }\n");
    }
    let shadows = of_group(tokens, Group::Shadow);
    if !shadows.is_empty() {
        out.push_str("    enum Shadows {\n");
        for t in shadows {
            let Value::Shadow(s) = &t.light else { continue };
            let layers: Vec<String> = s
                .iter()
                .filter(|l| !l.inset)
                .map(|l| {
                    format!(
                        "ThemeShadow(color: {}, radius: {}, x: {}, y: {})",
                        swift_color(l.color),
                        num(l.blur / 2.),
                        num(l.x),
                        num(l.y)
                    )
                })
                .collect();
            let _ = writeln!(
                out,
                "        static let {} = [{}]",
                ident(&t.short()),
                layers.join(", ")
            );
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n");
    out
}

fn argb(c: [u8; 4]) -> String {
    format!("Color(0x{:02X}{:02X}{:02X}{:02X})", c[3], c[0], c[1], c[2])
}

/// The tokens for Flutter: the colours as a theme extension with a light and a dark
/// value, the rest as constants.
pub(crate) fn flutter(tokens: &[Token]) -> String {
    let mut out = format!("// {HEADER}\n\nimport 'package:flutter/material.dart';\n\n");
    let colors: Vec<(&Token, [u8; 4], [u8; 4])> = of_group(tokens, Group::Color)
        .into_iter()
        .filter_map(|t| {
            let Value::Color(l) = t.light else {
                return None;
            };
            let d = match t.dark {
                Some(Value::Color(d)) => d,
                _ => l,
            };
            Some((t, l, d))
        })
        .collect();
    if !colors.is_empty() {
        let names: Vec<String> = colors.iter().map(|(t, _, _)| ident(&t.name)).collect();
        out.push_str("/// The design's colours. Add `AppColors.light` and `AppColors.dark` to your\n/// themes' extensions, then read them with `Theme.of(context).extension<AppColors>()!`.\n@immutable\nclass AppColors extends ThemeExtension<AppColors> {\n  const AppColors({\n");
        for n in &names {
            let _ = writeln!(out, "    required this.{n},");
        }
        out.push_str("  });\n\n");
        for n in &names {
            let _ = writeln!(out, "  final Color {n};");
        }
        for (mode, pick) in [("light", 1), ("dark", 2)] {
            let _ = writeln!(out, "\n  static const {mode} = AppColors(");
            for (n, (_, l, d)) in names.iter().zip(&colors) {
                let c = if pick == 1 { *l } else { *d };
                let _ = writeln!(out, "    {n}: {},", argb(c));
            }
            out.push_str("  );\n");
        }
        out.push_str("\n  @override\n  AppColors copyWith({\n");
        for n in &names {
            let _ = writeln!(out, "    Color? {n},");
        }
        out.push_str("  }) {\n    return AppColors(\n");
        for n in &names {
            let _ = writeln!(out, "      {n}: {n} ?? this.{n},");
        }
        out.push_str("    );\n  }\n\n  @override\n  AppColors lerp(ThemeExtension<AppColors>? other, double t) {\n    if (other is! AppColors) return this;\n    return AppColors(\n");
        for n in &names {
            let _ = writeln!(out, "      {n}: Color.lerp({n}, other.{n}, t)!,");
        }
        out.push_str("    );\n  }\n}\n");
    }
    for (g, name) in [
        (Group::Spacing, "AppSpacing"),
        (Group::Radius, "AppRadius"),
        (Group::FontSize, "AppFontSize"),
        (Group::Size, "AppSize"),
    ] {
        let list = of_group(tokens, g);
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\nabstract final class {name} {{");
        for t in list {
            let _ = writeln!(
                out,
                "  static const double {} = {};",
                ident(&t.short()),
                dart_num(px_of(&t.light))
            );
        }
        out.push_str("}\n");
    }
    let shadows = of_group(tokens, Group::Shadow);
    if !shadows.is_empty() {
        out.push_str("\nabstract final class AppShadows {\n");
        for t in shadows {
            let Value::Shadow(s) = &t.light else { continue };
            let layers: Vec<String> = s
                .iter()
                .filter(|l| !l.inset)
                .map(|l| {
                    format!(
                        "BoxShadow(color: {}, offset: Offset({}, {}), blurRadius: {}, spreadRadius: {})",
                        argb(l.color),
                        dart_num(l.x),
                        dart_num(l.y),
                        dart_num(l.blur),
                        dart_num(l.spread)
                    )
                })
                .collect();
            let _ = writeln!(
                out,
                "  static const {} = [{}];",
                ident(&t.short()),
                layers.join(", ")
            );
        }
        out.push_str("}\n");
    }
    out
}

fn dart_num(n: f32) -> String {
    let s = num(n);
    if s.contains('.') {
        s
    } else {
        format!("{s}.0")
    }
}

/// The tokens for React Native: colours for each appearance, and the sizes as numbers.
pub(crate) fn react_native(tokens: &[Token]) -> String {
    let mut out = format!("// {HEADER}\n\n");
    let colors = of_group(tokens, Group::Color);
    if !colors.is_empty() {
        out.push_str("export const colors = {\n");
        for (mode, dark) in [("light", false), ("dark", true)] {
            let _ = writeln!(out, "  {mode}: {{");
            for t in &colors {
                let v = if dark {
                    t.dark.as_ref().unwrap_or(&t.light)
                } else {
                    &t.light
                };
                if let Value::Color(c) = v {
                    let _ = writeln!(out, "    {}: '{}',", ident(&t.name), hex_of(*c));
                }
            }
            out.push_str("  },\n");
        }
        out.push_str("} as const;\n\nexport type ColorName = keyof typeof colors.light;\n");
    }
    for (g, name) in [
        (Group::Spacing, "spacing"),
        (Group::Radius, "radius"),
        (Group::FontSize, "fontSize"),
        (Group::Size, "size"),
    ] {
        let list = of_group(tokens, g);
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\nexport const {name} = {{");
        for t in list {
            let _ = writeln!(out, "  {}: {},", ident(&t.short()), num(px_of(&t.light)));
        }
        out.push_str("} as const;\n");
    }
    let shadows = of_group(tokens, Group::Shadow);
    if !shadows.is_empty() {
        out.push_str("\n// For the boxShadow style (React Native 0.76 and later).\nexport const shadow = {\n");
        for t in shadows {
            if let Value::Shadow(s) = &t.light {
                let _ = writeln!(out, "  {}: '{}',", ident(&t.short()), shadow_css(s));
            }
        }
        out.push_str("} as const;\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSS: &str = r#"/* theme */
:root {
  --primary: #2563eb;
  --surface: #f4f5f7;
  --text: #0f172a;
  --radius: 12px;
  --space: 8px;
  --font: -apple-system, "Segoe UI", sans-serif;
  --surface-2: color-mix(in srgb, var(--text) 7%, var(--surface));
  --primary-soft: color-mix(in srgb, var(--primary) 12%, transparent);
  --text-2xl: 28px;
  --radius-sm: calc(var(--radius) * 0.5);
  --shadow-sm: 0 1px 2px rgb(15 23 42 / 0.06);
  --line: 1.5;
}
@media (prefers-color-scheme: dark) {
  :root { --primary: #f97316; --text: #e8ebf1; }
}
"#;

    fn get<'a>(t: &'a [Token], n: &str) -> &'a Token {
        t.iter().find(|t| t.name == n).unwrap()
    }

    #[test]
    fn tokens_are_worked_out_for_both_themes() {
        let t = read(CSS);
        assert_eq!(
            get(&t, "primary").light,
            Value::Color([0x25, 0x63, 0xeb, 255])
        );
        assert_eq!(
            get(&t, "primary").dark,
            Some(Value::Color([0xf9, 0x73, 0x16, 255]))
        );
        assert_eq!(get(&t, "surface").dark, None);
        assert_eq!(get(&t, "radius-sm").light, Value::Px(6.));
        assert_eq!(get(&t, "radius-sm").group, Group::Radius);
        assert_eq!(get(&t, "radius-sm").short(), "sm");
        assert_eq!(get(&t, "radius").short(), "base");
        assert_eq!(get(&t, "text").group, Group::Color);
        assert_eq!(get(&t, "text-2xl").group, Group::FontSize);
        assert_eq!(get(&t, "line").light, Value::Number(1.5));
        // 7% of the text colour over the surface; the dark theme mixes its own text.
        let Value::Color(s2) = get(&t, "surface-2").light else {
            panic!()
        };
        assert_eq!(s2[3], 255);
        assert!(s2[0] < 0xf4 && s2[0] > 0xe0);
        assert!(get(&t, "surface-2").dark.is_some());
        assert_eq!(
            get(&t, "primary-soft").light,
            Value::Color([0x25, 0x63, 0xeb, 31])
        );
        assert_eq!(
            get(&t, "font").light,
            Value::Fonts(vec![
                "-apple-system".into(),
                "Segoe UI".into(),
                "sans-serif".into()
            ])
        );
        let Value::Shadow(sh) = &get(&t, "shadow-sm").light else {
            panic!()
        };
        assert_eq!((sh[0].y, sh[0].blur, sh[0].color[3]), (1., 2., 15));
    }

    #[test]
    fn names_become_code_names() {
        assert_eq!(ident("on-primary"), "onPrimary");
        assert_eq!(ident("surface-2"), "surface2");
        assert_eq!(ident("2xl"), "xl2");
        assert_eq!(ident("base"), "base");
    }

    #[test]
    fn every_stack_gets_the_same_values() {
        let t = read(CSS);
        let json: serde_json::Value = serde_json::from_str(&dtcg(&t)).unwrap();
        assert_eq!(json["color"]["primary"]["$value"], "#2563eb");
        assert_eq!(
            json["color"]["primary"]["$extensions"]["com.runo.modes"]["dark"],
            "#f97316"
        );
        assert_eq!(json["radius"]["sm"]["$value"]["value"], 6.0);
        assert_eq!(json["fontSize"]["2xl"]["$type"], "dimension");

        let tw = tailwind(&t);
        assert!(tw.contains("  --surface-2: #"));
        assert!(tw.contains("--color-primary: var(--primary);"));
        assert!(tw.contains("--radius-sm: var(--radius-sm);"));
        assert!(tw.contains("--spacing: var(--space);"));
        assert!(tw.contains("--font-sans: var(--font);"));
        assert!(tw.contains("@media (prefers-color-scheme: dark)"));
        assert!(tw.contains("--primary: #f97316;"));

        let swift = swiftui(&t);
        assert!(swift.contains("static let primary = adaptive(Color(.sRGB, red: 0.145"));
        assert!(swift.contains("static let sm: CGFloat = 6"));
        assert!(swift.contains("static let xl2: CGFloat = 28"));

        let dart = flutter(&t);
        assert!(dart.contains("    primary: Color(0xFF2563EB),"));
        assert!(dart.contains("    primary: Color(0xFFF97316),"));
        assert!(dart.contains("static const double sm = 6.0;"));
        assert!(dart.contains("Color.lerp(primary, other.primary, t)!"));

        let rn = react_native(&t);
        assert!(rn.contains("    primary: '#2563eb',"));
        assert!(rn.contains("    primary: '#f97316',"));
        assert!(rn.contains("  sm: 6,"));
        assert!(rn.contains("sm: '0px 1px 2px 0px rgb(15 23 42 / 0.059)'"));
    }
}
