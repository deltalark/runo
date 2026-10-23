//! Dragging a field's mark to change its number, the way design tools do: the screen
//! follows the pointer, and the whole drag undoes as one step.

use super::*;
use crate::designdraw::fields;

/// A field being dragged.
pub(super) struct Scrub {
    id: &'static str,
    /// Where the pointer went down, across the window.
    x0: f32,
    from: f32,
    /// The latest value, and the one last sent to the screen.
    last: String,
    sent: String,
    /// An edit is on its way, so the next waits for it.
    busy: bool,
    /// The button is up; the drag ends once its last edit lands.
    done: bool,
    /// The drag's first edit has made its undo step.
    stepped: bool,
}

impl DesignCanvas {
    /// Start dragging field `id`'s mark, when it holds a number to move.
    pub(super) fn start_scrub(&mut self, id: &'static str, e: &MouseDownEvent) {
        let (Some(f), Some(el)) = (fields::field(id), self.element.as_ref()) else {
            return;
        };
        let Some(from) = fields::scrub_from(f, el) else {
            return;
        };
        let shown = fields::display(f, el);
        self.scrub = Some(Scrub {
            id,
            x0: f32::from(e.position.x),
            from,
            last: shown.clone(),
            sent: shown,
            busy: false,
            done: false,
            stepped: false,
        });
    }

    /// Follow the pointer while a mark is dragged: a step a pixel, ten with Shift.
    pub(super) fn scrub_move(
        &mut self,
        e: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(s) = self.scrub.as_mut().filter(|s| !s.done) else {
            return false;
        };
        let Some(f) = fields::field(s.id) else {
            return false;
        };
        let step = if e.modifiers.shift { 10. } else { 1. };
        let typed = fields::scrubbed(f, s.from, (f32::from(e.position.x) - s.x0) * step);
        if typed == s.last {
            return true;
        }
        s.last = typed.clone();
        let id = s.id;
        if let Some(state) = self.field_state(id).cloned() {
            state.update(cx, |st, cx| st.set_value(typed, window, cx));
        }
        self.send_scrub(cx);
        true
    }

    /// Let go of a dragged mark.
    pub(super) fn end_scrub(&mut self) -> bool {
        let Some(s) = self.scrub.as_mut() else {
            return false;
        };
        s.done = true;
        if !s.busy {
            self.scrub = None;
        }
        true
    }

    /// Whether an edit landing now belongs to a drag that already has its undo step.
    pub(super) fn scrub_merges(&mut self) -> bool {
        self.scrub
            .as_mut()
            .is_some_and(|s| std::mem::replace(&mut s.stepped, true))
    }

    /// An edit has landed, or failed: send the value the pointer has moved on to since.
    pub(super) fn scrub_landed(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.scrub.as_mut() else {
            return;
        };
        s.busy = false;
        if s.last != s.sent {
            self.send_scrub(cx);
        } else if s.done {
            self.scrub = None;
        }
    }

    fn send_scrub(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.scrub.as_mut().filter(|s| !s.busy) else {
            return;
        };
        let (Some(f), Some(el)) = (fields::field(s.id), self.element.as_ref()) else {
            return;
        };
        let Some(value) = fields::parse(f, &s.last).filter(|_| self.screen.is_some()) else {
            return;
        };
        s.busy = true;
        s.sent = s.last.clone();
        self.field_vals.insert(s.id, s.last.clone());
        let mut map = serde_json::Map::new();
        for (k, v) in fields::styles(f, &value, el) {
            map.insert(k.into(), json!(v));
        }
        self.op("styles", &[serde_json::Value::Object(map)], After::Same, cx);
    }

    /// Whether a field's mark is being dragged, for the cursor.
    pub(super) fn scrubbing(&self) -> bool {
        self.scrub.as_ref().is_some_and(|s| !s.done)
    }
}
