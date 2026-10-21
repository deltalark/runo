//! Comments on the canvas: the comment tool (C) pins a note to a point on a screen, pins
//! open a thread with replies, and the right panel lists them all. Comments work in
//! Present too, pinned to the screen playing. Typing @ mentions someone; mentioning AI
//! hands the comment to AI to fix.

use super::*;
use crate::designdraw::notes::{self, Comment, Reply};
use gpui_kit::component::Sizable;

/// What the comment popover shows: a new comment being written, or a thread.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum CommentOpen {
    New { screen: String, x: f32, y: f32 },
    Thread(u32),
}

const PIN: f32 = 28.;
const POPOVER_W: f32 = 300.;

fn named(name: &str) -> IconName {
    crate::archdraw::view::icon_named(name).unwrap_or(IconName::Square)
}

pub(super) fn comment_icon() -> IconName {
    named("message-circle")
}

/// A round avatar with someone's initials.
fn avatar(name: &str, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(c_blue()))
        .text_size(px(size * 0.4))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(0xffffff))
        .child(notes::initials(name))
}

/// What someone said, with the people it mentions picked out.
fn said(text: &str, people: &[String]) -> StyledText {
    let style = HighlightStyle {
        color: Some(rgb(c_blue()).into()),
        font_weight: Some(FontWeight::SEMIBOLD),
        ..Default::default()
    };
    StyledText::new(text.to_string()).with_highlights(
        notes::mentions(text, people)
            .into_iter()
            .map(|r| (r, style)),
    )
}

/// One message of a thread: who, when, and what they said.
fn message(author: &str, at: &str, text: &str, people: &[String]) -> Div {
    div().flex().gap(px(8.)).child(avatar(author, 22.)).child(
        div()
            .flex_1()
            .min_w_0()
            .v_flex()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child(author.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(at.to_string()),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_text()))
                    .child(said(text, people)),
            ),
    )
}

impl DesignCanvas {
    pub(super) fn make_comment_field(
        window: &mut Window,
        cx: &mut Context<Self>,
        subs: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let f = cx.new(|cx| InputState::new(window, cx).placeholder("Add a comment"));
        subs.push(cx.subscribe_in(&f, window, |this, _, ev, window, cx| {
            match ev {
                InputEvent::PressEnter { .. } => this.post_comment(window, cx),
                // Mentions being typed are suggested as they go.
                InputEvent::Change => cx.notify(),
                _ => {}
            }
        }));
        f
    }

    /// Turn the comment tool on or off (C).
    pub(super) fn toggle_commenting(&mut self, cx: &mut Context<Self>) {
        self.commenting = !self.commenting;
        self.hand = false;
        self.comment_open = None;
        if self.commenting {
            self.comments = notes::load(&self.root);
            self.people = notes::people(&self.root, &self.comments);
            if self.author.is_empty() {
                self.author = notes::author(&self.root);
            }
        }
        cx.notify();
    }

    /// A click on the canvas with the comment tool: start a comment where it landed on a
    /// screen, or close the open one.
    pub(super) fn comment_down(&mut self, w: Pt, cx: &mut Context<Self>) {
        match self.screen_at(w) {
            Some((id, _)) => {
                let Some(s) = self.canvas.screen(&id) else {
                    return;
                };
                let (x, y) = ((w.0 - s.x).round(), (w.1 - s.y).round());
                self.select_screen(Some(id.clone()));
                self.open_comment(CommentOpen::New { screen: id, x, y });
            }
            None => self.comment_open = None,
        }
        cx.notify();
    }

    fn open_comment(&mut self, open: CommentOpen) {
        self.comment_open = Some(open);
        self.comment_focus = true;
    }

    fn post_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.f_comment.read(cx).value().trim().to_string();
        let Some(open) = self.comment_open.clone() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if self.author.is_empty() {
            self.author = notes::author(&self.root);
        }
        let (author, at) = (self.author.clone(), notes::now());
        let for_ai = notes::mentions_name(&text, &self.people, notes::AI);
        // Read the file again first, so comments added elsewhere aren't lost.
        self.comments = notes::load(&self.root);
        let id = match open {
            CommentOpen::New { screen, x, y } => {
                let id = notes::next_id(&self.comments);
                self.comments.push(Comment {
                    id,
                    screen,
                    x,
                    y,
                    author,
                    text,
                    at,
                    resolved: false,
                    replies: Vec::new(),
                });
                self.comment_open = Some(CommentOpen::Thread(id));
                id
            }
            CommentOpen::Thread(id) => {
                if let Some(c) = self.comments.iter_mut().find(|c| c.id == id) {
                    c.replies.push(Reply { author, text, at });
                }
                id
            }
        };
        self.save_comments();
        self.f_comment
            .update(cx, |s, cx| s.set_value("", window, cx));
        if for_ai {
            self.fix_comments(vec![id], window, cx);
        }
        cx.notify();
    }

    fn save_comments(&mut self) {
        if let Err(e) = notes::save(&self.root, &self.comments) {
            self.say(e);
        }
        self.note_writes();
    }

    fn resolve_comment(&mut self, id: u32, cx: &mut Context<Self>) {
        self.comments = notes::load(&self.root);
        if let Some(c) = self.comments.iter_mut().find(|c| c.id == id) {
            c.resolved = !c.resolved;
            if c.resolved {
                self.comment_open = None;
            }
        }
        self.save_comments();
        cx.notify();
    }

    fn delete_comment(&mut self, id: u32, cx: &mut Context<Self>) {
        self.comments = notes::load(&self.root);
        self.comments.retain(|c| c.id != id);
        self.comment_open = None;
        self.save_comments();
        cx.notify();
    }

    /// Finish the mention being typed as `name`.
    fn mention(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = notes::complete(self.f_comment.read(cx).value().as_ref(), name);
        let input = self.f_comment.clone();
        input.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Where a point on a screen is in the view: on the canvas, or in Present on the
    /// screen playing (and nowhere for the others).
    fn pin_point(&self, screen: &str, x: f32, y: f32) -> Option<Pt> {
        match &self.playing {
            Some(playing) if playing == screen => {
                let (left, top, scale) = self.play_frame()?;
                Some((left + x * scale, top + y * scale))
            }
            Some(_) => None,
            None => self
                .canvas
                .screen(screen)
                .map(|s| self.screen_pt((s.x + x, s.y + y))),
        }
    }

    /// In Present, where the screen playing sits in the view and how it's scaled.
    pub(super) fn play_frame(&self) -> Option<(f32, f32, f32)> {
        let s = self.canvas.screen(self.playing.as_ref()?)?;
        let (vw, vh) = self.view_size();
        let (w, h) = s.device.size();
        let scale = ((vh - 80.) / h).min((vw - 80.) / w).min(1.);
        Some((
            (vw - w * scale) / 2.,
            40. + (vh - 40. - h * scale) / 2.,
            scale,
        ))
    }

    /// A click on the screen playing with comments on: start a comment there.
    pub(super) fn play_comment_down(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let (Some(id), Some((left, top, scale))) = (self.playing.clone(), self.play_frame()) else {
            return;
        };
        let (x, y) = self.local(pos);
        let at = (((x - left) / scale).round(), ((y - top) / scale).round());
        if self.comment_open.is_some() {
            self.comment_open = None;
        } else {
            self.open_comment(CommentOpen::New {
                screen: id,
                x: at.0,
                y: at.1,
            });
        }
        cx.notify();
    }

    /// Put the cursor in the comment box once, when a comment opens.
    pub(super) fn focus_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.comment_focus) || self.comment_open.is_none() {
            return;
        }
        let input = self.f_comment.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
    }

    /// A comment as an agent reads it: where it's pinned, on what, and what was said.
    fn comment_line(&self, c: &Comment) -> Option<String> {
        let name = self.canvas.screen(&c.screen)?.name.clone();
        let under = self
            .shots
            .get(&c.screen)
            .and_then(|s| dom::under(&s.boxes, c.x, c.y, None))
            .map(|b| b.tag.clone());
        Some(notes::line(c, &name, under.as_deref()))
    }

    /// Every open comment as an agent reads it, for the context of an ask.
    pub(super) fn open_comment_lines(&self) -> Vec<String> {
        notes::load(&self.root)
            .iter()
            .filter(|c| !c.resolved)
            .filter_map(|c| self.comment_line(c))
            .collect()
    }

    /// Hand comments to AI to change the design. Keeping its change resolves them.
    pub(super) fn fix_comments(
        &mut self,
        ids: Vec<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.comments = notes::load(&self.root);
        let picked: Vec<Comment> = self
            .comments
            .iter()
            .filter(|c| ids.contains(&c.id) && !c.resolved)
            .cloned()
            .collect();
        let lines: Vec<String> = picked.iter().filter_map(|c| self.comment_line(c)).collect();
        if lines.is_empty() {
            self.say("Those comments' screens are gone.");
            cx.notify();
            return;
        }
        if let [one] = picked.as_slice() {
            self.select_screen(Some(one.screen.clone()));
        }
        if self.ask(notes::task(&lines), window, cx) {
            if let Some(ai) = self.ai.as_mut() {
                ai.comments = picked.iter().map(|c| c.id).collect();
            }
            self.comment_open = None;
            self.say("AI is on it. Keeping its change resolves the comment.");
        }
        cx.notify();
    }

    /// Mark the comments an AI change answered as resolved, once it's kept.
    pub(super) fn resolve_fixed(&mut self, ids: &[u32], what: &str) {
        if ids.is_empty() {
            return;
        }
        self.comments = notes::load(&self.root);
        for c in self.comments.iter_mut().filter(|c| ids.contains(&c.id)) {
            c.resolved = true;
            c.replies.push(Reply {
                author: "AI".into(),
                text: format!("Fixed: {what}"),
                at: notes::now(),
            });
        }
        self.save_comments();
    }

    /// Show a comment's thread, from the list.
    fn show_comment(&mut self, id: u32, cx: &mut Context<Self>) {
        if let Some(c) = self.comments.iter().find(|c| c.id == id) {
            let screen = c.screen.clone();
            // In Present, go to the screen it's on.
            if self.playing.as_ref().is_some_and(|p| *p != screen)
                && self.canvas.screen(&screen).is_some()
            {
                self.playing = Some(screen.clone());
                self.play_taps.clear();
                self.play_move = None;
                self.arm_delay(cx);
            }
            self.select_screen(Some(screen));
        }
        self.open_comment(CommentOpen::Thread(id));
        cx.notify();
    }

    // ---- What's drawn

    /// The pins on the screens and the open comment, over the canvas.
    pub(super) fn comment_layer(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if !self.commenting {
            return out;
        }
        let open_id = match &self.comment_open {
            Some(CommentOpen::Thread(id)) => Some(*id),
            _ => None,
        };
        let pin_at = |screen: &str, x: f32, y: f32| self.pin_point(screen, x, y);
        for (i, c) in self.comments.iter().enumerate() {
            if c.resolved && open_id != Some(c.id) {
                continue;
            }
            let Some(p) = pin_at(&c.screen, c.x, c.y) else {
                continue;
            };
            let on = open_id == Some(c.id);
            let id = c.id;
            out.push(
                div()
                    .id(("design-pin", i))
                    .absolute()
                    .left(px(p.0))
                    .top(px(p.1 - PIN))
                    .size(px(PIN))
                    .p(px(2.))
                    .rounded_tl(px(PIN / 2.))
                    .rounded_tr(px(PIN / 2.))
                    .rounded_br(px(PIN / 2.))
                    .bg(rgb(if on { c_blue() } else { 0xffffff }))
                    .shadow_md()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if this.comment_open == Some(CommentOpen::Thread(id)) {
                                this.comment_open = None;
                            } else {
                                this.open_comment(CommentOpen::Thread(id));
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        avatar(&c.author, PIN - 4.)
                            .rounded_bl(px(0.))
                            .when(c.resolved, |el| el.bg(rgb(c_muted()))),
                    )
                    .into_any_element(),
            );
        }
        let Some(open) = &self.comment_open else {
            return out;
        };
        let (screen, x, y) = match open {
            CommentOpen::New { screen, x, y } => (screen.clone(), *x, *y),
            CommentOpen::Thread(id) => match self.comments.iter().find(|c| c.id == *id) {
                Some(c) => (c.screen.clone(), c.x, c.y),
                None => return out,
            },
        };
        let Some(p) = pin_at(&screen, x, y) else {
            return out;
        };
        if matches!(open, CommentOpen::New { .. }) {
            out.push(
                div()
                    .absolute()
                    .left(px(p.0))
                    .top(px(p.1 - PIN))
                    .size(px(PIN))
                    .p(px(2.))
                    .rounded_tl(px(PIN / 2.))
                    .rounded_tr(px(PIN / 2.))
                    .rounded_br(px(PIN / 2.))
                    .bg(rgb(c_blue()))
                    .shadow_md()
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::Plus, 14., 0xffffff)),
                    )
                    .into_any_element(),
            );
        }
        out.push(self.comment_popover(open.clone(), p, cx));
        out
    }

    fn comment_popover(&self, open: CommentOpen, p: Pt, cx: &mut Context<Self>) -> AnyElement {
        let thread = match &open {
            CommentOpen::Thread(id) => self.comments.iter().find(|c| c.id == *id),
            _ => None,
        };
        let screen_name = match &open {
            CommentOpen::New { screen, .. } => screen.clone(),
            CommentOpen::Thread(_) => thread.map(|c| c.screen.clone()).unwrap_or_default(),
        };
        let screen_name = self
            .canvas
            .screen(&screen_name)
            .map(|s| s.name.clone())
            .unwrap_or(screen_name);
        let mut header = div()
            .h(px(36.))
            .px_3()
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child(if thread.is_some() {
                        format!("Comment on {screen_name}")
                    } else {
                        format!("New comment on {screen_name}")
                    }),
            );
        if let Some(c) = thread {
            let id = c.id;
            header = header
                .child(
                    figma::mini(
                        "design-comment-resolve",
                        IconName::CircleCheck,
                        if c.resolved { "Reopen" } else { "Resolve" },
                        c.resolved,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.resolve_comment(id, cx))),
                )
                .child(
                    figma::mini(
                        "design-comment-delete",
                        IconName::Trash,
                        "Delete the comment",
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.delete_comment(id, cx))),
                );
        }
        header = header.child(
            figma::mini(
                "design-comment-close",
                IconName::Close,
                "Close (Esc)",
                false,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.comment_open = None;
                cx.notify();
            })),
        );
        let mut body = div().v_flex().gap(px(10.)).px_3().py(px(10.));
        if let Some(c) = thread {
            body = body.child(message(&c.author, &c.at, &c.text, &self.people));
            for r in &c.replies {
                body = body.child(message(&r.author, &r.at, &r.text, &self.people));
            }
        }
        let field = div()
            .h(px(32.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(divider())
            .bg(tint(c_subtle()))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.f_comment)
                        .appearance(false)
                        .small()
                        .h(px(28.)),
                ),
            );
        let typed = self.f_comment.read(cx).value().to_string();
        let suggested: Vec<String> = notes::mention_query(&typed)
            .map(|q| {
                notes::suggest(&self.people, q)
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut picks = div().flex().flex_wrap().gap(px(4.));
        for (i, name) in suggested.iter().enumerate() {
            let name = name.clone();
            picks = picks.child(
                div()
                    .id(("design-mention", i))
                    .h(px(24.))
                    .pl(px(3.))
                    .pr(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .rounded_full()
                    .cursor_pointer()
                    .border_1()
                    .border_color(divider())
                    .text_size(px(11.5))
                    .text_color(rgb(c_text()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .child(avatar(&name, 18.))
                    .child(name.clone())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.mention(&name, window, cx)),
                    ),
            );
        }
        let mut foot = div()
            .px_3()
            .pb(px(10.))
            .v_flex()
            .gap(px(6.))
            .child(field)
            .when(!suggested.is_empty(), |f| f.child(picks))
            .child(figma::caption(if thread.is_some() {
                "Enter to reply. @ mentions someone; @AI hands it to AI."
            } else {
                "Enter to post. @ mentions someone; @AI hands it to AI."
            }));
        if let Some(c) = thread.filter(|c| !c.resolved) {
            let id = c.id;
            foot = foot.child(
                div()
                    .id("design-comment-fix")
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .border_1()
                    .border_color(divider())
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(c_text()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fix_comments(vec![id], window, cx)
                    }))
                    .child(icon(IconName::Sparkles, 13., c_blue()))
                    .child("Fix with AI"),
            );
        }
        // Kept inside the canvas: a thread near the bottom opens upward.
        let lines = thread.map_or(0, |c| 1 + c.replies.len()) as f32;
        let tall = 126.
            + lines * 56.
            + if thread.is_some() { 36. } else { 0. }
            + if suggested.is_empty() { 0. } else { 30. };
        let (vw, vh) = self.view_size();
        let top = (p.1 - PIN).min(vh - tall - 12.).max(8.);
        let left = if p.0 + PIN + 8. + POPOVER_W > vw - 8. {
            p.0 - POPOVER_W - 8.
        } else {
            p.0 + PIN + 8.
        };
        div()
            .id("design-comment-popover")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(POPOVER_W))
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(divider())
            .bg(float(c_canvas()))
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(header)
            .child(body)
            .child(foot)
            .into_any_element()
    }

    /// The right panel while commenting: every comment, open ones first.
    pub(super) fn comments_panel(&self, cx: &mut Context<Self>) -> Div {
        let open: Vec<&Comment> = self.comments.iter().filter(|c| !c.resolved).collect();
        let done = self.comments.len() - open.len();
        let mut out = div().v_flex();
        if self.comments.is_empty() {
            return out.child(div().px_3().pt_3().child(figma::caption(
                "Click anywhere on a screen to pin a comment there. Comments are kept in \
                 design/comments.toml, where agents read them too.",
            )));
        }
        let shown = open.into_iter().chain(
            self.comments
                .iter()
                .filter(|c| c.resolved && self.show_resolved),
        );
        let me = self.author.clone();
        let mentions_me = |c: &Comment| {
            std::iter::once(&c.text)
                .chain(c.replies.iter().map(|r| &r.text))
                .any(|t| notes::mentions_name(t, &self.people, &me))
        };
        for (i, c) in shown.enumerate() {
            let id = c.id;
            let on = self.comment_open == Some(CommentOpen::Thread(id));
            let screen = self
                .canvas
                .screen(&c.screen)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| c.screen.clone());
            let replies = c.replies.len();
            out = out.child(
                div()
                    .id(("design-comment-row", i))
                    .px_3()
                    .py(px(10.))
                    .v_flex()
                    .gap(px(4.))
                    .cursor_pointer()
                    .border_b_1()
                    .border_color(divider())
                    .when(on, |el| el.bg(tint(c_selected())))
                    .when(c.resolved, |el| el.opacity(0.6))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, _, _, cx| this.show_comment(id, cx)))
                    .child(message(&c.author, &c.at, &c.text, &self.people))
                    .child(
                        div()
                            .pl(px(30.))
                            .flex()
                            .gap(px(8.))
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(screen)
                            .when(replies > 0, |el| {
                                el.child(if replies == 1 {
                                    "1 reply".to_string()
                                } else {
                                    format!("{replies} replies")
                                })
                            })
                            .when(c.resolved, |el| el.child("Resolved"))
                            .when(mentions_me(c), |el| {
                                el.child(
                                    div()
                                        .text_color(rgb(c_blue()))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Mentions you"),
                                )
                            }),
                    ),
            );
        }
        let open_ids: Vec<u32> = self
            .comments
            .iter()
            .filter(|c| !c.resolved)
            .map(|c| c.id)
            .collect();
        if open_ids.len() > 1 {
            out = out.child(
                div().px_3().pt_3().child(
                    div()
                        .id("design-comments-fix-all")
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .border_1()
                        .border_color(divider())
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(c_text()))
                        .hover(|el| el.bg(tint(c_subtle())))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.fix_comments(open_ids.clone(), window, cx)
                        }))
                        .child(icon(IconName::Sparkles, 13., c_blue()))
                        .child(format!(
                            "Fix all {} with AI",
                            self.comments.iter().filter(|c| !c.resolved).count()
                        )),
                ),
            );
        }
        if done > 0 {
            out = out.child(
                div()
                    .id("design-comments-resolved")
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(rgb(c_blue()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_resolved = !this.show_resolved;
                        cx.notify();
                    }))
                    .child(if self.show_resolved {
                        "Hide resolved".to_string()
                    } else {
                        format!("Show {done} resolved")
                    }),
            );
        }
        out
    }
}
