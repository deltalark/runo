# block 0.1.6

Source: https://crates.io/crates/block/0.1.6

Archive SHA-256 (verified against Runo's original Cargo.lock):
`0d8c1fef690941d3e7788d328517591fecc684c084084702d6ff1641e993699a`

Runo does not use this crate directly. It comes in on macOS through `cocoa`,
`cocoa-foundation`, `metal`, `core-video` and `core-graphics2`, all under gpui, and
is patched in through `[patch.crates-io]` in the workspace `Cargo.toml`.

This copy preserves the published source and manifest. The published crate ships
no license file, so `LICENSE` is the standard MIT text its manifest names.
Package-only metadata (`Cargo.toml.orig`, `.cargo_vcs_info.json`) is omitted, and
the manifest's `objc_test_utils` dev-dependency is dropped because the published
crate does not include its path.

The local patch, in `src/lib.rs` only: `Class`, the type of the extern static
`_NSConcreteStackBlock`, was an empty enum. Rust warns that a static of an
uninhabited type will become a hard error (rust-lang/rust#74840), and Cargo
reported it as a future-incompatibility on every macOS build. It is now an
opaque `#[repr(C)]` zero-sized struct, the usual way to name an extern type; the
static is only ever used by address, so nothing else changes.

As a path dependency the crate's own lints are no longer capped, so the patch also
spells out the `"C"` ABI on its `extern` blocks and function pointers, which was
already the implied default; without it every build printed 46 deprecation
warnings in place of the one future-incompatibility note.

Remove this copy once gpui's macOS dependencies move off `block` (to `block2`).
