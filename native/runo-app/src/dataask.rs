//! Asking an agent about a table or a whole database. The New session dialog opens with
//! the schema as context and a note on the read-only data tools, so the agent can look
//! at the rows itself; what agents ran shows on the database's overview.

use super::*;
use db::saved::Activity;
use db::Table;

/// Agent queries the overview lists.
pub(crate) const SHOWN: usize = 8;

/// What the agent is told it can do, for connection `name`.
fn tools_note(name: &str, prod: bool) -> String {
    format!(
        "The database is the connection \"{name}\" in Runo's Data place. To look at its data, use the \
         data_tables and data_query tools with connection \"{name}\": they're read-only, hidden columns \
         come back masked, and everything you run is shown to me.{}",
        if prod {
            " It's production, so filter on an indexed column or add a LIMIT; a full scan of a big table is refused."
        } else {
            ""
        }
    )
}

/// The context given with a question about `table`.
pub(crate) fn table_context(name: &str, prod: bool, table: &Table) -> String {
    format!(
        "Table {}:\n{}\n\n{}",
        table.label(),
        db::agent::describe(table),
        tools_note(name, prod)
    )
}

/// The context given with a question about the whole database: its tables, biggest first.
pub(crate) fn database_context(name: &str, prod: bool, tables: &[Table]) -> String {
    let mut sorted: Vec<&Table> = tables.iter().collect();
    sorted.sort_by_key(|t| std::cmp::Reverse(t.estimate.unwrap_or(0)));
    let mut lines: Vec<String> = sorted
        .iter()
        .take(60)
        .map(|t| {
            format!(
                "- {}{} ({} columns{})",
                t.label(),
                if t.view { " (view)" } else { "" },
                t.columns.len(),
                t.estimate
                    .map(|n| format!(", about {} rows", db::query::about(n)))
                    .unwrap_or_default()
            )
        })
        .collect();
    if sorted.len() > 60 {
        lines.push(format!("- and {} more", sorted.len() - 60));
    }
    format!(
        "Tables:\n{}\n\n{}",
        lines.join("\n"),
        tools_note(name, prod)
    )
}

impl RunoApp {
    fn data_conn_name(&self) -> Option<String> {
        let id = self.data.selected.as_ref()?;
        self.data
            .saved
            .iter()
            .find(|s| &s.id == id)
            .map(|s| s.name.clone())
    }

    /// Open the New session dialog to ask about the open table.
    pub(crate) fn ask_about_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(pid), Some(name), Some(table)) = (
            self.data.project,
            self.data_conn_name(),
            self.data.table_of().cloned(),
        ) else {
            return;
        };
        let context = shell::StartContext {
            label: format!("{} · {}", name, table.label()),
            text: table_context(&name, self.data_prod(), &table),
            images: Vec::new(),
            in_place: true,
        };
        let task = format!(
            "Explain the {} table: what it holds and how it's used.",
            table.label()
        );
        self.start_with_context(pid, &task, context, window, cx);
    }

    /// Open the New session dialog to ask about the selected database.
    pub(crate) fn ask_about_database(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(pid), Some(name), Some(id)) = (
            self.data.project,
            self.data_conn_name(),
            self.data.selected.clone(),
        ) else {
            return;
        };
        let tables = self
            .data
            .conns
            .get(&id)
            .map(|c| c.tables.clone())
            .unwrap_or_default();
        let context = shell::StartContext {
            label: name.clone(),
            text: database_context(&name, self.data_prod(), &tables),
            images: Vec::new(),
            in_place: true,
        };
        self.start_with_context(
            pid,
            "Explain this database: its main tables and how they relate.",
            context,
            window,
            cx,
        );
    }

    /// What agents ran on the selected connection, for its overview.
    pub(crate) fn agent_activity(&self) -> Option<AnyElement> {
        if self.data.activity.is_empty() {
            return None;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut list = div().v_flex().gap_1().max_w(px(560.));
        for a in &self.data.activity {
            list = list.child(activity_row(a, now, self.mono_font.clone()));
        }
        Some(
            div()
                .v_flex()
                .gap_1()
                .child(crate::dataplace::label_row("WHAT AGENTS RAN").px_0())
                .child(list)
                .into_any_element(),
        )
    }
}

fn activity_row(a: &Activity, now: u64, mono: SharedString) -> impl IntoElement {
    let (glyph, color, note) = match &a.refused {
        Some(why) => (IconName::Ban, c_amber(), why.clone()),
        None => (
            IconName::Check,
            c_muted(),
            format!(
                "{} row{} · {} ms",
                a.rows,
                if a.rows == 1 { "" } else { "s" },
                a.millis
            ),
        ),
    };
    div()
        .px_3()
        .py_1()
        .v_flex()
        .gap(px(2.))
        .rounded_md()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(icon(glyph, 12., color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .font_family(mono)
                        .child(a.sql.split_whitespace().collect::<Vec<_>>().join(" ")),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .child(crate::playbooks::ago(a.at, now)),
                ),
        )
        .child(
            div()
                .pl(px(20.))
                .text_size(px(11.))
                .text_color(rgb(c_muted()))
                .truncate()
                .child(note),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::Column;
    use std::prelude::v1::test;

    fn table() -> Table {
        Table {
            schema: "public".into(),
            name: "users".into(),
            view: false,
            estimate: Some(1200),
            columns: vec![
                Column {
                    name: "id".into(),
                    data_type: "integer".into(),
                    primary: true,
                    ..Default::default()
                },
                Column {
                    name: "email".into(),
                    data_type: "text".into(),
                    nullable: true,
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn the_context_names_the_tools_and_hides_nothing_it_shouldnt() {
        let text = table_context("shop", false, &table());
        assert!(text.contains("data_query"));
        assert!(text.contains("connection \"shop\""));
        assert!(text.contains("email text (values hidden)"));
        assert!(!text.contains("production"));
        assert!(table_context("shop", true, &table()).contains("production"));
    }

    #[test]
    fn a_big_database_is_listed_biggest_first_and_cut_short() {
        let tables: Vec<Table> = (0..70)
            .map(|i| Table {
                name: format!("t{i}"),
                estimate: Some(i),
                ..Default::default()
            })
            .collect();
        let text = database_context("shop", false, &tables);
        assert!(text.find("- t69").unwrap() < text.find("- t68").unwrap());
        assert!(text.contains("- and 10 more"));
    }
}
