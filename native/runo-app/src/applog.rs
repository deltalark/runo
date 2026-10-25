//! Runo's own log file, crash reports, and the check for a newer Runo release.
//!
//! Each launch writes what used to go to stderr into `~/.runo/logs/runo-<time>.log`,
//! keeping the latest few. A log that grows past `LOG_LIMIT` (a Runo left open for weeks)
//! moves to `runo-<time>.log.1` and a fresh file starts, keeping `PARTS_KEPT` pieces. A
//! panic writes `crash-<time>.txt` there too, which the next launch offers to show.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Launch logs kept; older ones are deleted.
const LOGS_KEPT: usize = 10;
/// Size a launch's log may reach before it moves aside and a fresh one starts.
const LOG_LIMIT: u64 = 10 << 20;
/// Pieces of one launch's log kept, the current one included: at most about 30 MB.
const PARTS_KEPT: usize = 3;
/// How often the log's size is looked at.
const SIZE_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(30);
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

/// This launch's log file, once `init` made it.
static CURRENT_LOG: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Start this launch's log file and send stderr to it (and still to the terminal, when
/// Runo was started from one), then install the crash reporter.
pub fn init() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    prune_launches(&dir, LOGS_KEPT - 1);
    let path = dir.join(format!("runo-{}.log", stamp()));
    if let Ok(mut file) = std::fs::File::create(&path) {
        let _ = START.set(now());
        let _ = writeln!(
            file,
            "{}, started {}",
            about(),
            START.get().map_or("", |s| s.as_str())
        );
        let sink = redirect_stderr(file);
        let _ = CURRENT_LOG.set(path.clone());
        watch_size(path, sink);
    }
    install_panic_hook();
}

/// "Runo 0.5.9 on macos aarch64".
fn about() -> String {
    format!(
        "Runo {} on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Delete all but the newest `keep` launch logs in `dir`, with their older pieces.
fn prune_launches(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("runo-"))
        .collect();
    let mut launches: Vec<&str> = names
        .iter()
        .map(|n| n.split_once(".log").map_or(n.as_str(), |(base, _)| base))
        .collect();
    // Names carry the time, so they sort oldest first.
    launches.sort_unstable();
    launches.dedup();
    let extra = launches.len().saturating_sub(keep);
    for old in &launches[..extra] {
        for name in names
            .iter()
            .filter(|n| n.split_once(".log").is_some_and(|(b, _)| b == *old))
        {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
}

/// Move `path` to `<path>.1`, `.1` to `.2` and so on, so that with the fresh file about
/// to start at `path` there are `parts` pieces; the oldest goes.
fn shift_parts(path: &Path, parts: usize) {
    let piece = |n: usize| PathBuf::from(format!("{}.{n}", path.display()));
    let _ = std::fs::remove_file(piece(parts.saturating_sub(1)));
    for n in (1..parts.saturating_sub(1)).rev() {
        let _ = std::fs::rename(piece(n), piece(n + 1));
    }
    if parts > 1 {
        let _ = std::fs::rename(path, piece(1));
    }
}

/// Where stderr's lines end up, so a fresh file can take over when the log is full.
/// Windows only ever points stderr at the file.
#[cfg_attr(windows, allow(dead_code))]
enum Sink {
    /// Stderr is the file itself.
    Stderr,
    /// A thread copies stderr to the terminal and to this file.
    Shared(std::sync::Arc<std::sync::Mutex<std::fs::File>>),
    None,
}

/// Every little while, start a fresh file when the log has grown past `LOG_LIMIT`.
fn watch_size(path: PathBuf, sink: Sink) {
    if matches!(sink, Sink::None) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("runo-log-size".into())
        .spawn(move || loop {
            std::thread::sleep(SIZE_CHECK_EVERY);
            if std::fs::metadata(&path).map_or(0, |m| m.len()) < LOG_LIMIT {
                continue;
            }
            shift_parts(&path, PARTS_KEPT);
            let Ok(mut file) = std::fs::File::create(&path) else {
                continue;
            };
            let _ = writeln!(
                file,
                "{}, started {}, log continued {}",
                about(),
                START.get().map_or("", |s| s.as_str()),
                now()
            );
            match &sink {
                Sink::Stderr => point_stderr_at(file),
                Sink::Shared(shared) => {
                    if let Ok(mut current) = shared.lock() {
                        *current = file;
                    }
                }
                Sink::None => {}
            }
        });
}

/// When this launch started, for the first line of each fresh log piece.
static START: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[cfg(unix)]
fn point_stderr_at(file: std::fs::File) {
    use std::os::fd::IntoRawFd;
    // SAFETY: replaces descriptor 2 with one owned here, then closes the spare.
    unsafe {
        let fd = file.into_raw_fd();
        libc::dup2(fd, 2);
        libc::close(fd);
    }
}

#[cfg(unix)]
fn redirect_stderr(file: std::fs::File) -> Sink {
    use std::os::fd::FromRawFd;
    // SAFETY: plain descriptor calls on stderr and on descriptors owned here.
    unsafe {
        if libc::isatty(2) == 0 {
            point_stderr_at(file);
            return Sink::Stderr;
        }
        // A terminal is attached: copy everything to both through a pipe.
        let mut fds = [0; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Sink::None;
        }
        let terminal = libc::dup(2);
        if terminal < 0 || libc::dup2(fds[1], 2) < 0 {
            return Sink::None;
        }
        libc::close(fds[1]);
        let mut pipe = std::fs::File::from_raw_fd(fds[0]);
        let mut terminal = std::fs::File::from_raw_fd(terminal);
        let shared = std::sync::Arc::new(std::sync::Mutex::new(file));
        let file = shared.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut file) = file.lock() {
                    let _ = file.write_all(&buf[..n]);
                }
                let _ = terminal.write_all(&buf[..n]);
            }
        });
        Sink::Shared(shared)
    }
}

#[cfg(windows)]
fn point_stderr_at(file: std::fs::File) {
    use std::os::windows::io::IntoRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
    }
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    // SAFETY: hands the process a handle it keeps for its lifetime; Rust's stderr looks
    // the standard handle up on every write. The handle it replaces is left open, as a
    // write may be using it.
    unsafe {
        SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle() as _);
    }
}

#[cfg(windows)]
fn redirect_stderr(file: std::fs::File) -> Sink {
    point_stderr_at(file);
    Sink::Stderr
}

#[cfg(not(any(unix, windows)))]
fn point_stderr_at(_: std::fs::File) {}

#[cfg(not(any(unix, windows)))]
fn redirect_stderr(_: std::fs::File) -> Sink {
    Sink::None
}

/// Lines from the end of this launch's log kept in the diagnostics.
const DIAGNOSTIC_LINES: usize = 300;

/// What a bug report needs, to paste: Runo's version and system, the agent CLIs it found,
/// the settings with anything secret left out, and the end of this launch's log. Every
/// line goes through `redact` again, since child processes write to the log unmasked.
pub fn diagnostics() -> String {
    let settings = std::fs::read_to_string(crate::store::data_dir().join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .map(hide_secrets)
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| "(none saved)".into());
    let log = CURRENT_LOG
        .get()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    diagnostics_text(&crate::caps::versions(), &settings, &log)
}

fn diagnostics_text(agents: &[(String, String)], settings: &str, log: &str) -> String {
    let agents = if agents.is_empty() {
        "(none found yet)".to_string()
    } else {
        agents
            .iter()
            .map(|(id, version)| format!("{id} {version}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let lines: Vec<&str> = log.lines().collect();
    let tail = lines[lines.len().saturating_sub(DIAGNOSTIC_LINES)..]
        .iter()
        .map(|l| redact(l))
        .collect::<Vec<_>>()
        .join("\n");
    // `hide_secrets` already replaced secret fields with a note; `redact` would mask the
    // note too, so lines that end in one are kept as they are.
    let settings = settings
        .lines()
        .map(|line| {
            let value = line.trim_end().trim_end_matches(',');
            if value.ends_with("\"(empty)\"") || value.ends_with("\"(set, hidden)\"") {
                line.to_string()
            } else {
                redact(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\nAgent CLIs: {agents}\n\nSettings (secrets left out):\n```json\n{}\n```\n\nLast {} log lines:\n```\n{tail}\n```\n",
        about(),
        settings,
        lines.len().min(DIAGNOSTIC_LINES),
    )
}

/// Settings fields whose names say they hold something secret, such as `api_key`,
/// `figma_token` or `env` lines, replaced with a note of whether they were set.
fn hide_secrets(value: serde_json::Value) -> serde_json::Value {
    const SECRET: [&str; 7] = [
        "key",
        "token",
        "secret",
        "password",
        "env",
        "auth",
        "credential",
    ];
    match value {
        serde_json::Value::Object(map) => map
            .into_iter()
            .map(|(name, v)| {
                let lower = name.to_lowercase();
                let hidden = SECRET.iter().any(|w| lower.contains(w))
                    && !matches!(v, serde_json::Value::Bool(_) | serde_json::Value::Number(_));
                let v = if !hidden {
                    hide_secrets(v)
                } else if is_blank(&v) {
                    "(empty)".into()
                } else {
                    "(set, hidden)".into()
                };
                (name, v)
            })
            .collect(),
        serde_json::Value::Array(items) => items.into_iter().map(hide_secrets).collect(),
        other => other,
    }
}

fn is_blank(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        serde_json::Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

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
    /// Links to the release's `SHA256SUMS` and its Ed25519 signature, when published.
    pub checksums: Option<String>,
    pub signature: Option<String>,
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
    let link = |name: &str| assets.iter().find(|a| a.0 == name).map(|a| a.1.clone());
    Some(Release {
        version,
        page: value["html_url"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/latest")),
        asset: pick_asset(&assets, std::env::consts::OS, std::env::consts::ARCH),
        checksums: link(SUMS),
        signature: link(&format!("{SUMS}.sig")),
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

/// What the last update check found, in a line for Settings, and whether a check is
/// running now.
static CHECKED: std::sync::Mutex<(String, bool)> = std::sync::Mutex::new((String::new(), false));

/// What the last update check found, and whether one is running now.
pub fn last_check() -> (String, bool) {
    CHECKED.lock().map(|c| c.clone()).unwrap_or_default()
}

/// Note that a check started, so Settings shows it until it's done.
pub fn check_started() {
    if let Ok(mut c) = CHECKED.lock() {
        c.1 = true;
    }
}

fn checked(line: String) {
    if let Ok(mut c) = CHECKED.lock() {
        *c = (line, false);
    }
}

/// Ask GitHub for the latest release; None when it isn't newer or can't be reached.
pub fn check_release() -> Option<Release> {
    let found = fetch_release();
    let current = env!("CARGO_PKG_VERSION");
    let line = match &found {
        Some(release) => format!(
            "Runo {} is out. Leave Settings to download it from the update card.",
            release.version
        ),
        None if !LAST_FAILED.load(std::sync::atomic::Ordering::Relaxed) => {
            format!("Runo {current} is the latest version.")
        }
        None => "Couldn't reach GitHub to check. Try again later.".to_string(),
    };
    checked(line);
    found
}

/// Whether the last check couldn't get an answer from GitHub.
static LAST_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn fetch_release() -> Option<Release> {
    let failed = |f: bool| LAST_FAILED.store(f, std::sync::atomic::Ordering::Relaxed);
    failed(true);
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
            return None;
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
        return None;
    }
    let found = newer_release(&body, current);
    match &found {
        Some(release) => info(
            "app-update",
            format!("Runo {} is out (running {current})", release.version),
        ),
        None if serde_json::from_str::<serde_json::Value>(&body)
            .is_ok_and(|v| v["tag_name"].is_string()) =>
        {
            failed(false);
            info("app-update", format!("checked: {current} is the latest"))
        }
        // GitHub answers 404 until the first release is published.
        None if serde_json::from_str::<serde_json::Value>(&body)
            .is_ok_and(|v| v["message"] == "Not Found") =>
        {
            failed(false);
            info("app-update", "checked: no release published yet")
        }
        None => warn(
            "app-update",
            format!(
                "the update check got no release from GitHub: {}",
                gist(&body)
            ),
        ),
    }
    found
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

/// The checksum list every release publishes, in `sha256sum` format.
const SUMS: &str = "SHA256SUMS";

/// The public half of the key that signs `SHA256SUMS` (base64 Ed25519; see
/// scripts/update-key.sh). While it is empty, downloads are checked against the
/// checksums alone.
const UPDATE_KEY: &str = include_str!("update.pub");

/// Whether `url` is a download from Runo's own GitHub releases.
fn release_link(url: &str) -> bool {
    url.strip_prefix(&format!("https://github.com/{REPO}/releases/download/"))
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(".."))
}

/// The SHA-256 (lowercase hex) that `sums` lists for `name`.
fn listed_sum<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim_end().split_once(char::is_whitespace)?;
        // `sha256sum` marks binary-mode files with a leading `*`.
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then_some(hash)
    })
}

/// The SHA-256 of the file at `path`, as lowercase hex.
fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(ctx
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Check that `sums` is signed (when Runo has an update key) and lists `name` with the
/// hash of the file at `path`.
fn check_download(
    path: &Path,
    name: &str,
    sums: &str,
    signature: Option<&str>,
    key: &str,
) -> Result<(), String> {
    if !key.trim().is_empty() {
        let signature = signature.ok_or("The release's checksums aren't signed.")?;
        if !crate::registry::verify(sums.as_bytes(), signature, key) {
            return Err("The release's checksum signature doesn't match Runo's key.".into());
        }
    }
    let want = listed_sum(sums, name).ok_or("The release's checksums don't list this file.")?;
    let got = sha256_file(path).map_err(|e| format!("Could not read the download: {e}"))?;
    if !got.eq_ignore_ascii_case(want) {
        return Err("The download doesn't match the release's checksum.".into());
    }
    Ok(())
}

/// Fetch a small text file (checksums, signature) from the release.
fn fetch_text(url: &str) -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-filesize",
            "1048576",
            "-m",
            "60",
            url,
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Could not fetch {}: {}",
            url.rsplit('/').next().unwrap_or(url),
            gist(String::from_utf8_lossy(&out.stderr).trim())
        ));
    }
    String::from_utf8(out.stdout).map_err(|_| "The release's checksums aren't text.".into())
}

/// Mark a download as coming from the internet, as a browser would, so macOS checks
/// its signature and notarization when it is opened.
#[cfg(target_os = "macos")]
fn mark_downloaded(path: &Path) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let ok = std::process::Command::new("xattr")
        .args(["-w", "com.apple.quarantine", &format!("0081;{now:x};Runo;")])
        .arg(path)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return Err("Could not mark the download for macOS to check.".into());
    }
    // A disk image can be checked now: refuse one Gatekeeper wouldn't open.
    if path.extension().is_some_and(|e| e == "dmg") {
        let out = std::process::Command::new("spctl")
            .args([
                "--assess",
                "--type",
                "open",
                "--context",
                "context:primary-signature",
            ])
            .arg(path)
            .output()
            .map_err(|e| format!("Could not run spctl: {e}"))?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
            warn(
                "app-update",
                format!("Gatekeeper rejected {}: {}", path.display(), gist(&why)),
            );
            return Err("macOS says the disk image isn't signed and notarized.".into());
        }
    }
    Ok(())
}

/// Mark a download as coming from the internet (Mark of the Web), so Windows checks it
/// with SmartScreen when it is opened.
#[cfg(windows)]
fn mark_downloaded(path: &Path) -> Result<(), String> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(&stream, "[ZoneTransfer]\r\nZoneId=3\r\n")
        .map_err(|e| format!("Could not mark the download for Windows to check: {e}"))
}

/// Linux has no download marking; the checksum check is all there is.
#[cfg(not(any(target_os = "macos", windows)))]
fn mark_downloaded(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Download this platform's file of `release` into `dir`, and keep it only when it
/// matches the release's (signed) checksums. Nothing is installed: the person opens it.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    let (name, url) = release
        .asset
        .as_ref()
        .ok_or("The release has no file for this computer.")?;
    let name = Path::new(name)
        .file_name()
        .ok_or("The release file has no name.")?;
    let refuse = |why: String| {
        warn(
            "app-update",
            format!("refused the {} download: {why}", release.version),
        );
        why
    };
    let links = [
        Some(url),
        release.checksums.as_ref(),
        release.signature.as_ref(),
    ];
    if links.iter().flatten().any(|l| !release_link(l)) {
        return Err(refuse(
            "The download isn't from Runo's GitHub releases.".into(),
        ));
    }
    let sums_url = release
        .checksums
        .as_ref()
        .ok_or_else(|| refuse("The release has no checksums, so it can't be checked.".into()))?;
    let sums = fetch_text(sums_url).map_err(refuse)?;
    let signature = match &release.signature {
        Some(link) if !UPDATE_KEY.trim().is_empty() => Some(fetch_text(link).map_err(refuse)?),
        _ => None,
    };
    let path = dir.join(name);
    let partial = dir.join(format!("{}.part", name.to_string_lossy()));
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "-m",
            "900",
            "-o",
        ])
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
    if let Err(why) = check_download(
        &partial,
        &name.to_string_lossy(),
        &sums,
        signature.as_deref(),
        UPDATE_KEY,
    ) {
        let _ = std::fs::remove_file(&partial);
        return Err(refuse(why));
    }
    if let Err(why) = mark_downloaded(&partial) {
        let _ = std::fs::remove_file(&partial);
        return Err(refuse(why));
    }
    std::fs::rename(&partial, &path).map_err(|e| {
        error(
            "app-update",
            format!("could not save {}: {e}", path.display()),
        );
        e.to_string()
    })?;
    info(
        "app-update",
        format!("downloaded and checked {}", path.display()),
    );
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

    fn logs_temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runo-applog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_full_log_moves_aside_keeping_a_few_pieces() {
        let dir = logs_temp("parts");
        let path = dir.join("runo-20261025-100000.log");
        for n in 0..5 {
            std::fs::write(&path, format!("piece {n}")).unwrap();
            shift_parts(&path, 3);
        }
        std::fs::write(&path, "current").unwrap();
        assert_eq!(
            names_in(&dir),
            [
                "runo-20261025-100000.log",
                "runo-20261025-100000.log.1",
                "runo-20261025-100000.log.2"
            ]
        );
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        assert_eq!(read("runo-20261025-100000.log.1"), "piece 4");
        assert_eq!(read("runo-20261025-100000.log.2"), "piece 3");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_launches_go_with_all_their_pieces() {
        let dir = logs_temp("launches");
        for name in [
            "runo-20261023-090000.log",
            "runo-20261023-090000.log.1",
            "runo-20261024-090000.log",
            "runo-20261025-090000.log",
            "runo-20261025-090000.log.1",
            "runo-20261025-090000.log.2",
            "crash-20261023-090000.txt",
        ] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        prune_launches(&dir, 2);
        assert_eq!(
            names_in(&dir),
            [
                "crash-20261023-090000.txt",
                "runo-20261024-090000.log",
                "runo-20261025-090000.log",
                "runo-20261025-090000.log.1",
                "runo-20261025-090000.log.2"
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn diagnostics_leave_secrets_out() {
        let settings = serde_json::json!({
            "theme": "dark",
            "api_key": "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
            "figma_token": "",
            "check_runo_updates": true,
            "agents": { "claude": { "env": ["ANTHROPIC_API_KEY=sk-ant-x"], "binary": "claude" } },
            "connections": [{ "host": "box", "password": "hunter2" }],
        });
        let settings = serde_json::to_string_pretty(&hide_secrets(settings)).unwrap();
        let mut log: Vec<String> = (0..400).map(|n| format!("line {n}")).collect();
        log.push("git push https://x-access-token:ghp_abcdefghijklmnopqrstuvwxyz0123456789@github.com/a/b".into());
        let text = diagnostics_text(
            &[("claude".into(), "2.1.300".into())],
            &settings,
            &log.join("\n"),
        );
        assert!(text.starts_with(&format!("Runo {}", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains("Agent CLIs: claude 2.1.300"));
        assert!(text.contains("\"theme\": \"dark\""));
        assert!(text.contains("\"check_runo_updates\": true"));
        assert!(text.contains("\"binary\": \"claude\""));
        assert!(text.contains("\"figma_token\": \"(empty)\""));
        for secret in ["abcdefghijklmnopqrstuvwxyz", "sk-ant-x", "hunter2", "ghp_"] {
            assert!(!text.contains(secret), "{secret} leaked:\n{text}");
        }
        // Only the end of the log.
        assert!(text.contains("Last 300 log lines"));
        assert!(!text.contains("line 100\n"));
        assert!(text.contains("line 399"));
    }

    #[test]
    fn each_platform_gets_its_own_file_from_a_real_release() {
        // The files v0.5.9 shipped, plus the checksum list releases are getting.
        let names = [
            "Runo-0.5.9-linux-arm64.deb",
            "Runo-0.5.9-linux-arm64.tar.gz",
            "Runo-0.5.9-linux-x86_64.deb",
            "Runo-0.5.9-linux-x86_64.tar.gz",
            "Runo-0.5.9-macos-arm64.dmg",
            "Runo-0.5.9-macos-arm64.zip",
            "Runo-0.5.9-macos-x86_64.dmg",
            "Runo-0.5.9-macos-x86_64.zip",
            "Runo-0.5.9-windows-x86_64.zip",
            "SHA256SUMS",
        ];
        let assets: Vec<(String, String)> = names
            .iter()
            .map(|n| (n.to_string(), format!("https://x/{n}")))
            .collect();
        let pick = |os, arch| pick_asset(&assets, os, arch).map(|a| a.0);
        assert_eq!(
            pick("macos", "aarch64").as_deref(),
            Some("Runo-0.5.9-macos-arm64.dmg")
        );
        assert_eq!(
            pick("macos", "x86_64").as_deref(),
            Some("Runo-0.5.9-macos-x86_64.dmg")
        );
        assert_eq!(
            pick("linux", "x86_64").as_deref(),
            Some("Runo-0.5.9-linux-x86_64.deb")
        );
        assert_eq!(
            pick("linux", "aarch64").as_deref(),
            Some("Runo-0.5.9-linux-arm64.deb")
        );
        assert_eq!(
            pick("windows", "x86_64").as_deref(),
            Some("Runo-0.5.9-windows-x86_64.zip")
        );
        // No Windows build for ARM: nothing rather than the x86_64 one.
        assert_eq!(pick("windows", "aarch64"), None);
    }

    #[test]
    fn only_a_newer_final_release_is_offered() {
        let reply = |tag: &str, extra: &str| {
            format!(
                r#"{{"tag_name":"{tag}","html_url":"https://x/{tag}","draft":false,
                "prerelease":false,{extra}"assets":[]}}"#
            )
        };
        let found =
            |tag: &str, current: &str| newer_release(&reply(tag, ""), current).map(|r| r.version);
        // Numbers compare as numbers, not text.
        assert_eq!(found("v0.10.0", "0.9.9").as_deref(), Some("0.10.0"));
        assert_eq!(found("v0.5.10", "0.5.9").as_deref(), Some("0.5.10"));
        assert_eq!(found("v1.0.0", "0.99.0").as_deref(), Some("1.0.0"));
        // The same or an older one is never offered, which would be a downgrade.
        assert_eq!(found("v0.5.9", "0.5.9"), None);
        assert_eq!(found("v0.5.8", "0.5.9"), None);
        assert_eq!(found("v0.4.12", "0.5.0"), None);
        // A tag that isn't a version is ignored rather than offered.
        assert_eq!(found("nightly", "0.5.9"), None);
        // Drafts never count, even when newer.
        let draft = reply("v9.0.0", "").replace(r#""draft":false"#, r#""draft":true"#);
        assert!(newer_release(&draft, "0.5.9").is_none());
        // With no file for this platform, the release page is still offered.
        let release = newer_release(&reply("v9.0.0", ""), "0.5.9").unwrap();
        assert_eq!(release.asset, None);
        assert_eq!(release.page, "https://x/v9.0.0");
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

    #[test]
    fn releases_link_their_checksums() {
        let json = r#"{"tag_name":"v0.3.0","assets":[
            {"name":"Runo-0.3.0-macos-arm64.dmg","browser_download_url":"https://x/arm.dmg"},
            {"name":"SHA256SUMS","browser_download_url":"https://x/SHA256SUMS"},
            {"name":"SHA256SUMS.sig","browser_download_url":"https://x/SHA256SUMS.sig"}]}"#;
        let release = newer_release(json, "0.2.9").unwrap();
        assert_eq!(release.checksums.as_deref(), Some("https://x/SHA256SUMS"));
        assert_eq!(
            release.signature.as_deref(),
            Some("https://x/SHA256SUMS.sig")
        );
    }

    #[test]
    fn only_runos_own_release_links_are_downloaded() {
        assert!(release_link(
            "https://github.com/deltalark/runo/releases/download/v0.6.0/Runo.dmg"
        ));
        for bad in [
            "http://github.com/deltalark/runo/releases/download/v0.6.0/Runo.dmg",
            "https://github.com/someone/runo/releases/download/v0.6.0/Runo.dmg",
            "https://github.com/deltalark/runo/releases/download/../../x/Runo.dmg",
            "https://evil.example/deltalark/runo/releases/download/v1/Runo.dmg",
            "https://github.com/deltalark/runo/releases/download/",
        ] {
            assert!(!release_link(bad), "{bad}");
        }
    }

    #[test]
    fn downloads_must_match_the_signed_checksums() {
        let dir = temp("sums");
        let file = dir.join("Runo.dmg");
        std::fs::write(&file, b"hello").unwrap();
        // sha256("hello")
        let hash = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let sums = format!("{}  other.zip\n{hash} *Runo.dmg\n", "0".repeat(64));
        assert_eq!(listed_sum(&sums, "Runo.dmg"), Some(hash));
        assert_eq!(listed_sum(&sums, "Runo"), None);
        assert_eq!(listed_sum("short  Runo.dmg", "Runo.dmg"), None);

        // Without a key, the checksum alone decides.
        assert!(check_download(&file, "Runo.dmg", &sums, None, "").is_ok());
        assert!(check_download(&file, "missing.dmg", &sums, None, "").is_err());
        std::fs::write(&file, b"tampered").unwrap();
        assert!(check_download(&file, "Runo.dmg", &sums, None, "").is_err());
        std::fs::write(&file, b"hello").unwrap();

        // With a key, the checksums must carry its signature.
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD;
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        use ring::signature::KeyPair;
        let key = b64.encode(pair.public_key().as_ref());
        let signature = b64.encode(pair.sign(sums.as_bytes()).as_ref());
        assert!(check_download(&file, "Runo.dmg", &sums, Some(&signature), &key).is_ok());
        assert!(check_download(&file, "Runo.dmg", &sums, None, &key).is_err());
        let forged = sums.replace(hash, &"1".repeat(64));
        assert!(check_download(&file, "Runo.dmg", &forged, Some(&signature), &key).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
