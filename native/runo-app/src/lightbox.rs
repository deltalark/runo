//! The image viewer: click a picture in a chat and it opens large over the window, fitted
//! to it, with zoom, a gallery of every picture in the conversation, and Copy, Open,
//! Show and Save. The maths (fit, zoom around a point, clamping, gallery order) is plain
//! functions at the top, tested at the foot.

use super::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Room left around the picture, and the header's height above it.
const PAD: f32 = 24.;
const HEADER: f32 = 64.;
/// The most a picture is magnified: eight screen points to its pixel.
pub(crate) const MAX_SCALE: f32 = 8.;
/// One step of ⌘+ or ⌘-.
const STEP: f32 = 1.25;
/// Pictures larger than this on either side, or with more pixels than `MAX_PIXELS`, are
/// decoded once and scaled down to `SHOWN_EDGE`, so a huge screenshot doesn't hold
/// hundreds of megabytes of texture.
const MAX_EDGE: u32 = 8192;
const MAX_PIXELS: u64 = 40_000_000;
const SHOWN_EDGE: u32 = 4096;

/// How large the picture is drawn and where: `scale` screen points per image pixel, and
/// the picture's centre offset from the middle of the stage by `x`, `y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Zoom {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
}

/// The scale that fits `image` inside `stage`, never enlarging a small one past its size.
pub(crate) fn fit_scale(image: (f32, f32), stage: (f32, f32)) -> f32 {
    if image.0 <= 0. || image.1 <= 0. || stage.0 <= 0. || stage.1 <= 0. {
        return 1.;
    }
    (stage.0 / image.0).min(stage.1 / image.1).min(1.)
}

impl Zoom {
    pub(crate) fn fit(image: (f32, f32), stage: (f32, f32)) -> Self {
        Zoom {
            scale: fit_scale(image, stage),
            x: 0.,
            y: 0.,
        }
    }

    /// Keep the scale between fitted and `MAX_SCALE`, and the picture over the stage: a
    /// side narrower than the stage stays centred, a wider one can't leave a gap.
    pub(crate) fn clamp(self, image: (f32, f32), stage: (f32, f32)) -> Self {
        let min = fit_scale(image, stage);
        let scale = self.scale.clamp(min, MAX_SCALE.max(min));
        let reach = |img: f32, room: f32| ((img * scale - room) / 2.).max(0.);
        let (rx, ry) = (reach(image.0, stage.0), reach(image.1, stage.1));
        Zoom {
            scale,
            x: self.x.clamp(-rx, rx),
            y: self.y.clamp(-ry, ry),
        }
    }

    /// Scale by `factor`, keeping the image point under `at` (relative to the stage's
    /// middle) where it is.
    pub(crate) fn zoom_at(
        self,
        factor: f32,
        at: (f32, f32),
        image: (f32, f32),
        stage: (f32, f32),
    ) -> Self {
        let wanted = Zoom {
            scale: self.scale * factor,
            ..self
        }
        .clamp(image, stage)
        .scale;
        let k = wanted / self.scale;
        Zoom {
            scale: wanted,
            x: at.0 - (at.0 - self.x) * k,
            y: at.1 - (at.1 - self.y) * k,
        }
        .clamp(image, stage)
    }

    pub(crate) fn pan(self, dx: f32, dy: f32, image: (f32, f32), stage: (f32, f32)) -> Self {
        Zoom {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
        .clamp(image, stage)
    }

    /// Double-click: from fitted (or any zoom) to actual size around `at`, and from actual
    /// size back to fitted.
    pub(crate) fn toggle_actual(
        self,
        at: (f32, f32),
        image: (f32, f32),
        stage: (f32, f32),
    ) -> Self {
        let fit = fit_scale(image, stage);
        if (self.scale - 1.).abs() < 1e-3 && (fit - 1.).abs() > 1e-3 {
            Zoom::fit(image, stage)
        } else {
            self.zoom_at(1. / self.scale, at, image, stage)
        }
    }

    /// Whether the picture is magnified past fitting, so it can be moved around.
    pub(crate) fn zoomed(&self, image: (f32, f32), stage: (f32, f32)) -> bool {
        self.scale > fit_scale(image, stage) + 1e-3
    }
}

/// Every picture in a conversation, in order: each group is one message's pictures (and
/// the last may be the composer's).
pub(crate) fn gallery(groups: &[&[String]]) -> Vec<PathBuf> {
    groups
        .iter()
        .flat_map(|g| g.iter().map(PathBuf::from))
        .collect()
}

/// Where picture `item` of group `group` sits in the gallery.
pub(crate) fn flat_index(groups: &[&[String]], group: usize, item: usize) -> Option<usize> {
    let g = groups.get(group)?;
    (item < g.len()).then(|| groups[..group].iter().map(|g| g.len()).sum::<usize>() + item)
}

/// The picture `delta` along from `index`, stopping at either end.
pub(crate) fn step(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + delta).clamp(0, len as isize - 1) as usize
}

/// "2 / 5", or nothing for a single picture.
pub(crate) fn counter(index: usize, len: usize) -> Option<String> {
    (len > 1).then(|| format!("{} / {}", index + 1, len))
}

/// What the viewer knows about the picture shown.
pub(crate) struct Info {
    bytes: u64,
    /// Width and height in pixels, when the header could be read.
    size: Option<(u32, u32)>,
}

enum Shown {
    /// Drawn from the file, as GPUI decodes it (animated GIFs move).
    File(PathBuf),
    /// Scaled down from a picture too large to keep whole.
    Scaled(Arc<RenderImage>),
}

enum Load {
    Reading,
    Ready(Info, Shown),
    Failed(String),
}

/// Which picture of a session was clicked.
#[derive(Clone, Copy)]
pub(crate) enum Pick {
    /// Message `.0`, picture `.1`.
    Message(usize, usize),
    /// Picture `.0` attached in the composer.
    Attached(usize),
}

pub(crate) struct Lightbox {
    images: Vec<PathBuf>,
    index: usize,
    load: Load,
    /// None while fitted, so a resized window refits.
    zoom: Option<Zoom>,
    /// The last pointer position while dragging the picture.
    drag: Option<Point<Pixels>>,
    /// A drag moved the picture, so the click it ends with doesn't close the viewer.
    moved: bool,
    /// Counts loads, so a slow one for a picture no longer shown is dropped.
    epoch: u64,
    /// Counts openings, so the fade plays once each time.
    opening: u64,
}

/// Read a picture's size, and scale a huge one down. Runs off the main thread.
fn load(path: &Path) -> Load {
    let meta = match std::fs::metadata(path) {
        Ok(m) if m.is_file() => m,
        Ok(_) => return Load::Failed("It isn't a file.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Load::Failed("The file is no longer there.".into())
        }
        Err(e) => return Load::Failed(format!("Couldn't read it: {e}")),
    };
    let size = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .ok()
        .and_then(|r| r.into_dimensions().ok());
    let info = Info {
        bytes: meta.len(),
        size,
    };
    match size {
        Some((w, h)) if w > MAX_EDGE || h > MAX_EDGE || w as u64 * h as u64 > MAX_PIXELS => {
            match scaled(path) {
                Ok(image) => Load::Ready(info, Shown::Scaled(image)),
                Err(e) => Load::Failed(format!("It's too large to open: {e}")),
            }
        }
        _ => Load::Ready(info, Shown::File(path.to_path_buf())),
    }
}

/// The picture decoded and scaled to fit `SHOWN_EDGE`, as BGRA for GPUI.
fn scaled(path: &Path) -> Result<Arc<RenderImage>, String> {
    let image = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let mut rgba = image.thumbnail(SHOWN_EDGE, SHOWN_EDGE).into_rgba8();
    drop(image);
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

/// The clipboard's name for the picture's format, from its bytes.
fn clip_format(bytes: &[u8]) -> Option<ImageFormat> {
    Some(match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => ImageFormat::Png,
        image::ImageFormat::Jpeg => ImageFormat::Jpeg,
        image::ImageFormat::WebP => ImageFormat::Webp,
        image::ImageFormat::Gif => ImageFormat::Gif,
        image::ImageFormat::Bmp => ImageFormat::Bmp,
        image::ImageFormat::Tiff => ImageFormat::Tiff,
        image::ImageFormat::Ico => ImageFormat::Ico,
        _ => return None,
    })
}

/// The stage, where the picture is drawn: the window less the header and padding.
fn stage_rect(window: &Window) -> Bounds<Pixels> {
    let v = window.viewport_size();
    Bounds::new(
        point(px(PAD), px(HEADER)),
        size(
            (v.width - px(2. * PAD)).max(px(1.)),
            (v.height - px(HEADER + PAD)).max(px(1.)),
        ),
    )
}

fn wh(s: Size<Pixels>) -> (f32, f32) {
    (f32::from(s.width), f32::from(s.height))
}

/// A pointer position as an offset from the stage's middle.
fn from_middle(p: Point<Pixels>, stage: Bounds<Pixels>) -> (f32, f32) {
    let c = stage.center();
    (f32::from(p.x - c.x), f32::from(p.y - c.y))
}

impl RunoApp {
    /// A picture in a chat that opens the viewer when clicked.
    pub(crate) fn viewable(
        &self,
        id: impl Into<ElementId>,
        session: u64,
        pick: Pick,
        image: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .flex_none()
            .cursor_pointer()
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new("Click to view").build(window, cx)
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.open_lightbox(session, pick, window, cx);
            }))
            .child(image)
    }

    /// Open the viewer on a session's picture, with every picture in the conversation
    /// to step through.
    pub(crate) fn open_lightbox(
        &mut self,
        session: u64,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.state.session(session) else {
            return;
        };
        let mut groups: Vec<&[String]> = s.messages.iter().map(|m| m.images.as_slice()).collect();
        let attached = self
            .attachments
            .get(&session)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        groups.push(attached);
        let at = match pick {
            Pick::Message(m, i) => flat_index(&groups, m, i),
            Pick::Attached(i) => flat_index(&groups, groups.len() - 1, i),
        };
        let Some(index) = at else {
            return;
        };
        let opening = self.lightbox.as_ref().map_or(0, |lb| lb.opening) + 1;
        self.lightbox = Some(Lightbox {
            images: gallery(&groups),
            index,
            load: Load::Reading,
            zoom: None,
            drag: None,
            moved: false,
            epoch: 0,
            opening,
        });
        self.load_lightbox(cx);
        window.focus(&self.lightbox_focus, cx);
        cx.notify();
    }

    /// Close the viewer and give the keyboard back to the composer.
    pub(crate) fn close_lightbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lightbox = None;
        if self.view == View::Session && self.current().is_some() {
            self.focus_composer(window, cx);
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn load_lightbox(&mut self, cx: &mut Context<Self>) {
        let Some(lb) = self.lightbox.as_mut() else {
            return;
        };
        let Some(path) = lb.images.get(lb.index).cloned() else {
            return;
        };
        lb.epoch += 1;
        lb.load = Load::Reading;
        lb.zoom = None;
        let epoch = lb.epoch;
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { load(&path) }).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(lb) = this.lightbox.as_mut().filter(|lb| lb.epoch == epoch) {
                    lb.load = loaded;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn lightbox_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(lb) = self.lightbox.as_mut() else {
            return;
        };
        let next = step(lb.index, lb.images.len(), delta);
        if next != lb.index {
            lb.index = next;
            self.load_lightbox(cx);
            cx.notify();
        }
    }

    /// The picture's pixel size and the stage's, when both are known.
    fn lightbox_geometry(&self, window: &Window) -> Option<((f32, f32), Bounds<Pixels>)> {
        let lb = self.lightbox.as_ref()?;
        let Load::Ready(info, _) = &lb.load else {
            return None;
        };
        let (w, h) = info.size?;
        Some(((w as f32, h as f32), stage_rect(window)))
    }

    /// Change the zoom with `f` given the current one, the picture and the stage.
    fn lightbox_zoom(
        &mut self,
        window: &Window,
        f: impl FnOnce(Zoom, (f32, f32), (f32, f32)) -> Zoom,
    ) -> bool {
        let Some((image, stage)) = self.lightbox_geometry(window) else {
            return false;
        };
        let Some(lb) = self.lightbox.as_mut() else {
            return false;
        };
        let room = wh(stage.size);
        let now = lb.zoom.unwrap_or_else(|| Zoom::fit(image, room));
        let next = f(now, image, room);
        // Back at fitted, follow the window again.
        lb.zoom = next.zoomed(image, room).then_some(next);
        next != now
    }

    fn lightbox_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let m = &keystroke.modifiers;
        let plain = !m.modified();
        let cmd = m.secondary() && !m.alt;
        match keystroke.key.as_str() {
            "left" if plain => self.lightbox_step(-1, cx),
            "right" if plain => self.lightbox_step(1, cx),
            "=" | "+" if cmd => {
                self.lightbox_zoom(window, |z, i, r| z.zoom_at(STEP, (0., 0.), i, r));
            }
            "-" if cmd => {
                self.lightbox_zoom(window, |z, i, r| z.zoom_at(1. / STEP, (0., 0.), i, r));
            }
            "0" if cmd => {
                self.lightbox_zoom(window, |_, i, r| Zoom::fit(i, r));
            }
            "c" if cmd => self.lightbox_copy(cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn lightbox_path(&self) -> Option<PathBuf> {
        let lb = self.lightbox.as_ref()?;
        lb.images.get(lb.index).cloned()
    }

    fn lightbox_copy(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.lightbox_path() else {
            return;
        };
        let copied = std::fs::read(&path)
            .map_err(|e| format!("Couldn't read the picture: {e}"))
            .and_then(|bytes| {
                let format = clip_format(&bytes)
                    .ok_or_else(|| "Runo can't copy this kind of picture.".to_string())?;
                cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(format, bytes)));
                Ok(())
            });
        match copied {
            Ok(()) => self.info = Some(("Copied the picture".into(), std::time::Instant::now())),
            Err(e) => self.fail("image", e),
        }
    }

    fn lightbox_save(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.lightbox_path() else {
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image.png".into());
        let home = crate::store::home_dir();
        let dir = home
            .as_ref()
            .map(|h| h.join("Downloads"))
            .filter(|d| d.is_dir())
            .or(home)
            .unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(to))) = rx.await {
                let result = std::fs::copy(&path, &to);
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(_) => {
                            this.info = Some((
                                format!("Saved to {}", to.display()),
                                std::time::Instant::now(),
                            ))
                        }
                        Err(e) => this.fail("image", format!("Couldn't save the picture: {e}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The viewer over the window, when it is open.
    pub(crate) fn lightbox_view(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let lb = self.lightbox.as_ref()?;
        let path = lb.images.get(lb.index)?.clone();
        let stage = stage_rect(window);
        let room = wh(stage.size);
        let count = lb.images.len();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // The header: name, size, dimensions and place in the gallery; then the actions.
        let mut facts: Vec<String> = Vec::new();
        let mut zoom_label = None;
        if let Load::Ready(info, _) = &lb.load {
            facts.push(crate::fileview::human_size(info.bytes));
            if let Some((w, h)) = info.size {
                facts.push(format!("{w} × {h}"));
                let image = (w as f32, h as f32);
                let z = lb.zoom.unwrap_or_else(|| Zoom::fit(image, room));
                zoom_label = Some(format!("{:.0}%", z.scale * 100.));
            }
        }
        facts.extend(counter(lb.index, count));
        let failed = matches!(lb.load, Load::Failed(_));
        let action = |id: &'static str, name: IconName, tip: &'static str| {
            self.ghost_button_sized(id, name, tip, 30.)
        };
        let open_tip = if cfg!(target_os = "macos") {
            "Open in Preview"
        } else {
            "Open in the default app"
        };
        let reveal_tip = if cfg!(target_os = "macos") {
            "Show in Finder"
        } else {
            "Show in folder"
        };
        let header = div()
            .id("lightbox-header")
            .absolute()
            .top(px(12.))
            .left(px(PAD))
            .right(px(PAD))
            .h(px(HEADER - 24.))
            .pl_3()
            .pr_1()
            .flex()
            .items_center()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_lg()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(icon(IconName::Image, 14., c_muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(facts.join(" · ")),
                    ),
            )
            .when_some(zoom_label, |el, label| {
                el.child(
                    div()
                        .id("lightbox-zoom")
                        .flex_none()
                        .px_2()
                        .py(px(2.))
                        .rounded_md()
                        .cursor_pointer()
                        .text_size(px(11.5))
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Fit or actual size (double-click the picture)",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.lightbox_zoom(window, |z, i, r| z.toggle_actual((0., 0.), i, r));
                            cx.notify();
                        }))
                        .child(label),
                )
            })
            .when(!failed, |el| {
                el.child(
                    action("lightbox-copy", IconName::Copy, "Copy image").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.lightbox_copy(cx);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    action("lightbox-open", IconName::ExternalLink, open_tip).on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(p) = this.lightbox_path() {
                                cx.open_with_system(&p);
                            }
                        }),
                    ),
                )
                .child(
                    action("lightbox-reveal", IconName::FolderOpen, reveal_tip).on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(p) = this.lightbox_path() {
                                cx.reveal_path(&p);
                            }
                        }),
                    ),
                )
                .child(
                    action("lightbox-save", IconName::Download, "Save as…")
                        .on_click(cx.listener(|this, _, _, cx| this.lightbox_save(cx))),
                )
            })
            .child(div().w(px(1.)).h(px(18.)).bg(divider()))
            .child(
                action("lightbox-close", IconName::X, "Close (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| this.close_lightbox(window, cx))),
            );

        // The picture, or why it can't be shown.
        let cant = move |why: String, name: String| {
            div()
                .size_full()
                .v_flex()
                .items_center()
                .justify_center()
                .gap_2()
                .child(icon(IconName::Image, 28., 0xa0a0a0))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(0., 0., 1., 1.))
                        .child("Can't show this image"),
                )
                .child(
                    div()
                        .max_w(px(420.))
                        .text_center()
                        .text_size(px(12.))
                        .text_color(hsla(0., 0., 1., 0.7))
                        .child(if why.is_empty() { name } else { why }),
                )
                .into_any_element()
        };
        let zoomed;
        let picture: AnyElement = match &lb.load {
            Load::Reading => {
                zoomed = false;
                div().into_any_element()
            }
            Load::Failed(why) => {
                zoomed = false;
                cant(why.clone(), name.clone())
            }
            Load::Ready(info, shown) => {
                let source: ImageSource = match shown {
                    Shown::File(p) => p.clone().into(),
                    Shown::Scaled(r) => r.clone().into(),
                };
                let fallback_name = name.clone();
                let base = img(source)
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        cant(
                            "Runo couldn't read it as a picture.".into(),
                            fallback_name.clone(),
                        )
                    });
                match info.size {
                    Some((w, h)) => {
                        let image = (w as f32, h as f32);
                        let z = lb
                            .zoom
                            .unwrap_or_else(|| Zoom::fit(image, room))
                            .clamp(image, room);
                        zoomed = z.zoomed(image, room);
                        let (dw, dh) = (image.0 * z.scale, image.1 * z.scale);
                        let dragging = lb.drag.is_some();
                        div()
                            .id("lightbox-picture")
                            .absolute()
                            .left(px(room.0 / 2. + z.x - dw / 2.))
                            .top(px(room.1 / 2. + z.y - dh / 2.))
                            .w(px(dw))
                            .h(px(dh))
                            .when(zoomed, |el| {
                                el.cursor(if dragging {
                                    CursorStyle::ClosedHand
                                } else {
                                    CursorStyle::OpenHand
                                })
                            })
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                                    if e.click_count == 2 {
                                        let at = from_middle(e.position, stage_rect(window));
                                        this.lightbox_zoom(window, |z, i, r| {
                                            z.toggle_actual(at, i, r)
                                        });
                                    } else if let Some(lb) = this.lightbox.as_mut() {
                                        lb.drag = Some(e.position);
                                        lb.moved = false;
                                    }
                                    cx.notify();
                                }),
                            )
                            .child(base.size_full())
                            .into_any_element()
                    }
                    // A format whose size Runo can't read (SVG): fitted, without zoom.
                    None => {
                        zoomed = false;
                        div()
                            .id("lightbox-picture")
                            .size_full()
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(base.size_full())
                            .into_any_element()
                    }
                }
            }
        };

        let arrow =
            |id: &'static str, glyph: IconName, tip: &'static str, delta: isize, on: bool| {
                div()
                    .id(id)
                    .absolute()
                    .top(px(f32::from(stage.center().y) - 20.))
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(float_edge())
                    .bg(float(c_sheet()))
                    .shadow_lg()
                    .when(on, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(rgb(c_raised())))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                            })
                    })
                    .when(!on, |el| el.opacity(0.35))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.lightbox_step(delta, cx);
                    }))
                    .child(icon(glyph, 18., c_text()))
            };

        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.82)
        } else {
            hsla(0., 0., 0., 0.7)
        };
        let overlay = div()
            .id("lightbox")
            .absolute()
            .inset_0()
            .bg(backdrop)
            .occlude()
            .track_focus(&self.lightbox_focus)
            .key_context("Lightbox")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if this.lightbox_key(&e.keystroke, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if let Some(lb) = this.lightbox.as_mut() {
                        lb.moved = false;
                    }
                }),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(lb) = this.lightbox.as_mut() {
                    if std::mem::take(&mut lb.moved) {
                        return;
                    }
                }
                this.close_lightbox(window, cx);
            }))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| {
                let d = e.delta.pixel_delta(px(20.));
                let (dx, dy) = (f32::from(d.x), f32::from(d.y));
                let changed = if e.modifiers.secondary() {
                    let at = from_middle(e.position, stage_rect(window));
                    let factor = (dy * 0.01).exp();
                    this.lightbox_zoom(window, |z, i, r| z.zoom_at(factor, at, i, r))
                } else {
                    this.lightbox_zoom(window, |z, i, r| z.pan(dx, dy, i, r))
                };
                if changed {
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .on_pinch(cx.listener(|this, e: &PinchEvent, window, cx| {
                let at = from_middle(e.position, stage_rect(window));
                let factor = (1. + e.delta).max(0.1);
                if this.lightbox_zoom(window, |z, i, r| z.zoom_at(factor, at, i, r)) {
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                let Some(from) = this.lightbox.as_ref().and_then(|lb| lb.drag) else {
                    return;
                };
                if e.pressed_button != Some(MouseButton::Left) {
                    if let Some(lb) = this.lightbox.as_mut() {
                        lb.drag = None;
                    }
                    cx.notify();
                    return;
                }
                let (dx, dy) = (
                    f32::from(e.position.x - from.x),
                    f32::from(e.position.y - from.y),
                );
                this.lightbox_zoom(window, |z, i, r| z.pan(dx, dy, i, r));
                if let Some(lb) = this.lightbox.as_mut() {
                    lb.drag = Some(e.position);
                    lb.moved |= dx.abs() + dy.abs() > 0.5;
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if let Some(lb) = this.lightbox.as_mut() {
                        if lb.drag.take().is_some() {
                            cx.notify();
                        }
                    }
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(stage.origin.x)
                    .top(stage.origin.y)
                    .w(stage.size.width)
                    .h(stage.size.height)
                    .overflow_hidden()
                    .when(zoomed, |el| el.rounded_md())
                    .child(picture),
            )
            .child(header)
            .when(count > 1, |el| {
                el.child(
                    arrow(
                        "lightbox-prev",
                        IconName::ChevronLeft,
                        "Previous (←)",
                        -1,
                        lb.index > 0,
                    )
                    .left(px(PAD + 8.)),
                )
                .child(
                    arrow(
                        "lightbox-next",
                        IconName::ChevronRight,
                        "Next (→)",
                        1,
                        lb.index + 1 < count,
                    )
                    .right(px(PAD + 8.)),
                )
            });
        Some(if motion::calm(cx) {
            overlay.into_any_element()
        } else {
            fade(overlay, ("lightbox-fade", lb.opening as usize), 160)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        counter, fit_scale, flat_index, gallery, load, step, Load, Shown, Zoom, MAX_EDGE,
        MAX_SCALE, SHOWN_EDGE,
    };
    use std::path::PathBuf;

    const STAGE: (f32, f32) = (1000., 800.);

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn fit_scale_fits_large_and_keeps_small() {
        // Wider than tall: the width decides.
        assert!(near(fit_scale((2000., 1000.), STAGE), 0.5));
        // Taller than wide: the height decides.
        assert!(near(fit_scale((1000., 1600.), STAGE), 0.5));
        // Smaller than the stage: shown at its own size.
        assert!(near(fit_scale((200., 100.), STAGE), 1.));
        // Nonsense sizes don't divide by zero.
        assert!(near(fit_scale((0., 100.), STAGE), 1.));
        assert!(near(fit_scale((100., 100.), (0., 0.)), 1.));
    }

    #[test]
    fn clamp_keeps_scale_in_range_and_picture_over_stage() {
        let image = (2000., 1600.);
        // Below fitted comes back to fitted, centred.
        let z = Zoom {
            scale: 0.1,
            x: 50.,
            y: -50.,
        }
        .clamp(image, STAGE);
        assert!(near(z.scale, 0.5));
        assert!(near(z.x, 0.) && near(z.y, 0.));
        // Past the maximum comes back to it.
        let z = Zoom {
            scale: 100.,
            x: 0.,
            y: 0.,
        }
        .clamp(image, STAGE);
        assert!(near(z.scale, MAX_SCALE));
        // At 100% the picture overhangs 500 each side across and 400 down: no further.
        let z = Zoom {
            scale: 1.,
            x: 9000.,
            y: -9000.,
        }
        .clamp(image, STAGE);
        assert!(near(z.x, 500.) && near(z.y, -400.));
    }

    #[test]
    fn zoom_at_keeps_the_point_under_the_pointer() {
        let image = (2000., 1600.);
        let start = Zoom::fit(image, STAGE);
        let at = (200., -100.);
        // The image point under `at` before zooming.
        let before = (
            (at.0 - start.x) / start.scale,
            (at.1 - start.y) / start.scale,
        );
        let z = start.zoom_at(2., at, image, STAGE);
        assert!(near(z.scale, 1.));
        let after = ((at.0 - z.x) / z.scale, (at.1 - z.y) / z.scale);
        assert!(near(before.0, after.0) && near(before.1, after.1));
        // Zooming out past fitted lands on fitted, centred.
        let back = z.zoom_at(0.01, at, image, STAGE);
        assert_eq!(back, Zoom::fit(image, STAGE));
        assert!(!back.zoomed(image, STAGE));
    }

    #[test]
    fn toggle_actual_goes_to_100_and_back() {
        let image = (2000., 1600.);
        let fit = Zoom::fit(image, STAGE);
        let actual = fit.toggle_actual((0., 0.), image, STAGE);
        assert!(near(actual.scale, 1.));
        assert_eq!(actual.toggle_actual((0., 0.), image, STAGE), fit);
        // A small picture is already at 100% when fitted: it stays there.
        let small = (100., 100.);
        let z = Zoom::fit(small, STAGE).toggle_actual((10., 10.), small, STAGE);
        assert!(near(z.scale, 1.) && near(z.x, 0.));
    }

    #[test]
    fn pan_stops_at_the_edges() {
        let image = (2000., 1600.);
        let z = Zoom {
            scale: 1.,
            x: 0.,
            y: 0.,
        };
        let moved = z.pan(-10_000., 30., image, STAGE);
        assert!(near(moved.x, -500.) && near(moved.y, 30.));
        // Fitted, there is nowhere to move.
        let fit = Zoom::fit(image, STAGE);
        assert_eq!(fit.pan(40., 40., image, STAGE), fit);
    }

    #[test]
    fn gallery_runs_through_messages_in_order() {
        let a = vec!["a.png".to_string(), "b.png".to_string()];
        let none: Vec<String> = Vec::new();
        let c = vec!["c.png".to_string()];
        let attached = vec!["d.png".to_string()];
        let groups: Vec<&[String]> = vec![&a, &none, &c, &attached];
        let all = gallery(&groups);
        assert_eq!(
            all,
            ["a.png", "b.png", "c.png", "d.png"]
                .map(PathBuf::from)
                .to_vec()
        );
        assert_eq!(flat_index(&groups, 0, 1), Some(1));
        assert_eq!(flat_index(&groups, 2, 0), Some(2));
        assert_eq!(flat_index(&groups, 3, 0), Some(3));
        // Out of range.
        assert_eq!(flat_index(&groups, 1, 0), None);
        assert_eq!(flat_index(&groups, 0, 2), None);
        assert_eq!(flat_index(&groups, 9, 0), None);
    }

    #[test]
    fn step_stops_at_the_ends_and_counter_reads_one_based() {
        assert_eq!(step(0, 5, -1), 0);
        assert_eq!(step(0, 5, 1), 1);
        assert_eq!(step(4, 5, 1), 4);
        assert_eq!(step(2, 5, -2), 0);
        assert_eq!(step(0, 0, 1), 0);
        assert_eq!(counter(1, 5).as_deref(), Some("2 / 5"));
        assert_eq!(counter(0, 1), None);
    }

    #[test]
    fn missing_file_cannot_be_shown() {
        let gone = std::env::temp_dir().join("runo-lightbox-no-such-file.png");
        assert!(matches!(load(&gone), Load::Failed(_)));
    }

    #[test]
    fn huge_pictures_are_scaled_down() {
        let dir = std::env::temp_dir().join(format!("runo-lightbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wide.png");
        image::RgbaImage::new(MAX_EDGE + 8, 4).save(&path).unwrap();
        match load(&path) {
            Load::Ready(info, Shown::Scaled(r)) => {
                assert_eq!(info.size, Some((MAX_EDGE + 8, 4)));
                assert!(r.size(0).width.0 as u32 <= SHOWN_EDGE);
            }
            _ => panic!("expected a scaled picture"),
        }
        let small = dir.join("small.png");
        image::RgbaImage::new(4, 4).save(&small).unwrap();
        assert!(matches!(load(&small), Load::Ready(_, Shown::File(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
