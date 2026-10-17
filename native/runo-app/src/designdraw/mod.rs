//! The design canvas: screens kept as real HTML in `design/`, drawn by WebKit on
//! macOS (a headless Chromium elsewhere), edited directly on the canvas and with AI.

pub(crate) mod brief;
pub(crate) mod checks;
pub(crate) mod codegen;
pub(crate) mod dom;
pub(crate) mod export;
pub(crate) mod fields;
pub(crate) mod library;
pub(crate) mod model;
pub(crate) mod notes;
pub(crate) mod parts;
pub(crate) mod render;
pub(crate) mod shared;
pub(crate) mod spec;
pub(crate) mod styles;
pub(crate) mod tokens;
pub(crate) mod vector;
pub(crate) mod versions;
pub(crate) mod view;
#[cfg(target_os = "macos")]
pub(crate) mod webkit;
