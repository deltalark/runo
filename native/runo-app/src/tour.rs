//! The first-run tour: a spotlight that walks through the parts of the window and says what
//! each one does. It starts once on a fresh install and can be replayed from the palette or
//! from Settings.

use super::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Where each highlighted part of the window was drawn last frame, keyed by name.
pub(crate) type Anchors = Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>;

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Right,
    Left,
    Above,
    Below,
}

struct Step {
    /// The part of the window to highlight; `None` shows the card in the middle.
    anchor: Option<&'static str>,
    side: Side,
    glyph: IconName,
    title: &'static str,
    body: &'static str,
}

const STEPS: &[Step] = &[
    Step {
        anchor: None,
        side: Side::Below,
        glyph: IconName::Sparkles,
        title: "Welcome to Runo",
        body: "Runo runs coding agents like Claude Code, Codex and Gemini side by side, each in its own session. This quick tour shows where everything is. It takes about a minute.",
    },
    Step {
        anchor: Some("new-session"),
        side: Side::Right,
        glyph: IconName::SquarePen,
        title: "Start a session",
        body: "Describe a task, pick an agent and a project, and Runo starts it. Each session can run in its own isolated copy of the code, so agents never step on each other.",
    },
    Step {
        anchor: Some("nav"),
        side: Side::Right,
        glyph: IconName::Layers,
        title: "Everything in one place",
        body: "The rail on the left switches between Home, Sessions, Review, GitHub, Playbooks and History, then Design and Architecture, with Notes and Settings at the bottom. Press {go-home} to {go-history} to jump between them. The bell in the top bar opens the Inbox ({go-inbox}).",
    },
    Step {
        anchor: Some("projects"),
        side: Side::Right,
        glyph: IconName::Folder,
        title: "Projects and sessions",
        body: "Sessions are grouped by project. A dot means an agent is working or something waits for you. Right-click any session for pin, rename, open beside, archive and more.",
    },
    Step {
        anchor: Some("tabs"),
        side: Side::Below,
        glyph: IconName::PanelTop,
        title: "Tabs",
        body: "Open sessions stay in tabs so you can jump between agents while they work. {next-tab} moves to the next one, {close-tab} closes it, and you can drag tabs into any order.",
    },
    Step {
        anchor: Some("composer"),
        side: Side::Above,
        glyph: IconName::MessageSquare,
        title: "Talk to the agent",
        body: "Type what you want changed. Mention files with @, attach images, and pick the model, effort and permission mode below. Enter sends, Shift+Enter adds a line.",
    },
    Step {
        anchor: Some("agent-chip"),
        side: Side::Above,
        glyph: IconName::ArrowLeftRight,
        title: "Switch agents any time",
        body: "Hand the same session to another agent. Runo shows what carries over and what it costs before anything happens, and the new agent picks up exactly where the last one stopped.",
    },
    Step {
        anchor: Some("right-pane"),
        side: Side::Left,
        glyph: IconName::GitCompare,
        title: "Review the work",
        body: "Changes shows every edit as a diff with commit, push and pull request buttons. Browser previews your app, and Files lets you read and edit the code yourself.",
    },
    Step {
        anchor: Some("terminal"),
        side: Side::Left,
        glyph: IconName::SquareTerminal,
        title: "Terminal",
        body: "A real shell in the session's folder. Your saved commands appear as buttons, and the agent's own command output shows here too.",
    },
    Step {
        anchor: None,
        side: Side::Below,
        glyph: IconName::BellDot,
        title: "Approve from anywhere",
        body: "When any agent needs a permission, has a question or wants a plan approved, a card pops up in the corner. You can answer it without leaving what you're doing. Runo also tells you when an agent has an update.",
    },
    Step {
        anchor: Some("palette"),
        side: Side::Below,
        glyph: IconName::Search,
        title: "Quick open",
        body: "Jump to any session, project, file, setting or command from the keyboard. Press {quick-open}.",
    },
    Step {
        anchor: Some("settings"),
        side: Side::Right,
        glyph: IconName::Settings,
        title: "Make it yours",
        body: "Set up each agent, choose a theme, turn on the glass look, and decide what agents may do without asking. You can replay this tour from Settings or the palette.",
    },
    Step {
        anchor: None,
        side: Side::Below,
        glyph: IconName::Rocket,
        title: "You're all set",
        body: "Add a project folder and start your first session. Have fun building.",
    },
];

/// A step's text with each `{shortcut-id}` replaced by the keys it uses now, so the tour
/// follows the person's own keybindings.
pub(crate) fn with_keys(text: &str, label: impl Fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str(&label(&rest[start + 1..start + len]));
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// Marks the element it sits in as `key` for the tour. The parent must be `relative()`.
pub(crate) fn anchor(anchors: &Anchors, key: &'static str) -> impl IntoElement {
    let anchors = anchors.clone();
    canvas(
        move |bounds, _, _| {
            anchors.borrow_mut().insert(key, bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

const CARD_W: f32 = 360.;
const CARD_H: f32 = 230.;
const GAP: f32 = 14.;
const MARGIN: f32 = 12.;

impl RunoApp {
    /// Start the tour in Sessions with a session, the sidebar and the panel open, so every
    /// part it points at is on screen.
    pub(crate) fn start_tour(&mut self) {
        self.ctx_menu = None;
        self.switch_plan = None;
        self.menu = None;
        if self.current().is_none() {
            if let Some(id) = self
                .state
                .sessions
                .iter()
                .find(|s| !s.archived)
                .map(|s| s.id)
            {
                self.select(id);
            }
        }
        self.view = View::Session;
        self.show_sidebar = true;
        self.show_panel = true;
        self.panel_auto_hidden = false;
        self.tour = Some(0);
    }

    pub(crate) fn end_tour(&mut self, cx: &mut Context<Self>) {
        self.tour = None;
        let st = cx.global::<AppSettings>();
        if !st.tour_seen || !st.whats_new_seen {
            // The tour already shows the new layout, so its "What's new" card is not needed.
            settings::update(cx, |s| {
                s.tour_seen = true;
                s.whats_new_seen = true;
            });
        }
    }

    fn tour_go(&mut self, step: usize, cx: &mut Context<Self>) {
        if step >= STEPS.len() {
            self.end_tour(cx);
        } else {
            self.tour = Some(step);
        }
        cx.notify();
    }

    /// Arrow keys and Enter step through the tour. Returns whether the key was used.
    pub(crate) fn tour_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(at) = self.tour else { return false };
        match key {
            "right" | "enter" | "space" => self.tour_go(at + 1, cx),
            "left" if at > 0 => self.tour_go(at - 1, cx),
            _ => return false,
        }
        true
    }

    pub(crate) fn tour_view(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let at = self.tour?;
        let step = STEPS.get(at)?;
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let target = step.anchor.and_then(|k| self.tour_rects.get(k)).map(|b| {
            let pad = 6.;
            (
                f32::from(b.origin.x) - pad,
                f32::from(b.origin.y) - pad,
                f32::from(b.size.width) + pad * 2.,
                f32::from(b.size.height) + pad * 2.,
            )
        });
        let dim = if is_dark() {
            hsla(0., 0., 0., 0.62)
        } else {
            hsla(0., 0., 0., 0.38)
        };
        let shade = |x: f32, y: f32, sw: f32, sh: f32| {
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(sw.max(0.)))
                .h(px(sh.max(0.)))
                .bg(dim)
        };

        let last = at + 1 == STEPS.len();
        let body = with_keys(step.body, |id| keymap::label(cx, id));
        let dots = div().flex().gap(px(5.)).children((0..STEPS.len()).map(|i| {
            div()
                .h(px(6.))
                .w(px(if i == at { 16. } else { 6. }))
                .rounded_full()
                .bg(rgb(if i <= at { c_blue() } else { c_border() }))
        }));
        let card = div()
            .id("tour-card")
            .w(px(CARD_W))
            .p_5()
            .v_flex()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(if at == 0 {
                        // The welcome step introduces the app with its own icon.
                        img(app_icon()).size(px(40.)).flex_none().into_any_element()
                    } else {
                        div()
                            .size(px(32.))
                            .flex_none()
                            .rounded_lg()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(settings::mix(c_blue(), c_raised(), 0.16)))
                            .child(icon(step.glyph, 16., c_blue()))
                            .into_any_element()
                    })
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(c_muted()))
                                    .child(format!("{} of {}", at + 1, STEPS.len())),
                            )
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(step.title),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .text_color(rgb(c_secondary()))
                    .child(body),
            )
            .child(
                div()
                    .pt_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(dots)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(!last, |el| {
                                el.child(
                                    div()
                                        .id("tour-skip")
                                        .px_2()
                                        .h(px(28.))
                                        .flex()
                                        .items_center()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .text_size(px(12.5))
                                        .text_color(rgb(c_muted()))
                                        .hover(|el| el.text_color(rgb(c_text())))
                                        .child("Skip")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.end_tour(cx);
                                            cx.notify();
                                        })),
                                )
                            })
                            .when(at > 0, |el| {
                                el.child(self.button("tour-back", "Back", false).on_click(
                                    cx.listener(move |this, _, _, cx| this.tour_go(at - 1, cx)),
                                ))
                            })
                            .child(
                                self.button(
                                    "tour-next",
                                    if last {
                                        "Done"
                                    } else if at == 0 {
                                        "Show me"
                                    } else {
                                        "Next"
                                    },
                                    true,
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.tour_go(at + 1, cx)),
                                ),
                            ),
                    ),
            );
        let card = rise(card, SharedString::from(format!("tour-card-{at}")), 280);

        let mut layer = div()
            .id("tour")
            .absolute()
            .inset_0()
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation());
        match target {
            Some((x, y, tw, th)) => {
                layer = layer
                    .child(shade(0., 0., w, y))
                    .child(shade(0., y + th, w, h - y - th))
                    .child(shade(0., y, x, th))
                    .child(shade(x + tw, y, w - x - tw, th))
                    .child(
                        div()
                            .absolute()
                            .left(px(x))
                            .top(px(y))
                            .w(px(tw))
                            .h(px(th))
                            .rounded_lg()
                            .border_2()
                            .border_color(rgb(c_blue()))
                            .animated(
                                SharedString::from(format!("tour-ring-{at}")),
                                Animation::new(Duration::from_millis(1600)).repeat(),
                                |el, d| {
                                    el.opacity(
                                        0.55 + 0.45 * (d * std::f32::consts::TAU).cos().abs(),
                                    )
                                },
                            ),
                    );
                let (cx_, cy) = place(step.side, (x, y, tw, th), (w, h));
                layer = layer.child(div().absolute().left(px(cx_)).top(px(cy)).child(card));
            }
            None => {
                layer = layer.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(dim)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(card),
                );
            }
        }
        Some(fade(layer, SharedString::from("tour-fade"), 200))
    }
}

impl RunoApp {
    /// Once, for people who used Runo before the Workbench layout: what moved where.
    pub(crate) fn whats_new_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let st = cx.global::<AppSettings>();
        if !st.tour_seen
            || st.whats_new_seen
            || !st.browser_asked
            || self.tour.is_some()
            || self.view == View::Settings
            || self.demo
        {
            return None;
        }
        let points = [
            (
                IconName::Layers,
                format!(
                    "A rail on the left for Home, Sessions, Playbooks and Reviews ({} to {}).",
                    keymap::label(cx, "go-home"),
                    keymap::label(cx, "go-reviews")
                ),
            ),
            (
                IconName::Inbox,
                format!(
                    "One Inbox for permission prompts, plans, questions and finished runs ({}).",
                    keymap::label(cx, "go-inbox")
                ),
            ),
            (
                IconName::PanelTop,
                format!(
                    "Sessions open as tabs: {} for the next, {} to close, drag to reorder.",
                    keymap::label(cx, "next-tab"),
                    keymap::label(cx, "close-tab")
                ),
            ),
            (
                IconName::Folder,
                "The project switcher in the top bar; Home shows that project or all of them."
                    .to_string(),
            ),
        ];
        let mut list = div().v_flex().gap_2();
        for (glyph, text) in points {
            list = list.child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap_2()
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .text_color(rgb(c_secondary()))
                    .child(
                        div()
                            .flex_none()
                            .pt(px(2.))
                            .child(icon(glyph, 14., c_blue())),
                    )
                    // min_w_0 lets the text shrink to the card and wrap instead of spilling out.
                    .child(div().flex_1().min_w_0().child(text)),
            );
        }
        let close = |tour: bool| {
            cx.listener(move |this: &mut Self, _, _, cx| {
                settings::update(cx, |s| s.whats_new_seen = true);
                if tour {
                    this.start_tour();
                }
                cx.notify();
            })
        };
        let card = div()
            .id("whats-new")
            .w(px(CARD_W + 40.))
            .p_5()
            .v_flex()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(img(app_icon()).size(px(32.)).flex_none())
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("What's new: the Workbench"),
                    ),
            )
            .child(list)
            .child(
                div()
                    .pt_1()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("whats-new-tour", "Show me around", false)
                            .on_click(close(true)),
                    )
                    .child(
                        self.button("whats-new-ok", "Got it", true)
                            .on_click(close(false)),
                    ),
            );
        let dim = if is_dark() {
            hsla(0., 0., 0., 0.62)
        } else {
            hsla(0., 0., 0., 0.38)
        };
        let layer = div()
            .id("whats-new-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(dim)
            .flex()
            .items_center()
            .justify_center()
            .child(rise(card, SharedString::from("whats-new-card"), 280));
        Some(fade(layer, SharedString::from("whats-new-fade"), 200))
    }

    /// Once, after the tour (or after updating, for people who saw it already): ask whether
    /// agents may use the built-in browser.
    pub(crate) fn browser_offer(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let st = cx.global::<AppSettings>();
        if !st.tour_seen || st.browser_asked || self.tour.is_some() || self.view == View::Settings {
            return None;
        }
        let found = browser_engine::find_chromium();
        let body = match &found {
            Some(path) => format!(
                "Runo can open your app in the Browser tab using a headless copy of {}, with its own profile, so you and your agents can see and click through pages without leaving Runo. You can change this in Settings.",
                browser_engine::browser_name(path)
            ),
            None => "Runo can open your app in the Browser tab using a headless Chromium. None was found in Applications; install Chrome, Edge or Brave, or wait for the Chrome for Testing download coming next. You can turn this on later in Settings.".to_string(),
        };
        let answer = |on: bool| {
            cx.listener(move |_: &mut Self, _, _, cx| {
                settings::update(cx, |s| {
                    s.browser_asked = true;
                    s.browser_enabled = on;
                });
            })
        };
        let card = div()
            .id("browser-offer")
            .w(px(CARD_W))
            .p_5()
            .v_flex()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_canvas()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(32.))
                            .flex_none()
                            .rounded_lg()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(settings::mix(c_blue(), c_raised(), 0.16)))
                            .child(icon(IconName::Globe, 16., c_blue())),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Let agents use a built-in browser?"),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .text_color(rgb(c_secondary()))
                    .child(body),
            )
            .child(
                div()
                    .pt_1()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        self.button("browser-offer-later", "Not now", false)
                            .on_click(answer(false)),
                    )
                    .when(found.is_some(), |el| {
                        el.child(
                            self.button("browser-offer-on", "Turn on", true)
                                .on_click(answer(true)),
                        )
                    }),
            );
        let dim = if is_dark() {
            hsla(0., 0., 0., 0.62)
        } else {
            hsla(0., 0., 0., 0.38)
        };
        let layer = div()
            .id("browser-offer-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(dim)
            .flex()
            .items_center()
            .justify_center()
            .child(rise(card, SharedString::from("browser-offer-card"), 280));
        Some(fade(layer, SharedString::from("browser-offer-fade"), 200))
    }
}

/// Top-left corner for the card next to `target`, falling back to other sides when the
/// preferred one has no room, and always inside the window.
fn place(side: Side, target: (f32, f32, f32, f32), window: (f32, f32)) -> (f32, f32) {
    let (x, y, tw, th) = target;
    let (w, h) = window;
    let fits = |s: Side| match s {
        Side::Right => x + tw + GAP + CARD_W + MARGIN <= w,
        Side::Left => x - GAP - CARD_W >= MARGIN,
        Side::Above => y - GAP - CARD_H >= MARGIN,
        Side::Below => y + th + GAP + CARD_H + MARGIN <= h,
    };
    let order = [side, Side::Right, Side::Left, Side::Below, Side::Above];
    let side = order.into_iter().find(|s| fits(*s)).unwrap_or(side);
    let (cx, cy) = match side {
        Side::Right => (x + tw + GAP, y),
        Side::Left => (x - GAP - CARD_W, y),
        Side::Above => (x + tw / 2. - CARD_W / 2., y - GAP - CARD_H),
        Side::Below => (x + tw / 2. - CARD_W / 2., y + th + GAP),
    };
    (
        cx.clamp(MARGIN, (w - CARD_W - MARGIN).max(MARGIN)),
        cy.clamp(MARGIN, (h - CARD_H - MARGIN).max(MARGIN)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn step_text_uses_the_current_keys() {
        let text = with_keys("Press {quick-open} or {go-home}.", |id| match id {
            "quick-open" => "⌘K".into(),
            _ => "Ctrl+1".into(),
        });
        assert_eq!(text, "Press ⌘K or Ctrl+1.");
        assert_eq!(with_keys("No keys {here", |_| "x".into()), "No keys {here");
    }

    #[test]
    fn cards_stay_beside_their_target_and_inside_the_window() {
        let window = (1600., 1000.);
        // The sidebar's New session button: the card sits to its right.
        assert_eq!(
            place(Side::Right, (8., 60., 248., 40.), window),
            (8. + 248. + GAP, 60.)
        );
        // The composer near the bottom: above it.
        let (_, y) = place(Side::Above, (300., 800., 700., 100.), window);
        assert_eq!(y, 800. - GAP - CARD_H);
        // The right pane fills the right edge: the card goes to its left.
        let (x, _) = place(Side::Right, (1140., 60., 460., 900.), window);
        assert_eq!(x, 1140. - GAP - CARD_W);
        // A target too close to the bottom still leaves the card on screen.
        let (_, y) = place(Side::Below, (10., 980., 100., 20.), window);
        assert!(y + CARD_H <= 1000. - MARGIN);
        assert!(STEPS.iter().all(|s| !s.body.is_empty()));
    }
}
