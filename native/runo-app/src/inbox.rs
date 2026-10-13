//! The Inbox: one list of everything waiting on the person, across projects. Blocked
//! agents come first with their answer inline (Allow, Approve, an option), then sessions
//! that finished or failed unseen, then review requests and failed checks from GitHub.

use super::*;
use alerts::Waiting;
use shell::Need;

impl RunoApp {
    /// The inline answers for a blocked session: the same replies the corner cards send.
    fn waiting_actions(
        &self,
        n: usize,
        id: u64,
        index: usize,
        waiting: Waiting,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut actions = div().flex().flex_wrap().items_center().gap_2();
        match waiting {
            Waiting::Denied(tools) => {
                let (once, always) = (tools.clone(), tools);
                actions = actions
                    .child(
                        self.button(("inbox-once", n), "Allow once", false)
                            .h(px(26.))
                            .child(icon(IconName::Check, 12., c_secondary()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.answer_session(
                                    id,
                                    format!(
                                        "You now have permission to use {}. Continue.",
                                        once.join(", ")
                                    ),
                                    once.clone(),
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(("inbox-always", n), "Always allow", true)
                            .h(px(26.))
                            .child(icon(IconName::ShieldCheck, 12., c_canvas()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(provider) =
                                    this.state.session(id).map(|s| s.provider.clone())
                                {
                                    this.allow_tools_for(&provider, &always, cx);
                                }
                                this.answer_session(
                                    id,
                                    format!(
                                        "You now have permission to use {}. Continue.",
                                        always.join(", ")
                                    ),
                                    Vec::new(),
                                    cx,
                                );
                                cx.notify();
                            })),
                    );
            }
            Waiting::Questions(questions) => {
                let (qi, q) = self.next_question(id, index, &questions);
                for (oi, option) in q.options.iter().take(4).enumerate() {
                    let option = option.clone();
                    actions = actions.child(
                        self.chip(("inbox-opt", n * 10 + oi), option.clone(), false)
                            .h(px(26.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pick_answer(id, index, qi, option.clone(), cx);
                                cx.notify();
                            })),
                    );
                }
            }
            Waiting::Plan => {
                actions = actions.child(
                    self.button(("inbox-plan", n), "Approve and build", true)
                        .h(px(26.))
                        .child(icon(IconName::Play, 12., c_canvas()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(s) = this.state.session_mut(id) {
                                s.opts.insert("mode".into(), "edit".into());
                            }
                            this.answer_session(
                                id,
                                "The plan is approved. Go ahead and implement it.".into(),
                                Vec::new(),
                                cx,
                            );
                            cx.notify();
                        })),
                );
            }
        }
        actions
    }

    /// Clear a session from the Inbox without opening it.
    fn dismiss_need(&mut self, id: u64) {
        if let Some(request) = self.approval_for(id) {
            self.alerts_hidden.insert(format!("a:{}", request.id));
        }
        if let Some((index, _)) = self.state.session(id).and_then(alerts::waiting_on) {
            self.alerts_hidden.insert(format!("s:{id}:{index}"));
        }
        if let Some(s) = self.state.session_mut(id) {
            s.unread = false;
        }
        self.save();
    }

    fn inbox_row(
        &self,
        n: usize,
        session: &Session,
        need: Need,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = session.id;
        let provider = agent::provider(&session.provider);
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let live = (need == Need::Blocked)
            .then(|| self.approval_for(id))
            .flatten();
        let waiting = (need == Need::Blocked && live.is_none())
            .then(|| alerts::waiting_on(session))
            .flatten();
        let (glyph, color, what) = match waiting.as_ref().map(|(_, w)| w) {
            _ if live.is_some() => (
                IconName::ShieldAlert,
                c_amber(),
                live.map(|r| alerts::approval_text(provider.name, r))
                    .unwrap_or_default(),
            ),
            Some(Waiting::Denied(tools)) => (
                IconName::ShieldAlert,
                c_amber(),
                format!("{} wants to use {}", provider.name, alerts::and_list(tools)),
            ),
            Some(Waiting::Questions(questions)) => {
                let index = waiting.as_ref().map_or(0, |(i, _)| *i);
                let (_, q) = self.next_question(id, index, questions);
                (IconName::CircleQuestionMark, c_amber(), q.question)
            }
            Some(Waiting::Plan) => (
                IconName::ListChecks,
                c_amber(),
                format!(
                    "{} proposed a plan and waits for your go-ahead",
                    provider.name
                ),
            ),
            None if need == Need::Failed => (
                IconName::CircleX,
                c_red(),
                "The agent stopped with an error".to_string(),
            ),
            None => (
                IconName::CircleCheck,
                c_green(),
                "Finished while you were away".to_string(),
            ),
        };
        let mut actions = match (live, waiting) {
            (Some(request), _) => self.approval_actions("inbox", n, request, cx),
            (None, Some((index, w))) => self.waiting_actions(n, id, index, w, cx),
            (None, None) => div().flex().items_center().gap_2(),
        };
        actions = actions
            .child(
                self.button(("inbox-open", n), "Open", need != Need::Blocked)
                    .h(px(26.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select(id);
                        cx.notify();
                    })),
            )
            .child(
                self.ghost_button_sized(
                    ("inbox-dismiss", n),
                    IconName::Check,
                    if need == Need::Blocked {
                        "Dismiss"
                    } else {
                        "Mark as read"
                    },
                    26.,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dismiss_need(id);
                    cx.notify();
                })),
            );
        div()
            .id(("inbox-row", id as usize))
            .on_mouse_down(
                MouseButton::Right,
                self.on_context(ctxmenu::CtxTarget::Session(id), cx),
            )
            .p_3()
            .v_flex()
            .gap_2()
            .rounded_xl()
            .border_1()
            .border_color(rgb(if self.cursor == Some(id) {
                c_blue()
            } else {
                c_border()
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.provider_badge(provider, 26.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .v_flex()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(session.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(c_muted()))
                                    .child(format!("{project}  ·  {}", session.branch)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(rgb(color))
                            .child(icon(glyph, 12., color))
                            .child(need.label()),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(c_secondary()))
                    .child(what),
            )
            .child(actions)
    }

    /// A GitHub item waiting on the person: a review request or a failed check.
    #[allow(clippy::too_many_arguments)]
    fn inbox_gh_row(
        &self,
        n: usize,
        glyph: IconName,
        color: u32,
        label: &'static str,
        title: String,
        url: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(("inbox-gh", n))
            .p_3()
            .flex()
            .items_center()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(rgb(c_border()))
            .child(icon(glyph, 16., color))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(div().text_xs().text_color(rgb(c_muted())).child(label)),
            )
            .when(!url.is_empty(), |el| {
                el.child(
                    self.button(("inbox-gh-open", n), "Open", false)
                        .h(px(26.))
                        .child(icon(IconName::ExternalLink, 12., c_secondary()))
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                )
            })
            .into_any_element()
    }

    pub(crate) fn inbox_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let waiting = self.needs_you();
        let mut rows = div().v_flex().gap_2();
        for (n, (session, need)) in waiting.iter().enumerate() {
            rows = rows.child(stagger(
                self.inbox_row(n, session, *need, cx),
                SharedString::from(format!("inbox-in-{}", session.id)),
                n,
            ));
        }
        let requested = self.gh.requested();
        let failed = github::latest_failures(&self.gh.runs);
        let mut n = 0;
        for pr in requested {
            rows = rows.child(self.inbox_gh_row(
                n,
                IconName::GitPullRequest,
                c_blue(),
                "Review requested",
                format!("#{} {}", pr.number, pr.title),
                pr.url.clone(),
                cx,
            ));
            n += 1;
        }
        for run in failed {
            rows = rows.child(self.inbox_gh_row(
                n,
                IconName::CircleX,
                c_red(),
                "Check failed",
                format!("{}  ·  {}", run.title, run.detail),
                run.url.clone(),
                cx,
            ));
            n += 1;
        }
        let unread = waiting.iter().any(|(_, need)| *need != Need::Blocked);
        if waiting.is_empty() && n == 0 {
            rows = rows.child(empty_hint(
                "inbox-empty",
                IconName::Inbox,
                "You're all caught up",
                "Permission prompts, plans to approve, questions from agents and sessions that finish while you're elsewhere land here.",
            ));
        }
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .text_size(px(12.5))
            .text_color(rgb(c_muted()))
            .child(match waiting.len() + n {
                0 => String::new(),
                1 => "1 thing waiting on you".to_string(),
                total => format!("{total} things waiting on you"),
            })
            .when(unread, |el| {
                el.child(
                    self.button("inbox-read-all", "Mark all as read", false)
                        .h(px(26.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            for s in this.state.sessions.iter_mut() {
                                s.unread = false;
                            }
                            this.save();
                            cx.notify();
                        })),
                )
            });
        self.page("Inbox", div().v_flex().gap_3().child(header).child(rows))
    }
}
