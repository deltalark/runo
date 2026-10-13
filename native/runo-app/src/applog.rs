//! Runo's own log file, crash reports, and the check for a newer Runo release.
//!
//! Each launch writes what used to go to stderr into `~/.runo/logs/runo-<time>.log`,
//! keeping the latest few. A panic writes `crash-<time>.txt` there too, which the next
//! launch offers to show.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Launch logs kept; older ones are deleted.
const LOGS_KEPT: usize = 10;
/// Crash reports kept.
const CRASHES_KEPT: usize = 20;
/// Where Runo's releases are published.
pub const REPO: &str = "deltalark/runo";

pub fn logs_dir() -> PathBuf {
    crate::store::data_dir().join("logs")
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Delete all but the newest `keep` files in `dir` whose names start with `prefix`.
fn prune(dir: &Path, prefix: &str, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
        })
        .collect();
    // Names carry the time, so they sort oldest first.
    files.sort();
    let extra = files.len().saturating_sub(keep);
    for path in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
}

/// Start this launch's log file and send stderr to it (and still to the terminal, when
/// Runo was started from one), then install the crash reporter.
pub fn init() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    prune(&dir, "runo-", LOGS_KEPT - 1);
    let path = dir.join(format!("runo-{}.log", stamp()));
    if let Ok(mut file) = std::fs::File::create(&path) {
        let _ = writeln!(
            file,
            "Runo {} on {} {}, started {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        redirect_stderr(file);
    }
    install_panic_hook();
}

#[cfg(unix)]
fn redirect_stderr(file: std::fs::File) {
    use std::os::fd::{FromRawFd, IntoRawFd};
    // SAFETY: plain descriptor calls on stderr and on descriptors owned here.
    unsafe {
        if libc::isatty(2) == 0 {
            libc::dup2(file.into_raw_fd(), 2);
            return;
        }
        // A terminal is attached: copy everything to both through a pipe.
        let mut fds = [0; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        let terminal = libc::dup(2);
        if terminal < 0 || libc::dup2(fds[1], 2) < 0 {
            return;
        }
        libc::close(fds[1]);
        let mut pipe = std::fs::File::from_raw_fd(fds[0]);
        let mut terminal = std::fs::File::from_raw_fd(terminal);
        let mut file = file;
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let _ = file.write_all(&buf[..n]);
                let _ = terminal.write_all(&buf[..n]);
            }
        });
    }
}

#[cfg(windows)]
fn redirect_stderr(file: std::fs::File) {
    use std::os::windows::io::IntoRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
    }
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    // SAFETY: hands the process a handle it keeps for its lifetime; Rust's stderr looks
    // the standard handle up on every write.
    unsafe {
        SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle() as _);
    }
}

#[cfg(not(any(unix, windows)))]
fn redirect_stderr(_: std::fs::File) {}

/// Write a crash report for every panic, then let the usual message print to the log.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let report = crash_report(
            thread.name().unwrap_or("unnamed"),
            &info.to_string(),
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        let dir = logs_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("crash-{}.txt", stamp())), report);
        prune(&dir, "crash-", CRASHES_KEPT);
        previous(info);
    }));
}

fn crash_report(thread: &str, message: &str, backtrace: &str) -> String {
    format!(
        "Runo {} on {} {}\nTime: {}\nThread: {thread}\n\n{message}\n\nBacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
    )
}

/// A crash report left by an earlier launch.
#[derive(Clone, Debug, PartialEq)]
pub struct Crash {
    pub path: PathBuf,
    /// The panic message, first line.
    pub summary: String,
    /// The main thread panicked, so Runo closed.
    pub quit: bool,
}

/// The newest crash report not yet dismissed, if any.
pub fn unseen_crash(dir: &Path) -> Option<Crash> {
    let seen = std::fs::read_to_string(dir.join("crash-seen")).unwrap_or_default();
    let newest = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        .max()?;
    if newest.as_str() <= seen.trim() {
        return None;
    }
    let path = dir.join(&newest);
    let text = std::fs::read_to_string(&path).ok()?;
    let (head, body) = text.split_once("\n\n").unwrap_or((&text, ""));
    let summary = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string();
    Some(Crash {
        quit: head.contains("Thread: main"),
        summary,
        path,
    })
}

/// Don't offer this report (or older ones) again.
pub fn dismiss_crash(crash: &Crash) {
    if let (Some(dir), Some(name)) = (crash.path.parent(), crash.path.file_name()) {
        let _ = std::fs::write(dir.join("crash-seen"), name.to_string_lossy().as_bytes());
    }
}

/// A newer Runo on GitHub.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    /// The release page.
    pub page: String,
    /// The download for this platform: (file name, link), when the release has one.
    pub asset: Option<(String, String)>,
}

/// The release in a GitHub `releases/latest` reply, if it is newer than `current`.
pub fn newer_release(json: &str, current: &str) -> Option<Release> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if value["draft"].as_bool() == Some(true) || value["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let version = value["tag_name"]
        .as_str()?
        .trim_start_matches('v')
        .to_string();
    if !crate::alerts::is_newer(&version, current) {
        return None;
    }
    let assets: Vec<(String, String)> = value["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some((
                        a["name"].as_str()?.to_string(),
                        a["browser_download_url"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Release {
        version,
        page: value["html_url"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/latest")),
        asset: pick_asset(&assets, std::env::consts::OS, std::env::consts::ARCH),
    })
}

/// The release file for this platform: a disk image or zip on macOS (matching the
/// processor when there are several), an installer on Windows, a package on Linux.
pub fn pick_asset(assets: &[(String, String)], os: &str, arch: &str) -> Option<(String, String)> {
    let (kinds, words): (&[&str], &[&str]) = match os {
        "macos" => (&[".dmg", ".zip"], &["mac", "darwin", "osx", "apple"]),
        "windows" => (&[".msi", ".exe", ".zip"], &["win"]),
        _ => (&[".appimage", ".deb", ".tar.gz"], &["linux"]),
    };
    let arch_words: &[&str] = match arch {
        "aarch64" => &["aarch64", "arm64", "universal"],
        _ => &["x86_64", "x64", "amd64", "intel", "universal"],
    };
    let other_arch: &[&str] = match arch {
        "aarch64" => &["x86_64", "x64", "amd64", "intel"],
        _ => &["aarch64", "arm64"],
    };
    let mut best: Option<(usize, &(String, String))> = None;
    for asset in assets {
        let name = asset.0.to_lowercase();
        let Some(kind) = kinds.iter().position(|k| name.ends_with(k)) else {
            continue;
        };
        // A disk image, installer or package only fits one system; an archive has to
        // name it.
        let archive = name.ends_with(".zip") || name.ends_with(".tar.gz");
        if archive && !words.iter().any(|w| name.contains(w)) {
            continue;
        }
        if other_arch.iter().any(|w| name.contains(w))
            && !arch_words.iter().any(|w| name.contains(w))
        {
            continue;
        }
        let score = kind * 2 + usize::from(!arch_words.iter().any(|w| name.contains(w)));
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, asset));
        }
    }
    best.map(|(_, a)| a.clone())
}

/// Ask GitHub for the latest release; None when it isn't newer or can't be reached.
pub fn check_release() -> Option<Release> {
    let out = std::process::Command::new("curl")
        .args([
            "-sL",
            "-m",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: Runo",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    newer_release(
        &String::from_utf8_lossy(&out.stdout),
        env!("CARGO_PKG_VERSION"),
    )
}

/// The Downloads folder, or the home folder without one.
pub fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let downloads = home.join("Downloads");
    if downloads.is_dir() {
        downloads
    } else {
        home
    }
}

/// Download a release file into `dir`. Nothing is installed: the person opens it.
pub fn download(name: &str, url: &str, dir: &Path) -> Result<PathBuf, String> {
    let name = Path::new(name)
        .file_name()
        .ok_or("The release file has no name.")?;
    let path = dir.join(name);
    let partial = dir.join(format!("{}.part", name.to_string_lossy()));
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "-m", "900", "-o"])
        .arg(&partial)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not run curl: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&partial);
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if why.is_empty() {
            "The download failed.".into()
        } else {
            format!("The download failed: {why}")
        });
    }
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-applog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn crash_reports_are_offered_once() {
        let dir = temp("crash");
        assert!(unseen_crash(&dir).is_none());
        std::fs::write(
            dir.join("crash-20261001-100000.txt"),
            crash_report("main", "panicked at src/main.rs:1:1:\nboom", "trace"),
        )
        .unwrap();
        std::fs::write(
            dir.join("crash-20261002-100000.txt"),
            crash_report("runo-save", "panicked at src/persist.rs:9:9:\nbad", "trace"),
        )
        .unwrap();
        let crash = unseen_crash(&dir).unwrap();
        assert!(crash.path.ends_with("crash-20261002-100000.txt"));
        assert_eq!(crash.summary, "panicked at src/persist.rs:9:9:");
        assert!(!crash.quit);
        dismiss_crash(&crash);
        assert!(unseen_crash(&dir).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_logs_are_pruned() {
        let dir = temp("prune");
        for n in 0..5 {
            std::fs::write(dir.join(format!("runo-2026100{n}.log")), "").unwrap();
        }
        std::fs::write(dir.join("crash-1.txt"), "").unwrap();
        prune(&dir, "runo-", 2);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["crash-1.txt", "runo-20261003.log", "runo-20261004.log"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn newer_releases_are_found_with_their_download() {
        let json = r#"{"tag_name":"v0.3.0","html_url":"https://github.com/deltalark/runo/releases/tag/v0.3.0",
            "draft":false,"prerelease":false,"assets":[
            {"name":"Runo-0.3.0-macos-arm64.dmg","browser_download_url":"https://x/arm.dmg"},
            {"name":"Runo-0.3.0-macos-x86_64.dmg","browser_download_url":"https://x/intel.dmg"},
            {"name":"Runo-0.3.0-macos-arm64.zip","browser_download_url":"https://x/arm.zip"},
            {"name":"runo_0.3.0_amd64.deb","browser_download_url":"https://x/runo.deb"}]}"#;
        let release = newer_release(json, "0.2.9").unwrap();
        assert_eq!(release.version, "0.3.0");
        assert!(release.page.ends_with("v0.3.0"));
        assert!(newer_release(json, "0.3.0").is_none());
        assert!(newer_release("not json", "0.1.0").is_none());
        let pre = json.replace("\"prerelease\":false", "\"prerelease\":true");
        assert!(newer_release(&pre, "0.1.0").is_none());
        let assets: Vec<(String, String)> = serde_json::from_str::<serde_json::Value>(json)
            .unwrap()["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    a["name"].as_str().unwrap().to_string(),
                    a["browser_download_url"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let pick = |os, arch| pick_asset(&assets, os, arch).map(|a| a.1);
        assert_eq!(
            pick("macos", "aarch64").as_deref(),
            Some("https://x/arm.dmg")
        );
        assert_eq!(
            pick("macos", "x86_64").as_deref(),
            Some("https://x/intel.dmg")
        );
        assert_eq!(
            pick("linux", "x86_64").as_deref(),
            Some("https://x/runo.deb")
        );
        assert_eq!(pick("windows", "x86_64"), None);
        // A plain Runo.dmg is the macOS build.
        let plain = vec![("Runo.dmg".to_string(), "https://x/Runo.dmg".to_string())];
        assert!(pick_asset(&plain, "macos", "aarch64").is_some());
    }
}
