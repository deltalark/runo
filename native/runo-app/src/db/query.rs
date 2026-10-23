//! SQL that a person or an agent writes, run so that it can only read: one statement, of
//! a kind that reads, inside a read-only transaction that is always rolled back, with at
//! most `limit` rows fetched through a cursor. Here too: the plan check that warns before
//! a big scan on production, exact counts, column statistics, and CSV export.

use super::engine::{
    conditions, ident, kind_of_type, literal, pg_error, pg_rows, pg_text_rows, sqlite_cell,
    table_ref, tls, where_sql, Session, TlsCheck,
};
use super::{mask, sensitive_name, Browse, Cell, CellKind, Column, Kind, Rows, Table};
use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

/// Rows a query shows; the rest wait for an export.
pub(crate) const MAX_ROWS: usize = 1000;
/// Rows an export writes at most.
pub(crate) const EXPORT_MAX: usize = 1_000_000;
/// A full read of a table with at least this many rows is worth a warning on production.
pub(crate) const BIG_SCAN: i64 = 100_000;

/// What a statement may start with: the kinds that only read.
const READS_PG: [&str; 6] = ["select", "with", "values", "table", "explain", "show"];
const READS_SQLITE: [&str; 4] = ["select", "with", "values", "explain"];

/// Functions that act outside the transaction, or on the server itself, so rolling back
/// wouldn't undo them: stopping other sessions, reading the server's files, holding locks
/// past the transaction, reaching other databases or the network, running SQL passed as
/// text, changing settings or statistics.
const BLOCKED: [&str; 26] = [
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_signal_backend",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_promote",
    "pg_switch_wal",
    "pg_notify",
    "pg_stat_file",
    "pg_file_write",
    "pg_file_rename",
    "pg_file_unlink",
    "pg_log_backend_memory_contexts",
    "pg_import_system_collations",
    "set_config",
    "query_to_xml",
    "query_to_xmlschema",
    "query_to_xml_and_xmlschema",
    "cursor_to_xml",
    "cursor_to_xmlschema",
    "txid_current",
    "pg_current_xact_id",
    "load_extension",
    "readfile",
    "writefile",
    "fts3_tokenizer",
];

/// Families of such functions, by how their names start: files and large objects,
/// other servers (dblink, the http and pg_net extensions), locks, replication and
/// backups, statistics resets.
const BLOCKED_PREFIXES: [&str; 18] = [
    "pg_read_",
    "pg_ls_",
    "lo_",
    "dblink",
    "http",
    "net.",
    "pg_advisory_",
    "pg_try_advisory_",
    "pg_stat_reset",
    "pg_create_",
    "pg_drop_",
    "pg_replication_",
    "pg_logical_",
    "pg_copy_",
    "pg_wal_replay_",
    "pg_backup_",
    "pg_start_backup",
    "pg_stop_backup",
];

/// Whether calling `name` is refused.
fn blocked(name: &str) -> bool {
    BLOCKED.contains(&name) || BLOCKED_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Punct(char),
    Text,
}

/// The statement's words and punctuation, with strings and comments skipped, each with
/// the byte where it ends. Quoted names count as words, so `"pg_read_file"(` is caught.
fn tokens(sql: &str) -> Vec<(Tok, usize)> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let word_byte = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == b'\'' {
            let escapes = i > 0
                && matches!(b[i - 1], b'e' | b'E')
                && !word_byte(*b.get(i.wrapping_sub(2)).unwrap_or(&b' '));
            i += 1;
            while i < b.len() {
                if escapes && b[i] == b'\\' {
                    i += 2;
                } else if b[i] == b'\'' {
                    if b.get(i + 1) == Some(&b'\'') {
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push((Tok::Text, i.min(b.len())));
        } else if c == b'"' || c == b'`' || c == b'[' {
            let close = if c == b'[' { b']' } else { c };
            let start = i + 1;
            i += 1;
            while i < b.len() && !(b[i] == close && b.get(i + 1) != Some(&close)) {
                i += if b[i] == close { 2 } else { 1 };
            }
            let name = String::from_utf8_lossy(&b[start..i.min(b.len())]).to_lowercase();
            i += 1;
            out.push((Tok::Word(name), i.min(b.len())));
        } else if c == b'$' {
            // `$tag$ … $tag$` is a string; `$1` is a parameter.
            let tag_end = (i + 1..b.len()).find(|&j| !word_byte(b[j]) || b[j] >= 0x80);
            match tag_end {
                Some(j)
                    if b[j] == b'$' && !b[i + 1..j].iter().any(u8::is_ascii_digit)
                        || tag_end == Some(i + 1) && b.get(i + 1) == Some(&b'$') =>
                {
                    let tag = &sql[i..=j];
                    let body = j + 1;
                    let end = sql[body..]
                        .find(tag)
                        .map_or(b.len(), |k| body + k + tag.len());
                    out.push((Tok::Text, end));
                    i = end;
                }
                _ => {
                    let end = tag_end.unwrap_or(b.len());
                    out.push((Tok::Word(sql[i..end].to_string()), end));
                    i = end;
                }
            }
        } else if word_byte(c) {
            let start = i;
            while i < b.len() && word_byte(b[i]) {
                i += 1;
            }
            out.push((Tok::Word(sql[start..i].to_lowercase()), i));
        } else {
            i += 1;
            out.push((Tok::Punct(c as char), i));
        }
    }
    out
}

/// The statement's first keyword, lowercase.
pub(crate) fn first_word(sql: &str) -> String {
    tokens(sql)
        .into_iter()
        .find_map(|(t, _)| match t {
            Tok::Word(w) => Some(w),
            _ => None,
        })
        .unwrap_or_default()
}

/// `sql` if it's one statement that only reads, trimmed of trailing semicolons and
/// comments; otherwise why it won't run.
pub(crate) fn check(kind: Kind, sql: &str) -> Result<String, String> {
    let toks = tokens(sql);
    let mut statements = toks
        .split(|(t, _)| *t == Tok::Punct(';'))
        .filter(|s| !s.is_empty());
    let Some(statement) = statements.next() else {
        return Err("Write a query first.".into());
    };
    if statements.next().is_some() {
        return Err("Run one statement at a time. Runo runs a single statement, so nothing can slip past its read-only check.".into());
    }
    let words: Vec<(usize, &str)> = statement
        .iter()
        .enumerate()
        .filter_map(|(i, (t, _))| match t {
            Tok::Word(w) => Some((i, w.as_str())),
            _ => None,
        })
        .collect();
    let first = words.first().map_or("", |(_, w)| *w);
    let reads: &[&str] = match kind {
        Kind::Postgres => &READS_PG,
        _ => &READS_SQLITE,
    };
    let refuse = |word: &str| {
        format!(
            "Runo only runs statements that read here: {}. A statement starting with {} could change something.",
            reads
                .iter()
                .map(|r| r.to_uppercase())
                .collect::<Vec<_>>()
                .join(", "),
            word.to_uppercase()
        )
    };
    if !reads.contains(&first) {
        return Err(refuse(first));
    }
    // EXPLAIN ANALYZE runs what it explains, so that has to read too.
    if first == "explain" {
        let options = ["analyze", "analyse", "verbose", "query", "plan"];
        let inner = statement
            .iter()
            .skip(1)
            .scan(0i32, |depth, (t, _)| {
                let before = *depth;
                match t {
                    Tok::Punct('(') => *depth += 1,
                    Tok::Punct(')') => *depth -= 1,
                    _ => {}
                }
                Some((before, t))
            })
            .find_map(|(depth, t)| match t {
                Tok::Word(w) if depth == 0 && !options.contains(&w.as_str()) => Some(w.as_str()),
                _ => None,
            })
            .unwrap_or("");
        if !reads.contains(&inner) || matches!(inner, "explain" | "show") {
            return Err(refuse(inner));
        }
    }
    for (n, &(i, w)) in words.iter().enumerate() {
        let after = |k: usize| words.get(n + k).map_or("", |(_, w)| *w);
        let before = i.checked_sub(1).map(|j| &statement[j].0);
        // A write inside WITH: `WITH gone AS (DELETE FROM … RETURNING *) SELECT …`.
        if matches!(w, "insert" | "update" | "delete" | "merge" | "truncate")
            && before == Some(&Tok::Punct('('))
        {
            return Err(format!(
                "That statement has {} inside it, which writes. Runo only runs statements that read.",
                w.to_uppercase()
            ));
        }
        // SELECT … INTO makes a table.
        if w == "into" {
            return Err("SELECT … INTO creates a table, so Runo doesn't run it.".into());
        }
        // Row locks would hold up the app's own writes.
        if w == "for" && matches!(after(1), "update" | "share" | "no" | "key") {
            return Err("FOR UPDATE and FOR SHARE lock rows, so Runo doesn't run them.".into());
        }
        // U&"\0070g_…" spells a name in escapes, which would hide it from the checks
        // below.
        if w == "u"
            && matches!(statement.get(i + 1), Some((Tok::Punct('&'), end)) if *end == statement[i].1 + 1)
        {
            return Err(
                "Names written with U& escapes aren't run here. Write the name plainly.".into(),
            );
        }
    }
    for pair in statement.windows(2) {
        if let (Tok::Word(name), Tok::Punct('(')) = (&pair[0].0, &pair[1].0) {
            if blocked(name) {
                return Err(format!(
                    "{name}() acts outside the read-only transaction, so Runo doesn't run it."
                ));
            }
        }
    }
    // Schema-qualified calls of the network extensions: `net.http_get(…)`.
    for triple in statement.windows(4) {
        if let (Tok::Word(schema), Tok::Punct('.'), Tok::Word(name), Tok::Punct('(')) =
            (&triple[0].0, &triple[1].0, &triple[2].0, &triple[3].0)
        {
            if blocked(&format!("{schema}.{name}")) {
                return Err(format!(
                    "{schema}.{name}() reaches outside the database, so Runo doesn't run it."
                ));
            }
        }
    }
    let end = statement.last().map_or(0, |(_, end)| *end);
    Ok(sql[..end].trim().to_string())
}

/// A statement's result.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ran {
    pub rows: Rows,
    /// Each column's type, as the database names it.
    pub types: Vec<String>,
    /// The table column each result column comes straight from, when it does: (table,
    /// column). An expression has none.
    pub origins: Vec<Option<(String, String)>>,
    /// Columns whose values stay masked (see `hidden_columns`).
    pub hidden: Vec<bool>,
}

/// Which of a result's columns to mask. Going by the result's own column names alone, a
/// rename (`email AS contact`) or an expression (`upper(email)`) would show what the table
/// hides, so a column is masked when it comes from a hidden column, has a hidden column's
/// name, or is text worked out from a statement that reads a table with hidden columns.
/// Numbers and true/false from such expressions (counts, lengths) still show.
pub(crate) fn hidden_columns(ran: &Ran, sql: &str, tables: &[Table]) -> Vec<bool> {
    let words: HashSet<String> = tokens(sql)
        .into_iter()
        .filter_map(|(t, _)| match t {
            Tok::Word(w) => Some(w),
            _ => None,
        })
        .collect();
    let touches = tables
        .iter()
        .filter(|t| words.contains(&t.name.to_lowercase()))
        .any(|t| t.columns.iter().any(|c| c.sensitive()));
    ran.rows
        .columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if sensitive_name(name) {
                return true;
            }
            match ran.origins.get(i).cloned().flatten() {
                Some((_, column)) => sensitive_name(&column),
                None => {
                    touches
                        && !ran.rows.rows.iter().all(|r| {
                            r.get(i).is_none_or(|c| {
                                matches!(c.kind, CellKind::Null | CellKind::Number | CellKind::Bool)
                            })
                        })
                }
            }
        })
        .collect()
}

/// Stops a statement that is running on a session, from another thread.
pub(crate) enum Canceller {
    Postgres(postgres::CancelToken, TlsCheck),
    Sqlite(rusqlite::InterruptHandle),
}

impl Canceller {
    pub(crate) fn cancel(&self) {
        match self {
            Canceller::Postgres(token, check) => {
                if let Ok(tls) = tls(*check) {
                    let _ = token.cancel_query(tls);
                }
            }
            Canceller::Sqlite(handle) => handle.interrupt(),
        }
    }
}

/// Run `f` inside a read-only transaction and roll it back, whatever happened.
pub(super) fn read_only<T>(
    client: &mut postgres::Client,
    f: impl FnOnce(&mut postgres::Client) -> Result<T, String>,
) -> Result<T, String> {
    client
        .batch_execute("BEGIN TRANSACTION READ ONLY")
        .map_err(pg_error)?;
    let out = f(client);
    let rolled = client.batch_execute("ROLLBACK").map_err(pg_error);
    let out = out?;
    rolled?;
    Ok(out)
}

/// Whether a cursor can hold the statement, so rows come back a batch at a time.
fn cursor_kind(sql: &str) -> bool {
    matches!(
        first_word(sql).as_str(),
        "select" | "with" | "values" | "table"
    )
}

/// Shortened counts for sentences: 2.3M, 120k, 950.
pub(crate) fn about(n: i64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{}k", n / 1000),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

impl Session {
    /// The thing to call to stop what this session is running.
    pub(crate) fn canceller(&self) -> Canceller {
        match self {
            Session::Postgres(client, check) => Canceller::Postgres(client.cancel_token(), *check),
            Session::Sqlite(conn) => Canceller::Sqlite(conn.get_interrupt_handle()),
        }
    }

    /// Run one statement that only reads; at most `limit` rows come back.
    pub(crate) fn run(&mut self, sql: &str, limit: usize) -> Result<Ran, String> {
        let kind = self.kind();
        let sql = check(kind, sql)?;
        let start = Instant::now();
        let mut ran = match self {
            Session::Postgres(client, _) => pg_run(client, &sql, limit)?,
            Session::Sqlite(conn) => sqlite_run(conn, &sql, limit)?,
        };
        ran.rows.millis = start.elapsed().as_millis();
        ran.rows.more = ran.rows.rows.len() > limit;
        ran.rows.rows.truncate(limit);
        ran.rows.sql = sql;
        Ok(ran)
    }

    /// Why `sql` would weigh on this database, if it would: it reads every row of a big
    /// table because no index narrows it down. Postgres only; it asks for the plan, which
    /// runs nothing.
    pub(crate) fn scan_warning(
        &mut self,
        sql: &str,
        tables: &[Table],
    ) -> Result<Option<String>, String> {
        let Session::Postgres(client, _) = self else {
            return Ok(None);
        };
        if matches!(first_word(sql).as_str(), "explain" | "show") {
            return Ok(None);
        }
        let sql = check(Kind::Postgres, sql)?;
        let messages = read_only(client, |c| {
            c.simple_query(&format!("EXPLAIN (FORMAT JSON) {sql}"))
                .map_err(pg_error)
        })?;
        let text = messages
            .iter()
            .find_map(|m| match m {
                postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_string),
                _ => None,
            })
            .unwrap_or_default();
        let plan: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        Ok(plan_warning(&plan[0]["Plan"], tables))
    }

    /// How many rows match the browse's filters, counted.
    pub(crate) fn count(&mut self, table: &Table, b: &Browse) -> Result<i64, String> {
        let kind = self.kind();
        let (sql, values) = count_sql(kind, table, b);
        match self {
            Session::Postgres(client, _) => {
                let rows = read_only(client, |c| pg_rows(c, &sql))?;
                rows.first()
                    .and_then(|r| r.first().cloned().flatten())
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| "The count didn't come back.".to_string())
            }
            Session::Sqlite(conn) => conn
                .query_row(&sql, rusqlite::params_from_iter(values.iter()), |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string()),
        }
    }

    /// The database's own estimates for the table's columns, without reading it:
    /// Postgres's statistics. SQLite keeps none, so it gives nothing.
    pub(crate) fn estimated_stats(&mut self, table: &Table) -> Result<Vec<ColumnStat>, String> {
        let Session::Postgres(client, _) = self else {
            return Ok(Vec::new());
        };
        let schema = if table.schema.is_empty() {
            "public"
        } else {
            &table.schema
        };
        let rows = pg_rows(
            client,
            &format!(
                "SELECT attname, null_frac::text, n_distinct::text, most_common_vals::text FROM pg_stats WHERE schemaname = {} AND tablename = {}",
                literal(schema),
                literal(&table.name)
            ),
        )?;
        let total = table.estimate.unwrap_or(0) as f64;
        Ok(rows
            .into_iter()
            .map(|r| {
                let name = r[0].clone().unwrap_or_default();
                let distinct = r[2]
                    .as_deref()
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|n| if n < 0. { (-n * total).round() } else { n });
                let common = if sensitive_name(&name) {
                    Vec::new()
                } else {
                    pg_array(r[3].as_deref().unwrap_or(""))
                        .into_iter()
                        .take(3)
                        .collect()
                };
                ColumnStat {
                    nulls: r[1].as_deref().and_then(|v| v.parse().ok()),
                    distinct,
                    common,
                    name,
                    ..ColumnStat::default()
                }
            })
            .collect())
    }

    /// One column counted exactly over the rows the browse's filters keep: how many are
    /// empty, how many distinct values, and the smallest and largest.
    pub(crate) fn exact_stats(
        &mut self,
        table: &Table,
        column: &Column,
        b: &Browse,
    ) -> Result<ColumnStat, String> {
        let kind = self.kind();
        let col = ident(&column.name);
        let as_text = match kind {
            Kind::Postgres => format!("{col}::text"),
            _ => format!("CAST({col} AS TEXT)"),
        };
        let t = column.data_type.to_ascii_lowercase();
        let ordered = kind_of_type(&t) == CellKind::Number
            || t.contains("date")
            || t.contains("time")
            || kind == Kind::Sqlite;
        let shown = ordered && !column.sensitive();
        let range = match (shown, kind) {
            (false, _) => String::new(),
            (true, Kind::Postgres) => format!(", min({col})::text, max({col})::text"),
            (true, _) => format!(", CAST(min({col}) AS TEXT), CAST(max({col}) AS TEXT)"),
        };
        let (wheres, values) = conditions(kind, &b.filters, kind == Kind::Sqlite);
        let sql = format!(
            "SELECT count(*), count({col}), count(DISTINCT {as_text}){range} FROM {}{}",
            table_ref(kind, table),
            where_sql(&wheres)
        );
        let row: Vec<Option<String>> = match self {
            Session::Postgres(client, _) => read_only(client, |c| pg_rows(c, &sql))?
                .into_iter()
                .next()
                .unwrap_or_default(),
            Session::Sqlite(conn) => conn
                .query_row(&sql, rusqlite::params_from_iter(values.iter()), |r| {
                    (0..r.as_ref().column_count())
                        .map(|i| {
                            r.get::<_, rusqlite::types::Value>(i).map(|v| match v {
                                rusqlite::types::Value::Null => None,
                                rusqlite::types::Value::Integer(n) => Some(n.to_string()),
                                rusqlite::types::Value::Real(f) => Some(f.to_string()),
                                rusqlite::types::Value::Text(s) => Some(s),
                                rusqlite::types::Value::Blob(b) => {
                                    Some(format!("{} bytes", b.len()))
                                }
                            })
                        })
                        .collect()
                })
                .map_err(|e| e.to_string())?,
        };
        let num = |i: usize| -> Option<f64> { row.get(i).cloned().flatten()?.parse().ok() };
        let total = num(0).unwrap_or(0.);
        let filled = num(1).unwrap_or(0.);
        Ok(ColumnStat {
            name: column.name.clone(),
            rows: Some(total as i64),
            nulls: (total > 0.).then(|| (total - filled) / total),
            distinct: num(2),
            common: Vec::new(),
            min: row.get(3).cloned().flatten(),
            max: row.get(4).cloned().flatten(),
            exact: true,
        })
    }

    /// Write every row `sql` returns, up to `EXPORT_MAX`, to `path` as CSV, with the
    /// columns named in `hidden` masked. Returns how many rows were written.
    pub(crate) fn export_csv(
        &mut self,
        sql: &str,
        path: &Path,
        hidden: &HashSet<String>,
    ) -> Result<usize, String> {
        let kind = self.kind();
        let sql = check(kind, sql)?;
        let mut out = std::io::BufWriter::new(private_file(path).map_err(|e| e.to_string())?);
        let mut written = 0;
        let put = |out: &mut std::io::BufWriter<std::fs::File>,
                   fields: Vec<String>|
         -> Result<(), String> {
            let line = fields
                .iter()
                .map(|f| csv_field(f))
                .collect::<Vec<_>>()
                .join(",");
            writeln!(out, "{line}").map_err(|e| e.to_string())
        };
        match self {
            Session::Postgres(client, _) => {
                let names: Vec<String> = client
                    .prepare(&sql)
                    .map_err(pg_error)?
                    .columns()
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect();
                put(&mut out, names.clone())?;
                let masked: Vec<bool> = names.iter().map(|n| hidden.contains(n)).collect();
                read_only(client, |c| {
                    let cursor = cursor_kind(&sql);
                    if cursor {
                        c.batch_execute(&format!("DECLARE runo_export NO SCROLL CURSOR FOR {sql}"))
                            .map_err(pg_error)?;
                    }
                    loop {
                        let batch = if cursor {
                            c.simple_query("FETCH FORWARD 5000 FROM runo_export")
                        } else {
                            c.simple_query(&sql)
                        }
                        .map_err(pg_error)?;
                        let mut got = 0;
                        for m in batch {
                            if let postgres::SimpleQueryMessage::Row(r) = m {
                                got += 1;
                                let fields = (0..r.len())
                                    .map(|i| match r.get(i) {
                                        None => String::new(),
                                        Some(v) if masked.get(i) == Some(&true) => mask(v),
                                        Some(v) => v.to_string(),
                                    })
                                    .collect();
                                put(&mut out, fields)?;
                                written += 1;
                            }
                        }
                        if !cursor || got == 0 || written >= EXPORT_MAX {
                            return Ok(());
                        }
                    }
                })?;
            }
            Session::Sqlite(conn) => {
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                if !stmt.readonly() {
                    return Err(
                        "That statement would change the file, so Runo didn't run it.".into(),
                    );
                }
                let names: Vec<String> =
                    stmt.column_names().iter().map(|n| n.to_string()).collect();
                put(&mut out, names.clone())?;
                let masked: Vec<bool> = names.iter().map(|n| hidden.contains(n)).collect();
                let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
                while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                    let fields = (0..names.len())
                        .map(|i| {
                            let text = match row.get_ref(i) {
                                Ok(rusqlite::types::ValueRef::Null) | Err(_) => {
                                    return String::new()
                                }
                                Ok(rusqlite::types::ValueRef::Integer(n)) => n.to_string(),
                                Ok(rusqlite::types::ValueRef::Real(f)) => f.to_string(),
                                Ok(rusqlite::types::ValueRef::Text(t)) => {
                                    String::from_utf8_lossy(t).to_string()
                                }
                                Ok(rusqlite::types::ValueRef::Blob(b)) => {
                                    b.iter().map(|x| format!("{x:02x}")).collect()
                                }
                            };
                            if masked[i] {
                                mask(&text)
                            } else {
                                text
                            }
                        })
                        .collect();
                    put(&mut out, fields)?;
                    written += 1;
                    if written >= EXPORT_MAX {
                        break;
                    }
                }
            }
        }
        out.flush().map_err(|e| e.to_string())?;
        Ok(written)
    }
}

/// A new file only the person can read: an export can hold anything the database does.
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// One CSV field. A value a spreadsheet would run as a formula (`=…`, `+…`, `@…`) gets a
/// leading `'` so it opens as text.
pub(crate) fn csv_field(text: &str) -> String {
    let formula =
        text.starts_with(['=', '+', '-', '@', '\t', '\r']) && text.trim().parse::<f64>().is_err();
    let text = if formula {
        format!("'{text}")
    } else {
        text.to_string()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn count_sql(kind: Kind, table: &Table, b: &Browse) -> (String, Vec<String>) {
    let (wheres, values) = conditions(kind, &b.filters, kind == Kind::Sqlite);
    (
        format!(
            "SELECT count(*) FROM {}{}",
            table_ref(kind, table),
            where_sql(&wheres)
        ),
        values,
    )
}

/// A column's statistics: estimated from what the database keeps, or counted.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ColumnStat {
    pub name: String,
    /// Rows counted, when counted.
    pub rows: Option<i64>,
    /// The share of rows with no value, 0 to 1.
    pub nulls: Option<f64>,
    pub distinct: Option<f64>,
    /// The most common values; never for a column whose values are hidden.
    pub common: Vec<String>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub exact: bool,
}

/// The values of a Postgres array literal like `{a,"b c",d}`.
fn pg_array(text: &str) -> Vec<String> {
    let inner = text.trim().trim_start_matches('{').trim_end_matches('}');
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    if !cur.is_empty() || !out.is_empty() {
        out.push(cur);
    }
    out
}

/// The warning for a plan that reads every row of a big table. A sequential scan under
/// nothing but a LIMIT stops early and is fine, unless it filters, when it may read on
/// to the end looking for matches.
fn plan_warning(root: &serde_json::Value, tables: &[Table]) -> Option<String> {
    fn walk(
        node: &serde_json::Value,
        streaming: bool,
        tables: &[Table],
        worst: &mut Option<(String, i64)>,
    ) {
        let ty = node["Node Type"].as_str().unwrap_or("");
        if ty == "Seq Scan" {
            let rel = node["Relation Name"].as_str().unwrap_or("").to_string();
            let estimate = tables
                .iter()
                .filter(|t| t.name == rel)
                .filter_map(|t| t.estimate)
                .max()
                .unwrap_or_else(|| node["Plan Rows"].as_f64().unwrap_or(0.) as i64);
            let full = !streaming || node.get("Filter").is_some();
            if full && worst.as_ref().is_none_or(|(_, n)| estimate > *n) {
                *worst = Some((rel, estimate));
            }
        }
        let passes = matches!(ty, "Limit" | "Result" | "Gather" | "Subquery Scan");
        for child in node["Plans"].as_array().into_iter().flatten() {
            walk(child, streaming && passes, tables, worst);
        }
    }
    let mut worst = None;
    walk(root, true, tables, &mut worst);
    let (rel, n) = worst.filter(|(_, n)| *n >= BIG_SCAN)?;
    Some(format!(
        "This reads every row of {rel} (about {}) because no index narrows it down. On a production database that can slow things down for everyone using it.",
        about(n)
    ))
}

fn pg_run(client: &mut postgres::Client, sql: &str, limit: usize) -> Result<Ran, String> {
    // Preparing it parses it on the server, which refuses more than one statement.
    let statement = client.prepare(sql).map_err(pg_error)?;
    let names: Vec<String> = statement
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let types: Vec<String> = statement
        .columns()
        .iter()
        .map(|c| c.type_().name().to_string())
        .collect();
    let sources: Vec<Option<(u32, i16)>> = statement
        .columns()
        .iter()
        .map(|c| c.table_oid().zip(c.column_id()).filter(|(t, _)| *t != 0))
        .collect();
    drop(statement);
    let origins = pg_origins(client, &sources)?;
    let messages = read_only(client, |c| {
        if cursor_kind(sql) {
            c.batch_execute(&format!("DECLARE runo_query NO SCROLL CURSOR FOR {sql}"))
                .map_err(pg_error)?;
            c.simple_query(&format!("FETCH FORWARD {} FROM runo_query", limit + 1))
                .map_err(pg_error)
        } else {
            c.simple_query(sql).map_err(pg_error)
        }
    })?;
    let mut rows = pg_text_rows(messages, |i, _| {
        types.get(i).map_or(CellKind::Text, |t| kind_of_type(t))
    });
    if rows.columns.is_empty() {
        rows.columns = names;
    }
    Ok(Ran {
        rows,
        types,
        origins,
        hidden: Vec::new(),
    })
}

/// The table and column behind each (table oid, column number) the server reported.
fn pg_origins(
    client: &mut postgres::Client,
    sources: &[Option<(u32, i16)>],
) -> Result<Vec<Option<(String, String)>>, String> {
    let pairs: Vec<String> = sources
        .iter()
        .flatten()
        .map(|(t, c)| format!("({t}::oid, {c}::int2)"))
        .collect();
    if pairs.is_empty() {
        return Ok(vec![None; sources.len()]);
    }
    let rows = pg_rows(
        client,
        &format!(
            "SELECT a.attrelid::int8::text, a.attnum::text, c.relname, a.attname FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid WHERE (a.attrelid, a.attnum) IN ({})",
            pairs.join(", ")
        ),
    )?;
    let found: HashMap<(u32, i16), (String, String)> = rows
        .into_iter()
        .filter_map(|r| {
            let t = r.first()?.as_ref()?.parse().ok()?;
            let c = r.get(1)?.as_ref()?.parse().ok()?;
            Some(((t, c), (r.get(2)?.clone()?, r.get(3)?.clone()?)))
        })
        .collect();
    Ok(sources
        .iter()
        .map(|s| s.and_then(|k| found.get(&k).cloned()))
        .collect())
}

/// The table and column behind each of a SQLite statement's result columns, from
/// SQLite's column metadata (compiled into the bundled library).
fn sqlite_origins(conn: &rusqlite::Connection, sql: &str) -> Vec<Option<(String, String)>> {
    use rusqlite::ffi;
    use std::ffi::{CStr, CString};
    let Ok(text) = CString::new(sql) else {
        return Vec::new();
    };
    let text_of = |p: *const std::os::raw::c_char| {
        // SAFETY: SQLite returns null or a NUL-terminated string that lives as long
        // as the statement, which is finalized only after these are copied.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    };
    let mut out = Vec::new();
    // SAFETY: the handle belongs to `conn`, borrowed for the whole call, and the
    // statement prepared here is finalized before returning.
    unsafe {
        let mut stmt = std::ptr::null_mut();
        let rc = ffi::sqlite3_prepare_v2(
            conn.handle(),
            text.as_ptr(),
            -1,
            &mut stmt,
            std::ptr::null_mut(),
        );
        if rc != ffi::SQLITE_OK || stmt.is_null() {
            if !stmt.is_null() {
                ffi::sqlite3_finalize(stmt);
            }
            return Vec::new();
        }
        for i in 0..ffi::sqlite3_column_count(stmt) {
            let table = text_of(ffi::sqlite3_column_table_name(stmt, i));
            let column = text_of(ffi::sqlite3_column_origin_name(stmt, i));
            out.push(table.zip(column));
        }
        ffi::sqlite3_finalize(stmt);
    }
    out
}

fn sqlite_run(conn: &rusqlite::Connection, sql: &str, limit: usize) -> Result<Ran, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    if !stmt.readonly() {
        return Err("That statement would change the file, so Runo didn't run it.".into());
    }
    let columns: Vec<(String, String)> = stmt
        .columns()
        .iter()
        .map(|c| {
            (
                c.name().to_string(),
                c.decl_type().unwrap_or("").to_string(),
            )
        })
        .collect();
    let kinds: Vec<CellKind> = columns.iter().map(|(_, t)| kind_of_type(t)).collect();
    let mut out: Vec<Vec<Cell>> = Vec::new();
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut cells = Vec::with_capacity(columns.len());
        for (i, kind) in kinds.iter().enumerate() {
            cells.push(sqlite_cell(
                row.get_ref(i).map_err(|e| e.to_string())?,
                *kind,
            ));
        }
        out.push(cells);
        if out.len() > limit {
            break;
        }
    }
    Ok(Ran {
        origins: sqlite_origins(conn, sql),
        hidden: Vec::new(),
        types: columns.iter().map(|(_, t)| t.clone()).collect(),
        rows: Rows {
            columns: columns.into_iter().map(|(n, _)| n).collect(),
            rows: out,
            ..Rows::default()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_single_reading_statements_pass() {
        let ok = |sql: &str| check(Kind::Postgres, sql);
        assert_eq!(ok("select 1;").unwrap(), "select 1");
        assert_eq!(ok("  SELECT 1 ; -- done\n").unwrap(), "SELECT 1");
        assert_eq!(
            ok("with x as (select 1) select * from x").unwrap(),
            "with x as (select 1) select * from x"
        );
        assert!(ok("/* hi */ (select 1)").is_ok());
        assert!(ok("select ';' as semi, $$;drop$$ as dollar").is_ok());
        assert!(ok("select 1; drop table orders")
            .unwrap_err()
            .contains("one statement"));
        assert!(ok("delete from orders")
            .unwrap_err()
            .contains("only runs statements that read"));
        assert!(ok("SET transaction_read_only = off").is_err());
        assert!(ok("copy orders to program 'rm -rf /'").is_err());
        assert!(ok("select pg_terminate_backend(42)")
            .unwrap_err()
            .contains("pg_terminate_backend"));
        assert!(ok("select \"pg_read_file\"('/etc/passwd')").is_err());
        assert!(ok("select pg_catalog.set_config('transaction_read_only', 'off', true)").is_err());
        assert!(ok("select * from dblink('host=x', 'delete from t') as t(a int)").is_err());
        // Names only count as calls.
        assert!(ok("select set_config_name from settings").is_ok());
        assert!(ok("select 'pg_read_file(' as s").is_ok());
        assert!(ok("").unwrap_err().contains("Write a query"));
        assert!(ok("-- just a comment").is_err());
        assert!(check(Kind::Sqlite, "pragma writable_schema = on").is_err());
        assert!(check(Kind::Sqlite, "select load_extension('x')").is_err());
        assert!(check(Kind::Sqlite, "show tables").is_err());
    }

    #[test]
    fn writes_tucked_inside_a_read_are_refused() {
        let no = |sql: &str| check(Kind::Postgres, sql).unwrap_err();
        assert!(
            no("with gone as (delete from orders returning *) select * from gone")
                .contains("DELETE")
        );
        assert!(no(
            "with x as materialized (update orders set status = 'paid' returning id) select 1"
        )
        .contains("UPDATE"));
        assert!(no("explain analyze delete from orders").contains("DELETE"));
        assert!(no("explain (analyze, buffers) update orders set x = 1").contains("UPDATE"));
        assert!(no("explain explain select 1").contains("EXPLAIN"));
        assert!(no("select * into backup from orders").contains("INTO"));
        assert!(no("select * from orders for update").contains("lock rows"));
        assert!(no("select * from orders for no key update").contains("lock rows"));
        assert!(no("select U&\"\\0070g_terminate_backend\"(1)").contains("U&"));
        assert!(no("select pg_stat_reset()").contains("pg_stat_reset"));
        assert!(no("select pg_read_server_files_x()").contains("pg_read_"));
        assert!(no("select lo_get(1)").contains("lo_get"));
        assert!(no("select dblink_open('c', 'select 1')").contains("dblink_open"));
        assert!(
            no("select query_to_xml('delete from orders', true, true, '')")
                .contains("query_to_xml")
        );
        assert!(no("select net.http_get('https://example.com')").contains("http_get"));
        assert!(no("select net.request(1)").contains("net.request"));
        assert!(no("select pg_notify('c', 'hi')").contains("pg_notify"));
        // Reads that only look similar still run.
        let ok = |sql: &str| check(Kind::Postgres, sql);
        assert!(ok("explain analyze select * from orders").is_ok());
        assert!(ok("explain (format json) select 1").is_ok());
        assert!(ok("select 'delete' as word, (select 1) as one").is_ok());
        assert!(ok("select u & 3 from flags").is_ok());
        assert!(ok("select * from orders order by id for_report").is_ok());
        assert!(ok("select pg_advisory_unlock_all_x from t").is_ok());
        assert!(check(Kind::Sqlite, "explain query plan select 1").is_ok());
        assert!(check(Kind::Sqlite, "explain query plan delete from t").is_err());
    }

    #[test]
    fn spreadsheets_get_formulas_as_text() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_field("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_field("-12.5"), "-12.5");
        assert_eq!(csv_field("-cmd"), "'-cmd");
    }

    #[test]
    fn postgres_arrays_split() {
        assert_eq!(
            pg_array("{paid,failed,\"on hold\"}"),
            vec!["paid", "failed", "on hold"]
        );
        assert!(pg_array("").is_empty());
    }

    #[test]
    fn plans_warn_on_full_reads_of_big_tables() {
        let tables = vec![Table {
            name: "orders".into(),
            estimate: Some(2_300_000),
            ..Table::default()
        }];
        let plan = |json: &str| plan_warning(&serde_json::from_str(json).unwrap(), &tables);
        // A LIMIT over a plain scan stops early.
        assert!(plan(
            r#"{"Node Type":"Limit","Plans":[{"Node Type":"Seq Scan","Relation Name":"orders"}]}"#
        )
        .is_none());
        // Filtering or sorting reads all of it.
        let w = plan(r#"{"Node Type":"Limit","Plans":[{"Node Type":"Seq Scan","Relation Name":"orders","Filter":"(status = 'x')"}]}"#).unwrap();
        assert!(w.contains("orders") && w.contains("2.3M"), "{w}");
        assert!(plan(r#"{"Node Type":"Limit","Plans":[{"Node Type":"Sort","Plans":[{"Node Type":"Seq Scan","Relation Name":"orders"}]}]}"#).is_some());
        assert!(plan(r#"{"Node Type":"Aggregate","Plans":[{"Node Type":"Seq Scan","Relation Name":"orders"}]}"#).is_some());
        // An index lookup is fine, and so is a small table.
        assert!(plan(r#"{"Node Type":"Index Scan","Relation Name":"orders"}"#).is_none());
        assert!(plan(r#"{"Node Type":"Aggregate","Plans":[{"Node Type":"Seq Scan","Relation Name":"tiny","Plan Rows":40}]}"#).is_none());
    }

    fn sample() -> (std::path::PathBuf, Session) {
        let path = std::env::temp_dir().join(format!("runo-query-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, email TEXT, n REAL, ok BOOLEAN);
             INSERT INTO t VALUES (1, 'a@x.dev', 1.5, 1), (2, '=cmd', NULL, 0), (3, 'c@x.dev', 3, 1);",
        )
        .unwrap();
        drop(conn);
        let (s, _) = Session::open(Kind::Sqlite, path.to_str().unwrap()).unwrap();
        (path, s)
    }

    fn masked(s: &mut Session, sql: &str) -> Vec<bool> {
        let tables = s.tables().unwrap();
        let ran = s.run(sql, 10).unwrap();
        hidden_columns(&ran, sql, &tables)
    }

    #[test]
    fn renaming_or_working_on_a_hidden_column_keeps_it_hidden() {
        let path = std::env::temp_dir().join(format!("runo-mask-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, name TEXT);
             INSERT INTO users VALUES (1, 'a@x.dev', 'Ann'), (2, 'b@x.dev', 'Bo');
             CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
             INSERT INTO notes VALUES (1, 'hi');
             CREATE VIEW people AS SELECT id, email AS contact FROM users;",
        )
        .unwrap();
        drop(conn);
        let (mut s, _) = Session::open(Kind::Sqlite, path.to_str().unwrap()).unwrap();
        assert_eq!(
            masked(&mut s, "select id, email, name from users"),
            [false, true, false]
        );
        assert_eq!(
            masked(&mut s, "select email as contact, name as n from users"),
            [true, false]
        );
        assert_eq!(
            masked(&mut s, "select x from (select email as x from users)"),
            [true]
        );
        assert_eq!(masked(&mut s, "select contact from people"), [true]);
        assert_eq!(
            masked(
                &mut s,
                "select upper(email) as shout, count(*) as n, length(name) as l from users"
            ),
            [true, false, false]
        );
        // An expression over a table with nothing hidden shows.
        assert_eq!(masked(&mut s, "select upper(body) from notes"), [false]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sqlite_queries_read_count_and_export() {
        let (path, mut s) = sample();
        let ran = s.run("select id, ok, n from t order by id", 2).unwrap();
        assert_eq!(ran.rows.columns, vec!["id", "ok", "n"]);
        assert_eq!(ran.rows.rows.len(), 2);
        assert!(ran.rows.more);
        assert_eq!(ran.rows.rows[0][1], Cell::text("true", CellKind::Bool));
        assert!(s.run("delete from t", 10).is_err());
        assert!(s.run("select 1; delete from t", 10).is_err());

        let tables = s.tables().unwrap();
        let table = &tables[0];
        assert_eq!(s.count(table, &Browse::default()).unwrap(), 3);
        let n = table.columns.iter().find(|c| c.name == "n").unwrap();
        let stat = s.exact_stats(table, n, &Browse::default()).unwrap();
        assert_eq!(stat.rows, Some(3));
        assert!((stat.nulls.unwrap() - 1. / 3.).abs() < 1e-9);
        assert_eq!(stat.distinct, Some(2.));
        assert_eq!(
            (stat.min.as_deref(), stat.max.as_deref()),
            (Some("1.5"), Some("3.0"))
        );
        let email = table.columns.iter().find(|c| c.name == "email").unwrap();
        assert_eq!(
            s.exact_stats(table, email, &Browse::default()).unwrap().min,
            None
        );

        let out = std::env::temp_dir().join(format!("runo-export-{}.csv", std::process::id()));
        let hidden = HashSet::from(["email".to_string()]);
        let n = s
            .export_csv("select id, email from t", &out, &hidden)
            .unwrap();
        assert_eq!(n, 3);
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text, "id,email\n1,a•••@x.dev\n2,••••••\n3,c•••@x.dev\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = std::fs::remove_file(out);
        let _ = std::fs::remove_file(path);
    }

    /// Runs against a real server when `RUNO_TEST_PG` holds its URL.
    #[test]
    fn postgres_queries_stay_read_only() {
        let Ok(url) = std::env::var("RUNO_TEST_PG") else {
            return;
        };
        let (mut s, _) = Session::open(Kind::Postgres, &url).unwrap();
        let ran = s
            .run("select 1 as one, 'x'::text as t, true as b", 10)
            .unwrap();
        assert_eq!(ran.rows.columns, vec!["one", "t", "b"]);
        assert_eq!(ran.types, vec!["int4", "text", "bool"]);
        assert_eq!(ran.rows.rows[0][2], Cell::text("true", CellKind::Bool));
        let ran = s.run("select generate_series(1, 50)", 10).unwrap();
        assert_eq!((ran.rows.rows.len(), ran.rows.more), (10, true));
        // A data-modifying CTE gets past the keyword check but not the database.
        assert!(s
            .run(
                "with d as (delete from pg_class returning 1) select * from d",
                10
            )
            .is_err());
        assert!(s.run("explain select 1", 10).is_ok());
        assert!(s.run("show server_version", 10).is_ok());
        // Where a result column comes from, through renames and subqueries.
        let ran = s
            .run(
                "select x, c.relname as n, 1 as one from (select relname as x from pg_class limit 1) s, pg_class c limit 1",
                10,
            )
            .unwrap();
        assert_eq!(
            ran.origins,
            vec![
                Some(("pg_class".into(), "relname".into())),
                Some(("pg_class".into(), "relname".into())),
                None
            ]
        );
        // The session is still usable and still read-only afterwards.
        assert!(s.run("select 2", 10).is_ok());
        let Session::Postgres(client, _) = &mut s else {
            unreachable!()
        };
        let still = client.simple_query("SHOW transaction_read_only").unwrap();
        assert!(still
            .iter()
            .any(|m| matches!(m, postgres::SimpleQueryMessage::Row(r) if r.get(0) == Some("on"))));
    }
}
