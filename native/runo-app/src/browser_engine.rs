//! A headless Chromium driven over the DevTools protocol, for the Browser tab.
//!
//! Runo starts the Chrome, Edge, Brave or Chromium already on the machine with
//! `--remote-debugging-pipe`, so CDP runs over two inherited file descriptors (3 and 4)
//! as NUL-separated JSON. Each Runo session gets its own page (target); the one on
//! screen streams JPEG frames that are decoded here, off the UI thread.

use gpui_kit::RenderImage;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where Chromium-based browsers install on macOS, most likely first.
#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
];
#[cfg(not(target_os = "macos"))]
const CANDIDATES: &[&str] = &[
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/microsoft-edge",
    "/usr/bin/brave-browser",
];

/// The first candidate that exists, also looking under `~/Applications`.
pub fn pick_chromium(home: Option<&Path>, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    CANDIDATES.iter().find_map(|c| {
        let system = PathBuf::from(c);
        let user = home.and_then(|h| {
            c.strip_prefix("/Applications/")
                .map(|rest| h.join("Applications").join(rest))
        });
        [Some(system), user]
            .into_iter()
            .flatten()
            .find(|p| exists(p))
    })
}

pub fn find_chromium() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    pick_chromium(home.as_deref(), Path::exists)
}

/// The browser's name from its executable path, e.g. "Google Chrome".
pub fn browser_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Chromium".into())
}

/// One CDP message on the pipe: the JSON followed by a NUL byte.
pub fn frame_message(msg: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(msg).unwrap_or_default();
    bytes.push(0);
    bytes
}

/// Take every complete message off the front of `buf`, leaving a partial one behind.
pub fn split_messages(buf: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(end) = buf.iter().position(|b| *b == 0) {
        let rest = buf.split_off(end + 1);
        let mut msg = std::mem::replace(buf, rest);
        msg.pop();
        if !msg.is_empty() {
            out.push(msg);
        }
    }
    out
}

/// A point in the panel (logical pixels from its top-left) as page CSS pixels, given the
/// panel's size and the size of the page the shown frame was rendered at.
pub fn to_page(local: (f32, f32), panel: (f32, f32), page: (f32, f32)) -> (f64, f64) {
    let scale = |v: f32, from: f32, to: f32| {
        if from <= 0. {
            0.
        } else {
            (v.clamp(0., from) * to / from) as f64
        }
    };
    (
        scale(local.0, panel.0, page.0),
        scale(local.1, panel.1, page.1),
    )
}

/// CDP's modifier bit mask.
pub fn modifier_bits(alt: bool, ctrl: bool, meta: bool, shift: bool) -> u8 {
    alt as u8 | (ctrl as u8) << 1 | (meta as u8) << 2 | (shift as u8) << 3
}

/// A decoded frame: the image GPUI draws and the CSS size of the page it shows.
pub struct Frame {
    pub image: Arc<RenderImage>,
    pub css: (f32, f32),
}

/// What a page shows now, kept current by the reader thread.
#[derive(Default)]
pub struct PageInfo {
    pub url: String,
    pub title: String,
    pub loading: bool,
    /// An `alert`, `confirm` or `prompt` waiting for an answer: (kind, message).
    pub dialog: Option<(String, String)>,
}

struct Page {
    target: String,
    /// The flattened CDP session; empty until attached.
    session: String,
    info: PageInfo,
    viewport: (u32, u32, f32),
}

#[derive(Default)]
struct Shared {
    pages: HashMap<u64, Page>,
    /// The page streaming frames, by Runo session.
    active: Option<u64>,
    frame: Option<Frame>,
    replies: HashMap<u64, Sender<Value>>,
    dead: bool,
}

pub struct Engine {
    child: Child,
    writer: Mutex<std::fs::File>,
    next_id: AtomicU64,
    shared: Arc<Mutex<Shared>>,
    /// Bumped on every new frame or page change, so the UI knows to repaint.
    pub seq: Arc<AtomicU64>,
}

/// Duplicate `fd` above the standard descriptors, close-on-exec, and close the original.
#[cfg(unix)]
fn lift(fd: i32) -> std::io::Result<i32> {
    let high = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
    unsafe { libc::close(fd) };
    if high < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(high)
}

#[cfg(unix)]
fn pipe() -> std::io::Result<(i32, i32)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((lift(fds[0])?, lift(fds[1])?))
}

impl Engine {
    /// Start the browser headless with a Runo-only profile.
    #[cfg(unix)]
    pub fn start(browser: PathBuf, profile: &Path) -> Result<Engine, String> {
        use std::os::fd::FromRawFd;
        use std::os::unix::process::CommandExt;
        let _ = std::fs::create_dir_all(profile);
        let (to_child_r, to_child_w) = pipe().map_err(|e| e.to_string())?;
        let (from_child_r, from_child_w) = pipe().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&browser);
        cmd.args([
            "--headless=new",
            "--remote-debugging-pipe",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-sync",
            "--mute-audio",
            "--hide-scrollbars",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        unsafe {
            cmd.pre_exec(move || {
                // dup2 clears close-on-exec on the copies, so only 3 and 4 reach Chromium.
                if libc::dup2(to_child_r, 3) < 0 || libc::dup2(from_child_w, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn();
        unsafe {
            libc::close(to_child_r);
            libc::close(from_child_w);
        }
        let child = child.map_err(|e| format!("could not start {}: {e}", browser.display()))?;
        let writer = unsafe { std::fs::File::from_raw_fd(to_child_w) };
        let reader = unsafe { std::fs::File::from_raw_fd(from_child_r) };
        let engine = Engine {
            child,
            writer: Mutex::new(writer),
            next_id: AtomicU64::new(1),
            shared: Arc::new(Mutex::new(Shared::default())),
            seq: Arc::new(AtomicU64::new(0)),
        };
        let (shared, seq) = (engine.shared.clone(), engine.seq.clone());
        let writer = engine.writer.lock().unwrap().try_clone().map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("runo-cdp".into())
            .spawn(move || read_loop(reader, writer, shared, seq))
            .map_err(|e| e.to_string())?;
        engine.send("Target.setDiscoverTargets", json!({"discover": true}), None);
        Ok(engine)
    }

    #[cfg(not(unix))]
    pub fn start(_browser: PathBuf, _profile: &Path) -> Result<Engine, String> {
        Err("The built-in browser needs macOS or Linux for now.".into())
    }

    pub fn alive(&self) -> bool {
        !self.shared.lock().unwrap().dead
    }

    /// Fire a command; `session` targets a page, `None` the browser.
    fn send(&self, method: &str, params: Value, session: Option<&str>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.write(id, method, params, session);
        id
    }

    fn write(&self, id: u64, method: &str, params: Value, session: Option<&str>) {
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session.filter(|s| !s.is_empty()) {
            msg["sessionId"] = json!(s);
        }
        let _ = self.writer.lock().unwrap().write_all(&frame_message(&msg));
    }

    /// Send a command and wait for its result.
    fn call(&self, method: &str, params: Value, session: Option<&str>) -> Option<Value> {
        let (tx, rx) = channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.shared.lock().unwrap().replies.insert(id, tx);
        self.write(id, method, params, session);
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }

    fn session_of(&self, key: u64) -> Option<String> {
        let shared = self.shared.lock().unwrap();
        shared
            .pages
            .get(&key)
            .map(|p| p.session.clone())
            .filter(|s| !s.is_empty())
    }

    pub fn has_page(&self, key: u64) -> bool {
        self.shared.lock().unwrap().pages.contains_key(&key)
    }

    /// Open `url` in the session's page, creating the page the first time.
    pub fn open(self: &Arc<Self>, key: u64, url: &str, viewport: (u32, u32, f32)) {
        if let Some(session) = self.session_of(key) {
            self.send("Page.navigate", json!({"url": url}), Some(&session));
            return;
        }
        if self.has_page(key) {
            return;
        }
        self.shared.lock().unwrap().pages.insert(
            key,
            Page {
                target: String::new(),
                session: String::new(),
                info: PageInfo {
                    url: url.to_string(),
                    loading: true,
                    ..PageInfo::default()
                },
                viewport,
            },
        );
        let (this, url) = (self.clone(), url.to_string());
        std::thread::spawn(move || this.attach(key, &url));
    }

    /// Create and attach the page, then load `url` and start streaming if it is on screen.
    fn attach(&self, key: u64, url: &str) {
        let Some(target) = self
            .call("Target.createTarget", json!({"url": "about:blank"}), None)
            .and_then(|r| r["targetId"].as_str().map(str::to_string))
        else {
            return;
        };
        let Some(session) = self
            .call(
                "Target.attachToTarget",
                json!({"targetId": target, "flatten": true}),
                None,
            )
            .and_then(|r| r["sessionId"].as_str().map(str::to_string))
        else {
            return;
        };
        let viewport = {
            let mut shared = self.shared.lock().unwrap();
            let Some(page) = shared.pages.get_mut(&key) else {
                return;
            };
            page.target = target;
            page.session = session.clone();
            page.viewport
        };
        let s = Some(session.as_str());
        self.send("Page.enable", json!({}), s);
        self.send("Runtime.enable", json!({}), s);
        self.metrics(&session, viewport);
        self.send("Page.navigate", json!({"url": url}), s);
        if self.shared.lock().unwrap().active == Some(key) {
            self.screencast(&session, viewport);
        }
    }

    fn metrics(&self, session: &str, (w, h, scale): (u32, u32, f32)) {
        self.send(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": w, "height": h, "deviceScaleFactor": scale, "mobile": false}),
            Some(session),
        );
    }

    fn screencast(&self, session: &str, (w, h, scale): (u32, u32, f32)) {
        self.send(
            "Page.startScreencast",
            json!({
                "format": "jpeg",
                "quality": 70,
                "maxWidth": (w as f32 * scale) as u32,
                "maxHeight": (h as f32 * scale) as u32,
            }),
            Some(session),
        );
    }

    /// Stream the session's page, and stop streaming the one before.
    pub fn show(&self, key: Option<u64>) {
        let (old, new) = {
            let mut shared = self.shared.lock().unwrap();
            if shared.active == key {
                return;
            }
            let old = shared.active.and_then(|k| shared.pages.get(&k)).map(|p| p.session.clone());
            shared.active = key;
            shared.frame = None;
            let new = key
                .and_then(|k| shared.pages.get(&k))
                .map(|p| (p.session.clone(), p.viewport));
            (old, new)
        };
        if let Some(old) = old.filter(|s| !s.is_empty()) {
            self.send("Page.stopScreencast", json!({}), Some(&old));
        }
        if let Some((session, viewport)) = new.filter(|(s, _)| !s.is_empty()) {
            self.screencast(&session, viewport);
        }
    }

    /// Match the page to the panel: CSS size and device pixel ratio.
    pub fn resize(&self, key: u64, viewport: (u32, u32, f32)) {
        let (session, active) = {
            let mut shared = self.shared.lock().unwrap();
            let active = shared.active == Some(key);
            let Some(page) = shared.pages.get_mut(&key) else {
                return;
            };
            if page.viewport == viewport {
                return;
            }
            page.viewport = viewport;
            (page.session.clone(), active)
        };
        if session.is_empty() {
            return;
        }
        self.metrics(&session, viewport);
        if active {
            // The frame size is fixed when the screencast starts.
            self.send("Page.stopScreencast", json!({}), Some(&session));
            self.screencast(&session, viewport);
        }
    }

    /// The newest frame of the page on screen, if one came since the last call.
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.lock().unwrap().frame.take()
    }

    pub fn info(&self, key: u64) -> Option<(String, String, bool, Option<(String, String)>)> {
        let shared = self.shared.lock().unwrap();
        shared.pages.get(&key).map(|p| {
            (
                p.info.url.clone(),
                p.info.title.clone(),
                p.info.loading,
                p.info.dialog.clone(),
            )
        })
    }

    fn on_page(&self, key: u64, method: &str, params: Value) {
        if let Some(session) = self.session_of(key) {
            self.send(method, params, Some(&session));
        }
    }

    pub fn reload(&self, key: u64) {
        self.on_page(key, "Page.reload", json!({}));
    }

    pub fn history(&self, key: u64, back: bool) {
        let js = if back { "history.back()" } else { "history.forward()" };
        self.on_page(key, "Runtime.evaluate", json!({"expression": js}));
    }

    pub fn answer_dialog(&self, key: u64, accept: bool) {
        if let Some(page) = self.shared.lock().unwrap().pages.get_mut(&key) {
            page.info.dialog = None;
        }
        self.on_page(
            key,
            "Page.handleJavaScriptDialog",
            json!({"accept": accept}),
        );
    }

    /// `kind` is mouseMoved, mousePressed, mouseReleased or mouseWheel.
    #[allow(clippy::too_many_arguments)]
    pub fn mouse(
        &self,
        key: u64,
        kind: &str,
        (x, y): (f64, f64),
        button: &str,
        buttons: u8,
        clicks: usize,
        modifiers: u8,
        wheel: (f64, f64),
    ) {
        self.on_page(
            key,
            "Input.dispatchMouseEvent",
            json!({
                "type": kind, "x": x, "y": y, "button": button, "buttons": buttons,
                "clickCount": clicks, "modifiers": modifiers,
                "deltaX": wheel.0, "deltaY": wheel.1,
            }),
        );
    }

    pub fn insert_text(&self, key: u64, text: &str) {
        self.on_page(key, "Input.insertText", json!({"text": text}));
    }

    /// A key with no text of its own (Enter, arrows, Backspace…), or an editing command.
    pub fn key(&self, key: u64, down: bool, k: &KeySpec, modifiers: u8, commands: &[&str]) {
        let mut params = json!({
            "type": if down { if k.text.is_empty() { "rawKeyDown" } else { "keyDown" } } else { "keyUp" },
            "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.vk,
            "nativeVirtualKeyCode": k.vk, "modifiers": modifiers,
        });
        if down && !k.text.is_empty() {
            params["text"] = json!(k.text);
            params["unmodifiedText"] = json!(k.text);
        }
        if down && !commands.is_empty() {
            params["commands"] = json!(commands);
        }
        self.on_page(key, "Input.dispatchKeyEvent", params);
    }

    /// Close the session's page.
    pub fn close(&self, key: u64) {
        let page = self.shared.lock().unwrap().pages.remove(&key);
        if let Some(page) = page.filter(|p| !p.target.is_empty()) {
            self.send("Target.closeTarget", json!({"targetId": page.target}), None);
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.send("Browser.close", json!({}), None);
        std::thread::sleep(Duration::from_millis(60));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What `Input.dispatchKeyEvent` needs for a non-text key.
pub struct KeySpec {
    pub key: &'static str,
    pub code: &'static str,
    pub vk: u32,
    pub text: &'static str,
}

/// The CDP description of a GPUI key name, for keys that do something besides type.
pub fn key_spec(name: &str) -> Option<KeySpec> {
    let (key, code, vk, text) = match name {
        "enter" => ("Enter", "Enter", 13, "\r"),
        "backspace" => ("Backspace", "Backspace", 8, ""),
        "tab" => ("Tab", "Tab", 9, ""),
        "escape" => ("Escape", "Escape", 27, ""),
        "delete" => ("Delete", "Delete", 46, ""),
        "left" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "up" => ("ArrowUp", "ArrowUp", 38, ""),
        "right" => ("ArrowRight", "ArrowRight", 39, ""),
        "down" => ("ArrowDown", "ArrowDown", 40, ""),
        "home" => ("Home", "Home", 36, ""),
        "end" => ("End", "End", 35, ""),
        "pageup" => ("PageUp", "PageUp", 33, ""),
        "pagedown" => ("PageDown", "PageDown", 34, ""),
        _ => return None,
    };
    Some(KeySpec { key, code, vk, text })
}

/// Read CDP messages until the pipe closes: route replies, track pages, decode frames.
fn read_loop(
    mut reader: std::fs::File,
    mut writer: std::fs::File,
    shared: Arc<Mutex<Shared>>,
    seq: Arc<AtomicU64>,
) {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 1 << 16];
    let mut ack_id = 1u64 << 40;
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        for raw in split_messages(&mut buf) {
            let Ok(msg) = serde_json::from_slice::<Value>(&raw) else {
                continue;
            };
            if let Some(id) = msg["id"].as_u64() {
                let reply = shared.lock().unwrap().replies.remove(&id);
                if let Some(tx) = reply {
                    let _ = tx.send(msg["result"].clone());
                }
                continue;
            }
            let session = msg["sessionId"].as_str().unwrap_or_default();
            let params = &msg["params"];
            match msg["method"].as_str().unwrap_or_default() {
                "Page.screencastFrame" => {
                    // Ack first so Chromium can render the next frame while this one decodes.
                    ack_id += 1;
                    let ack = json!({"id": ack_id, "sessionId": session,
                        "method": "Page.screencastFrameAck",
                        "params": {"sessionId": params["sessionId"]}});
                    let _ = writer.write_all(&frame_message(&ack));
                    let active = {
                        let s = shared.lock().unwrap();
                        s.active
                            .and_then(|k| s.pages.get(&k))
                            .is_some_and(|p| p.session == session)
                    };
                    if !active {
                        continue;
                    }
                    let meta = &params["metadata"];
                    let css = (
                        meta["deviceWidth"].as_f64().unwrap_or(0.) as f32,
                        meta["deviceHeight"].as_f64().unwrap_or(0.) as f32,
                    );
                    if let Some(image) = params["data"].as_str().and_then(decode_frame) {
                        shared.lock().unwrap().frame = Some(Frame { image, css });
                        seq.fetch_add(1, Ordering::Relaxed);
                    }
                }
                "Target.targetInfoChanged" => {
                    let info = &params["targetInfo"];
                    let target = info["targetId"].as_str().unwrap_or_default();
                    let mut s = shared.lock().unwrap();
                    if let Some(page) = s.pages.values_mut().find(|p| p.target == target) {
                        page.info.url = info["url"].as_str().unwrap_or_default().to_string();
                        page.info.title = info["title"].as_str().unwrap_or_default().to_string();
                        seq.fetch_add(1, Ordering::Relaxed);
                    }
                }
                method @ ("Page.frameStartedLoading"
                | "Page.loadEventFired"
                | "Page.javascriptDialogOpening") => {
                    let mut s = shared.lock().unwrap();
                    if let Some(page) = s.pages.values_mut().find(|p| p.session == session) {
                        match method {
                            "Page.frameStartedLoading" => page.info.loading = true,
                            "Page.loadEventFired" => page.info.loading = false,
                            _ => {
                                page.info.dialog = Some((
                                    params["type"].as_str().unwrap_or("alert").to_string(),
                                    params["message"].as_str().unwrap_or_default().to_string(),
                                ))
                            }
                        }
                        seq.fetch_add(1, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        }
    }
    let mut s = shared.lock().unwrap();
    s.dead = true;
    s.replies.clear();
    seq.fetch_add(1, Ordering::Relaxed);
}

/// A base64 JPEG as a BGRA image GPUI can draw.
fn decode_frame(data: &str) -> Option<Arc<RenderImage>> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
    let mut rgba = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .ok()?
        .into_rgba8();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_nul_framed_and_split() {
        let bytes = frame_message(&json!({"id": 1, "method": "Page.enable"}));
        assert_eq!(*bytes.last().unwrap(), 0);
        let mut buf = bytes.clone();
        buf.extend_from_slice(b"{\"id\":2}\0{\"id\":");
        let msgs = split_messages(&mut buf);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1], b"{\"id\":2}");
        assert_eq!(buf, b"{\"id\":");
        buf.extend_from_slice(b"3}\0");
        assert_eq!(split_messages(&mut buf), vec![b"{\"id\":3}".to_vec()]);
        assert!(buf.is_empty());
    }

    #[test]
    fn panel_points_map_to_page_pixels() {
        // Same size: unchanged.
        assert_eq!(to_page((10., 20.), (400., 300.), (400., 300.)), (10., 20.));
        // The page is still at the old size after the panel grew: scale.
        assert_eq!(to_page((200., 150.), (400., 300.), (800., 600.)), (400., 300.));
        // Outside the panel clamps to its edge; an empty panel maps to the origin.
        assert_eq!(to_page((-5., 999.), (400., 300.), (400., 300.)), (0., 300.));
        assert_eq!(to_page((5., 5.), (0., 0.), (400., 300.)), (0., 0.));
        assert_eq!(modifier_bits(false, true, false, true), 2 | 8);
    }

    #[test]
    fn chromium_is_found_in_system_or_user_applications() {
        let home = Path::new("/Users/me");
        let user_brave = "/Users/me/Applications/Brave Browser.app/Contents/MacOS/Brave Browser";
        let found = pick_chromium(Some(home), |p| {
            p.to_str() == Some(user_brave) || p.ends_with("Microsoft Edge")
        });
        if cfg!(target_os = "macos") {
            // Edge comes before Brave in the list.
            assert!(found.unwrap().ends_with("Microsoft Edge"));
            let brave = pick_chromium(Some(home), |p| p.to_str() == Some(user_brave));
            assert_eq!(brave.as_deref(), Some(Path::new(user_brave)));
            assert_eq!(browser_name(Path::new(user_brave)), "Brave Browser");
        }
        assert_eq!(pick_chromium(Some(home), |_| false), None);
        assert_eq!(key_spec("enter").unwrap().vk, 13);
        assert!(key_spec("a").is_none());
    }
}
