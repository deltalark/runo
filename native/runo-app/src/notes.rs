//! The Project notes page: a Markdown notebook per project, with a project list, search,
//! an outline, formatting tools, a live preview, pinning and sharing with agents.

use super::*;
use gpui_kit::component::input::Position;
use workspace::unix_now;

/// How the notes are shown: the editor, the rendered Markdown, or both side by side.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotesMode {
    Write,
    Preview,
    Split,
}

/// A formatting button: wraps the selection, or starts each selected line.
#[derive(Clone, Copy)]
enum Format {
    Wrap(&'static str, &'static str, &'static str),
    Line(&'static str),
    Block(&'static str),
}

/// Sections people tend to keep, offered as one-click inserts.
/// Each ends where the person will type next.
const SECTIONS: [(&str, &str); 4] = [
    ("Commands", "## Commands\n\n- "),
    ("Decisions", "## Decisions\n\n- "),
    ("Gotchas", "## Gotchas\n\n- "),
    ("To do", "## To do\n\n- [ ] "),
];

/// Headings in Markdown text: (line index, level, title). Fenced code is skipped.
pub(crate) fn outline(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        if (1..=4).contains(&level) && t[level..].starts_with(' ') {
            out.push((i, level, t[level..].trim().to_string()));
        }
    }
    out
}

/// Open and done checklist items: (done, total).
pub(crate) fn tasks(text: &str) -> (usize, usize) {
    text.lines()
        .map(str::trim_start)
        .fold((0, 0), |(done, total), l| {
            if l.starts_with("- [x]") || l.starts_with("- [X]") {
                (done + 1, total + 1)
            } else if l.starts_with("- [ ]") {
                (done, total + 1)
            } else {
                (done, total)
            }
        })
}

/// How many times `query` appears in `text`, ignoring case.
pub(crate) fn matches(text: &str, query: &str) -> usize {
    if query.is_empty() {
        return 0;
    }
    text.to_lowercase().matches(&query.to_lowercase()).count()
}

/// The text the notes give an agent at the start of a conversation.
pub(crate) fn brief(project: &str, notes: &str) -> String {
    format!(
        "## Project notes for {project}\nThe person keeps these notes about the project. Follow them where they apply.\n\n{}\n\n## Task\n",
        notes.trim()
    )
}

impl RunoApp {
    /// Notes shared with agents for this project, ready to put ahead of a first prompt.
    pub(crate) fn shared_notes(&self, project_id: u64) -> Option<String> {
        if !self
            .state
            .notes_meta
            .get(&project_id)
            .is_some_and(|m| m.shared)
        {
            return None;
        }
        let notes = self.state.notes.get(&project_id)?;
        let name = self
            .state
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .map(|p| p.name.as_str())
            .unwrap_or("this project");
        (!notes.trim().is_empty()).then(|| brief(name, notes))
    }

    fn format_notes(&mut self, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        self.notes_input.update(cx, |input, cx| {
            let selected = input.selected_text().to_string();
            let text = input.value().to_string();
            let cursor = input.cursor().min(text.len());
            let at_line_start = cursor == 0 || text[..cursor].ends_with('\n');
            let new = match format {
                Format::Wrap(open, close, placeholder) => {
                    let inner = if selected.is_empty() {
                        placeholder
                    } else {
                        selected.as_str()
                    };
                    format!("{open}{inner}{close}")
                }
                Format::Line(prefix) => {
                    if selected.is_empty() {
                        format!("{}{prefix}", if at_line_start { "" } else { "\n" })
                    } else {
                        selected
                            .lines()
                            .map(|l| format!("{prefix}{l}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                }
                Format::Block(block) => {
                    // A new section starts after a blank line.
                    let before = &text[..cursor];
                    let lead = if before.trim().is_empty() || before.ends_with("\n\n") {
                        ""
                    } else if before.ends_with('\n') {
                        "\n"
                    } else {
                        "\n\n"
                    };
                    format!("{lead}{block}")
                }
            };
            input.replace(new, window, cx);
            input.focus(window, cx);
        });
        self.notes_changed(cx);
    }

    fn notes_meta_toggle(&mut self, pid: u64, set: fn(&mut store::NoteMeta)) {
        set(self.state.notes_meta.entry(pid).or_default());
        self.save();
    }

    fn tool_button(
        &self,
        id: (&'static str, usize),
        glyph: IconName,
        tip: &'static str,
    ) -> Stateful<Div> {
        self.ghost_button_sized(id, glyph, tip, 28.)
    }

    pub(crate) fn notes_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.notes_project();
        let query = self.notes_query.read(cx).value().trim().to_string();
        let now = unix_now();
        let meta = |pid: u64| self.state.notes_meta.get(&pid).cloned().unwrap_or_default();

        // Projects, pinned first, then the ones with notes, then the rest.
        let mut order: Vec<&Project> = self.state.projects.iter().collect();
        order.sort_by_key(|p| {
            (
                !meta(p.id).pinned,
                !self.state.notes.contains_key(&p.id),
                std::cmp::Reverse(meta(p.id).updated),
            )
        });
        let mut list = div().v_flex().gap(px(2.));
        let mut shown = 0;
        for (i, project) in order.iter().enumerate() {
            let pid = project.id;
            let text = self.state.notes.get(&pid).cloned().unwrap_or_default();
            let hits = matches(&format!("{}\n{text}", project.name), &query);
            if !query.is_empty() && hits == 0 {
                continue;
            }
            shown += 1;
            let m = meta(pid);
            let on = current == Some(pid);
            let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
            let sub = if !query.is_empty() {
                format!("{hits} match{}", if hits == 1 { "" } else { "es" })
            } else if text.trim().is_empty() {
                "No notes yet".to_string()
            } else if m.updated > 0 {
                format!("{lines} lines · {}", playbooks::ago(m.updated, now))
            } else {
                format!("{lines} lines")
            };
            list = list.child(
                div()
                    .id(("notes-project", i))
                    .px_2()
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .cursor_pointer()
                    .when(on, |el| el.bg(tint(c_active())))
                    .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.notes_project = Some(pid);
                        this.open_notes(window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(rgb(settings::mix(
                                projects::color_of(project).unwrap_or(c_secondary()),
                                c_canvas(),
                                if is_dark() { 0.2 } else { 0.1 },
                            )))
                            .child(icon(
                                projects::icon_of(project),
                                14.,
                                projects::color_of(project).unwrap_or(c_secondary()),
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(if on {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .child(project.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(rgb(if !query.is_empty() {
                                        c_blue()
                                    } else {
                                        c_muted()
                                    }))
                                    .child(sub),
                            ),
                    )
                    .when(m.shared, |el| el.child(icon(IconName::Bot, 12., c_green())))
                    .when(m.pinned, |el| el.child(icon(IconName::Pin, 12., c_amber()))),
            );
        }
        if shown == 0 {
            list = list.child(
                div()
                    .px_2()
                    .py_3()
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .child(if self.state.projects.is_empty() {
                        "Add a project to keep notes for it."
                    } else {
                        "No notes match."
                    }),
            );
        }

        let text = self.notes_input.read(cx).value().to_string();
        let heads = outline(&text);
        let mut toc = div().v_flex().gap(px(1.));
        for (j, (line, level, title)) in heads.iter().take(24).cloned().enumerate() {
            toc = toc.child(
                div()
                    .id(("notes-toc", j))
                    .h(px(24.))
                    .pl(px(8. + (level.saturating_sub(1)) as f32 * 12.))
                    .pr_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(if level <= 1 { c_text() } else { c_secondary() }))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.notes_mode == NotesMode::Preview {
                            this.notes_mode = NotesMode::Split;
                        }
                        this.notes_input.update(cx, |input, cx| {
                            input.set_cursor_position(
                                Position {
                                    line: line as u32,
                                    character: 0,
                                },
                                window,
                                cx,
                            );
                        });
                        cx.notify();
                    }))
                    .child(icon(IconName::Hash, 10., c_muted()))
                    .child(div().truncate().child(title)),
            );
        }

        let sidebar = div()
            .w(px(250.))
            .flex_none()
            .v_flex()
            .gap_4()
            .child(
                Input::new(&self.notes_query)
                    .bordered(true)
                    .prefix(icon(IconName::Search, 13., c_muted()))
                    .h(px(32.)),
            )
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(side_label("Projects"))
                    .child(list),
            )
            .when(!heads.is_empty(), |el| {
                el.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(side_label("Outline"))
                        .child(toc),
                )
            });

        let Some(pid) = current else {
            return self.notes_frame(
                sidebar,
                empty_hint(
                    "notes-empty",
                    IconName::NotebookPen,
                    "No project yet",
                    "Add a project to keep notes for it.",
                ),
            );
        };
        let project = self.state.projects.iter().find(|p| p.id == pid).cloned();
        let m = meta(pid);
        let words = text.split_whitespace().count();
        let (done, total) = tasks(&text);
        let mode = self.notes_mode;

        let mut modes = div()
            .flex()
            .items_center()
            .p(px(3.))
            .gap(px(2.))
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_border()))
            .bg(rgb(c_canvas()));
        for (j, (value, glyph, label)) in [
            (NotesMode::Write, IconName::Pencil, "Write"),
            (NotesMode::Split, IconName::Columns2, "Split"),
            (NotesMode::Preview, IconName::Eye, "Preview"),
        ]
        .into_iter()
        .enumerate()
        {
            let on = mode == value;
            modes = modes.child(
                div()
                    .id(("notes-mode", j))
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |el| {
                        el.bg(tint(c_selected()))
                            .text_color(rgb(c_blue()))
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!on, |el| {
                        el.text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.notes_mode = value;
                        cx.notify();
                    }))
                    .child(icon(glyph, 12., if on { c_blue() } else { c_muted() }))
                    .child(label),
            );
        }
        let toggle =
            |id: &'static str, on: bool, glyph: IconName, label: &'static str, color: u32| {
                div()
                    .id(id)
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_lg()
                    .border_1()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |el| {
                        el.border_color(rgb(settings::mix(color, c_border(), 0.5)))
                            .bg(rgb(settings::mix(
                                color,
                                c_canvas(),
                                if is_dark() { 0.16 } else { 0.08 },
                            )))
                            .text_color(rgb(color))
                    })
                    .when(!on, |el| {
                        el.border_color(rgb(c_border()))
                            .text_color(rgb(c_secondary()))
                            .hover(|el| el.bg(tint(c_subtle())))
                    })
                    .child(icon(glyph, 13., if on { color } else { c_muted() }))
                    .child(label)
            };

        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(project.as_ref().map(|p| p.name.clone()).unwrap_or_default()),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .truncate()
                            .font_family(self.mono_font.clone())
                            .text_color(rgb(c_muted()))
                            .child(project.as_ref().map(|p| p.path.clone()).unwrap_or_default()),
                    ),
            )
            .child(
                toggle("notes-pin", m.pinned, IconName::Pin, if m.pinned { "Pinned" } else { "Pin" }, c_amber())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.notes_meta_toggle(pid, |m| m.pinned = !m.pinned);
                        cx.notify();
                    })),
            )
            .child(
                toggle(
                    "notes-share",
                    m.shared,
                    IconName::Bot,
                    if m.shared { "Shared with agents" } else { "Share with agents" },
                    c_green(),
                )
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Give these notes to the agent at the start of each new session in this project.",
                    )
                    .build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.notes_meta_toggle(pid, |m| m.shared = !m.shared);
                    let on = this.state.notes_meta.get(&pid).is_some_and(|m| m.shared);
                    this.info = Some((
                        if on {
                            "New sessions in this project start with these notes."
                        } else {
                            "Agents no longer get these notes."
                        }
                        .into(),
                        std::time::Instant::now(),
                    ));
                    cx.notify();
                })),
            )
            .child(modes);

        let tools: [(IconName, &'static str, Format); 10] = [
            (IconName::Heading1, "Heading", Format::Line("# ")),
            (IconName::Heading2, "Subheading", Format::Line("## ")),
            (IconName::Bold, "Bold", Format::Wrap("**", "**", "bold")),
            (IconName::Italic, "Italic", Format::Wrap("_", "_", "italic")),
            (
                IconName::Code,
                "Inline code",
                Format::Wrap("`", "`", "code"),
            ),
            (IconName::List, "Bulleted list", Format::Line("- ")),
            (IconName::ListChecks, "Checklist", Format::Line("- [ ] ")),
            (IconName::Quote, "Quote", Format::Line("> ")),
            (
                IconName::CodeXml,
                "Code block",
                Format::Wrap("```\n", "\n```", "code"),
            ),
            (
                IconName::Link,
                "Link",
                Format::Wrap("[", "](https://)", "link"),
            ),
        ];
        let mut toolbar = div().flex().items_center().gap(px(2.));
        for (j, (glyph, tip, format)) in tools.into_iter().enumerate() {
            if j == 2 || j == 5 || j == 8 {
                toolbar = toolbar.child(div().w(px(1.)).h(px(16.)).mx_1().bg(rgb(c_border())));
            }
            toolbar = toolbar.child(self.tool_button(("notes-tool", j), glyph, tip).on_click(
                cx.listener(move |this, _, window, cx| {
                    this.format_notes(format, window, cx);
                    cx.notify();
                }),
            ));
        }
        let mut inserts = div().flex().items_center().gap_1();
        for (j, (label, block)) in SECTIONS.iter().enumerate() {
            inserts = inserts.child(
                div()
                    .id(("notes-insert", j))
                    .h(px(24.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())).text_color(rgb(c_text())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.format_notes(Format::Block(block), window, cx);
                        cx.notify();
                    }))
                    .child(icon(IconName::Plus, 10., c_muted()))
                    .child(*label),
            );
        }

        let editor = div().flex_1().min_w_0().text_size(px(13.5)).child(
            Textarea::new(&self.notes_input)
                .appearance(false)
                .bordered(false)
                .h(px(520.)),
        );
        let preview = div()
            .id("notes-preview")
            .flex_1()
            .min_w_0()
            .h(px(520.))
            .overflow_y_scroll()
            .text_size(px(13.5))
            .child(if text.trim().is_empty() {
                div()
                    .pt_2()
                    .text_color(rgb(c_muted()))
                    .child("Nothing to preview yet.")
                    .into_any_element()
            } else {
                TextView::markdown(SharedString::from(format!("notes-md-{pid}")), text.clone())
                    .selectable(true)
                    .into_any_element()
            });
        let surface = match mode {
            NotesMode::Write => div().flex().child(editor),
            NotesMode::Preview => div().flex().child(preview),
            NotesMode::Split => div()
                .flex()
                .gap_4()
                .child(editor)
                .child(div().w(px(1.)).bg(rgb(c_border())))
                .child(preview),
        };

        let stat = |glyph: IconName, text: String| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(icon(glyph, 11., c_muted()))
                .child(text)
        };
        let footer = div()
            .flex()
            .items_center()
            .gap_4()
            .text_size(px(11.5))
            .text_color(rgb(c_muted()))
            .child(stat(IconName::Type, format!("{words} words")))
            .child(stat(IconName::Hash, format!("{} sections", heads.len())))
            .when(total > 0, |el| {
                el.child(stat(IconName::ListChecks, format!("{done}/{total} done")))
            })
            .child(div().flex_1())
            .child(stat(
                IconName::Check,
                if m.updated > 0 {
                    format!("Saved {}", playbooks::ago(m.updated, now))
                } else {
                    "Saved as you type".into()
                },
            ));

        let card = div()
            .flex_1()
            .min_w_0()
            .v_flex()
            .gap_3()
            .child(head)
            .when(m.shared, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_lg()
                        .bg(rgb(settings::mix(c_green(), c_canvas(), if is_dark() { 0.12 } else { 0.06 })))
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .child(icon(IconName::Bot, 13., c_green()))
                        .child("New sessions in this project start with these notes, so agents follow your commands and decisions."),
                )
            })
            .child(
                div()
                    .v_flex()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .bg(tint(c_raised()))
                    .overflow_hidden()
                    .when(mode != NotesMode::Preview, |el| {
                        el.child(
                            div()
                                .px_2()
                                .py_1p5()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .border_b_1()
                                .border_color(rgb(c_border()))
                                .child(toolbar)
                                .child(inserts),
                        )
                    })
                    .child(div().px_4().py_3().child(surface))
                    .child(
                        div()
                            .px_4()
                            .py_2()
                            .border_t_1()
                            .border_color(rgb(c_border()))
                            .child(footer),
                    ),
            );
        self.notes_frame(sidebar, card.into_any_element())
    }

    /// The page around the notes: title, then the project list beside the notebook.
    fn notes_frame(&self, sidebar: Div, main: AnyElement) -> AnyElement {
        div()
            .flex_1()
            .min_w_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .max_w(px(1180.))
                    .mx_auto()
                    .p_8()
                    .v_flex()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .size(px(44.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(12.))
                                    .border_1()
                                    .border_color(rgb(settings::mix(c_amber(), c_canvas(), 0.3)))
                                    .bg(linear_gradient(
                                        135.,
                                        linear_color_stop(
                                            rgb(settings::mix(c_amber(), c_canvas(), if is_dark() { 0.3 } else { 0.2 })),
                                            0.,
                                        ),
                                        linear_color_stop(
                                            rgb(settings::mix(c_amber(), c_canvas(), if is_dark() { 0.1 } else { 0.05 })),
                                            1.,
                                        ),
                                    ))
                                    .child(icon(IconName::NotebookPen, 21., c_amber())),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(22.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Project notes"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(rgb(c_secondary()))
                                            .child("Commands, decisions and gotchas for each project, in Markdown. Saved as you type, and shared with agents when you choose."),
                                    ),
                            ),
                    )
                    .child(div().flex().items_start().gap_6().child(sidebar).child(main)),
            )
            .into_any_element()
    }
}

fn side_label(text: &'static str) -> Div {
    div()
        .px_2()
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(c_muted()))
        .child(text.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn outline_lists_headings_outside_code() {
        let text = "# App\nintro\n## Commands\n```\n# not a heading\n```\n### Deep\n#nospace\n";
        assert_eq!(
            outline(text),
            vec![
                (0, 1, "App".to_string()),
                (2, 2, "Commands".to_string()),
                (6, 3, "Deep".to_string()),
            ]
        );
    }

    #[test]
    fn counts_tasks_and_matches() {
        assert_eq!(tasks("- [x] a\n- [ ] b\n  - [X] c\n- d"), (2, 3));
        assert_eq!(matches("Cargo test\ncargo run", "CARGO"), 2);
        assert_eq!(matches("anything", ""), 0);
    }

    #[test]
    fn brief_puts_notes_before_the_task() {
        let b = brief("runo", "  - Use cargo\n");
        assert!(b.starts_with("## Project notes for runo"));
        assert!(b.contains("- Use cargo"));
        assert!(b.ends_with("## Task\n"));
    }
}
