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

/// How much a log line matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// Longest message kept on one line; the rest is cut.
const LINE_LIMIT: usize = 1000;

/// Something worth knowing later, such as a turn starting or ending.
pub fn info(area: &str, message: impl AsRef<str>) {
    write(Level::Info, area, message.as_ref());
}

/// Something went wrong that the person can work around, such as a failed git command.
pub fn warn(area: &str, message: impl AsRef<str>) {
    write(Level::Warn, area, message.as_ref());
}

/// Something went wrong that stops a piece of Runo working, such as a save that failed.
pub fn error(area: &str, message: impl AsRef<str>) {
    write(Level::Error, area, message.as_ref());
}

/// Like `warn`, but a message repeated by a background retry is written once per launch.
pub fn warn_once(area: &str, message: impl AsRef<str>) {
    once(Level::Warn, area, message.as_ref());
}

/// Like `error`, but a message repeated by a background retry is written once per launch.
pub fn error_once(area: &str, message: impl AsRef<str>) {
    once(Level::Error, area, message.as_ref());
}

fn once(level: Level, area: &str, message: &str) {
    static SEEN: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (area, message).hash(&mut hasher);
    let key = hasher.finish();
    let fresh = SEEN.lock().map_or(true, |mut seen| {
        if seen.contains(&key) {
            false
        } else {
            // Bounded, so a flood of distinct messages can't grow it forever.
            if seen.len() >= 512 {
                seen.clear();
            }
            seen.push(key);
            true
        }
    });
    if fresh {
        write(level, area, message);
    }
}

/// Write one line to the log (stderr, which `init` sends to the log file). Writing never
/// panics: a log that can't be written is skipped.
fn write(level: Level, area: &str, message: &str) {
    let time = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string();
    let line = format_line(&time, level, area, message);
    let _ = std::io::stderr().lock().write_all(line.as_bytes());
}

/// `<time> <LEVEL> [area] message`, on one line with secrets masked.
pub fn format_line(time: &str, level: Level, area: &str, message: &str) -> String {
    let mut text = String::with_capacity(message.len().min(LINE_LIMIT) + 48);
    for (i, line) in message
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
    {
        if i > 0 {
            text.push_str(" | ");
        }
        text.push_str(line);
    }
    let mut text = redact(&text);
    if text.chars().count() > LINE_LIMIT {
        text = text.chars().take(LINE_LIMIT).collect::<String>() + "…";
    }
    format!("{time} {:<5} [{area}] {text}\n", level.tag())
}

/// The line of a command's output that says what went wrong: the first that reads like an
/// error, else the first that isn't blank. Cut to a sensible length.
pub fn gist(output: &str) -> String {
    const WORDS: [&str; 6] = ["error", "fatal", "denied", "failed", "not found", "cannot"];
    let lines = || output.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines()
        .find(|l| {
            let lower = l.to_lowercase();
            WORDS.iter().any(|w| lower.contains(w))
        })
        .or_else(|| lines().next())
        .unwrap_or("");
    if line.chars().count() > 300 {
        line.chars().take(300).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}

/// Prefixes of well-known API keys and tokens.
const TOKEN_PREFIXES: [&str; 14] = [
    "github_pat_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "glpat-",
    "sk-",
    "xoxb-",
    "xoxp-",
    "xapp-",
    "AKIA",
    "AIza",
    "npm_",
];

/// Names whose value is a secret, in `name=value` or `name: value`.
const SECRET_NAMES: [&str; 9] = [
    "token",
    "secret",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "api-key",
    "authorization",
    "credential",
];

const MASK: &str = "***";

fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|s| name.contains(s))
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+' | '=')
}

/// Mask tokens, API keys, passwords in URLs and `secret=value` pairs, so text can be
/// logged. Errors from git, gh and agent CLIs sometimes echo them.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // The next word is a secret: after `Bearer`, or after `Authorization:` and the like.
    let mut mask_next = false;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end_matches(char::is_whitespace);
        let space = &piece[word.len()..];
        if word.is_empty() {
            out.push_str(space);
            continue;
        }
        let lower = word.to_ascii_lowercase();
        // An auth scheme: the credential follows it.
        let scheme =
            matches!(lower.as_str(), "bearer" | "basic") || (mask_next && lower == "token");
        if scheme {
            out.push_str(word);
            mask_next = true;
        } else if mask_next {
            out.push_str(MASK);
            mask_next = false;
        } else if (word.ends_with(':') || word.ends_with('='))
            && secret_name(word.trim_end_matches([':', '=']))
        {
            out.push_str(word);
            mask_next = true;
        } else {
            out.push_str(&redact_word(word));
        }
        out.push_str(space);
    }
    out
}

/// One word without spaces: `name=secret`, `"name":"secret"`, a URL with a password, or a
/// known token.
fn redact_word(word: &str) -> String {
    // name=value or "name": "value" in one word.
    for sep in ['=', ':'] {
        if let Some((name, value)) = word.split_once(sep) {
            let value_body = value.trim_matches(|c: char| c == '"' || c == '\'');
            if !value_body.is_empty()
                && !value_body.starts_with("//")
                && secret_name(name.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
            {
                // Keep the quotes and brackets around the value.
                let punct = |c: char| matches!(c, '"' | '\'' | '}' | ']' | ')' | ',' | ';');
                let open = &value[..value.len() - value.trim_start_matches(punct).len()];
                let rest = &value[open.len()..];
                let close = &rest[rest.trim_end_matches(punct).len()..];
                return format!("{name}{sep}{open}{MASK}{close}");
            }
        }
    }
    let mut word = word.to_string();
    // scheme://user:password@host
    if let Some(start) = word.find("://").map(|i| i + 3) {
        let rest = &word[start..];
        let host_end = rest.find('/').unwrap_or(rest.len());
        if let Some(at) = rest[..host_end].rfind('@') {
            word = format!("{}{MASK}{}", &word[..start], &rest[at..]);
        }
    }
    for prefix in TOKEN_PREFIXES {
        let mut from = 0;
        while let Some(found) = word[from..].find(prefix).map(|i| i + from) {
            let boundary = word[..found]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric());
            let body_start = found + prefix.len();
            let body_len = word[body_start..]
                .find(|c: char| !token_char(c))
                .unwrap_or(word.len() - body_start);
            if boundary && body_len >= 12 {
                word.replace_range(body_start..body_start + body_len, MASK);
                from = body_start + MASK.len();
            } else {
                from = body_start;
            }
        }
    }
    word
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
    fetch_release().ok().flatten()
}

/// Ask GitHub for the latest release: Some when it is newer than this Runo, None when
/// this Runo is the latest, or why the check failed, in words for the person.
pub fn fetch_release() -> Result<Option<Release>, String> {
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
        .output();
    let out = match out {
        Ok(out) => out,
        Err(e) => {
            warn(
                "app-update",
                format!("could not run curl to check for updates: {e}"),
            );
            return Err(format!("Runo couldn't run curl to check: {e}"));
        }
    };
    let current = env!("CARGO_PKG_VERSION");
    let body = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        warn(
            "app-update",
            format!(
                "the update check failed: curl exited with {}",
                out.status
                    .code()
                    .map_or("no code".to_string(), |c| format!("code {c}"))
            ),
        );
        return Err("Couldn't reach GitHub. Check your connection, then try again.".into());
    }
    let found = newer_release(&body, current);
    let value = serde_json::from_str::<serde_json::Value>(&body).ok();
    match &found {
        Some(release) => info(
            "app-update",
            format!("Runo {} is out (running {current})", release.version),
        ),
        None if value.as_ref().is_some_and(|v| v["tag_name"].is_string()) => {
            info("app-update", format!("checked: {current} is the latest"))
        }
        // GitHub answers 404 until the first release is published.
        None if value.as_ref().is_some_and(|v| v["message"] == "Not Found") => {
            info("app-update", "checked: no release published yet")
        }
        None => {
            warn(
                "app-update",
                format!(
                    "the update check got no release from GitHub: {}",
                    gist(&body)
                ),
            );
            // GitHub explains a refusal, such as its hourly limit, in `message`.
            return Err(match value.as_ref().and_then(|v| v["message"].as_str()) {
                Some(message) => format!("GitHub said: {message}"),
                None => "GitHub's answer didn't name a release. Try again later.".into(),
            });
        }
    }
    Ok(found)
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
        .map_err(|e| {
            error(
                "app-update",
                format!("could not run curl to download {url}: {e}"),
            );
            format!("Could not run curl: {e}")
        })?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&partial);
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        warn(
            "app-update",
            format!(
                "downloading {url} failed: curl exited with {}: {}",
                out.status
                    .code()
                    .map_or("no code".to_string(), |c| format!("code {c}")),
                gist(&why)
            ),
        );
        return Err(if why.is_empty() {
            "The download failed.".into()
        } else {
            format!("The download failed: {why}")
        });
    }
    std::fs::rename(&partial, &path).map_err(|e| {
        error(
            "app-update",
            format!("could not save {}: {e}", path.display()),
        );
        e.to_string()
    })?;
    info("app-update", format!("downloaded {}", path.display()));
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
    fn log_lines_carry_time_level_and_area_on_one_line() {
        let line = format_line(
            "2026-10-14 09:30:00.123",
            Level::Warn,
            "git",
            "Push failed\n  remote: rejected\n\n",
        );
        assert_eq!(
            line,
            "2026-10-14 09:30:00.123 WARN  [git] Push failed | remote: rejected\n"
        );
        assert!(format_line("t", Level::Error, "save", "x").starts_with("t ERROR [save] x"));
        assert!(format_line("t", Level::Info, "turn", "x").starts_with("t INFO  [turn] x"));
        let long = format_line("t", Level::Info, "a", &"y".repeat(5000));
        assert!(long.chars().count() < LINE_LIMIT + 40);
        assert!(long.ends_with("…\n"));
        // Writing never panics, whatever is in the message.
        warn("test", "a line from the tests");
        warn_once("test", "only once");
        warn_once("test", "only once");
    }

    #[test]
    fn secrets_are_masked() {
        let cases = [
            (
                "remote: https://x-access-token:ghp_abcdefghijklmnop1234@github.com/o/r.git",
                "remote: https://***@github.com/o/r.git",
            ),
            ("using ghp_abcdefghijklmnop1234 now", "using ghp_*** now"),
            (
                "key github_pat_11ABCDEFG0123456789_abcdefghijk",
                "key github_pat_***",
            ),
            (
                "ANTHROPIC_API_KEY=sk-ant-api03-abcdefghijklmnop",
                "ANTHROPIC_API_KEY=***",
            ),
            (
                "Invalid key sk-proj-abcdefghijklmnopqrstu",
                "Invalid key sk-***",
            ),
            (
                "Authorization: Bearer abc.def.ghi",
                "Authorization: Bearer ***",
            ),
            ("authorization: token abcdef123", "authorization: token ***"),
            ("--token=s3cret --verbose", "--token=*** --verbose"),
            (r#"{"api_key":"abc123"}"#, r#"{"api_key":"***"}"#),
            ("password: hunter2", "password: ***"),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input), want, "{input}");
        }
        // Ordinary errors pass through untouched.
        for plain in [
            "fatal: not a git repository (or any of the parent directories): .git",
            "error: failed to push some refs to 'https://github.com/o/r.git'",
            "the task-list is skipped",
            "could not start \"claude\": No such file or directory (os error 2)",
        ] {
            assert_eq!(redact(plain), plain);
        }
        // And the log line masks too.
        let line = format_line(
            "t",
            Level::Warn,
            "gh",
            "token=abcdef ghp_abcdefghijklmnop1234",
        );
        assert!(
            !line.contains("abcdef ") && !line.contains("1234"),
            "{line}"
        );
    }

    #[test]
    fn the_gist_is_the_line_that_says_what_failed() {
        assert_eq!(
            gist("Cloning into 'x'...\nfatal: repository not found\nmore"),
            "fatal: repository not found"
        );
        assert_eq!(gist("\n  just output\nsecond"), "just output");
        assert_eq!(gist(""), "");
        assert!(gist(&"e".repeat(500)).chars().count() <= 301);
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
        // Releases bump the last number, so it is compared as a number, not as text.
        let patch = json.replace("v0.3.0", "v0.5.10");
        assert_eq!(newer_release(&patch, "0.5.9").unwrap().version, "0.5.10");
        assert!(newer_release(&patch, "0.5.10").is_none());
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
