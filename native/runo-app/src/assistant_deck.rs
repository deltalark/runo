//! The assistant's briefing and its list of chats. A new chat opens on the briefing: a
//! few tiles with what needs the person, what's running, what's ready to review, the day's
//! spend and the clash radar, each a click from its place. The command deck is the same
//! panel made big in the middle of the window, for a proper look at where things stand.

use super::*;
use crate::assistant_act::Action;
use crate::assistant_mem::{title, when};
use crate::shell::Bucket;

/// One tile of the briefing.
struct Tile {
    label: &'static str,
    value: String,
    detail: String,
    /// The colour of its dot, when it's worth a look.
    lit: Option<u32>,
    /// Where a click goes.
    go: Option<Action>,
}

/// Up to `n` titles, joined, with how many more there are.
fn some_titles(titles: &[&str], n: usize) -> String {
    let mut out = titles
        .iter()
        .take(n)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if titles.len() > n {
        out.push_str(&format!(" and {} more", titles.len() - n));
    }
    out
}

impl RunoApp {
    fn briefing_tiles(&self) -> Vec<Tile> {
        let live: Vec<&Session> = self.state.sessions.iter().filter(|s| !s.archived).collect();
        let in_bucket = |b: Bucket| -> Vec<&str> {
            live.iter()
                .filter(|s| self.bucket(s) == Some(b))
                .map(|s| s.title.as_str())
                .collect()
        };
        let needs = in_bucket(Bucket::NeedsYou);
        let running = in_bucket(Bucket::Running);
        let review = in_bucket(Bucket::Review);
        let open = |place: &str| {
            Some(Action::Open {
                place: place.into(),
            })
        };
        let clashes = self.git_radar.as_ref().map_or(0, |h| h.len());
        let spend = self.counts().spend_today;
        let mut tiles = vec![
            Tile {
                label: "Needs you",
                value: needs.len().to_string(),
                detail: if needs.is_empty() {
                    "Nothing is waiting on you".into()
                } else {
                    some_titles(&needs, 2)
                },
                lit: (!needs.is_empty()).then_some(c_amber()),
                go: open("inbox"),
            },
            Tile {
                label: "Running",
                value: running.len().to_string(),
                detail: if running.is_empty() {
                    "No agent is working".into()
                } else {
                    some_titles(&running, 2)
                },
                lit: (!running.is_empty()).then_some(c_green()),
                go: open("sessions"),
            },
            Tile {
                label: "Ready to review",
                value: review.len().to_string(),
                detail: if review.is_empty() {
                    "No changes waiting".into()
                } else {
                    some_titles(&review, 2)
                },
                lit: (!review.is_empty()).then_some(c_blue()),
                go: open("review"),
            },
            Tile {
                label: "Spent today",
                value: format!("${:.2}", spend / 100.0),
                detail: "Reported by your agents".into(),
                lit: None,
                go: None,
            },
            Tile {
                label: "Clash radar",
                value: clashes.to_string(),
                detail: if clashes == 0 {
                    "No two lines of work overlap".into()
                } else {
                    "Places two lines of work change the same lines".into()
                },
                lit: (clashes > 0).then_some(c_red()),
                go: None,
            },
        ];
        let notes = crate::assistant_mem::notes_for(
            &self.assistant.kept.notes,
            &self.assistant_project_name(),
        );
        tiles.push(Tile {
            label: "Remembered",
            value: notes.len().to_string(),
            detail: notes
                .first()
                .cloned()
                .unwrap_or_else(|| "Tell me what to keep with /remember".into()),
            lit: None,
            go: None,
        });
        tiles
    }

    /// The briefing a new chat opens on, in `cols` columns.
    pub(crate) fn assistant_briefing(&self, cols: u16, cx: &mut Context<Self>) -> Div {
        let mut grid = div().grid().grid_cols(cols).gap(px(8.));
        // The corner panel shows the first four; the deck has room for all.
        let shown = if cols < 3 { 4 } else { usize::MAX };
        for (i, t) in self.briefing_tiles().into_iter().take(shown).enumerate() {
            let go = t.go.clone();
            let tile = div()
                .id(("assistant-tile", i))
                .v_flex()
                .gap(px(3.))
                .p(px(10.))
                .rounded_lg()
                .border_1()
                .border_color(divider())
                .bg(rgb(c_raised()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_size(px(10.5))
                        .text_color(rgb(c_muted()))
                        .when_some(t.lit, |el, c| {
                            el.child(
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(rgb(c))
                                    .shadow(vec![crate::assistant::halo(c, 0.7, 6., 0., 0.)]),
                            )
                        })
                        .child(t.label.to_uppercase()),
                )
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t.value),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(c_secondary()))
                        .truncate()
                        .child(t.detail),
                )
                .when_some(go, |el, action| {
                    el.cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let _ = this.run_action(&action, window, cx);
                            cx.notify();
                        }))
                });
            grid = grid.child(tile);
        }
        grid
    }

    /// Earlier chats to pick up, and what the assistant remembers here.
    pub(crate) fn assistant_chats_view(&self, cx: &mut Context<Self>) -> Div {
        let heading = |text: &'static str| {
            div()
                .pt_1()
                .text_size(px(11.))
                .text_color(rgb(c_muted()))
                .child(text)
        };
        let mut list = div().v_flex().gap_2().child(heading("CHATS"));
        let chats: Vec<_> = self
            .assistant
            .kept
            .chats
            .iter()
            .filter(|c| !c.msgs.is_empty())
            .collect();
        if chats.is_empty() {
            list = list.child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child("Chats you have with me are kept here, so you can pick one up later."),
            );
        }
        for c in chats {
            let id = c.id;
            let here = id == self.assistant.chat_id;
            list =
                list.child(
                    div()
                        .id(("assistant-chat", id as usize))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(if here { rgb(c_blue()) } else { divider() })
                        .bg(rgb(c_raised()))
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .child(icon(IconName::MessageSquareText, 13., c_muted()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .v_flex()
                                .child(div().text_size(px(12.5)).truncate().child(title(&c.msgs)))
                                .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child(
                                    format!("{} · {} messages", when(c.updated), c.msgs.len()),
                                )),
                        )
                        .child(
                            self.ghost_button_sized(
                                ("assistant-chat-delete", id as usize),
                                IconName::Trash,
                                "Delete chat",
                                24.,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.assistant_delete_chat(id);
                                    cx.notify();
                                },
                            )),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.assistant_open_chat(id);
                            cx.notify();
                        })),
                );
        }
        list = list.child(heading("REMEMBERED"));
        if self.assistant.kept.notes.is_empty() {
            list = list.child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child("Nothing yet. Say \"remember …\" or type /remember, and I'll keep it in mind in every chat."),
            );
        }
        for (i, n) in self.assistant.kept.notes.iter().enumerate() {
            let text = n.text.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(divider())
                    .child(icon(IconName::Brain, 13., c_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(div().text_size(px(12.5)).child(n.text.clone()))
                            .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child(
                                if n.project.is_empty() {
                                    "Every project".to_string()
                                } else {
                                    n.project.clone()
                                },
                            )),
                    )
                    .child(
                        self.ghost_button_sized(
                            ("assistant-note-forget", i),
                            IconName::X,
                            "Forget",
                            24.,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let _ = this.assistant_forget(&text);
                            cx.notify();
                        })),
                    ),
            );
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn titles_are_cut_with_a_count() {
        assert_eq!(some_titles(&["a", "b"], 2), "a, b");
        assert_eq!(some_titles(&["a", "b", "c", "d"], 2), "a, b and 2 more");
    }
}
