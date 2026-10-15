//! Screens drawn by Runo itself: the HTML and CSS laid out by Blitz (Firefox's CSS engine
//! with Taffy layout) and painted on the CPU, so the canvas needs no browser to show them.
//! What the canvas asks of a drawn screen (its links, its boxes, what the checks look at) is
//! read from the laid-out page here, the way `dom.rs` asks a browser for it.

use super::checks::{Fact, Facts};
use super::dom::{Box as Rect, Bx, Link, Path};
use super::model::{screen_path, Device};
use super::render::Shot;
use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, FontContext, Node, NodeId};
use blitz_html::HtmlDocument;
use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use gpui_kit::RenderImage;
use std::sync::Arc;

/// Pixels per CSS pixel, as the browser draws them.
const SCALE: f32 = 2.;
/// A screen taller than its device is drawn down to this many screens high.
const MAX_SCREENS_TALL: f32 = 4.;
/// Elements that take no space, left out of paths as the page's own helpers do.
const SKIP: &[&str] = &["script", "style", "link", "meta", "title", "template"];
/// What counts as something to tap, for the checks.
const CLICK_TAGS: &[&str] = &["a", "button", "input", "select", "textarea"];

/// Whether screens are drawn by Runo rather than a browser. `RUNO_DESIGN_ENGINE=chrome`
/// goes back to the browser.
pub(crate) fn enabled() -> bool {
    std::env::var("RUNO_DESIGN_ENGINE").map_or(true, |v| v != "chrome")
}

/// Reads `file://` resources (stylesheets, pictures, fonts) straight from disk. Anything
/// else is left out: a screen is drawn from the design folder alone.
struct Files;

impl NetProvider for Files {
    fn fetch(&self, _doc: usize, request: Request, handler: std::boxed::Box<dyn NetHandler>) {
        if request.url.scheme() != "file" {
            return;
        }
        if let Some(bytes) = request
            .url
            .to_file_path()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
        {
            handler.bytes(request.url.to_string(), bytes.into());
        }
    }
}

/// Draw a screen from its file. `fonts` keeps the system's fonts loaded between screens.
pub(crate) fn draw(
    root: &std::path::Path,
    id: &str,
    device: Device,
    dark: bool,
    fonts: &mut Option<FontContext>,
) -> Result<Shot, String> {
    let file = screen_path(root, id);
    let html = std::fs::read_to_string(&file).map_err(|e| format!("{id}.html didn't load: {e}"))?;
    let url = super::render::file_url(&file);
    let font_ctx = fonts.get_or_insert_with(font_context).clone();
    // Blitz is young: a page it can't handle shouldn't take the app down with it.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render(&html, &url, device, dark, font_ctx)
    }))
    .map_err(|_| format!("Couldn't draw {id}.html."))?
}

/// The system's fonts plus Runo's own Inter, which screens ask for by name and which stands
/// in for the system font, so a screen looks the same on every machine.
fn font_context() -> FontContext {
    use parley::fontique::{Blob, GenericFamily};
    static INTER: &[u8] = include_bytes!("../../assets/fonts/InterVariable.ttf");
    static INTER_ITALIC: &[u8] = include_bytes!("../../assets/fonts/InterVariable-Italic.ttf");
    let mut fonts = FontContext::default();
    let mut inter = Vec::new();
    for data in [INTER, INTER_ITALIC] {
        for (family, _) in fonts
            .collection
            .register_fonts(Blob::new(Arc::new(data)), None)
        {
            if !inter.contains(&family) {
                inter.push(family);
            }
        }
    }
    for generic in [GenericFamily::SansSerif, GenericFamily::SystemUi] {
        let rest: Vec<_> = fonts.collection.generic_families(generic).collect();
        fonts
            .collection
            .set_generic_families(generic, inter.iter().copied().chain(rest));
    }
    fonts
}

fn render(
    html: &str,
    url: &str,
    device: Device,
    dark: bool,
    fonts: FontContext,
) -> Result<Shot, String> {
    let (w, h) = device.size();
    let scheme = if dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    };
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url: Some(url.to_string()),
            viewport: Some(Viewport::new(
                (w * SCALE) as u32,
                (h * SCALE) as u32,
                SCALE,
                scheme,
            )),
            net_provider: Some(Arc::new(Files)),
            font_ctx: Some(fonts),
            ..Default::default()
        },
    );
    // Stylesheets and pictures arrive as messages; take them in before laying out.
    doc.handle_messages();
    doc.resolve(0.);
    let page = Page::read(&doc, w);
    show_placeholders(&mut doc);
    let height = page.bottom.clamp(h, h * MAX_SCREENS_TALL);
    let (pw, ph) = ((w * SCALE).round() as u32, (height * SCALE).round() as u32);
    let mut rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| blitz_paint::paint_scene(scene, &mut doc, SCALE as f64, pw, ph, 0, 0),
        pw,
        ph,
    );
    // GPUI draws BGRA.
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let frame = image::RgbaImage::from_raw(pw, ph, rgba)
        .ok_or_else(|| "The screen drew at the wrong size.".to_string())?;
    Ok(Shot {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(frame)])),
        size: (w, height),
        links: page.links,
        facts: Some(page.facts),
        boxes: page.boxes,
    })
}

/// Blitz doesn't draw an empty field's placeholder, so write it in as the field's text, in a
/// fainter colour, the way a browser shows it. This happens after the page is read, so the
/// checks still see the field as empty.
fn show_placeholders(doc: &mut BaseDocument) {
    fn empty_fields(doc: &BaseDocument, id: NodeId, out: &mut Vec<(NodeId, String, String)>) {
        let Some(node) = doc.get_node(id) else {
            return;
        };
        if matches!(tag(node).as_deref(), Some("input" | "textarea"))
            && attr(node, "value").is_none_or(str::is_empty)
            && !matches!(
                attr(node, "type"),
                Some("checkbox" | "radio" | "hidden" | "range" | "color" | "file")
            )
        {
            if let Some(text) = attr(node, "placeholder").filter(|t| !t.is_empty()) {
                let style = attr(node, "style").unwrap_or_default().to_string();
                out.push((id, text.to_string(), style));
            }
        }
        for &child in &node.children {
            empty_fields(doc, child, out);
        }
    }
    let mut fields = Vec::new();
    empty_fields(doc, doc.root_element().id, &mut fields);
    if fields.is_empty() {
        return;
    }
    {
        let mut edit = doc.mutate();
        for (id, text, style) in fields {
            edit.set_attribute(id, blitz_dom::qual_name!("value"), &text);
            let faint = format!("{style};color:color-mix(in srgb,currentColor 55%,transparent)");
            edit.set_attribute(id, blitz_dom::qual_name!("style"), &faint);
        }
    }
    doc.resolve(0.);
}

/// What the canvas reads from a laid-out page.
#[derive(Default)]
struct Page {
    links: Vec<Link>,
    boxes: Vec<Bx>,
    facts: Facts,
    /// The bottom of the content, in CSS pixels.
    bottom: f32,
}

impl Page {
    fn read(doc: &BaseDocument, width: f32) -> Self {
        let mut page = Page {
            facts: Facts {
                width,
                scroll: width,
                items: Vec::new(),
            },
            ..Default::default()
        };
        let root = doc.root_element();
        page.bottom = root.final_layout().size.height;
        if let Some(body) = find(doc, root.id, "body") {
            page.walk(doc, body, &mut Vec::new(), &mut Vec::new(), false, None);
        }
        page
    }

    /// Visit the elements under `parent` in document order. `path` is each element's path
    /// as the page helpers name it, `fact_path` as the checks name it (they also leave out
    /// `<br>`). Inside something that clips, nothing adds to how wide or tall the page is.
    fn walk(
        &mut self,
        doc: &BaseDocument,
        parent: NodeId,
        path: &mut Path,
        fact_path: &mut Path,
        clipped: bool,
        click: Option<()>,
    ) {
        let Some(node) = doc.get_node(parent) else {
            return;
        };
        let (mut i, mut fi) = (0, 0);
        for &child in &node.children {
            let Some(el) = doc.get_node(child) else {
                continue;
            };
            let Some(tag) = tag(el) else {
                continue;
            };
            if SKIP.contains(&tag.as_str()) {
                continue;
            }
            path.push(i);
            i += 1;
            let br = tag == "br";
            if !br {
                fact_path.push(fi);
                fi += 1;
            }
            // Inside something hidden, Stylo gives no style: nothing here is drawn, and
            // asking for its style anyway trips Stylo's checks.
            if el.primary_styles().is_none() {
                path.pop();
                if !br {
                    fact_path.pop();
                }
                continue;
            }
            let style = |k: &str| doc.resolved_style_value(child, k);
            let display = style("display");
            let shown = display != "none" && style("visibility") != "hidden";
            let rect = doc.get_client_bounding_rect(child).map(|r| Rect {
                x: r.x as f32,
                y: r.y as f32,
                w: r.width as f32,
                h: r.height as f32,
            });
            let sized = rect.as_ref().filter(|r| r.w > 0. && r.h > 0.);
            if let Some(goto) = attr(el, "data-goto") {
                if let Some(r) = &rect {
                    self.links.push(Link {
                        goto: goto.trim_end_matches(".html").to_string(),
                        rect: r.clone(),
                        path: path.clone(),
                        transition: attr(el, "data-transition").unwrap_or_default().to_string(),
                        speed: attr(el, "data-speed").unwrap_or_default().to_string(),
                    });
                }
            }
            let is_click = clickable(el, &tag);
            if let (true, Some(r)) = (shown, sized) {
                if !clipped {
                    self.bottom = self.bottom.max(r.y + r.h);
                    self.facts.scroll = self.facts.scroll.max(r.x + r.w);
                }
                if self.boxes.len() < 1500 {
                    let row = (display.contains("flex")
                        && style("flex-direction").starts_with("row"))
                        || display.contains("grid")
                        || display == "inline";
                    self.boxes.push(Bx {
                        path: path.clone(),
                        rect: r.clone(),
                        row,
                    });
                }
                if !br && self.facts.items.len() < 800 {
                    if let Some(f) = fact(doc, child, el, &tag, r, is_click && click.is_none()) {
                        self.facts.items.push(Fact {
                            path: fact_path.clone(),
                            ..f
                        });
                    }
                }
            }
            if shown {
                let clips = clipped
                    || matches!(
                        style("overflow-x").as_str(),
                        "hidden" | "clip" | "auto" | "scroll"
                    );
                let click = click.or(is_click.then_some(()));
                self.walk(doc, child, path, fact_path, clips, click);
            }
            path.pop();
            if !br {
                fact_path.pop();
            }
        }
    }
}

/// The first element called `name` under `from`, depth first.
fn find(doc: &BaseDocument, from: NodeId, name: &str) -> Option<NodeId> {
    let node = doc.get_node(from)?;
    if tag(node).as_deref() == Some(name) {
        return Some(from);
    }
    node.children.iter().find_map(|&c| find(doc, c, name))
}

fn tag(node: &Node) -> Option<String> {
    node.element_data()
        .map(|e| e.name.local.to_string().to_ascii_lowercase())
}

/// An attribute's value by name.
fn attr<'a>(el: &'a Node, name: &str) -> Option<&'a str> {
    el.attrs()?
        .iter()
        .find(|a| &*a.name.local == name)
        .map(|a| a.value.as_str())
}

fn clickable(el: &Node, tag: &str) -> bool {
    CLICK_TAGS.contains(&tag)
        || attr(el, "data-goto").is_some()
        || attr(el, "onclick").is_some()
        || attr(el, "role") == Some("button")
}

/// What the checks need to know about an element, or None when there's nothing to check.
fn fact(
    doc: &BaseDocument,
    id: NodeId,
    el: &Node,
    tag: &str,
    rect: &Rect,
    click: bool,
) -> Option<Fact> {
    let own: String = el
        .children
        .iter()
        .filter_map(|&c| doc.get_node(c)?.text_data().map(|t| t.content.clone()))
        .collect();
    let own = own.trim();
    let img = tag == "img";
    let raw = raw_colours(attr(el, "style").unwrap_or_default());
    if own.is_empty() && !click && !img && raw.is_empty() {
        return None;
    }
    let style = |k: &str| doc.resolved_style_value(id, k);
    let classes: Vec<&str> = attr(el, "class")
        .unwrap_or_default()
        .split_whitespace()
        .take(2)
        .collect();
    let label = if classes.is_empty() {
        tag.to_string()
    } else {
        format!("{tag}.{}", classes.join("."))
    };
    let text = (!own.is_empty()).then(|| el.text_content().trim().chars().take(40).collect());
    Some(Fact {
        path: Vec::new(),
        label,
        rect: rect.clone(),
        text,
        color: style("color"),
        background: (!own.is_empty()).then(|| behind(doc, id)).flatten(),
        size: style("font-size")
            .trim_end_matches("px")
            .parse()
            .unwrap_or(0.),
        weight: style("font-weight").parse().unwrap_or(400),
        click,
        inline: style("display") == "inline",
        img,
        alt: img.then(|| attr(el, "alt").map(String::from)).flatten(),
        raw,
        goto: attr(el, "data-goto").map(|g| g.trim_end_matches(".html").to_string()),
    })
}

/// The colour behind an element: the first ancestor with a background colour. A picture
/// or gradient behind it can't be judged.
fn behind(doc: &BaseDocument, mut id: NodeId) -> Option<String> {
    loop {
        let node = doc.get_node(id)?;
        if node.is_element() {
            let image = doc.resolved_style_value(id, "background-image");
            if !image.is_empty() && image != "none" {
                return None;
            }
            let c = doc.resolved_style_value(id, "background-color");
            if !c.is_empty() && c != "transparent" && !c.replace(' ', "").ends_with(",0)") {
                return Some(c);
            }
        }
        match node.parent {
            Some(p) => id = p,
            None => return Some("rgb(255, 255, 255)".into()),
        }
    }
}

/// Colours written straight into an element's `style` rather than taken from the theme.
fn raw_colours(style: &str) -> Vec<String> {
    let mut out = Vec::new();
    for decl in style.split(';') {
        let Some((k, v)) = decl.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        if ["color", "background-color", "background", "border-color"].contains(&k.as_str())
            && !v.contains("var(")
            && (v.contains('#') || v.contains("rgb") || v.contains("hsl"))
            && !out.contains(&k)
        {
            out.push(k);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn raw_colours_are_the_ones_written_in() {
        assert_eq!(
            raw_colours("color: #fff; background: var(--x); border-color: rgb(1,2,3)"),
            vec!["color".to_string(), "border-color".to_string()]
        );
    }

    #[test]
    fn screens_draw_without_a_browser() {
        let root = std::env::temp_dir().join(format!("runo-native-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        super::super::model::seed(&root).unwrap();
        let html = super::super::model::template("Sign in", "Sign in", Device::Phone).replace(
            "<button class=\"button primary\">",
            "<button class=\"button primary\" data-goto=\"home.html\">",
        );
        std::fs::write(screen_path(&root, "sign-in"), html).unwrap();
        let shot = draw(&root, "sign-in", Device::Phone, false, &mut None).unwrap();
        assert_eq!(shot.size.0, 390.);
        assert_eq!(shot.links.len(), 1);
        assert_eq!(shot.links[0].goto, "home");
        let r = &shot.links[0].rect;
        assert!(r.w > 100. && r.h > 20., "the button is laid out: {r:?}");
        // The button is one of the boxes, at the same place, and the checks see its text.
        assert!(shot.boxes.iter().any(|b| b.path == shot.links[0].path));
        let facts = shot.facts.unwrap();
        assert!(facts
            .items
            .iter()
            .any(|f| f.click && f.goto.as_deref() == Some("home")));
        // Something was painted: not every pixel is the same.
        let px = shot.image.as_bytes(0).unwrap();
        assert!(px.chunks(4).any(|p| p != &px[..4]));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_fields_placeholder_is_drawn() {
        let page = |input: &str| format!("<!doctype html><body style=\"margin:0\">{input}</body>");
        let draw_page = |html: String| {
            render(
                &html,
                "file:///x.html",
                Device::Phone,
                false,
                font_context(),
            )
            .unwrap()
        };
        let blank = draw_page(page("<input style=\"width:300px\">"));
        let hinted = draw_page(page("<input style=\"width:300px\" placeholder=\"Search\">"));
        assert_ne!(
            blank.image.as_bytes(0).unwrap(),
            hinted.image.as_bytes(0).unwrap(),
            "the placeholder shows"
        );
    }
}
