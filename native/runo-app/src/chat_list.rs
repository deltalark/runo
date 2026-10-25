//! The chat as a virtual list: each frame lays out and paints only the rows in view, so a
//! long conversation scrolls as smoothly as a short one. Rows are the cards at the top, each
//! message, Crew's runs of news, and the "Working" or "Worked" line at the end. Each frame compares them
//! with the last frame's and tells the list which ones were added, removed or changed, so
//! rows out of view keep the heights they were measured at.
//!
//! Parsed Markdown is kept per message (see `MarkdownCache`), so a reply scrolled back into
//! view shows at once instead of being parsed again, and a row that comes back into view
//! unchanged shows as it was instead of fading in again.

use super::*;
use gpui_kit::component::text::TextViewState;
use std::hash::{Hash, Hasher};
use std::time::Instant;

/// How far above and below the view rows are laid out ahead of being scrolled to.
const OVERDRAW: f32 = 800.;
/// A row that changed this recently still plays its arrival.
const FRESH: Duration = Duration::from_millis(1000);
/// Parsed Markdown is kept for this many chats at a time.
const KEEP_CHATS: usize = 6;

/// One row of the chat.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ChatRow {
    Race,
    Kept,
    Started,
    Strip,
    /// Crew's news rows in a row, messages `start..end`, drawn close together as one list.
    News(usize, usize),
    /// A Crew card other than news: message `i`.
    Card(usize),
    Message(usize),
    Pending,
    Working,
    /// "Worked", for a moment after a turn ends.
    Finished,
}

struct Row {
    row: ChatRow,
    /// Changes when the row's height may have changed.
    version: u64,
    changed: Instant,
}

/// The rows the chat list showed last frame.
#[derive(Default)]
pub(crate) struct ChatRows {
    session: Option<u64>,
    compact: bool,
    working: bool,
    /// Settings that change every row's height, such as the font.
    epoch: u64,
    rows: Vec<Row>,
    /// Crew cards left out of the chat, by message.
    hidden: Vec<bool>,
}

/// Parsed Markdown of the chats shown recently, by session, message and part.
#[derive(Default)]
pub(crate) struct MarkdownCache {
    docs: HashMap<(u64, usize, u8), Entity<TextViewState>>,
    /// The chats with docs here, the most recently shown last.
    recent: Vec<u64>,
}

/// A new list for a chat: rows are measured as they come into view.
pub(crate) fn new_list_state() -> ListState {
    ListState::new(0, ListAlignment::Top, px(OVERDRAW))
}

pub(crate) fn hash(value: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// How many chat rows were drawn since the last call, for `bench`.
pub(crate) fn take_rows_drawn() -> u32 {
    ROWS_DRAWN.replace(0)
}

thread_local! {
    static ROWS_DRAWN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Set while a row that was shown before is drawn again, so its one-shot animations
    /// show their end instead of playing again (see `page_settling`).
    pub(crate) static ROW_SETTLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tell `list` which rows changed between `old` and `new`, keeping `old` up to date.
/// Rows with the same identity and version keep their measured heights.
fn sync_rows(list: &ListState, old: &mut Vec<Row>, new: Vec<(ChatRow, u64)>, now: Instant) {
    let common = old
        .iter()
        .zip(&new)
        .take_while(|(a, b)| a.row == b.0)
        .count();
    let mut i = 0;
    while i < common {
        if old[i].version == new[i].1 {
            i += 1;
            continue;
        }
        let start = i;
        while i < common && old[i].version != new[i].1 {
            old[i].version = new[i].1;
            old[i].changed = now;
            i += 1;
        }
        list.remeasure_items(start..i);
    }
    if common < old.len() || common < new.len() {
        list.splice(common..old.len(), new.len() - common);
        old.truncate(common);
        old.extend(new.into_iter().skip(common).map(|(row, version)| Row {
            row,
            version,
            changed: now,
        }));
    }
}

impl ChatRows {
    /// Bring `list` up to date with `rows`, the rows of chat `session` this frame.
    pub(crate) fn update(
        &mut self,
        list: &ListState,
        session: u64,
        compact: bool,
        rows: Vec<(ChatRow, u64)>,
        epoch: u64,
    ) {
        let now = Instant::now();
        if self.session != Some(session) || self.compact != compact {
            // Rows already there when the chat opens show as they are.
            let then = now.checked_sub(FRESH * 2).unwrap_or(now);
            list.reset(rows.len());
            self.rows = rows
                .into_iter()
                .map(|(row, version)| Row {
                    row,
                    version,
                    changed: then,
                })
                .collect();
            self.session = Some(session);
            self.compact = compact;
        } else {
            sync_rows(list, &mut self.rows, rows, now);
            if self.epoch != epoch {
                list.remeasure();
            }
        }
        self.epoch = epoch;
    }

    /// Row `ix`, and whether it changed recently enough to still play its arrival.
    pub(crate) fn row(&self, ix: usize) -> Option<(ChatRow, bool)> {
        self.rows
            .get(ix)
            .map(|r| (r.row, r.changed.elapsed() < FRESH))
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
}

/// Settings that change the height of every row, such as the font.
pub(crate) fn layout_epoch(cx: &App) -> u64 {
    let st = cx.global::<AppSettings>();
    hash((st.content_max_width().map(f32::to_bits), &st.font))
}

impl RunoApp {
    /// Parsed Markdown for part `part` of message `index` of chat `sid`, kept between
    /// frames and brought up to date with `text`. Thread numbers link to their thread
    /// through `links`, in Crew's chat.
    pub(crate) fn markdown_view(
        &self,
        sid: u64,
        index: usize,
        part: u8,
        text: &str,
        links: Option<&crate::thread_refs::ThreadLinks>,
        cx: &mut Context<Self>,
    ) -> TextView {
        let linked;
        let text = match links {
            Some(links) => {
                linked = links.link(text);
                linked.as_str()
            }
            None => text,
        };
        let mut cache = self.md_cache.borrow_mut();
        if cache.recent.last() != Some(&sid) {
            cache.recent.retain(|s| *s != sid);
            cache.recent.push(sid);
            if cache.recent.len() > KEEP_CHATS {
                let gone = cache.recent.remove(0);
                cache.docs.retain(|k, _| k.0 != gone);
            }
        }
        let state = match cache.docs.get(&(sid, index, part)) {
            Some(state) => {
                state.update(cx, |s, cx| s.set_text(text, cx));
                state.clone()
            }
            None => {
                let state = cx.new(|cx| TextViewState::markdown(text, cx));
                cache.docs.insert((sid, index, part), state.clone());
                state
            }
        };
        TextView::new(&state)
            .selectable(true)
            .when_some(links.cloned(), |view, links| view.plugin(links))
    }

    /// The rows of `session`'s chat, each with a version that changes with its height.
    fn chat_rows(
        &self,
        session: &Session,
        working: bool,
        hidden: &[bool],
        cx: &mut Context<Self>,
    ) -> Vec<(ChatRow, u64)> {
        let id = session.id;
        let mut rows = Vec::with_capacity(session.messages.len() + 6);
        if session.race != 0 {
            rows.push((ChatRow::Race, 0));
        }
        if self.kept_card(id, cx).is_some() {
            rows.push((ChatRow::Kept, 0));
        }
        if session.messages.first().is_some_and(|m| !m.time.is_empty()) {
            rows.push((ChatRow::Started, 0));
        }
        if session.crew.is_some() {
            rows.push((
                ChatRow::Strip,
                hash(session.crew.as_ref().map(|c| &c.decision)),
            ));
        }
        let last_agent = session.messages.iter().rposition(|m| m.role == Role::Agent);
        let last_changes = last_agent
            .and_then(|i| session.messages.get(i))
            .map_or(0, |m| self.turn_changes(m).len());
        let mut news: Option<(usize, usize)> = None;
        let mut news_len = 0usize;
        let close = |news: &mut Option<(usize, usize)>,
                     news_len: &mut usize,
                     rows: &mut Vec<(ChatRow, u64)>| {
            if let Some((start, end)) = news.take() {
                rows.push((ChatRow::News(start, end), *news_len as u64));
                *news_len = 0;
            }
        };
        for (i, m) in session.messages.iter().enumerate() {
            if hidden.get(i).copied().unwrap_or(false) {
                continue;
            }
            if let Some(card) = &m.crew {
                if matches!(card, crew::Card::News { .. }) {
                    news = Some((news.map_or(i, |n| n.0), i + 1));
                    news_len += m.text.len();
                } else {
                    close(&mut news, &mut news_len, &mut rows);
                    rows.push((ChatRow::Card(i), hash((m.text.len(), card))));
                }
                continue;
            }
            close(&mut news, &mut news_len, &mut rows);
            let last = Some(i) == last_agent;
            rows.push((
                ChatRow::Message(i),
                hash((
                    m.text.len(),
                    m.steps.len(),
                    self.open_steps.contains(&(id, i)),
                    self.open_wakes.contains(&(id, i)),
                    (
                        m.plan.len(),
                        m.questions.len(),
                        m.denied.len(),
                        m.images.len(),
                    ),
                    (m.meta.len(), m.usage.is_empty(), m.checkpoint.is_empty()),
                    last.then_some((working, last_changes)),
                )),
            ));
        }
        close(&mut news, &mut news_len, &mut rows);
        if self.pending_start.contains_key(&id) {
            rows.push((ChatRow::Pending, 0));
        } else if working {
            rows.push((ChatRow::Working, 0));
        } else if self
            .finished
            .get(&id)
            .is_some_and(|(at, _)| at.elapsed() < working::FINISH)
        {
            rows.push((ChatRow::Finished, 0));
        }
        rows
    }

    /// The chat of `session` as a list that lays out only the rows in view.
    pub(crate) fn chat_list(
        &self,
        session: &Session,
        working: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hidden = self.crew_cards_hidden(session, cx);
        let rows = self.chat_rows(session, working, &hidden, cx);
        let epoch = layout_epoch(cx);
        {
            let mut st = self.chat_rows.borrow_mut();
            st.update(&self.chat_list, session.id, compact, rows, epoch);
            st.working = working;
            st.hidden = hidden;
        }
        let view = cx.entity().downgrade();
        list(self.chat_list.clone(), move |ix, _, cx| {
            view.update(cx, |this, cx| this.chat_row(ix, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full()
        .into_any_element()
    }

    /// Row `ix` of the chat drawn last by `chat_list`, padded like the column it sits in.
    fn chat_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        ROWS_DRAWN.set(ROWS_DRAWN.get() + 1);
        let (row, fresh, count, working, compact, sid) = {
            let st = self.chat_rows.borrow();
            let Some(r) = st.rows.get(ix) else {
                return div().into_any_element();
            };
            (
                r.row,
                r.changed.elapsed() < FRESH,
                st.rows.len(),
                st.working,
                st.compact,
                st.session,
            )
        };
        let Some(session) = sid.and_then(|id| self.state.session(id)) else {
            return div().into_any_element();
        };
        ROW_SETTLED.set(!fresh);
        let content = self.chat_row_content(session, row, working, cx);
        ROW_SETTLED.set(false);
        let Some(content) = content else {
            return div().into_any_element();
        };
        div()
            .w_full()
            .child(
                div()
                    .w_full()
                    .when_some(cx.global::<AppSettings>().content_max_width(), |el, w| {
                        el.max_w(px(w))
                    })
                    .mx_auto()
                    .when(compact, |el| el.px_4())
                    .when(!compact, |el| el.px_8())
                    // The column's top and bottom padding, and the gap between rows.
                    .pt(px(match (ix, compact) {
                        (0, true) => 16.,
                        _ => 24.,
                    }))
                    .when(ix + 1 == count, |el| {
                        el.pb(px(if compact { 16. } else { 32. }))
                    })
                    .child(content),
            )
            .into_any_element()
    }

    fn chat_row_content(
        &self,
        session: &Session,
        row: ChatRow,
        working: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let id = session.id;
        let provider = agent::provider(&session.provider);
        Some(match row {
            ChatRow::Race => self.race_strip(session, cx).into_any_element(),
            ChatRow::Kept => self.kept_card(id, cx)?,
            ChatRow::Started => div()
                .w_full()
                .flex()
                .justify_center()
                .text_xs()
                .text_color(rgb(c_muted()))
                .text_size(px(11.5))
                .child(format!("Started {}", session.messages.first()?.time))
                .into_any_element(),
            ChatRow::Strip => self.thread_strip(session, false, cx)?,
            ChatRow::News(start, end) => {
                let hidden = self.chat_rows.borrow().hidden.clone();
                let mut news = Vec::new();
                for i in start..end.min(session.messages.len()) {
                    if hidden.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    news.extend(self.crew_card(id, i, &session.messages[i], cx));
                }
                div().v_flex().gap(px(2.)).children(news).into_any_element()
            }
            ChatRow::Card(i) => self.crew_card(id, i, session.messages.get(i)?, cx)?,
            ChatRow::Message(i) => {
                let message = session.messages.get(i)?;
                let last_agent = session.messages.iter().rposition(|m| m.role == Role::Agent);
                let show = Some(i) == last_agent && !self.turn_changes(message).is_empty();
                div()
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_context(ctxmenu::CtxTarget::Message(i), cx),
                    )
                    .child(self.message_view(
                        i,
                        message,
                        handoff::provider_at(session, i),
                        show,
                        working && Some(i) == last_agent,
                        cx,
                    ))
                    .into_any_element()
            }
            ChatRow::Pending => {
                let task = self.pending_start.get(&id)?;
                div()
                    .p_4()
                    .v_flex()
                    .gap_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(divider())
                    .bg(tint(c_subtle()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(icon(IconName::Loader, 14., c_blue()))
                            .child("Preparing the worktree"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(c_secondary()))
                            .child(format!(
                                "The setup command is running in the Setup terminal tab, where you can answer what it asks. {} starts on “{}” when it finishes.",
                                provider.name,
                                task.lines().next().unwrap_or_default()
                            )),
                    )
                    .into_any_element()
            }
            ChatRow::Working => {
                let started = self.turns.get(&id).map(|t| t.started);
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(13.))
                    .child(working::running(working::doing(&session.messages), started))
                    .child(self.arcade_chip(cx))
                    .into_any_element()
            }
            ChatRow::Finished => div()
                .text_size(px(13.))
                .child(self.finished_row(id)?)
                .into_any_element(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    fn rows(v: &[(ChatRow, u64)]) -> Vec<Row> {
        let now = Instant::now();
        v.iter()
            .map(|&(row, version)| Row {
                row,
                version,
                changed: now,
            })
            .collect()
    }

    #[test]
    fn appending_a_message_keeps_the_rows_before_it() {
        let list = new_list_state();
        let before = [(ChatRow::Message(0), 1), (ChatRow::Working, 0)];
        list.splice(0..0, before.len());
        let mut old = rows(&before);
        let new = vec![
            (ChatRow::Message(0), 1),
            (ChatRow::Message(1), 5),
            (ChatRow::Working, 0),
        ];
        sync_rows(&list, &mut old, new, Instant::now());
        assert_eq!(list.item_count(), 3);
        assert_eq!(
            old.iter().map(|r| r.row).collect::<Vec<_>>(),
            [ChatRow::Message(0), ChatRow::Message(1), ChatRow::Working]
        );
    }

    #[test]
    fn a_changed_row_takes_its_new_version() {
        let list = new_list_state();
        let before = [(ChatRow::Message(0), 1), (ChatRow::Message(1), 2)];
        list.splice(0..0, before.len());
        let mut old = rows(&before);
        sync_rows(
            &list,
            &mut old,
            vec![(ChatRow::Message(0), 1), (ChatRow::Message(1), 3)],
            Instant::now(),
        );
        assert_eq!(list.item_count(), 2);
        assert_eq!(old[1].version, 3);
    }

    #[test]
    fn a_removed_row_shrinks_the_list() {
        let list = new_list_state();
        let before = [(ChatRow::Message(0), 1), (ChatRow::Working, 0)];
        list.splice(0..0, before.len());
        let mut old = rows(&before);
        sync_rows(
            &list,
            &mut old,
            vec![(ChatRow::Message(0), 1)],
            Instant::now(),
        );
        assert_eq!(list.item_count(), 1);
        assert_eq!(old.len(), 1);
    }
}
