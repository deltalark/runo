//! Read-only sessions on a database. Postgres sessions set
//! `default_transaction_read_only`, so the server refuses any write; SQLite files are
//! opened read-only with `query_only` on. Both have a statement timeout, and the grid's
//! filters become SQL with every value quoted or bound, never pasted in.

use super::{Browse, Cell, CellKind, Column, Filter, Kind, Op, Rows, Table, PAGE};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one statement may run before the database stops it.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);

/// Longest text kept for one cell; the inspector shows this much.
const CELL_MAX: usize = 4000;

pub(crate) enum Session {
    Postgres(postgres::Client),
    Sqlite(rusqlite::Connection),
}

/// What opening a session learned about the server.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Opened {
    /// "Postgres 16.4" or "SQLite 3.46.0".
    pub server: String,
    /// The server says the session can't write.
    pub read_only: bool,
}

/// `name` as a quoted SQL identifier.
pub(crate) fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `value` as a quoted SQL string literal (standard_conforming_strings on).
pub(crate) fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\0', "").replace('\'', "''"))
}

/// `value` with LIKE's wildcards escaped by backslash.
fn like_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn table_ref(kind: Kind, table: &Table) -> String {
    if kind == Kind::Postgres && !table.schema.is_empty() {
        format!("{}.{}", ident(&table.schema), ident(&table.name))
    } else {
        ident(&table.name)
    }
}

/// The WHERE, ORDER BY and paging of a browse, with values written in as literals (for
/// Postgres, and for showing) or as `?` with the values beside (for SQLite).
pub(crate) fn browse_sql(
    kind: Kind,
    table: &Table,
    b: &Browse,
    bind: bool,
) -> (String, Vec<String>) {
    let mut values = Vec::new();
    let mut put = |v: String| -> String {
        if bind {
            values.push(v);
            "?".into()
        } else {
            literal(&v)
        }
    };
    let mut wheres = Vec::new();
    for Filter { column, op, value } in &b.filters {
        let col = ident(column);
        let cond = match op {
            Op::Eq => format!("{col} = {}", put(value.clone())),
            Op::Ne => format!("{col} <> {}", put(value.clone())),
            Op::Gt => format!("{col} > {}", put(value.clone())),
            Op::Lt => format!("{col} < {}", put(value.clone())),
            Op::Contains => {
                let pattern = put(format!("%{}%", like_escape(value)));
                match kind {
                    Kind::Postgres => format!("{col}::text ILIKE {pattern}"),
                    _ => format!("CAST({col} AS TEXT) LIKE {pattern} ESCAPE '\\'"),
                }
            }
            Op::IsNull => format!("{col} IS NULL"),
            Op::NotNull => format!("{col} IS NOT NULL"),
        };
        wheres.push(cond);
    }
    let mut sql = format!("SELECT * FROM {}", table_ref(kind, table));
    if !wheres.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&wheres.join(" AND "));
    }
    if let Some((column, desc)) = &b.sort {
        sql.push_str(&format!(" ORDER BY {}", ident(column)));
        sql.push_str(if *desc { " DESC" } else { " ASC" });
        if kind == Kind::Postgres {
            sql.push_str(" NULLS LAST");
        }
    }
    sql.push_str(&format!(" LIMIT {} OFFSET {}", PAGE + 1, b.offset));
    (sql, values)
}

/// How a column's values look, from its type name.
pub(crate) fn kind_of_type(data_type: &str) -> CellKind {
    let t = data_type.to_ascii_lowercase();
    if t.starts_with("bool") {
        CellKind::Bool
    } else if t.starts_with("json") {
        CellKind::Json
    } else if t == "bytea" || t == "blob" {
        CellKind::Bytes
    } else if [
        "int", "serial", "numeric", "decimal", "real", "double", "float", "money", "bigint",
        "smallint",
    ]
    .iter()
    .any(|n| t.starts_with(n))
    {
        CellKind::Number
    } else {
        CellKind::Text
    }
}

fn clip(mut text: String) -> String {
    if text.len() > CELL_MAX {
        let mut end = CELL_MAX;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

/// How a Postgres connection uses TLS, following libpq's `sslmode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TlsCheck {
    /// Encrypt when the server offers it, without checking its certificate: libpq's
    /// `prefer` and `require`, and what a database on this machine gets.
    Encrypt,
    /// Encrypt and check the certificate against the system's trusted ones.
    Verify,
}

/// The TLS check for `target` and the URL the driver gets. A database elsewhere is
/// checked unless its URL asks for less; the driver only knows `disable`, `prefer` and
/// `require`, so the stricter modes become `require` with the check on.
pub(crate) fn tls_plan(target: &str) -> (TlsCheck, String) {
    let mode = target
        .split(['?', '&'])
        .find_map(|p| p.strip_prefix("sslmode="))
        .unwrap_or("")
        .to_string();
    let (host, _, _) = super::url_parts(target);
    let check = match mode.as_str() {
        "verify-full" | "verify-ca" => TlsCheck::Verify,
        "disable" | "allow" | "prefer" | "require" => TlsCheck::Encrypt,
        _ if super::local_host(&host) => TlsCheck::Encrypt,
        _ => TlsCheck::Verify,
    };
    let url = match mode.as_str() {
        "verify-full" | "verify-ca" => {
            target.replace(&format!("sslmode={mode}"), "sslmode=require")
        }
        "allow" => target.replace("sslmode=allow", "sslmode=prefer"),
        "" if check == TlsCheck::Verify => {
            let sep = if target.contains('?') { '&' } else { '?' };
            format!("{target}{sep}sslmode=require")
        }
        _ => target.to_string(),
    };
    (check, url)
}

/// Accepts any certificate but still checks the handshake's signatures, as libpq does
/// below `verify-ca`.
#[derive(Debug)]
struct Unchecked(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for Unchecked {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn tls(check: TlsCheck) -> Result<tokio_postgres_rustls::MakeRustlsConnect, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let config = match check {
        TlsCheck::Verify => {
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(cert);
            }
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        TlsCheck::Encrypt => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Unchecked(provider)))
            .with_no_client_auth(),
    };
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

/// A database's own error, in a sentence the place can show.
fn pg_error(e: postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        let mut msg = db.message().to_string();
        if let Some(hint) = db.hint() {
            msg.push_str(". ");
            msg.push_str(hint);
        }
        return msg;
    }
    let text = e.to_string();
    if text.contains("password authentication failed") {
        "The password was refused.".into()
    } else if text.contains("Connection refused") || text.contains("error connecting to server") {
        "Nothing answered at that address. Is the database running?".into()
    } else if text.contains("invalid peer certificate") || text.contains("UnknownIssuer") {
        "The server's TLS certificate isn't trusted by this computer. Add its CA to your system's certificates, or put sslmode=require in the URL to encrypt without checking it.".into()
    } else if text.contains("TLS handshake") {
        "The TLS handshake failed. If the server doesn't use TLS, put sslmode=disable in the URL."
            .into()
    } else {
        text
    }
}

impl Session {
    /// Open a read-only session on the database at `target`: a URL, or a SQLite file's path.
    pub(crate) fn open(kind: Kind, target: &str) -> Result<(Session, Opened), String> {
        match kind {
            Kind::Postgres => {
                let (check, url) = tls_plan(target);
                let mut config: postgres::Config =
                    url.parse().map_err(|e: postgres::Error| e.to_string())?;
                config.application_name("Runo");
                config.connect_timeout(Duration::from_secs(10));
                let mut client = config.connect(tls(check)?).map_err(pg_error)?;
                client
                    .batch_execute(&format!(
                        "SET default_transaction_read_only = on; SET statement_timeout = {}; SET standard_conforming_strings = on;",
                        TIMEOUT.as_millis()
                    ))
                    .map_err(pg_error)?;
                let read_only = client
                    .simple_query("SHOW transaction_read_only")
                    .map_err(pg_error)?
                    .iter()
                    .find_map(|m| match m {
                        postgres::SimpleQueryMessage::Row(r) => r.get(0).map(|v| v == "on"),
                        _ => None,
                    })
                    .unwrap_or(false);
                let version = client
                    .simple_query("SHOW server_version")
                    .ok()
                    .and_then(|ms| {
                        ms.iter().find_map(|m| match m {
                            postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_string),
                            _ => None,
                        })
                    })
                    .unwrap_or_default();
                let version = version.split_whitespace().next().unwrap_or("").to_string();
                Ok((
                    Session::Postgres(client),
                    Opened {
                        server: format!("Postgres {version}").trim().to_string(),
                        read_only,
                    },
                ))
            }
            Kind::Sqlite => {
                use rusqlite::OpenFlags;
                let path = target
                    .trim_start_matches("sqlite:")
                    .trim_start_matches("file:");
                if !std::path::Path::new(path).is_file() {
                    return Err(format!("There's no file at {path}."));
                }
                let conn = rusqlite::Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| e.to_string())?;
                conn.pragma_update(None, "query_only", 1)
                    .map_err(|e| e.to_string())?;
                conn.busy_timeout(Duration::from_secs(3))
                    .map_err(|e| e.to_string())?;
                let read_only = conn
                    .pragma_query_value(None, "query_only", |r| r.get::<_, i64>(0))
                    .map(|v| v == 1)
                    .unwrap_or(false);
                Ok((
                    Session::Sqlite(conn),
                    Opened {
                        server: format!("SQLite {}", rusqlite::version()),
                        read_only,
                    },
                ))
            }
            other => Err(format!("{} isn't supported yet.", other.label())),
        }
    }

    /// Whether the server ended the session: it restarted, or the network dropped it
    /// while the computer slept.
    pub(crate) fn is_closed(&self) -> bool {
        match self {
            Session::Postgres(client) => client.is_closed(),
            Session::Sqlite(_) => false,
        }
    }

    pub(crate) fn kind(&self) -> Kind {
        match self {
            Session::Postgres(_) => Kind::Postgres,
            Session::Sqlite(_) => Kind::Sqlite,
        }
    }

    /// Every table and view with its columns, keys and estimated size.
    pub(crate) fn tables(&mut self) -> Result<Vec<Table>, String> {
        match self {
            Session::Postgres(client) => pg_tables(client),
            Session::Sqlite(conn) => sqlite_tables(conn).map_err(|e| e.to_string()),
        }
    }

    /// One page of a table's rows.
    pub(crate) fn browse(&mut self, table: &Table, b: &Browse) -> Result<Rows, String> {
        let kind = self.kind();
        let (shown, _) = browse_sql(kind, table, b, false);
        let types: HashMap<&str, CellKind> = table
            .columns
            .iter()
            .map(|c| (c.name.as_str(), kind_of_type(&c.data_type)))
            .collect();
        let start = Instant::now();
        let mut rows = match self {
            Session::Postgres(client) => {
                let messages = client.simple_query(&shown).map_err(pg_error)?;
                let mut columns: Vec<String> = Vec::new();
                let mut rows = Vec::new();
                for m in messages {
                    match m {
                        postgres::SimpleQueryMessage::RowDescription(cols) => {
                            columns = cols.iter().map(|c| c.name().to_string()).collect();
                        }
                        postgres::SimpleQueryMessage::Row(r) => {
                            if columns.is_empty() {
                                columns =
                                    r.columns().iter().map(|c| c.name().to_string()).collect();
                            }
                            let cells = (0..r.len())
                                .map(|i| match r.get(i) {
                                    None => Cell::null(),
                                    Some(v) => {
                                        let kind = columns
                                            .get(i)
                                            .and_then(|n| types.get(n.as_str()))
                                            .copied()
                                            .unwrap_or(CellKind::Text);
                                        let text = if kind == CellKind::Bool {
                                            match v {
                                                "t" => "true".into(),
                                                "f" => "false".into(),
                                                v => v.to_string(),
                                            }
                                        } else {
                                            v.to_string()
                                        };
                                        Cell::text(clip(text), kind)
                                    }
                                })
                                .collect();
                            rows.push(cells);
                        }
                        _ => {}
                    }
                }
                if columns.is_empty() {
                    columns = table.columns.iter().map(|c| c.name.clone()).collect();
                }
                Rows {
                    columns,
                    rows,
                    ..Rows::default()
                }
            }
            Session::Sqlite(conn) => {
                let (sql, values) = browse_sql(kind, table, b, true);
                sqlite_rows(conn, &sql, &values, &types).map_err(|e| e.to_string())?
            }
        };
        rows.millis = start.elapsed().as_millis();
        rows.more = rows.rows.len() > PAGE;
        rows.rows.truncate(PAGE);
        rows.sql = shown;
        Ok(rows)
    }
}

fn pg_rows(client: &mut postgres::Client, sql: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    Ok(client
        .simple_query(sql)
        .map_err(pg_error)?
        .into_iter()
        .filter_map(|m| match m {
            postgres::SimpleQueryMessage::Row(r) => {
                Some((0..r.len()).map(|i| r.get(i).map(str::to_string)).collect())
            }
            _ => None,
        })
        .collect())
}

const PG_SKIP: &str = "n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%'";

fn pg_tables(client: &mut postgres::Client) -> Result<Vec<Table>, String> {
    let rels = pg_rows(
        client,
        &format!(
            "SELECT n.nspname, c.relname, c.relkind, c.reltuples::bigint FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r','p','v','m','f') AND {PG_SKIP} ORDER BY n.nspname <> 'public', n.nspname, c.relname"
        ),
    )?;
    let cols = pg_rows(
        client,
        &format!(
            "SELECT n.nspname, c.relname, a.attname, format_type(a.atttypid, a.atttypmod), (NOT a.attnotnull)::text FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r','p','v','m','f') AND {PG_SKIP} ORDER BY n.nspname, c.relname, a.attnum"
        ),
    )?;
    let keys = pg_rows(
        client,
        &format!(
            "SELECT n.nspname, c.relname, con.contype::text, a.attname, fn.nspname, fc.relname, fa.attname FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum LEFT JOIN pg_class fc ON fc.oid = con.confrelid LEFT JOIN pg_namespace fn ON fn.oid = fc.relnamespace LEFT JOIN pg_attribute fa ON fa.attrelid = con.confrelid AND fa.attnum = con.confkey[k.ord] WHERE con.contype IN ('p','f') AND {PG_SKIP}"
        ),
    )?;
    let s = |v: &Option<String>| v.clone().unwrap_or_default();
    let mut tables: Vec<Table> = rels
        .iter()
        .map(|r| Table {
            schema: s(&r[0]),
            name: s(&r[1]),
            view: matches!(r[2].as_deref(), Some("v" | "m")),
            estimate: r[3]
                .as_deref()
                .and_then(|v| v.parse().ok())
                .filter(|n: &i64| *n >= 0),
            columns: Vec::new(),
        })
        .collect();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for (i, t) in tables.iter().enumerate() {
        index.insert((t.schema.clone(), t.name.clone()), i);
    }
    for r in &cols {
        if let Some(&i) = index.get(&(s(&r[0]), s(&r[1]))) {
            tables[i].columns.push(Column {
                name: s(&r[2]),
                data_type: s(&r[3]),
                nullable: r[4].as_deref() == Some("true"),
                ..Column::default()
            });
        }
    }
    for r in &keys {
        let Some(&i) = index.get(&(s(&r[0]), s(&r[1]))) else {
            continue;
        };
        let Some(col) = tables[i].columns.iter_mut().find(|c| c.name == s(&r[3])) else {
            continue;
        };
        match r[2].as_deref() {
            Some("p") => col.primary = true,
            Some("f") => {
                let schema = s(&r[4]);
                let name = if schema == "public" || schema.is_empty() {
                    s(&r[5])
                } else {
                    format!("{schema}.{}", s(&r[5]))
                };
                col.refers = Some((name, s(&r[6])));
            }
            _ => {}
        }
    }
    Ok(tables)
}

fn sqlite_tables(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<Table>> {
    let mut stmt = conn.prepare(
        "SELECT name, type FROM sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let file_size = conn
        .path()
        .and_then(|p| std::fs::metadata(p).ok())
        .map_or(u64::MAX, |m| m.len());
    let mut tables = Vec::new();
    for (name, kind) in names {
        let mut columns: Vec<Column> = conn
            .prepare(&format!("PRAGMA table_info({})", ident(&name)))?
            .query_map([], |r| {
                Ok(Column {
                    name: r.get(1)?,
                    data_type: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    nullable: r.get::<_, i64>(3)? == 0,
                    primary: r.get::<_, i64>(5)? > 0,
                    refers: None,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let fks: Vec<(String, String, Option<String>)> = conn
            .prepare(&format!("PRAGMA foreign_key_list({})", ident(&name)))?
            .query_map([], |r| Ok((r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (to_table, from, to) in fks {
            if let Some(c) = columns.iter_mut().find(|c| c.name == from) {
                c.refers = Some((to_table, to.unwrap_or_else(|| "rowid".into())));
            }
        }
        // SQLite keeps no row counts; count small files, where it's quick.
        let estimate = if kind == "table" && file_size < 64 << 20 {
            conn.query_row(&format!("SELECT count(*) FROM {}", ident(&name)), [], |r| {
                r.get(0)
            })
            .ok()
        } else {
            None
        };
        tables.push(Table {
            schema: String::new(),
            name,
            view: kind == "view",
            estimate,
            columns,
        });
    }
    Ok(tables)
}

fn sqlite_rows(
    conn: &rusqlite::Connection,
    sql: &str,
    values: &[String],
    types: &HashMap<&str, CellKind>,
) -> rusqlite::Result<Rows> {
    use rusqlite::types::ValueRef;
    let mut stmt = conn.prepare(sql)?;
    let columns: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let mut out = Vec::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(values.iter()))?;
    while let Some(row) = rows.next()? {
        let mut cells = Vec::with_capacity(columns.len());
        for (i, name) in columns.iter().enumerate() {
            let declared = types.get(name.as_str()).copied().unwrap_or(CellKind::Text);
            cells.push(match row.get_ref(i)? {
                ValueRef::Null => Cell::null(),
                ValueRef::Integer(n) if declared == CellKind::Bool => {
                    Cell::text(if n == 0 { "false" } else { "true" }, CellKind::Bool)
                }
                ValueRef::Integer(n) => Cell::text(n.to_string(), CellKind::Number),
                ValueRef::Real(f) => Cell::text(f.to_string(), CellKind::Number),
                ValueRef::Text(t) => {
                    let text = String::from_utf8_lossy(t).to_string();
                    let kind = if declared == CellKind::Json
                        || (text.starts_with('{') && text.ends_with('}'))
                    {
                        CellKind::Json
                    } else {
                        CellKind::Text
                    };
                    Cell::text(clip(text), kind)
                }
                ValueRef::Blob(b) => Cell::text(format!("{} bytes", b.len()), CellKind::Bytes),
            });
        }
        out.push(cells);
    }
    Ok(Rows {
        columns,
        rows: out,
        ..Rows::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        Table {
            schema: "public".into(),
            name: "orders".into(),
            ..Table::default()
        }
    }

    #[test]
    fn filters_quote_every_value_and_name() {
        let b = Browse {
            filters: vec![
                Filter {
                    column: "status".into(),
                    op: Op::Eq,
                    value: "x'; DROP TABLE orders; --".into(),
                },
                Filter {
                    column: "we\"ird".into(),
                    op: Op::Contains,
                    value: "50%_off".into(),
                },
                Filter {
                    column: "paid_at".into(),
                    op: Op::IsNull,
                    value: String::new(),
                },
            ],
            sort: Some(("created_at".into(), true)),
            offset: 200,
        };
        let (sql, values) = browse_sql(Kind::Postgres, &table(), &b, false);
        assert_eq!(
            sql,
            "SELECT * FROM \"public\".\"orders\" WHERE \"status\" = 'x''; DROP TABLE orders; --' AND \"we\"\"ird\"::text ILIKE '%50\\%\\_off%' AND \"paid_at\" IS NULL ORDER BY \"created_at\" DESC NULLS LAST LIMIT 101 OFFSET 200"
        );
        assert!(values.is_empty());
        let (sql, values) = browse_sql(Kind::Sqlite, &table(), &b, true);
        assert!(sql.starts_with("SELECT * FROM \"orders\" WHERE \"status\" = ? AND CAST(\"we\"\"ird\" AS TEXT) LIKE ? ESCAPE '\\'"));
        assert_eq!(values, vec!["x'; DROP TABLE orders; --", "%50\\%\\_off%"]);
    }

    fn sample_db() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("runo-engine-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT NOT NULL);
             CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id), status TEXT, total REAL, paid BOOLEAN, meta JSON);
             INSERT INTO customers VALUES (1, 'a@x.dev'), (2, 'b@x.dev');
             INSERT INTO orders VALUES (1, 1, 'paid', 9.5, 1, '{\"n\":1}'), (2, 1, 'failed', 12, 0, NULL), (3, 2, '50% off', 3, 1, NULL);
             CREATE VIEW failed AS SELECT * FROM orders WHERE status = 'failed';",
        )
        .unwrap();
        path
    }

    #[test]
    fn sqlite_sessions_read_and_never_write() {
        let path = sample_db();
        let (mut s, opened) = Session::open(Kind::Sqlite, path.to_str().unwrap()).unwrap();
        assert!(opened.read_only);
        assert!(opened.server.starts_with("SQLite 3."));
        let tables = s.tables().unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["customers", "failed", "orders"]);
        let orders = tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(orders.estimate, Some(3));
        assert!(orders.columns[0].primary);
        assert_eq!(
            orders.columns[1].refers,
            Some(("customers".into(), "id".into()))
        );
        assert!(tables.iter().find(|t| t.name == "failed").unwrap().view);

        let rows = s
            .browse(
                orders,
                &Browse {
                    filters: vec![Filter {
                        column: "status".into(),
                        op: Op::Contains,
                        value: "%".into(),
                    }],
                    ..Browse::default()
                },
            )
            .unwrap();
        assert_eq!(rows.rows.len(), 1, "only the row with a real % matches");
        assert_eq!(rows.rows[0][3], Cell::text("3", CellKind::Number));

        let rows = s
            .browse(
                orders,
                &Browse {
                    sort: Some(("total".into(), true)),
                    ..Browse::default()
                },
            )
            .unwrap();
        assert_eq!(rows.columns[0], "id");
        assert_eq!(rows.rows[0][0].text, "2");
        assert_eq!(rows.rows[0][4], Cell::text("false", CellKind::Bool));
        assert_eq!(rows.rows[1][5].kind, CellKind::Json);
        assert_eq!(rows.rows[0][5], Cell::null());
        assert!(!rows.more);

        let Session::Sqlite(conn) = &s else {
            unreachable!()
        };
        assert!(conn.execute("DELETE FROM orders", []).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn certificates_are_checked_away_from_this_machine() {
        let (check, url) = tls_plan("postgres://u@localhost/app");
        assert_eq!(
            (check, url.as_str()),
            (TlsCheck::Encrypt, "postgres://u@localhost/app")
        );
        let (check, url) = tls_plan("postgres://u@db.example.com/app");
        assert_eq!(check, TlsCheck::Verify);
        assert_eq!(url, "postgres://u@db.example.com/app?sslmode=require");
        let (check, url) = tls_plan("postgres://u@db.example.com/app?sslmode=require");
        assert_eq!(
            (check, url.as_str()),
            (
                TlsCheck::Encrypt,
                "postgres://u@db.example.com/app?sslmode=require"
            )
        );
        let (check, url) = tls_plan("postgres://u@localhost/app?x=1&sslmode=verify-full");
        assert_eq!(
            (check, url.as_str()),
            (
                TlsCheck::Verify,
                "postgres://u@localhost/app?x=1&sslmode=require"
            )
        );
        let (_, url) = tls_plan("postgres://u@db.example.com/app?connect_timeout=5");
        assert!(url.ends_with("connect_timeout=5&sslmode=require"));
    }

    #[test]
    fn types_decide_how_cells_look() {
        assert_eq!(kind_of_type("bigint"), CellKind::Number);
        assert_eq!(kind_of_type("numeric(10,2)"), CellKind::Number);
        assert_eq!(kind_of_type("jsonb"), CellKind::Json);
        assert_eq!(kind_of_type("boolean"), CellKind::Bool);
        assert_eq!(kind_of_type("timestamp with time zone"), CellKind::Text);
        assert_eq!(kind_of_type("character varying(255)"), CellKind::Text);
    }

    /// Runs against a real server when `RUNO_TEST_PG` holds its URL.
    #[test]
    fn postgres_sessions_are_read_only() {
        let Ok(url) = std::env::var("RUNO_TEST_PG") else {
            return;
        };
        let (mut s, opened) = Session::open(Kind::Postgres, &url).unwrap();
        assert!(opened.read_only, "{opened:?}");
        let Session::Postgres(client) = &mut s else {
            unreachable!()
        };
        let err = client
            .batch_execute("CREATE TABLE runo_should_not_exist (id int)")
            .unwrap_err();
        assert!(pg_error(err).contains("read-only"));
        let tables = s.tables().unwrap();
        if let Some(t) = tables.iter().find(|t| !t.view) {
            s.browse(t, &Browse::default()).unwrap();
        }
    }
}
