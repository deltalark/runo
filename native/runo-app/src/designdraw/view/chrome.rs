//! The editor's chrome, laid out the way design tools are: the tools in a bar floating at
//! the bottom of the canvas, with the Design, Prototype and Code modes at its end; Present
//! and Hand off above the right panel, the zoom beside its tabs; the project and its pages
//! above the layers; and the menus that open from them, or from a right click.

use super::shapes::Shape;
use super::*;
use gpui_kit::component::Sizable;

/// The menus the chrome opens. One is open at a time.
#[derive(Clone, PartialEq)]
pub(super) enum Flyout {
    /// The move and hand tools.
    Move,
    /// The shapes and picture tools.
    Shapes,
    Zoom,
    Present,
    /// What can be done to the selection: from the right panel's ⋯ or a right click on the
    /// canvas, at that point.
    Selection(Option<Pt>),
    Help,
    /// How the width or height is sized: "w" or "h".
    Size(&'static str),
    /// How the selection is positioned.
    Place,
    /// A page's own menu, from a right click on its row.
    Page(String),
}

type Act = Rc<dyn Fn(&mut DesignCanvas, &mut Window, &mut Context<DesignCanvas>)>;

/// A line of a menu.
pub(super) enum Item {
    Do {
        label: SharedString,
        keys: &'static str,
        /// A tick in front, for a setting that's on.
        on: bool,
        danger: bool,
        act: Act,
    },
    Line,
}

impl Item {
    pub(super) fn new(
        label: impl Into<SharedString>,
        keys: &'static str,
        act: impl Fn(&mut DesignCanvas, &mut Window, &mut Context<DesignCanvas>) + 'static,
    ) -> Self {
        Item::Do {
            label: label.into(),
            keys,
            on: false,
            danger: false,
            act: Rc::new(act),
        }
    }

    fn ticked(mut self, yes: bool) -> Self {
        if let Item::Do { on, .. } = &mut self {
            *on = yes;
        }
        self
    }

    fn danger(mut self) -> Self {
        if let Item::Do { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
}

/// The bottom toolbar's height, and how far above the canvas's bottom edge it floats.
pub(super) const BAR_H: f32 = 44.;
pub(super) const BAR_GAP: f32 = 12.;

impl DesignCanvas {
    // ---- Pages

    /// The page shown.
    pub(super) fn page_now(&self) -> Option<String> {
        self.canvas.page_or_first(self.page.as_deref())
    }

    /// Whether a screen is on the page shown.
    pub(super) fn on_page(&self, s: &Screen) -> bool {
        self.page_now().is_none_or(|p| s.page() == p)
    }

    /// The screens on the page shown.
    pub(super) fn screens_here(&self) -> impl DoubleEndedIterator<Item = &Screen> + '_ {
        let page = self.page_now();
        self.canvas
            .screens
            .iter()
            .filter(move |s| page.as_deref().is_none_or(|p| s.page() == p))
    }

    /// The page a new screen for `device` gets: none when the page shown is the one that
    /// device's screens are on anyway, so the screen follows its device.
    pub(super) fn page_for(&self, device: Device) -> Option<String> {
        self.page_now().filter(|p| p != device.label())
    }

    pub(super) fn show_page(&mut self, page: String, cx: &mut Context<Self>) {
        if self.page_now().as_deref() == Some(page.as_str()) {
            return;
        }
        // New screens on a device's page are for that device.
        if let Some(d) = Device::ALL.into_iter().find(|d| d.label() == page) {
            self.new_device = d;
        }
        self.page = Some(page);
        // What's picked stays on its page.
        if self
            .screen
            .as_ref()
            .and_then(|id| self.canvas.screen(id))
            .is_some_and(|s| !self.on_page(s))
        {
            self.select_screen(None);
        }
        self.fitted = false;
        cx.notify();
    }

    pub(super) fn add_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.canvas.new_page_name();
        if self.canvas.pages.is_empty() {
            self.canvas.pages = self.canvas.page_list();
        }
        self.canvas.pages.push(name.clone());
        self.save_canvas();
        self.show_page(name.clone(), cx);
        self.start_page_rename(name, window, cx);
    }

    pub(super) fn start_page_rename(
        &mut self,
        page: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.f_page
            .update(cx, |s, cx| s.set_value(page.clone(), window, cx));
        let input = self.f_page.clone();
        window.defer(cx, move |window, cx| {
            input.update(cx, |s, cx| s.focus(window, cx))
        });
        self.renaming_page = Some(page);
        cx.notify();
    }

    pub(super) fn finish_page_rename(&mut self, typed: String, cx: &mut Context<Self>) {
        let Some(from) = self.renaming_page.take() else {
            return;
        };
        let to = typed.trim().to_string();
        if !to.is_empty() && to != from && !self.canvas.page_list().contains(&to) {
            self.canvas.rename_page(&from, &to);
            if self.page_now().is_none_or(|p| p == from) || self.page.as_deref() == Some(&from) {
                self.page = Some(to);
            }
            self.save_canvas();
        }
        cx.notify();
    }

    /// Remove an empty page. A page with screens on it stays.
    pub(super) fn delete_page(&mut self, page: &str, cx: &mut Context<Self>) {
        if self.canvas.screens.iter().any(|s| s.page() == page) {
            self.say("Move or delete the screens on this page first.");
            cx.notify();
            return;
        }
        self.canvas.pages = self.canvas.page_list();
        self.canvas.pages.retain(|p| p != page);
        if self.page.as_deref() == Some(page) {
            self.page = None;
        }
        self.save_canvas();
        cx.notify();
    }

    /// Move the selected screen to another page, and follow it there.
    pub(super) fn move_screen_to(&mut self, page: String, cx: &mut Context<Self>) {
        let Some(id) = self.screen.clone() else {
            return;
        };
        let spot = self.canvas.next_spot_on(&page);
        if let Some(s) = self.canvas.screen_mut(&id) {
            s.page = (page != s.device.label()).then(|| page.clone());
            (s.x, s.y) = spot;
        }
        self.save_canvas();
        self.page = Some(page);
        self.fitted = false;
        cx.notify();
    }

    // ---- The left panel's head and pages

    /// The project's name and where its design lives, with the button that puts the left
    /// panel away.
    pub(super) fn file_head(&self, cx: &mut Context<Self>) -> AnyElement {
        let name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Design".into());
        div()
            .flex_none()
            .px_3()
            .pt(px(10.))
            .pb(px(8.))
            .flex()
            .items_start()
            .gap_2()
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_text()))
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(if self.branch.is_empty() {
                                format!("{FOLDER}/")
                            } else {
                                format!("{FOLDER}/ · {}", self.branch)
                            }),
                    ),
            )
            .child(
                super::figma::mini(
                    "design-layers-toggle",
                    IconName::PanelLeftClose,
                    "Hide layers (⌘\\)",
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_layers(cx))),
            )
            .into_any_element()
    }

    /// The pages, the one shown picked: click one to show it, double-click to rename it.
    pub(super) fn pages_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pages = self.canvas.page_list();
        let now = self.page_now();
        let mut list = div().v_flex().px_1();
        for (i, page) in pages.iter().enumerate() {
            let count = self
                .canvas
                .screens
                .iter()
                .filter(|s| s.page() == page)
                .count();
            let on = now.as_deref() == Some(page.as_str());
            let renaming = self.renaming_page.as_deref() == Some(page.as_str());
            let (p1, p2) = (page.clone(), page.clone());
            list = list.child(
                div()
                    .id(("design-page", i))
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(c_text()))
                    .when(on, |el| {
                        el.bg(tint(c_selected())).font_weight(FontWeight::MEDIUM)
                    })
                    .hover(|el| el.bg(tint(c_subtle())))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if e.click_count() >= 2 {
                            this.start_page_rename(p1.clone(), window, cx);
                        } else {
                            this.show_page(p1.clone(), cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.flyout = Some(Flyout::Page(p2.clone()));
                            cx.notify();
                        }),
                    )
                    .child(if renaming {
                        div()
                            .flex_1()
                            .min_w_0()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(Input::new(&self.f_page).xsmall())
                    } else {
                        div().flex_1().min_w_0().truncate().child(page.clone())
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(10.5))
                            .text_color(rgb(c_muted()))
                            .child(count.to_string()),
                    )
                    .when(
                        self.flyout.as_ref() == Some(&Flyout::Page(page.clone())),
                        |el| {
                            el.relative()
                                .child(self.page_menu(page.clone(), window, cx))
                        },
                    ),
            );
        }
        div()
            .flex_none()
            .pb(px(6.))
            .border_b_1()
            .border_color(divider())
            .child(
                div()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_text()))
                    .child("Pages")
                    .child(
                        super::figma::mini("design-page-add", IconName::Plus, "Add a page", false)
                            .on_click(cx.listener(|this, _, window, cx| this.add_page(window, cx))),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    fn page_menu(&self, page: String, _: &Window, cx: &mut Context<Self>) -> AnyElement {
        let (a, b) = (page.clone(), page.clone());
        self.menu_at(
            "design-page-menu",
            vec![
                Item::new("Rename", "", move |this, window, cx| {
                    this.start_page_rename(a.clone(), window, cx)
                }),
                Item::new("Delete page", "", move |this, _, cx| {
                    this.delete_page(&b, cx)
                })
                .danger(),
            ],
            180.,
            Anchor::TopLeft,
            cx,
        )
    }

    // ---- The toolbar at the bottom of the canvas

    pub(super) fn bottom_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let sep = || div().w(px(1.)).h(px(22.)).mx(px(4.)).bg(divider());
        let moving = !self.hand && !self.commenting && self.shape.is_none() && self.pen.is_none();
        let shape_on = self.shape.is_some_and(|s| {
            matches!(s, Shape::Rect | Shape::Ellipse | Shape::Line | Shape::Arrow)
        });
        let shape_now = self.shape.filter(|_| shape_on).unwrap_or(self.last_shape);
        let shape_icon = |s: Shape| match s {
            Shape::Rect => IconName::Square,
            Shape::Ellipse => IconName::Circle,
            Shape::Line => IconName::Minus,
            Shape::Arrow => IconName::MoveUpRight,
            Shape::Pen => IconName::PenTool,
        };
        let mode = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(9.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(if on {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(rgb(if on { c_text() } else { c_muted() }))
                .when(on, |el| el.bg(tint(c_active())))
                .when(!on, |el| el.hover(|el| el.text_color(rgb(c_text()))))
                .child(label)
        };
        let design_on = !matches!(self.panel, Panel::Prototype | Panel::Code);
        let bar = div()
            .id("design-bar")
            .h(px(BAR_H))
            .px(px(5.))
            .flex()
            .items_center()
            .gap(px(2.))
            .rounded(px(12.))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_raised()))
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.tool_group(
                "design-tool-move",
                if self.hand {
                    IconName::Hand
                } else {
                    IconName::MousePointer2
                },
                if self.hand { "Hand (H)" } else { "Move (V)" },
                moving || self.hand,
                Flyout::Move,
                cx,
                |this, cx| {
                    let hand = this.hand;
                    this.pick_tool(if hand { "hand" } else { "move" }, cx)
                },
            ))
            .child(
                self.tool(
                    "design-tool-frame",
                    IconName::Frame,
                    "Frame: a new screen (F)",
                    self.new_open,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.flyout = None;
                    this.new_open = !this.new_open;
                    cx.notify();
                })),
            )
            .child(self.tool_group(
                "design-tool-shape",
                shape_icon(shape_now),
                match shape_now {
                    Shape::Ellipse => "Ellipse (O)",
                    Shape::Line => "Line (L)",
                    Shape::Arrow => "Arrow (⇧L)",
                    _ => "Rectangle (R)",
                },
                shape_on,
                Flyout::Shapes,
                cx,
                move |this, cx| {
                    let s = this.last_shape;
                    this.set_shape((this.shape != Some(s)).then_some(s), cx);
                },
            ))
            .child(
                self.tool(
                    "design-tool-pen",
                    IconName::PenTool,
                    "Pen (P)",
                    self.shape == Some(Shape::Pen),
                )
                .on_click(cx.listener(|this, _, _, cx| this.pick_tool("pen", cx))),
            )
            .child(
                self.tool("design-tool-text", IconName::Type, "Text (T)", false)
                    .on_click(cx.listener(|this, _, _, cx| this.pick_tool("text", cx))),
            )
            .child(
                self.tool(
                    "design-tool-comment",
                    comments::comment_icon(),
                    "Comment (C)",
                    self.commenting,
                )
                .on_click(cx.listener(|this, _, _, cx| this.pick_tool("comment", cx))),
            )
            .child(
                self.tool(
                    "design-tool-assets",
                    components::component_icon(),
                    "Components and assets",
                    self.assets && !self.layers_hidden,
                )
                .on_click(cx.listener(|this, _, _, cx| this.pick_tool("assets", cx))),
            )
            .child(
                self.tool("design-tool-ai", IconName::Sparkles, "Ask AI (/)", false)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.flyout = None;
                        this.focus_ask(window, cx)
                    })),
            )
            .child(sep())
            .child(
                div()
                    .p(px(2.))
                    .flex()
                    .gap(px(2.))
                    .rounded(px(8.))
                    .bg(tint(c_subtle()))
                    .child(
                        mode("design-mode-design", "Design", design_on)
                            .tooltip(tip("Design: layers and their styles"))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_mode(Panel::Design, cx)),
                            ),
                    )
                    .child(
                        mode(
                            "design-mode-proto",
                            "Prototype",
                            self.panel == Panel::Prototype,
                        )
                        .tooltip(tip("Prototype: links between screens and how they move"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.set_mode(Panel::Prototype, cx)),
                        ),
                    )
                    .child(
                        mode("design-mode-code", "Code", self.panel == Panel::Code)
                            .tooltip(tip("Code: what a developer needs, and the handoff"))
                            .on_click(cx.listener(|this, _, _, cx| this.set_mode(Panel::Code, cx))),
                    ),
            );
        let menu = match &self.flyout {
            Some(Flyout::Move) => Some(self.menu_at(
                "design-move-menu",
                vec![
                    Item::new("Move", "V", |this, _, cx| this.pick_tool("move", cx))
                        .ticked(!self.hand),
                    Item::new("Hand", "H", |this, _, cx| this.pick_tool("hand", cx))
                        .ticked(self.hand),
                ],
                180.,
                Anchor::BottomLeft,
                cx,
            )),
            Some(Flyout::Shapes) => {
                let mut items: Vec<Item> = [
                    (Shape::Rect, "Rectangle", "R"),
                    (Shape::Ellipse, "Ellipse", "O"),
                    (Shape::Line, "Line", "L"),
                    (Shape::Arrow, "Arrow", "⇧L"),
                ]
                .into_iter()
                .map(|(s, label, keys)| {
                    Item::new(label, keys, move |this, _, cx| {
                        this.last_shape = s;
                        this.set_shape(Some(s), cx);
                    })
                    .ticked(self.shape == Some(s))
                })
                .collect();
                items.push(Item::Line);
                items.push(Item::new("Picture…", "", |this, _, cx| {
                    this.image_tool(cx)
                }));
                Some(self.menu_at("design-shape-menu", items, 200., Anchor::BottomLeft, cx))
            }
            _ => None,
        };
        let left = match self.flyout {
            Some(Flyout::Shapes) => 76.,
            _ => 4.,
        };
        div()
            .absolute()
            .bottom(px(BAR_GAP))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(div().relative().child(bar).when_some(menu, |el, m| {
                el.child(div().absolute().top(px(-6.)).left(px(left)).child(m))
            }))
            .into_any_element()
    }

    fn tool(
        &self,
        id: &'static str,
        name: IconName,
        tip_text: &'static str,
        on: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .size(px(34.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(7.))
            .cursor_pointer()
            .when(on, |el| el.bg(rgb(c_blue())))
            .when(!on, |el| el.hover(|el| el.bg(tint(c_subtle()))))
            .tooltip(tip(tip_text))
            .child(icon(name, 16., if on { 0xffffff } else { c_text() }))
    }

    /// A tool with others like it behind a small arrow, as design tools group them.
    #[allow(clippy::too_many_arguments)]
    fn tool_group(
        &self,
        id: &'static str,
        name: IconName,
        tip_text: &'static str,
        on: bool,
        flyout: Flyout,
        cx: &mut Context<Self>,
        act: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let open = self.flyout.as_ref() == Some(&flyout);
        div()
            .flex()
            .items_center()
            .child(self.tool(id, name, tip_text, on).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.flyout = None;
                    act(this, cx);
                    cx.notify();
                },
            )))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-more")))
                    .w(px(14.))
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|el| el.bg(tint(c_subtle())))
                    .child(icon(
                        if open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        },
                        10.,
                        c_muted(),
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.flyout = (!open).then(|| flyout.clone());
                        this.new_open = false;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Pick a tool by name, putting the others away.
    pub(super) fn pick_tool(&mut self, tool: &str, cx: &mut Context<Self>) {
        self.flyout = None;
        match tool {
            "move" | "hand" => {
                self.set_shape(None, cx);
                self.hand = tool == "hand";
                self.commenting = false;
                self.comment_open = None;
            }
            "pen" => {
                let on = self.shape == Some(Shape::Pen);
                self.set_shape((!on).then_some(Shape::Pen), cx);
            }
            "text" => self.insert("<p>Text</p>", cx),
            "comment" => self.toggle_commenting(cx),
            "assets" => {
                self.layers_hidden = false;
                self.assets = !self.assets;
                self.history = false;
                if self.assets {
                    self.refresh_libraries();
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Design, Prototype or Code: what the right panel shows, and what the canvas draws.
    pub(super) fn set_mode(&mut self, panel: Panel, cx: &mut Context<Self>) {
        self.flyout = None;
        // Design also covers the Theme and Checks tabs: going back to it keeps them.
        if panel != Panel::Design || matches!(self.panel, Panel::Prototype | Panel::Code) {
            self.panel = panel;
        }
        cx.notify();
    }

    /// Whether links between screens are drawn on the canvas.
    pub(super) fn links_shown(&self) -> bool {
        self.panel == Panel::Prototype || self.hotspots
    }

    // ---- The right panel's head

    /// Undo and redo, Present with its choices, and Hand off.
    pub(super) fn right_head(&self, cx: &mut Context<Self>) -> AnyElement {
        let present = if self.playing.is_some() {
            text_btn("design-play", "Stop", Some(IconName::Square), false)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.playing = None;
                    cx.notify();
                }))
                .into_any_element()
        } else {
            let open = self.flyout == Some(Flyout::Present);
            div()
                .relative()
                .flex()
                .items_center()
                .child(
                    icon_btn("design-play", IconName::Play, "Present (⌥⌘↵)", false).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.flyout = None;
                            this.play(cx)
                        }),
                    ),
                )
                .child(
                    div()
                        .id("design-play-more")
                        .w(px(14.))
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(|el| el.bg(tint(c_subtle())))
                        .child(icon(IconName::ChevronDown, 10., c_muted()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.flyout = (!open).then_some(Flyout::Present);
                            cx.notify();
                        })),
                )
                .when(open, |el| {
                    let android = self.canvas.platform == "android";
                    el.child(div().absolute().top(px(30.)).right_0().child(self.menu_at(
                        "design-present-menu",
                        vec![
                                Item::new("Present from the selected screen", "", |this, _, cx| {
                                    this.play(cx)
                                }),
                                Item::new("Present from the start", "", |this, _, cx| {
                                    this.screen = None;
                                    this.element = None;
                                    this.play(cx)
                                }),
                                Item::Line,
                                Item::new("Moves like iOS", "", |this, _, cx| {
                                    this.set_platform("ios", cx)
                                })
                                .ticked(!android),
                                Item::new("Moves like Android", "", |this, _, cx| {
                                    this.set_platform("android", cx)
                                })
                                .ticked(android),
                            ],
                        240.,
                        Anchor::TopRight,
                        cx,
                    )))
                })
                .into_any_element()
        };
        let can_undo = !self.undo.is_empty() || self.ai.as_ref().is_some_and(|ai| ai.changed);
        div()
            .h(px(44.))
            .flex_none()
            .px_2()
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(divider())
            .child(
                icon_btn("design-undo", IconName::Undo2, "Undo (⌘Z)", false)
                    .when(!can_undo, |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
            )
            .child(
                icon_btn("design-redo", IconName::Redo2, "Redo (⇧⌘Z)", false)
                    .when(self.redo.is_empty(), |el| el.opacity(0.4))
                    .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
            )
            .child(div().flex_1())
            .child(present)
            .child(
                text_btn("design-handoff", "Hand off", None, true)
                    .ml_1()
                    .tooltip(tip("Hand the design to the agents that build it"))
                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(Panel::Code, cx))),
            )
            .into_any_element()
    }

    /// The zoom, as a menu that also holds what the canvas shows.
    pub(super) fn zoom_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.flyout == Some(Flyout::Zoom);
        let zoom_to = |z: f32| {
            move |this: &mut DesignCanvas, _: &mut Window, cx: &mut Context<DesignCanvas>| {
                this.zoom_step(z / this.zoom);
                cx.notify();
            }
        };
        div()
            .relative()
            .ml_auto()
            .child(
                div()
                    .id("design-zoom")
                    .h(px(28.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(rgb(c_secondary()))
                    .hover(|el| el.bg(tint(c_subtle())))
                    .child(format!("{}%", (self.zoom * 100.).round()))
                    .child(icon(IconName::ChevronDown, 10., c_muted()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.flyout = (!open).then_some(Flyout::Zoom);
                        cx.notify();
                    })),
            )
            .when(open, |el| {
                el.child(div().absolute().top(px(30.)).right_0().child(self.menu_at(
                    "design-zoom-menu",
                    vec![
                        Item::new("Zoom in", "⌘+", |this, _, cx| {
                            this.zoom_step(1.25);
                            cx.notify()
                        }),
                        Item::new("Zoom out", "⌘−", |this, _, cx| {
                            this.zoom_step(1. / 1.25);
                            cx.notify()
                        }),
                        Item::new("Zoom to fit", "⇧1", |this, _, cx| {
                            this.fit();
                            cx.notify()
                        }),
                        Item::new("Zoom to selection", "⇧2", |this, _, cx| {
                            this.zoom_to_selection();
                            cx.notify()
                        }),
                        Item::new("Zoom to 50%", "", zoom_to(0.5)),
                        Item::new("Zoom to 100%", "⇧0", zoom_to(1.)),
                        Item::new("Zoom to 200%", "", zoom_to(2.)),
                        Item::Line,
                        Item::new("Prototype links in Design", "", |this, _, cx| {
                            this.hotspots = !this.hotspots;
                            cx.notify()
                        })
                        .ticked(self.hotspots),
                        Item::new("Layout grids", "⌃G", |this, _, cx| {
                            this.show_grids = !this.show_grids;
                            cx.notify()
                        })
                        .ticked(self.show_grids),
                        Item::new("Arrange screens along the flow", "", |this, _, cx| {
                            this.arrange(cx)
                        }),
                    ],
                    250.,
                    Anchor::TopRight,
                    cx,
                )))
            })
            .into_any_element()
    }

    // ---- The selection's menu

    /// What can be done to the selection, as design tools list it.
    pub(super) fn selection_items(&self) -> Vec<Item> {
        let Some(el) = self.element.as_ref() else {
            // A screen.
            let mut items = vec![
                Item::new("Paste here", "⌘V", |this, _, cx| this.paste(cx)),
                Item::new("Duplicate", "⌘D", |this, _, cx| this.duplicate_screen(cx)),
                Item::new("Present from here", "", |this, _, cx| this.play(cx)),
                Item::new("Zoom to it", "⇧2", |this, _, cx| {
                    this.zoom_to_selection();
                    cx.notify()
                }),
            ];
            let here = self.page_now();
            let others: Vec<String> = self
                .canvas
                .page_list()
                .into_iter()
                .filter(|p| Some(p) != here.as_ref())
                .collect();
            if !others.is_empty() {
                items.push(Item::Line);
                for p in others {
                    let label = format!("Move to {p}");
                    items.push(Item::new(label, "", move |this, _, cx| {
                        this.move_screen_to(p.clone(), cx)
                    }));
                }
            }
            items.push(Item::new("Move to a new page", "", |this, _, cx| {
                let p = this.canvas.new_page_name();
                if this.canvas.pages.is_empty() {
                    this.canvas.pages = this.canvas.page_list();
                }
                this.canvas.pages.push(p.clone());
                this.move_screen_to(p, cx)
            }));
            items.push(Item::Line);
            items.push(Item::new("Delete", "⌫", |this, _, cx| this.delete_screen(cx)).danger());
            return items;
        };
        let multi = !self.also.is_empty();
        let mut items = vec![
            Item::new("Copy", "⌘C", |this, _, cx| this.copy(false, cx)),
            Item::new("Cut", "⌘X", |this, _, cx| this.copy(true, cx)),
            Item::new("Paste", "⌘V", |this, _, cx| this.paste(cx)),
            Item::new("Duplicate", "⌘D", |this, _, cx| {
                this.duplicate_element(cx)
            }),
            Item::Line,
            Item::new("Select parent", "Esc", |this, window, cx| {
                this.escape(window, cx)
            }),
            Item::new("Bring forward", "⌘]", |this, _, cx| this.nudge(1, cx)),
            Item::new("Send backward", "⌘[", |this, _, cx| this.nudge(-1, cx)),
            Item::Line,
            Item::new("Group selection", "⌘G", |this, _, cx| {
                this.group(false, cx)
            }),
            Item::new("Add auto layout", "⇧A", |this, _, cx| {
                this.auto_layout(cx)
            }),
        ];
        if multi || el.can_hold() {
            items.push(Item::new("Ungroup", "⇧⌘G", |this, _, cx| {
                this.ungroup(cx)
            }));
        }
        if !multi && el.component.is_none() {
            items.push(Item::new("Create component", "⌥⌘K", |this, _, cx| {
                this.create_component(cx)
            }));
        }
        items.extend([
            Item::Line,
            Item::new(
                if el.hidden { "Show" } else { "Hide" },
                "⇧⌘H",
                |this, _, cx| this.op("hide", &[], After::Same, cx),
            ),
            Item::new(
                if el.locked { "Unlock" } else { "Lock" },
                "⇧⌘L",
                |this, _, cx| this.lock_selected(cx),
            ),
            Item::new("Zoom to selection", "⇧2", |this, _, cx| {
                this.zoom_to_selection();
                cx.notify()
            }),
            Item::new("Ask AI about it", "/", |this, window, cx| {
                this.focus_ask(window, cx)
            }),
            Item::Line,
            Item::new("Delete", "⌫", |this, _, cx| this.delete_element(cx)).danger(),
        ]);
        items
    }

    /// The selection's menu where the canvas was right-clicked.
    pub(super) fn selection_menu_on_canvas(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(Flyout::Selection(Some(at))) = &self.flyout else {
            return None;
        };
        Some(
            div()
                .absolute()
                .left(px(at.0))
                .top(px(at.1))
                .child(self.menu_at(
                    "design-sel-menu",
                    self.selection_items(),
                    230.,
                    Anchor::TopLeft,
                    cx,
                ))
                .into_any_element(),
        )
    }

    // ---- Help

    pub(super) fn help_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.flyout == Some(Flyout::Help);
        let keys: [(&str, &str); 16] = [
            ("Move · Hand", "V · H"),
            ("Frame (new screen)", "F"),
            ("Rectangle · Ellipse · Line", "R · O · L"),
            ("Pen · Text · Comment", "P · T · C"),
            ("Ask AI", "/"),
            ("Pan", "Space-drag"),
            ("Zoom to fit · selection · 100%", "⇧1 · ⇧2 · ⇧0"),
            ("Measure", "Hold ⌥"),
            ("Group · Ungroup", "⌘G · ⇧⌘G"),
            ("Auto layout", "⇧A"),
            ("Create component", "⌥⌘K"),
            ("Duplicate", "⌘D"),
            ("Bring forward · Send backward", "⌘] · ⌘["),
            ("Hide · Lock", "⇧⌘H · ⇧⌘L"),
            ("Select parent", "Esc"),
            ("Present", "⌥⌘↵"),
        ];
        let mut list = div().v_flex().gap(px(2.));
        for (what, k) in keys {
            list = list.child(
                div()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .text_size(px(12.))
                    .child(div().text_color(rgb(c_text())).child(what))
                    .child(div().text_color(rgb(c_muted())).child(k)),
            );
        }
        div()
            .absolute()
            .right(px(14.))
            .bottom(px(BAR_GAP + 6.))
            .child(
                div()
                    .id("design-help")
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(float_edge())
                    .bg(float(c_raised()))
                    .shadow_md()
                    .cursor_pointer()
                    .occlude()
                    .tooltip(tip("Keyboard shortcuts"))
                    .child(icon(IconName::CircleQuestionMark, 15., c_secondary()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.flyout = (!open).then_some(Flyout::Help);
                        cx.notify();
                    })),
            )
            .when(open, |el| {
                el.child(
                    div().absolute().bottom(px(40.)).right_0().child(
                        deferred(
                            anchored()
                                .anchor(Anchor::BottomRight)
                                .snap_to_window_with_margin(px(8.))
                                .child(
                                    div()
                                        .id("design-help-card")
                                        .w(px(330.))
                                        .p_3()
                                        .v_flex()
                                        .gap_2()
                                        .rounded_lg()
                                        .border_1()
                                        .border_color(float_edge())
                                        .bg(float(c_raised()))
                                        .shadow_lg()
                                        .occlude()
                                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                            this.flyout = None;
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(c_text()))
                                                .child("Keyboard shortcuts"),
                                        )
                                        .child(list),
                                ),
                        )
                        .with_priority(3),
                    ),
                )
            })
            .into_any_element()
    }

    // ---- Menus

    /// A menu floating over everything, anchored by `corner` where it's placed.
    pub(super) fn menu_at(
        &self,
        id: &'static str,
        items: Vec<Item>,
        w: f32,
        corner: Anchor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut list = div()
            .id(id)
            .w(px(w))
            .max_h(px(520.))
            .overflow_y_scroll()
            .p_1()
            .v_flex()
            .rounded_lg()
            .border_1()
            .border_color(float_edge())
            .bg(float(c_raised()))
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.flyout = None;
                cx.notify();
            }));
        for (i, item) in items.into_iter().enumerate() {
            list = list.child(match item {
                Item::Line => div().my(px(4.)).h(px(1.)).bg(divider()).into_any_element(),
                Item::Do {
                    label,
                    keys,
                    on,
                    danger,
                    act,
                } => div()
                    .id((id, i))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .text_color(rgb(if danger { c_red() } else { c_text() }))
                    .hover(|el| el.bg(tint(c_active())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.flyout = None;
                        act(this, window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(12.))
                            .flex_none()
                            .when(on, |el| el.child(icon(IconName::Check, 12., c_text()))),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(rgb(c_muted()))
                            .child(keys),
                    )
                    .into_any_element(),
            });
        }
        deferred(
            anchored()
                .anchor(corner)
                .snap_to_window_with_margin(px(8.))
                .child(list),
        )
        .with_priority(3)
        .into_any_element()
    }
}
