//! Counting in the Data place: a table's exact row count on demand, and each column's
//! statistics, first as the database estimates them and then, when asked, counted.

use super::*;
use crate::dataplace::{label_row, small_button, Stop, Then, Warning};
use db::query::ColumnStat;
use db::Table;

#[derive(Default)]
pub(crate) struct Stats {
    /// The columns panel is open.
    pub open: bool,
    /// The exact count, for the statement it counted.
    pub count: Option<(String, Result<i64, String>)>,
    pub counting: bool,
    /// The estimates, for the table they're about.
    pub estimated: Option<(String, Result<Vec<ColumnStat>, String>)>,
    /// Columns counted exactly, by statement and column.
    pub exact: HashMap<(String, String), Result<ColumnStat, String>>,
    pub busy: HashSet<String>,
}

/// `n` with thousands separated: 1,234,567.
pub(crate) fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// A share as a percentage that never reads 0% when something is there.
pub(crate) fn percent(share: f64) -> String {
    match share * 100. {
        0. => "0%".into(),
        p if p < 1. => "<1%".into(),
        p => format!("{:.0}%", p),
    }
}

impl RunoApp {
    /// The statement the current browse counts over: its filters, without paging.
    fn whole_sql(&self) -> Option<String> {
        let table = self.data.table_of()?;
        let kind = self
            .data
            .selected
            .as_ref()
            .and_then(|id| self.data.saved.iter().find(|s| &s.id == id))
            .map_or(db::Kind::Postgres, |s| s.kind);
        Some(db::engine::select_sql(kind, table, &self.data.browse, false).0)
    }

    /// The exact count for the current browse, when it's been counted.
    pub(crate) fn exact_count(&self) -> Option<&Result<i64, String>> {
        let whole = self.whole_sql()?;
        self.data
            .stats
            .count
            .as_ref()
            .filter(|(sql, _)| *sql == whole)
            .map(|(_, r)| r)
    }

    /// Count the rows the current filters keep. On production a count that reads the
    /// whole of a big table waits for "Run anyway" unless `confirmed`.
    pub(crate) fn count_rows(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        let (Some(id), Some(table), Some(whole)) = (
            self.data.selected.clone(),
            self.data.table_of().cloned(),
            self.whole_sql(),
        ) else {
            return;
        };
        let heavy_check = self.data_prod() && !confirmed && !self.data.cleared.contains(&whole);
        let browse = self.data.browse.clone();
        let tables = self.data_tables(&id);
        let session = self.data_session(&id);
        self.data.stats.counting = true;
        self.data.warning = None;
        cx.notify();
        self.bg(
            {
                let whole = whole.clone();
                move || {
                    session.with(|s| {
                        if heavy_check {
                            let sql = format!("SELECT count(*) FROM ({whole}) AS counted");
                            if let Ok(Some(w)) = s.scan_warning(&sql, &tables) {
                                return Err(Stop::Heavy(w));
                            }
                        }
                        s.count(&table, &browse).map_err(Stop::Failed)
                    })
                }
            },
            move |this, result, cx| {
                this.data.stats.counting = false;
                match result {
                    Ok(n) => this.data.stats.count = Some((whole, Ok(n))),
                    Err(Stop::Failed(e)) => this.data.stats.count = Some((whole, Err(e))),
                    Err(Stop::Heavy(text)) => {
                        this.data.warning = Some(Warning {
                            text,
                            sql: whole,
                            then: Then::Count,
                        })
                    }
                }
                cx.notify();
            },
        );
    }

    /// The selected connection's tables.
    pub(crate) fn data_tables(&self, id: &str) -> Vec<Table> {
        self.data
            .conns
            .get(id)
            .map(|c| c.tables.clone())
            .unwrap_or_default()
    }

    /// Open or close the columns panel, reading the database's estimates when it opens.
    pub(crate) fn toggle_stats(&mut self, cx: &mut Context<Self>) {
        self.data.stats.open = !self.data.stats.open;
        if self.data.stats.open {
            self.data.row = None;
            self.load_estimates();
        }
        cx.notify();
    }

    pub(crate) fn load_estimates(&mut self) {
        let (Some(id), Some(table)) = (self.data.selected.clone(), self.data.table_of().cloned())
        else {
            return;
        };
        let label = table.label();
        if self
            .data
            .stats
            .estimated
            .as_ref()
            .is_some_and(|(l, r)| *l == label && r.is_ok())
        {
            return;
        }
        let session = self.data_session(&id);
        self.bg(
            move || session.with(|s| s.estimated_stats(&table).map_err(Stop::Failed)),
            move |this, result, cx| {
                this.data.stats.estimated = Some((
                    label,
                    result.map_err(|e| match e {
                        Stop::Failed(e) | Stop::Heavy(e) => e,
                    }),
                ));
                cx.notify();
            },
        );
    }

    /// Count one column exactly over the rows the filters keep.
    pub(crate) fn exact_column(&mut self, name: &str, confirmed: bool, cx: &mut Context<Self>) {
        let (Some(id), Some(table), Some(whole)) = (
            self.data.selected.clone(),
            self.data.table_of().cloned(),
            self.whole_sql(),
        ) else {
            return;
        };
        let Some(column) = table.columns.iter().find(|c| c.name == name).cloned() else {
            return;
        };
        let key = format!("{whole} -- {name}");
        let heavy_check = self.data_prod() && !confirmed && !self.data.cleared.contains(&key);
        let browse = self.data.browse.clone();
        let tables = self.data_tables(&id);
        let session = self.data_session(&id);
        self.data.stats.busy.insert(name.to_string());
        self.data.warning = None;
        cx.notify();
        let name = name.to_string();
        self.bg(
            {
                let whole = whole.clone();
                move || {
                    session.with(|s| {
                        if heavy_check {
                            let sql = format!("SELECT count(*) FROM ({whole}) AS counted");
                            if let Ok(Some(w)) = s.scan_warning(&sql, &tables) {
                                return Err(Stop::Heavy(w));
                            }
                        }
                        s.exact_stats(&table, &column, &browse)
                            .map_err(Stop::Failed)
                    })
                }
            },
            move |this, result, cx| {
                this.data.stats.busy.remove(&name);
                match result {
                    Ok(stat) => {
                        this.data.stats.exact.insert((whole, name), Ok(stat));
                    }
                    Err(Stop::Failed(e)) => {
                        this.data.stats.exact.insert((whole, name), Err(e));
                    }
                    Err(Stop::Heavy(text)) => {
                        this.data.warning = Some(Warning {
                            text,
                            sql: key,
                            then: Then::Column(name),
                        })
                    }
                }
                cx.notify();
            },
        );
    }

    /// The columns panel: each column's type, keys and statistics.
    pub(crate) fn data_stats_panel(
        &self,
        table: &Table,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.data.stats.open {
            return None;
        }
        let whole = self.whole_sql().unwrap_or_default();
        let estimated = self
            .data
            .stats
            .estimated
            .as_ref()
            .filter(|(l, _)| *l == table.label())
            .map(|(_, r)| r);
        let sqlite = table.schema.is_empty();
        let mut cards = div().v_flex().gap_2();
        for (i, column) in table.columns.iter().enumerate() {
            let est = estimated
                .and_then(|r| r.as_ref().ok())
                .and_then(|all| all.iter().find(|s| s.name == column.name));
            let exact = self
                .data
                .stats
                .exact
                .get(&(whole.clone(), column.name.clone()));
            let busy = self.data.stats.busy.contains(&column.name);
            let mut lines = div().v_flex().gap(px(2.));
            if let Some(est) = est {
                let mut parts = Vec::new();
                if let Some(n) = est.nulls {
                    parts.push(format!("{} empty", percent(n)));
                }
                if let Some(d) = est.distinct.filter(|d| *d > 0.) {
                    parts.push(format!("~{} distinct", thousands(d as i64)));
                }
                if !parts.is_empty() {
                    lines = lines.child(stat_line(format!("Estimated: {}", parts.join(" · "))));
                }
                if !est.common.is_empty() {
                    lines =
                        lines.child(stat_line(format!("Most common: {}", est.common.join(", "))));
                }
            }
            match exact {
                Some(Ok(stat)) => {
                    let mut parts = vec![format!("{} rows", thousands(stat.rows.unwrap_or(0)))];
                    if let Some(n) = stat.nulls {
                        parts.push(format!("{} empty", percent(n)));
                    }
                    if let Some(d) = stat.distinct {
                        parts.push(format!("{} distinct", thousands(d as i64)));
                    }
                    lines = lines.child(stat_line(format!("Counted: {}", parts.join(" · "))));
                    if let (Some(lo), Some(hi)) = (&stat.min, &stat.max) {
                        lines = lines.child(stat_line(format!("From {lo} to {hi}")));
                    }
                }
                Some(Err(e)) => {
                    lines = lines.child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(c_red()))
                            .child(e.clone()),
                    );
                }
                None => {}
            }
            let name = column.name.clone();
            let mut badges = Vec::new();
            if column.primary {
                badges.push("key".to_string());
            }
            if let Some((t, _)) = &column.refers {
                badges.push(format!("→ {t}"));
            }
            if column.sensitive() {
                badges.push("hidden".into());
            }
            cards = cards.child(
                div()
                    .py_2()
                    .v_flex()
                    .gap_1()
                    .border_b_1()
                    .border_color(divider())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(column.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .font_family(self.mono_font.clone())
                                    .text_color(rgb(c_muted()))
                                    .child(db::short_type(&column.data_type)),
                            )
                            .children(badges.into_iter().map(|b| {
                                div().text_size(px(10.5)).text_color(rgb(c_blue())).child(b)
                            }))
                            .child(div().flex_1())
                            .child(if busy {
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(c_muted()))
                                    .child("Counting…")
                                    .into_any_element()
                            } else {
                                small_button(("data-stat-count", i), "Count", false)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.exact_column(&name, false, cx);
                                    }))
                                    .into_any_element()
                            }),
                    )
                    .child(lines),
            );
        }
        let note = match estimated {
            _ if sqlite => "SQLite keeps no statistics. Count a column to see its values.".to_string(),
            Some(Ok(all)) if all.is_empty() => {
                "Postgres has no statistics for this table yet (ANALYZE gathers them). Count a column to see its values.".into()
            }
            Some(Ok(_)) => {
                "Estimates come from Postgres's statistics, without reading the table. Count reads it, over the rows your filters keep.".into()
            }
            Some(Err(e)) => format!("The estimates didn't load: {e}"),
            None => "Loading the estimates…".into(),
        };
        Some(
            div()
                .id("data-stats")
                .w(px(340.))
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
                                .child(format!("{} · columns", table.label())),
                        )
                        .child(
                            self.ghost_button_sized("data-stats-close", IconName::X, "Close", 22.)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.data.stats.open = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_muted()))
                        .child(note),
                )
                .child(label_row(format!("COLUMNS · {}", table.columns.len())).px_0())
                .child(cards)
                .into_any_element(),
        )
    }
}

fn stat_line(text: String) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(c_secondary()))
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn numbers_read_well() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(-1234567), "-1,234,567");
        assert_eq!(percent(0.), "0%");
        assert_eq!(percent(0.004), "<1%");
        assert_eq!(percent(0.256), "26%");
    }
}
