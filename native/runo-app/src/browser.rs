//! The Browser tab: a live page from the headless Chromium in `browser_engine`, drawn as
//! an image and driven by forwarding the panel's mouse and keys. Before a page is open it
//! lists the dev servers running on this machine.

use super::*;
use browser_engine::{key_spec, modifier_bits, to_page, Engine, KeySpec};
use std::cell::Cell;
use std::collections::HashSet;
use std::net::{SocketAddr, TcpStream};
use std::rc::Rc;
use std::sync::Arc;

/// Ports dev servers commonly listen on.
const DEV_PORTS: [u16; 14] = [
    3000, 3001, 4000, 4200, 4321, 5000, 5173, 5174, 6006, 8000, 8080, 8081, 8888, 9000,
];

/// Ports that are rarely web pages: databases, caches and the like.
const NOT_WEB: [u16; 9] = [3306, 5432, 6379, 6380, 11211, 27017, 9200, 2375, 5672];

/// Listening TCP ports in `/proc/net/tcp`-style text, on loopback or all interfaces.
pub(crate) fn parse_proc_net(text: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // sl local_address rem_address st ...; 0A is LISTEN.
        let (Some(local), Some("0A")) = (cols.get(1), cols.get(3).copied()) else {
            continue;
        };
        let Some((addr, port)) = local.rsplit_once(':') else {
            continue;
        };
        let local_only = matches!(
            addr,
            "0100007F" | "00000000" | "00000000000000000000000000000000"
        ) || addr == "00000000000000000000000001000000"
            || addr.ends_with("0100007F");
        if let (true, Ok(port)) = (local_only, u16::from_str_radix(port, 16)) {
            ports.push(port);
        }
    }
    ports
}

/// Listening ports in `lsof -nP -iTCP -sTCP:LISTEN` output.
pub(crate) fn parse_lsof(text: &str) -> Vec<u16> {
    text.lines()
        .filter(|l| l.contains("(LISTEN)"))
        .filter_map(|l| {
            let name = l.split_whitespace().rev().nth(1)?;
            name.rsplit_once(':')?.1.parse().ok()
        })
        .collect()
}

/// Dev-server-like ports listening on this machine: unprivileged, not ephemeral and not
/// a well-known database or cache.
fn listening_ports() -> Vec<u16> {
    let mut ports = Vec::new();
    if cfg!(target_os = "linux") {
        for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
            ports.extend(parse_proc_net(
                &std::fs::read_to_string(file).unwrap_or_default(),
            ));
        }
    } else if let Ok(out) = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output()
    {
        ports.extend(parse_lsof(&String::from_utf8_lossy(&out.stdout)));
    }
    dev_ports(ports)
}

/// Keep the ports a dev server could be on: unprivileged, not ephemeral and not a
/// well-known database or cache.
pub(crate) fn dev_ports(mut ports: Vec<u16>) -> Vec<u16> {
    ports.retain(|p| (1024..32768).contains(p) && !NOT_WEB.contains(p));
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// The message asking the agent to fix errors the page logged.
pub(crate) fn console_prompt(url: &str, errors: &[String]) -> String {
    format!(
        "The page at {url} logged these errors in the browser console:\n\n```\n{}\n```\n\n\
         Find the cause and fix it.",
        errors.join("\n")
    )
}

/// Turn what was typed into a URL: `localhost:3000` and `:5173` work too.
pub(crate) fn normalize_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains(char::is_whitespace) {
        return None;
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Some(raw.to_string());
    }
    if let Some(port) = raw.strip_prefix(':') {
        return port
            .parse::<u16>()
            .ok()
            .map(|p| format!("http://localhost:{p}"));
    }
    if raw.parse::<u16>().is_ok() {
        return Some(format!("http://localhost:{raw}"));
    }
    let local =
        raw.starts_with("localhost") || raw.starts_with("127.") || raw.starts_with("0.0.0.0");
    if local {
        return Some(format!("http://{raw}"));
    }
    raw.contains('.').then(|| format!("https://{raw}"))
}

/// http(s) links printed in terminal output, newest first, without duplicates.
pub(crate) fn urls_in(lines: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in lines.iter().rev() {
        for (i, _) in line.match_indices("http") {
            let rest = &line[i..];
            if !(rest.starts_with("http://") || rest.starts_with("https://")) {
                continue;
            }
            let url: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && !"\"'<>`)]}".contains(*c))
                .collect();
            let url = url.trim_end_matches(['.', ',', ';', ':']).to_string();
            if url.len() > 10 && !found.contains(&url) {
                found.push(url);
            }
        }
    }
    found.truncate(8);
    found
}

/// The Browser tab's side of the engine: frames on screen and where the page was drawn.
#[derive(Default)]
pub(crate) struct BrowserView {
    /// The frame on screen and the CSS size of the page it shows.
    frame: Option<(Arc<RenderImage>, (f32, f32))>,
    /// The frame before, kept one more paint before its texture is freed.
    stale: Option<Arc<RenderImage>>,
    /// The page area last frame, in window coordinates, and the window's scale factor.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    scale: Rc<Cell<f32>>,
    /// The engine's change counter when last looked at.
    seen: u64,
    /// Sessions with a page open; the rest show the start page.
    pages: HashSet<u64>,
    /// The URL last put in the address field, so typing there is not overwritten.
    synced_url: String,
    /// Mouse buttons held down over the page, as CDP's bit mask.
    buttons: u8,
    /// The console error list is expanded.
    console_open: bool,
    pub error: Option<String>,
}

/// A frame-area size as the page viewport: CSS width, height and device pixel ratio.
fn viewport(bounds: Bounds<Pixels>, scale: f32) -> (u32, u32, f32) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w < 50. || h < 50. {
        return (900, 700, scale.max(1.));
    }
    (w.round() as u32, h.round() as u32, scale.max(1.))
}

fn button_name(button: MouseButton) -> (&'static str, u8) {
    match button {
        MouseButton::Right => ("right", 2),
        MouseButton::Middle => ("middle", 4),
        _ => ("left", 1),
    }
}

impl RunoApp {
    /// Look for listening dev-server ports on this machine, and where the session runs
    /// when that is a connection, off the UI thread.
    pub(crate) fn scan_ports(&mut self) {
        let elsewhere = self
            .current()
            .and_then(|s| Some((s.id, s.dir.clone(), s.sandbox()?)));
        if let Some((id, dir, sandbox)) = elsewhere {
            if self.remote_scanning.insert(id) {
                self.bg(
                    move || {
                        sandbox
                            .listening(&dir)
                            .map(|text| dev_ports(connections::parse_listening(&text)))
                    },
                    move |this, found, cx| {
                        this.remote_scanning.remove(&id);
                        this.remote_ports.insert(id, found);
                        cx.notify();
                    },
                );
            }
        }
        if self.browser_scanning {
            return;
        }
        self.browser_scanning = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut ports: Vec<u16> = DEV_PORTS
                .iter()
                .copied()
                .filter(|p| {
                    let addr = SocketAddr::from(([127, 0, 0, 1], *p));
                    TcpStream::connect_timeout(&addr, Duration::from_millis(120)).is_ok()
                })
                .collect();
            for port in listening_ports() {
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
            ports.sort_unstable();
            let open: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
            let _ = tx.send(Event::Output("ports".into(), Ok(open.join(","))));
        });
    }

    pub(crate) fn remember_url(&mut self, url: &str) {
        self.browser_recent.retain(|u| u != url);
        self.browser_recent.insert(0, url.to_string());
        self.browser_recent.truncate(6);
    }

    pub(crate) fn open_in_browser(&mut self, url: String, cx: &mut Context<Self>) {
        self.remember_url(&url);
        cx.open_url(&url);
    }

    /// Open a dev server running where session `id` runs: forward its port to this
    /// machine first (an SSH tunnel, or through Cove), then open it in the panel.
    fn open_forwarded(&mut self, id: u64, port: u16, cx: &mut Context<Self>) {
        if let Some(forward) = self.forwards.get(&(id, port)) {
            let url = format!("http://localhost:{}", forward.local);
            self.open_in_panel(url, cx);
            return;
        }
        let Some((dir, sandbox, place)) = self
            .state
            .session(id)
            .and_then(|s| Some((s.dir.clone(), s.sandbox()?, connections::name(s.connection))))
        else {
            return;
        };
        if self.forwarding.is_some() {
            return;
        }
        self.forwarding = Some((id, port));
        self.bg(
            move || sandbox.forward(&dir, port),
            move |this, result, cx| {
                this.forwarding = None;
                match result {
                    Ok(forward) => {
                        let url = format!("http://localhost:{}", forward.local);
                        this.forwards.insert((id, port), forward);
                        if this.selected == Some(id) {
                            this.open_in_panel(url, cx);
                        }
                    }
                    Err(e) => {
                        this.notice =
                            Some(format!("Couldn't forward port {port} from {place}: {e}"))
                    }
                }
                cx.notify();
            },
        );
    }

    /// A dev server where the session runs: its port there, forwarded or not.
    fn remote_port_row(&self, n: usize, id: u64, port: u16, cx: &mut Context<Self>) -> AnyElement {
        let local = self.forwards.get(&(id, port)).map(|f| f.local);
        let busy = self.forwarding == Some((id, port));
        let label = match local {
            Some(local) => format!(":{port}  →  localhost:{local}"),
            None => format!(":{port}"),
        };
        let mut row = div()
            .id(("remote-port", n))
            .h(px(32.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|el| el.bg(tint(c_subtle())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_forwarded(id, port, cx);
                cx.notify();
            }))
            .child(icon(IconName::Globe, 13., c_muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(self.mono_font.clone())
                    .text_size(px(12.))
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(if busy {
                        "Forwarding…"
                    } else if local.is_some() {
                        "Open"
                    } else {
                        "Forward and open"
                    }),
            );
        if local.is_some() {
            row = row.child(
                self.ghost_button_sized(
                    ("remote-port-stop", n),
                    IconName::X,
                    "Stop forwarding",
                    24.,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.forwards.remove(&(id, port));
                    cx.stop_propagation();
                    cx.notify();
                })),
            );
        }
        row.into_any_element()
    }

    /// Results of the port scan.
    pub(crate) fn handle_browser_job(&mut self, key: &str, result: Result<String, String>) -> bool {
        if key != "ports" {
            return false;
        }
        self.browser_scanning = false;
        self.browser_ports = result
            .unwrap_or_default()
            .split(',')
            .filter_map(|p| p.parse().ok())
            .collect();
        true
    }

    /// The page shown for the selected session.
    fn browser_key(&self) -> u64 {
        self.selected.unwrap_or(0)
    }

    /// The running engine, starting Chromium the first time.
    fn engine(&mut self) -> Option<Arc<Engine>> {
        if let Some(engine) = self.browser.as_ref().filter(|e| e.alive()) {
            return Some(engine.clone());
        }
        self.browser_view.pages.clear();
        let Some(path) = browser_engine::find_chromium() else {
            self.browser_view.error = Some(browser_engine::not_found_message().into());
            return None;
        };
        let profile = store::data_dir().join("browser").join("profile");
        match Engine::start(path, &profile) {
            Ok(engine) => {
                self.browser_view.error = None;
                self.browser = Some(Arc::new(engine));
                self.browser.clone()
            }
            Err(e) => {
                self.browser_view.error = Some(e);
                None
            }
        }
    }

    /// Show `url` in the panel, in the selected session's page.
    pub(crate) fn open_in_panel(&mut self, url: String, cx: &mut Context<Self>) {
        self.remember_url(&url);
        let Some(engine) = self.engine() else {
            cx.notify();
            return;
        };
        let key = self.browser_key();
        let v = &self.browser_view;
        engine.show(Some(key));
        engine.open(key, &url, viewport(v.bounds.get(), v.scale.get()));
        self.browser_view.pages.insert(key);
        self.browser_view.synced_url.clear();
        cx.notify();
    }

    /// Back to the dev-server list, closing the session's page.
    fn close_page(&mut self, cx: &mut Context<Self>) {
        let key = self.browser_key();
        if let Some(engine) = &self.browser {
            engine.show(None);
            engine.close(key);
        }
        self.browser_view.pages.remove(&key);
        self.browser_view.frame = None;
        cx.notify();
    }

    /// Called every display frame: pick up new frames, follow the panel's size and the
    /// selected session, and free the textures of frames no longer shown.
    pub(crate) fn pump_browser(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.browser.clone() else {
            return;
        };
        let key = self.browser_key();
        let visible = self.right_tab == RightTab::Browser
            && self.show_panel
            && self.view == View::Session
            && self.browser_view.pages.contains(&key)
            && cx.global::<AppSettings>().browser_enabled;
        engine.show(visible.then_some(key));
        if let Some(note) = engine.take_notes().pop() {
            self.info = Some((note, std::time::Instant::now()));
            cx.notify();
        }
        if !visible {
            if let Some((old, _)) = self.browser_view.frame.take() {
                cx.drop_image(old, None);
            }
            return;
        }
        let v = &self.browser_view;
        engine.resize(key, viewport(v.bounds.get(), v.scale.get()));
        let seq = engine.seq.load(std::sync::atomic::Ordering::Relaxed);
        if seq == self.browser_view.seen {
            return;
        }
        self.browser_view.seen = seq;
        if let Some(frame) = engine.take_frame() {
            if let Some(stale) = self.browser_view.stale.take() {
                cx.drop_image(stale, None);
            }
            self.browser_view.stale = self.browser_view.frame.take().map(|(img, _)| img);
            self.browser_view.frame = Some((frame.image, frame.css));
        }
        cx.notify();
    }

    /// Put the page's URL in the address field when it navigates or the session changes,
    /// unless the field is being edited.
    pub(crate) fn sync_browser_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.browser_key();
        let url = self
            .browser
            .as_ref()
            .filter(|_| self.browser_view.pages.contains(&key))
            .and_then(|e| e.info(key))
            .map(|info| info.url)
            .filter(|url| url != "about:blank")
            .unwrap_or_default();
        if url == self.browser_view.synced_url
            || self.address.read(cx).focus_handle(cx).is_focused(window)
        {
            return;
        }
        self.browser_view.synced_url = url.clone();
        self.address
            .update(cx, |input, cx| input.set_value(url, window, cx));
    }

    pub(crate) fn submit_address(&mut self, open: bool, cx: &mut Context<Self>) {
        let raw = self.address.read(cx).value().to_string();
        let panel = cx.global::<AppSettings>().browser_enabled;
        match normalize_url(&raw) {
            Some(url) if open && panel => self.open_in_panel(url, cx),
            Some(url) => self.open_in_browser(url, cx),
            None if raw.trim().is_empty() => {}
            None => self.notice = Some("Enter a URL such as localhost:3000 or example.com.".into()),
        }
        cx.notify();
    }

    /// A panel point as page CSS pixels.
    fn page_point(&self, pos: Point<Pixels>) -> (f64, f64) {
        let b = self.browser_view.bounds.get();
        let panel = (f32::from(b.size.width), f32::from(b.size.height));
        let page = self.browser_view.frame.as_ref().map_or(panel, |f| f.1);
        let local = (f32::from(pos.x - b.origin.x), f32::from(pos.y - b.origin.y));
        to_page(local, panel, page)
    }

    fn page_mouse(
        &mut self,
        kind: &str,
        pos: Point<Pixels>,
        button: Option<MouseButton>,
        clicks: usize,
        m: Modifiers,
        wheel: (f64, f64),
    ) {
        let Some(engine) = self.browser.clone() else {
            return;
        };
        let (name, bit) = button.map(button_name).unwrap_or(("none", 0));
        match kind {
            "mousePressed" => self.browser_view.buttons |= bit,
            "mouseReleased" => self.browser_view.buttons &= !bit,
            _ => {}
        }
        let mods = modifier_bits(m.alt, m.control, m.platform, m.shift);
        engine.mouse(
            self.browser_key(),
            kind,
            self.page_point(pos),
            name,
            self.browser_view.buttons,
            clicks,
            mods,
            wheel,
        );
    }

    /// Keys typed with the page focused: text goes in as text, other keys as key events,
    /// and the usual ⌘ shortcuts do what they do in a browser.
    fn page_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(engine) = self.browser.clone() else {
            return;
        };
        let key = self.browser_key();
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        let mods = modifier_bits(m.alt, m.control, m.platform, m.shift);
        // The usual shortcut key: Cmd on macOS, Ctrl elsewhere.
        let command = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        if command {
            let edit = |command: &'static str, k: &'static str, code: &'static str, vk: u32| {
                let spec = KeySpec {
                    key: k,
                    code,
                    vk,
                    text: "",
                    command,
                };
                engine.key(key, true, &spec, mods, &[command]);
                engine.key(key, false, &spec, mods, &[]);
            };
            match ks.key.as_str() {
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        engine.insert_text(key, &text);
                    }
                }
                "a" => edit("selectAll", "a", "KeyA", 65),
                // Headless Chromium has no clipboard of its own: read the selection and
                // put it on the system clipboard, then delete it for a cut.
                k @ ("c" | "x") => {
                    let cut = k == "x";
                    let page = engine.clone();
                    self.bg(
                        move || page.selected_text(key),
                        move |this, text, cx| {
                            let Some(text) = text else {
                                return;
                            };
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            if let (true, Some(engine)) = (cut, this.browser.clone()) {
                                let spec = KeySpec {
                                    key: "Delete",
                                    code: "Delete",
                                    vk: 46,
                                    text: "",
                                    command: "deleteForward",
                                };
                                engine.key(key, true, &spec, 0, &["deleteForward"]);
                                engine.key(key, false, &spec, 0, &[]);
                            }
                        },
                    );
                }
                "z" if m.shift => edit("redo", "z", "KeyZ", 90),
                "z" => edit("undo", "z", "KeyZ", 90),
                "r" => engine.reload(key),
                "[" => engine.history(key, true),
                "]" => engine.history(key, false),
                "l" => window.focus(&self.address.read(cx).focus_handle(cx), cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if let Some(spec) = key_spec(&ks.key) {
            engine.key(key, true, &spec, mods, &[]);
        } else if let Some(text) = ks.key_char.as_deref().filter(|_| !m.control) {
            engine.insert_text(key, text);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn page_key_up(&mut self, ev: &KeyUpEvent) {
        let (Some(engine), Some(spec)) = (self.browser.clone(), key_spec(&ev.keystroke.key)) else {
            return;
        };
        let m = ev.keystroke.modifiers;
        let mods = modifier_bits(m.alt, m.control, m.platform, m.shift);
        engine.key(self.browser_key(), false, &spec, mods, &[]);
    }

    /// The slim bar over the page: back, forward, reload, address, open outside.
    fn browser_bar(&self, page: bool, loading: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let nav = |id: &'static str, name: IconName, tip: &'static str| {
            self.ghost_button_sized(id, name, tip, 26.)
                .when(!page, |el| el.opacity(0.4))
        };
        div()
            .h(px(40.))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(rgb(c_border()))
            .child(
                nav("page-back", IconName::ArrowLeft, "Back").on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(e) = &this.browser {
                            e.history(this.browser_key(), true);
                        }
                    },
                )),
            )
            .child(
                nav("page-forward", IconName::ArrowRight, "Forward").on_click(cx.listener(
                    |this, _, _, _| {
                        if let Some(e) = &this.browser {
                            e.history(this.browser_key(), false);
                        }
                    },
                )),
            )
            .child(
                nav(
                    "page-reload",
                    if loading {
                        IconName::Loader
                    } else {
                        IconName::RotateCw
                    },
                    "Reload",
                )
                .on_click(cx.listener(|this, _, _, _| {
                    if let Some(e) = &this.browser {
                        e.reload(this.browser_key());
                    }
                })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.address).bordered(true).h(px(28.))),
            )
            .child(
                self.ghost_button_sized(
                    "page-open-outside",
                    IconName::ExternalLink,
                    "Open in Chrome",
                    26.,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let raw = this.address.read(cx).value().to_string();
                    if let Some(url) = normalize_url(&raw) {
                        this.open_in_browser(url, cx);
                    }
                })),
            )
            .when(page, |el| {
                el.child(
                    self.ghost_button_sized("page-close", IconName::X, "Close the page", 26.)
                        .on_click(cx.listener(|this, _, _, cx| this.close_page(cx))),
                )
            })
    }

    /// The live page, filling the panel, with input forwarded to Chromium.
    fn page_view(&self, dialog: Option<(String, String)>, cx: &mut Context<Self>) -> AnyElement {
        let mouse_down = |button: MouseButton| {
            cx.listener(move |this: &mut Self, ev: &MouseDownEvent, window, cx| {
                window.focus(&this.browser_focus, cx);
                this.page_mouse(
                    "mousePressed",
                    ev.position,
                    Some(button),
                    ev.click_count,
                    ev.modifiers,
                    (0., 0.),
                );
                cx.stop_propagation();
            })
        };
        let mouse_up = |button: MouseButton| {
            cx.listener(move |this: &mut Self, ev: &MouseUpEvent, _, _| {
                this.page_mouse(
                    "mouseReleased",
                    ev.position,
                    Some(button),
                    ev.click_count,
                    ev.modifiers,
                    (0., 0.),
                );
            })
        };
        let mut page = div()
            .id("browser-page")
            .key_context("RunoBrowser")
            .track_focus(&self.browser_focus)
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .bg(rgb(0xffffff))
            .cursor_default()
            .on_mouse_down(MouseButton::Left, mouse_down(MouseButton::Left))
            .on_mouse_down(MouseButton::Right, mouse_down(MouseButton::Right))
            .on_mouse_up(MouseButton::Left, mouse_up(MouseButton::Left))
            .on_mouse_up(MouseButton::Right, mouse_up(MouseButton::Right))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, _| {
                this.page_mouse("mouseMoved", ev.position, None, 0, ev.modifiers, (0., 0.));
            }))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let d = ev.delta.pixel_delta(px(16.));
                let wheel = (-f32::from(d.x) as f64, -f32::from(d.y) as f64);
                this.page_mouse("mouseWheel", ev.position, None, 0, ev.modifiers, wheel);
                cx.stop_propagation();
            }))
            .on_key_down(
                cx.listener(|this, ev: &KeyDownEvent, window, cx| this.page_key(ev, window, cx)),
            )
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, _| this.page_key_up(ev)));
        match &self.browser_view.frame {
            Some((image, _)) => {
                page = page.child(
                    img(ImageSource::Render(image.clone()))
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                );
            }
            None => {
                page = page.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.5))
                        .text_color(rgb(0x6b7280))
                        .child("Loading…"),
                );
            }
        }
        if let Some((kind, message)) = dialog {
            page = page.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(hsla(0., 0., 0., 0.3))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id("page-dialog")
                            .w(px(300.))
                            .p_4()
                            .v_flex()
                            .gap_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(float_edge())
                            .bg(float(c_canvas()))
                            .shadow_xl()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(div().text_size(px(13.)).child(message))
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .when(kind != "alert", |el| {
                                        el.child(
                                            self.button("dialog-cancel", "Cancel", false).on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    if let Some(e) = &this.browser {
                                                        e.answer_dialog(this.browser_key(), false);
                                                    }
                                                    cx.notify();
                                                }),
                                            ),
                                        )
                                    })
                                    .child(self.button("dialog-ok", "OK", true).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            if let Some(e) = &this.browser {
                                                e.answer_dialog(this.browser_key(), true);
                                            }
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    ),
            );
        }
        page.into_any_element()
    }

    fn url_row(
        &self,
        id: (&'static str, usize),
        url: &str,
        lead: IconName,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (show, open) = (url.to_string(), url.to_string());
        stagger(
            div()
                .id(id)
                .group("url-row")
                .h(px(32.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .hover(|el| el.bg(tint(c_subtle())))
                .on_click(cx.listener(move |this, _, _, cx| this.open_in_panel(show.clone(), cx)))
                .child(icon(lead, 13., c_muted()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(self.mono_font.clone())
                        .text_size(px(12.))
                        .child(url.to_string()),
                )
                .child(
                    self.ghost_button_sized(
                        (id.0, id.1 + 2000),
                        IconName::ExternalLink,
                        "Open in Chrome",
                        24.,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_in_browser(open.clone(), cx);
                        cx.stop_propagation();
                        cx.notify();
                    })),
                ),
            SharedString::from(format!("{}-{}-in", id.0, id.1)),
            id.1,
        )
    }

    /// Before a page is open: dev servers running here, links the terminal printed, recent.
    fn start_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let id = self.browser_key();
        let mut printed: Vec<String> = self.terminal.get(&id).cloned().unwrap_or_default();
        for shell in self.session_shells(id) {
            printed.extend(shell.text().lines().map(str::to_string));
        }
        let detected = urls_in(&printed);
        let section = |title: &'static str| {
            div()
                .mt_2()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(c_muted()))
                .child(title)
        };
        let mut body = div()
            .id("browser-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("Pages open here, in a headless Chrome. Bare ports like :5173 work."),
            );
        if let Some(error) = &self.browser_view.error {
            body = body.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(tint(c_subtle()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::CircleX, 14., c_red()))
                    .child(div().text_size(px(12.5)).child(error.clone())),
            );
        }
        // A session on a connection: its dev servers there, forwarded here to open.
        if let Some(session) = self.current().filter(|s| s.sandbox().is_some()) {
            let place = connections::name(session.connection);
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section("RUNNING WHERE THIS SESSION RUNS"))
                    .child(
                        self.ghost_button_sized(
                            "scan-remote",
                            IconName::RefreshCw,
                            "Scan again",
                            24.,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.scan_ports();
                            cx.notify();
                        })),
                    ),
            );
            let note = |text: String| {
                div()
                    .px_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child(text)
            };
            match self.remote_ports.get(&session.id) {
                _ if self.remote_scanning.contains(&session.id) => {
                    body = body.child(note(format!("Looking for dev servers on {place}…")))
                }
                None => {
                    body = body.child(note(format!("Scan to look for dev servers on {place}.")))
                }
                Some(Err(e)) => {
                    body = body.child(note(format!("Couldn't list the ports on {place}: {e}")))
                }
                Some(Ok(ports)) if ports.is_empty() => {
                    body = body.child(note(format!(
                    "No dev server is listening on {place}. Start one in the session's Terminal."
                )))
                }
                Some(Ok(ports)) => {
                    body = body.child(note(format!(
                        "On {place}. Opening one forwards its port to this machine."
                    )));
                    for (n, port) in ports.clone().into_iter().enumerate() {
                        body = body.child(self.remote_port_row(n, session.id, port, cx));
                    }
                }
            }
        }
        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section("RUNNING ON THIS MACHINE"))
                .child(
                    self.ghost_button_sized("scan-ports", IconName::RefreshCw, "Scan again", 24.)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.scan_ports();
                            cx.notify();
                        })),
                ),
        );
        if self.browser_ports.is_empty() {
            body = body.child(
                div()
                    .px_2()
                    .text_size(px(12.5))
                    .text_color(rgb(c_muted()))
                    .child(if self.browser_scanning {
                        "Looking for dev servers…"
                    } else {
                        "No dev server is listening on this machine. Start one in the Terminal."
                    }),
            );
        }
        for (i, port) in self.browser_ports.iter().enumerate() {
            body = body.child(self.url_row(
                ("port-url", i),
                &format!("http://localhost:{port}"),
                IconName::Globe,
                cx,
            ));
        }
        if !detected.is_empty() {
            body = body.child(section("PRINTED IN THE TERMINAL"));
            for (i, url) in detected.iter().enumerate() {
                body = body.child(self.url_row(("term-url", i), url, IconName::SquareTerminal, cx));
            }
        }
        if !self.browser_recent.is_empty() {
            body = body.child(section("RECENT"));
            for (i, url) in self.browser_recent.iter().enumerate() {
                body = body.child(self.url_row(("recent-url", i), url, IconName::Clock, cx));
            }
        }
        body.into_any_element()
    }

    /// Console errors of the page, under it: a count that opens the list, with buttons
    /// to send them to the session's agent or clear them.
    fn console_bar(&self, url: &str, errors: &[String], cx: &mut Context<Self>) -> AnyElement {
        let open = self.browser_view.console_open;
        let n = errors.len();
        let prompt = console_prompt(url, errors);
        let mut bar = div()
            .flex_none()
            .border_t_1()
            .border_color(rgb(c_border()))
            .v_flex()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .child(
                        div()
                            .id("console-toggle")
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .text_color(rgb(c_red()))
                            .child(icon(
                                if open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                },
                                12.,
                                c_muted(),
                            ))
                            .child(format!(
                                "{n} console error{}",
                                if n == 1 { "" } else { "s" }
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.browser_view.console_open = !this.browser_view.console_open;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("console-send", "Send to agent", false)
                            .h(px(22.))
                            .px_2()
                            .text_xs()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(id) = this.selected {
                                    this.state
                                        .log(id, "Sent browser console errors to the agent");
                                    this.send_or_queue(id, prompt.clone(), cx);
                                }
                                let key = this.browser_key();
                                if let Some(engine) = &this.browser {
                                    engine.clear_console(key);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        self.ghost_button_sized("console-clear", IconName::X, "Clear", 22.)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let key = this.browser_key();
                                if let Some(engine) = &this.browser {
                                    engine.clear_console(key);
                                }
                                cx.notify();
                            })),
                    ),
            );
        if open {
            let mut list = div()
                .id("console-list")
                .max_h(px(160.))
                .overflow_y_scroll()
                .px_3()
                .pb_2()
                .v_flex()
                .gap_1()
                .font_family(self.mono_font.clone())
                .text_size(px(11.5))
                .text_color(rgb(c_secondary()));
            for line in errors.iter().rev() {
                list = list.child(div().child(line.clone()));
            }
            bar = bar.child(list);
        }
        bar.into_any_element()
    }

    pub(crate) fn browser_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let key = self.browser_key();
        let info = self
            .browser
            .as_ref()
            .filter(|_| self.browser_view.pages.contains(&key))
            .and_then(|e| e.info(key));
        let page = info.is_some();
        let (url, console) = info
            .as_ref()
            .map(|i| (i.url.clone(), i.console.clone()))
            .unwrap_or_default();
        let (loading, dialog) = info.map_or((false, None), |i| (i.loading, i.dialog));
        let (bounds, scale) = (
            self.browser_view.bounds.clone(),
            self.browser_view.scale.clone(),
        );
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(self.browser_bar(page, loading, cx))
            .child(
                // Measured on the start page too, so a page opens at the panel's size.
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .v_flex()
                    .child(
                        canvas(
                            move |b, window, _| {
                                bounds.set(b);
                                scale.set(window.scale_factor());
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .child(if page {
                        self.page_view(dialog, cx)
                    } else {
                        self.start_page(cx)
                    }),
            )
            .when(page && !console.is_empty(), |el| {
                el.child(self.console_bar(&url, &console, cx))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    #[test]
    fn listening_ports_are_read_from_proc_and_lsof() {
        let proc = "  sl  local_address rem_address   st tx_queue rx_queue\n\
   0: 0100007F:1538 00000000:0000 0A 00000000:00000000 00:00000000 00000000\n\
   1: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000\n\
   2: 0100007F:1538 0100007F:A000 01 00000000:00000000 00:00000000 00000000\n\
   3: 0A00020F:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000\n";
        assert_eq!(parse_proc_net(proc), vec![5432, 8080]);
        let lsof = "COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\n\
node 1 me 20u IPv6 0x1 0t0 TCP *:5173 (LISTEN)\n\
node 1 me 21u IPv4 0x2 0t0 TCP 127.0.0.1:3000 (LISTEN)\n\
curl 2 me 5u IPv4 0x3 0t0 TCP 10.0.0.2:5555->1.2.3.4:443 (ESTABLISHED)\n";
        assert_eq!(parse_lsof(lsof), vec![5173, 3000]);
        let prompt = console_prompt("http://localhost:3000/", &["boom".into()]);
        assert!(prompt.contains("http://localhost:3000/") && prompt.contains("```\nboom\n```"));
    }

    #[test]
    fn urls_are_normalized_found_and_titled() {
        assert_eq!(
            normalize_url("localhost:3000").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(
            normalize_url(":5173").as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(
            normalize_url("8080").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(
            normalize_url("example.com").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            normalize_url("https://a.b/c").as_deref(),
            Some("https://a.b/c")
        );
        assert_eq!(normalize_url("not a url"), None);
        assert_eq!(normalize_url("word"), None);
        let lines = vec![
            "  ➜  Local:   http://localhost:5173/".to_string(),
            "see (https://docs.rs/x)".to_string(),
            "again http://localhost:5173/.".to_string(),
        ];
        assert_eq!(
            urls_in(&lines),
            ["http://localhost:5173/", "https://docs.rs/x"]
        );
    }
}
