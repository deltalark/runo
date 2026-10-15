//! WebKit, the engine built into macOS, drawing the design canvas's screens: a web view
//! in a window that's never shown, loaded with a screen, asked things in JavaScript and
//! captured as an image. Nothing to install, and screens render as Safari renders them.
//!
//! WebKit only runs on the main thread, so each call is sent there and waits for its
//! answer. The canvas calls from its own thread; a call made on the main thread itself
//! runs there and keeps the run loop turning while it waits.

use block2::RcBlock;
use dispatch2::DispatchQueue;
use gpui_kit::RenderImage;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSBackingStoreType, NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWindow,
    NSWindowStyleMask,
};
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
use objc2_foundation::{NSDictionary, NSError, NSPoint, NSRect, NSSize, NSString, NSURL};
use objc2_web_kit::{WKContentWorld, WKSnapshotConfiguration, WKWebView, WKWebViewConfiguration};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one call may take before it counts as unanswered.
const ANSWER: Duration = Duration::from_secs(10);

struct View {
    window: Retained<NSWindow>,
    web: Retained<WKWebView>,
}

thread_local! {
    /// The web views, on the main thread, by page id.
    static VIEWS: RefCell<HashMap<u64, View>> = RefCell::new(HashMap::new());
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A page in WebKit. Its web view lives on the main thread; this is the handle to it.
pub(crate) struct Page {
    id: u64,
    size: (f32, f32),
}

/// Run `f` on the main thread, and wait up to `ANSWER` for what it sends.
fn main_thread<T: Send + 'static>(
    f: impl FnOnce(MainThreadMarker, Sender<T>) + Send + 'static,
) -> Option<T> {
    let (tx, rx) = channel();
    if let Some(mtm) = MainThreadMarker::new() {
        f(mtm, tx);
        // WebKit answers through the run loop, so keep it turning until it has.
        let start = Instant::now();
        loop {
            if let Ok(v) = rx.try_recv() {
                return Some(v);
            }
            if start.elapsed() > ANSWER {
                return None;
            }
            // SAFETY: the default mode, run briefly on the main thread.
            unsafe { CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, 0.005, true) };
        }
    }
    DispatchQueue::main().exec_async(move || {
        f(
            MainThreadMarker::new().expect("the main queue runs on the main thread"),
            tx,
        );
    });
    rx.recv_timeout(ANSWER).ok()
}

/// Run `f` on the main thread with the page's web view, and wait for what it sends.
fn on_main<T: Send + 'static>(
    id: u64,
    f: impl FnOnce(&View, MainThreadMarker, Sender<T>) + Send + 'static,
) -> Option<T> {
    main_thread(move |mtm, tx| {
        VIEWS.with(|v| {
            if let Some(view) = v.borrow().get(&id) {
                f(view, mtm, tx);
            }
        });
    })
}

impl Page {
    /// A new page of `w` by `h` CSS pixels.
    pub(crate) fn new(w: f32, h: f32) -> Result<Page, String> {
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        main_thread(move |mtm, tx| {
            let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(w as f64, h as f64));
            // SAFETY: plain AppKit and WebKit setup on the main thread.
            unsafe {
                let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    frame,
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                );
                window.setReleasedWhenClosed(false);
                let config = WKWebViewConfiguration::new(mtm);
                let web =
                    WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config);
                window.setContentView(Some(&web));
                VIEWS.with(|v| v.borrow_mut().insert(id, View { window, web }));
            }
            let _ = tx.send(());
        })
        .ok_or("WebKit didn't start.")?;
        Ok(Page { id, size: (w, h) })
    }

    /// Set the page's size in CSS pixels.
    fn resize(&mut self, w: f32, h: f32) {
        if self.size == (w, h) {
            return;
        }
        self.size = (w, h);
        on_main(self.id, move |v, _, tx| {
            let size = NSSize::new(w as f64, h as f64);
            v.window.setContentSize(size);
            v.web.setFrame(NSRect::new(NSPoint::new(0., 0.), size));
            let _ = tx.send(());
        });
    }

    /// Load `file` with reading allowed under `folder`, at `w` by `h`, and wait
    /// until it and its fonts have loaded. False when it didn't within `wait`.
    pub(crate) fn load(
        &mut self,
        file: &Path,
        folder: &Path,
        (w, h): (f32, f32),
        wait: Duration,
    ) -> bool {
        self.resize(w, h);
        // Mark the page shown now, so the new one is told apart from it.
        self.eval("window.__runoOld = true");
        let (file, folder) = (file.to_path_buf(), folder.to_path_buf());
        let started = on_main(self.id, move |v, _, tx| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
            let read = NSURL::fileURLWithPath(&NSString::from_str(&folder.to_string_lossy()));
            // SAFETY: a file URL and the folder it may read, on the main thread.
            let nav = unsafe { v.web.loadFileURL_allowingReadAccessToURL(&url, &read) };
            let _ = tx.send(nav.is_some());
        });
        if started != Some(true) {
            return false;
        }
        let start = Instant::now();
        while start.elapsed() < wait {
            let done = self.eval(
                "!window.__runoOld && document.readyState === 'complete' \
                 ? document.fonts.ready.then(() => true) : false",
            );
            if done.and_then(|v| v.as_bool()) == Some(true) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Run JavaScript in the page and give its value, after any promise it returns
    /// settles. None when it threw or the page didn't answer.
    pub(crate) fn eval(&self, js: &str) -> Option<Value> {
        let js = js.to_string();
        let out = on_main(self.id, move |v, mtm, tx| {
            let body = NSString::from_str(
                "const v = await (0, eval)(src); return JSON.stringify(v === undefined ? null : v);",
            );
            let key = NSString::from_str("src");
            let src = NSString::from_str(&js);
            let value: &AnyObject = &src;
            let args = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[value]);
            let done = RcBlock::new(move |result: *mut AnyObject, err: *mut NSError| {
                if !err.is_null() || result.is_null() {
                    let _ = tx.send(None);
                    return;
                }
                // SAFETY: the function returns a string, and WebKit hands back an NSString.
                let text = unsafe { &*(result as *const NSString) }.to_string();
                let _ = tx.send(Some(text));
            });
            // SAFETY: a function body with its arguments, in the page's own world.
            unsafe {
                v.web
                    .callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                        &body,
                        Some(&args),
                        None,
                        &WKContentWorld::pageWorld(mtm),
                        Some(&done),
                    );
            }
        })??;
        serde_json::from_str(&out).ok()
    }

    /// Prefer the dark or the light colour scheme.
    pub(crate) fn color_scheme(&self, dark: bool) {
        on_main(self.id, move |v, _, tx| {
            // SAFETY: AppKit's own appearance names.
            let name = unsafe {
                if dark {
                    NSAppearanceNameDarkAqua
                } else {
                    NSAppearanceNameAqua
                }
            };
            let appearance = NSAppearance::appearanceNamed(name);
            v.web.setAppearance(appearance.as_deref());
            let _ = tx.send(());
        });
    }

    /// The top `width` by `height` CSS pixels of the page, past its height if need be, at
    /// twice the size, as an image GPUI can draw.
    pub(crate) fn capture(&mut self, width: f32, height: f32) -> Option<Arc<RenderImage>> {
        let (w, h) = self.size;
        let taller = height > h;
        if taller {
            self.resize(width, height);
            // Lay the page out at its new height before it's captured. (A window that's
            // never shown gets no animation frames to wait for.)
            self.eval("document.documentElement.offsetHeight");
        }
        let png = on_main(self.id, move |v, mtm, tx| {
            // SAFETY: a snapshot of the view's own area, on the main thread.
            unsafe {
                let config = WKSnapshotConfiguration::new(mtm);
                config.setRect(NSRect::new(
                    NSPoint::new(0., 0.),
                    NSSize::new(width as f64, height as f64),
                ));
                config.setSnapshotWidth(Some(&objc2_foundation::NSNumber::new_f64(
                    (width * 2.) as f64,
                )));
                let done = RcBlock::new(move |image: *mut NSImage, _err: *mut NSError| {
                    let _ = tx.send(if image.is_null() {
                        None
                    } else {
                        png_of(&*image)
                    });
                });
                v.web
                    .takeSnapshotWithConfiguration_completionHandler(Some(&config), &done);
            }
        })
        .flatten();
        if taller {
            self.resize(w, h);
        }
        let mut rgba = image::load_from_memory_with_format(&png?, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
    }
}

/// An image as PNG bytes.
fn png_of(image: &NSImage) -> Option<Vec<u8>> {
    let tiff = image.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    // SAFETY: an empty property dictionary is allowed.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(png.to_vec())
}

impl Drop for Page {
    fn drop(&mut self) {
        let id = self.id;
        let close = move || {
            if let Some(v) = VIEWS.with(|v| v.borrow_mut().remove(&id)) {
                v.window.close();
            }
        };
        if MainThreadMarker::new().is_some() {
            close();
        } else {
            DispatchQueue::main().exec_async(close);
        }
    }
}
