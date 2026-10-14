//! How much of each agent's plan is left: the share of each limit window used and when it
//! resets, read from what the agent CLI itself keeps. Claude Code's login can ask its
//! usage endpoint; Codex writes its limits into every session log. Agents whose CLI
//! shares nothing say whether they are signed in instead. Reads run off the UI thread
//! and are cached; the status bar and its Usage popover draw from the cache.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// One limit window: how much of it is used, and when it starts over.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub label: &'static str,
    /// Percent used, 0 to 100.
    pub used: f32,
    /// Unix seconds when the window resets, when known.
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    /// The plan's limit windows.
    Limits(Vec<Window>),
    /// Nothing to act on, such as an agent whose CLI shares no usage.
    Note(String),
    /// Something the person can fix, with a short form for the status bar.
    Problem { short: String, long: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub reading: Reading,
    /// Unix seconds the reading describes: when it was fetched, or for Codex, when the
    /// agent last reported its limits.
    pub at: i64,
}

impl Usage {
    /// The window closest to its limit, which the status bar shows.
    pub fn headline(&self) -> Option<&Window> {
        match &self.reading {
            Reading::Limits(w) => w.iter().fold(None, |best: Option<&Window>, w| match best {
                Some(b) if b.used >= w.used => Some(b),
                _ => Some(w),
            }),
            _ => None,
        }
    }
}

static STATE: RwLock<Option<HashMap<String, Usage>>> = RwLock::new(None);
static CHECKED: Mutex<Option<Instant>> = Mutex::new(None);
static BUSY: Mutex<bool> = Mutex::new(false);

/// How long a reading stands before opening the popover reads again.
const FRESH: Duration = Duration::from_secs(60);

/// The cached reading for an agent, once one was made.
pub fn get(provider: &str) -> Option<Usage> {
    STATE.read().ok()?.as_ref()?.get(provider).cloned()
}

/// Whether a read is running now, for the refresh button.
pub fn busy() -> bool {
    BUSY.lock().map(|b| *b).unwrap_or(false)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Read every installed agent's usage again in the background; unless `force`, only
/// when the last read is more than a minute old.
pub fn refresh(force: bool) {
    if let Ok(mut checked) = CHECKED.lock() {
        if !force && checked.is_some_and(|at| at.elapsed() < FRESH) {
            return;
        }
        *checked = Some(Instant::now());
    }
    if let Ok(mut busy) = BUSY.lock() {
        if *busy {
            return;
        }
        *busy = true;
    }
    crate::caps::mark_updated();
    std::thread::spawn(|| {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        let mut found = HashMap::new();
        if let Some(home) = home {
            for p in crate::agent::PROVIDERS.iter() {
                if !crate::caps::get(p.id).installed {
                    continue;
                }
                if let Some(u) = read(p.id, &home) {
                    found.insert(p.id.to_string(), u);
                }
            }
        }
        if let Ok(mut state) = STATE.write() {
            *state = Some(found);
        }
        if let Ok(mut busy) = BUSY.lock() {
            *busy = false;
        }
        crate::caps::mark_updated();
    });
}

fn read(provider: &str, home: &Path) -> Option<Usage> {
    let reading = match provider {
        "claude" => claude(home),
        "codex" => {
            let dir = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            return Some(codex(&dir.join("sessions"), now()));
        }
        _ => match crate::signin::get(provider) {
            Some(false) => Reading::Problem {
                short: "Not signed in".into(),
                long: "Not signed in. Sign in from its CLI or in Settings.".into(),
            },
            Some(true) => Reading::Note("Signed in. Its CLI doesn't share usage.".into()),
            None => Reading::Note("Its CLI doesn't share usage.".into()),
        },
    };
    Some(Usage { reading, at: now() })
}

// Claude Code

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// Claude Code's subscription login: its OAuth token, from the credentials file or,
/// on macOS, the keychain.
fn claude_token(home: &Path) -> Option<String> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let from_file = std::fs::read_to_string(dir.join(".credentials.json")).ok();
    let from_keychain = || {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let out = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let text = from_file.or_else(from_keychain)?;
    oauth_token(&text)
}

fn oauth_token(credentials: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(credentials.trim()).ok()?;
    let token = v["claudeAiOauth"]["accessToken"].as_str()?;
    (!token.is_empty()).then(|| token.to_string())
}

fn claude(home: &Path) -> Reading {
    let Some(token) = claude_token(home) else {
        let key = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]
            .iter()
            .any(|k| {
                std::env::var(k).is_ok_and(|v| !v.is_empty())
                    || crate::shellenv::login_var(k).is_some_and(|v| !v.is_empty())
            });
        return if key {
            Reading::Note("Uses an API key, billed per token with no plan limits.".into())
        } else {
            Reading::Problem {
                short: "Not signed in".into(),
                long: "Not signed in with a Claude plan. Run claude and log in.".into(),
            }
        };
    };
    match fetch(CLAUDE_USAGE_URL, &token) {
        Some((200, body)) => claude_windows(&body).map_or_else(
            || Reading::Note("Claude didn't report any limits.".into()),
            Reading::Limits,
        ),
        Some((401 | 403, _)) => Reading::Problem {
            short: "Sign-in expired".into(),
            long: "Sign-in expired. Run Claude once to refresh it.".into(),
        },
        Some((code, _)) => Reading::Problem {
            short: "Usage unavailable".into(),
            long: format!("Claude's usage service answered {code}. Try again later."),
        },
        None => Reading::Problem {
            short: "Offline".into(),
            long: "Couldn't reach Claude's usage service.".into(),
        },
    }
}

/// GET a URL with a bearer token; the HTTP status and body. The token goes to curl on
/// stdin so it never shows in the process list.
fn fetch(url: &str, token: &str) -> Option<(u16, String)> {
    let mut child = Command::new("curl")
        .args(["-sS", "-m", "10", "-H", "@-", "-w", "\n%{http_code}", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = write!(
            stdin,
            "Authorization: Bearer {token}\nanthropic-beta: oauth-2025-04-20\nAccept: application/json\n"
        );
    }
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.trim_end().rsplit_once('\n')?;
    let code: u16 = code.trim().parse().ok()?;
    (code != 0).then(|| (code, body.to_string()))
}

/// The windows in Claude's usage answer, shortest first.
fn claude_windows(body: &str) -> Option<Vec<Window>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let windows: Vec<Window> = [
        ("five_hour", "5-hour"),
        ("seven_day", "Weekly"),
        ("seven_day_opus", "Weekly Opus"),
        ("seven_day_sonnet", "Weekly Sonnet"),
    ]
    .iter()
    .filter_map(|(key, label)| {
        let w = v.get(key)?;
        let used = w.get("utilization")?.as_f64()? as f32;
        let resets_at = w["resets_at"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp());
        Some(Window {
            label,
            used: used.clamp(0., 100.),
            resets_at,
        })
    })
    .collect();
    (!windows.is_empty()).then_some(windows)
}

// Codex

/// Codex's limits as of its latest session log, which records them after every turn.
fn codex(sessions: &Path, now: i64) -> Usage {
    for file in newest_logs(sessions, 8) {
        let Some(tail) = read_tail(&file, 512 * 1024) else {
            continue;
        };
        if let Some((windows, at)) = tail.lines().rev().find_map(codex_limits) {
            let windows = windows
                .into_iter()
                .map(|w| match w.resets_at {
                    // The window started over since Codex last reported.
                    Some(r) if r <= now => Window {
                        used: 0.,
                        resets_at: None,
                        ..w
                    },
                    _ => w,
                })
                .collect();
            return Usage {
                reading: Reading::Limits(windows),
                at,
            };
        }
    }
    let reading = match crate::signin::get("codex") {
        Some(false) => Reading::Problem {
            short: "Not signed in".into(),
            long: "Not signed in. Run codex login.".into(),
        },
        _ => Reading::Problem {
            short: "Run Codex to refresh".into(),
            long: "Codex reports its limits after each turn. Run it once to see them.".into(),
        },
    };
    Usage { reading, at: now }
}

/// The limits in one session log line, and when they were recorded.
fn codex_limits(line: &str) -> Option<(Vec<Window>, i64)> {
    if !line.contains("rate_limits") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let at = v["timestamp"]
        .as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp())?;
    let payload = v.get("payload").unwrap_or(&v);
    let limits = payload.get("rate_limits").filter(|l| l.is_object())?;
    let window = |key: &str| {
        let w = limits.get(key).filter(|w| w.is_object())?;
        let used = w["used_percent"].as_f64()? as f32;
        let minutes = w["window_minutes"].as_i64();
        let resets_at = w["resets_at"]
            .as_i64()
            .or_else(|| w["resets_in_seconds"].as_i64().map(|s| at + s));
        let label = match minutes {
            Some(300) => "5-hour",
            Some(10080) => "Weekly",
            Some(m) if m >= 1440 * 28 => "Monthly",
            Some(m) if m >= 1440 => "Daily",
            _ if key == "primary" => "Short-term",
            _ => "Long-term",
        };
        Some(Window {
            label,
            used: used.clamp(0., 100.),
            resets_at,
        })
    };
    let windows: Vec<Window> = ["primary", "secondary"]
        .iter()
        .filter_map(|k| window(k))
        .collect();
    (!windows.is_empty()).then_some((windows, at))
}

/// The newest `rollout-*.jsonl` files under Codex's dated sessions folders.
fn newest_logs(sessions: &Path, limit: usize) -> Vec<PathBuf> {
    fn sorted(dir: &Path) -> Vec<PathBuf> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        entries.sort();
        entries.reverse();
        entries
    }
    let mut out = Vec::new();
    for year in sorted(sessions).into_iter().filter(|p| p.is_dir()) {
        for month in sorted(&year).into_iter().filter(|p| p.is_dir()) {
            for day in sorted(&month).into_iter().filter(|p| p.is_dir()) {
                let mut files: Vec<(std::time::SystemTime, PathBuf)> = sorted(&day)
                    .into_iter()
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
                    })
                    .map(|p| {
                        let modified = p
                            .metadata()
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::UNIX_EPOCH);
                        (modified, p)
                    })
                    .collect();
                files.sort_by(|a, b| b.cmp(a));
                out.extend(files.into_iter().map(|(_, p)| p));
                if out.len() >= limit {
                    out.truncate(limit);
                    return out;
                }
            }
        }
    }
    out
}

/// The last `max` bytes of a file, from the first whole line.
fn read_tail(path: &Path, max: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest.to_string())?
    } else {
        text
    })
}

/// Time left until `at`, as "5d 10h", "3h 12m" or "45m".
pub fn left(at: i64, now: i64) -> String {
    let secs = (at - now).max(0);
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{}m", m.max(1))
    }
}

/// How long ago `at` was, as "just now", "12m ago", "3h ago" or "2d ago".
pub fn ago(at: i64, now: i64) -> String {
    let secs = (now - at).max(0);
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// Seconds since the epoch now, for formatting.
pub fn now_secs() -> i64 {
    now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_windows() {
        let body = r#"{"five_hour":{"utilization":12.0,"resets_at":"2026-10-14T20:00:00+00:00"},
            "seven_day":{"utilization":29.4,"resets_at":"2026-10-20T07:00:00Z"},
            "seven_day_opus":null,"seven_day_oauth_apps":null}"#;
        let w = claude_windows(body).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].label, "5-hour");
        assert_eq!(w[1].label, "Weekly");
        assert!((w[1].used - 29.4).abs() < 0.01);
        assert_eq!(w[1].resets_at, Some(1_792_479_600));
        let u = Usage {
            reading: Reading::Limits(w),
            at: 0,
        };
        assert_eq!(u.headline().unwrap().label, "Weekly");
        assert_eq!(claude_windows("{}"), None);
    }

    #[test]
    fn reads_the_oauth_token() {
        let creds = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","expiresAt":1}}"#;
        assert_eq!(oauth_token(creds).as_deref(), Some("sk-ant-oat01-x"));
        assert_eq!(oauth_token("{}"), None);
    }

    #[test]
    fn reads_codex_limits_old_and_new() {
        let new = r#"{"timestamp":"2026-10-14T10:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":40.0,"window_minutes":300,"resets_at":1792000000},"secondary":{"used_percent":2.0,"window_minutes":10080,"resets_at":1792500000}}}}"#;
        let (w, at) = codex_limits(new).unwrap();
        assert_eq!(
            at,
            chrono::DateTime::parse_from_rfc3339("2026-10-14T10:00:00Z")
                .unwrap()
                .timestamp()
        );
        assert_eq!(w[0].label, "5-hour");
        assert_eq!(w[1].label, "Weekly");
        assert_eq!(w[1].resets_at, Some(1_792_500_000));
        let old = r#"{"timestamp":"2026-10-14T10:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":5.5,"window_minutes":300,"resets_in_seconds":600}}}}"#;
        let (w, at) = codex_limits(old).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].resets_at, Some(at + 600));
        let none = r#"{"timestamp":"2026-10-14T10:00:00Z","payload":{"type":"token_count","rate_limits":null}}"#;
        assert_eq!(codex_limits(none), None);
    }

    #[test]
    fn codex_uses_the_newest_log_and_clears_past_windows() {
        let dir = std::env::temp_dir().join(format!("runo-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let day = dir.join("2026/10/14");
        std::fs::create_dir_all(&day).unwrap();
        let line = |used: f64, resets: i64| {
            format!(
                "{{\"timestamp\":\"2026-10-14T10:00:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"primary\":{{\"used_percent\":{used},\"window_minutes\":300,\"resets_at\":{resets}}},\"secondary\":{{\"used_percent\":3.0,\"window_minutes\":10080,\"resets_at\":2000000000}}}}}}}}\n"
            )
        };
        std::fs::write(
            day.join("rollout-2026-10-14T09-00-00-a.jsonl"),
            line(90., 1_000),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(
            day.join("rollout-2026-10-14T10-00-00-b.jsonl"),
            format!("{{\"type\":\"session_meta\"}}\n{}", line(40., 1_000)),
        )
        .unwrap();
        let u = codex(&dir, 1_500);
        let Reading::Limits(w) = &u.reading else {
            panic!("{u:?}")
        };
        // The 5-hour window reset at 1000, before now.
        assert_eq!(w[0].used, 0.);
        assert_eq!(w[0].resets_at, None);
        assert_eq!(u.headline().unwrap().label, "Weekly");
        let empty = codex(&dir.join("missing"), 0);
        assert!(matches!(empty.reading, Reading::Problem { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn formats_time_left() {
        assert_eq!(left(5 * 86_400 + 10 * 3600 + 59, 0), "5d 10h");
        assert_eq!(left(3 * 3600 + 12 * 60, 0), "3h 12m");
        assert_eq!(left(30, 0), "1m");
        assert_eq!(ago(0, 7200), "2h ago");
    }
}
