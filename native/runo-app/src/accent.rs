//! The accent corner's hover: the bracket on an open tab or a selected row reaches a
//! little further along its edges and brightens while the pointer is over it, easing in
//! and out. With Animations off it snaps.

use super::*;
use std::cell::RefCell;
use std::time::Instant;

thread_local! {
    /// Whether the pointer was over each bracket's tab or row when it last painted, and
    /// since when, keyed by the bracket.
    static HOVERS: RefCell<HashMap<SharedString, (bool, Instant)>> =
        RefCell::new(HashMap::new());
}

/// How long a bracket takes to reach out or settle back when hovered.
const HOVER_MS: f32 = 180.;

/// How far the bracket `key` is into its hovered look, from 0 to 1, with the pointer
/// over `area` (its tab or row) or not. Eases both ways, and redraws the view when the
/// pointer crosses into or out of `area`, so it needs nothing from the element it marks.
/// Call it while painting the bracket.
pub fn hover(key: &SharedString, area: Bounds<Pixels>, window: &mut Window) -> f32 {
    let over = area.contains(&window.mouse_position());
    let view = window.current_view();
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && area.contains(&e.position) != over {
            cx.notify(view);
        }
    });
    let now = Instant::now();
    let since = HOVERS.with_borrow_mut(|h| {
        // First seen: already settled, so nothing moves on arrival.
        let seen = h
            .entry(key.clone())
            .or_insert((over, now - Duration::from_secs(1)));
        if seen.0 != over {
            *seen = (over, now);
        }
        seen.1.elapsed().as_secs_f32() * 1000.
    });
    let t = if anim_on() {
        (since / HOVER_MS).min(1.)
    } else {
        1.
    };
    if t < 1. {
        window.request_animation_frame();
    }
    let t = 1. - (1. - t).powi(3);
    if over {
        t
    } else {
        1. - t
    }
}

/// The bracket's colour, `hover` of the way to its hovered look: the accent, a step
/// brighter while hovered.
pub fn color(hover: f32) -> Hsla {
    rgb(mix(
        c_blue(),
        0xffffff,
        hover * if is_dark() { 0.25 } else { 0.1 },
    ))
    .into()
}
