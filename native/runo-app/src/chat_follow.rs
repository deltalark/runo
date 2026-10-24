//! The chat follows the bottom only while it is pinned there. Scrolling up unpins it, so
//! streamed output no longer moves the view and a "New messages" pill shows instead;
//! scrolling back down, sending, or opening a chat pins it again. The main chat and the
//! thread open in the Threads tab each keep their own `ChatFollow`.

use super::*;

/// Within this distance of the bottom the chat counts as at the bottom.
const NEAR: f32 = 4.;

/// Whether a chat's scroll follows the bottom, and what was in the chat when it stopped.
pub(crate) struct ChatFollow {
    pinned: bool,
    /// The person's last scroll was upward: a chat a few pixels from the bottom stays
    /// unpinned until they scroll down again, so a slow scroll up isn't pulled back.
    up: bool,
    /// The session and its contents when the chat was unpinned, to tell what is new.
    seen: Option<(u64, Seen)>,
}

impl Default for ChatFollow {
    fn default() -> Self {
        Self {
            pinned: true,
            up: false,
            seen: None,
        }
    }
}

impl ChatFollow {
    /// The person scrolled, up or down; `now` is the chat as it stands, kept when
    /// scrolling up unpins it.
    fn scrolled(&mut self, up: bool, now: Option<(u64, Seen)>) {
        self.up = up;
        if up && self.pinned {
            self.pinned = false;
            self.seen = now;
        }
    }

    /// Pin again once scrolled back down to the bottom.
    fn settle(&mut self, at_bottom: bool) {
        if !self.pinned && !self.up && at_bottom {
            self.pinned = true;
            self.seen = None;
        }
    }
}

/// How far a session's chat has got: its messages, and how much of the last one.
#[derive(Clone, Copy, PartialEq)]
struct Seen {
    messages: usize,
    last: usize,
}

fn seen(session: &Session) -> Seen {
    Seen {
        messages: session.messages.len(),
        last: session
            .messages
            .last()
            .map_or(0, |m| m.text.len() + m.steps.len()),
    }
}

/// `scroll` was at, or within a few pixels of, its bottom when last laid out.
pub(crate) fn at_bottom(scroll: &ScrollHandle) -> bool {
    -scroll.offset().y >= scroll.max_offset().y - px(NEAR)
}

impl ChatFollow {
    /// Scroll to the bottom only when pinned there.
    fn follow(&self, scroll: &ScrollHandle) {
        if self.pinned && autoscroll() {
            scroll.scroll_to_bottom();
        }
    }

    /// Jump to the bottom and pin there.
    fn jump(&mut self, scroll: &ScrollHandle) {
        scroll.scroll_to_bottom();
        *self = Self::default();
    }

    /// The person scrolled over `session`'s chat: scrolling up unpins it where it is.
    fn wheel(&mut self, event: &ScrollWheelEvent, window: &Window, session: Option<&Session>) {
        let dy = event.delta.pixel_delta(window.line_height()).y;
        if dy == px(0.) {
            return;
        }
        self.scrolled(dy > px(0.), session.map(|s| (s.id, seen(s))));
    }

    /// Before each frame: pin again once scrolled back down to the bottom, and keep a
    /// pinned chat at the bottom of whatever it now shows.
    fn update(&mut self, scroll: &ScrollHandle) {
        self.settle(at_bottom(scroll));
        self.follow(scroll);
    }

    /// The pill floating over the bottom of an unpinned chat once more has arrived:
    /// clicking it runs `jump`, which goes to the newest and pins the chat again.
    fn pill(
        &self,
        session: &Session,
        id: &'static str,
        jump: fn(&mut RunoApp),
        cx: &mut Context<RunoApp>,
    ) -> Option<Div> {
        let (was, then) = self.seen.filter(|_| !self.pinned)?;
        let now = seen(session);
        if was != session.id || now == then {
            return None;
        }
        let more = now.messages.saturating_sub(then.messages);
        let label = match more {
            0 => "New messages".to_string(),
            1 => "1 new message".to_string(),
            n => format!("{n} new messages"),
        };
        Some(
            div()
                .absolute()
                .bottom(px(12.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id(id)
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .rounded_full()
                        .cursor_pointer()
                        .border_1()
                        .border_color(rgb(settings::mix(c_blue(), c_canvas(), 0.28)))
                        .bg(rgb(settings::mix(
                            c_blue(),
                            c_canvas(),
                            if is_dark() { 0.18 } else { 0.1 },
                        )))
                        .when(!glass_on(), |el| el.shadow_sm())
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_blue()))
                        .hover(|el| {
                            el.bg(rgb(settings::mix(
                                c_blue(),
                                c_canvas(),
                                if is_dark() { 0.26 } else { 0.16 },
                            )))
                        })
                        .child(icon(IconName::ArrowDown, 11., c_blue()))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            jump(this);
                            cx.notify();
                        })),
                ),
        )
    }
}

impl RunoApp {
    /// Scroll the chat to the bottom only when it is pinned there.
    pub(crate) fn follow_bottom(&self) {
        self.chat_follow.follow(&self.chat_scroll);
    }

    /// Jump the chat to the bottom and pin it there: after sending, or opening a chat.
    pub(crate) fn jump_to_bottom(&mut self) {
        self.chat_follow.jump(&self.chat_scroll);
    }

    /// The person scrolled the chat: scrolling up unpins it where it is.
    pub(crate) fn chat_wheel(&mut self, event: &ScrollWheelEvent, window: &Window) {
        let session = self.selected.and_then(|id| self.state.session(id));
        self.chat_follow.wheel(event, window, session);
    }

    /// Before each frame: the chat, and the thread open in the Threads tab, follow the
    /// bottom while pinned there.
    pub(crate) fn update_chat_follow(&mut self) {
        self.chat_follow.update(&self.chat_scroll);
        if self.peek.is_some() {
            self.peek_follow.update(&self.peek_scroll);
        }
    }

    /// The chat's "New messages" pill, while unpinned and more has arrived.
    pub(crate) fn new_messages_pill(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        self.chat_follow
            .pill(session, "new-messages", Self::jump_to_bottom, cx)
    }

    /// Scroll the thread open in the Threads tab to the bottom only when pinned there.
    pub(crate) fn peek_follow_bottom(&self) {
        self.peek_follow.follow(&self.peek_scroll);
    }

    /// Jump the thread open in the Threads tab to the bottom and pin it there: on opening
    /// it, switching threads, or sending to it.
    pub(crate) fn peek_jump_to_bottom(&mut self) {
        self.peek_follow.jump(&self.peek_scroll);
    }

    /// The person scrolled the thread open in the Threads tab.
    pub(crate) fn peek_wheel(&mut self, event: &ScrollWheelEvent, window: &Window) {
        let session = self.peek.and_then(|id| self.state.session(id));
        self.peek_follow.wheel(event, window, session);
    }

    /// The Threads tab's "New messages" pill over the open thread.
    pub(crate) fn peek_messages_pill(
        &self,
        session: &Session,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        self.peek_follow
            .pill(session, "peek-new-messages", Self::peek_jump_to_bottom, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    const NOW: Option<(u64, Seen)> = Some((
        1,
        Seen {
            messages: 3,
            last: 10,
        },
    ));

    #[test]
    fn scrolling_up_unpins_and_keeps_what_was_seen() {
        let mut f = ChatFollow::default();
        f.scrolled(true, NOW);
        assert!(!f.pinned);
        assert!(f.seen == NOW);
    }

    #[test]
    fn a_slow_scroll_up_near_the_bottom_stays_unpinned() {
        let mut f = ChatFollow::default();
        f.scrolled(true, NOW);
        f.settle(true);
        assert!(!f.pinned);
    }

    #[test]
    fn scrolling_back_down_to_the_bottom_pins_again() {
        let mut f = ChatFollow::default();
        f.scrolled(true, NOW);
        f.scrolled(false, None);
        f.settle(false);
        assert!(!f.pinned);
        f.settle(true);
        assert!(f.pinned);
        assert!(f.seen.is_none());
    }

    #[test]
    fn jumping_pins_again_and_forgets_the_last_chat() {
        let mut f = ChatFollow::default();
        f.scrolled(true, NOW);
        f.jump(&ScrollHandle::new());
        assert!(f.pinned);
        assert!(!f.up);
        assert!(f.seen.is_none());
    }

    #[test]
    fn scrolling_down_while_pinned_changes_nothing() {
        let mut f = ChatFollow::default();
        f.scrolled(false, NOW);
        f.settle(false);
        assert!(f.pinned);
        assert!(f.seen.is_none());
    }
}
