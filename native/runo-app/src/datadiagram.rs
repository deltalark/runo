//! The Data place's Diagram tab: the selected database's tables drawn as boxes, with a
//! line for each foreign key. Tables that others point at sit to the left of the ones
//! pointing at them, and tables with no keys either way wait in the last columns.

use super::*;
use db::Table;

/// A table box's width.
const NODE_W: f32 = 210.;
/// The box's title strip.
const HEADER: f32 = 30.;
/// One column's line.
const ROW: f32 = 20.;
/// Columns a box lists before "+N more".
const MAX_ROWS: usize = 14;
const GAP_X: f32 = 90.;
const GAP_Y: f32 = 24.;
const MARGIN: f32 = 24.;

/// One table's box.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node {
    /// Index into the connection's tables.
    pub table: usize,
    pub x: f32,
    pub y: f32,
    /// The columns listed, keys first, by index into the table's columns.
    pub shown: Vec<usize>,
    /// Columns left out.
    pub more: usize,
}

impl Node {
    fn height(&self) -> f32 {
        HEADER + ROW * (self.shown.len() + usize::from(self.more > 0)) as f32 + 6.
    }

    /// Where a line meets column `col`: its row, or the title when it isn't listed.
    fn anchor_y(&self, col: usize) -> f32 {
        match self.shown.iter().position(|&c| c == col) {
            Some(i) => self.y + HEADER + ROW * i as f32 + ROW / 2.,
            None => self.y + HEADER / 2.,
        }
    }
}

/// A foreign key: table `from`'s column `from_col` points at `to`'s `to_col` (`None`
/// when the column isn't found).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Edge {
    pub from: usize,
    pub from_col: usize,
    pub to: usize,
    pub to_col: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layout {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub width: f32,
    pub height: f32,
}

/// The columns a box lists: the primary key and foreign keys first, then the rest in
/// order, cut at `MAX_ROWS`.
fn shown_columns(t: &Table) -> (Vec<usize>, usize) {
    let key = |i: &usize| t.columns[*i].primary || t.columns[*i].refers.is_some();
    let mut order: Vec<usize> = (0..t.columns.len()).filter(key).collect();
    order.extend((0..t.columns.len()).filter(|i| !key(i)));
    let more = order.len().saturating_sub(MAX_ROWS);
    order.truncate(MAX_ROWS);
    (order, more)
}

/// Lay the tables out: a column per depth of foreign keys, each ordered to keep lines
/// short, then the tables with no keys.
pub(crate) fn layout(tables: &[Table]) -> Layout {
    let index: HashMap<String, usize> = tables
        .iter()
        .enumerate()
        .map(|(i, t)| (t.label(), i))
        .collect();
    let mut edges = Vec::new();
    for (i, t) in tables.iter().enumerate() {
        for (c, col) in t.columns.iter().enumerate() {
            let Some((to, to_col)) = &col.refers else {
                continue;
            };
            let Some(&j) = index.get(to) else {
                continue;
            };
            edges.push(Edge {
                from: i,
                from_col: c,
                to: j,
                to_col: tables[j].columns.iter().position(|c| &c.name == to_col),
            });
        }
    }

    // A table's depth: one more than the deepest table it points at. Cycles stop where
    // they come back round.
    let mut points_at: Vec<Vec<usize>> = vec![Vec::new(); tables.len()];
    let mut linked = vec![false; tables.len()];
    for e in &edges {
        if e.from != e.to {
            points_at[e.from].push(e.to);
            linked[e.from] = true;
            linked[e.to] = true;
        }
    }
    let mut depth: Vec<Option<usize>> = vec![None; tables.len()];
    fn visit(
        i: usize,
        points_at: &[Vec<usize>],
        depth: &mut [Option<usize>],
        on_path: &mut Vec<bool>,
    ) -> usize {
        if let Some(d) = depth[i] {
            return d;
        }
        on_path[i] = true;
        let mut d = 0;
        for &j in &points_at[i] {
            if !on_path[j] {
                d = d.max(visit(j, points_at, depth, on_path) + 1);
            }
        }
        on_path[i] = false;
        depth[i] = Some(d);
        d
    }
    let mut on_path = vec![false; tables.len()];
    for i in 0..tables.len() {
        visit(i, &points_at, &mut depth, &mut on_path);
    }

    let ranks = (0..tables.len())
        .filter(|&i| linked[i])
        .filter_map(|i| depth[i])
        .max()
        .map_or(0, |d| d + 1);
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); ranks];
    for i in 0..tables.len() {
        if linked[i] {
            columns[depth[i].unwrap_or(0)].push(i);
        }
    }
    for col in &mut columns {
        col.sort_by_key(|&i| tables[i].label());
    }

    // Order each column by where its neighbours sit, a few times over, so lines cross
    // less.
    let neighbours: Vec<Vec<usize>> = (0..tables.len())
        .map(|i| {
            edges
                .iter()
                .filter_map(|e| match (e.from == i, e.to == i) {
                    (true, false) => Some(e.to),
                    (false, true) => Some(e.from),
                    _ => None,
                })
                .collect()
        })
        .collect();
    for _ in 0..4 {
        let mut place = vec![0f32; tables.len()];
        for col in &columns {
            for (p, &i) in col.iter().enumerate() {
                place[i] = p as f32;
            }
        }
        for col in &mut columns {
            let centre = |i: usize| {
                let n = &neighbours[i];
                if n.is_empty() {
                    place[i]
                } else {
                    n.iter().map(|&j| place[j]).sum::<f32>() / n.len() as f32
                }
            };
            let mut keyed: Vec<(f32, usize)> = col.iter().map(|&i| (centre(i), i)).collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            *col = keyed.into_iter().map(|(_, i)| i).collect();
        }
    }

    let mut nodes = Vec::with_capacity(tables.len());
    let mut height = 0f32;
    let mut x = MARGIN;
    for col in &columns {
        let mut y = MARGIN;
        for &i in col {
            let (shown, more) = shown_columns(&tables[i]);
            let node = Node {
                table: i,
                x,
                y,
                shown,
                more,
            };
            y += node.height() + GAP_Y;
            nodes.push(node);
        }
        height = height.max(y);
        x += NODE_W + GAP_X;
    }

    // The tables with no keys, in columns about as tall as the graph.
    let tall = height.max(720.);
    let mut alone: Vec<usize> = (0..tables.len()).filter(|&i| !linked[i]).collect();
    alone.sort_by_key(|&i| tables[i].label());
    let mut y = MARGIN;
    let mut used = false;
    for i in alone {
        let (shown, more) = shown_columns(&tables[i]);
        let node = Node {
            table: i,
            x,
            y,
            shown,
            more,
        };
        if used && y + node.height() > tall {
            x += NODE_W + GAP_Y;
            y = MARGIN;
        }
        let node = Node { x, y, ..node };
        y += node.height() + GAP_Y;
        height = height.max(y);
        nodes.push(node);
        used = true;
    }
    let right = if used { x + NODE_W } else { x - GAP_X };
    nodes.sort_by_key(|n| n.table);
    Layout {
        nodes,
        edges,
        width: right.max(MARGIN) + MARGIN,
        height: height - GAP_Y + MARGIN,
    }
}

impl RunoApp {
    pub(crate) fn data_diagram(
        &self,
        conn: &crate::dataplace::Conn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if conn.tables.is_empty() {
            return crate::dataplace::centered(
                "No tables yet. Once the database has some, they're drawn here with their foreign keys.",
            );
        }
        let layout = layout(&conn.tables);
        let edges_layout = layout.clone();
        let line = rgba((c_muted() << 8) | 0x88);
        let lines = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let at = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));
                for e in &edges_layout.edges {
                    let (Some(a), Some(b)) =
                        (edges_layout.nodes.get(e.from), edges_layout.nodes.get(e.to))
                    else {
                        continue;
                    };
                    let y0 = a.anchor_y(e.from_col);
                    let y1 = e.to_col.map_or(b.y + HEADER / 2., |c| b.anchor_y(c));
                    let mut p = PathBuilder::stroke(px(1.4));
                    if e.from == e.to {
                        // A key on its own table: a loop out of the right side.
                        let x = a.x + NODE_W;
                        p.move_to(at(x, y0));
                        p.cubic_bezier_to(at(x, y1), at(x + 40., y0), at(x + 40., y1));
                    } else if (a.x - b.x).abs() < 1. {
                        // The same column: out of the left side and back.
                        p.move_to(at(a.x, y0));
                        p.cubic_bezier_to(at(b.x, y1), at(a.x - 50., y0), at(b.x - 50., y1));
                    } else {
                        let (x0, x1) = if a.x > b.x {
                            (a.x, b.x + NODE_W)
                        } else {
                            (a.x + NODE_W, b.x)
                        };
                        let bend = (x1 - x0) / 2.;
                        p.move_to(at(x0, y0));
                        p.cubic_bezier_to(at(x1, y1), at(x0 + bend, y0), at(x1 - bend, y1));
                    }
                    if let Ok(path) = p.build() {
                        window.paint_path(path, line);
                    }
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .w(px(layout.width))
        .h(px(layout.height));

        let mut plane = div()
            .relative()
            .w(px(layout.width))
            .h(px(layout.height))
            .child(lines);
        for node in &layout.nodes {
            plane = plane.child(self.diagram_node(&conn.tables[node.table], node, cx));
        }
        div()
            .id("data-diagram")
            .size_full()
            .overflow_scroll()
            .child(plane)
            .into_any_element()
    }

    fn diagram_node(&self, table: &Table, node: &Node, cx: &mut Context<Self>) -> impl IntoElement {
        let label = table.label();
        let pick = label.clone();
        let mut body = div().v_flex().py(px(3.));
        for &c in &node.shown {
            let col = &table.columns[c];
            let (mark, color) = if col.primary {
                ("PK", c_amber())
            } else if col.refers.is_some() {
                ("FK", c_blue())
            } else {
                ("", c_muted())
            };
            body = body.child(
                div()
                    .h(px(ROW))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.5))
                    .child(
                        div()
                            .w(px(16.))
                            .text_size(px(9.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(color))
                            .child(mark),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(col.name.clone()))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(80.))
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(rgb(c_muted()))
                            .font_family(self.mono_font.clone())
                            .child(db::short_type(&col.data_type)),
                    ),
            );
        }
        if node.more > 0 {
            body = body.child(
                div()
                    .h(px(ROW))
                    .px_2()
                    .pl(px(32.))
                    .flex()
                    .items_center()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child(format!("+{} more", node.more)),
            );
        }
        div()
            .id(SharedString::from(format!("data-node-{label}")))
            .absolute()
            .left(px(node.x))
            .top(px(node.y))
            .w(px(NODE_W))
            .h(px(node.height()))
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .bg(rgb(c_raised()))
            .overflow_hidden()
            .cursor_pointer()
            .hover(|el| el.border_color(rgb(c_blue())))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pick_table(pick.clone(), Vec::new(), cx);
            }))
            .child(
                div()
                    .h(px(HEADER))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(tint(c_subtle()))
                    .border_b_1()
                    .border_color(divider())
                    .child(icon(
                        if table.view {
                            IconName::Eye
                        } else {
                            IconName::Table
                        },
                        12.,
                        c_muted(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    )
                    .child(
                        div().text_size(px(10.5)).text_color(rgb(c_muted())).child(
                            table
                                .estimate
                                .map(crate::dataplace::short_count)
                                .unwrap_or_default(),
                        ),
                    ),
            )
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::Column;
    use std::prelude::v1::test;

    fn table(name: &str, cols: &[(&str, Option<&str>)]) -> Table {
        Table {
            schema: "public".into(),
            name: name.into(),
            columns: std::iter::once(Column {
                name: "id".into(),
                primary: true,
                ..Default::default()
            })
            .chain(cols.iter().map(|(c, to)| Column {
                name: (*c).into(),
                refers: to.map(|t| (t.to_string(), "id".to_string())),
                ..Default::default()
            }))
            .collect(),
            ..Default::default()
        }
    }

    fn shop() -> Vec<Table> {
        vec![
            table(
                "order_items",
                &[
                    ("order_id", Some("orders")),
                    ("product_id", Some("products")),
                ],
            ),
            table(
                "orders",
                &[("customer_id", Some("customers")), ("note", None)],
            ),
            table("customers", &[("name", None)]),
            table("products", &[("title", None)]),
            table("settings", &[("value", None)]),
        ]
    }

    #[test]
    fn tables_pointed_at_sit_left_of_the_ones_pointing() {
        let l = layout(&shop());
        let x = |i: usize| l.nodes[i].x;
        assert!(x(2) < x(1), "customers left of orders");
        assert!(x(1) < x(0), "orders left of order_items");
        assert!(x(3) < x(0), "products left of order_items");
        assert!(x(4) > x(0), "a table with no keys goes last");
        assert_eq!(l.edges.len(), 3);
        assert!(l.edges.iter().all(|e| e.to_col == Some(0)));
    }

    #[test]
    fn boxes_never_overlap_and_fit_the_plane() {
        let mut tables = shop();
        for i in 0..40 {
            tables.push(table(&format!("lonely_{i}"), &[("a", None), ("b", None)]));
        }
        let l = layout(&tables);
        for (i, a) in l.nodes.iter().enumerate() {
            assert!(a.x + NODE_W <= l.width && a.y + a.height() <= l.height + 0.5);
            for b in &l.nodes[i + 1..] {
                let apart = a.x + NODE_W <= b.x
                    || b.x + NODE_W <= a.x
                    || a.y + a.height() <= b.y
                    || b.y + b.height() <= a.y;
                assert!(apart, "{} and {} overlap", a.table, b.table);
            }
        }
    }

    #[test]
    fn cycles_and_self_keys_still_lay_out() {
        let tables = vec![
            table("a", &[("b_id", Some("b"))]),
            table("b", &[("a_id", Some("a"))]),
            table("staff", &[("manager_id", Some("staff"))]),
        ];
        let l = layout(&tables);
        assert_eq!(l.nodes.len(), 3);
        assert_eq!(l.edges.len(), 3);
    }

    #[test]
    fn a_wide_table_lists_its_keys_first_and_counts_the_rest() {
        let mut cols: Vec<(String, Option<&str>)> =
            (0..20).map(|i| (format!("c{i}"), None)).collect();
        cols.push(("owner_id".into(), Some("users")));
        let cols: Vec<(&str, Option<&str>)> = cols.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        let t = table("wide", &cols);
        let (shown, more) = shown_columns(&t);
        assert_eq!(shown.len(), MAX_ROWS);
        assert_eq!(more, 22 - MAX_ROWS);
        assert_eq!(t.columns[shown[1]].name, "owner_id");
    }
}
