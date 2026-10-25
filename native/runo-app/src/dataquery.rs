//! The Data place's Query tab: an SQL editor that completes the open database's tables
//! and columns, runs what's written read-only (see `db::query`), and keeps saved queries,
//! recent runs and CSV exports.

use super::*;
use crate::dataplace::{centered, label_row, small_button, Grid, Stop};
use db::query::{Ran, MAX_ROWS};
use db::saved::SavedQuery;
use db::{Cell, CellKind, Column, Env, Table};
use gpui_kit::component::input::{CompletionProvider, Editor, EditorState, Rope, RopeExt};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::Sizable as _;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    TextEdit,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Recent runs kept per connection.
const RECENT: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DataTab {
    #[default]
    Browse,
    Query,
    Diagram,
}

/// A statement that ran, for the Recent list.
#[derive(Clone, Debug)]
pub(crate) struct Past {
    pub sql: String,
    pub summary: String,
    pub failed: bool,
}

/// What the editor can complete: the open database's tables and their columns.
#[derive(Default)]
pub(crate) struct Words {
    /// Each table's name and its columns' names and types.
    pub tables: Vec<(String, Vec<(String, String)>)>,
}

pub(crate) struct QueryPage {
    pub editor: Entity<EditorState>,
    pub grid: Entity<TableState<Grid>>,
    pub words: Rc<RefCell<Words>>,
    /// The name a query is being saved under.
    pub name: Entity<InputState>,
    pub naming: bool,
    pub running: bool,
    pub ran: Option<Ran>,
    pub error: Option<String>,
    /// The result row shown beside the grid.
    pub row: Option<usize>,
    pub recent: HashMap<String, Vec<Past>>,
    pub saved: Vec<SavedQuery>,
    /// What the last export did.
    pub note: Option<String>,
}

impl QueryPage {
    pub(crate) fn new(
        ui_tx: Sender<ship::UiJob>,
        mono: SharedString,
        window: &mut Window,
        cx: &mut Context<RunoApp>,
    ) -> (Self, Vec<Subscription>) {
        let words: Rc<RefCell<Words>> = Rc::default();
        let provider = Rc::new(SqlCompletions(words.clone()));
        let editor = cx.new(|cx| {
            let mut e = EditorState::new(window, cx)
                .language("sql")
                .line_number(true)
                .soft_wrap(true)
                .placeholder("SELECT * FROM orders WHERE status = 'failed' LIMIT 50");
            e.lsp_mut().completion_provider = Some(provider);
            e
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name this query"));
        let grid = cx.new(|cx| {
            TableState::new(Grid::empty(ui_tx, mono), window, cx)
                .row_selectable(true)
                .col_selectable(false)
                .col_movable(false)
                .sortable(false)
        });
        let subs = vec![
            cx.subscribe_in(&name, window, |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let name = input.read(cx).value().trim().to_string();
                    this.save_query(name, cx);
                    input.update(cx, |i, cx| i.set_value("", window, cx));
                }
            }),
            cx.subscribe_in(&grid, window, |this, _, event: &TableEvent, _, cx| {
                match event {
                    TableEvent::SelectRow(i) | TableEvent::SelectCell(i, _) => {
                        this.data.query.row = Some(*i)
                    }
                    TableEvent::ClearSelection => this.data.query.row = None,
                    _ => return,
                }
                cx.notify();
            }),
        ];
        (
            QueryPage {
                editor,
                grid,
                words,
                name,
                naming: false,
                running: false,
                ran: None,
                error: None,
                row: None,
                recent: HashMap::new(),
                saved: db::saved::load_queries(),
                note: None,
            },
            subs,
        )
    }
}

const KEYWORDS: [&str; 44] = [
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "LIKE",
    "ILIKE",
    "BETWEEN",
    "JOIN",
    "LEFT JOIN",
    "INNER JOIN",
    "ON",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "AS",
    "DISTINCT",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "WITH",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "DESC",
    "ASC",
    "EXPLAIN",
    "UNION",
    "EXISTS",
    "COALESCE",
    "NOW()",
    "INTERVAL",
    "CAST",
    "TRUE",
];

/// Words that end a table reference, so they aren't read as its alias.
const NOT_ALIAS: [&str; 16] = [
    "where", "join", "left", "right", "inner", "outer", "full", "on", "group", "order", "limit",
    "having", "union", "as", "cross", "using",
];

/// What to offer at `offset` in `text`: where the word being typed starts, and the
/// candidates as (label, kind, detail). After `t.` it's that table's columns, `t` being a
/// table or an alias given to one in the text.
pub(crate) fn complete(
    words: &Words,
    text: &str,
    offset: usize,
) -> (usize, Vec<(String, CompletionItemKind, String)>) {
    let offset = (0..=offset.min(text.len()))
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    let before = &text[..offset];
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| word_char(*c))
        .last()
        .map_or(offset, |(i, _)| i);
    let prefix = before[start..].to_lowercase();
    let qualifier = before[..start].strip_suffix('.').map(|q| {
        let from = q
            .char_indices()
            .rev()
            .take_while(|(_, c)| word_char(*c))
            .last()
            .map_or(q.len(), |(i, _)| i);
        q[from..].to_lowercase()
    });
    let lower = text.to_lowercase();
    let toks: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .filter(|w| !w.is_empty())
        .collect();
    // Tables the text reads from, and the aliases they go by.
    let mut used: Vec<(String, String)> = Vec::new();
    for (i, w) in toks.iter().enumerate() {
        if *w == "from" || *w == "join" {
            if let Some(t) = toks.get(i + 1) {
                let name = t.rsplit('.').next().unwrap_or(t).to_string();
                let mut alias = toks.get(i + 2).copied().unwrap_or("");
                if alias == "as" {
                    alias = toks.get(i + 3).copied().unwrap_or("");
                }
                let alias = if alias.is_empty() || NOT_ALIAS.contains(&alias) {
                    name.clone()
                } else {
                    alias.to_string()
                };
                used.push((name, alias));
            }
        }
    }
    let table_of = |name: &str| {
        words
            .tables
            .iter()
            .find(|(t, _)| t.to_lowercase() == name || t.rsplit('.').next() == Some(name))
    };
    let mut out = Vec::new();
    let matches = |label: &str| label.to_lowercase().starts_with(&prefix);
    if let Some(q) = qualifier {
        let target = used
            .iter()
            .find(|(_, alias)| *alias == q)
            .map(|(t, _)| t.clone())
            .unwrap_or(q);
        if let Some((_, cols)) = table_of(&target) {
            for (c, ty) in cols {
                if matches(c) {
                    out.push((c.clone(), CompletionItemKind::FIELD, db::short_type(ty)));
                }
            }
        }
        return (start, out);
    }
    if prefix.is_empty() {
        return (start, out);
    }
    let mut seen = HashSet::new();
    for (t, _) in &used {
        if let Some((name, cols)) = table_of(t) {
            for (c, ty) in cols {
                if matches(c) && seen.insert(c.clone()) {
                    out.push((
                        c.clone(),
                        CompletionItemKind::FIELD,
                        format!("{name} · {}", db::short_type(ty)),
                    ));
                }
            }
        }
    }
    for (t, cols) in &words.tables {
        if matches(t) {
            out.push((
                t.clone(),
                CompletionItemKind::CLASS,
                format!("{} columns", cols.len()),
            ));
        }
    }
    let lower_case = prefix == before[start..];
    for k in KEYWORDS {
        if matches(k) {
            let k = if lower_case {
                k.to_lowercase()
            } else {
                k.to_string()
            };
            out.push((k, CompletionItemKind::KEYWORD, String::new()));
        }
    }
    out.truncate(40);
    (start, out)
}

struct SqlCompletions(Rc<RefCell<Words>>);

impl CompletionProvider for SqlCompletions {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let text = rope.to_string();
        let (start, found) = complete(&self.0.borrow(), &text, offset);
        let range = lsp_types::Range {
            start: rope.offset_to_position(start),
            end: rope.offset_to_position(offset),
        };
        let items = found
            .into_iter()
            .map(|(label, kind, detail)| CompletionItem {
                label: label.clone(),
                kind: Some(kind),
                detail: (!detail.is_empty()).then_some(detail),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: label,
                })),
                ..Default::default()
            })
            .collect();
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        !new_text.is_empty()
            && new_text
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    }
}

/// A result's columns as a table the grid can show: no keys, types as the database
/// named them, and hidden where the result's own rule says (see `hidden_columns`).
pub(crate) fn result_table(ran: &Ran) -> Table {
    Table {
        columns: ran
            .rows
            .columns
            .iter()
            .enumerate()
            .map(|(i, name)| Column {
                name: name.clone(),
                data_type: ran.types.get(i).cloned().unwrap_or_default(),
                nullable: true,
                hidden: ran.hidden.get(i).copied().unwrap_or(false),
                ..Column::default()
            })
            .collect(),
        ..Table::default()
    }
}

/// The SQL that starts a query on a table.
pub(crate) fn starter(kind: db::Kind, table: &Table) -> String {
    format!(
        "SELECT *\nFROM {}\nLIMIT 100",
        if table.schema.is_empty() || table.schema == "public" {
            if table
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                table.name.clone()
            } else {
                db::engine::ident(&table.name)
            }
        } else {
            db::engine::table_ref(kind, table)
        }
    )
}

impl RunoApp {
    /// Point the editor's completions at the open connection's tables.
    pub(crate) fn refresh_words(&mut self) {
        let tables = self
            .data
            .selected
            .as_ref()
            .and_then(|id| self.data.conns.get(id))
            .map(|c| c.tables.clone())
            .unwrap_or_default();
        self.data.query.words.borrow_mut().tables = tables
            .iter()
            .map(|t| {
                (
                    t.label(),
                    t.columns
                        .iter()
                        .map(|c| (c.name.clone(), c.data_type.clone()))
                        .collect(),
                )
            })
            .collect();
    }

    pub(crate) fn set_data_tab(
        &mut self,
        tab: DataTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.data.tab = tab;
        self.data.warning = None;
        if tab == DataTab::Query {
            self.refresh_words();
            self.data
                .query
                .editor
                .update(cx, |e, cx| e.focus(window, cx));
        }
        cx.notify();
    }

    /// Put `sql` in the editor, on the Query tab.
    pub(crate) fn edit_query(&mut self, sql: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.data.query.editor.update(cx, |e, cx| {
            e.set_value(sql.to_string(), window, cx);
            e.focus(window, cx);
        });
        self.set_data_tab(DataTab::Query, window, cx);
    }

    /// Run what's in the editor. On production a statement that would read a whole big
    /// table waits for "Run anyway" unless `confirmed`.
    pub(crate) fn run_query(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        let Some(id) = self.data.selected.clone() else {
            return;
        };
        let Some(saved) = self.data.saved.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        if self.data.query.running {
            return;
        }
        let sql = self.data.query.editor.read(cx).value().to_string();
        if let Err(e) = db::query::check(saved.kind, &sql) {
            self.data.query.error = Some(e);
            self.data.query.ran = None;
            cx.notify();
            return;
        }
        let heavy_check =
            saved.env == Env::Prod && !confirmed && !self.data.cleared.contains(sql.trim());
        let tables = self
            .data
            .conns
            .get(&id)
            .map(|c| c.tables.clone())
            .unwrap_or_default();
        self.data.query.running = true;
        self.data.query.error = None;
        self.data.query.note = None;
        self.data.warning = None;
        cx.notify();
        let session = self.data_session(&id);
        self.bg(
            {
                let sql = sql.clone();
                move || {
                    session.with(|s| {
                        if heavy_check {
                            if let Ok(Some(warning)) = s.scan_warning(&sql, &tables) {
                                return Err(Stop::Heavy(warning));
                            }
                        }
                        let mut ran = s.run(&sql, MAX_ROWS).map_err(Stop::Failed)?;
                        ran.hidden = db::query::hidden_columns(&ran, &sql, &tables);
                        Ok(ran)
                    })
                }
            },
            move |this, result, cx| {
                this.data.query.running = false;
                this.data.query.row = None;
                let recent = this.data.query.recent.entry(id).or_default();
                match result {
                    Ok(ran) => {
                        let rows = ran.rows.rows.len();
                        recent.insert(
                            0,
                            Past {
                                sql: sql.trim().to_string(),
                                summary: format!(
                                    "{rows}{} row{} · {} ms",
                                    if ran.rows.more { "+" } else { "" },
                                    if rows == 1 { "" } else { "s" },
                                    ran.rows.millis
                                ),
                                failed: false,
                            },
                        );
                        let table = result_table(&ran);
                        let revealed = this.data.revealed.clone();
                        this.data.query.grid.update(cx, |state, cx| {
                            state
                                .delegate_mut()
                                .fill(&table, &ran.rows, None, &revealed);
                            state.clear_selection(cx);
                            state.refresh(cx);
                        });
                        this.data.query.ran = Some(ran);
                    }
                    Err(Stop::Heavy(warning)) => {
                        this.data.warning = Some(dataplace::Warning {
                            text: warning,
                            sql: sql.trim().to_string(),
                            then: dataplace::Then::Query,
                        });
                    }
                    Err(Stop::Failed(e)) => {
                        recent.insert(
                            0,
                            Past {
                                sql: sql.trim().to_string(),
                                summary: e.clone(),
                                failed: true,
                            },
                        );
                        this.data.query.ran = None;
                        this.data.query.error = Some(e);
                    }
                }
                recent.dedup_by(|a, b| a.sql == b.sql && a.failed == b.failed);
                recent.truncate(RECENT);
                cx.notify();
            },
        );
    }

    fn save_query(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(conn) = self.data.selected.clone() else {
            return;
        };
        let sql = self.data.query.editor.read(cx).value().trim().to_string();
        if sql.is_empty() {
            return;
        }
        let name = if name.is_empty() {
            sql.lines()
                .next()
                .unwrap_or("Query")
                .chars()
                .take(48)
                .collect()
        } else {
            name
        };
        self.data
            .query
            .saved
            .retain(|q| !(q.conn == conn && q.name == name));
        self.data.query.saved.push(SavedQuery {
            id: db::saved::new_id(),
            conn,
            name,
            sql,
        });
        self.data.query.naming = false;
        if let Err(e) = db::saved::store_queries(&self.data.query.saved) {
            self.data.query.note = Some(format!("Couldn't save it: {e}"));
        }
        cx.notify();
    }

    fn forget_query(&mut self, id: &str, cx: &mut Context<Self>) {
        self.data.query.saved.retain(|q| q.id != id);
        let _ = db::saved::store_queries(&self.data.query.saved);
        cx.notify();
    }

    /// Ask where to save a CSV of `sql`'s rows, then write it with `hidden` masked.
    pub(crate) fn export_csv(
        &mut self,
        sql: String,
        name: String,
        hidden: HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        let dir = crate::store::home_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&format!("{name}.csv")));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let _ = this.update(cx, |this, cx| this.write_csv(sql, path, hidden, false, cx));
            }
        })
        .detach();
    }

    pub(crate) fn write_csv(
        &mut self,
        sql: String,
        path: PathBuf,
        hidden: HashSet<String>,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.data.selected.clone() else {
            return;
        };
        let prod = self
            .data
            .saved
            .iter()
            .any(|s| s.id == id && s.env == Env::Prod);
        let heavy_check = prod && !confirmed && !self.data.cleared.contains(sql.trim());
        let tables = self
            .data
            .conns
            .get(&id)
            .map(|c| c.tables.clone())
            .unwrap_or_default();
        let session = self.data_session(&id);
        self.data.query.note = Some("Exporting…".into());
        cx.notify();
        self.bg(
            {
                let (sql, path, hidden) = (sql.clone(), path.clone(), hidden.clone());
                move || {
                    session.with(|s| {
                        if heavy_check {
                            if let Ok(Some(warning)) = s.scan_warning(&sql, &tables) {
                                return Err(Stop::Heavy(warning));
                            }
                        }
                        s.export_csv(&sql, &path, &hidden).map_err(Stop::Failed)
                    })
                }
            },
            move |this, result, cx| {
                this.data.query.note = match result {
                    Ok(n) => Some(format!(
                        "Exported {n} row{} to {}{}",
                        if n == 1 { "" } else { "s" },
                        path.display(),
                        if n >= db::query::EXPORT_MAX {
                            " (the first million)"
                        } else {
                            ""
                        }
                    )),
                    Err(Stop::Heavy(text)) => {
                        this.data.warning = Some(dataplace::Warning {
                            text,
                            sql: sql.trim().to_string(),
                            then: dataplace::Then::Export(path, hidden),
                        });
                        None
                    }
                    Err(Stop::Failed(e)) => Some(format!("The export failed: {e}")),
                };
                cx.notify();
            },
        );
    }

    /// Columns of the current result whose values stay hidden.
    fn query_hidden(&self) -> HashSet<String> {
        self.data
            .query
            .ran
            .as_ref()
            .map(|ran| {
                ran.rows
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(i, c)| {
                        ran.hidden.get(*i).copied().unwrap_or(true)
                            && !self.data.revealed.contains(*c)
                    })
                    .map(|(_, c)| c.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Saved queries and recent runs for the open connection, for the side list.
    pub(crate) fn data_query_side(&self, mut list: Div, cx: &mut Context<Self>) -> Div {
        let Some(conn) = self.data.selected.clone() else {
            return list;
        };
        let saved: Vec<&SavedQuery> = self
            .data
            .query
            .saved
            .iter()
            .filter(|q| q.conn == conn)
            .collect();
        list = list.child(label_row(format!("SAVED QUERIES · {}", saved.len())));
        if saved.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .pb_1()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child("Save a query to keep it here."),
            );
        }
        for (i, q) in saved.iter().enumerate() {
            let sql = q.sql.clone();
            let id = q.id.clone();
            list = list.child(
                div()
                    .id(("data-saved-q", i))
                    .group("saved-q")
                    .mx(px(6.))
                    .px_2()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_query(&sql, window, cx);
                    }))
                    .child(icon(IconName::Save, 12., c_muted()))
                    .child(div().flex_1().min_w_0().truncate().child(q.name.clone()))
                    .child(
                        self.ghost_button_sized(("data-saved-x", i), IconName::X, "Remove", 18.)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.forget_query(&id, cx);
                            })),
                    ),
            );
        }
        let recent = self
            .data
            .query
            .recent
            .get(&conn)
            .cloned()
            .unwrap_or_default();
        if !recent.is_empty() {
            list = list.child(label_row("RECENT"));
            for (i, past) in recent.iter().take(8).enumerate() {
                let sql = past.sql.clone();
                list = list.child(
                    div()
                        .id(("data-recent", i))
                        .mx(px(6.))
                        .px_2()
                        .py(px(4.))
                        .v_flex()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_query(&sql, window, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_family(self.mono_font.clone())
                                .truncate()
                                .child(past.sql.split_whitespace().collect::<Vec<_>>().join(" ")),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .truncate()
                                .text_color(rgb(if past.failed { c_red() } else { c_muted() }))
                                .child(past.summary.clone()),
                        ),
                );
            }
        }
        list
    }

    pub(crate) fn data_query_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let q = &self.data.query;
        let toolbar =
            div()
                .h(px(40.))
                .flex_none()
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .border_t_1()
                .border_b_1()
                .border_color(divider())
                .child(if q.running {
                    small_button("data-q-cancel", "Stop", false)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_data(cx)))
                } else {
                    small_button("data-q-run", "Run", true)
                        .on_click(cx.listener(|this, _, _, cx| this.run_query(false, cx)))
                })
                .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child(
                    if cfg!(target_os = "macos") {
                        "⌘↩"
                    } else {
                        "Ctrl+Enter"
                    },
                ))
                .child(div().flex_1())
                .when(q.naming, |el| {
                    el.child(
                        div()
                            .w(px(220.))
                            .child(Input::new(&q.name).bordered(true).h(px(26.))),
                    )
                })
                .child(
                    small_button(
                        "data-q-save",
                        if q.naming { "Save" } else { "Save query" },
                        false,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.data.query.naming {
                            let name = this.data.query.name.read(cx).value().trim().to_string();
                            this.save_query(name, cx);
                            this.data
                                .query
                                .name
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        } else {
                            this.data.query.naming = true;
                            this.data.query.name.update(cx, |i, cx| i.focus(window, cx));
                            cx.notify();
                        }
                    })),
                )
                .when(q.ran.is_some(), |el| {
                    el.child(small_button("data-q-export", "Export CSV", false).on_click(
                        cx.listener(|this, _, _, cx| {
                            let Some(ran) = this.data.query.ran.as_ref() else {
                                return;
                            };
                            let sql = ran.rows.sql.clone();
                            let hidden = this.query_hidden();
                            this.export_csv(sql, "query".into(), hidden, cx);
                        }),
                    ))
                });
        let result: AnyElement = if let Some(e) = &q.error {
            div()
                .size_full()
                .p_4()
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("That didn't run"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(c_secondary()))
                        .font_family(self.mono_font.clone())
                        .child(e.clone()),
                )
                .into_any_element()
        } else if q.running {
            centered("Running read-only…")
        } else if q.ran.is_some() {
            DataTable::new(&q.grid)
                .with_size(gpui_kit::component::Size::Small)
                .stripe(false)
                .bordered(false)
                .into_any_element()
        } else {
            centered("Write a query and press Run. It runs read-only, and nothing it does is kept.")
        };
        let status = q.ran.as_ref().map(|ran| {
            let n = ran.rows.rows.len();
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
                .child(if ran.rows.more {
                    format!("The first {n} rows. Export CSV for all of them.")
                } else {
                    format!("{n} row{}", if n == 1 { "" } else { "s" })
                })
                .child(format!("{} ms", ran.rows.millis))
        });
        let note = q.note.clone().map(|n| {
            div()
                .px_3()
                .py_1()
                .flex_none()
                .text_size(px(11.5))
                .text_color(rgb(c_secondary()))
                .child(n)
        });
        let inspector = self.query_inspector(cx);
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .v_flex()
                    .child(
                        div()
                            .h(px(180.))
                            .flex_none()
                            .pt(px(6.))
                            .font_family(self.mono_font.clone())
                            .text_size(px(12.5))
                            // Cmd/Ctrl-Enter runs; caught before the editor turns it
                            // into a new line.
                            .capture_action(cx.listener(
                                |this, action: &gpui_kit::component::input::Enter, _, cx| {
                                    if action.secondary {
                                        cx.stop_propagation();
                                        this.run_query(false, cx);
                                    }
                                },
                            ))
                            .child(Editor::new(&q.editor).appearance(false).h_full()),
                    )
                    .child(toolbar)
                    .children(self.data_warning_bar(cx))
                    .children(note)
                    .child(div().flex_1().min_h_0().child(result))
                    .children(status),
            )
            .children(inspector)
            .into_any_element()
    }

    /// The selected result row, one value per line, with a copy button.
    fn query_inspector(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ran = self.data.query.ran.as_ref()?;
        let i = self.data.query.row?;
        let row = ran.rows.rows.get(i)?;
        let hidden = self.query_hidden();
        let copied = dataplace::row_json(&ran.rows.columns, row, |n| hidden.contains(n));
        let mut lines = div().v_flex();
        for (c, name) in ran.rows.columns.iter().enumerate() {
            let cell = row.get(c).cloned().unwrap_or_else(Cell::null);
            let shown = if hidden.contains(name) && cell.kind != CellKind::Null {
                db::mask(&cell.text)
            } else if cell.kind == CellKind::Json {
                dataplace::pretty_json(&cell.text)
            } else {
                cell.text.clone()
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .font_family(self.mono_font.clone())
                            .text_color(rgb(if cell.kind == CellKind::Null {
                                c_muted()
                            } else {
                                c_text()
                            }))
                            .child(shown),
                    ),
            );
        }
        Some(
            div()
                .id("data-q-inspector")
                .w(px(300.))
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
                                .child(format!("Row {}", i + 1)),
                        )
                        .child(
                            self.ghost_button_sized(
                                "data-q-copy",
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
                            self.ghost_button_sized("data-q-close", IconName::X, "Close", 22.)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.data.query.row = None;
                                    this.data
                                        .query
                                        .grid
                                        .update(cx, |s, cx| s.clear_selection(cx));
                                    cx.notify();
                                })),
                        ),
                )
                .child(lines)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn words() -> Words {
        Words {
            tables: vec![
                (
                    "orders".into(),
                    vec![
                        ("id".into(), "bigint".into()),
                        ("status".into(), "text".into()),
                        ("customer_id".into(), "bigint".into()),
                    ],
                ),
                (
                    "customers".into(),
                    vec![
                        ("id".into(), "bigint".into()),
                        ("email".into(), "text".into()),
                    ],
                ),
            ],
        }
    }

    fn labels(text: &str) -> Vec<String> {
        let (_, found) = complete(&words(), text, text.len());
        found.into_iter().map(|(l, _, _)| l).collect()
    }

    #[test]
    fn completes_tables_columns_and_keywords() {
        assert_eq!(labels("select * from ord"), vec!["orders", "order by"]);
        assert_eq!(labels("SELECT * FROM ORD"), vec!["orders", "ORDER BY"]);
        assert_eq!(labels("select * from orders where st"), vec!["status"]);
        // Lowercase typing gets lowercase keywords.
        assert!(labels("select * from orders whe").contains(&"where".to_string()));
        // After a dot: that table's columns, through an alias too.
        let text = "select c. from customers c";
        let (start, found) = complete(&words(), text, 9);
        assert_eq!(start, 9);
        let found: Vec<String> = found.into_iter().map(|(l, _, _)| l).collect();
        assert_eq!(found, vec!["id", "email"]);
        let text = "select o.cu from orders as o";
        let (start, found) = complete(&words(), text, 11);
        assert_eq!(start, 9);
        assert_eq!(found[0].0, "customer_id");
        // Nothing typed, nothing offered.
        assert!(labels("select ").is_empty());
    }

    #[test]
    fn starters_quote_only_what_needs_it() {
        let t = Table {
            schema: "public".into(),
            name: "orders".into(),
            ..Table::default()
        };
        assert_eq!(
            starter(db::Kind::Postgres, &t),
            "SELECT *\nFROM orders\nLIMIT 100"
        );
        let t = Table {
            schema: "billing".into(),
            name: "Invoices".into(),
            ..Table::default()
        };
        assert_eq!(
            starter(db::Kind::Postgres, &t),
            "SELECT *\nFROM \"billing\".\"Invoices\"\nLIMIT 100"
        );
    }
}
