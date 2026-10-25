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
    /// The terminal panel under the side panel: drawn over the side panel's foot rather
    /// than inside it, so a shell's output draws only the terminal.
    Terminal,
}

impl Part {
    const ALL: [Part; 8] = [
        Part::TopBar,
        Part::Rail,
        Part::Sidebar,
        Part::Right,
        Part::Status,
        Part::Chat,
        Part::CanvasChat,
        Part::Terminal,
    ];
}

pub(crate) struct Pane {
    app: WeakEntity<RunoApp>,
    part: Part,
    _app_changed: Subscription,
}

/// How often a part was drawn, and how long building, laying out and prepainting it took
/// in all and at most, for `bench`.
#[derive(Clone, Copy, Default)]
struct Drawn {
    count: u32,
    total: Duration,
    max: Duration,
}

thread_local! {
    static DRAWN: std::cell::RefCell<[Drawn; 8]> = std::cell::RefCell::new([Drawn::default(); 8]);
}

/// How many times each part was drawn since the last call, by name, with the time it took
/// when frame stats are on: "Right 3 (12.0 ms, max 5.1)".
pub(crate) fn take_drawn() -> String {
    let drawn = DRAWN.with(|d| std::mem::take(&mut *d.borrow_mut()));
    let ms = |d: Duration| d.as_secs_f64() * 1000.;
    Part::ALL
        .iter()
        .zip(drawn)
        .map(|(part, d)| {
            if d.total.is_zero() {
                format!("{part:?} {}", d.count)
            } else {
                format!(
                    "{part:?} {} ({:.1} ms, max {:.1})",
                    d.count,
                    ms(d.total),
                    ms(d.max)
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        let began = std::time::Instant::now();
        DRAWN.with(|d| d.borrow_mut()[part as usize].count += 1);
        let view = cx.entity_id();
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let el = motion::within(view, || {
            app.update(cx, |app, cx| app.draw_part(part, window, cx))
        });
        if framestats::on() {
            Timed {
                part,
                began,
                child: el,
            }
            .into_any_element()
        } else {
            el
        }
    }
}

/// A pane's content, timed from its render to the end of its prepaint: a cached view lays
/// its content out between the two, so this is the pane's whole cost but its paint.
struct Timed {
    part: Part,
    began: std::time::Instant,
    child: AnyElement,
}

impl IntoElement for Timed {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Timed {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
        let took = self.began.elapsed();
        DRAWN.with(|d| {
            let d = &mut d.borrow_mut()[self.part as usize];
            d.total += took;
            d.max = d.max.max(took);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
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
        let panes = Part::ALL
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

    /// Draw `parts` again, and nothing else of the window but its frame.
    pub(crate) fn notify_parts(&self, parts: &[Part], cx: &mut App) {
        if let Some(panes) = &self.panes {
            for &part in parts {
                cx.notify(panes.get(part).entity_id());
            }
        }
    }

    /// Draw the conversations again, the open chat's and the one over a canvas, and with
    /// `peek` the side panel too, where the thread beside Crew's chat is, and nothing else
    /// of the window.
    pub(crate) fn notify_chat(&self, peek: bool, cx: &mut App) {
        self.notify_parts(&[Part::Chat, Part::CanvasChat], cx);
        if peek {
            self.notify_parts(&[Part::Right], cx);
        }
    }

    /// Whether output from session `id` shows in the side panel: it is the thread open
    /// beside Crew's chat.
    pub(crate) fn peeked(&self, id: u64) -> bool {
        self.peek == Some(id) && self.show_panel && self.right_tab == RightTab::Threads
    }

    /// Draw again what output from agents whose chat isn't open changes: the sidebar, the
    /// top bar, the rail and the status bar. With Crew's chat open its thread cards and
    /// Threads panel follow the threads too, so those draw as well. Away from a session,
    /// the page shows sessions in its own ways (Home, a canvas), so the window draws.
    pub(crate) fn notify_background(&self, cx: &mut Context<Self>) {
        if self.view != View::Session || self.tour.is_some() {
            cx.notify();
            return;
        }
        self.notify_parts(&[Part::TopBar, Part::Rail, Part::Sidebar, Part::Status], cx);
        if self.current().is_some_and(|s| s.canvas == crew::CANVAS) {
            self.notify_parts(&[Part::Chat, Part::Right], cx);
        }
    }

    /// What `part` shows.
    fn draw_part(&mut self, part: Part, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match part {
            Part::TopBar => self.top_bar(cx).into_any_element(),
            Part::Rail => self.mode_rail(cx),
            Part::Sidebar => self.sidebar(cx).into_any_element(),
            Part::Right => self.right_pane(cx).into_any_element(),
            Part::Status => self.status_bar(cx),
            Part::Terminal => self.terminal_panel(window, cx).into_any_element(),
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
