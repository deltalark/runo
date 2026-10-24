//! The "Working · 6s" row shown while an agent runs: a small ring with a glowing arc
//! sweeping round it and a core breathing in the middle, what the agent is doing ("Thinking",
//! "Reading", "Running Bash") with a soft light passing along the word, and the time so
//! far. The ring is one canvas and the word one text, and both move on the window's
//! shared clock (see `motion`). When the turn ends the arc closes into a ring, a check
//! draws in, and the row fades out. Holding still, the arc rests part way round.

use super::*;
use crate::assistant::{glow, halo};
use crate::harness::event::{StepState, ToolKind, ToolStep};
use std::sync::Arc;
use std::time::Instant;

/// Side of the square the ring is drawn in.
const BOX: f32 = 16.;
/// Radius of the ring, to the middle of its line.
const RADIUS: f32 = 5.75;
const LINE: f32 = 1.6;
/// One turn of the arc, and one breath of the arc's length.
const SPIN: Duration = Duration::from_millis(1300);
const BREATHE: Duration = Duration::from_millis(2600);
/// One pass of the light along the word, with a rest after it.
const SHIMMER: Duration = Duration::from_millis(2400);
/// How long a finished turn's row stays: it settles into a check, then fades out.
pub(crate) const FINISH: Duration = Duration::from_millis(2400);
const SETTLE: f32 = 0.65;
const FADE: f32 = 0.45;

/// The indicator's colour: the accent, lifted in dark mode where it sits too close to the
/// canvas.
fn accent() -> u32 {
    if is_dark() {
        mix(c_blue(), 0xffffff, 0.6)
    } else {
        c_blue()
    }
}

/// What the agent is doing, from its turn so far: the tool it is running, else writing
/// its reply, else thinking.
pub(crate) fn doing(messages: &[Message]) -> SharedString {
    let turn = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .map_or(messages, |i| &messages[i + 1..]);
    let Some(reply) = turn.iter().rev().find(|m| m.role == Role::Agent) else {
        return "Thinking".into();
    };
    if let Some(step) = reply
        .steps
        .iter()
        .rev()
        .find(|s| s.state == StepState::Running)
    {
        return tool_label(step);
    }
    if reply.text.trim().is_empty() {
        "Thinking".into()
    } else {
        "Writing".into()
    }
}

/// A word or two for a running tool call.
fn tool_label(step: &ToolStep) -> SharedString {
    match step.kind {
        ToolKind::Read => "Reading".into(),
        ToolKind::Edit | ToolKind::Write | ToolKind::Delete => "Editing".into(),
        // "Running Bash" when the tool has a short name of its own, else plainly.
        ToolKind::Shell => {
            let tool = step.tool.as_str();
            if (2..=12).contains(&tool.len())
                && tool.starts_with(|c: char| c.is_ascii_uppercase())
                && tool.chars().all(|c| c.is_ascii_alphanumeric())
            {
                format!("Running {tool}").into()
            } else {
                "Running a command".into()
            }
        }
        ToolKind::Search | ToolKind::List => "Searching".into(),
        ToolKind::WebSearch => "Searching the web".into(),
        ToolKind::Fetch => "Browsing".into(),
        ToolKind::Subagent => "Delegating".into(),
        ToolKind::Todo | ToolKind::Plan => "Planning".into(),
        ToolKind::Think => "Thinking".into(),
        ToolKind::Question => "Asking".into(),
        ToolKind::Mcp | ToolKind::Other => "Working".into(),
    }
}

/// How the ring looks in one frame.
#[derive(Clone, Copy)]
struct Ring {
    /// Where the arc's head is, in turns clockwise from the top.
    head: f32,
    /// How far round the arc reaches behind its head, in turns.
    sweep: f32,
    /// The core's breath, 0 to 1.
    core: f32,
    /// How far a finished turn has settled, 0 to 1: the arc closes, then the check draws.
    done: f32,
}

impl Ring {
    /// At rest: the arc part way round with the core lit.
    const STILL: Ring = Ring {
        head: 0.12,
        sweep: 0.42,
        core: 0.7,
        done: 0.,
    };

    fn now() -> Ring {
        let spin = motion::phase(SPIN);
        let breathe = 0.5 - 0.5 * (motion::phase(BREATHE) * std::f32::consts::TAU).cos();
        Ring {
            // A touch of easing on top of a steady turn, so the head surges a little as
            // the arc stretches.
            head: spin + 0.04 * (breathe * std::f32::consts::PI).sin(),
            sweep: 0.24 + 0.36 * breathe,
            core: 0.5 - 0.5 * (motion::phase(BREATHE / 2) * std::f32::consts::TAU).cos(),
            done: 0.,
        }
    }
}

/// The row for a running turn: the ring, `label` and `detail`, such as the time so far.
pub(crate) fn indicator(label: impl Into<SharedString>, detail: Option<String>) -> AnyElement {
    let moving = motion::moving();
    if detail.is_some() {
        motion::ticking();
    }
    let ring = if moving { Ring::now() } else { Ring::STILL };
    let label = label.into();
    let shimmer = moving.then(|| motion::phase(SHIMMER));
    row(ring, shimmer_text(label, shimmer), detail).into_any_element()
}

/// The row for a turn that ended `at`, which `took` long: the ring closes into a check,
/// then the row fades out. `None` once it has gone.
pub(crate) fn finished(at: Instant, took: Duration) -> Option<AnyElement> {
    let age = at.elapsed();
    if age >= FINISH {
        return None;
    }
    let age = age.as_secs_f32();
    let fading = age > FINISH.as_secs_f32() - FADE;
    // Smooth frames while it settles and fades, a slower tick while it rests, so it
    // goes on time.
    let moving = (age < SETTLE || fading) && motion::moving();
    motion::ticking();
    let (done, opacity) = if moving {
        let settle = ease_out_cubic((age / SETTLE).min(1.));
        let fade = ((FINISH.as_secs_f32() - age) / FADE).clamp(0., 1.);
        (settle, fade)
    } else {
        (1., 1.)
    };
    let mut ring = if moving { Ring::now() } else { Ring::STILL };
    ring.done = done;
    let label = StyledText::new("Worked").with_highlights([(
        0.."Worked".len(),
        HighlightStyle {
            color: Some(rgb(c_secondary()).into()),
            ..Default::default()
        },
    )]);
    Some(
        row(ring, label, Some(elapsed(took)))
            .opacity(opacity)
            .into_any_element(),
    )
}

fn row(ring: Ring, label: StyledText, detail: Option<String>) -> Div {
    let color = accent();
    let green = if is_dark() {
        mix(c_green(), 0xffffff, 0.75)
    } else {
        c_green()
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            canvas(
                |_, _, _| (),
                move |b, _, window, _| paint_ring(b, ring, color, green, window),
            )
            .flex_none()
            .size(px(BOX)),
        )
        .child(label)
        .when_some(detail, |row, d| {
            row.child(
                div()
                    .text_color(rgb(c_muted()))
                    // Figures of one width, so the time ticks without the row shifting.
                    .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                    .child(format!("· {d}")),
            )
        })
}

/// `label` in the secondary text colour with, at `shimmer` of the way through its loop,
/// a soft band of the accent passing along it.
fn shimmer_text(label: SharedString, shimmer: Option<f32>) -> StyledText {
    let (base, lit) = (c_secondary(), accent());
    let n = label.chars().count();
    let highlights: Vec<_> = label
        .char_indices()
        .enumerate()
        .map(|(i, (at, ch))| {
            let near = shimmer.map_or(0., |p| band(i, n, p));
            (
                at..at + ch.len_utf8(),
                HighlightStyle {
                    color: Some(rgb(mix(lit, base, 0.9 * near)).into()),
                    ..Default::default()
                },
            )
        })
        .collect();
    StyledText::new(label).with_highlights(highlights)
}

/// How lit letter `i` of `n` is, 0 to 1, at `phase` of the shimmer's loop. The band
/// crosses in the first three quarters of the loop, easing in and out, then rests out of
/// sight before the next pass.
fn band(i: usize, n: usize, phase: f32) -> f32 {
    let t = (phase / 0.75).min(1.);
    let t = t * t * (3. - 2. * t);
    let pos = -3. + (n as f32 + 6.) * t;
    let d = (i as f32 - pos) / 1.8;
    (-d * d).exp()
}

fn paint_ring(b: Bounds<Pixels>, ring: Ring, accent: u32, green: u32, window: &mut Window) {
    let c = b.center();
    let at = |turns: f32, r: f32| {
        let a = turns * std::f32::consts::TAU;
        point(c.x + px(r * a.sin()), c.y - px(r * a.cos()))
    };
    // Settling: the arc stretches to a full ring and turns green, then the check draws
    // in where the core was.
    let close = (ring.done / 0.6).min(1.);
    let check = ((ring.done - 0.45) / 0.55).clamp(0., 1.);
    let color = mix(green, accent, close);
    let sweep = ring.sweep + (1. - ring.sweep) * close;
    let dark = is_dark();

    // The track, faint, all the way round.
    let mut track = PathBuilder::stroke(px(LINE));
    for i in 0..=48 {
        let p = at(i as f32 / 48., RADIUS);
        if i == 0 {
            track.move_to(p);
        } else {
            track.line_to(p);
        }
    }
    if let Ok(path) = track.build() {
        let a = if dark { 0.2 } else { 0.16 };
        window.paint_path(path, glow(color, a * (1. - close)));
    }

    // The arc: short lengths, each a little brighter towards the head, so it fades out
    // into the track behind.
    const PIECES: usize = 20;
    let tail = ring.head - sweep;
    for i in 0..PIECES {
        let (from, to) = (i as f32 / PIECES as f32, (i + 1) as f32 / PIECES as f32);
        let mut f = PathBuilder::stroke(px(LINE));
        for k in 0..=3 {
            let t = from + (to - from) * k as f32 / 3.;
            let p = at(tail + sweep * t, RADIUS);
            if k == 0 {
                f.move_to(p);
            } else {
                f.line_to(p);
            }
        }
        if let Ok(path) = f.build() {
            let a = to.powf(1.5) + (1. - to.powf(1.5)) * close;
            window.paint_path(path, glow(color, a));
        }
    }

    // The head: a bright point with a soft bloom behind it, gone once the ring closes.
    let head = at(ring.head, RADIUS);
    let lit = 1. - close;
    if lit > 0. {
        let d = px(3.2);
        let bounds = Bounds::new(point(head.x - d / 2., head.y - d / 2.), size(d, d));
        window.paint_drop_shadows(
            bounds,
            (d / 2.).into(),
            &[halo(
                color,
                (if dark { 0.75 } else { 0.55 }) * lit,
                5.,
                0.5,
                0.,
            )],
        );
        window.paint_quad(fill(bounds, glow(color, lit)).corner_radii(d / 2.));
    }

    // The core, breathing, with a halo that swells with it; it gives way to the check.
    let fade = 1. - check;
    if fade > 0. {
        let r = 1.35 + 0.45 * ring.core;
        let d = px(r * 2.);
        let bounds = Bounds::new(point(c.x - d / 2., c.y - d / 2.), size(d, d));
        window.paint_drop_shadows(
            bounds,
            (d / 2.).into(),
            &[halo(color, (0.15 + 0.3 * ring.core) * fade, 4., 0.5, 0.)],
        );
        window.paint_quad(
            fill(bounds, glow(color, (0.55 + 0.45 * ring.core) * fade)).corner_radii(d / 2.),
        );
    }

    // The check, drawn along its length.
    if check > 0. {
        let pts: [(f32, f32); 3] = [(-2.5, 0.1), (-0.8, 1.9), (2.6, -1.9)];
        let lens = [
            (pts[1].0 - pts[0].0).hypot(pts[1].1 - pts[0].1),
            (pts[2].0 - pts[1].0).hypot(pts[2].1 - pts[1].1),
        ];
        let mut left = check * (lens[0] + lens[1]);
        let p = |x: f32, y: f32| point(c.x + px(x), c.y + px(y));
        let mut f = PathBuilder::stroke(px(1.5));
        f.move_to(p(pts[0].0, pts[0].1));
        for (i, len) in lens.iter().enumerate() {
            let t = (left / len).min(1.);
            let (a, z) = (pts[i], pts[i + 1]);
            f.line_to(p(a.0 + (z.0 - a.0) * t, a.1 + (z.1 - a.1) * t));
            left -= len;
            if left <= 0. {
                break;
            }
        }
        if let Ok(path) = f.build() {
            window.paint_path(path, glow(color, check));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn agent(text: &str, steps: Vec<ToolStep>) -> Message {
        let mut m = Message::new(Role::Agent, text);
        m.steps = steps;
        m
    }

    fn step(kind: ToolKind, tool: &str, state: StepState) -> ToolStep {
        ToolStep {
            kind,
            tool: tool.into(),
            label: String::new(),
            state,
            ..Default::default()
        }
    }

    #[test]
    fn says_what_the_agent_is_doing() {
        let ask = Message::new(Role::User, "Fix it");
        assert_eq!(&*doing(std::slice::from_ref(&ask)), "Thinking");
        let reading = agent("", vec![step(ToolKind::Read, "Read", StepState::Running)]);
        assert_eq!(&*doing(&[ask.clone(), reading]), "Reading");
        let bash = agent("", vec![step(ToolKind::Shell, "Bash", StepState::Running)]);
        assert_eq!(&*doing(&[ask.clone(), bash]), "Running Bash");
        let codex = agent(
            "",
            vec![step(ToolKind::Shell, "exec_command", StepState::Running)],
        );
        assert_eq!(&*doing(&[ask.clone(), codex]), "Running a command");
        let writing = agent(
            "Found it.",
            vec![step(ToolKind::Read, "Read", StepState::Done)],
        );
        assert_eq!(&*doing(&[ask.clone(), writing]), "Writing");
        // A reply from an earlier turn doesn't count.
        let old = agent("Done.", vec![]);
        assert_eq!(&*doing(&[old, ask]), "Thinking");
    }

    #[test]
    fn the_shimmer_band_crosses_then_rests() {
        // Before it starts and while it rests, no letter is lit much.
        assert!((0..7).all(|i| band(i, 7, 0.) < 0.1));
        assert!((0..7).all(|i| band(i, 7, 0.9) < 0.1));
        // Half way across, the middle letter is lit and the ends aren't.
        assert!(band(3, 7, 0.375) > 0.95);
        assert!(band(0, 7, 0.375) < 0.1);
    }
}
