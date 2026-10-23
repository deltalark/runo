//! Asks typed while AI is still working wait in line, each with the screen and layers it
//! was asked about, and start one after another: when the change before ends, it's kept
//! (it stays in History) and the next ask goes to the agent.

use super::*;

/// An ask waiting for the one before it.
pub(super) struct Queued {
    pub task: String,
    /// Shortened, for its row.
    pub label: String,
    screen: Option<String>,
    element: Option<Element>,
    also: Vec<Element>,
    /// It was about the selection.
    scoping: bool,
}

impl DesignCanvas {
    /// Put an ask in line behind the change AI is making, with what's selected now.
    pub(super) fn queue_ask(&mut self, task: String, cx: &mut Context<Self>) {
        self.queued.push(Queued {
            label: summary(&task),
            task,
            screen: self.screen.clone(),
            element: self.element.clone(),
            also: self.also.clone(),
            scoping: std::mem::take(&mut self.scoping),
        });
        self.ask_new = None;
        self.say(match self.queued.len() {
            1 => "Queued. It starts when AI finishes this change.".to_string(),
            n => format!("Queued, {n} in line."),
        });
        cx.notify();
    }

    /// Start the next ask in line once AI is done with the one before: that change is kept.
    pub(super) fn run_queued(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.queued.is_empty() || self.ai.as_ref().is_some_and(|ai| ai.busy()) {
            return;
        }
        if self.ai.is_some() {
            self.keep_ai(cx);
        }
        let q = self.queued.remove(0);
        self.select_screen(q.screen);
        self.element = q.element;
        self.also = q.also;
        self.scoping = q.scoping;
        if !self.ask(q.task, window, cx) {
            self.queued.clear();
        }
        self.load_layers(cx);
    }

    /// The asks in line, each with a way to take it out.
    pub(super) fn queued_rows(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.queued.is_empty() {
            return None;
        }
        let mut col = div().v_flex().gap(px(2.)).pt_1();
        for (i, q) in self.queued.iter().enumerate() {
            col = col.child(
                div()
                    .h(px(24.))
                    .px_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(rgb(c_secondary()))
                    .child(icon(IconName::CornerDownRight, 12., c_muted()))
                    .child(
                        div()
                            .flex_none()
                            .text_color(rgb(c_muted()))
                            .child(if i == 0 { "Next" } else { "Then" }),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(q.label.clone()))
                    .child(
                        icon_btn(
                            ("design-queued-drop", i),
                            IconName::X,
                            "Take out of line",
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if i < this.queued.len() {
                                this.queued.remove(i);
                            }
                            cx.notify();
                        })),
                    ),
            );
        }
        Some(col)
    }

    /// While AI works, a line to type the next ask into.
    pub(super) fn queue_input(&self) -> Div {
        div()
            .mt_1()
            .h(px(30.))
            .px_1()
            .flex()
            .items_center()
            .gap_1()
            .border_t_1()
            .border_color(divider())
            .child(icon(IconName::ListPlus, 13., c_muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.f_ask).appearance(false).h(px(26.))),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child("Enter to queue"),
            )
    }
}
