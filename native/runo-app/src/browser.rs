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
    pub error: Option<String>,
    /// Frames received, for a rough frame rate.
    frames: u64,
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
    /// Look for listening dev-server ports on this machine, off the UI thread.
    pub(crate) fn scan_ports(&mut self) {
        if self.browser_scanning {
            return;
        }
        self.browser_scanning = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let open: Vec<String> = DEV_PORTS
                .iter()
                .filter(|p| {
                    let addr = SocketAddr::from(([127, 0, 0, 1], **p));
                    TcpStream::connect_timeout(&addr, Duration::from_millis(120)).is_ok()
                })
                .map(|p| p.to_string())
                .collect();
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
            self.browser_view.error =
                Some("No Chrome, Edge, Brave or Chromium found in Applications.".into());
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
            self.browser_view.frames += 1;
        }
        cx.notify();
    }

    /// Put the page's URL in the address field when it navigates, unless it is being edited.
    pub(crate) fn sync_browser_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(engine) = self.browser.clone() else {
            return;
        };
        let Some((url, ..)) = engine.info(self.browser_key()) else {
            return;
        };
        if url == self.browser_view.synced_url || url.is_empty() || url == "about:blank" {
            return;
        }
        if self.address.read(cx).focus_handle(cx).is_focused(window) {
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
        let local = (
            f32::from(pos.x - b.origin.x),
            f32::from(pos.y - b.origin.y),
        );
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
        if m.platform {
            let edit = |command: &'static str, k: &'static str, code: &'static str, vk: u32| {
                let spec = KeySpec { key: k, code, vk, text: "" };
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
                "x" => edit("cut", "x", "KeyX", 88),
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
        let (Some(engine), Some(spec)) = (self.browser.clone(), key_spec(&ev.keystroke.key))
        else {
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
            .child(nav("page-back", IconName::ArrowLeft, "Back").on_click(cx.listener(
                |this, _, _, _| {
                    if let Some(e) = &this.browser {
                        e.history(this.browser_key(), true);
                    }
                },
            )))
            .child(nav("page-forward", IconName::ArrowRight, "Forward").on_click(cx.listener(
                |this, _, _, _| {
                    if let Some(e) = &this.browser {
                        e.history(this.browser_key(), false);
                    }
                },
            )))
            .child(
                nav(
                    "page-reload",
                    if loading { IconName::Loader } else { IconName::RotateCw },
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
        let (bounds, scale) = (self.browser_view.bounds.clone(), self.browser_view.scale.clone());
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
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.page_key(ev, window, cx)
            }))
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
                                        el.child(self.button("dialog-cancel", "Cancel", false).on_click(
                                            cx.listener(|this, _, _, cx| {
                                                if let Some(e) = &this.browser {
                                                    e.answer_dialog(this.browser_key(), false);
                                                }
                                                cx.notify();
                                            }),
                                        ))
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
                        "No dev server found on the usual ports. Start one in the Terminal."
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

    pub(crate) fn browser_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let key = self.browser_key();
        let info = self
            .browser
            .as_ref()
            .filter(|_| self.browser_view.pages.contains(&key))
            .and_then(|e| e.info(key));
        let page = info.is_some();
        let (loading, dialog) = info.map(|(_, _, l, d)| (l, d)).unwrap_or((false, None));
        div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .child(self.browser_bar(page, loading, cx))
            .child(if page {
                self.page_view(dialog, cx)
            } else {
                self.start_page(cx)
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
