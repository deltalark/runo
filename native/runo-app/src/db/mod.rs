//! Databases for the Data place: the ones a project uses, found by reading its files
//! (`detect`), the connections the person saved (`saved`), and read-only sessions on
//! them (`engine`). Every session is opened read-only, and the database itself enforces
//! that, so nothing the place or an agent runs through it can change data.

pub(crate) mod agent;
pub(crate) mod detect;
pub(crate) mod engine;
pub(crate) mod query;
pub(crate) mod saved;
pub(crate) mod yamlish;

use serde::{Deserialize, Serialize};

/// What kind of database a connection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Postgres,
    Sqlite,
    Mysql,
    Redis,
    Mongo,
}

impl Kind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Postgres => "Postgres",
            Kind::Sqlite => "SQLite",
            Kind::Mysql => "MySQL",
            Kind::Redis => "Redis",
            Kind::Mongo => "MongoDB",
        }
    }

    /// Two letters for the badge beside a connection.
    pub(crate) fn mark(self) -> &'static str {
        match self {
            Kind::Postgres => "PG",
            Kind::Sqlite => "SL",
            Kind::Mysql => "MY",
            Kind::Redis => "RD",
            Kind::Mongo => "MG",
        }
    }

    /// Whether Runo can open it yet. The others are found and listed, so the person sees
    /// them, but open in a later release.
    pub(crate) fn supported(self) -> bool {
        matches!(self, Kind::Postgres | Kind::Sqlite)
    }

    /// The kind a URL's scheme names.
    pub(crate) fn of_url(url: &str) -> Option<Kind> {
        let scheme = url.split_once(':')?.0.to_ascii_lowercase();
        Some(match scheme.as_str() {
            "postgres" | "postgresql" => Kind::Postgres,
            "mysql" | "mariadb" => Kind::Mysql,
            "redis" | "rediss" => Kind::Redis,
            "mongodb" | "mongodb+srv" => Kind::Mongo,
            "sqlite" | "file" => Kind::Sqlite,
            _ => return None,
        })
    }
}

/// Where a database runs, which decides how carefully Runo treats it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Env {
    /// This machine: localhost, a Docker container, a file in the project.
    #[default]
    Local,
    /// Somewhere else, with nothing saying it is production.
    Staging,
    Prod,
}

impl Env {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Env::Local => "LOCAL",
            Env::Staging => "STAGING",
            Env::Prod => "PROD",
        }
    }
}

/// Whether `host` is this machine.
pub(crate) fn local_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    h.is_empty()
        || h == "localhost"
        || h == "::1"
        || h.starts_with("127.")
        || h == "0.0.0.0"
        || h == "host.docker.internal"
}

/// Whether any of `hints` (a file name, a host, a database name) says production.
pub(crate) fn says_prod(hints: &[&str]) -> bool {
    hints.iter().any(|h| {
        let h = h.to_ascii_lowercase();
        h.contains("prod") || h.contains("live")
    })
}

/// A URL with its password taken out, for showing and saving.
pub(crate) fn redact(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) if u.password().is_some() => {
            let _ = u.set_password(Some("•••"));
            u.to_string().replace("%E2%80%A2%E2%80%A2%E2%80%A2", "•••")
        }
        _ => url.to_string(),
    }
}

/// The host, port and database name in a URL, for labels.
pub(crate) fn url_parts(url: &str) -> (String, Option<u16>, String) {
    match url::Url::parse(url) {
        Ok(u) => (
            u.host_str().unwrap_or("").to_string(),
            u.port(),
            u.path().trim_start_matches('/').to_string(),
        ),
        Err(_) => (String::new(), None, String::new()),
    }
}

/// How a cell's value looks, so the grid can colour it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellKind {
    Null,
    Number,
    Bool,
    Text,
    Json,
    Bytes,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cell {
    pub text: String,
    pub kind: CellKind,
}

impl Cell {
    pub(crate) fn null() -> Self {
        Cell {
            text: "NULL".into(),
            kind: CellKind::Null,
        }
    }

    pub(crate) fn text(text: impl Into<String>, kind: CellKind) -> Self {
        Cell {
            text: text.into(),
            kind,
        }
    }
}

/// A column of a table, as the schema describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary: bool,
    /// The table and column this one points at, when it is a foreign key.
    pub refers: Option<(String, String)>,
}

impl Column {
    /// Whether the column holds personal or secret data, so its values stay hidden until
    /// the person asks to see them.
    pub(crate) fn sensitive(&self) -> bool {
        sensitive_name(&self.name)
    }
}

/// Column names whose values are hidden by default.
pub(crate) fn sensitive_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const WORDS: [&str; 12] = [
        "password", "passwd", "secret", "token", "api_key", "apikey", "email", "phone", "ssn",
        "salt", "otp", "hash",
    ];
    WORDS.iter().any(|w| n.contains(w)) && !n.ends_with("_at") && !n.ends_with("_id")
}

/// A column type as the grid's header shows it: Postgres' long names shortened to the
/// ones people type.
pub(crate) fn short_type(data_type: &str) -> String {
    const SHORT: [(&str, &str); 7] = [
        ("timestamp with time zone", "timestamptz"),
        ("timestamp without time zone", "timestamp"),
        ("time with time zone", "timetz"),
        ("time without time zone", "time"),
        ("character varying", "varchar"),
        ("character", "char"),
        ("double precision", "float8"),
    ];
    for (long, short) in SHORT {
        if let Some(rest) = data_type.strip_prefix(long) {
            return format!("{short}{rest}");
        }
    }
    data_type.to_string()
}

/// A hidden value as the grid shows it: an email keeps its first letter and domain, so
/// rows can still be told apart; anything else is dots.
pub(crate) fn mask(text: &str) -> String {
    match text.split_once('@') {
        Some((user, domain)) if !user.is_empty() && domain.contains('.') => {
            format!("{}•••@{domain}", user.chars().next().unwrap_or('•'))
        }
        _ => "••••••".into(),
    }
}

/// A table or view.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Table {
    /// The schema it is in; empty for SQLite.
    pub schema: String,
    pub name: String,
    pub view: bool,
    /// Rows, from the database's statistics; not counted.
    pub estimate: Option<i64>,
    pub columns: Vec<Column>,
}

impl Table {
    /// The name to show: schema-qualified outside the default schema.
    pub(crate) fn label(&self) -> String {
        if self.schema.is_empty() || self.schema == "public" || self.schema == "main" {
            self.name.clone()
        } else {
            format!("{}.{}", self.schema, self.name)
        }
    }
}

/// Some rows of a result.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    /// More rows match than were fetched.
    pub more: bool,
    pub millis: u128,
    /// The statement that ran, to show and to copy.
    pub sql: String,
}

/// A condition on one column, picked in the filter bar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Filter {
    pub column: String,
    pub op: Op,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Eq,
    Ne,
    Gt,
    Lt,
    Contains,
    IsNull,
    NotNull,
}

impl Op {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "≠",
            Op::Gt => ">",
            Op::Lt => "<",
            Op::Contains => "contains",
            Op::IsNull => "is null",
            Op::NotNull => "is not null",
        }
    }

    /// Whether the condition takes a value.
    pub(crate) fn takes_value(self) -> bool {
        !matches!(self, Op::IsNull | Op::NotNull)
    }
}

/// What to fetch from a table: a page of rows, filtered and sorted.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Browse {
    pub filters: Vec<Filter>,
    /// A column and whether it sorts descending.
    pub sort: Option<(String, bool)>,
    pub offset: usize,
}

/// Rows per page in the grid.
pub(crate) const PAGE: usize = 100;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_url_schemes() {
        assert_eq!(Kind::of_url("postgresql://a@b/c"), Some(Kind::Postgres));
        assert_eq!(Kind::of_url("rediss://x:6380"), Some(Kind::Redis));
        assert_eq!(Kind::of_url("mongodb+srv://c.x.net/db"), Some(Kind::Mongo));
        assert_eq!(Kind::of_url("file:./dev.db"), Some(Kind::Sqlite));
        assert_eq!(Kind::of_url("https://example.com"), None);
        assert_eq!(Kind::of_url("plain"), None);
    }

    #[test]
    fn passwords_never_show() {
        let shown = redact("postgres://app:s3cr%40t@db.internal:5432/app");
        assert!(!shown.contains("s3cr"));
        assert!(shown.contains("app:•••@db.internal:5432/app"));
        assert_eq!(redact("postgres://app@db/app"), "postgres://app@db/app");
    }

    #[test]
    fn hosts_and_names_say_where_a_database_runs() {
        assert!(local_host("localhost") && local_host("127.0.0.1") && local_host("[::1]"));
        assert!(!local_host("db.internal"));
        assert!(says_prod(&["botoxy-prod.cluster.rds.amazonaws.com"]));
        assert!(says_prod(&[".env.production"]));
        assert!(!says_prod(&["staging.internal", "app_dev"]));
    }

    #[test]
    fn long_type_names_are_shortened() {
        assert_eq!(short_type("timestamp with time zone"), "timestamptz");
        assert_eq!(short_type("character varying(64)"), "varchar(64)");
        assert_eq!(short_type("character(3)"), "char(3)");
        assert_eq!(short_type("jsonb"), "jsonb");
    }

    #[test]
    fn secret_looking_columns_are_hidden() {
        assert!(sensitive_name("password_hash"));
        assert!(sensitive_name("Email"));
        assert!(sensitive_name("api_token"));
        assert!(!sensitive_name("email_verified_at"));
        assert!(!sensitive_name("token_id"));
        assert!(!sensitive_name("total_cents"));
    }
}
