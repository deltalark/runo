# Releasing Runo

A release is a `v*` tag. Pushing one runs `.github/workflows/release.yml`, which builds `Runo.app` for Apple silicon and Intel Macs (a zip and a disk image of each), Runo for x86_64 and arm64 Linux (a `.deb` and a tarball of each) and `Runo.exe` for 64-bit Windows (a zip), and attaches them all to a GitHub release. Runo's update check reads `releases/latest` and offers the download for the user's platform.

## Test builds

To build the app without releasing it, open the repository's Actions tab, pick **Release**, and click **Run workflow** on any branch. The run builds the same files for every platform, named with the commit (for example `Runo-0.1.0-macos-arm64-a2831ac.dmg`), and keeps them as downloadable artifacts on the run's page instead of creating a release. Pull requests that change the workflow run it the same way.

## Signing and notarization

Only the Mac builds are signed. The Windows build is unsigned, so SmartScreen warns before running it the first time (More info → Run anyway).


Without these repository secrets, the build is signed ad hoc and macOS asks before opening it the first time (right-click → Open). With them, the app and disk image are signed with your Developer ID and notarized.

| Secret | What it is |
| --- | --- |
| `APPLE_CERT_P12` | Your "Developer ID Application" certificate and key, exported from Keychain Access as a .p12 and base64-encoded (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERT_PASSWORD` | The password you set when exporting the .p12 |
| `APPLE_ID` | The Apple ID email of your developer account |
| `APPLE_TEAM_ID` | Your 10-character team ID (developer.apple.com → Membership) |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple ID (appleid.apple.com → Sign-In and Security) |

Add them in the repository's Settings → Secrets and variables → Actions.

## Version numbers

Every release bumps the last number: 0.5.0, then 0.5.1, 0.5.2 and so on. Only a major release bumps the middle number and starts the last one again (0.5.4, then 0.6.0). Runo compares versions number by number, so 0.5.10 is newer than 0.5.9.

## Cutting a release

1. Set `version` in the root `Cargo.toml` (for example `0.5.1`).
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [0.5.1] - YYYY-MM-DD` and start a new empty `## [Unreleased]` above it.
3. Merge that to `main`, then tag it: `git tag v0.5.1 && git push origin v0.5.1`.

The workflow stops if the tag doesn't match the version in `Cargo.toml`. A running Runo on an older version shows the update card on its next start, or within six hours while it runs (Settings can turn the check off). Settings > General > Updates also has Check now, and the Runo menu Check for Updates…, for checking by hand.
