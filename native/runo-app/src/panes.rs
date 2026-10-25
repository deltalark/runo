//! The window's big parts as views of their own: the top bar, the mode rail, the sidebar,
//! the chat, the side panel and the status bar. Each is drawn as a cached view, so a frame
//! that changes only something else, such as a key typed in the composer, its blinking
//! caret, or a loop in another part, reuses what each part drew last time instead of
//! building, laying out and painting the whole window again.
//!
//! A part is drawn by `RunoApp` as before, from `Pane::render`. Every change to the app
//! (`cx.notify()` on it) draws all of them again, as it always did; what's new is that a
//! change only to the window's own frame, or to one part's motion (see `motion`), draws
//! only that.

use super::*;

/// Which part a pane draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Part {
    TopBar,
    Rail,
    Sidebar,
    Right,
    Status,
    /// The open chat's conversation.
    Chat,
    /// The conversation of the chat floating over a canvas.
    CanvasChat,
}

pub(crate) struct Pane {
    app: WeakEntity<RunoApp>,
    part: Part,
    _app_changed: Subscription,
}

thread_local! {
    /// How many times each part was drawn, for `bench`.
    static DRAWN: std::cell::RefCell<[u32; 7]> = const { std::cell::RefCell::new([0; 7]) };
}

/// How many times each part was drawn since the last call, by name.
pub(crate) fn take_drawn() -> String {
    let drawn = DRAWN.with(|d| std::mem::take(&mut *d.borrow_mut()));
    [
        Part::TopBar,
        Part::Rail,
        Part::Sidebar,
        Part::Right,
        Part::Status,
        Part::Chat,
        Part::CanvasChat,
    ]
    .iter()
    .zip(drawn)
    .map(|(part, n)| format!("{part:?} {n}"))
    .collect::<Vec<_>>()
    .join(", ")
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        DRAWN.with(|d| d.borrow_mut()[part as usize] += 1);
        let view = cx.entity_id();
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        motion::within(view, || {
            app.update(cx, |app, cx| app.draw_part(part, window, cx))
        })
    }
}

/// Draws nothing; notified, it draws the window's own frame again without the panes.
pub(crate) struct Ticker;

impl Render for Ticker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(crate) struct Panes {
    pub ticker: Entity<Ticker>,
    panes: Vec<Entity<Pane>>,
}

impl Panes {
    fn new(cx: &mut Context<RunoApp>) -> Panes {
        let app = cx.entity();
        let panes = [
            Part::TopBar,
            Part::Rail,
            Part::Sidebar,
            Part::Right,
            Part::Status,
            Part::Chat,
            Part::CanvasChat,
        ]
        .into_iter()
        .map(|part| {
            cx.new(|cx| Pane {
                app: app.downgrade(),
                part,
                _app_changed: cx.observe(&app, |_, _, cx| cx.notify()),
            })
        })
        .collect();
        Panes {
            ticker: cx.new(|_| Ticker),
            panes,
        }
    }

    /// The panes are made in `Part`'s order.
    fn get(&self, part: Part) -> &Entity<Pane> {
        &self.panes[part as usize]
    }
}

impl RunoApp {
    /// The panes, made on the first frame (they need the app's own entity).
    pub(crate) fn panes(&mut self, cx: &mut Context<Self>) -> &Panes {
        if self.panes.is_none() {
            self.panes = Some(Panes::new(cx));
        }
        self.panes.as_ref().expect("just made")
    }

    /// `part`, drawn as a cached view laid out at `style`, which must give it a definite
    /// size: a cached view isn't measured from what it holds. While the tour runs, parts
    /// are drawn every frame: it finds what it points at as it is painted, and a part
    /// reused from the last frame isn't painted again.
    pub(crate) fn part(&self, part: Part, style: StyleRefinement) -> AnyElement {
        let Some(panes) = &self.panes else {
            return div().into_any_element();
        };
        let pane = panes.get(part).clone();
        if self.tour.is_some() {
            let mut el = div();
            *el.style() = style;
            el.child(pane).into_any_element()
        } else {
            pane.cached(style).into_any_element()
        }
    }

    /// Draw the conversations again, the open chat's, the one over a canvas and the
    /// thread beside Crew's chat, and nothing else of the window.
    pub(crate) fn notify_chat(&self, cx: &mut Context<Self>) {
        if let Some(panes) = &self.panes {
            for part in [Part::Chat, Part::CanvasChat, Part::Right] {
                App::notify(cx, panes.get(part).entity_id());
            }
        }
    }

    /// What `part` shows.
    fn draw_part(&mut self, part: Part, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match part {
            Part::TopBar => self.top_bar(cx).into_any_element(),
            Part::Rail => self.mode_rail(cx),
            Part::Sidebar => self.sidebar(cx).into_any_element(),
            Part::Right => self.right_pane(window, cx).into_any_element(),
            Part::Status => self.status_bar(cx),
            Part::Chat | Part::CanvasChat => {
                let compact = part == Part::CanvasChat;
                let id = self.chat_shown[compact as usize].get();
                match id.and_then(|id| self.state.session(id)) {
                    Some(session) => {
                        let working = self.handles.contains_key(&Target::Session(session.id));
                        self.chat_list(session, working, compact, cx)
                    }
                    None => div().into_any_element(),
                }
            }
        }
    }
}
