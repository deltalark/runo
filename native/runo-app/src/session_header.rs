//! The bar across the top of a session: its title and branch on the left, the session's
//! actions and status on the right. The bar measures itself and, as it narrows, gives up
//! labels before it gives up the title: first the actions go to icons, then the Chat / CLI
//! switch and status, and last the actions and branch fold into a "More" menu.

use super::*;
use gpui_kit::component::tooltip::Tooltip;

/// How much of the header fits, widest first.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub(crate) enum Fit {
    /// Everything with its label.
    Full,
    /// Open Design, Playbooks and Runs on as icons.
    Icons,
    /// Also Chat / CLI as icons, the status as its dot, and a shorter branch.
    Tight,
    /// The actions and the branch move into the More menu.
    Narrow,
}

/// Room the title keeps before anything else gives way.
const TITLE_ROOM: f32 = 260.;
/// Roughly how wide a character of 12px and 14px medium text sets.
const CHAR_12: f32 = 6.2;
const CHAR_14: f32 = 7.8;
const GAP: f32 = 12.;
/// An icon-only header button, 26px square, and the 8px after it.
const ICON_BUTTON: f32 = 34.;
/// The estimates run a little short of real text, so keep some spare.
const SLACK: f32 = 40.;

/// What the header has to show, to work out how much of it fits.
#[derive(Clone, Copy, Default)]
pub(crate) struct Parts {
    pub title: usize,
    pub branch: usize,
    pub worktree: bool,
    /// The "Open Design" or "Open Architecture" label, when asked from a canvas.
    pub canvas: Option<usize>,
    pub browser: bool,
    /// The Runs on label, when it's shown.
    pub runs_on: Option<usize>,
    /// The status label, when the session isn't working.
    pub status: Option<usize>,
}

impl Parts {
    /// The width the header needs at `fit` to keep the title its room.
    fn need(&self, fit: Fit) -> f32 {
        let label = |chars: usize| chars as f32 * CHAR_12;
        // A text button: 8px each side, its icon and the 5px gap, then 8px after it.
        let action = |chars: usize| {
            if fit >= Fit::Icons {
                ICON_BUTTON
            } else {
                16. + 12. + 5. + label(chars) + 8.
            }
        };
        let mut items = vec![(self.title as f32 * CHAR_14 + 20.).min(TITLE_ROOM)];
        if fit < Fit::Narrow {
            if self.branch > 0 {
                let cap = if fit >= Fit::Tight { 96. } else { 160. };
                items
                    .push(18. + label(self.branch).min(cap) + if self.worktree { 68. } else { 0. });
            }
            items.extend(self.canvas.map(action));
            items.push(action("Playbooks".len()));
            if self.browser {
                items.push(37.);
            }
            items.extend(self.runs_on.map(action));
        }
        // Chat / CLI: a 2px track around two options of icon, gap and label.
        items.push(if fit >= Fit::Tight {
            4. + 2. * 28.
        } else {
            4. + 2. * (16. + 12. + 5.) + label("Chat".len() + "CLI".len())
        });
        items.extend(self.status.map(|chars| {
            if fit >= Fit::Tight {
                6.
            } else {
                12. + label(chars)
            }
        }));
        // Delete, or More in its place.
        items.push(28.);
        // The spacer between the left and right groups is a child too.
        let gaps = items.len() as f32 * GAP;
        32. + SLACK + gaps + items.iter().sum::<f32>()
    }

    /// The widest fit for a header `width` wide.
    pub(crate) fn fit(&self, width: f32) -> Fit {
        [Fit::Full, Fit::Icons, Fit::Tight]
            .into_iter()
            .find(|fit| self.need(*fit) <= width)
            .unwrap_or(Fit::Narrow)
    }
}

impl RunoApp {
    pub(crate) fn session_header(
        &self,
        session: &Session,
        working: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = session.id;
        let isolated = session.isolated;
        let on_canvas = shell::canvas_of(session);
        let serving = cx.global::<AppSettings>().browser_enabled && self.app_serving(session);
        let runs_on = self.runs_on_label(session);
        let parts = Parts {
            title: session.title.chars().count(),
            branch: session.branch.chars().count(),
            worktree: isolated,
            canvas: on_canvas.map(|v| "Open ".len() + shell::canvas_title(v).len()),
            browser: serving,
            runs_on: runs_on.as_ref().map(|l| l.chars().count()),
            status: (!working).then(|| Self::status_label(session.status).len()),
        };
        let fit = parts.fit(self.header_width.get());
        let icons = fit >= Fit::Icons;
        let tight = fit >= Fit::Tight;
        let narrow = fit == Fit::Narrow;
        // Measured as it lays out; a width that changes the fit draws again with it.
        let measure = {
            let width = self.header_width.clone();
            canvas(
                move |b, window, _| {
                    let w = f32::from(b.size.width);
                    // Too late to change this frame, so ask for the next one.
                    if parts.fit(width.replace(w)) != parts.fit(w) {
                        window.on_next_frame(|window, _| window.refresh());
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        // A header action: its label when there's room, otherwise just its icon with the
        // label as the tooltip.
        let action = |key: &'static str, glyph: IconName, size: f32, label: SharedString| {
            let tip = label.clone();
            div()
                .id(key)
                .h(px(26.))
                .when(icons, |el| el.w(px(26.)).justify_center())
                .when(!icons, |el| el.px_2())
                .mr_2()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .rounded_md()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(rgb(c_secondary()))
                .hover(|el| el.bg(tint(c_subtle())))
                .when(icons, |el| {
                    el.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                })
                .child(icon(glyph, size, c_secondary()))
                .when(!icons, |el| el.child(label))
        };
        let title = if self.renaming {
            div()
                .w(px(360.))
                .min_w(px(120.))
                .flex_shrink(1.)
                .child(Input::new(&self.rename).bordered(true).h(px(28.)))
                .into_any_element()
        } else {
            // With the branch folded away, the tooltip names it.
            let tip: SharedString = if narrow && !session.branch.is_empty() {
                format!("{} · Click to rename", session.branch).into()
            } else {
                "Click to rename".into()
            };
            div()
                .id("session-title")
                .flex()
                .items_center()
                .gap_2()
                // The title is what says which session this is, so the branch and the
                // actions give way before it does.
                .min_w(px(96.))
                .flex_shrink(1.)
                .cursor_pointer()
                .group("title")
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.start_rename(window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .truncate()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(session.title.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .opacity(0.)
                        .group_hover("title", |el| el.opacity(1.))
                        .child(icon(IconName::Pencil, 12., c_muted())),
                )
                .into_any_element()
        };
        div()
            .relative()
            .h(px(44.))
            .flex_none()
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(divider())
            .child(measure)
            .child(title)
            .when(!narrow, |el| {
                el.child(
                    // The branch keeps room for a short name like "local folder", gives
                    // way before the title, and a long one is cut, with the full name in
                    // the tooltip.
                    div()
                        .id("session-branch")
                        .flex()
                        .min_w(px(64.))
                        .flex_shrink(4.)
                        .tooltip({
                            let branch = session.branch.clone();
                            move |window, cx| Tooltip::new(branch.clone()).build(window, cx)
                        })
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(rgb(c_muted()))
                        .child(icon(IconName::GitBranch, 12., c_muted()).flex_none())
                        .child(
                            div()
                                .max_w(px(if tight { 96. } else { 160. }))
                                .truncate()
                                .child(session.branch.clone()),
                        )
                        .when(isolated, |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .px(px(6.))
                                    .py(px(1.))
                                    .rounded_sm()
                                    .bg(tint(c_subtle()))
                                    .text_size(px(11.))
                                    .child("worktree"),
                            )
                        }),
                )
            })
            .child(div().flex_1())
            .when(!narrow, |el| {
                el
                    // Asked from a canvas: back to it, where its changes show.
                    .children(on_canvas.map(|view| {
                        action(
                            "hdr-canvas",
                            if view == View::Design {
                                Mode::Design.glyph()
                            } else {
                                Mode::Architecture.glyph()
                            },
                            12.,
                            format!("Open {}", shell::canvas_title(view)).into(),
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.open_session(id, window, cx);
                                cx.notify();
                            },
                        ))
                    }))
                    .child(
                        action("hdr-playbooks", IconName::Play, 11., "Playbooks".into())
                            .on_mouse_down(
                                MouseButton::Left,
                                self.on_context(
                                    ctxmenu::CtxTarget::Playbooks(session.project_id),
                                    cx,
                                ),
                            ),
                    )
                    // Only offered once something is being served for this session.
                    .when(serving, |el| {
                        el.child(
                            div()
                                .id("hdr-browser")
                                .h(px(26.))
                                .px_2()
                                .mr_2()
                                .flex()
                                .flex_none()
                                .items_center()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|el| el.bg(tint(c_subtle())))
                                .tooltip(|window, cx| {
                                    Tooltip::new("Open your app in a Browser tab").build(window, cx)
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_browser();
                                    cx.notify();
                                }))
                                .child(icon(IconName::Globe, 13., c_secondary())),
                        )
                    })
                    .children(runs_on.map(|label| {
                        let local = session.connection == connections::LOCAL;
                        let color = if local { c_secondary() } else { c_blue() };
                        action(
                            "hdr-runs-on",
                            if local {
                                IconName::Cpu
                            } else {
                                IconName::Server
                            },
                            11.,
                            label.into(),
                        )
                        .text_color(rgb(color))
                        .on_mouse_down(
                            MouseButton::Left,
                            self.on_context(ctxmenu::CtxTarget::RunsOn(id), cx),
                        )
                    }))
            })
            .child(self.view_toggle(session, tight, cx))
            .when(!working, |el| {
                let label = Self::status_label(session.status);
                el.child(
                    div()
                        .id("hdr-status")
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(rgb(c_secondary()))
                        .when(tight, |el| {
                            el.tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
                        })
                        .child(self.status_dot(session.status, "hdr-dot", 6.))
                        .when(!tight, |el| el.child(label)),
                )
            })
            .child(if narrow {
                self.ghost_button("hdr-more", IconName::Ellipsis, "More")
                    .on_mouse_down(
                        MouseButton::Left,
                        self.on_context(ctxmenu::CtxTarget::Header(id), cx),
                    )
            } else {
                self.ghost_button("delete", IconName::Trash, "Delete session")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if confirm_delete() {
                            this.confirm_delete = Some(id);
                        } else {
                            this.delete_session(id);
                        }
                        cx.notify();
                    }))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    fn parts() -> Parts {
        Parts {
            title: "Add a dark theme toggle".len(),
            branch: "main".len(),
            worktree: false,
            canvas: Some("Open Design".len()),
            browser: false,
            runs_on: Some("This machine".len()),
            status: Some("Completed".len()),
        }
    }

    #[test]
    fn narrows_in_order() {
        let p = parts();
        let mut last = Fit::Full;
        for w in (200..1400).rev().step_by(10) {
            let fit = p.fit(w as f32);
            assert!(fit >= last, "{w}px went back to {fit:?} after {last:?}");
            last = fit;
        }
        assert_eq!(p.fit(1400.), Fit::Full);
        assert_eq!(p.fit(200.), Fit::Narrow);
    }

    #[test]
    fn each_fit_is_reached() {
        let p = parts();
        let seen: std::collections::HashSet<_> = (200..1400).map(|w| p.fit(w as f32)).collect();
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn fewer_actions_fit_sooner() {
        let full = parts();
        let bare = Parts {
            canvas: None,
            runs_on: None,
            ..full
        };
        assert!(bare.need(Fit::Full) < full.need(Fit::Full));
        assert!(bare.fit(full.need(Fit::Full) - 1.) == Fit::Full);
    }
}
