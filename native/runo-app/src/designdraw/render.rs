//! Screens drawn by a headless Chromium of the canvas's own: one page, loaded with the
//! screen being worked on, captured as an image for the canvas and asked about elements.
//!
//! Every call blocks, so callers run them off the UI thread, one at a time.

use super::dom::{self, Link};
use super::model::{screen_path, Device};
use crate::browser_engine::{self, Engine};
use gpui_kit::RenderImage;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The canvas's one page in its engine.
const PAGE: u64 = 1;
/// Pixels per CSS pixel in captures, so screens stay sharp when zoomed in.
const SCALE: f32 = 2.;
/// A screen taller than its device is captured down to this many screens high.
const MAX_SCREENS_TALL: f32 = 4.;

/// A screen as last drawn.
#[derive(Clone)]
pub(crate) struct Shot {
    pub image: Arc<RenderImage>,
    /// The CSS size drawn: the device's width, and its height or the page's if taller.
    pub size: (f32, f32),
    pub links: Vec<Link>,
    /// What the design checks look at, when the page could say.
    pub facts: Option<super::checks::Facts>,
    /// Every element shown, for finding what's under the pointer.
    pub boxes: Vec<super::dom::Bx>,
}

pub(crate) struct Renderer {
    root: PathBuf,
    engine: Option<Arc<Engine>>,
    /// The screen loaded in the page, and whether the page shows it as saved.
    current: Option<String>,
    dark: bool,
}

impl Renderer {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            engine: None,
            current: None,
            dark: false,
        }
    }

    fn engine(&mut self) -> Result<Arc<Engine>, String> {
        if let Some(e) = self.engine.as_ref().filter(|e| e.alive()) {
            return Ok(e.clone());
        }
        self.current = None;
        let path = browser_engine::find_chromium()
            .ok_or_else(|| browser_engine::not_found_message().to_string())?;
        let profile = crate::store::data_dir()
            .join("browser")
            .join("design-profile");
        let engine = Arc::new(Engine::start(path, &profile)?);
        engine.open(PAGE, "about:blank", (390, 844, SCALE));
        let start = std::time::Instant::now();
        while !engine.ready(PAGE) {
            if start.elapsed() > Duration::from_secs(15) || !engine.alive() {
                return Err("The browser that draws the screens didn't start.".into());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        self.engine = Some(engine.clone());
        Ok(engine)
    }

    /// Load the screen unless it's loaded already (or `fresh`, to read the file again).
    fn open(&mut self, id: &str, device: Device, fresh: bool) -> Result<Arc<Engine>, String> {
        let engine = self.engine()?;
        if !fresh && self.current.as_deref() == Some(id) {
            return Ok(engine);
        }
        self.current = None;
        let (w, h) = device.size();
        let url = file_url(&screen_path(&self.root, id));
        if !engine.load(
            PAGE,
            &url,
            (w as u32, h as u32, SCALE),
            Duration::from_secs(8),
        ) {
            return Err(format!("{id}.html didn't load."));
        }
        engine.color_scheme(PAGE, self.dark);
        self.current = Some(id.to_string());
        Ok(engine)
    }

    /// Draw the screen from its file.
    pub(crate) fn shoot(&mut self, id: &str, device: Device) -> Result<Shot, String> {
        let engine = self.open(id, device, true)?;
        let (w, h) = device.size();
        let page_h = engine
            .eval(PAGE, &dom::call("size", &[]))
            .and_then(|v| v["h"].as_f64())
            .unwrap_or(h as f64) as f32;
        let height = page_h.clamp(h, h * MAX_SCREENS_TALL);
        let image = engine
            .capture(PAGE, w, height)
            .ok_or_else(|| format!("Couldn't draw {id}.html."))?;
        let links = engine
            .eval(PAGE, &dom::call("links", &[]))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let facts = engine
            .eval(PAGE, super::checks::FACTS)
            .and_then(|v| serde_json::from_value(v).ok());
        let boxes = engine
            .eval(PAGE, &dom::call("boxes", &[]))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Ok(Shot {
            image,
            size: (w, height),
            links,
            facts,
            boxes,
        })
    }

    /// Ask the screen's page something.
    pub(crate) fn eval(&mut self, id: &str, device: Device, js: &str) -> Result<Value, String> {
        let engine = self.open(id, device, false)?;
        engine
            .eval(PAGE, js)
            .ok_or_else(|| "The screen didn't answer.".to_string())
    }

    /// Change the screen with `js`, save it, and draw it again. Gives the HTML from before,
    /// for undo, or None when the change did nothing.
    pub(crate) fn edit(
        &mut self,
        id: &str,
        device: Device,
        js: &str,
    ) -> Result<Option<(String, Shot)>, String> {
        let before = self.html(id, device)?;
        let done = self.eval(id, device, js)?;
        if done.is_null() {
            return Ok(None);
        }
        let after = self.html(id, device)?;
        if after == before {
            return Ok(None);
        }
        self.write(id, &after)?;
        Ok(Some((before, self.shoot(id, device)?)))
    }

    /// The screen's HTML as the page holds it now.
    fn html(&mut self, id: &str, device: Device) -> Result<String, String> {
        self.eval(id, device, &dom::call("html", &[]))?
            .as_str()
            .map(String::from)
            .ok_or_else(|| "The screen didn't answer.".to_string())
    }

    /// Save HTML as the screen's file; the page reads it on its next load.
    pub(crate) fn write(&mut self, id: &str, html: &str) -> Result<(), String> {
        let file = screen_path(&self.root, id);
        std::fs::write(&file, html)
            .map_err(|e| format!("Couldn't save {}: {e}", file.display()))?;
        if self.current.as_deref() == Some(id) {
            self.current = None;
        }
        Ok(())
    }

    /// Prefer the dark colour scheme in every screen drawn from now on.
    pub(crate) fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        self.current = None;
    }

    /// Forget what the page holds, after a file changed outside the canvas.
    pub(crate) fn forget(&mut self) {
        self.current = None;
    }
}

/// A `file://` URL for a local path.
pub(crate) fn file_url(path: &std::path::Path) -> String {
    let mut out = String::from("file://");
    let text = path.to_string_lossy().replace('\\', "/");
    if !text.starts_with('/') {
        out.push('/');
    }
    for c in text.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '%' => out.push_str("%25"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws, picks and edits a real screen. Needs a Chromium, so it runs only when
    /// RUNO_BROWSER_TESTS is set.
    #[test]
    fn screens_draw_pick_and_edit() {
        if std::env::var_os("RUNO_BROWSER_TESTS").is_none() {
            return;
        }
        let root = std::env::temp_dir().join(format!("runo-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        super::super::model::seed(&root).unwrap();
        let html = super::super::model::template("Sign in", "Sign in", Device::Phone).replace(
            "<button class=\"button primary\">",
            "<button class=\"button primary\" data-goto=\"home\">",
        );
        std::fs::write(screen_path(&root, "sign-in"), html).unwrap();
        let mut r = Renderer::new(root.clone());
        let shot = r.shoot("sign-in", Device::Phone).unwrap();
        assert_eq!(shot.size.0, 390.);
        assert_eq!(shot.links.len(), 1);
        assert_eq!(shot.links[0].goto, "home");
        let b = &shot.links[0].rect;
        let v = r
            .eval(
                "sign-in",
                Device::Phone,
                &dom::pick(b.x + b.w / 2., b.y + b.h / 2.),
            )
            .unwrap();
        let el: dom::Element = serde_json::from_value(v).unwrap();
        assert_eq!(el.tag, "button");
        let (before, _) = r
            .edit(
                "sign-in",
                Device::Phone,
                &dom::op(&el.path, "text", &[serde_json::json!("Log in")]),
            )
            .unwrap()
            .unwrap();
        assert!(before.contains(">Sign in</button>"));
        let saved = std::fs::read_to_string(screen_path(&root, "sign-in")).unwrap();
        assert!(saved.contains(">Log in</button>"));
    }

    #[test]
    fn file_urls_are_escaped() {
        assert_eq!(
            file_url(std::path::Path::new("/a b/#1/x.html")),
            "file:///a%20b/%231/x.html"
        );
    }
}
