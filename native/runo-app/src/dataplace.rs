//! The Data place: the top bar project's databases. It opens on what Runo found in the
//! project's files, connects read-only, and browses tables in a grid with filters,
//! sorting, and a row inspector that follows foreign keys.

use super::*;
use crate::dataquery::DataTab;
use db::detect::Found;
use db::engine::Opened;
pub(crate) use db::mask;
use db::saved::Saved;
use db::{Browse, Cell, CellKind, Env, Filter, Kind, Op, Rows, Table};
use gpui_kit::component::table::{
    Column as GridColumn, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::Sizable as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Width of the connections and tables list.
const SIDE_WIDTH: f32 = 264.;
/// Width of the row inspector.
const INSPECTOR_WIDTH: f32 = 300.;

/// Open sessions, by saved connection id, shared with the threads that run queries.
type Live = Arc<Mutex<HashMap<String, db::engine::Session>>>;
/// How to stop what each session is running, kept apart so a stop needn't wait for it.
type Cancellers = Arc<Mutex<HashMap<String, db::query::Canceller>>>;

/// Why a run on a worker thread didn't give a result.
pub(crate) enum Stop {
    Failed(String),
    /// It would read a whole big table on production: the warning to show first.
    Heavy(String),
}

/// A connection's session as worker threads reach it. When the server dropped it (a
/// restart, or the computer slept), it reconnects once and tries again.
pub(crate) struct SessionRef {
    live: Live,
    cancellers: Cancellers,
    id: String,
    saved: Option<Saved>,
    root: Option<String>,
}

impl SessionRef {
    pub(crate) fn with<T>(
        &self,
        f: impl Fn(&mut db::engine::Session) -> Result<T, Stop>,
    ) -> Result<T, Stop> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let session = live
            .get_mut(&self.id)
            .ok_or_else(|| Stop::Failed("The connection closed. Connect again.".into()))?;
        match f(session) {
            Err(Stop::Failed(_)) if session.is_closed() => {
                *session = self.reopen().map_err(Stop::Failed)?;
                self.cancellers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(self.id.clone(), session.canceller());
                f(session)
            }
            other => other,
        }
    }

    fn reopen(&self) -> Result<db::engine::Session, String> {
        let (Some(saved), Some(root)) = (&self.saved, &self.root) else {
            return Err("The connection closed. Connect again.".into());
        };
        let target = db::saved::target(saved, Path::new(root))?;
        let (session, opened) = db::engine::Session::open(saved.kind, &target)
            .map_err(|e| format!("The connection closed, and reconnecting failed: {e}"))?;
        if !opened.read_only {
            return Err("The database didn't confirm a read-only session on reconnecting, so Runo didn't use it.".into());
        }
        Ok(session)
    }
}

/// A run held back on production until the person says go.
pub(crate) struct Warning {
    pub text: String,
    /// The statement it's about; once run anyway, it isn't asked about again.
    pub sql: String,
    pub then: Then,
}

/// What to do once the person says go.
pub(crate) enum Then {
    Browse,
    Query,
    Count,
    Column(String),
    Export(std::path::PathBuf, HashSet<String>),
}

/// One saved connection's state while the place is open.
#[derive(Default)]
pub(crate) struct Conn {
    pub opened: Option<Opened>,
    pub tables: Vec<Table>,
    pub error: Option<String>,
    pub connecting: bool,
}

pub(crate) struct DataPage {
    /// The URL or file path typed to add a connection.
    pub url: Entity<InputState>,
    /// A condition typed in the filter bar, like `status = failed`.
    pub filter: Entity<InputState>,
    /// Narrows the list of tables.
    pub search: Entity<InputState>,
    pub grid: Entity<TableState<Grid>>,
    /// The project shown.
    pub project: Option<u64>,
    pub found: Vec<Found>,
    pub scanning: bool,
    /// Every project's saved connections.
    pub saved: Vec<Saved>,
    pub selected: Option<String>,
    pub conns: HashMap<String, Conn>,
    live: Live,
    /// The open table, by `Table::label`.
    pub table: Option<String>,
    pub browse: Browse,
    pub rows: Option<Rows>,
    pub loading: bool,
    pub rows_error: Option<String>,
    /// The row shown in the inspector.
    pub row: Option<usize>,
    /// Hidden columns the person chose to see.
    pub revealed: HashSet<String>,
    pub adding: bool,
    pub add_error: Option<String>,
    pub filter_error: Option<String>,
    /// The connection whose Remove was pressed once, waiting for the second press.
    pub confirm_remove: Option<String>,
    pub tab: crate::dataquery::DataTab,
    pub stats: crate::datastats::Stats,
    pub query: crate::dataquery::QueryPage,
    cancellers: Cancellers,
    /// A run waiting for "Run anyway".
    pub warning: Option<Warning>,
    /// Statements the person said to run anyway on production.
    pub cleared: HashSet<String>,
    /// What agents ran on the selected connection, newest first.
    pub activity: Vec<db::saved::Activity>,
}

impl DataPage {
    pub(crate) fn new(
        ui_tx: Sender<ship::UiJob>,
        mono: SharedString,
        window: &mut Window,
        cx: &mut Context<RunoApp>,
    ) -> (Self, Vec<Subscription>) {
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("postgres://user:password@host:5432/name, or a SQLite file's path")
        });
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter, like status = failed, total > 1000, email contains gmail")
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a table"));
        let (query, query_subs) =
            crate::dataquery::QueryPage::new(ui_tx.clone(), mono.clone(), window, cx);
        let grid = cx.new(|cx| {
            TableState::new(Grid::empty(ui_tx, mono), window, cx)
                .row_selectable(true)
                .col_selectable(false)
                .col_movable(false)
                .sortable(true)
        });
        let mut subs = vec![
            cx.subscribe_in(&url, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().trim().to_string();
                    if this.add_typed(&text, cx) {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&filter, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().trim().to_string();
                    if this.add_filter_text(&text, cx) {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&grid, window, |this, _, event: &TableEvent, _, cx| {
                match event {
                    TableEvent::SelectRow(i) | TableEvent::SelectCell(i, _) => {
                        this.data.row = Some(*i);
                        this.data.stats.open = false;
                    }
                    TableEvent::ClearSelection => this.data.row = None,
                    _ => return,
                }
                cx.notify();
            }),
        ];
        subs.extend(query_subs);
        (
            Self {
                url,
                filter,
                search,
                grid,
                project: None,
                found: Vec::new(),
                scanning: false,
                saved: db::saved::load(),
                selected: None,
                conns: HashMap::new(),
                live: Arc::default(),
                table: None,
                browse: Browse::default(),
                rows: None,
                loading: false,
                rows_error: None,
                row: None,
                revealed: HashSet::new(),
                adding: false,
                add_error: None,
                filter_error: None,
                confirm_remove: None,
                tab: Default::default(),
                stats: Default::default(),
                query,
                cancellers: Arc::default(),
                warning: None,
                cleared: HashSet::new(),
                activity: Vec::new(),
            },
            subs,
        )
    }

    pub(crate) fn table_by_label(&self, label: &str) -> Option<&Table> {
        let id = self.selected.as_ref()?;
        self.conns
            .get(id)?
            .tables
            .iter()
            .find(|t| t.label() == label)
    }

    pub(crate) fn table_of(&self) -> Option<&Table> {
        let id = self.selected.as_ref()?;
        let label = self.table.as_ref()?;
        self.conns
            .get(id)?
            .tables
            .iter()
            .find(|t| &t.label() == label)
    }
}

/// `n` rows in a few characters: 950, 8.2k, 1.2M.
pub(crate) fn short_count(n: i64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 10_000 => format!("{}k", n / 1000),
        n if n >= 1000 && n % 1000 < 50 => format!("{}k", n / 1000),
        n if n >= 1000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// A hidden value: an email keeps its first letter and domain, anything else is dots.
/// A JSON value laid out over lines, or the text as it is when it isn't JSON.
pub(crate) fn pretty_json(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| text.to_string())
}

/// A row as a JSON object, for the clipboard. Values still hidden stay masked.
pub(crate) fn row_json(
    columns: &[String],
    row: &[Cell],
    hidden: impl Fn(&String) -> bool,
) -> String {
    let mut out = serde_json::Map::new();
    for (name, cell) in columns.iter().zip(row) {
        let value = match cell.kind {
            CellKind::Null => serde_json::Value::Null,
            _ if hidden(name) => serde_json::Value::String(mask(&cell.text)),
            CellKind::Bool => serde_json::Value::Bool(cell.text == "true"),
            CellKind::Number => serde_json::from_str::<serde_json::Number>(&cell.text)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|_| serde_json::Value::String(cell.text.clone())),
            CellKind::Json => serde_json::from_str(&cell.text)
                .unwrap_or_else(|_| serde_json::Value::String(cell.text.clone())),
            _ => serde_json::Value::String(cell.text.clone()),
        };
        out.insert(name.clone(), value);
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(out)).unwrap_or_default()
}

/// A condition typed in the filter bar, against the table's columns: `status = failed`,
/// `total > 1000`, `email contains gmail`, `paid_at is null`.
pub(crate) fn parse_filter(text: &str, columns: &[db::Column]) -> Result<Filter, String> {
    let text = text.trim();
    let lower = text.to_lowercase();
    // The longest column name the text starts with, so `email_verified_at` beats `email`.
    let column = columns
        .iter()
        .filter(|c| {
            let n = c.name.to_lowercase();
            lower.starts_with(&n)
                && lower[n.len()..]
                    .chars()
                    .next()
                    .is_some_and(|ch| !ch.is_alphanumeric() && ch != '_')
        })
        .max_by_key(|c| c.name.len())
        .ok_or_else(|| {
            let first = text
                .split(|c: char| c.is_whitespace() || "=<>!≠~".contains(c))
                .next()
                .unwrap_or("");
            let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
            format!(
                "This table has no column called {first}. Its columns: {}.",
                names.join(", ")
            )
        })?;
    let rest = text[column.name.len()..].trim_start();
    let rest_lower = rest.to_lowercase();
    let ops: [(&str, Op); 11] = [
        ("is not null", Op::NotNull),
        ("is null", Op::IsNull),
        ("contains", Op::Contains),
        ("!=", Op::Ne),
        ("<>", Op::Ne),
        ("≠", Op::Ne),
        ("~", Op::Contains),
        ("=", Op::Eq),
        (">", Op::Gt),
        ("<", Op::Lt),
        ("is", Op::Eq),
    ];
    let (word, op) = ops
        .iter()
        .find(|(w, _)| rest_lower.starts_with(w))
        .copied()
        .ok_or_else(|| {
            "Write a condition after the column: =, ≠, >, <, contains, is null or is not null."
                .to_string()
        })?;
    let value = rest[word.len()..].trim();
    let value = value
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .or_else(|| value.strip_prefix('"').and_then(|v| v.strip_suffix('"')))
        .unwrap_or(value)
        .to_string();
    if op.takes_value() && value.is_empty() {
        return Err(format!("Give a value after {word}."));
    }
    // A number column compared with something that isn't a number would only come back as
    // the database's error, so say it here.
    let numeric = db::engine::kind_of_type(&column.data_type) == CellKind::Number;
    if numeric && op != Op::Contains && op.takes_value() && value.parse::<f64>().is_err() {
        return Err(format!(
            "{} holds numbers, and {value} isn't one.",
            column.name
        ));
    }
    Ok(Filter {
        column: column.name.clone(),
        op,
        value: if op.takes_value() {
            value
        } else {
            String::new()
        },
    })
}

fn kind_color(kind: Kind) -> u32 {
    match kind {
        Kind::Postgres => c_blue(),
        Kind::Sqlite => c_green(),
        Kind::Mysql => c_amber(),
        Kind::Redis => c_red(),
        Kind::Mongo => c_green(),
    }
}

fn env_color(env: Env) -> u32 {
    match env {
        Env::Local => c_green(),
        Env::Staging => c_amber(),
        Env::Prod => c_red(),
    }
}

fn kind_badge(kind: Kind, size: f32) -> Div {
    let color = kind_color(kind);
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(rgb(settings::mix(
            color,
            c_canvas(),
            if is_dark() { 0.3 } else { 0.18 },
        )))
        .text_color(rgb(color))
        .text_size(px(size * 0.4))
        .font_weight(FontWeight::BOLD)
        .child(kind.mark())
}

fn env_chip(env: Env) -> Div {
    let color = env_color(env);
    div()
        .flex_none()
        .px(px(5.))
        .h(px(16.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .bg(rgb(settings::mix(
            color,
            c_canvas(),
            if is_dark() { 0.22 } else { 0.14 },
        )))
        .text_color(rgb(color))
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .child(env.label())
}

pub(crate) fn label_row(text: impl Into<SharedString>) -> Div {
    div()
        .px_3()
        .pt(px(14.))
        .pb(px(6.))
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_muted()))
        .child(div().flex_1().child(text.into()))
}

pub(crate) fn small_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .flex_none()
        .h(px(26.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded_md()
        .cursor_pointer()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(label.into());
    if primary {
        el.bg(rgb(c_text()))
            .text_color(rgb(c_canvas()))
            .hover(|el| el.opacity(0.9))
    } else {
        el.border_1()
            .border_color(divider())
            .hover(|el| el.bg(tint(c_subtle())))
    }
}

/// One line of the connection page's facts: an icon and a sentence.
fn note_row(glyph: IconName, color: u32, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.5))
        .text_color(rgb(c_muted()))
        .child(icon(glyph, 13., color))
        .child(text.into())
}

/// How the session travels, warning when it crosses the network in the clear.
fn transport_row(t: db::engine::Transport, local: bool) -> Div {
    use db::engine::Transport;
    let (glyph, color, text) = match t {
        Transport::File => (IconName::Lock, c_green(), "A file on this computer.".to_string()),
        Transport::Verified => (
            IconName::Lock,
            c_green(),
            "Encrypted, with the server's certificate verified.".into(),
        ),
        Transport::Encrypted if local => (
            IconName::Lock,
            c_muted(),
            "Encrypted. The certificate isn't checked for a database on this computer.".into(),
        ),
        Transport::Encrypted => (
            IconName::LockOpen,
            c_amber(),
            "Encrypted, but the server's certificate isn't checked (sslmode=require), so another machine could stand in for it.".into(),
        ),
        Transport::Plain if local => (
            IconName::LockOpen,
            c_muted(),
            "Not encrypted, which is fine on this computer.".into(),
        ),
        Transport::Plain => (
            IconName::TriangleAlert,
            c_amber(),
            "Not encrypted: rows and the password cross the network in the clear (sslmode=disable).".into(),
        ),
    };
    note_row(glyph, color, text)
}

pub(crate) fn centered(text: impl Into<SharedString>) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(rgb(c_muted()))
        .child(text.into())
        .into_any_element()
}

impl RunoApp {
    /// The project folder the Data place reads.
    fn data_root(&self) -> Option<String> {
        let pid = self.data.project?;
        let p = self.state.projects.iter().find(|p| p.id == pid)?;
        (!p.path.is_empty()).then(|| p.path.clone())
    }

    /// Follow the top bar's project: read its files for databases when it changes.
    pub(crate) fn prepare_data(&mut self, cx: &mut Context<Self>) {
        if self.all_scope() {
            return;
        }
        let pid = self.current_project();
        if pid == self.data.project {
            return;
        }
        self.data.project = pid;
        self.data.found.clear();
        self.data.table = None;
        self.data.rows = None;
        self.data.row = None;
        self.data.selected = pid.and_then(|pid| {
            self.data
                .saved
                .iter()
                .find(|s| s.project == pid)
                .map(|s| s.id.clone())
        });
        self.scan_data(cx);
        if let Some(id) = self.data.selected.clone() {
            self.open_or_hold(&id, cx);
        }
    }

    /// Open connection `id`, unless it's production: that waits on its page for the
    /// person to say connect, so a click in the list or a project switch never opens one.
    fn open_or_hold(&mut self, id: &str, cx: &mut Context<Self>) {
        let prod = self
            .data
            .saved
            .iter()
            .any(|s| s.id == id && s.env == Env::Prod);
        if prod {
            self.data.conns.entry(id.to_string()).or_default();
            cx.notify();
        } else {
            self.open_data_conn(id, cx);
        }
    }

    /// Look through the project's files for databases, off the UI thread.
    pub(crate) fn scan_data(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.data_root() else {
            return;
        };
        let pid = self.data.project;
        self.data.scanning = true;
        cx.notify();
        self.bg(
            move || {
                let mut found = db::detect::find(Path::new(&root));
                let ports = db::detect::open_ports(&found);
                found.extend(ports);
                db::detect::probe(&mut found);
                found
            },
            move |this, found, cx| {
                if this.data.project == pid {
                    this.data.found = found;
                    this.data.scanning = false;
                    cx.notify();
                }
            },
        );
    }

    fn save_data_conns(&self) {
        if let Err(e) = db::saved::store(&self.data.saved) {
            eprintln!("runo: couldn't save database connections: {e}");
        }
    }

    fn connect_found(&mut self, key: &str, cx: &mut Context<Self>) {
        let (Some(pid), Some(found)) = (
            self.data.project,
            self.data.found.iter().find(|f| f.key == key),
        ) else {
            return;
        };
        // Already in the list: open that one rather than adding it twice.
        let existing = self.data.saved.iter().find(|s| {
            s.project == pid && matches!(&s.source, db::saved::Source::Found { key: k } if k == key)
        });
        if let Some(id) = existing.map(|s| s.id.clone()) {
            self.select_data_conn(&id, cx);
            if self.data.conns.get(&id).is_none_or(|c| c.opened.is_none()) {
                self.open_data_conn(&id, cx);
            }
            return;
        }
        let saved = db::saved::from_found(pid, found);
        let id = saved.id.clone();
        self.data.saved.push(saved);
        self.save_data_conns();
        // Pressing Connect on it is the go-ahead, production or not.
        self.select_data_conn(&id, cx);
        self.open_data_conn(&id, cx);
    }

    /// Add a connection the person typed. Returns whether it was added.
    fn add_typed(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(pid) = self.data.project else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        match db::saved::from_typed(pid, text) {
            Ok(saved) => {
                let id = saved.id.clone();
                self.data.saved.push(saved);
                self.save_data_conns();
                self.data.adding = false;
                self.data.add_error = None;
                self.select_data_conn(&id, cx);
                self.open_data_conn(&id, cx);
                true
            }
            Err(e) => {
                self.data.add_error = Some(e);
                false
            }
        }
    }

    fn select_data_conn(&mut self, id: &str, cx: &mut Context<Self>) {
        self.data.selected = Some(id.to_string());
        self.data.confirm_remove = None;
        self.data.table = None;
        self.data.rows = None;
        self.data.row = None;
        self.data.rows_error = None;
        self.data.activity = db::saved::activity(id, crate::dataask::SHOWN);
        let open = self.data.conns.get(id).is_some_and(|c| c.opened.is_some());
        if !open {
            self.open_or_hold(id, cx);
        }
        cx.notify();
    }

    /// Open a read-only session and read the schema, off the UI thread.
    fn open_data_conn(&mut self, id: &str, cx: &mut Context<Self>) {
        let (Some(saved), Some(root)) = (
            self.data.saved.iter().find(|s| s.id == id).cloned(),
            self.data_root(),
        ) else {
            return;
        };
        let conn = self.data.conns.entry(id.to_string()).or_default();
        if conn.connecting {
            return;
        }
        conn.connecting = true;
        conn.error = None;
        let live = self.data.live.clone();
        let cancellers = self.data.cancellers.clone();
        let id = id.to_string();
        cx.notify();
        self.bg(
            {
                let id = id.clone();
                move || -> Result<(Opened, Vec<Table>), String> {
                    if !saved.kind.supported() {
                        return Err(format!(
                            "Runo can find {} databases but can't open them yet. Postgres and SQLite work today.",
                            saved.kind.label()
                        ));
                    }
                    let target = db::saved::target(&saved, Path::new(&root))?;
                    let (mut session, opened) = db::engine::Session::open(saved.kind, &target)?;
                    if !opened.read_only {
                        return Err("The database didn't confirm a read-only session, so Runo didn't open it.".into());
                    }
                    let tables = session.tables()?;
                    cancellers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(id.clone(), session.canceller());
                    live.lock().unwrap_or_else(|e| e.into_inner()).insert(id, session);
                    Ok((opened, tables))
                }
            },
            move |this, result, cx| {
                let conn = this.data.conns.entry(id).or_default();
                conn.connecting = false;
                match result {
                    Ok((opened, tables)) => {
                        conn.opened = Some(opened);
                        conn.tables = tables;
                        this.refresh_words();
                    }
                    Err(e) => {
                        conn.opened = None;
                        conn.error = Some(e);
                    }
                }
                cx.notify();
            },
        );
    }

    /// Close the session but keep the connection in the list.
    fn disconnect_data_conn(&mut self, id: &str, cx: &mut Context<Self>) {
        self.data.conns.remove(id);
        self.data
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if self.data.selected.as_deref() == Some(id) {
            self.data.selected = None;
            self.data.table = None;
            self.data.rows = None;
            self.data.row = None;
        }
        cx.notify();
    }

    /// Take the connection out of the list, with its saved password.
    fn forget_data_conn(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.data.saved.iter().position(|s| s.id == id) {
            let saved = self.data.saved.remove(i);
            db::saved::forget(&saved);
            self.save_data_conns();
        }
        self.data.conns.remove(id);
        self.data
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if self.data.selected.as_deref() == Some(id) {
            self.data.selected = None;
            self.data.table = None;
            self.data.rows = None;
        }
        cx.notify();
    }

    pub(crate) fn pick_table(
        &mut self,
        label: String,
        filters: Vec<Filter>,
        cx: &mut Context<Self>,
    ) {
        self.data.tab = DataTab::Browse;
        self.data.warning = None;
        self.data.table = Some(label);
        self.data.browse = Browse {
            filters,
            ..Browse::default()
        };
        self.data.revealed.clear();
        self.data.filter_error = None;
        self.load_rows(cx);
    }

    /// The open session for connection `id`, to use on a worker thread.
    pub(crate) fn data_session(&self, id: &str) -> SessionRef {
        SessionRef {
            live: self.data.live.clone(),
            cancellers: self.data.cancellers.clone(),
            id: id.to_string(),
            saved: self.data.saved.iter().find(|s| s.id == id).cloned(),
            root: self.data_root(),
        }
    }

    /// The selected connection's kind of database.
    pub(crate) fn data_kind(&self) -> Kind {
        self.data
            .selected
            .as_ref()
            .and_then(|id| self.data.saved.iter().find(|s| &s.id == id))
            .map_or(Kind::Postgres, |s| s.kind)
    }

    /// Whether the selected connection is production, so heavy reads wait for a go.
    pub(crate) fn data_prod(&self) -> bool {
        self.data
            .selected
            .as_ref()
            .and_then(|id| self.data.saved.iter().find(|s| &s.id == id))
            .is_some_and(|s| s.env == Env::Prod)
    }

    /// Stop what the selected connection is running.
    pub(crate) fn cancel_data(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.data.selected.clone() else {
            return;
        };
        let cancellers = self.data.cancellers.clone();
        std::thread::spawn(move || {
            if let Some(c) = cancellers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
            {
                c.cancel();
            }
        });
        cx.notify();
    }

    pub(crate) fn load_rows(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(table)) = (self.data.selected.clone(), self.data.table_of().cloned())
        else {
            return;
        };
        let browse = self.data.browse.clone();
        let kind = self
            .data
            .saved
            .iter()
            .find(|s| s.id == id)
            .map_or(Kind::Postgres, |s| s.kind);
        // Sorting or filtering can read the whole table; asked about once per statement.
        let whole = db::engine::select_sql(kind, &table, &browse, false).0;
        let heavy_check = self.data_prod()
            && (!browse.filters.is_empty() || browse.sort.is_some())
            && !self.data.cleared.contains(&whole);
        let tables = self
            .data
            .conns
            .get(&id)
            .map(|c| c.tables.clone())
            .unwrap_or_default();
        let session = self.data_session(&id);
        self.data.loading = true;
        self.data.rows_error = None;
        self.data.warning = None;
        cx.notify();
        let label = table.label();
        self.bg(
            move || {
                session.with(|s| {
                    if heavy_check {
                        let (paged, _) = db::engine::browse_sql(kind, &table, &browse, false);
                        if let Ok(Some(warning)) = s.scan_warning(&paged, &tables) {
                            return Err(Stop::Heavy(warning));
                        }
                    }
                    s.browse(&table, &browse)
                        .map(|rows| (rows, table.clone()))
                        .map_err(Stop::Failed)
                })
            },
            move |this, result, cx| {
                if this.data.table.as_deref() != Some(label.as_str()) {
                    return;
                }
                this.data.loading = false;
                this.data.row = None;
                match result {
                    Ok((rows, table)) => {
                        let sort = this.data.browse.sort.clone();
                        let revealed = this.data.revealed.clone();
                        this.data.grid.update(cx, |state, cx| {
                            state.delegate_mut().fill(&table, &rows, sort, &revealed);
                            state.clear_selection(cx);
                            state.refresh(cx);
                        });
                        this.data.rows = Some(rows);
                    }
                    Err(Stop::Heavy(text)) => {
                        this.data.rows = None;
                        this.data.warning = Some(Warning {
                            text,
                            sql: whole,
                            then: Then::Browse,
                        });
                    }
                    Err(Stop::Failed(e)) => {
                        this.data.rows = None;
                        this.data.rows_error = Some(e);
                    }
                }
                cx.notify();
            },
        );
    }

    /// Export the open table's rows, with its filters and sort, as CSV.
    fn export_browse(&mut self, cx: &mut Context<Self>) {
        let Some(table) = self.data.table_of().cloned() else {
            return;
        };
        let sql = db::engine::select_sql(self.data_kind(), &table, &self.data.browse, false).0;
        let hidden = table
            .columns
            .iter()
            .filter(|c| c.sensitive() && !self.data.revealed.contains(&c.name))
            .map(|c| c.name.clone())
            .collect();
        self.export_csv(sql, table.name.clone(), hidden, cx);
    }

    /// Do what the warning held back, and don't ask about that statement again.
    fn run_anyway(&mut self, cx: &mut Context<Self>) {
        let Some(w) = self.data.warning.take() else {
            return;
        };
        self.data.cleared.insert(w.sql.trim().to_string());
        match w.then {
            Then::Browse => self.load_rows(cx),
            Then::Query => self.run_query(true, cx),
            Then::Count => self.count_rows(true, cx),
            Then::Column(name) => self.exact_column(&name, true, cx),
            Then::Export(path, hidden) => self.write_csv(w.sql, path, hidden, true, cx),
        }
    }

    /// The warning held over a run on production, with Run anyway and Cancel.
    pub(crate) fn data_warning_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let w = self.data.warning.as_ref()?;
        Some(
            div()
                .mx_3()
                .my_2()
                .px_3()
                .py_2()
                .flex_none()
                .flex()
                .items_start()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(rgba((c_amber() << 8) | 0x66))
                .bg(rgba((c_amber() << 8) | 0x14))
                .child(
                    div()
                        .pt(px(2.))
                        .child(icon(IconName::TriangleAlert, 14., c_amber())),
                )
                .child(
                    div()
                        .flex_1()
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Heavy read on production"),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .child(w.text.clone()),
                        )
                        .child(
                            div()
                                .pt_1()
                                .flex()
                                .gap_2()
                                .child(
                                    small_button("data-warn-run", "Run anyway", false).on_click(
                                        cx.listener(|this, _, _, cx| this.run_anyway(cx)),
                                    ),
                                )
                                .child(small_button("data-warn-cancel", "Cancel", false).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.data.warning = None;
                                        cx.notify();
                                    }),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Add the condition typed in the filter bar. Returns whether it was understood.
    fn add_filter_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if text.is_empty() {
            return false;
        }
        let Some(table) = self.data.table_of() else {
            return false;
        };
        match parse_filter(text, &table.columns) {
            Ok(filter) => {
                self.data.filter_error = None;
                self.data.browse.filters.push(filter);
                self.data.browse.offset = 0;
                self.load_rows(cx);
                true
            }
            Err(e) => {
                self.data.filter_error = Some(e);
                false
            }
        }
    }

    fn toggle_reveal(&mut self, column: &str, cx: &mut Context<Self>) {
        if !self.data.revealed.remove(column) {
            self.data.revealed.insert(column.to_string());
        }
        if let (Some(rows), Some(table)) = (self.data.rows.clone(), self.data.table_of().cloned()) {
            let sort = self.data.browse.sort.clone();
            let revealed = self.data.revealed.clone();
            self.data.grid.update(cx, |state, cx| {
                state.delegate_mut().fill(&table, &rows, sort, &revealed);
                cx.notify();
            });
        }
        cx.notify();
    }

    // ---------- views ----------

    pub(crate) fn data_view(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.all_scope() {
            return shell::mode_frame(
                Mode::Data,
                "Data",
                Some(self.mode_project_chip(cx)),
                "Pick a project to see its databases.",
                self.project_chooser(Mode::Data, cx),
                None,
            );
        }
        let Some(project) = self
            .data
            .project
            .and_then(|id| self.state.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            return shell::mode_frame(
                Mode::Data,
                "Data",
                None,
                "Look inside the project's databases.",
                div().child(empty_hint(
                    "data-empty",
                    IconName::FolderPlus,
                    "No project yet",
                    "Add a project to see its databases.",
                )),
                None,
            );
        };
        let conn = self
            .data
            .selected
            .as_ref()
            .and_then(|id| self.data.conns.get(id));
        let read_only = conn
            .and_then(|c| c.opened.as_ref())
            .is_some_and(|o| o.read_only);
        let header = div()
            .h(px(40.))
            .flex_none()
            .relative()
            .px(px(10.))
            .flex()
            .items_end()
            .gap_1()
            .child(shell::strip_fill("data", pane(c_sheet())))
            .child(
                div()
                    .h(px(28.))
                    .mb(px(3.))
                    .px_2()
                    .mr_1()
                    .flex()
                    .items_center()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Data"),
            )
            .children(
                [
                    (DataTab::Browse, "Browse", IconName::Table),
                    (DataTab::Query, "Query", IconName::SquareTerminal),
                    (DataTab::Diagram, "Diagram", IconName::Network),
                ]
                .into_iter()
                .map(|(tab, label, glyph)| {
                    shell::strip_tab(
                        SharedString::from(format!("data-tab-{label}")),
                        "data",
                        self.data.tab == tab,
                    )
                    .child(icon(glyph, 12., c_muted()))
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_data_tab(tab, window, cx);
                    }))
                }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .mb(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(read_only, |el| {
                        el.child(
                            div()
                                .h(px(24.))
                                .px(px(9.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .rounded_full()
                                .border_1()
                                .border_color(divider())
                                .text_size(px(11.5))
                                .text_color(rgb(c_secondary()))
                                .child(div().size(px(6.)).rounded_full().bg(rgb(c_green())))
                                .child("Read-only"),
                        )
                    })
                    .child(
                        self.ghost_button_sized(
                            "data-rescan",
                            IconName::RefreshCw,
                            "Refresh: read the rows again, and look through the project's files",
                            26.,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.scan_data(cx);
                            if this.data.table.is_some() {
                                this.load_rows(cx);
                            } else if let Some(id) = this.data.selected.clone() {
                                // Read the schema again, as it may have changed.
                                this.open_data_conn(&id, cx);
                            }
                        })),
                    ),
            );
        let main = match conn {
            None => self.data_found_page(&project, cx),
            Some(c) if c.connecting => centered("Connecting read-only…"),
            Some(c) if c.error.is_some() => {
                self.data_error_page(c.error.clone().unwrap_or_default(), cx)
            }
            Some(c) if c.opened.is_none() => self.data_hold_page(cx),
            Some(c) => match (self.data.tab, self.data.table_of()) {
                (DataTab::Query, _) => self.data_query_page(cx),
                (DataTab::Diagram, _) => self.data_diagram(c, cx),
                (DataTab::Browse, Some(table)) => self.data_table_page(table, cx),
                (DataTab::Browse, None) => self.data_overview(c, cx),
            },
        };
        let body = shell::island(
            div().size_full().flex().child(self.data_side(cx)).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .bg(glass_fill(c_canvas()))
                    .child(main),
            ),
            rgba(0),
            [true, true, false, false],
        );
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(header)
            .child(div().flex_1().min_h_0().flex().child(body.size_full()))
            .into_any_element()
    }

    fn data_side(&self, cx: &mut Context<Self>) -> AnyElement {
        let pid = self.data.project;
        let mine: Vec<&Saved> = self
            .data
            .saved
            .iter()
            .filter(|s| Some(s.project) == pid)
            .collect();
        let mut list = div().v_flex().gap(px(2.));
        list = list.child(
            label_row(format!(
                "CONNECTIONS{}",
                if mine.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", mine.len())
                }
            ))
            .child(
                self.ghost_button_sized(
                    "data-add",
                    IconName::Plus,
                    "Add a connection by URL or file",
                    22.,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.data.adding = !this.data.adding;
                    this.data.add_error = None;
                    if this.data.adding {
                        this.data.url.update(cx, |i, cx| i.focus(window, cx));
                    }
                    cx.notify();
                })),
            ),
        );
        if self.data.adding {
            list = list.child(
                div()
                    .px_3()
                    .pb_2()
                    .v_flex()
                    .gap_1()
                    .child(Input::new(&self.data.url).bordered(true).h(px(28.)))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(match self.data.add_error {
                                Some(_) => c_red(),
                                None => c_muted(),
                            }))
                            .child(self.data.add_error.clone().unwrap_or_else(|| {
                                "Paste a URL, then Enter. A password goes to the Keychain, never into Runo's files.".into()
                            })),
                    ),
            );
        }
        if mine.is_empty() && !self.data.adding {
            list = list.child(
                div()
                    .px_3()
                    .pb_2()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child("None yet. Connect to one found below, or add one by URL."),
            );
        }
        for (i, saved) in mine.iter().enumerate() {
            let on = self.data.selected.as_deref() == Some(saved.id.as_str());
            let id = saved.id.clone();
            let forget = saved.id.clone();
            let state = self.data.conns.get(&saved.id);
            let sub = match state {
                Some(c) if c.connecting => "Connecting…".to_string(),
                Some(c) if c.error.is_some() => "Couldn't connect".to_string(),
                Some(c) => c
                    .opened
                    .as_ref()
                    .map(|o| format!("{} · {} tables", o.server, c.tables.len()))
                    .unwrap_or_else(|| saved.shown.clone()),
                None => saved.shown.clone(),
            };
            list = list.child(
                div()
                    .id(("data-conn", i))
                    .mx(px(6.))
                    .px_2()
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(on, |el| el.bg(tint(c_active())))
                    .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_data_conn(&id, cx);
                    }))
                    .child(kind_badge(saved.kind, 22.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(saved.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(c_muted()))
                                    .truncate()
                                    .child(sub),
                            ),
                    )
                    .child(env_chip(saved.env))
                    .when(on, |el| {
                        el.child(
                            self.ghost_button_sized(
                                ("data-disconnect", i),
                                IconName::X,
                                "Disconnect",
                                20.,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.disconnect_data_conn(&forget, cx);
                                },
                            )),
                        )
                    }),
            );
        }
        // The open connection's tables.
        if let Some(conn) = self
            .data
            .selected
            .as_ref()
            .and_then(|id| self.data.conns.get(id))
            .filter(|c| c.opened.is_some())
        {
            let query = self.data.search.read(cx).value().trim().to_lowercase();
            let tables: Vec<&Table> = conn
                .tables
                .iter()
                .filter(|t| query.is_empty() || t.label().to_lowercase().contains(&query))
                .collect();
            if self.data.tab == DataTab::Query {
                list = self.data_query_side(list, cx);
            }
            list = list
                .child(label_row(format!("TABLES · {}", conn.tables.len())))
                .child(
                    div()
                        .px_3()
                        .pb_1()
                        .child(Input::new(&self.data.search).bordered(true).h(px(26.))),
                );
            for (i, table) in tables.iter().enumerate() {
                let label = table.label();
                let on = self.data.table.as_deref() == Some(label.as_str());
                let pick = label.clone();
                list = list.child(
                    div()
                        .id(("data-table", i))
                        .mx(px(6.))
                        .px_2()
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_size(px(12.5))
                        .when(on, |el| el.bg(tint(c_active())))
                        .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.data.tab == DataTab::Query {
                                // Start a query on it rather than leaving the editor.
                                let start = this
                                    .data
                                    .table_by_label(&pick)
                                    .map(|t| crate::dataquery::starter(this.data_kind(), t));
                                if let Some(sql) = start {
                                    this.edit_query(&sql, window, cx);
                                }
                            } else {
                                this.pick_table(pick.clone(), Vec::new(), cx);
                            }
                        }))
                        .child(icon(
                            if table.view {
                                IconName::Eye
                            } else {
                                IconName::Table
                            },
                            12.,
                            c_muted(),
                        ))
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        .children(table.estimate.map(|n| {
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(rgb(c_muted()))
                                .font_family(self.mono_font.clone())
                                .child(short_count(n))
                        })),
                );
            }
        }
        // What the project's files name that isn't saved yet.
        let saved_keys: HashSet<&str> = mine
            .iter()
            .filter_map(|s| match &s.source {
                db::saved::Source::Found { key } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        let unsaved: Vec<&Found> = self
            .data
            .found
            .iter()
            .filter(|f| !saved_keys.contains(f.key.as_str()))
            .collect();
        if !unsaved.is_empty() && !mine.is_empty() {
            list = list.child(label_row(format!(
                "FOUND IN THE PROJECT · {}",
                unsaved.len()
            )));
            for (i, f) in unsaved.iter().enumerate() {
                let key = f.key.clone();
                list = list.child(
                    div()
                        .id(("data-found-side", i))
                        .mx(px(6.))
                        .px_2()
                        .py(px(5.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.connect_found(&key, cx);
                        }))
                        .child(kind_badge(f.kind, 18.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(c_secondary()))
                                .child(f.name.clone()),
                        )
                        .child(env_chip(f.env)),
                );
            }
        }
        div()
            .w(px(SIDE_WIDTH))
            .flex_none()
            .h_full()
            .v_flex()
            .bg(pane(c_sheet()))
            .child(
                div()
                    .id("data-side")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(12.))
                    .child(list),
            )
            .into_any_element()
    }

    /// The first page: every database the project's files name.
    fn data_found_page(&self, project: &Project, cx: &mut Context<Self>) -> AnyElement {
        let mut cards = div().v_flex().gap_2().max_w(px(820.));
        for (i, f) in self.data.found.iter().enumerate() {
            let key = f.key.clone();
            let mut sources = div().flex().flex_wrap().gap_1().mt_1();
            for s in &f.sources {
                sources = sources.child(
                    div()
                        .px(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(divider())
                        .text_size(px(10.5))
                        .text_color(rgb(c_muted()))
                        .child(s.clone()),
                );
            }
            if f.running == Some(false) {
                sources = sources.child(
                    div()
                        .px(px(6.))
                        .rounded(px(4.))
                        .bg(tint(c_subtle()))
                        .text_size(px(10.5))
                        .text_color(rgb(c_amber()))
                        .child(if f.kind == Kind::Sqlite {
                            "File missing"
                        } else {
                            "Not running"
                        }),
                );
            }
            let action: AnyElement = if !f.kind.supported() {
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(format!("{} support is coming", f.kind.label()))
                    .into_any_element()
            } else {
                let label = match (f.kind, f.env) {
                    (Kind::Sqlite, _) => "Open",
                    (_, Env::Prod) => "Connect read-only",
                    _ => "Connect",
                };
                small_button(("data-connect", i), label, i == 0 && f.env == Env::Local)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.connect_found(&key, cx);
                    }))
                    .into_any_element()
            };
            cards = cards.child(
                div()
                    .p_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .bg(tint(c_raised()))
                    .child(kind_badge(f.kind, 26.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(f.name.clone()),
                                    )
                                    .child(env_chip(f.env)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(c_muted()))
                                    .font_family(self.mono_font.clone())
                                    .truncate()
                                    .child(f.shown.clone()),
                            )
                            .child(sources),
                    )
                    .child(action),
            );
        }
        let intro = if self.data.scanning && self.data.found.is_empty() {
            "Reading the project's files…".to_string()
        } else if self.data.found.is_empty() {
            "Runo looked in .env files, Docker Compose, Prisma, Drizzle, Rails' database.yml and for SQLite files, and found nothing. Add one by URL with the + beside Connections.".to_string()
        } else {
            "Found by reading the project's files. Passwords stay where they are; Runo reads them when it connects, and every session is read-only.".to_string()
        };
        div()
            .id("data-found")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .p_6()
                    .v_flex()
                    .gap_4()
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Databases in {}", project.name)),
                            )
                            .child(
                                div()
                                    .max_w(px(640.))
                                    .text_size(px(12.5))
                                    .text_color(rgb(c_muted()))
                                    .child(intro),
                            ),
                    )
                    .child(cards),
            )
            .into_any_element()
    }

    fn data_error_page(&self, error: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .max_w(px(520.))
                    .v_flex()
                    .items_center()
                    .gap_3()
                    .child(icon(IconName::Unplug, 22., c_muted()))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Couldn't connect"),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .text_center()
                            .child(error),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(small_button("data-retry", "Try again", true).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.data.selected.clone() {
                                        this.open_data_conn(&id, cx);
                                    }
                                }),
                            ))
                            .child(self.data_remove_button(cx)),
                    ),
            )
            .into_any_element()
    }

    /// Takes the selected connection out of the list; a second press confirms.
    fn data_remove_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let armed =
            self.data.confirm_remove.is_some() && self.data.confirm_remove == self.data.selected;
        small_button(
            "data-remove",
            if armed {
                "Press again to remove"
            } else {
                "Remove from the list"
            },
            false,
        )
        .when(armed, |el| el.text_color(rgb(c_red())))
        .on_click(cx.listener(move |this, _, _, cx| {
            let Some(id) = this.data.selected.clone() else {
                return;
            };
            if this.data.confirm_remove.as_deref() == Some(id.as_str()) {
                this.data.confirm_remove = None;
                this.forget_data_conn(&id, cx);
            } else {
                this.data.confirm_remove = Some(id);
                cx.notify();
            }
        }))
    }

    /// A connection that isn't open: production waits here for a go.
    fn data_hold_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let saved = self
            .data
            .selected
            .as_ref()
            .and_then(|id| self.data.saved.iter().find(|s| &s.id == id));
        let Some(saved) = saved else {
            return centered("Not connected.");
        };
        let prod = saved.env == Env::Prod;
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .max_w(px(440.))
                    .v_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(prod, |el| el.child(icon(IconName::ShieldAlert, 15., c_red())))
                            .child(if prod {
                                format!("Connect to {}, a production database?", saved.name)
                            } else {
                                format!("{} isn't connected.", saved.name)
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_muted()))
                            .font_family(self.mono_font.clone())
                            .child(saved.shown.clone()),
                    )
                    .when(prod, |el| {
                        el.child(
                            div()
                                .text_size(px(12.5))
                                .text_color(rgb(c_secondary()))
                                .child("It opens read-only, so nothing here can change it. Reads that would scan a big table ask first, and agents get hidden columns masked."),
                        )
                    })
                    .child(
                        div().flex().child(
                            small_button("data-hold-connect", if prod { "Connect read-only" } else { "Connect" }, true)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.data.selected.clone() {
                                        this.open_data_conn(&id, cx);
                                    }
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// A connected database before a table is picked: what it is, and its biggest tables.
    fn data_overview(&self, conn: &Conn, cx: &mut Context<Self>) -> AnyElement {
        let opened = conn.opened.clone();
        let saved = self
            .data
            .selected
            .as_ref()
            .and_then(|id| self.data.saved.iter().find(|s| &s.id == id));
        let mut biggest: Vec<&Table> = conn.tables.iter().filter(|t| !t.view).collect();
        biggest.sort_by_key(|t| std::cmp::Reverse(t.estimate.unwrap_or(0)));
        let mut list = div().v_flex().gap_1().max_w(px(560.));
        for (i, t) in biggest.iter().take(8).enumerate() {
            let label = t.label();
            let pick = label.clone();
            list = list.child(
                div()
                    .id(("data-big", i))
                    .px_3()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_table(pick.clone(), Vec::new(), cx);
                    }))
                    .child(icon(IconName::Table, 13., c_muted()))
                    .child(div().flex_1().text_size(px(12.5)).child(label))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("{} columns", t.columns.len())),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .font_family(self.mono_font.clone())
                            .text_right()
                            .child(t.estimate.map(short_count).unwrap_or_default()),
                    ),
            );
        }
        let views = conn.tables.iter().filter(|t| t.view).count();
        div()
            .id("data-overview")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .p_6()
                    .v_flex()
                    .gap_4()
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(saved.map(|s| s.name.clone()).unwrap_or_default()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(c_muted()))
                                    .font_family(self.mono_font.clone())
                                    .truncate()
                                    .child(format!(
                                        "{} · {}",
                                        opened.as_ref().map(|o| o.server.clone()).unwrap_or_default(),
                                        saved.map(|s| s.shown.clone()).unwrap_or_default()
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(12.5))
                                    .text_color(rgb(c_muted()))
                                    .child(icon(IconName::ShieldCheck, 13., c_green()))
                                    .child(format!(
                                        "Read-only, confirmed by the database. {} tables{}. Pick one on the left.",
                                        conn.tables.len() - views,
                                        match views {
                                            0 => String::new(),
                                            1 => " and 1 view".into(),
                                            n => format!(" and {n} views"),
                                        }
                                    )),
                            )
                            .children(opened.as_ref().map(|o| {
                                let local = saved.is_none_or(|s| {
                                    db::local_host(&db::url_parts(&s.shown).0)
                                });
                                transport_row(o.transport, local)
                            }))
                            .when(opened.as_ref().is_some_and(|o| o.superuser), |el| {
                                el.child(note_row(
                                    IconName::ShieldAlert,
                                    c_amber(),
                                    "Signed in as a superuser. Runo keeps the session read-only, but a role that can only read these tables is safer, above all for agents.",
                                ))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(small_button("data-disconnect-page", "Disconnect", false).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(id) = this.data.selected.clone() {
                                        this.disconnect_data_conn(&id, cx);
                                    }
                                }),
                            ))
                            .child(self.data_remove_button(cx))
                            .child(small_button("data-ask-db", "Ask about this database", true).on_click(
                                cx.listener(|this, _, window, cx| this.ask_about_database(window, cx)),
                            )),
                    )
                    .when(!biggest.is_empty(), |el| {
                        el.child(label_row("LARGEST TABLES").px_0()).child(list)
                    })
                    .children(self.agent_activity()),
            )
            .into_any_element()
    }

    fn data_table_page(&self, table: &Table, cx: &mut Context<Self>) -> AnyElement {
        // Filter chips, then the box to type another.
        let mut chips = div().flex().flex_wrap().items_center().gap_1();
        for (i, f) in self.data.browse.filters.iter().enumerate() {
            let text = if f.op.takes_value() {
                format!("{} {} {}", f.column, f.op.label(), f.value)
            } else {
                format!("{} {}", f.column, f.op.label())
            };
            chips = chips.child(
                div()
                    .id(("data-chip", i))
                    .h(px(24.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(divider())
                    .bg(tint(c_subtle()))
                    .text_size(px(11.5))
                    .font_family(self.mono_font.clone())
                    .child(text)
                    .child(
                        self.ghost_button_sized(
                            ("data-chip-x", i),
                            IconName::X,
                            "Remove this filter",
                            18.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if i < this.data.browse.filters.len() {
                                this.data.browse.filters.remove(i);
                                this.data.browse.offset = 0;
                                this.load_rows(cx);
                            }
                        })),
                    ),
            );
        }
        let bar = div()
            .px_3()
            .py_2()
            .flex_none()
            .v_flex()
            .gap_1()
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(IconName::ListFilter, 13., c_muted()))
                    .child(chips)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.))
                            .child(Input::new(&self.data.filter).bordered(false).h(px(26.))),
                    )
                    .children(self.data.browse.sort.as_ref().map(|(col, desc)| {
                        div()
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .child(format!("Sorted by {col} {}", if *desc { "↓" } else { "↑" }))
                    }))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                self.ghost_button_sized(
                                    "data-columns",
                                    IconName::Activity,
                                    "Columns: types, keys and statistics",
                                    24.,
                                )
                                .when(self.data.stats.open, |el| el.bg(tint(c_active())))
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_stats(cx))),
                            )
                            .child(
                                self.ghost_button_sized(
                                    "data-to-query",
                                    IconName::SquareTerminal,
                                    "Write a query on this table",
                                    24.,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        let sql = this.data.table_of().map(|t| {
                                            crate::dataquery::starter(this.data_kind(), t)
                                        });
                                        if let Some(sql) = sql {
                                            this.edit_query(&sql, window, cx);
                                        }
                                    },
                                )),
                            )
                            .child(
                                self.ghost_button_sized(
                                    "data-export",
                                    IconName::Download,
                                    "Export these rows as CSV (hidden columns stay hidden)",
                                    24.,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.export_browse(cx))),
                            )
                            .child(
                                self.ghost_button_sized(
                                    "data-ask",
                                    IconName::Sparkles,
                                    "Ask an agent about this table",
                                    24.,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.ask_about_table(window, cx),
                                )),
                            ),
                    ),
            )
            .children(self.data.filter_error.clone().map(|e| {
                div()
                    .pl(px(21.))
                    .text_size(px(11.5))
                    .text_color(rgb(c_red()))
                    .child(e)
            }));
        let grid: AnyElement =
            if let Some(e) = &self.data.rows_error {
                div()
                    .size_full()
                    .p_6()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("The database refused that"),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(c_secondary()))
                            .child(e.clone()),
                    )
                    .when(!self.data.browse.filters.is_empty(), |el| {
                        el.child(div().text_size(px(12.)).text_color(rgb(c_muted())).child(
                            "A filter may not fit its column's type. Remove it above with ×.",
                        ))
                    })
                    .child(
                        div().pt_2().flex().child(
                            small_button("data-rows-retry", "Try again", false)
                                .on_click(cx.listener(|this, _, _, cx| this.load_rows(cx))),
                        ),
                    )
                    .into_any_element()
            } else if self.data.warning.is_some() && self.data.rows.is_none() {
                centered("Nothing has run yet.")
            } else if self.data.rows.is_none() {
                centered("Loading rows…")
            } else {
                DataTable::new(&self.data.grid)
                    .with_size(gpui_kit::component::Size::Small)
                    .stripe(false)
                    .bordered(false)
                    .into_any_element()
            };
        let status = self.data.rows.as_ref().map(|rows| {
            let first = self.data.browse.offset + 1;
            let last = self.data.browse.offset + rows.rows.len();
            let range = match rows.rows.len() {
                0 => "No rows match".to_string(),
                1 if first == 1 && !rows.more => "1 row".to_string(),
                n if first == 1 && !rows.more => format!("{n} rows"),
                _ if rows.more => match table.estimate {
                    Some(n) if self.data.browse.filters.is_empty() && n > last as i64 => {
                        format!("Rows {first}–{last} of about {}", short_count(n))
                    }
                    _ => format!("Rows {first}–{last}, more after"),
                },
                _ => format!("Rows {first}–{last}, the last"),
            };
            let sql = rows.sql.clone();
            let back = self.data.browse.offset > 0;
            div()
                .h(px(32.))
                .flex_none()
                .px_3()
                .flex()
                .items_center()
                .gap_3()
                .border_t_1()
                .border_color(divider())
                .text_size(px(11.5))
                .text_color(rgb(c_muted()))
                .child(div().flex_none().child(range))
                .child(match self.exact_count() {
                    Some(Ok(n)) => div()
                        .flex_none()
                        .text_color(rgb(c_secondary()))
                        .child(format!("{} counted", crate::datastats::thousands(*n)))
                        .into_any_element(),
                    Some(Err(e)) => div()
                        .flex_none()
                        .max_w(px(240.))
                        .truncate()
                        .text_color(rgb(c_red()))
                        .child(format!("Count failed: {e}"))
                        .into_any_element(),
                    None if self.data.stats.counting => {
                        div().flex_none().child("Counting…").into_any_element()
                    }
                    None if rows.more || self.data.browse.offset > 0 => div()
                        .id("data-count")
                        .flex_none()
                        .cursor_pointer()
                        .text_color(rgb(c_blue()))
                        .hover(|el| el.underline())
                        .on_click(cx.listener(|this, _, _, cx| this.count_rows(false, cx)))
                        .child("Count exactly")
                        .into_any_element(),
                    None => div().into_any_element(),
                })
                .child(div().flex_none().child(format!("{} ms", rows.millis)))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            self.ghost_button_sized(
                                "data-prev",
                                IconName::ChevronLeft,
                                "Previous page",
                                22.,
                            )
                            .when(!back, |el| el.opacity(0.35))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.data.browse.offset > 0 {
                                    this.data.browse.offset =
                                        this.data.browse.offset.saturating_sub(db::PAGE);
                                    this.load_rows(cx);
                                }
                            })),
                        )
                        .child(
                            self.ghost_button_sized(
                                "data-next",
                                IconName::ChevronRight,
                                "Next page",
                                22.,
                            )
                            .when(!rows.more, |el| el.opacity(0.35))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.data.rows.as_ref().is_some_and(|r| r.more) {
                                    this.data.browse.offset += db::PAGE;
                                    this.load_rows(cx);
                                }
                            })),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(self.mono_font.clone())
                        .child(sql.clone()),
                )
                .child(
                    self.ghost_button_sized("data-copy-sql", IconName::Copy, "Copy this SQL", 22.)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(sql.clone()));
                        })),
                )
        });
        let center = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .v_flex()
            .child(bar)
            .children(self.data_warning_bar(cx))
            .children(
                self.data
                    .query
                    .note
                    .clone()
                    .filter(|_| self.data.tab == DataTab::Browse)
                    .map(|n| {
                        div()
                            .px_3()
                            .py_1()
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(rgb(c_secondary()))
                            .child(n)
                    }),
            )
            .child(div().flex_1().min_h_0().relative().child(grid).when(
                self.data.loading && self.data.rows.is_some(),
                |el| {
                    el.child(
                        div()
                            .absolute()
                            .top_2()
                            .right_3()
                            .px_2()
                            .rounded_md()
                            .bg(tint(c_raised()))
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child("Loading…"),
                    )
                },
            ))
            .children(status);
        div()
            .size_full()
            .flex()
            .child(center)
            .children(
                self.data_stats_panel(table, cx)
                    .or_else(|| self.data_inspector(table, cx)),
            )
            .into_any_element()
    }

    /// The picked row, one column per line, with its foreign keys as links.
    fn data_inspector(&self, table: &Table, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows = self.data.rows.as_ref()?;
        let i = self.data.row?;
        let row = rows.rows.get(i)?;
        let mut lines = div().v_flex().gap(px(1.));
        for (c, name) in rows.columns.iter().enumerate() {
            let cell = row.get(c).cloned().unwrap_or_else(Cell::null);
            let meta = table.columns.iter().find(|m| &m.name == name);
            let hidden = meta.is_some_and(|m| m.sensitive()) && !self.data.revealed.contains(name);
            let refers = meta
                .and_then(|m| m.refers.clone())
                .filter(|_| cell.kind != CellKind::Null);
            let shown = if hidden && cell.kind != CellKind::Null {
                mask(&cell.text)
            } else if cell.kind == CellKind::Json {
                pretty_json(&cell.text)
            } else {
                cell.text.clone()
            };
            let color = match (&refers, cell.kind) {
                (Some(_), _) => c_blue(),
                (_, CellKind::Null) => c_muted(),
                (_, CellKind::Number) => c_amber(),
                _ => c_text(),
            };
            let column = name.clone();
            let value = cell.text.clone();
            let value_el = div()
                .id(("data-value", c))
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .font_family(self.mono_font.clone())
                .text_color(rgb(color))
                .when(refers.is_some(), |el| el.cursor_pointer().underline())
                .child(shown);
            let value_el = if let Some((target, target_col)) = refers {
                value_el.on_click(cx.listener(move |this, _, _, cx| {
                    let label = this
                        .data
                        .selected
                        .as_ref()
                        .and_then(|id| this.data.conns.get(id))
                        .and_then(|c| {
                            c.tables
                                .iter()
                                .find(|t| t.label() == target || t.name == target)
                                .map(|t| t.label())
                        });
                    if let Some(label) = label {
                        this.pick_table(
                            label,
                            vec![Filter {
                                column: target_col.clone(),
                                op: Op::Eq,
                                value: value.clone(),
                            }],
                            cx,
                        );
                    }
                }))
            } else {
                value_el
            };
            lines = lines.child(
                div()
                    .py(px(5.))
                    .flex()
                    .items_start()
                    .gap_2()
                    .border_b_1()
                    .border_color(divider())
                    .child(
                        div()
                            .w(px(104.))
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(rgb(c_muted()))
                            .truncate()
                            .child(name.clone()),
                    )
                    .child(value_el)
                    .when(meta.is_some_and(|m| m.sensitive()), |el| {
                        el.child(
                            self.ghost_button_sized(
                                ("data-reveal", c),
                                if hidden {
                                    IconName::Eye
                                } else {
                                    IconName::EyeOff
                                },
                                "Show or hide this column's values",
                                20.,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.toggle_reveal(&column, cx);
                                },
                            )),
                        )
                    }),
            );
        }
        // Tables whose foreign keys point at this one.
        let pk = table
            .columns
            .iter()
            .find(|c| c.primary)
            .map(|c| c.name.clone());
        let pk_value = pk.as_ref().and_then(|pk| {
            rows.columns
                .iter()
                .position(|c| c == pk)
                .and_then(|i| row.get(i))
                .map(|c| c.text.clone())
        });
        let title = match (&pk, &pk_value) {
            (Some(pk), Some(v)) => format!("{} · {pk} {v}", table.label()),
            _ => format!(
                "{} · row {}",
                table.label(),
                self.data.browse.offset + i + 1
            ),
        };
        let copied = row_json(&rows.columns, row, |name| {
            table
                .columns
                .iter()
                .any(|m| &m.name == name && m.sensitive())
                && !self.data.revealed.contains(name)
        });
        let mut linked = div().v_flex();
        let mut any = false;
        if let (Some(pk), Some(pk_value)) = (pk, pk_value) {
            let tables = self
                .data
                .selected
                .as_ref()
                .and_then(|id| self.data.conns.get(id))
                .map(|c| c.tables.clone())
                .unwrap_or_default();
            for (n, other) in tables.iter().enumerate() {
                for col in &other.columns {
                    let Some((to, to_col)) = &col.refers else {
                        continue;
                    };
                    if (to != &table.label() && to != &table.name) || to_col != &pk {
                        continue;
                    }
                    any = true;
                    let label = other.label();
                    let filter = Filter {
                        column: col.name.clone(),
                        op: Op::Eq,
                        value: pk_value.clone(),
                    };
                    linked = linked.child(
                        div()
                            .id(SharedString::from(format!("data-linked-{n}-{}", col.name)))
                            .py(px(4.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .hover(|el| el.text_color(rgb(c_blue())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pick_table(label.clone(), vec![filter.clone()], cx);
                            }))
                            .child(icon(IconName::ArrowUpRight, 12., c_blue()))
                            .child(format!(
                                "{} where {} = {}",
                                other.label(),
                                col.name,
                                pk_value
                            )),
                    );
                }
            }
        }
        Some(
            div()
                .id("data-inspector")
                .w(px(INSPECTOR_WIDTH))
                .flex_none()
                .h_full()
                .overflow_y_scroll()
                .border_l_1()
                .border_color(divider())
                .bg(pane(c_sheet()))
                .p_3()
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(title),
                        )
                        .child(
                            self.ghost_button_sized(
                                "data-inspector-copy",
                                IconName::Copy,
                                "Copy this row as JSON",
                                22.,
                            )
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copied.clone(),
                                    ));
                                },
                            )),
                        )
                        .child(
                            self.ghost_button_sized(
                                "data-inspector-close",
                                IconName::X,
                                "Close",
                                22.,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.row = None;
                                this.data.grid.update(cx, |s, cx| s.clear_selection(cx));
                                cx.notify();
                            })),
                        ),
                )
                .child(lines)
                .when(any, |el| {
                    el.child(label_row("LINKED ROWS").px_0()).child(linked)
                })
                .into_any_element(),
        )
    }
}

/// The grid's rows and columns, given to gpui-kit's table.
pub(crate) struct Grid {
    columns: Vec<GridCol>,
    rows: Vec<Vec<Cell>>,
    sort: Option<(String, bool)>,
    mono: SharedString,
    ui_tx: Sender<ship::UiJob>,
}

struct GridCol {
    name: String,
    data_type: String,
    hidden: bool,
    link: bool,
    width: f32,
}

impl Grid {
    pub(crate) fn empty(ui_tx: Sender<ship::UiJob>, mono: SharedString) -> Self {
        Grid {
            columns: Vec::new(),
            rows: Vec::new(),
            sort: None,
            mono,
            ui_tx,
        }
    }

    pub(crate) fn fill(
        &mut self,
        table: &Table,
        rows: &Rows,
        sort: Option<(String, bool)>,
        revealed: &HashSet<String>,
    ) {
        self.columns = rows
            .columns
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let meta = table.columns.iter().find(|c| &c.name == name);
                let longest = rows
                    .rows
                    .iter()
                    .take(50)
                    .map(|r| r.get(i).map_or(4, |c| c.text.chars().count().min(60)))
                    .max()
                    .unwrap_or(4);
                // The header holds the name, its type in small type and the sort arrow.
                let data_type = meta
                    .map(|m| db::short_type(&m.data_type))
                    .unwrap_or_default();
                let header =
                    name.chars().count() as f32 * 7.5 + data_type.len().min(16) as f32 * 6.2 + 44.;
                GridCol {
                    name: name.clone(),
                    data_type,
                    hidden: meta.is_some_and(|m| m.sensitive()) && !revealed.contains(name),
                    link: meta.is_some_and(|m| m.refers.is_some()),
                    width: (longest as f32 * 7.2 + 24.).max(header).clamp(64., 360.),
                }
            })
            .collect();
        self.rows = rows.rows.clone();
        self.sort = sort;
    }
}

impl TableDelegate for Grid {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> GridColumn {
        let c = &self.columns[col_ix];
        let sort = match &self.sort {
            Some((name, true)) if *name == c.name => ColumnSort::Descending,
            Some((name, false)) if *name == c.name => ColumnSort::Ascending,
            _ => ColumnSort::Default,
        };
        GridColumn::new(
            SharedString::from(c.name.clone()),
            SharedString::from(c.name.clone()),
        )
        .width(px(c.width))
        .sort(sort)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let c = &self.columns[col_ix];
        div()
            .size_full()
            .flex()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_secondary()))
                    .child(c.name.clone()),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(c_muted()))
                    .font_family(self.mono.clone())
                    .truncate()
                    .child(if c.hidden {
                        "hidden".to_string()
                    } else {
                        c.data_type.clone()
                    }),
            )
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let col = &self.columns[col_ix];
        let cell = self
            .rows
            .get(row_ix)
            .and_then(|r| r.get(col_ix))
            .cloned()
            .unwrap_or_else(Cell::null);
        let null = cell.kind == CellKind::Null;
        let text = if col.hidden && !null {
            mask(&cell.text)
        } else {
            // One line in the grid; the inspector shows the rest.
            cell.text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(200)
                .collect()
        };
        let color = match cell.kind {
            _ if col.link && !null => c_blue(),
            CellKind::Null => c_muted(),
            CellKind::Number => c_amber(),
            CellKind::Bool => c_green(),
            CellKind::Json | CellKind::Bytes => c_secondary(),
            CellKind::Text if col.hidden => c_muted(),
            CellKind::Text => c_text(),
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .when(cell.kind == CellKind::Number, |el| el.justify_end())
            .overflow_hidden()
            .text_size(px(12.))
            .font_family(self.mono.clone())
            .text_color(rgb(color))
            .when(null, |el| el.italic())
            .child(div().truncate().child(text))
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let Some(name) = self.columns.get(col_ix).map(|c| c.name.clone()) else {
            return;
        };
        let sort = match sort {
            ColumnSort::Default => None,
            ColumnSort::Ascending => Some((name, false)),
            ColumnSort::Descending => Some((name, true)),
        };
        let _ = self.ui_tx.send(Box::new(
            move |this: &mut RunoApp, cx: &mut Context<RunoApp>| {
                this.data.browse.sort = sort;
                this.data.browse.offset = 0;
                this.load_rows(cx);
            },
        ));
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.5))
            .text_color(rgb(c_muted()))
            .child("No rows match.")
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let hidden = self.columns.get(col_ix).is_some_and(|c| c.hidden);
        self.rows
            .get(row_ix)
            .and_then(|r| r.get(col_ix))
            .map(|c| {
                if hidden {
                    mask(&c.text)
                } else {
                    c.text.clone()
                }
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn cols(names: &[&str]) -> Vec<db::Column> {
        names
            .iter()
            .map(|n| db::Column {
                name: n.to_string(),
                ..db::Column::default()
            })
            .collect()
    }

    #[test]
    fn typed_filters_become_conditions() {
        let c = cols(&["status", "email", "email_verified_at", "total"]);
        let f = parse_filter("status = failed", &c).unwrap();
        assert_eq!(
            (f.column.as_str(), f.op, f.value.as_str()),
            ("status", Op::Eq, "failed")
        );
        let f = parse_filter("email_verified_at is null", &c).unwrap();
        assert_eq!((f.column.as_str(), f.op), ("email_verified_at", Op::IsNull));
        let f = parse_filter("EMAIL contains 'gmail.com'", &c).unwrap();
        assert_eq!(
            (f.column.as_str(), f.op, f.value.as_str()),
            ("email", Op::Contains, "gmail.com")
        );
        let f = parse_filter("total>1000", &c).unwrap();
        assert_eq!((f.op, f.value.as_str()), (Op::Gt, "1000"));
        let f = parse_filter("status != \"paid\"", &c).unwrap();
        assert_eq!((f.op, f.value.as_str()), (Op::Ne, "paid"));
        let f = parse_filter("status is not null", &c).unwrap();
        assert_eq!(f.op, Op::NotNull);
        assert!(parse_filter("nope = 1", &c)
            .unwrap_err()
            .contains("no column called nope"));
        assert!(parse_filter("status", &c).is_err());
        assert!(parse_filter("status =", &c).is_err());
        // A column name that only starts another word isn't a match.
        assert!(parse_filter("statuses = 1", &c).is_err());
    }

    #[test]
    fn number_columns_want_numbers() {
        let c = vec![db::Column {
            name: "total_cents".into(),
            data_type: "integer".into(),
            ..db::Column::default()
        }];
        assert!(parse_filter("total_cents > -12.5", &c).is_ok());
        assert!(parse_filter("total_cents contains 12", &c).is_ok());
        let e = parse_filter("total_cents > lots", &c).unwrap_err();
        assert!(e.contains("holds numbers"), "{e}");
        let e = parse_filter("totl=3", &c).unwrap_err();
        assert!(
            e.contains("called totl.") && e.contains("total_cents"),
            "{e}"
        );
    }

    #[test]
    fn rows_copy_as_json_with_hidden_values_masked() {
        let columns = vec![
            "id".to_string(),
            "email".into(),
            "meta".into(),
            "note".into(),
        ];
        let row = vec![
            Cell::text("7", CellKind::Number),
            Cell::text("ann@example.com", CellKind::Text),
            Cell::text("{\"a\":1}", CellKind::Json),
            Cell::null(),
        ];
        let json: serde_json::Value =
            serde_json::from_str(&row_json(&columns, &row, |n| n == "email")).unwrap();
        assert_eq!(json["id"], 7);
        assert_eq!(json["meta"]["a"], 1);
        assert!(json["note"].is_null());
        assert!(!json["email"].as_str().unwrap().contains("ann@"));
        assert_eq!(pretty_json("{\"a\":1}"), "{\n  \"a\": 1\n}");
        assert_eq!(pretty_json("not json"), "not json");
    }

    #[test]
    fn counts_and_masks_stay_short() {
        assert_eq!(short_count(950), "950");
        assert_eq!(short_count(8_214), "8.2k");
        assert_eq!(short_count(5_000), "5k");
        assert_eq!(short_count(52_000), "52k");
        assert_eq!(short_count(1_204_551), "1.2M");
        assert_eq!(mask("maria@fastmail.com"), "m•••@fastmail.com");
        assert_eq!(mask("tok_abc"), "••••••");
    }
}
