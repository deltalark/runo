# Contributing

Thanks for helping with Runo. It is a Rust workspace; the app lives in `native/runo-app`.

## Build and run

```bash
cargo run -p runo
```

On Linux you may need the UI toolkit's system libraries (libxkbcommon, Wayland or X11, and Vulkan development packages).

## Layout

- `native/runo-app/src/main.rs` - window, sidebar, conversation, composer, right pane
- `native/runo-app/src/settings_ui.rs` - the Settings screen
- `native/runo-app/src/settings.rs` - persisted settings, palette, skills and MCP discovery
- `native/runo-app/src/agent.rs` - launching agent CLIs and shell commands, building their arguments
- `native/runo-app/src/stream.rs` and `native/runo-app/src/harness/` - turning each CLI's output into typed events, and the per-session event log
- `native/runo-app/src/caps.rs` - discovering each CLI's models, modes and levels
- `native/runo-app/src/git.rs` - worktrees, status and diffs
- `native/runo-app/src/store.rs` - projects and sessions on disk
- `vendor/portable-pty` - vendored PTY library, kept for the terminal

## Before you open a pull request

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

Agent CLIs change their flags between versions. Prefer discovering options from the CLI over hard-coding them, and add a test for any argument mapping you change. When a CLI's output format changes, record a real turn and add it under `native/runo-app/tests/fixtures` (see the README there).
