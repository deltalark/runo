//! The design canvas: screens kept as real HTML in `design/`, drawn by WebKit on
//! macOS (a headless Chromium elsewhere), edited directly on the canvas and with AI.

pub(crate) mod brief;
pub(crate) mod checks;
pub(crate) mod dom;
pub(crate) mod model;
pub(crate) mod render;
pub(crate) mod view;
#[cfg(target_os = "macos")]
pub(crate) mod webkit;
