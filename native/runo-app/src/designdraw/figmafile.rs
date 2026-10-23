//! Import from Figma: the frames a Figma link points at, read through Figma's REST API
//! with the access token from Settings and written as screens. Auto layout becomes flex,
//! everything else sits where it is in its frame, and pictures and drawings come along
//! as files and inline SVG.

use super::model::{Device, ASSETS};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// Frames one import brings in at most.
const MAX_FRAMES: usize = 40;

/// Drawings one import asks Figma to export at most.
const MAX_VECTORS: usize = 400;

/// A Figma file, and the frame or page the link points at.
#[derive(Debug, PartialEq)]
pub(crate) struct Link {
    pub key: String,
    pub node: Option<String>,
}

/// The file and node in a link copied from Figma, such as
/// `https://www.figma.com/design/KEY/Name?node-id=1-2`.
pub(crate) fn link(url: &str) -> Option<Link> {
    let rest = url.trim().split_once("figma.com/")?.1;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut parts = path.split('/');
    if !matches!(parts.next()?, "file" | "design" | "proto") {
        return None;
    }
    let mut key = parts.next()?;
    // A branch has a key of its own.
    if parts.next() == Some("branch") {
        key = parts.next()?;
    }
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let node = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("node-id="))
        .map(|id| id.replace("%3A", ":").replace("%3a", ":").replace('-', ":"))
        .filter(|id| !id.is_empty());
    Some(Link {
        key: key.to_string(),
        node,
    })
}

/// A frame made into a screen.
pub(crate) struct Imported {
    pub name: String,
    pub device: Device,
    /// What goes in the screen's `<body>`.
    pub body: String,
}

/// Pictures and drawings fetched for the frames: picture fills by their reference, as the
/// path a screen uses, and drawings by node id, as SVG.
#[derive(Default)]
pub(crate) struct Assets {
    pub images: HashMap<String, String>,
    pub svgs: HashMap<String, String>,
}

fn ty(n: &Value) -> &str {
    n["type"].as_str().unwrap_or("")
}

fn kids(n: &Value) -> impl Iterator<Item = &Value> {
    n["children"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["visible"].as_bool() != Some(false))
}

fn is_frame(n: &Value) -> bool {
    matches!(ty(n), "FRAME" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE")
}

/// Shapes Figma exports as SVG, since CSS can't draw them.
fn is_drawing(n: &Value) -> bool {
    matches!(
        ty(n),
        "VECTOR" | "BOOLEAN_OPERATION" | "STAR" | "LINE" | "REGULAR_POLYGON"
    )
}

/// The frames under a node: the node itself when it is one, else the frames on the page or
/// in the section, or on the file's first page.
pub(crate) fn frames(root: &Value) -> Vec<&Value> {
    match ty(root) {
        "DOCUMENT" => kids(root).next().map(frames).unwrap_or_default(),
        "CANVAS" | "SECTION" => kids(root)
            .flat_map(|c| match ty(c) {
                "SECTION" => frames(c),
                _ if is_frame(c) => vec![c],
                _ => Vec::new(),
            })
            .collect(),
        _ => vec![root],
    }
}

/// The picture references and drawing ids a frame needs.
pub(crate) fn wanted(n: &Value, images: &mut Vec<String>, drawings: &mut Vec<String>) {
    if n["visible"].as_bool() == Some(false) {
        return;
    }
    if is_drawing(n) {
        if let Some(id) = n["id"].as_str() {
            drawings.push(id.to_string());
        }
        return;
    }
    for f in paints(&n["fills"]) {
        if let Some(r) = f["imageRef"].as_str().filter(|_| ty(f) == "IMAGE") {
            if !images.iter().any(|i| i == r) {
                images.push(r.to_string());
            }
        }
    }
    for c in kids(n) {
        wanted(c, images, drawings);
    }
}

fn paints(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["visible"].as_bool() != Some(false))
}

/// The device a frame of this width is for.
pub(crate) fn device(width: f64) -> Device {
    if width < 600. {
        Device::Phone
    } else if width < 1100. {
        Device::Tablet
    } else {
        Device::Desktop
    }
}

/// A frame as a screen. Values in a `style` attribute quote with `'`.
pub(crate) fn screen(frame: &Value, assets: &Assets) -> Imported {
    let mut body = String::new();
    node(frame, None, assets, &mut body, 0);
    Imported {
        name: frame["name"].as_str().unwrap_or("Frame").trim().to_string(),
        device: device(bounds(frame).2),
        body: body.trim_end().to_string(),
    }
}

fn bounds(n: &Value) -> (f64, f64, f64, f64) {
    let b = &n["absoluteBoundingBox"];
    let f = |k: &str| b[k].as_f64().unwrap_or(0.);
    (f("x"), f("y"), f("width"), f("height"))
}

/// A number for CSS, to two places and without a trailing `.0`.
fn num(n: f64) -> String {
    let r = (n * 100.).round() / 100.;
    if r.fract() == 0. {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn px(n: f64) -> String {
    format!("{}px", num(n))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A Figma colour, with the paint's opacity, as CSS.
fn color(c: &Value, opacity: f64) -> String {
    let ch = |k: &str| (c[k].as_f64().unwrap_or(0.).clamp(0., 1.) * 255.).round() as u8;
    let a = c["a"].as_f64().unwrap_or(1.) * opacity;
    if a >= 0.999 {
        format!("#{:02X}{:02X}{:02X}", ch("r"), ch("g"), ch("b"))
    } else {
        format!("rgba({}, {}, {}, {})", ch("r"), ch("g"), ch("b"), num(a))
    }
}

fn paint_opacity(p: &Value) -> f64 {
    p["opacity"].as_f64().unwrap_or(1.)
}

fn stops(p: &Value) -> String {
    let o = paint_opacity(p);
    p["gradientStops"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            format!(
                "{} {}%",
                color(&s["color"], o),
                num(s["position"].as_f64().unwrap_or(0.) * 100.)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A paint as one CSS background layer, or the colour when it's a plain one.
fn layer(p: &Value, w: f64, h: f64, assets: &Assets) -> Option<String> {
    match ty(p) {
        "SOLID" => Some(color(&p["color"], paint_opacity(p))),
        "GRADIENT_LINEAR" => {
            let hs = p["gradientHandlePositions"].as_array()?;
            let pt = |i: usize| {
                (
                    hs.get(i).and_then(|v| v["x"].as_f64()).unwrap_or(0.),
                    hs.get(i).and_then(|v| v["y"].as_f64()).unwrap_or(0.),
                )
            };
            let ((x0, y0), (x1, y1)) = (pt(0), pt(1));
            // CSS angles start at the top and turn clockwise.
            let deg = ((x1 - x0) * w).atan2(-(y1 - y0) * h).to_degrees();
            Some(format!("linear-gradient({}deg, {})", num(deg), stops(p)))
        }
        "GRADIENT_RADIAL" | "GRADIENT_DIAMOND" => Some(format!("radial-gradient({})", stops(p))),
        "GRADIENT_ANGULAR" => Some(format!("conic-gradient({})", stops(p))),
        "IMAGE" => {
            let url = assets.images.get(p["imageRef"].as_str()?)?;
            let fit = match p["scaleMode"].as_str() {
                Some("FIT") => "center / contain no-repeat",
                Some("TILE") => "repeat",
                _ => "center / cover no-repeat",
            };
            Some(format!("url('{url}') {fit}"))
        }
        _ => None,
    }
}

/// The CSS background for a node's fills: the top paint first, as CSS wants it.
fn background(n: &Value, w: f64, h: f64, assets: &Assets) -> Option<(&'static str, String)> {
    let layers: Vec<(bool, String)> = paints(&n["fills"])
        .filter_map(|p| Some((ty(p) == "SOLID", layer(p, w, h, assets)?)))
        .collect();
    match layers.as_slice() {
        [] => None,
        [(true, c)] => Some(("background-color", c.clone())),
        _ => {
            let css: Vec<String> = layers
                .iter()
                .enumerate()
                .rev()
                .map(|(i, (solid, l))| {
                    // Only the bottom layer may be a bare colour.
                    if *solid && i != 0 {
                        format!("linear-gradient({l}, {l})")
                    } else {
                        l.clone()
                    }
                })
                .collect();
            Some(("background", css.join(", ")))
        }
    }
}

type Styles = Vec<(&'static str, String)>;

fn sizing(n: &Value, axis: &str) -> &'static str {
    let key = if axis == "h" {
        "layoutSizingHorizontal"
    } else {
        "layoutSizingVertical"
    };
    match n[key].as_str() {
        Some("FILL") => "fill",
        Some("HUG") => "hug",
        _ => "fixed",
    }
}

/// Where a node sits: in its parent's auto layout, or placed in its parent's box.
fn placement(n: &Value, parent: &Value, st: &mut Styles) {
    let (x, y, w, h) = bounds(n);
    let (px0, py0, _, _) = bounds(parent);
    let text = ty(n) == "TEXT";
    let grows = n["textAutoResize"].as_str();
    let row = match parent["layoutMode"].as_str() {
        Some("HORIZONTAL") => Some(true),
        Some("VERTICAL") => Some(false),
        _ => None,
    };
    match row.filter(|_| n["layoutPositioning"].as_str() != Some("ABSOLUTE")) {
        Some(row) => {
            if !text {
                st.push(("position", "relative".into()));
            }
            for (axis, size, along) in [("h", w, row), ("v", h, !row)] {
                let prop = if axis == "h" { "width" } else { "height" };
                match sizing(n, axis) {
                    "fill" if along => {
                        st.push(("flex", "1 1 0".into()));
                        st.push((
                            if axis == "h" {
                                "min-width"
                            } else {
                                "min-height"
                            },
                            "0".into(),
                        ));
                    }
                    "fill" => st.push(("align-self", "stretch".into())),
                    "hug" => {}
                    _ => {
                        let free = text
                            && match grows {
                                Some("WIDTH_AND_HEIGHT") => true,
                                Some("HEIGHT") => axis == "v",
                                _ => false,
                            };
                        if !free {
                            st.push((prop, px(size)));
                        }
                        if along {
                            st.push(("flex-shrink", "0".into()));
                        }
                    }
                }
            }
        }
        None => {
            st.push(("position", "absolute".into()));
            st.push(("left", px(x - px0)));
            st.push(("top", px(y - py0)));
            match (text, grows) {
                (true, Some("WIDTH_AND_HEIGHT")) => {}
                (true, Some("HEIGHT")) => st.push(("width", px(w))),
                _ => {
                    st.push(("width", px(w)));
                    st.push(("height", px(h)));
                }
            }
        }
    }
}

/// A frame's auto layout as flex.
fn auto_layout(n: &Value, st: &mut Styles) {
    let dir = match n["layoutMode"].as_str() {
        Some("HORIZONTAL") => "row",
        Some("VERTICAL") => "column",
        _ => return,
    };
    st.push(("display", "flex".into()));
    st.push(("flex-direction", dir.into()));
    if n["layoutWrap"].as_str() == Some("WRAP") {
        st.push(("flex-wrap", "wrap".into()));
        if let Some(g) = n["counterAxisSpacing"].as_f64().filter(|g| *g > 0.) {
            st.push(("row-gap", px(g)));
        }
    }
    let between = n["primaryAxisAlignItems"].as_str() == Some("SPACE_BETWEEN");
    if let Some(g) = n["itemSpacing"].as_f64().filter(|g| *g != 0. && !between) {
        st.push((
            if dir == "row" {
                "column-gap"
            } else {
                "row-gap"
            },
            px(g),
        ));
    }
    let justify = match n["primaryAxisAlignItems"].as_str() {
        Some("CENTER") => "center",
        Some("MAX") => "flex-end",
        Some("SPACE_BETWEEN") => "space-between",
        _ => "flex-start",
    };
    st.push(("justify-content", justify.into()));
    let align = match n["counterAxisAlignItems"].as_str() {
        Some("CENTER") => "center",
        Some("MAX") => "flex-end",
        Some("BASELINE") => "baseline",
        _ => "flex-start",
    };
    st.push(("align-items", align.into()));
    let pad = |k: &str| n[k].as_f64().unwrap_or(0.);
    let (t, r, b, l) = (
        pad("paddingTop"),
        pad("paddingRight"),
        pad("paddingBottom"),
        pad("paddingLeft"),
    );
    if t + r + b + l > 0. {
        st.push((
            "padding",
            format!("{} {} {} {}", px(t), px(r), px(b), px(l)),
        ));
    }
}

/// Stroke, corners, shadows and blur.
fn decoration(n: &Value, st: &mut Styles) {
    if let Some(s) = paints(&n["strokes"]).find(|p| ty(p) == "SOLID") {
        let c = color(&s["color"], paint_opacity(s));
        let style = if n["strokeDashes"].as_array().is_some_and(|d| !d.is_empty()) {
            "dashed"
        } else {
            "solid"
        };
        let each = &n["individualStrokeWeights"];
        if each.is_object() {
            for (side, k) in [
                ("border-top", "top"),
                ("border-right", "right"),
                ("border-bottom", "bottom"),
                ("border-left", "left"),
            ] {
                let w = each[k].as_f64().unwrap_or(0.);
                if w > 0. {
                    st.push((side, format!("{} {style} {c}", px(w))));
                }
            }
        } else if let Some(w) = n["strokeWeight"].as_f64().filter(|w| *w > 0.) {
            st.push(("border", format!("{} {style} {c}", px(w))));
        }
    }
    if ty(n) == "ELLIPSE" {
        st.push(("border-radius", "50%".into()));
    } else if let Some(r) = n["rectangleCornerRadii"]
        .as_array()
        .filter(|r| r.len() == 4)
    {
        let r: Vec<String> = r.iter().map(|v| px(v.as_f64().unwrap_or(0.))).collect();
        st.push(("border-radius", r.join(" ")));
    } else if let Some(r) = n["cornerRadius"].as_f64().filter(|r| *r > 0.) {
        st.push(("border-radius", px(r)));
    }
    let mut shadows = Vec::new();
    for e in paints(&n["effects"]) {
        let o = |k: &str| e["offset"][k].as_f64().unwrap_or(0.);
        let radius = e["radius"].as_f64().unwrap_or(0.);
        match ty(e) {
            "DROP_SHADOW" | "INNER_SHADOW" => shadows.push(format!(
                "{}{} {} {} {} {}",
                if ty(e) == "INNER_SHADOW" {
                    "inset "
                } else {
                    ""
                },
                px(o("x")),
                px(o("y")),
                px(radius),
                px(e["spread"].as_f64().unwrap_or(0.)),
                color(&e["color"], 1.)
            )),
            "LAYER_BLUR" => st.push(("filter", format!("blur({})", px(radius / 2.)))),
            "BACKGROUND_BLUR" => st.push(("backdrop-filter", format!("blur({})", px(radius / 2.)))),
            _ => {}
        }
    }
    if !shadows.is_empty() {
        st.push(("box-shadow", shadows.join(", ")));
    }
}

fn text_styles(n: &Value, st: &mut Styles) {
    let s = &n["style"];
    if let Some(f) = s["fontFamily"].as_str() {
        st.push((
            "font-family",
            format!("'{}', var(--font)", f.replace(['\'', '"'], "")),
        ));
    }
    if let Some(v) = s["fontSize"].as_f64() {
        st.push(("font-size", px(v)));
    }
    if let Some(v) = s["fontWeight"].as_f64() {
        st.push(("font-weight", num(v)));
    }
    if s["italic"].as_bool() == Some(true) {
        st.push(("font-style", "italic".into()));
    }
    match (s["lineHeightUnit"].as_str(), s["lineHeightPx"].as_f64()) {
        (Some("INTRINSIC_%"), _) | (_, None) => {}
        (_, Some(v)) => st.push(("line-height", px(v))),
    }
    if let Some(v) = s["letterSpacing"].as_f64().filter(|v| *v != 0.) {
        st.push(("letter-spacing", px(v)));
    }
    let align = match s["textAlignHorizontal"].as_str() {
        Some("CENTER") => Some("center"),
        Some("RIGHT") => Some("right"),
        Some("JUSTIFIED") => Some("justify"),
        _ => None,
    };
    if let Some(a) = align {
        st.push(("text-align", a.into()));
    }
    match s["textCase"].as_str() {
        Some("UPPER") => st.push(("text-transform", "uppercase".into())),
        Some("LOWER") => st.push(("text-transform", "lowercase".into())),
        Some("TITLE") => st.push(("text-transform", "capitalize".into())),
        _ => {}
    }
    match s["textDecoration"].as_str() {
        Some("UNDERLINE") => st.push(("text-decoration", "underline".into())),
        Some("STRIKETHROUGH") => st.push(("text-decoration", "line-through".into())),
        _ => {}
    }
    if let Some(c) = paints(&n["fills"]).find(|p| ty(p) == "SOLID") {
        st.push(("color", color(&c["color"], paint_opacity(c))));
    }
    let wrap = if n["textAutoResize"].as_str() == Some("WIDTH_AND_HEIGHT") {
        "pre"
    } else {
        "pre-wrap"
    };
    st.push(("white-space", wrap.into()));
    match s["textAlignVertical"].as_str() {
        Some(v @ ("CENTER" | "BOTTOM")) if n["textAutoResize"].as_str().is_none() => {
            st.push(("display", "flex".into()));
            st.push(("flex-direction", "column".into()));
            st.push((
                "justify-content",
                if v == "CENTER" { "center" } else { "flex-end" }.into(),
            ));
        }
        _ => {}
    }
}

fn style_attr(st: &Styles) -> String {
    st.iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Write a node and what's in it.
fn node(n: &Value, parent: Option<&Value>, assets: &Assets, out: &mut String, depth: usize) {
    if n["visible"].as_bool() == Some(false) {
        return;
    }
    let (_, _, w, h) = bounds(n);
    let mut st: Styles = Vec::new();
    match parent {
        Some(p) => placement(n, p, &mut st),
        None => {
            st.push(("position", "relative".into()));
            st.push(("width", "100%".into()));
            st.push(("min-height", px(h)));
        }
    }
    if let Some(o) = n["opacity"].as_f64().filter(|o| *o < 1.) {
        st.push(("opacity", num(o)));
    }
    let name = esc(n["name"].as_str().unwrap_or(""));
    let pad = "  ".repeat(depth);
    if ty(n) == "TEXT" {
        text_styles(n, &mut st);
        let text = esc(n["characters"].as_str().unwrap_or(""));
        let _ = writeln!(
            out,
            "{pad}<p data-name=\"{name}\" style=\"{}\">{text}</p>",
            style_attr(&st)
        );
        return;
    }
    if is_drawing(n) {
        let svg = n["id"]
            .as_str()
            .and_then(|id| assets.svgs.get(id))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{pad}<div data-name=\"{name}\" style=\"{}\">{svg}</div>",
            style_attr(&st)
        );
        return;
    }
    if let Some((k, v)) = background(n, w, h, assets) {
        st.push((k, v));
    }
    decoration(n, &mut st);
    auto_layout(n, &mut st);
    if n["clipsContent"].as_bool() == Some(true) || parent.is_none() {
        st.push(("overflow", "hidden".into()));
    }
    let tag = if parent.is_none() { "main" } else { "div" };
    let _ = write!(
        out,
        "{pad}<{tag} data-name=\"{name}\" style=\"{}\">",
        style_attr(&st)
    );
    let mut inner = String::new();
    for c in kids(n) {
        node(c, Some(n), assets, &mut inner, depth + 1);
    }
    if inner.is_empty() {
        let _ = writeln!(out, "</{tag}>");
    } else {
        let _ = write!(out, "\n{inner}{pad}</{tag}>\n");
    }
}

// ---- Figma's REST API

/// Where Figma's API is; `RUNO_FIGMA_API` points elsewhere for testing.
fn api() -> String {
    std::env::var("RUNO_FIGMA_API").unwrap_or_else(|_| "https://api.figma.com".into())
}

/// GET from Figma's API. The token goes to curl on stdin so it never shows in the process
/// list.
fn get(url: &str, token: &str) -> Result<Value, String> {
    let mut child = Command::new("curl")
        .args(["-sS", "-m", "90", "-H", "@-", "-w", "\n%{http_code}", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Couldn't reach Figma: curl isn't available.".to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = write!(stdin, "X-Figma-Token: {token}\nAccept: application/json\n");
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("Couldn't reach Figma: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.trim_end().rsplit_once('\n').unwrap_or(("", &text));
    match code.trim().parse::<u16>().unwrap_or(0) {
        200 => {
            serde_json::from_str(body).map_err(|_| "Figma sent back something unreadable.".into())
        }
        0 => Err("Couldn't reach Figma. Check the connection and try again.".into()),
        403 => Err(
            "Figma turned the access token down. Check it in Settings > General > Design.".into(),
        ),
        404 => Err("Figma has no file at that link, or the token can't open it.".into()),
        429 => Err("Figma says that's too many requests for now. Try again in a minute.".into()),
        code => {
            let why = serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|v| {
                    v["err"]
                        .as_str()
                        .or(v["message"].as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            Err(format!("Figma answered {code}. {why}").trim().to_string())
        }
    }
}

/// Download a file Figma links to; those links need no token.
fn download(url: &str) -> Option<Vec<u8>> {
    let out = Command::new("curl")
        .args(["-sSfL", "-m", "90", url])
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// An exported SVG ready to go inline: without its XML line, filling its box.
fn inline_svg(svg: &str) -> String {
    let s = svg.trim();
    let s = match s.find("<svg") {
        Some(i) => &s[i..],
        None => return String::new(),
    };
    s.replacen(
        "<svg",
        "<svg style=\"display: block; width: 100%; height: 100%; overflow: visible\"",
        1,
    )
}

/// Read the frames a Figma link points at and make them into screens, saving their
/// pictures in the design's assets. Runs off the UI thread.
pub(crate) fn import(url: &str, token: &str, design: &Path) -> Result<Vec<Imported>, String> {
    let l =
        link(url).ok_or("That isn't a link to a Figma file. In Figma, use Share > Copy link.")?;
    let base = api();
    let doc = match &l.node {
        Some(id) => {
            let v = get(
                &format!(
                    "{base}/v1/files/{}/nodes?ids={}",
                    l.key,
                    id.replace(':', "%3A")
                ),
                token,
            )?;
            v["nodes"][id]["document"].clone()
        }
        None => get(&format!("{base}/v1/files/{}", l.key), token)?["document"].clone(),
    };
    let frames: Vec<&Value> = frames(&doc).into_iter().take(MAX_FRAMES).collect();
    if frames.is_empty() {
        return Err(
            "There are no frames there to bring in. Link a frame, or a page with frames on it."
                .into(),
        );
    }
    let (mut images, mut drawings) = (Vec::new(), Vec::new());
    for f in &frames {
        wanted(f, &mut images, &mut drawings);
    }
    let mut assets = Assets::default();
    if !images.is_empty() {
        let fills = get(&format!("{base}/v1/files/{}/images", l.key), token)?;
        let dir = design.join(ASSETS);
        let _ = std::fs::create_dir_all(&dir);
        for r in &images {
            let Some(src) = fills["meta"]["images"][r].as_str() else {
                continue;
            };
            let Some(bytes) = download(src) else {
                continue;
            };
            let short: String = r
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(16)
                .collect();
            let name = format!("figma-{short}.png");
            if std::fs::write(dir.join(&name), bytes).is_ok() {
                assets
                    .images
                    .insert(r.clone(), format!("../{ASSETS}/{name}"));
            }
        }
    }
    drawings.truncate(MAX_VECTORS);
    for ids in drawings.chunks(50) {
        let ids = ids.join(",").replace(':', "%3A");
        let Ok(v) = get(
            &format!("{base}/v1/images/{}?ids={ids}&format=svg", l.key),
            token,
        ) else {
            continue;
        };
        for (id, src) in v["images"].as_object().into_iter().flatten() {
            let Some(svg) = src.as_str().and_then(download) else {
                continue;
            };
            assets
                .svgs
                .insert(id.clone(), inline_svg(&String::from_utf8_lossy(&svg)));
        }
    }
    Ok(frames.iter().map(|f| screen(f, &assets)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::prelude::v1::test;

    #[test]
    fn links_give_the_file_and_node() {
        assert_eq!(
            link("https://www.figma.com/design/AbC123/My-app?node-id=12-34&t=x"),
            Some(Link {
                key: "AbC123".into(),
                node: Some("12:34".into())
            })
        );
        assert_eq!(
            link("https://figma.com/file/Key9/Name?node-id=1%3A2"),
            Some(Link {
                key: "Key9".into(),
                node: Some("1:2".into())
            })
        );
        assert_eq!(
            link("https://www.figma.com/design/Main1/branch/Br4nch/Name"),
            Some(Link {
                key: "Br4nch".into(),
                node: None
            })
        );
        assert_eq!(link("https://example.com/design/x"), None);
        assert_eq!(link("https://www.figma.com/community/file/1"), None);
    }

    fn bb(x: f64, y: f64, w: f64, h: f64) -> Value {
        json!({"x": x, "y": y, "width": w, "height": h})
    }

    fn sample() -> Value {
        json!({
            "id": "1:1", "type": "FRAME", "name": "Sign in",
            "absoluteBoundingBox": bb(100., 0., 390., 844.),
            "fills": [{"type": "SOLID", "color": {"r": 1, "g": 1, "b": 1, "a": 1}}],
            "children": [
                {"id": "1:2", "type": "TEXT", "name": "Title", "characters": "Hi <you>",
                 "absoluteBoundingBox": bb(124., 40., 200., 30.), "textAutoResize": "WIDTH_AND_HEIGHT",
                 "style": {"fontFamily": "Inter", "fontSize": 24, "fontWeight": 700, "lineHeightPx": 30, "lineHeightUnit": "PIXELS"},
                 "fills": [{"type": "SOLID", "color": {"r": 0, "g": 0, "b": 0, "a": 1}}]},
                {"id": "1:3", "type": "FRAME", "name": "Button", "layoutMode": "HORIZONTAL",
                 "primaryAxisAlignItems": "CENTER", "counterAxisAlignItems": "CENTER",
                 "itemSpacing": 8, "paddingLeft": 16, "paddingRight": 16, "paddingTop": 12, "paddingBottom": 12,
                 "cornerRadius": 12, "absoluteBoundingBox": bb(116., 700., 358., 48.),
                 "fills": [{"type": "SOLID", "color": {"r": 0.851, "g": 0.22, "b": 0.102, "a": 1}}],
                 "effects": [{"type": "DROP_SHADOW", "visible": true, "radius": 8, "offset": {"x": 0, "y": 2}, "color": {"r": 0, "g": 0, "b": 0, "a": 0.25}}],
                 "children": [
                    {"id": "1:4", "type": "VECTOR", "name": "Icon", "absoluteBoundingBox": bb(200., 714., 20., 20.)},
                    {"id": "1:5", "type": "TEXT", "name": "Label", "characters": "Go",
                     "layoutSizingHorizontal": "HUG", "layoutSizingVertical": "HUG",
                     "absoluteBoundingBox": bb(228., 714., 30., 20.), "style": {"fontSize": 16}},
                    {"id": "1:6", "type": "RECTANGLE", "name": "Hidden", "visible": false,
                     "absoluteBoundingBox": bb(0., 0., 1., 1.)}
                 ]},
                {"id": "1:7", "type": "RECTANGLE", "name": "Photo",
                 "absoluteBoundingBox": bb(100., 100., 390., 200.),
                 "fills": [{"type": "IMAGE", "imageRef": "abc", "scaleMode": "FILL"}]}
            ]
        })
    }

    #[test]
    fn a_frame_becomes_a_screen() {
        let f = sample();
        let mut assets = Assets::default();
        assets
            .images
            .insert("abc".into(), "../assets/figma-abc.png".into());
        assets.svgs.insert("1:4".into(), "<svg></svg>".into());
        let s = screen(&f, &assets);
        assert_eq!(s.name, "Sign in");
        assert_eq!(s.device, Device::Phone);
        let b = &s.body;
        assert!(b.starts_with("<main data-name=\"Sign in\" style=\"position: relative; width: 100%; min-height: 844px; background-color: #FFFFFF; overflow: hidden\">"), "{b}");
        // Placed where it sits in the frame, sized by its text.
        assert!(b.contains("<p data-name=\"Title\" style=\"position: absolute; left: 24px; top: 40px; font-family: 'Inter', var(--font); font-size: 24px; font-weight: 700; line-height: 30px; color: #000000; white-space: pre\">"), "{b}");
        assert!(b.contains("Hi &lt;you&gt;"));
        // Auto layout as flex.
        assert!(b.contains("display: flex; flex-direction: row; column-gap: 8px; justify-content: center; align-items: center; padding: 12px 16px 12px 16px"), "{b}");
        assert!(
            b.contains("border-radius: 12px; box-shadow: 0px 2px 8px 0px rgba(0, 0, 0, 0.25)"),
            "{b}"
        );
        assert!(b.contains("background-color: #D9381A"), "{b}");
        // In the flow: a fixed drawing keeps its size, hugging text takes none.
        assert!(b.contains("<div data-name=\"Icon\" style=\"position: relative; width: 20px; flex-shrink: 0; height: 20px\"><svg></svg></div>"), "{b}");
        assert!(
            b.contains(
                "<p data-name=\"Label\" style=\"font-size: 16px; white-space: pre-wrap\">Go</p>"
            ),
            "{b}"
        );
        assert!(!b.contains("Hidden"));
        assert!(
            b.contains("background: url('../assets/figma-abc.png') center / cover no-repeat"),
            "{b}"
        );
    }

    #[test]
    fn pages_give_their_frames_and_what_they_need() {
        let page = json!({"type": "CANVAS", "children": [
            sample(),
            {"type": "SECTION", "children": [{"type": "FRAME", "name": "Inner", "children": []}]},
            {"type": "TEXT", "name": "Note"}
        ]});
        let fs = frames(&page);
        assert_eq!(fs.len(), 2);
        let (mut images, mut drawings) = (Vec::new(), Vec::new());
        wanted(fs[0], &mut images, &mut drawings);
        assert_eq!(images, ["abc"]);
        assert_eq!(drawings, ["1:4"]);
        assert_eq!(device(1440.), Device::Desktop);
    }

    #[test]
    fn gradients_point_the_way_figma_draws_them() {
        let p = json!({"type": "GRADIENT_LINEAR",
            "gradientHandlePositions": [{"x": 0.5, "y": 0}, {"x": 0.5, "y": 1}, {"x": 0, "y": 0}],
            "gradientStops": [
                {"position": 0, "color": {"r": 1, "g": 0, "b": 0, "a": 1}},
                {"position": 1, "color": {"r": 0, "g": 0, "b": 1, "a": 0.5}}]});
        assert_eq!(
            layer(&p, 100., 100., &Assets::default()).unwrap(),
            "linear-gradient(180deg, #FF0000 0%, rgba(0, 0, 255, 0.5) 100%)"
        );
    }

    #[test]
    fn exported_svgs_fill_their_box() {
        assert_eq!(
            inline_svg("<?xml version=\"1.0\"?>\n<svg width=\"2\"><path/></svg>"),
            "<svg style=\"display: block; width: 100%; height: 100%; overflow: visible\" width=\"2\"><path/></svg>"
        );
    }
}
