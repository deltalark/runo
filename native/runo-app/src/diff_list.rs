//! The diff in the Changes panel as a virtual list: each frame lays out and paints only the
//! lines in view, so a diff of thousands of lines draws as fast as a short one. Each file's
//! diff is parsed once and kept until its text changes, and the list is told which of its
//! rows changed since the last frame (a run folded or unfolded, a note added), so the rest
//! keep the heights they were measured at.

use super::*;
use std::rc::Rc;

/// How far above and below the view lines are laid out ahead of being scrolled to.
const OVERDRAW: f32 = 600.;

/// One row of the diff list. `file` indexes the files shown, `row` that file's parsed rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DiffItem {
    /// A file's name, over its lines, when every changed file is shown.
    Header(usize),
    /// Unchanged lines `start..end` folded into one row.
    Fold {
        file: usize,
        start: usize,
        end: usize,
    },
    Line {
        file: usize,
        row: usize,
    },
    /// The session's note `note` under the line it is about.
    Note { file: usize, note: usize },
    /// The box a note is being written in, under its line.
    Writing,
}

/// A file's diff, parsed.
struct Parsed {
    /// The hash of the text it was parsed from.
    hash: u64,
    rows: Rc<Vec<git::DiffRow>>,
    /// Folding runs of unchanged lines: the end of the run starting at each row.
    runs: HashMap<usize, usize>,
}

/// What the diff list showed last frame.
#[derive(Default)]
pub(crate) struct DiffRows {
    /// Parsed diffs by path, kept while their text stays the same.
    parsed: HashMap<String, Parsed>,
    /// The files shown, in order, with their rows.
    files: Vec<(String, Rc<Vec<git::DiffRow>>)>,
    items: Vec<DiffItem>,
    /// What the list shows: the session, the file or every file, and settings that change
    /// every row's height. A new one starts the list again at its top.
    shown: Option<u64>,
    epoch: Option<u64>,
}

pub(crate) fn new_list_state() -> ListState {
    ListState::new(0, ListAlignment::Top, px(OVERDRAW))
}

/// Runs of more than 8 unchanged lines, as (first, end) row indexes, keeping 3 lines of
/// context next to each change so long unchanged stretches can be folded away.
fn collapsible_runs(rows: &[git::DiffRow]) -> Vec<(usize, usize)> {
    const KEEP: usize = 3;
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if rows[i].kind != git::LineKind::Context {
            i += 1;
            continue;
        }
        let start = i;
        while i < rows.len() && rows[i].kind == git::LineKind::Context {
            i += 1;
        }
        let lead = if start == 0 { 0 } else { KEEP };
        let tail = if i == rows.len() { 0 } else { KEEP };
        if i - start > lead + tail + 2 {
            out.push((start + lead, i - tail));
        }
    }
    out
}

/// Tell `list` how `old` became `new`: the rows they start and end with stay as they
/// were measured, and the ones between are replaced.
fn splice(list: &ListState, old: &[DiffItem], new: &[DiffItem]) {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (old_end, new_end) = (old.len() - tail, new.len() - tail);
    if head < old_end || head < new_end {
        list.splice(head..old_end, new_end - head);
    }
}

/// The marker before a line of `kind`.
fn mark(kind: git::LineKind) -> &'static str {
    match kind {
        git::LineKind::Added => "+",
        git::LineKind::Removed => "−",
        git::LineKind::Note => "",
        _ => " ",
    }
}

impl RunoApp {
    /// The diffs the panel shows: the selected file's, or every changed file's.
    fn diff_texts(&self) -> Vec<(&str, &str)> {
        if self.diff_all {
            self.all_diffs
                .iter()
                .map(|(p, d)| (p.as_str(), d.as_str()))
                .collect()
        } else {
            vec![(
                self.selected_file.as_deref().unwrap_or_default(),
                self.diff.as_str(),
            )]
        }
    }

    /// The rows of the files shown, one after another.
    fn diff_items(&self, files: &[(String, Rc<Vec<git::DiffRow>>)]) -> Vec<DiffItem> {
        let notes = self.current().map(|s| s.notes.as_slice()).unwrap_or_default();
        let mut items = Vec::new();
        let st = self.diff_rows.borrow();
        for (file, (path, rows)) in files.iter().enumerate() {
            if self.diff_all {
                items.push(DiffItem::Header(file));
            }
            let runs = st.parsed.get(path).map(|p| &p.runs);
            let mut row = 0;
            while row < rows.len() {
                if let Some(&end) = runs.and_then(|r| r.get(&row)) {
                    if !self.diff_expanded.contains(&(path.clone(), row)) {
                        items.push(DiffItem::Fold {
                            file,
                            start: row,
                            end,
                        });
                        row = end;
                        continue;
                    }
                }
                let line = &rows[row];
                items.push(DiffItem::Line { file, row });
                let can_note = line.kind != git::LineKind::Note && line.number.is_some();
                if line.kind != git::LineKind::Hunk && can_note {
                    let number = line.number.unwrap_or(0);
                    let mark = mark(line.kind);
                    for (note, n) in notes.iter().enumerate() {
                        if n.path == *path && n.line == number && n.code.starts_with(mark) {
                            items.push(DiffItem::Note { file, note });
                        }
                    }
                    if self.note_at.as_ref().is_some_and(|(p, l, code)| {
                        p == path && *l == number && *code == format!("{mark}{}", line.text)
                    }) {
                        items.push(DiffItem::Writing);
                    }
                }
                row += 1;
            }
        }
        items
    }

    /// The diff, or every changed file's diff one after another, as a list that lays out
    /// only the lines in view.
    pub(crate) fn diff_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let texts = self.diff_texts();
        let files: Vec<(String, Rc<Vec<git::DiffRow>>)> = {
            let mut st = self.diff_rows.borrow_mut();
            // Diffs no longer shown are let go.
            st.parsed
                .retain(|path, _| texts.iter().any(|(p, _)| *p == path.as_str()));
            texts
                .iter()
                .map(|&(path, diff)| {
                    let hash = chat_list::hash(diff);
                    let parsed = st.parsed.entry(path.to_string()).or_insert(Parsed {
                        hash: !hash,
                        rows: Rc::default(),
                        runs: HashMap::new(),
                    });
                    if parsed.hash != hash {
                        let rows = git::diff_rows(diff);
                        parsed.runs = collapsible_runs(&rows).into_iter().collect();
                        parsed.rows = Rc::new(rows);
                        parsed.hash = hash;
                    }
                    (path.to_string(), parsed.rows.clone())
                })
                .collect()
        };
        let items = self.diff_items(&files);
        let shown = chat_list::hash((
            self.selected,
            self.diff_all,
            files.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        ));
        let epoch = chat_list::hash((chat_list::layout_epoch(cx), self.diff_wrap));
        {
            let mut st = self.diff_rows.borrow_mut();
            if st.shown != Some(shown) {
                self.diff_list.reset(items.len());
            } else {
                splice(&self.diff_list, &st.items, &items);
                if st.epoch != Some(epoch) {
                    self.diff_list.remeasure();
                }
            }
            st.shown = Some(shown);
            st.epoch = Some(epoch);
            st.files = files;
            st.items = items;
        }
        let view = cx.entity().downgrade();
        div()
            .size_full()
            .font_family(self.mono_font.clone())
            .text_xs()
            .child(
                list(self.diff_list.clone(), move |ix, _, cx| {
                    view.update(cx, |this, cx| this.diff_item(ix, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .size_full(),
            )
            .into_any_element()
    }

    /// Row `ix` of the diff list drawn last by `diff_view`.
    fn diff_item(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        use git::LineKind;
        let (item, path, rows, last) = {
            let st = self.diff_rows.borrow();
            let Some(&item) = st.items.get(ix) else {
                return div().into_any_element();
            };
            let file = match item {
                DiffItem::Header(file)
                | DiffItem::Fold { file, .. }
                | DiffItem::Line { file, .. }
                | DiffItem::Note { file, .. } => file,
                DiffItem::Writing => 0,
            };
            let Some((path, rows)) = st.files.get(file) else {
                return div().into_any_element();
            };
            (item, path.clone(), rows.clone(), ix + 1 == st.items.len())
        };
        // The column's padding, top and bottom.
        let pad = |el: AnyElement| {
            div()
                .when(ix == 0, |el| el.pt_1())
                .when(last, |el| el.pb_1())
                .child(el)
                .into_any_element()
        };
        let el = match item {
            DiffItem::Header(_) => {
                let id = path.clone();
                div()
                    .id(("dl-file", ix))
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_y_1()
                    .border_color(divider())
                    .bg(inset(c_sidebar()))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.diff_all = false;
                        this.open_change(&id);
                        cx.notify();
                    }))
                    .child(icon(IconName::File, 12., c_muted()))
                    .child(path)
                    .into_any_element()
            }
            DiffItem::Fold { start, end, .. } => {
                let key = (path, start);
                div()
                    .id(("dl-fold", ix))
                    .h(px(22.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .bg(tint(c_subtle()))
                    .text_color(rgb(c_muted()))
                    .hover(|el| el.text_color(rgb(c_text())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.diff_expanded.insert(key.clone());
                        cx.notify();
                    }))
                    .child(icon(IconName::ChevronsUpDown, 12., c_muted()))
                    .child(format!("{} unchanged lines", end - start))
                    .into_any_element()
            }
            DiffItem::Line { row, .. } => {
                let Some(row) = rows.get(row) else {
                    return div().into_any_element();
                };
                if row.kind == LineKind::Hunk {
                    return pad(
                        div()
                            .id(("dl", ix))
                            .mt_1()
                            .px_3()
                            .py(px(3.))
                            .text_color(rgb(c_blue()))
                            .bg(tint(c_selected()))
                            .whitespace_nowrap()
                            .child(row.text.clone())
                            .into_any_element(),
                    );
                }
                let mark = mark(row.kind);
                let (bg, color) = match row.kind {
                    LineKind::Added => (Some(c_add()), c_add()),
                    LineKind::Removed => (Some(c_del()), c_del()),
                    LineKind::Note => (None, c_muted()),
                    _ => (None, c_text()),
                };
                let line = row.number.unwrap_or(0);
                let can_note = row.kind != LineKind::Note && row.number.is_some();
                let target = (path, line, format!("{mark}{}", row.text));
                div()
                    .id(("dl", ix))
                    .group("diff-line")
                    .flex()
                    .min_h(px(19.))
                    .when_some(bg, |el, bg| el.bg(diff_fill(bg)))
                    .when(can_note, |el| {
                        el.cursor_pointer()
                            .hover(|el| el.bg(tint(c_selected())))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.note_at = Some(target.clone());
                                this.note_input
                                    .update(cx, |input, cx| input.focus(window, cx));
                                cx.notify();
                            }))
                    })
                    .child(
                        div()
                            .w(px(44.))
                            .flex_none()
                            .px_2()
                            .text_right()
                            .text_color(rgb(c_muted()))
                            .child(row.number.map(|n| n.to_string()).unwrap_or_default()),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .text_color(rgb(color))
                            .child(mark),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(!self.diff_wrap, |el| el.whitespace_nowrap())
                            .when(self.diff_wrap, |el| el.whitespace_normal())
                            .text_color(rgb(color))
                            .when(row.kind == LineKind::Note, |el| el.italic())
                            .child(row.text.clone()),
                    )
                    .when(can_note, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .px_2()
                                .opacity(0.)
                                .group_hover("diff-line", |el| el.opacity(1.))
                                .child(icon(IconName::MessageSquarePlus, 12., c_blue())),
                        )
                    })
                    .into_any_element()
            }
            DiffItem::Note { note: index, .. } => {
                let Some(note) = self.current().and_then(|s| s.notes.get(index)) else {
                    return div().into_any_element();
                };
                div()
                    .mx_2()
                    .my_1()
                    .ml(px(58.))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_start()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .bg(rgb(c_canvas()))
                    .font_family(cx.theme().font_family.clone())
                    .child(icon(IconName::MessageSquareText, 13., c_blue()))
                    .child(div().flex_1().text_xs().child(note.text.clone()))
                    .child(
                        div()
                            .id(("note-x", index))
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|el| el.bg(tint(c_subtle())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_note(index);
                                cx.notify();
                            }))
                            .child(icon(IconName::X, 12., c_muted())),
                    )
                    .into_any_element()
            }
            DiffItem::Writing => div()
                .ml(px(58.))
                .mr_2()
                .my_1()
                .flex()
                .items_center()
                .gap_2()
                .font_family(cx.theme().font_family.clone())
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(&self.note_input).bordered(true).h(px(30.))),
                )
                .child(
                    div()
                        .id("note-cancel")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .cursor_pointer()
                        .text_color(rgb(c_secondary()))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.note_at = None;
                            cx.notify();
                        }))
                        .child("Cancel"),
                )
                .into_any_element(),
        };
        pad(el)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    fn row(kind: git::LineKind) -> git::DiffRow {
        git::DiffRow {
            kind,
            number: Some(1),
            text: String::new(),
        }
    }

    #[test]
    fn long_unchanged_runs_fold_keeping_context() {
        use git::LineKind::*;
        let mut rows = vec![row(Added)];
        rows.extend((0..20).map(|_| row(Context)));
        rows.push(row(Removed));
        assert_eq!(collapsible_runs(&rows), vec![(4, 18)]);
    }

    #[test]
    fn splicing_keeps_the_rows_around_a_change() {
        let list = new_list_state();
        let line = |row| DiffItem::Line { file: 0, row };
        let old = [line(0), line(1), line(2)];
        list.splice(0..0, old.len());
        let new = [line(0), line(1), DiffItem::Writing, line(2)];
        splice(&list, &old, &new);
        assert_eq!(list.item_count(), 4);
        splice(&list, &new, &old);
        assert_eq!(list.item_count(), 3);
        splice(&list, &old, &old);
        assert_eq!(list.item_count(), 3);
    }
}
