# Architecture review: engineering quality, delivery and platforms

Reviewed at `5edcf99` (Release v0.5.9), 2026-10-25. This was a read-only review: nothing was built and no tests were run. The test counts come from grepping `#[test]` in `native/runo-app/src`. CI and release timings come from `gh run list`/`gh run view`. Latest crate versions come from crates.io.

Severity scale: **Critical** means users lose data or get a broken or unsafe build today. **High** means it breaks a platform or release, or is a large blind spot. **Medium** means it costs time or confidence. **Low** is hygiene. Effort: **S** is under half a day, **M** is one to three days, **L** is a week or more.

## Top 5

1. **Releases ship unsigned and un-notarized on macOS, the primary platform** (Critical, S once the certificate exists). The v0.5.9 run logged `No signing certificate is set, so this build is signed ad hoc` for both Mac jobs. The tag build still published. Fail tag builds when signing secrets are missing, and add the Developer ID secrets. See R1.
2. **Windows is checked only at release time, and a failing platform still publishes a partial release** (High, S). v0.5.7 went out as a non-draft release with Mac and Linux assets and no Windows zip. Add a Windows `cargo check`/clippy job to CI, and publish releases as drafts that are promoted only after every job succeeds. See C1 and R2.
3. **Downgrades and forward-incompatible saves silently drop sessions** (High, M). There is no schema version and no `#[serde(other)]` anywhere. A session saved with a newer `Status` variant (for example `Stopped`, added recently) fails to parse in an older build and is moved aside, so it disappears from the sidebar. See O1.
4. **Updates are downloaded without an integrity check, and there is no rollback or channel** (High, M). `curl` fetches whatever `releases/latest` names into `~/Downloads`. There is no checksum, no signature, no beta channel and no way back to the previous build. See R3.
5. **The turn lifecycle and the app shell have no tests** (High, L). `main.rs` (10,089 lines), `settings_ui.rs`, `crew_view.rs`, `branches.rs`, `fixes.rs` and `githistory.rs` have zero tests. `turns.rs` has 9 tests across 2,982 lines, and none drive a whole turn from start to end, through interruption or through the slot queue. See T1.

## Ranked gaps

| # | Gap | Area | Severity | Effort |
|---|---|---|---|---|
| R1 | Unsigned, un-notarized Mac releases | Release | Critical | S |
| C1 | No Windows job in CI | CI | High | S |
| R2 | Partial releases are published as final | Release | High | S |
| O1 | No schema version; downgrade drops sessions | Operability | High | M |
| R3 | Update download has no integrity check, rollback or channel | Release/updater | High | M |
| T1 | Turn lifecycle and app shell untested | Tests | High | L |
| X2 | Database passwords stored in plaintext on Linux and Windows (no owner-only ACL on Windows) | Cross-platform/security | High | M |
| X1 | Windows silently loses features (browser, design canvas, process list, keep-awake, remote checkpoints) | Cross-platform | High | M |
| R4 | Toolchain unpinned; builds not reproducible | Release/CI | Medium | S |
| D1 | gpui stack behind and its vendored patch undocumented | Dependencies | Medium | M |
| C2 | No cargo-deny / cargo-audit / license gate | CI | Medium | S |
| O2 | Native crashes are not captured; per-launch log unbounded | Operability | Medium | M |
| T2 | Time-based and process-global tests can hang or flake | Tests | Medium | S |
| C3 | CI runs twice per PR branch and on tags; `cargo-bundle` rebuilt every release | CI | Medium | S |
| X3 | Linux/Windows packaging: no installer, no AppImage, update opens a `.deb` | Cross-platform | Medium | M |
| D2 | vt100 three minors behind with a known wrap bug | Dependencies | Medium | M |
| Doc1 | No architecture doc; CONTRIBUTING layout stale | Docs | Medium | S |
| R5 | Changelog and GitHub release notes disagree | Release | Low | S |
| O3 | No "copy diagnostics" for bug reports | Operability | Low | S |
| Doc2 | README and bug template disagree on platform support | Docs | Low | S |
| C4 | No MSRV, no docs job | CI | Low | S |

Each gap is described in detail below.

---

## Tests

**What exists.** There are 904 `#[test]` functions across 214 source files (about 200k lines), and two are `#[ignore]`d. All of them are unit or in-crate integration tests. `native/runo-app/tests/` holds only recorded agent transcripts (`tests/fixtures/*.jsonl`) that the harness tests replay; there is no `tests/*.rs` integration crate. No test drives the GUI.

**Per-module coverage of the critical paths:**

| Path | File | Lines | Tests | Verdict |
|---|---|---|---|---|
| Store load/save, interrupted turns, corrupt state | `store.rs` | 1,778 | 23 | Good: `saves_and_loads_round_trip`, `running_sessions_load_as_interrupted`, `corrupt_state_is_moved_aside_not_overwritten`, `old_layouts_still_load`. The only data migration, `migrate_check_level` (`store.rs:1121`), is covered through `proof::migrate` (`proof.rs:1002-1010`). |
| Per-session files, atomic writes | `persist.rs` | 345 | 4 | Adequate. Nothing tests a session that fails to parse (`persist.rs:157` moves it aside). |
| Settings migrations | `settings.rs` | 1,681 | 11 | Good: `concurrency_settings_from_0_5_8_still_load_and_carry_over`, `settings_from_0_5_4_…_still_load`. |
| Turn lifecycle | `turns.rs`, `turnend.rs`, `main.rs` | 2,982 + 155 + 10,089 | 9 + 4 + 0 | **Weak.** The tests cover checkpoints, message re-queueing and turn-end classification. Start → stream → end, cancel, and the slot queue live in `main.rs`, which has no tests. |
| Crew action parsing | `crew.rs` | 1,832 | 29 | Strong, including `the_action_parser_survives_anything` (`crew.rs:1603`) and `broken_actions_run_nothing_and_say_why`. `crew_view.rs` (1,529 lines) has none. |
| Approvals | `approvals.rs` | 419 | 6 | Adequate; tests use the real file-based handshake. |
| Cleanup deletion rules | `cleanup.rs` | 828 | 7 | Good. Includes `nothing_outside_the_worktree_folder_or_off_prefix`, `work_in_progress_is_held` and a real-repo `scans_and_removes_in_a_real_repository`. Not covered: a worktree with unpushed commits but a clean tree, and symlinked worktree roots. |
| PR watcher | `prwatch.rs` | 736 | 11 | Strong state-machine coverage (wake caps, back-off, merge/close). |
| Agent CLI updater | `updater.rs` | 492 | 2 | Thin: two table tests (`plans_follow_the_copy_runo_runs`, `failures_are_explained`). |
| Runo self-update | `applog.rs:441-703`, `health.rs` | — | 1 | `newer_releases_are_found_with_their_download` covers parsing and asset choice. Nothing covers download, partial-file cleanup, or a Windows asset (the test asserts `pick("windows", …) == None` only because its fixture has no Windows asset, `applog.rs:888`). |

**Largest files with zero tests:** `main.rs` 10,089, `archdraw/view.rs` 7,929, `settings_ui.rs` 4,255, `designdraw/view/figma.rs` 2,087, `parts.rs` 1,600, `branches.rs` 1,568, `crew_view.rs` 1,529, `designdraw/view/chrome.rs` 1,232, `fixes.rs` 1,128, `githistory.rs` 1,074, `menubar_panel.rs` 992, `ctxmenu.rs` 913, `proof_ui.rs` 868, `clashes.rs` 855. Most of these are views. `fixes.rs`, `clashes.rs`, `githistory.rs` and `branches.rs` hold logic that could be unit-tested.

### T1. Turn lifecycle and app shell untested
- **Severity:** High. **Effort:** L.
- **Evidence:** The 0-test files listed above. `turns.rs` tests (`turns.rs:2798-2961`) cover helpers, not a turn end to end.
- **Impact:** The perf rework in 0.5.9 (turn end and checks moved off the UI thread) and the Crew wake logic changed the most sensitive code path without a regression net. Bugs show up as stuck "Working" sessions or lost queued messages.
- **Fix:** Pull the turn state machine (queued → slot → running → ending → done/failed/stopped) out of `main.rs` into a pure module that takes events and returns effects, as `prwatch.rs` already does. Then table-test it, and replay the existing `tests/fixtures/*.jsonl` transcripts through it. Start with interruption, cancel during checks, and a full slot queue.

### T2. Time-based and process-global tests
- **Severity:** Medium. **Effort:** S.
- **Evidence:**
  - Unbounded busy-wait loops: `wake.rs:140-155` (`while … { sleep(5ms) }`) and `approvals.rs:323-327, 373-377` (`loop { … sleep(20ms) }`). A regression hangs the test run instead of failing it.
  - `persist.rs:281` relies on file mtimes changing within a 20 ms sleep. That is fine on APFS and ext4, but flaky on coarse-mtime filesystems.
  - `Instant`/`Duration` assertions in `pty.rs` (17), `connections.rs` (13), `alerts.rs` (9) and `agent.rs` (8).
  - Process-wide environment mutation: `store.rs:1770-1775` and `connections.rs:3534-3552` change `HOME` and `RUNO_HOME`. `ENV_LOCK` (`store.rs:1742`) protects only tests that take it; any concurrent test that calls `data_dir()` (43 call sites) can race.
- **Tests that need external tools:** git (CI configures it, `ci.yml:48-53`) in `connections.rs`, `architecture.rs`, `git.rs`, `cleanup.rs`, `ship.rs` and `cove.rs`. Loopback TCP and `/dev/tcp` in `connections.rs:2595-2724`, `phone.rs` and `browser.rs`; these are Unix-shell specific and will need gating when Windows runs tests.
- **`#[ignore]` tests:** `archdraw/io.rs:1585` (`shapes_preview`) and `io.rs:1780` (`shipyard_pictures`, which needs `RUNO_SHOTS`). Both generate pictures for eyeballing and are fine to keep ignored.
- **Fix:** Add a `wait_until(timeout, cond)` helper that panics on timeout, and use it in the busy loops. Replace mtime checks with the digests `persist` already keeps. Make `data_dir()` read an overridable thread-local in tests instead of `HOME`. Consider `cargo nextest` (process per test) in CI.

---

## CI (`.github/workflows/ci.yml`)

The current setup: one job, a matrix of `macos-latest` and `ubuntu-latest` (`ci.yml:25`). It runs fmt (`:56`), clippy with `-D warnings` on `-p runo --all-targets` (`:59`) and `cargo test -p runo` (`:62`) on both OSes, with `Swatinem/rust-cache@v2` (`:46`). Recent runs take 3–7 minutes warm and up to 14 minutes cold. The nightly `compat.yml` builds on Ubuntu and checks every agent CLI's flags and protocols.

### C1. No Windows job
- **Severity:** High. **Effort:** S.
- **Evidence:** `ci.yml:25` has no `windows-latest`. v0.5.7's release run failed on Windows only (run 40177914410). The fix in 0.5.8 had to be found from a tag build (`CHANGELOG.md`, 0.5.8: "frame-time stats used a Unix-only call").
- **Impact:** Unix-only code (75 `cfg(unix)` gates, plus raw `libc` calls) is first compiled for Windows after the tag is pushed.
- **Fix:** Add `windows-latest` to the matrix running `cargo clippy -p runo --all-targets -- -D warnings`. Run `cargo test` there too once Unix-shell tests are gated (T2). It is about 15 minutes cold and fast once cached. If runner minutes matter, run Windows `cargo check` only on PRs to `main`.

### C2. No supply-chain gate
- **Severity:** Medium. **Effort:** S.
- **Evidence:** No `deny.toml`. No `cargo-deny`, `cargo-audit` or `cargo-vet` step in any workflow. There are 916 packages in `Cargo.lock`, including unmaintained `instant 0.1.13`, archived `paste`, `objc 0.2` and deprecated `cocoa` (all pulled in through gpui).
- **Impact:** RUSTSEC advisories (for example in `rustls`, `ring`, `postgres` or `rusqlite`'s bundled SQLite) and license drift (the app is MIT and bundles many crates) go unnoticed.
- **Fix:** Add `deny.toml` (advisories, licenses, bans for duplicate majors as warnings, sources) and an `EmbarkStudios/cargo-deny-action` job on Ubuntu, plus a weekly scheduled run so new advisories surface without a push.

### C3. Wasted runs
- **Severity:** Medium. **Effort:** S.
- **Evidence:** `ci.yml:4-6` has `on: push` and `on: pull_request` with no branch filter, so every PR branch runs twice (for example `runo/crew-wakes-decisions`: push 5 min plus pull_request 4 min). Tag pushes run CI too (`v0.5.9`, 4 min). Concurrency groups on `github.ref` differ between push and PR, so neither cancels the other. Release jobs `cargo install cargo-bundle --locked` from source on every job (`release.yml:59, 185`), unpinned and uncached.
- **Impact:** Double runner minutes. All parallel threads share the plan's CI minutes, and a merge queue of 5 merges cancelled 4 `main` runs in a row.
- **Fix:** Use `push: branches: [main]` plus `pull_request`. Install `cargo-bundle` with `taiki-e/install-action` at a pinned version.

### C4. No MSRV or docs job
- **Severity:** Low. **Effort:** S.
- **Evidence:** No `rust-version` in either manifest. `vendor/gpui-pre-apple` uses edition 2024, so the floor is at least 1.85, but nothing states it. `ci.yml:39` says "The repository pins no toolchain". There is no `cargo doc` check.
- **Fix:** Set `rust-version` in `[workspace.package]`, and pin the toolchain (see R4), which makes a separate MSRV job unnecessary. A `cargo doc --no-deps` job adds little for an app and is optional.

---

## Release (`.github/workflows/release.yml`) and the in-app updater

The current setup: a tag `v*` builds five jobs: Mac arm64 and x86_64 (zip and dmg), Linux x86_64 and arm64 on `ubuntu-22.04` (deb and tarball), and Windows x86_64 (zip). Each job checks that the tag matches `Cargo.toml` (`:39-46`, `:157-164`, `:247-254`) and attaches its own files with `softprops/action-gh-release@v2` and `generate_release_notes: true`. A release run takes about 25 minutes (v0.5.9: Mac arm64 was the long pole at 25 minutes). `workflow_dispatch` gives commit-stamped test builds.

### R1. Mac releases are unsigned and un-notarized
- **Severity:** Critical. **Effort:** S once a Developer ID exists (the Apple Developer Program is $99/yr).
- **Evidence:** `release.yml:101-105` falls back to `codesign --sign -` with a warning. The v0.5.9 run logged `##[warning]No signing certificate is set` on both Mac jobs and published anyway. Notarization (`:107-112`, `:122-128`) is skipped.
- **Impact:** On macOS 15+, Gatekeeper no longer offers right-click → Open for unnotarized apps. Users must go to System Settings → Privacy & Security → Open Anyway, which `docs/releasing.md:14` and the README's "drag Runo to Applications" (`README.md:305`) don't mention. Every update repeats the friction. Keychain items (database passwords, `security-framework`) are bound to the ad-hoc identity, which changes every build, so macOS can re-prompt for Keychain access after each update.
- **Fix:** Add the five secrets. Make tag builds fail when they are missing (an `if: startsWith(github.ref,'refs/tags/')` step that exits 1), and keep the ad-hoc fallback for `workflow_dispatch`/PR runs only. While unsigned, document the Open Anyway step in the README. Replace `codesign --deep` (`:98`), which Apple discourages, with signing nested code inside-out, or keep it but verify with `spctl -a -vv`.

### R2. Partial releases go out as final
- **Severity:** High. **Effort:** S.
- **Evidence:** Each platform job attaches its own files (`release.yml:137-142`, `:232-237`, `:295-300`). The first finished job creates a public, non-draft release. `gh release view v0.5.7` shows Mac and Linux assets, no Windows asset, `draft: false`. Nothing runs tests before release; a tag can be pushed on a commit whose CI failed.
- **Impact:** Every running Runo sees `releases/latest` as soon as the first job uploads (`applog.rs:583`). Windows users get "update available" with no asset for them (`pick_asset` returns `None`, so they are sent to the release page). Mac users can download a dmg before the Linux build has even finished or failed.
- **Fix:** Build jobs only upload artifacts. A final `publish` job `needs: [macos, linux, windows]` downloads them, writes `SHA256SUMS`, and creates the release in one step (or creates it as draft and flips it). Gate the tag on CI passing for that commit (`gh api …/commits/$SHA/check-runs` or `workflow_run`).

### R3. Update download has no integrity check, rollback or channel
- **Severity:** High. **Effort:** M.
- **Evidence:**
  - `applog.rs:551-639` asks `api.github.com/repos/deltalark/runo/releases/latest` through `curl` every 6 hours (`health.rs:14`). It skips drafts and prereleases (`applog.rs:454`), compares versions (`alerts::is_newer`), and picks an asset by OS and arch keywords (`:489-526`).
  - `download()` (`:656-703`) runs `curl -fsSL` into `~/Downloads` and renames `.part` to the final name. No hash or signature is checked, although `ring` is already used to verify agent manifests (`registry.rs`).
  - Nothing installs. The person opens the file (`health.rs:925`).
  - There is no beta channel (prereleases are ignored outright), no rollback, and no "skip this version" beyond hiding the card.
  - The check depends on a `curl` binary. That is fine on macOS, Linux and Windows 10+, but it fails quietly on Linux machines without curl.
- **Impact:** A compromised release asset or a MITM'd redirect would be opened by the user with no check. With R1 unsigned, nothing in the chain proves the binary came from the project. Because there is no rollback, a bad release (like 0.5.7's broken top bar, fixed in 0.5.9) can only be undone by hunting down the old asset by hand.
- **Fix:**
  1. Publish `SHA256SUMS` plus a detached signature made with the same `ring` key as `agents.json`. Verify both before renaming `.part`.
  2. Add a "Beta" setting that accepts `prerelease: true`, using `/releases` instead of `/releases/latest`.
  3. Keep the previous app in `~/.runo/previous/` (Mac zip or Windows zip) and offer "Go back to x.y.z". This is sound only if O1 is fixed first.
  4. Longer term on macOS, use Sparkle-style in-place replacement once signed.

### R4. Toolchain unpinned; builds not reproducible
- **Severity:** Medium. **Effort:** S.
- **Evidence:** `rustup update stable` in every workflow (`ci.yml:42`, `release.yml:50, 177, 258`). No `rust-toolchain.toml`. `cargo-bundle` is unpinned. The runners are `macos-latest` and `windows-latest`, which float.
- **Impact:** A new stable can fail clippy `-D warnings` on an unrelated PR or change codegen between a tag and its re-run. Two runs of the same tag aren't comparable, so you can't tell whether a release binary matches a commit.
- **Fix:** Add `rust-toolchain.toml` (a channel like `1.9x`, components rustfmt and clippy, the Mac targets), with a monthly bump PR. Pin `cargo-bundle` and the runner images (`macos-15`, `windows-2025`). Record `rustc -Vv` and the commit in the release notes. Full bit-for-bit reproducibility isn't worth chasing for a GUI app; pinning is.

### R5. Changelog vs release notes
- **Severity:** Low. **Effort:** S.
- **Evidence:** `docs/releasing.md:33` keeps `CHANGELOG.md` by hand (well-written, user-facing), but the release uses `generate_release_notes: true` (`release.yml:142`), so GitHub shows a PR-title list. The update card's "What's new" opens that page (`health.rs` `runo-notes`).
- **Fix:** Have the publish job (R2) extract the `## [x.y.z]` section of `CHANGELOG.md` as `body`. Also check that `[Unreleased]` is non-empty, so a release can't ship without notes.

---

## Cross-platform

The code has 93 `target_os = "macos"`, 20 `target_os = "linux"`, 55 Windows and 75 `cfg(unix)` gates. The Windows binary builds since 0.5.8, but nothing tests it (C1) and no tests run there.

Every `std::os::unix` and `libc::` call outside tests is behind a gate. No ungated Unix call was found by reading the code, but no cross-compile was run.

**Feature matrix.** (a) means a working fallback, (b) means a visible explanation, (c) means silent or broken.

| Feature | macOS | Linux | Windows | Evidence |
|---|---|---|---|---|
| Built-in browser (Chromium over CDP) | Chromium in /Applications | (a) xdg-settings, /usr/bin, /opt, /snap | (b) "needs macOS or Linux for now" | `browser_engine.rs:166-185, 545-548` |
| Design canvas | WebKit | (a) headless Chromium | (b) same message | `designdraw/render.rs:127-175` |
| Database passwords | Keychain | (c) plaintext `~/.runo/data/secrets.json`, 0600 | (c) plaintext, **inherited ACL** (mode bits are `cfg(unix)`) | `db/saved.rs:48-76, 325-387` |
| Menu bar item | AppKit status item | hidden, no explanation | hidden | `menubar.rs:405-433`, `settings_ui.rs:808` |
| Dock badge | NSDockTile | (c) no-op | (c) no-op | `dock.rs:54-57` |
| Process list (dev-server port detection, cleanup) | libproc | (a) /proc | (c) always empty | `procs.rs:98-101` |
| Keep awake during turns | caffeinate | (a) systemd-inhibit | (c) no-op, setting still shown | `turns.rs:1577-1594`, `settings_ui.rs:819` |
| Login-shell PATH import | `$SHELL -l -i` | (a) | inherits PATH only | `shellenv.rs:74` |
| Playbooks while closed | launchd | (a) systemd user timer | (a) schtasks | `pbrunner/job.rs:73-90, 356-414` |
| Notifications | native / osascript | (a) native / notify-send | (a) native | `desktop_notify.rs:55-61` |
| Remote-session checkpoint and restore | sh | sh | (c) `Command::new("sh")` ungated; fails without Git Bash on PATH | `connections.rs:1363` (used at 1399, 1494, 1541, 1558, 2098) |
| Cove on this machine | `curl \| sh`, launchctl | `curl \| sh` | (c) raw spawn error | `cove.rs:165-170, 250-264` |
| Shell for quick and setup commands, terminals | sh | sh | (a) `cmd /C`, PowerShell for tabs | `terminal.rs:90-97`, `pbrunner.rs:331-339`, `pty.rs:124, 220` |

**Paths.** Most code uses `HOME` then `USERPROFILE` (for example `store.rs:1108`). These sites read `HOME` only, so on Windows `~/` expansion and default folders silently fail:
- `pbfile.rs:323` and `coveserver.rs:51` (`~/` expansion)
- `dataquery.rs:590` and `pbshare.rs:382` (save dialogs open in `%TEMP%`)
- `lightbox.rs:473-476`
- `fileview.rs:560`

There are also hard-coded `/` prefixes: `main.rs:3959` and `cliview.rs:420` build `format!("{}/", dir.trim_end_matches('/'))` to match transcript working directories, which never matches `C:\…`. Earlier-turn and transcript lookups can come back empty on Windows.

### X2. Database passwords in plaintext off macOS
- **Severity:** High. **Effort:** M.
- **Evidence:** `db/saved.rs:325-387` uses the Keychain only under `cfg(target_os = "macos")`. Elsewhere passwords go to `secrets.json`. On Windows, the 0600/0700 modes (`db/saved.rs:54, 73`) don't apply, so the file inherits the parent's ACL.
- **Impact:** Production database credentials sit readable to anything running as the user (including the agents Runo itself starts) and get swept into backups and dotfile syncs.
- **Fix:** Use the `keyring` crate (Secret Service on Linux, Credential Manager on Windows, Keychain on macOS) with the plaintext file as a fallback only when no secret service exists, and say so in the Data place's UI. Set an owner-only DACL on Windows if the file fallback stays.
- **Related, by design:** agents run with no OS sandbox on any platform. `seatbelt.rs` scans commits and pushes for secrets; it does not sandbox processes. Isolation is opt-in through Cove or a remote connection. That's a reasonable choice, but `docs/guide/git-safety.md` and the README should say plainly that local agents run with the person's full rights.

### X1. Windows degrades silently
- **Severity:** High. **Effort:** M.
- **Evidence:** The (c) rows in the table above, plus the `HOME`-only and `/`-prefix sites.
- **Impact:** Windows users meet broken features with no explanation (remote checkpoints, an empty process list, keep-awake that does nothing) on a platform that CI doesn't build (C1) and the testing backlog hasn't covered.
- **Fix:**
  1. Add one `home_dir()` helper and replace the six `HOME`-only sites with it.
  2. Normalise paths with `Path::starts_with` instead of string prefixes.
  3. Gate or replace `local_sh` (run git directly, which these scripts mostly wrap).
  4. Hide the keep-awake setting off macOS and Linux, or use `SetThreadExecutionState`.
  5. Implement `procs::all` with `sysinfo` or Toolhelp32.
  6. Give the browser and design canvas a Windows Chromium path; the CDP engine is platform-neutral except for the launcher.
  7. Add a "Not on this platform" note for each hidden setting.

### X3. Packaging and updates on Linux and Windows
- **Severity:** Medium. **Effort:** M.
- **Evidence:**
  - Windows ships a bare `Runo.exe` in a zip (`release.yml:280-286`): unsigned (SmartScreen warns, `docs/releasing.md:11`), with no installer, Start-menu entry, uninstaller or file association.
  - Linux ships a `.deb` and a tarball built on `ubuntu-22.04` (glibc 2.35 floor, which is good). There is no AppImage, Flatpak or rpm.
  - The updater's Linux choice prefers `.appimage`, then `.deb` (`applog.rs:493`), so a tarball or Fedora user is offered a `.deb`.
  - Nothing installs in place on any platform.
- **Fix:**
  - Windows: an MSI or Inno Setup installer built in the release job, signed with an Authenticode certificate (Azure Trusted Signing is cheap). Then the updater can run the installer.
  - Linux: add an AppImage (one file, self-updating with zsync) and have the updater pick by how Runo was installed (`/usr/bin` means deb, anything else means AppImage or tarball).

---

## Dependencies

**Vendored crates** (`vendor/`, patched through `[patch.crates-io]` or path deps):

| Crate | Base | Patch | Upstream now | Re-sync doc |
|---|---|---|---|---|
| `gpui-pre-apple` | 0.3.8, zed@`279fe07` (`vendor/gpui-pre-apple/Cargo.toml:21-29`) | Two changes in `src/metal_renderer.rs` only: destination alpha `One` → `OneMinusSourceAlpha` (glass windows) and 8× MSAA for paths (commit `a865ba1`) | gpui-pre-apple **0.3.9** (2026-10-12); `gpui-kit` **0.8.0** (2026-10-19), Runo is on 0.7.1 | **None.** No `UPSTREAM.md` (the other three vendored crates have one); root `Cargo.toml:73` still says "one blending fix". It vendors about 19k lines of gpui sources for its `build.rs` |
| `block` | 0.1.6 | Opaque `#[repr(C)]` class to silence a future hard error | unmaintained; replaced by `block2` | `vendor/block/UPSTREAM.md` |
| `portable-pty` | 0.9.0 (wezterm `f8921727`) | Windows `spawn_command_in_job` (ConPTY job list) | 0.9.0 is still latest | `UPSTREAM.md` |
| `vt100` | 0.15.2 | Dim (SGR 2) and strikethrough | **0.18.1** (2026-10-19) | `UPSTREAM.md` |

### D1. gpui stack drift and the undocumented gpui patch
- **Severity:** Medium. **Effort:** M.
- **Evidence:** The table above. gpui-kit 0.7.1 → 0.8.0 is a minor bump on a 0.x crate, so expect breaking changes. The vendored apple crate pins `gpui = "=0.3.8"`, so moving to 0.3.9 needs a re-vendor.
- **Impact:** Each upstream rendering or input fix (IME, Wayland, Windows DirectComposition) arrives later and costs more the longer it waits. Whoever re-syncs must rediscover both patches from git history.
- **Fix:** Write `vendor/gpui-pre-apple/UPSTREAM.md` in the same shape as the other three: base rev, the two `// Runo:` changes, how to re-vendor (copy the crate from `~/.cargo/registry`, re-apply, diff), and what upstream change would let it be dropped. Try upstreaming the blend fix. Schedule a gpui-kit 0.8 bump as its own PR.

### D2. vt100 behind with a known bug
- **Severity:** Medium. **Effort:** M.
- **Evidence:** The root `Cargo.toml:31-35` turns off `overflow-checks` for vt100 in dev builds because it "subtracts with wrap-around when the view is scrolled back more than a screen". Release builds rely on wrap-around behaviour. Upstream is 0.18.1.
- **Fix:** Check whether 0.16 to 0.18 fix the scrollback underflow and add dim. If so, re-vendor only strikethrough; if not, fix the subtraction in the vendored copy (a `saturating_sub`) rather than relying on profile flags.

**Other dependency notes:**
- **Duplicates:** `windows-sys` ×5, `windows` ×4, `hashbrown` ×4, `syn` 2 and 3, `bitflags` 1 and 2 (the vendored portable-pty pulls in 1.x), two `cocoa` and two `objc2`. Almost all come through gpui and can't be fixed locally. A `cargo-deny` `bans` warning (C2) would track them.
- **Build time and size:** release profile `codegen-units = 1`, thin LTO, `panic = "abort"`, `strip = "symbols"` (root `Cargo.toml:16-21`). Shipped sizes for v0.5.9 are 15–23 MB per asset, which is reasonable. Release jobs take 14–25 minutes. `rusqlite` `bundled` compiles SQLite from C on every cold build, and `postgres` plus `tokio-postgres-rustls` bring in a tokio runtime for the Data place. The 14 tree-sitter grammars are the next-largest optional chunk. No Cargo features let a contributor build without them. A `minimal` feature set (no Data place, no design canvas, fewer grammars) would cut cold builds for contributors who don't touch those areas, though it isn't urgent.
- **Unmaintained transitive crates:** `instant`, `paste`, `objc 0.2`, `cocoa`, `block`. All come in through gpui; nothing to do beyond tracking them.

---

## Operability

**What exists:**
- Logging. `applog.rs` writes one log per launch to `~/.runo/logs/runo-<time>.log` and keeps 10 (`applog.rs:11`). Lines are `time LEVEL [area] message`, collapsed to one line, capped at 1,000 characters and secret-redacted (`applog.rs:126-292`, well tested). A `warn_once` dedupe stops retries from flooding the log.
- Crash reporting. A panic hook writes `crash-<time>.txt` with a backtrace and keeps 20 (`:370-385`). The next launch offers it, with a link to `issues/new` (`health.rs:861`).
- Settings → Logs and crash reports → Show reveals the folder (`settings_ui.rs:843`).
- No telemetry. That is a deliberate, privacy-friendly choice; `assistant_telemetry` is a UI label, not data collection.

### O1. No schema version; downgrade drops sessions
- **Severity:** High. **Effort:** M.
- **Evidence:**
  - `state.json` and the per-session files have no version field.
  - `grep 'serde(other)'` finds nothing. `Status` (`store.rs:15-23`) gained `Stopped` recently, and `Role` is a closed enum too.
  - When a session file fails to parse, `persist::read` moves it aside (`persist.rs:157`, `store::move_aside` at `store.rs:1072`), and the session disappears from the UI. When `state.json` fails to parse, the whole index is moved aside and Runo starts empty (`store.rs:1153-1156`).
  - Unknown fields written by a newer version are dropped silently on the next save, because serde ignores unknown fields by default and `skip_serializing_if` omits empties. A downgrade followed by an upgrade loses them.
  - Settings use `read_json_or_backup` plus `migrate()` (`settings.rs:369-391`), which has the same downgrade exposure.
- **Impact:** An updater with rollback (R3), or a person reinstalling an older build after a bad release, will drop every session saved in a newer state. The data sits in a `.json.corrupt-<time>` file, but no UI tells the person where.
- **Fix:**
  1. Add `#[serde(other)]` fallbacks (`Status::Unknown` → shown as Done/Failed) to every persisted enum.
  2. Add `"format": N` to `state.json` and the session files. If the format is newer than this build supports, open read-only with a banner ("saved by Runo x.y; update to edit") instead of moving files aside.
  3. Keep the last good `state.json` as `state.json.prev` on the first save after a version change.
  4. Show moved-aside files in the startup alerts.
  5. Add tests: "a session from a newer Runo still loads" and "an unknown status loads".

### O2. Native crashes and log size
- **Severity:** Medium. **Effort:** M.
- **Evidence:**
  - Release builds use `panic = "abort"` (root `Cargo.toml:20`). The hook still runs first, so panics are captured.
  - Native faults are not captured: SIGSEGV or SIGABRT in Metal, ObjC or WebKit, Wayland, or a C library like SQLite. The OS crash reporter catches them on macOS (`~/Library/Logs/DiagnosticReports`), but Runo doesn't look there. Nothing captures them on Linux or Windows.
  - Logs rotate by launch count, not size (`applog.rs:296-313`). A Runo left running for weeks writes one ever-growing file. `LINE_LIMIT` caps each line, not the total.
  - On Windows, `redirect_stderr` sends stderr only to the file (`applog.rs:351-364`), so `Runo.exe` started from a terminal shows nothing there.
- **Fix:** Rotate within a launch at about 20 MB (`runo-<time>.1.log`). On macOS, look for `Runo-*.ips` in `DiagnosticReports` newer than the last launch and offer it like a panic report. On Linux and Windows, install a minimal signal or SEH handler that writes a one-line marker (`crash-native-<time>.txt`) with the last 200 log lines.

### O3. No one-click diagnostics bundle
- **Severity:** Low. **Effort:** S.
- **Evidence:** The bug template (`.github/ISSUE_TEMPLATE/bug_report.yml`) asks for version and OS but not logs. The only diagnostics affordance is "Show" on the logs folder.
- **Fix:** Add a "Copy diagnostics" button. It would copy the Runo version, OS and arch, agent CLI versions (already known from `caps`), redacted settings, and the last 300 log lines (already redacted) to the clipboard. Add a "Logs" field to the bug template that points to it.

---

## Docs

**What exists:** `README.md` (456 lines, feature tour), 36 user guides in `docs/guide/`, `docs/providers.md`, `docs/phone-bridge.md`, `docs/releasing.md`, `docs/testing-backlog.md` (an honest list of what isn't verified, including other OSes), `CONTRIBUTING.md` (33 lines), `SECURITY.md` and `CODE_OF_CONDUCT.md`.

### Doc1. No architecture doc; CONTRIBUTING is stale
- **Severity:** Medium. **Effort:** S.
- **Evidence:** `CONTRIBUTING.md:13-23` lists 8 files out of 214 and only `vendor/portable-pty` among the 4 vendored crates. Nothing explains the main moving parts: `turns` / `harness` / `stream` (how a turn runs), `crew` / `prwatch` / `wake` (Crew and PR watching), `persist` / `store` (data on disk, `~/.runo` layout), `connections` / `cove` (remote runs), and `approvals` / `seatbelt` (safety). `main.rs` at 10k lines is the first file a newcomer opens.
- **Fix:** Add `docs/architecture.md`: a one-page map of modules, a turn's sequence diagram, threads and channels (`wake.rs`), the data layout on disk, and the vendored crates. Link it from CONTRIBUTING, and add "how to run one test", "how to record a fixture" and "how to test on Linux in Cove".

### Doc2. Platform claims disagree
- **Severity:** Low. **Effort:** S.
- **Evidence:** The README's download section mentions only Macs (`README.md:305`), and its requirements call macOS "primary" (`:309`) while Linux and Windows assets ship. The bug template says "Runo supports macOS, Linux, and Windows" (`bug_report.yml:8`). Only 3 of 36 guides mention Linux or Windows. CONTRIBUTING and the PR template run `clippy --workspace`, but CI runs `-p runo`, which is the same today but will drift.
- **Fix:** Add a "Platform support" table to the README (per feature: macOS / Linux / Windows, from the X1 table), and link it from the guides that are Mac-only.
