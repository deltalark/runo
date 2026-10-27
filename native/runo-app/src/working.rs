//! The "Working · 6s" row shown while an agent runs: a small ring with a glowing arc
//! sweeping round it and a core breathing in the middle, what the agent is doing ("Thinking",
//! "Reading", "Running Bash") with a soft light passing along the word, and the time so
//! far. The ring is one canvas and the word one text, and both move on the window's
//! shared clock (see `motion`): where they are is worked out from the time of the frame
//! alone, so drawing the row again, for a new word or a new second, carries on from where
//! it was rather than starting over, and a late frame only skips ahead. When the turn ends the arc closes into a ring, a check
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

/// The label of the step the agent is running now, such as "Ran cargo test", from its
/// turn so far.
pub(crate) fn running_step(messages: &[Message]) -> Option<String> {
    let turn = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .map_or(messages, |i| &messages[i + 1..]);
    turn.iter()
        .rev()
        .find(|m| m.role == Role::Agent)?
        .steps
        .iter()
        .rev()
        .find(|s| s.state == StepState::Running)
        .map(|s| s.label.clone())
}

/// A subagent the agent started this turn, for the list under the Working line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Helper {
    /// What it was asked to do, such as "Find the parser".
    pub what: String,
    pub state: StepState,
    /// How long it has run, or ran.
    pub took: Duration,
    /// The latest step it took, while it runs and the stream carries its steps.
    pub latest: String,
}

/// The subagents the agent started this turn, oldest first.
pub(crate) fn helpers(messages: &[Message]) -> Vec<Helper> {
    let turn = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .map_or(messages, |i| &messages[i + 1..]);
    let now = motion::frame().unwrap_or_else(Instant::now);
    turn.iter()
        .filter(|m| m.role == Role::Agent)
        .flat_map(|m| &m.steps)
        .filter(|s| s.kind == ToolKind::Subagent)
        .map(|s| Helper {
            what: s
                .label
                .strip_prefix("Ran a subagent: ")
                .unwrap_or(&s.label)
                .to_string(),
            state: s.state,
            took: match (s.state, s.started) {
                (StepState::Running, Some(at)) => now.saturating_duration_since(at),
                _ => Duration::from_millis(s.ms),
            },
            latest: if s.state == StepState::Running {
                s.latest.clone()
            } else {
                String::new()
            },
        })
        .collect()
}

/// "2 helper agents" while any of `helpers` run, for the Working line.
pub(crate) fn helpers_label(helpers: &[Helper]) -> Option<String> {
    match helpers
        .iter()
        .filter(|h| h.state == StepState::Running)
        .count()
    {
        0 => None,
        1 => Some("1 helper agent".into()),
        n => Some(format!("{n} helper agents")),
    }
}

/// One line for a helper: what it does, how it stands and for how long, and its latest step.
pub(crate) fn helper_line(h: &Helper) -> String {
    let state = match h.state {
        StepState::Running => "running",
        StepState::Done => "done",
        StepState::Failed => "failed",
    };
    let mut line = format!("{} · {state}", h.what);
    if !h.took.is_zero() {
        line.push_str(&format!(" {}", elapsed(h.took)));
    }
    if !h.latest.is_empty() {
        line.push_str(&format!(" · {}", h.latest));
    }
    line
}

/// The helpers, one row each, shown under the Working line when it's opened.
fn helper_list(helpers: &[Helper]) -> Div {
    let mut list = div()
        .v_flex()
        .ml(px(BOX / 2.))
        .pl_3()
        .border_l_1()
        .border_color(divider());
    for h in helpers {
        let mark = match h.state {
            StepState::Running => div()
                .size(px(12.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .child(div().size(px(6.)).rounded_full().bg(rgb(accent())))
                .into_any_element(),
            StepState::Done => icon(IconName::Check, 12., c_muted()).into_any_element(),
            StepState::Failed => icon(IconName::X, 12., c_red()).into_any_element(),
        };
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(22.))
                .text_color(rgb(c_secondary()))
                .child(mark)
                .child(div().min_w_0().truncate().child(helper_line(h))),
        );
    }
    list
}

impl RunoApp {
    /// The Working line of session `id`'s chat or peek: what the agent is doing and for how
    /// long and, while it runs subagents, how many, which a click lists.
    pub(crate) fn working_line(
        &self,
        id: u64,
        messages: &[Message],
        cx: &mut Context<Self>,
    ) -> Div {
        let started = self.turns.get(&id).map(|t| t.started);
        let helpers = helpers(messages);
        let label = helpers_label(&helpers);
        let open = !helpers.is_empty() && self.open_helpers.contains(&id);
        let line = running(doing(messages), label.clone(), started);
        let note = self.working_note(id, cx);
        if helpers.is_empty() {
            return div().v_flex().gap_1().child(line).children(note);
        }
        div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .id(SharedString::from(format!("helpers-{id}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.open_helpers.remove(&id) {
                            this.open_helpers.insert(id);
                        }
                        cx.notify();
                    }))
                    .child(line)
                    .child(icon(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        12.,
                        c_muted(),
                    )),
            )
            .when(open, |el| el.child(helper_list(&helpers)))
            .children(note)
    }

    /// Under the Working line: the permission the turn waits on, with its answers, even
    /// when its card was hidden; or, when the turn has printed nothing for too long, how
    /// long and on what, with Stop and Retry.
    fn working_note(&self, id: u64, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = |text: String| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(rgb(c_amber()))
                .child(icon(IconName::Clock, 12., c_amber()))
                .child(div().min_w_0().truncate().child(text))
        };
        if let Some(request) = self.approval_for(id) {
            let what = crate::approvals::describe(&request.tool, &request.input);
            return Some(
                div()
                    .v_flex()
                    .gap_1()
                    .child(row(format!("Waiting for your permission to {what}")))
                    .child(self.approval_actions("wk", 0, request, cx))
                    .into_any_element(),
            );
        }
        let st = self.stall(id)?;
        let stopping = self.turns.get(&id).is_some_and(|t| t.stopping);
        let text = crate::stall::working_text(st.quiet, st.step.as_deref());
        Some(
            row(text)
                .when(!stopping, |el| {
                    el.child(
                        self.button(("stall-stop", id as usize), "Stop", false)
                            .h(px(22.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.stop_session(id);
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(("stall-retry", id as usize), "Retry", true)
                            .h(px(22.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.retry_stalled(id, cx);
                                cx.notify();
                            })),
                    )
                })
                .into_any_element(),
        )
    }

    /// Changes when the Working line's height may have: helpers added, the list opened, or
    /// a note on a permission or a stall shown.
    pub(crate) fn working_version(&self, id: u64, messages: &[Message]) -> u64 {
        let n = helpers(messages).len();
        chat_list::hash((
            n,
            n > 0 && self.open_helpers.contains(&id),
            self.approval_for(id).map(|r| r.id.clone()),
            self.stall(id).is_some(),
        ))
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
        // The helper agents it waits on are named beside it (see `helpers_label`).
        ToolKind::Subagent => "Working".into(),
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
        Ring::at(motion::now())
    }

    /// The ring at time `t` on the window's clock.
    fn at(t: Duration) -> Ring {
        let phase = |period: Duration| motion::phase_at(t, period);
        let spin = phase(SPIN);
        let breathe = 0.5 - 0.5 * (phase(BREATHE) * std::f32::consts::TAU).cos();
        Ring {
            // A touch of easing on top of a steady turn, so the head surges a little as
            // the arc stretches.
            head: spin + 0.04 * (breathe * std::f32::consts::PI).sin(),
            sweep: 0.24 + 0.36 * breathe,
            core: 0.5 - 0.5 * (phase(BREATHE / 2) * std::f32::consts::TAU).cos(),
            done: 0.,
        }
    }
}

/// The row for a turn that has run since `started`: the ring, `label` and the time so
/// far, which, while nothing moves, redraws just as its second changes.
/// `helpers`, such as "2 helper agents", goes before the time.
pub(crate) fn running(
    label: impl Into<SharedString>,
    helpers: Option<String>,
    started: Option<Instant>,
) -> AnyElement {
    let took = started.map(|at| {
        let took = motion::frame()
            .unwrap_or_else(Instant::now)
            .saturating_duration_since(at);
        motion::tick_in(to_next_second(took));
        took
    });
    indicator(label, detail(helpers, took.map(elapsed)))
}

/// The Working line's text after its label: "2 helper agents · 12:03".
fn detail(helpers: Option<String>, time: Option<String>) -> Option<String> {
    match (helpers, time) {
        (Some(h), Some(t)) => Some(format!("{h} · {t}")),
        (h, t) => h.or(t),
    }
}

/// How long after `took` the time shown next changes, a little past the second so the
/// frame then is sure to show the new one.
fn to_next_second(took: Duration) -> Duration {
    Duration::from_millis(1000 - took.subsec_millis() as u64 + 10)
}

/// The row for a running turn: the ring, `label` and `detail`, such as how many run.
pub(crate) fn indicator(label: impl Into<SharedString>, detail: Option<String>) -> AnyElement {
    let moving = motion::moving();
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
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ring_canvas(ring, accent()))
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

/// The ring on its own: the small animated arc, drawn in `color`, at `BOX` across.
/// For reuse where the row is laid out differently (menu bar panel, etc.).
pub(crate) fn ring(color: u32) -> AnyElement {
    let ring = if motion::moving() {
        Ring::now()
    } else {
        Ring::STILL
    };
    ring_canvas(ring, color).into_any_element()
}

fn ring_canvas(ring: Ring, color: u32) -> impl IntoElement {
    let green = if is_dark() {
        mix(c_green(), 0xffffff, 0.75)
    } else {
        c_green()
    };
    canvas(
        |_, _, _| (),
        move |b, _, window, _| paint_ring(b, ring, color, green, window),
    )
    .flex_none()
    .size(px(BOX))
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
    // From three letters before the first to three after the last, so it's centred on
    // the middle letter half way across.
    let pos = -3. + (n as f32 + 5.) * t;
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
    fn the_working_line_counts_helper_agents() {
        let ask = Message::new(Role::User, "Fix it");
        let mut find = ToolStep::running("a", "Task", "Ran a subagent: Find the parser");
        find.latest = "Read src/stream.rs".into();
        let mut review = ToolStep::running("b", "Agent", "Ran a subagent: Review");
        review.finish(true);
        review.ms = 62_000;
        let reply = agent("", vec![find, review]);
        let all = [ask, reply];
        assert_eq!(&*doing(&all), "Working");
        let hs = helpers(&all);
        assert_eq!(hs.len(), 2);
        assert_eq!(helpers_label(&hs).as_deref(), Some("1 helper agent"));
        assert_eq!(
            detail(helpers_label(&hs), Some("12:03".into())).as_deref(),
            Some("1 helper agent · 12:03")
        );
        assert_eq!(
            helper_line(&hs[0]),
            "Find the parser · running · Read src/stream.rs"
        );
        assert_eq!(helper_line(&hs[1]), "Review · done 1:02");
        // Once none run, the line shows the time alone.
        assert_eq!(helpers_label(&hs[1..]), None);
        assert_eq!(detail(None, Some("8s".into())).as_deref(), Some("8s"));
    }

    #[test]
    fn the_time_redraws_just_after_its_second_changes() {
        let at = |ms| to_next_second(Duration::from_millis(ms));
        assert_eq!(at(27_000), Duration::from_millis(1010));
        assert_eq!(at(27_400), Duration::from_millis(610));
        assert_eq!(at(27_999), Duration::from_millis(11));
        // And then the time shown is the next second's.
        let took = Duration::from_millis(27_400);
        assert_eq!(elapsed(took + at(27_400)), "28s");
    }

    #[test]
    fn the_ring_moves_on_smoothly_and_never_starts_over() {
        // Frame after frame at 60 a second, three days into the app and 27 minutes into
        // a turn: the head always moves forward a little, never jumps or goes back to the
        // top.
        let frame = Duration::from_micros(16_667);
        let mut t = Duration::from_secs(3 * 24 * 3600 + 27 * 60);
        let mut last = Ring::at(t);
        for _ in 0..600 {
            t += frame;
            let ring = Ring::at(t);
            let step = (ring.head - last.head).rem_euclid(1.);
            assert!(
                step > 0.008 && step < 0.02,
                "a step of {step} turns at {t:?}"
            );
            assert!((ring.sweep - last.sweep).abs() < 0.02);
            last = ring;
        }
        // A dropped frame is only a longer step.
        let step = (Ring::at(t + frame * 2).head - last.head).rem_euclid(1.);
        assert!(step > 0.02 && step < 0.04);
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
