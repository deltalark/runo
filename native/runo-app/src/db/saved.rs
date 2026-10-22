//! The connections the person keeps, per project, in `~/.runo/data/connections.json`.
//! No secret is written there: a connection found in the project's files is found again
//! each time it opens, and a password typed or pasted goes to the macOS Keychain (on
//! other systems, to a file only the person can read).

use super::{detect, redact, Env, Kind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "lowercase")]
pub(crate) enum Source {
    /// Found in the project's files, by `detect::Found::key`.
    Found { key: String },
    /// Added by URL; the password, if any, is kept apart.
    Url { url: String, password: bool },
    /// A SQLite file picked by path.
    File { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Saved {
    pub id: String,
    pub project: u64,
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub env: Env,
    pub source: Source,
    /// The address without secrets, as shown in the list.
    #[serde(default)]
    pub shown: String,
}

#[cfg(test)]
thread_local! {
    /// Tests keep their files apart from each other and from the person's.
    pub(crate) static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn folder() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with_borrow(|d| d.clone()) {
        return dir;
    }
    crate::store::data_dir().join("data")
}

fn file() -> PathBuf {
    folder().join("connections.json")
}

pub(crate) fn load() -> Vec<Saved> {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn store(all: &[Saved]) -> std::io::Result<()> {
    std::fs::create_dir_all(folder())?;
    let text = serde_json::to_string_pretty(all).unwrap_or_else(|_| "[]".into());
    std::fs::write(file(), text)
}

/// A query kept under a name, for one connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedQuery {
    pub id: String,
    /// The saved connection it runs on.
    pub conn: String,
    pub name: String,
    pub sql: String,
}

fn queries_file() -> PathBuf {
    folder().join("queries.json")
}

pub(crate) fn load_queries() -> Vec<SavedQuery> {
    std::fs::read_to_string(queries_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn store_queries(all: &[SavedQuery]) -> std::io::Result<()> {
    std::fs::create_dir_all(folder())?;
    let text = serde_json::to_string_pretty(all).unwrap_or_else(|_| "[]".into());
    std::fs::write(queries_file(), text)
}

/// One thing an agent ran on a database, kept so the person can see it in the Data place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Activity {
    /// Seconds since 1970.
    pub at: u64,
    pub conn: String,
    pub sql: String,
    #[serde(default)]
    pub rows: usize,
    #[serde(default)]
    pub millis: u128,
    /// Why it didn't run, when it didn't.
    #[serde(default)]
    pub refused: Option<String>,
}

fn activity_file() -> PathBuf {
    folder().join("activity.jsonl")
}

/// The log keeps about this much; older lines are dropped.
const ACTIVITY_MAX: u64 = 1 << 20;

/// Add a line to the activity log, which only the person can read.
pub(crate) fn log_activity(entry: &Activity) {
    let path = activity_file();
    let _ = std::fs::create_dir_all(folder());
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > ACTIVITY_MAX) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let keep: Vec<&str> = text.lines().collect();
            let half = keep[keep.len() / 2..].join("\n");
            let _ = std::fs::write(&path, half + "\n");
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let (Ok(mut file), Ok(line)) = (options.open(&path), serde_json::to_string(entry)) {
        use std::io::Write as _;
        let _ = writeln!(file, "{line}");
    }
}

/// The latest `n` entries for connection `conn`, newest first.
pub(crate) fn activity(conn: &str, n: usize) -> Vec<Activity> {
    let text = std::fs::read_to_string(activity_file()).unwrap_or_default();
    let mut out: Vec<Activity> = text
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Activity>(l).ok())
        .filter(|a| a.conn == conn)
        .take(n)
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.at));
    out
}

/// A new connection's id: unique enough for one person's list.
pub(crate) fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("c{:x}", nanos)
}

/// Save a connection found in the project.
pub(crate) fn from_found(project: u64, found: &detect::Found) -> Saved {
    Saved {
        id: new_id(),
        project,
        name: found.name.clone(),
        kind: found.kind,
        env: found.env,
        source: Source::Found {
            key: found.key.clone(),
        },
        shown: found.shown.clone(),
    }
}

/// Save a connection to a URL or a SQLite file the person typed. The password, if the URL
/// has one, goes to the Keychain under the connection's id.
pub(crate) fn from_typed(project: u64, typed: &str) -> Result<Saved, String> {
    let typed = typed.trim();
    let id = new_id();
    let path = Path::new(typed.trim_start_matches("file:"));
    if Kind::of_url(typed).is_none() && detect::is_sqlite(path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        return Ok(Saved {
            id,
            project,
            name,
            kind: Kind::Sqlite,
            env: Env::Local,
            source: Source::File {
                path: path.to_string_lossy().to_string(),
            },
            shown: typed.to_string(),
        });
    }
    let kind = Kind::of_url(typed).ok_or_else(|| {
        "That isn't a database URL or a SQLite file. Paste something like postgres://user:password@host:5432/name.".to_string()
    })?;
    if kind == Kind::Sqlite {
        return from_typed(
            project,
            typed
                .trim_start_matches("sqlite:")
                .trim_start_matches("file:"),
        );
    }
    let mut url = url::Url::parse(typed).map_err(|e| format!("That URL can't be read: {e}."))?;
    let password = url.password().map(|p| {
        percent_encoding::percent_decode_str(p)
            .decode_utf8_lossy()
            .to_string()
    });
    if let Some(pw) = &password {
        set_secret(&id, pw)?;
        let _ = url.set_password(None);
    }
    let host = url.host_str().unwrap_or("").to_string();
    let db = url.path().trim_start_matches('/').to_string();
    let env = if super::says_prod(&[&host, &db]) {
        Env::Prod
    } else if super::local_host(&host) {
        Env::Local
    } else {
        Env::Staging
    };
    let name = if !db.is_empty() {
        db
    } else {
        host.split('.').next().unwrap_or("database").to_string()
    };
    Ok(Saved {
        id,
        project,
        name,
        kind,
        env,
        shown: redact(url.as_str()),
        source: Source::Url {
            url: url.to_string(),
            password: password.is_some(),
        },
    })
}

/// What to connect to for `saved`, secrets filled in, for the project at `root`.
pub(crate) fn target(saved: &Saved, root: &Path) -> Result<String, String> {
    match &saved.source {
        Source::File { path } => Ok(path.clone()),
        Source::Url { url, password } => {
            let mut url = url::Url::parse(url).map_err(|e| e.to_string())?;
            if *password {
                let pw = get_secret(&saved.id).ok_or_else(|| {
                    "Its password isn't in the Keychain any more. Remove the connection and add it again.".to_string()
                })?;
                let _ = url.set_password(Some(&pw));
            }
            Ok(url.to_string())
        }
        Source::Found { key } => {
            let mut found = detect::find(root);
            found.extend(detect::open_ports(&found));
            found
                .into_iter()
                .find(|f| &f.key == key)
                .map(|f| f.target)
                .ok_or_else(|| {
                    format!(
                        "It isn't in the project's files any more (it came from {}).",
                        key.split(':').nth(1).unwrap_or(key)
                    )
                })
        }
    }
}

pub(crate) fn forget(saved: &Saved) {
    if matches!(saved.source, Source::Url { password: true, .. }) {
        delete_secret(&saved.id);
    }
}

#[cfg(target_os = "macos")]
const SERVICE: &str = "Runo database";

#[cfg(target_os = "macos")]
fn set_secret(id: &str, secret: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(SERVICE, id, secret.as_bytes())
        .map_err(|e| format!("The Keychain refused the password: {e}"))
}

#[cfg(target_os = "macos")]
fn get_secret(id: &str) -> Option<String> {
    security_framework::passwords::get_generic_password(SERVICE, id)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).to_string())
}

#[cfg(target_os = "macos")]
fn delete_secret(id: &str) {
    let _ = security_framework::passwords::delete_generic_password(SERVICE, id);
}

/// Elsewhere, a JSON file of secrets readable only by the person.
#[cfg(not(target_os = "macos"))]
fn secrets_file() -> PathBuf {
    folder().join("secrets.json")
}

#[cfg(not(target_os = "macos"))]
fn secrets() -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(secrets_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn write_secrets(all: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    std::fs::create_dir_all(folder()).map_err(|e| e.to_string())?;
    let path = secrets_file();
    std::fs::write(&path, serde_json::to_string(all).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn set_secret(id: &str, secret: &str) -> Result<(), String> {
    let mut all = secrets();
    all.insert(id.to_string(), secret.to_string());
    write_secrets(&all)
}

#[cfg(not(target_os = "macos"))]
fn get_secret(id: &str) -> Option<String> {
    secrets().get(id).cloned()
}

#[cfg(not(target_os = "macos"))]
fn delete_secret(id: &str) {
    let mut all = secrets();
    if all.remove(id).is_some() {
        let _ = write_secrets(&all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_urls_keep_their_password_apart() {
        let home = std::env::temp_dir().join(format!("runo-saved-{}", std::process::id()));
        TEST_DIR.with_borrow_mut(|d| *d = Some(home.clone()));
        let saved = from_typed(7, "postgres://app:s%40cret@db.internal:5432/shop").unwrap();
        assert_eq!(saved.kind, Kind::Postgres);
        assert_eq!(saved.name, "shop");
        assert_eq!(saved.env, Env::Staging);
        let Source::Url { url, password } = &saved.source else {
            panic!("{saved:?}")
        };
        assert!(*password);
        assert!(!url.contains("cret"));
        assert!(!saved.shown.contains("cret"));
        store(std::slice::from_ref(&saved)).unwrap();
        assert!(!std::fs::read_to_string(file()).unwrap().contains("cret"));
        assert_eq!(load(), vec![saved.clone()]);
        let full = target(&saved, Path::new("/nowhere")).unwrap();
        assert_eq!(full, "postgres://app:s%40cret@db.internal:5432/shop");
        forget(&saved);
        assert!(target(&saved, Path::new("/nowhere")).is_err());

        let prod = from_typed(7, "postgres://u@shop-prod.example.com/shop").unwrap();
        assert_eq!(prod.env, Env::Prod);
        assert!(from_typed(7, "not a url").is_err());
        let _ = std::fs::remove_dir_all(home);
    }
}
