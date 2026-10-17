//! Screens drawn by an engine of the canvas's own: one page, loaded with the screen being
//! worked on, captured as an image for the canvas and asked about elements in JavaScript.
//! On macOS that's WebKit, the engine built into the system; elsewhere a headless Chromium.
//!
//! Every call blocks, so callers run them off the UI thread, one at a time.

use super::dom::{self, Link};
use super::model::{screen_path, Device};
use gpui_kit::RenderImage;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A screen taller than its device is captured down to this many screens high.
const MAX_SCREENS_TALL: f32 = 4.;

/// Where a project's screen browser keeps its profile. Each project's canvas runs its own
/// browser, and Chromium won't start a second one on a profile that's in use, so every
/// project gets its own, and so does every run of Runo: a browser left behind by a run
/// that crashed or was force-quit can't block the next one.
#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn profile_dir(root: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut h);
    profiles_dir().join(format!("{:016x}-{}", h.finish(), std::process::id()))
}

#[cfg(any(not(target_os = "macos"), test))]
fn profiles_dir() -> PathBuf {
    crate::store::data_dir()
        .join("browser")
        .join("design-profiles")
}

/// Remove the profiles of earlier runs whose browser has quit, so they don't pile up.
#[cfg(not(target_os = "macos"))]
fn clear_old_profiles() {
    let own = format!("-{}", std::process::id());
    for entry in std::fs::read_dir(profiles_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Chromium removes its lock when it quits.
        let in_use = std::fs::symlink_metadata(path.join("SingletonLock")).is_ok();
        if !name.ends_with(&own) && !in_use {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// A screen drawn for Smart animate, and how tall it's drawn.
pub(crate) type Plate = (Arc<RenderImage>, f32);

/// What a Smart animate move is drawn from: both screens without the layers it carries,
/// and those layers alone, see-through around them, as they look where they land.
pub(crate) struct Plates {
    pub old: Plate,
    pub new: Plate,
    pub layers: Option<Arc<RenderImage>>,
}

impl Plates {
    pub(crate) fn images(self) -> Vec<Arc<RenderImage>> {
        [self.old.0, self.new.0]
            .into_iter()
            .chain(self.layers)
            .collect()
    }
}

/// A picture drawn on white and again on black made one, with the page see-through: what
/// shows through of the white says how see-through each pixel is.
fn matte(white: &RenderImage, black: &RenderImage) -> Option<Arc<RenderImage>> {
    let (w, b) = (white.as_bytes(0)?, black.as_bytes(0)?);
    let size = white.size(0);
    if w.len() != b.len() || size != black.size(0) {
        return None;
    }
    let mut out = vec![0u8; w.len()];
    for ((o, pw), pb) in out
        .chunks_exact_mut(4)
        .zip(w.chunks_exact(4))
        .zip(b.chunks_exact(4))
    {
        if pw[..3] == pb[..3] {
            // The same on both: solid.
            o.copy_from_slice(pb);
            o[3] = 255;
            continue;
        }
        let through = (pw[0].saturating_sub(pb[0]) as u32
            + pw[1].saturating_sub(pb[1]) as u32
            + pw[2].saturating_sub(pb[2]) as u32)
            / 3;
        let a = 255 - through;
        for i in 0..3 {
            o[i] = (pb[i] as u32 * 255).checked_div(a).unwrap_or(0).min(255) as u8;
        }
        o[3] = a as u8;
    }
    let rgba = image::RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, out)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

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

/// The page the screens are drawn in, in WebKit.
#[cfg(target_os = "macos")]
struct Surface(super::webkit::Page);

#[cfg(target_os = "macos")]
impl Surface {
    fn start(_root: &Path) -> Result<Self, String> {
        super::webkit::Page::new(390., 844.).map(Self)
    }

    fn alive(&self) -> bool {
        true
    }

    fn load(&mut self, file: &Path, folder: &Path, size: (f32, f32), wait: Duration) -> bool {
        self.0.load(file, folder, size, wait)
    }

    fn eval(&self, js: &str) -> Option<Value> {
        self.0.eval(js)
    }

    fn color_scheme(&self, dark: bool) {
        self.0.color_scheme(dark)
    }

    fn capture(&mut self, w: f32, h: f32) -> Option<Arc<RenderImage>> {
        self.0.capture(w, h)
    }
}

/// The page the screens are drawn in, in a headless Chromium.
#[cfg(not(target_os = "macos"))]
struct Surface(Arc<crate::browser_engine::Engine>);

#[cfg(not(target_os = "macos"))]
impl Surface {
    /// The canvas's one page in its engine.
    const PAGE: u64 = 1;
    /// Pixels per CSS pixel in captures, so screens stay sharp when zoomed in.
    const SCALE: f32 = 2.;

    fn start(root: &Path) -> Result<Self, String> {
        use crate::browser_engine::{self, Engine};
        let path = browser_engine::find_chromium()
            .ok_or_else(|| browser_engine::not_found_message().to_string())?;
        clear_old_profiles();
        let profile = profile_dir(root);
        let engine = Arc::new(Engine::start(path, &profile)?);
        engine.open(Self::PAGE, "about:blank", (390, 844, Self::SCALE));
        let start = std::time::Instant::now();
        while !engine.ready(Self::PAGE) {
            if start.elapsed() > Duration::from_secs(30) || !engine.alive() {
                return Err("The browser that draws the screens didn't start.".into());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        Ok(Self(engine))
    }

    fn alive(&self) -> bool {
        self.0.alive()
    }

    fn load(&mut self, file: &Path, _folder: &Path, (w, h): (f32, f32), wait: Duration) -> bool {
        self.0.load(
            Self::PAGE,
            &file_url(file),
            (w as u32, h as u32, Self::SCALE),
            wait,
        )
    }

    fn eval(&self, js: &str) -> Option<Value> {
        self.0.eval(Self::PAGE, js)
    }

    fn color_scheme(&self, dark: bool) {
        self.0.color_scheme(Self::PAGE, dark)
    }

    fn capture(&mut self, w: f32, h: f32) -> Option<Arc<RenderImage>> {
        self.0.capture(Self::PAGE, w, h)
    }
}

/// Changes the canvas asks for without waiting on the renderer, which can be busy loading
/// a page for seconds. The renderer takes them up before its next job.
#[derive(Default)]
pub(crate) struct Asked {
    forget: AtomicBool,
    /// 0 for no change, 1 for light, 2 for dark.
    dark: AtomicU8,
}

impl Asked {
    /// Forget what the page holds, after a file changed outside the canvas.
    pub(crate) fn forget(&self) {
        self.forget.store(true, Ordering::Release);
    }

    /// Prefer the dark colour scheme in every screen drawn from now on.
    pub(crate) fn set_dark(&self, dark: bool) {
        self.dark.store(1 + dark as u8, Ordering::Release);
    }
}

pub(crate) struct Renderer {
    root: PathBuf,
    asked: Arc<Asked>,
    surface: Option<Surface>,
    /// The screen loaded in the page, and whether the page shows it as saved.
    current: Option<String>,
    dark: bool,
    /// Play tapped the page, so it no longer shows the file as saved.
    played: bool,
}

impl Renderer {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            asked: Arc::default(),
            surface: None,
            current: None,
            dark: false,
            played: false,
        }
    }

    /// Where the canvas leaves changes for this renderer without locking it.
    pub(crate) fn asked(&self) -> Arc<Asked> {
        self.asked.clone()
    }

    /// Take up what the canvas asked for since the last job.
    fn catch_up(&mut self) {
        match self.asked.dark.swap(0, Ordering::AcqRel) {
            0 => {}
            v => self.set_dark(v == 2),
        }
        if self.asked.forget.swap(false, Ordering::AcqRel) {
            self.forget();
        }
    }

    fn surface(&mut self) -> Result<&mut Surface, String> {
        if !self.surface.as_ref().is_some_and(Surface::alive) {
            self.current = None;
            self.surface = Some(Surface::start(&self.root)?);
        }
        Ok(self.surface.as_mut().expect("just started"))
    }

    /// Load the screen unless it's loaded already (or `fresh`, to read the file again).
    fn open(&mut self, id: &str, device: Device, fresh: bool) -> Result<&mut Surface, String> {
        let fresh = fresh || std::mem::take(&mut self.played);
        self.open_as_is(id, device, fresh)
    }

    /// Load the screen unless it's loaded already, keeping what Play did to it.
    fn open_as_is(
        &mut self,
        id: &str,
        device: Device,
        fresh: bool,
    ) -> Result<&mut Surface, String> {
        self.catch_up();
        if fresh || self.current.as_deref() != Some(id) {
            self.current = None;
            let file = screen_path(&self.root, id);
            let folder = super::model::folder(&self.root);
            let dark = self.dark;
            let surface = self.surface()?;
            if !surface.load(&file, &folder, device.size(), Duration::from_secs(8)) {
                return Err(format!("{id}.html didn't load."));
            }
            surface.color_scheme(dark);
            self.current = Some(id.to_string());
        }
        self.surface()
    }

    /// Draw a screen from another file, such as a version kept of it. The page goes back
    /// to the canvas's screens on the next job.
    pub(crate) fn shoot_file(&mut self, file: &Path, device: Device) -> Result<Shot, String> {
        self.catch_up();
        self.current = None;
        let folder = file
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let dark = self.dark;
        let surface = self.surface()?;
        if !surface.load(file, &folder, device.size(), Duration::from_secs(8)) {
            return Err(format!("{} didn't load.", file.display()));
        }
        surface.color_scheme(dark);
        let name = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("screen");
        Self::capture(surface, name, device)
    }

    /// Draw the screen from its file.
    pub(crate) fn shoot(&mut self, id: &str, device: Device) -> Result<Shot, String> {
        let engine = self.open(id, device, true)?;
        Self::capture(engine, id, device)
    }

    /// Tap the screen at a point in Play, as a person would, and draw what it shows then.
    /// The page keeps its state between taps on the same screen; anything else that
    /// reads the page loads the file again first, so Play never ends up saved.
    /// `before` are the taps made on this visit already, made again when the page had to
    /// be loaded afresh.
    pub(crate) fn tap(
        &mut self,
        id: &str,
        device: Device,
        before: &[(f32, f32)],
        at: (f32, f32),
    ) -> Result<Shot, String> {
        self.catch_up();
        let fresh = self.current.as_deref() != Some(id) || !self.played;
        self.played = true;
        let engine = self.open_as_is(id, device, fresh)?;
        engine.eval(super::model::UI_JS);
        if fresh {
            for p in before {
                Self::click(engine, *p);
            }
        }
        Self::click(engine, at);
        // Let CSS transitions settle.
        std::thread::sleep(Duration::from_millis(200));
        Self::capture(engine, id, device)
    }

    fn click(engine: &mut Surface, at: (f32, f32)) {
        engine.eval(&format!(
            "(() => {{ const el = document.elementFromPoint({}, {}); \
             if (!el || el.closest('[data-goto]')) return false; \
             if (el.focus) el.focus(); el.click(); return true; }})()",
            at.0, at.1
        ));
    }

    /// How tall the page is drawn: the device's height, or the page's if taller.
    fn height(engine: &mut Surface, device: Device) -> f32 {
        let h = device.size().1;
        let page_h = engine
            .eval(&dom::call("size", &[]))
            .and_then(|v| v["h"].as_f64())
            .unwrap_or(h as f64) as f32;
        page_h.clamp(h, h * MAX_SCREENS_TALL)
    }

    /// The page as it is, without the elements at `hide`, and how tall it's drawn.
    fn plate(engine: &mut Surface, device: Device, hide: &[dom::Path]) -> Option<Plate> {
        engine.eval(&dom::call("hide", &[serde_json::json!(hide), true.into()]));
        let height = Self::height(engine, device);
        let image = engine.capture(device.size().0, height);
        engine.eval(&dom::call("hide", &[serde_json::json!(hide), false.into()]));
        Some((image?, height))
    }

    /// Only the elements at `show`, see-through around them.
    fn cutout(
        engine: &mut Surface,
        device: Device,
        show: &[dom::Path],
        height: f32,
    ) -> Option<Arc<RenderImage>> {
        let w = device.size().0;
        let mut on = |bg: &str| {
            engine.eval(&dom::call("solo", &[serde_json::json!(show), bg.into()]));
            engine.capture(w, height)
        };
        let (white, black) = (on("#fff"), on("#000"));
        engine.eval(&dom::call("solo", &[serde_json::json!(show), Value::Null]));
        matte(&*white?, &*black?)
    }

    /// The screen landed on drawn without `hide` and with only `hide`.
    fn landing(
        engine: &mut Surface,
        device: Device,
        hide: &[dom::Path],
    ) -> Option<(Plate, Option<Arc<RenderImage>>)> {
        let plate = Self::plate(engine, device, hide)?;
        let layers = Self::cutout(engine, device, hide, plate.1);
        Some((plate, layers))
    }

    /// The two screens of a Smart animate move as saved, each without the layers it
    /// carries, for the layers to move over as one gives way to the other.
    pub(crate) fn plates(
        &mut self,
        from: (&str, Device, &[dom::Path]),
        to: (&str, Device, &[dom::Path]),
    ) -> Result<Plates, String> {
        let engine = self.open(from.0, from.1, true)?;
        let old = Self::plate(engine, from.1, from.2);
        let engine = self.open(to.0, to.1, true)?;
        let new = Self::landing(engine, to.1, to.2);
        old.zip(new)
            .map(|(old, (new, layers))| Plates { old, new, layers })
            .ok_or_else(|| format!("Couldn't draw {}.html.", to.0))
    }

    /// The same for a tap that moves layers about on the screen playing: the screen before
    /// the last of `taps` and after it. The page is left as Play had it.
    pub(crate) fn tap_plates(
        &mut self,
        id: &str,
        device: Device,
        taps: &[(f32, f32)],
        hide: (&[dom::Path], &[dom::Path]),
    ) -> Result<Plates, String> {
        let Some((last, before)) = taps.split_last() else {
            return Err("Nothing was tapped.".into());
        };
        self.catch_up();
        self.played = true;
        let engine = self.open_as_is(id, device, true)?;
        engine.eval(super::model::UI_JS);
        for p in before {
            Self::click(engine, *p);
        }
        if !before.is_empty() {
            std::thread::sleep(Duration::from_millis(200));
        }
        let old = Self::plate(engine, device, hide.0);
        Self::click(engine, *last);
        std::thread::sleep(Duration::from_millis(200));
        let new = Self::landing(engine, device, hide.1);
        old.zip(new)
            .map(|(old, (new, layers))| Plates { old, new, layers })
            .ok_or_else(|| format!("Couldn't draw {id}.html."))
    }

    fn capture(engine: &mut Surface, id: &str, device: Device) -> Result<Shot, String> {
        let w = device.size().0;
        let height = Self::height(engine, device);
        let image = engine
            .capture(w, height)
            .ok_or_else(|| format!("Couldn't draw {id}.html."))?;
        let links = engine
            .eval(&dom::call("links", &[]))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let facts = engine
            .eval(super::checks::FACTS)
            .and_then(|v| serde_json::from_value(v).ok());
        let boxes = engine
            .eval(&dom::call("boxes", &[]))
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
            .eval(js)
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
        Ok(self.edit_giving(id, device, js)?.map(|(b, s, _)| (b, s)))
    }

    /// `edit`, also giving what `js` answered.
    pub(crate) fn edit_giving(
        &mut self,
        id: &str,
        device: Device,
        js: &str,
    ) -> Result<Option<(String, Shot, Value)>, String> {
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
        Ok(Some((before, self.shoot(id, device)?, done)))
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
#[cfg(any(not(target_os = "macos"), test))]
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
    /// RUNO_BROWSER_TESTS is set. Not on macOS: WebKit answers on the main thread's run
    /// loop, which the test runner doesn't turn.
    #[cfg(not(target_os = "macos"))]
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

        // Another project's canvas draws while this one's browser is still running.
        let other = std::env::temp_dir().join(format!("runo-render-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&other);
        super::super::model::seed(&other).unwrap();
        std::fs::write(
            screen_path(&other, "home"),
            super::super::model::template("Blank", "Home", Device::Phone),
        )
        .unwrap();
        let mut r2 = Renderer::new(other.clone());
        assert!(r2.shoot("home", Device::Phone).is_ok());
        assert!(r.shoot("sign-in", Device::Phone).is_ok());
    }

    #[test]
    fn each_project_has_its_own_browser_profile() {
        let a = profile_dir(std::path::Path::new("/p/a"));
        assert_eq!(a, profile_dir(std::path::Path::new("/p/a")));
        assert_ne!(a, profile_dir(std::path::Path::new("/p/b")));
    }

    #[test]
    fn file_urls_are_escaped() {
        assert_eq!(
            file_url(std::path::Path::new("/a b/#1/x.html")),
            "file:///a%20b/%231/x.html"
        );
    }
}

#[cfg(test)]
mod matte_tests {
    use super::matte;
    use gpui_kit::RenderImage;

    fn picture(px: [u8; 4]) -> RenderImage {
        let rgba = image::RgbaImage::from_raw(1, 1, px.to_vec()).unwrap();
        RenderImage::new(vec![image::Frame::new(rgba)])
    }

    #[test]
    fn the_page_comes_out_see_through_and_layers_solid() {
        let clear = matte(&picture([255, 255, 255, 255]), &picture([0, 0, 0, 255])).unwrap();
        assert_eq!(clear.as_bytes(0).unwrap()[3], 0);
        let solid = matte(&picture([40, 80, 120, 255]), &picture([40, 80, 120, 255])).unwrap();
        assert_eq!(solid.as_bytes(0).unwrap(), &[40, 80, 120, 255]);
        // Half see-through: half the white comes through.
        let half = matte(&picture([178, 178, 178, 255]), &picture([50, 50, 50, 255])).unwrap();
        let b = half.as_bytes(0).unwrap();
        assert!((b[3] as i32 - 127).abs() <= 1 && (b[0] as i32 - 100).abs() <= 1);
    }
}
