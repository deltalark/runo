# vt100 0.15.2

Source: https://crates.io/crates/vt100/0.15.2

Archive SHA-256 (verified against Runo's original Cargo.lock):
`84cd863bf0db7e392ba3bd04994be3473491b31e66340672af5d11943c6274de`

This copy preserves the published source, manifest, and MIT license. Package-only
metadata (`Cargo.lock`, `Cargo.toml.orig`, `.cargo_vcs_info.json`) is omitted, and
the manifest's dev-dependencies are dropped because the published crate ships no
tests.

The local patch adds the dim (SGR 2) and strikethrough (SGR 9, reset by 29)
attributes: `Cell::dim`, `Cell::strikethrough` and the matching `Screen`
accessors. SGR 22 clears bold and dim together, and the formatted output writes
them back the same way.

Only these upstream files are modified:

- `src/attrs.rs`
- `src/cell.rs`
- `src/screen.rs`
- `src/term.rs`

vt100 0.16 adds dim but still not strikethrough. Remove this copy once an
upstream release supports both.
