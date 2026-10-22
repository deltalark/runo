//! The Arcade: three small games to play while agents work. Spark (fly a spark through
//! the gates), Commits (a snake that grows on commits) and Merge (2048). Each is drawn in
//! the theme's own colours, keeps its best score in `arcade.json`, and pauses when the
//! Arcade closes, so a round picks up where it left off.

use super::*;
use std::path::PathBuf;
use std::time::Instant;

/// The play field, in pixels.
const W: f32 = 440.;
const H: f32 = 300.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Game {
    Spark,
    Commits,
    Merge,
}

impl Game {
    const ALL: [Game; 3] = [Game::Spark, Game::Commits, Game::Merge];

    fn name(self) -> &'static str {
        match self {
            Game::Spark => "Spark",
            Game::Commits => "Commits",
            Game::Merge => "Merge",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Game::Spark => "spark",
            Game::Commits => "commits",
            Game::Merge => "merge",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Game::Spark => "Space or click to lift",
            Game::Commits => "Arrows to steer",
            Game::Merge => "Arrows to slide",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Ready,
    Playing,
    Paused,
    Over,
}

/// A small xorshift generator: games need variety, not quality.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9, |d| d.as_nanos() as u64);
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn unit(&mut self) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.
    }
}

// Spark: a lit dot that falls and lifts, flying through gaps in rounded gates.

const SPARK_X: f32 = 110.;
const SPARK_R: f32 = 9.;
const GRAVITY: f32 = 980.;
const LIFT: f32 = -310.;
const GATE_W: f32 = 52.;
const GAP: f32 = 112.;
const GATE_EVERY: f32 = 210.;
const SPEED: f32 = 150.;

struct Gate {
    x: f32,
    /// Top of the gap.
    gap: f32,
    passed: bool,
}

struct Spark {
    y: f32,
    vy: f32,
    gates: Vec<Gate>,
    /// How far the field has scrolled, for the dot grid behind it.
    scroll: f32,
}

impl Spark {
    fn new() -> Self {
        Spark {
            y: H / 2.,
            vy: 0.,
            gates: Vec::new(),
            scroll: 0.,
        }
    }

    /// Moves the field on by `dt` seconds. Returns how many gates were passed, or None
    /// when the spark hit something.
    fn step(&mut self, dt: f32, rng: &mut Rng) -> Option<u32> {
        self.vy += GRAVITY * dt;
        self.y += self.vy * dt;
        self.scroll += SPEED * dt;
        for g in &mut self.gates {
            g.x -= SPEED * dt;
        }
        self.gates.retain(|g| g.x > -GATE_W);
        if self.gates.last().is_none_or(|g| g.x < W - GATE_EVERY) {
            let gap = 34. + rng.unit() * (H - GAP - 68.);
            self.gates.push(Gate {
                x: W + 10.,
                gap,
                passed: false,
            });
        }
        if self.y - SPARK_R < 0. || self.y + SPARK_R > H {
            return None;
        }
        let mut passed = 0;
        for g in &mut self.gates {
            let overlaps = SPARK_X + SPARK_R > g.x && SPARK_X - SPARK_R < g.x + GATE_W;
            if overlaps && (self.y - SPARK_R < g.gap || self.y + SPARK_R > g.gap + GAP) {
                return None;
            }
            if !g.passed && g.x + GATE_W < SPARK_X - SPARK_R {
                g.passed = true;
                passed += 1;
            }
        }
        Some(passed)
    }
}

// Commits: a snake on a grid that grows each time it reaches a commit.

const COLS: i32 = 22;
const ROWS: i32 = 15;
const CELL: f32 = 20.;

struct Commits {
    body: std::collections::VecDeque<(i32, i32)>,
    dir: (i32, i32),
    /// The turn asked for since the last step; applied on the next one, so two quick
    /// presses can't fold the snake back on itself.
    next: (i32, i32),
    food: (i32, i32),
    /// Seconds since the last step.
    acc: f32,
}

impl Commits {
    fn new(rng: &mut Rng) -> Self {
        let mut c = Commits {
            body: [(8, 7), (7, 7), (6, 7)].into_iter().collect(),
            dir: (1, 0),
            next: (1, 0),
            food: (0, 0),
            acc: 0.,
        };
        c.place_food(rng);
        c
    }

    fn place_food(&mut self, rng: &mut Rng) {
        loop {
            let at = (
                rng.below(COLS as usize) as i32,
                rng.below(ROWS as usize) as i32,
            );
            if !self.body.contains(&at) {
                self.food = at;
                return;
            }
        }
    }

    fn turn(&mut self, d: (i32, i32)) {
        if (d.0 + self.dir.0, d.1 + self.dir.1) != (0, 0) {
            self.next = d;
        }
    }

    /// Seconds between steps: a little quicker for every commit eaten.
    fn tick(&self) -> f32 {
        (0.12 - (self.body.len() as f32 - 3.) * 0.002).max(0.06)
    }

    fn step(&mut self, dt: f32, rng: &mut Rng) -> Option<u32> {
        self.acc += dt;
        let mut ate = 0;
        while self.acc >= self.tick() {
            self.acc -= self.tick();
            self.dir = self.next;
            let head = self.body[0];
            let to = (head.0 + self.dir.0, head.1 + self.dir.1);
            let grows = to == self.food;
            if !grows {
                self.body.pop_back();
            }
            if to.0 < 0 || to.1 < 0 || to.0 >= COLS || to.1 >= ROWS || self.body.contains(&to) {
                return None;
            }
            self.body.push_front(to);
            if grows {
                ate += 1;
                self.place_food(rng);
            }
        }
        Some(ate)
    }
}

// Merge: 2048 on a 4 by 4 board.

struct Merge {
    cells: [[u32; 4]; 4],
    /// The tile that appeared last, so it can pop in.
    fresh: Option<(usize, usize)>,
    moves: u64,
}

impl Merge {
    fn new(rng: &mut Rng) -> Self {
        let mut m = Merge {
            cells: [[0; 4]; 4],
            fresh: None,
            moves: 0,
        };
        m.spawn(rng);
        m.spawn(rng);
        m
    }

    fn spawn(&mut self, rng: &mut Rng) {
        let empty: Vec<(usize, usize)> = (0..16)
            .map(|i| (i / 4, i % 4))
            .filter(|&(r, c)| self.cells[r][c] == 0)
            .collect();
        if empty.is_empty() {
            return;
        }
        let (r, c) = empty[rng.below(empty.len())];
        self.cells[r][c] = if rng.below(10) == 0 { 4 } else { 2 };
        self.fresh = Some((r, c));
    }

    /// Slides one line toward its start, merging equal neighbours once. Returns the line
    /// and the points scored.
    fn slide(line: [u32; 4]) -> ([u32; 4], u32) {
        let tiles: Vec<u32> = line.into_iter().filter(|&v| v != 0).collect();
        let mut out = [0; 4];
        let (mut i, mut n, mut score) = (0, 0, 0);
        while i < tiles.len() {
            if i + 1 < tiles.len() && tiles[i] == tiles[i + 1] {
                out[n] = tiles[i] * 2;
                score += out[n];
                i += 2;
            } else {
                out[n] = tiles[i];
                i += 1;
            }
            n += 1;
        }
        (out, score)
    }

    /// Slides the board one way, `(dr, dc)`. Returns the points scored, or None when
    /// nothing moved.
    fn shift(&mut self, d: (i32, i32), rng: &mut Rng) -> Option<u32> {
        let before = self.cells;
        let mut score = 0;
        for k in 0..4 {
            // The line's cells, in the order tiles slide toward.
            let at = |i: usize| -> (usize, usize) {
                match d {
                    (0, -1) => (k, i),
                    (0, 1) => (k, 3 - i),
                    (-1, 0) => (i, k),
                    _ => (3 - i, k),
                }
            };
            let line = [0, 1, 2, 3].map(|i| {
                let (r, c) = at(i);
                self.cells[r][c]
            });
            let (out, s) = Self::slide(line);
            score += s;
            for (i, v) in out.into_iter().enumerate() {
                let (r, c) = at(i);
                self.cells[r][c] = v;
            }
        }
        if self.cells == before {
            return None;
        }
        self.moves += 1;
        self.spawn(rng);
        Some(score)
    }

    fn stuck(&self) -> bool {
        for r in 0..4 {
            for c in 0..4 {
                let v = self.cells[r][c];
                if v == 0
                    || (c < 3 && self.cells[r][c + 1] == v)
                    || (r < 3 && self.cells[r + 1][c] == v)
                {
                    return false;
                }
            }
        }
        true
    }
}

fn bests_path() -> PathBuf {
    store::data_dir().join("arcade.json")
}

fn load_bests() -> HashMap<String, u32> {
    std::fs::read_to_string(bests_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_bests(bests: &HashMap<String, u32>) {
    if let Ok(text) = serde_json::to_string_pretty(bests) {
        let path = bests_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, text);
    }
}

pub(crate) struct Arcade {
    pub(crate) focus: FocusHandle,
    game: Game,
    phase: Phase,
    score: u32,
    /// Whether this round beat the best score it started with.
    record: bool,
    bests: HashMap<String, u32>,
    rng: Rng,
    spark: Spark,
    commits: Commits,
    merge: Merge,
    last: Option<Instant>,
}

impl Arcade {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let mut rng = Rng::new();
        let commits = Commits::new(&mut rng);
        let merge = Merge::new(&mut rng);
        Arcade {
            focus: cx.focus_handle(),
            game: Game::Spark,
            phase: Phase::Ready,
            score: 0,
            record: false,
            bests: load_bests(),
            rng,
            spark: Spark::new(),
            commits,
            merge,
            last: None,
        }
    }

    fn best(&self) -> u32 {
        self.bests.get(self.game.key()).copied().unwrap_or(0)
    }

    fn reset(&mut self) {
        self.score = 0;
        self.record = false;
        self.last = None;
        match self.game {
            Game::Spark => self.spark = Spark::new(),
            Game::Commits => self.commits = Commits::new(&mut self.rng),
            Game::Merge => self.merge = Merge::new(&mut self.rng),
        }
        // Merge has no clock, so it is ready to play as soon as it is dealt.
        self.phase = if self.game == Game::Merge {
            Phase::Playing
        } else {
            Phase::Ready
        };
    }

    fn pick(&mut self, game: Game) {
        if self.game != game {
            self.game = game;
            self.reset();
        }
    }

    /// Holds the round still: called when the Arcade closes.
    pub(crate) fn pause(&mut self) {
        if self.phase == Phase::Playing && self.game != Game::Merge {
            self.phase = Phase::Paused;
        }
        self.last = None;
    }

    fn add(&mut self, points: u32) {
        self.score += points;
        if self.score > self.best() {
            self.record = true;
            self.bests.insert(self.game.key().into(), self.score);
        }
    }

    fn over(&mut self) {
        self.phase = Phase::Over;
        if self.record {
            save_bests(&self.bests);
        }
    }

    fn start(&mut self) {
        match self.phase {
            Phase::Over => {
                self.reset();
                if self.game != Game::Merge {
                    self.phase = Phase::Playing;
                }
            }
            Phase::Ready | Phase::Paused => self.phase = Phase::Playing,
            Phase::Playing => {}
        }
        self.last = None;
    }

    fn lift(&mut self) {
        if self.game != Game::Spark {
            return;
        }
        if self.phase != Phase::Playing {
            self.start();
        }
        self.spark.vy = LIFT;
    }

    fn key(&mut self, key: &str) -> bool {
        let dir = match key {
            "up" | "w" => Some((-1, 0)),
            "down" | "s" => Some((1, 0)),
            "left" | "a" => Some((0, -1)),
            "right" | "d" => Some((0, 1)),
            _ => None,
        };
        match key {
            "1" => self.pick(Game::Spark),
            "2" => self.pick(Game::Commits),
            "3" => self.pick(Game::Merge),
            "p" if self.phase == Phase::Playing => self.pause(),
            "p" if self.phase == Phase::Paused => self.start(),
            "r" => self.reset(),
            "space" | "up" | "w" if self.game == Game::Spark => self.lift(),
            "space" | "enter" if self.phase != Phase::Playing => self.start(),
            _ => match (self.game, dir) {
                (Game::Commits, Some((dr, dc))) => {
                    if self.phase != Phase::Playing {
                        self.start();
                    }
                    self.commits.turn((dc, dr));
                }
                (Game::Merge, Some(d)) if self.phase == Phase::Playing => {
                    if let Some(points) = self.merge.shift(d, &mut self.rng) {
                        self.add(points);
                        if self.merge.stuck() {
                            self.over();
                        }
                    }
                }
                _ => return false,
            },
        }
        true
    }

    /// Moves a clocked game on to now.
    fn advance(&mut self) {
        if self.phase != Phase::Playing || self.game == Game::Merge {
            self.last = None;
            return;
        }
        let now = Instant::now();
        // A long gap (a hidden window, a slow frame) is taken as one short step, so the
        // spark never falls through a gate unseen.
        let dt = self
            .last
            .map_or(0., |t| now.duration_since(t).as_secs_f32().min(1. / 30.));
        self.last = Some(now);
        let stepped = match self.game {
            Game::Spark => self.spark.step(dt, &mut self.rng),
            Game::Commits => self.commits.step(dt, &mut self.rng),
            Game::Merge => Some(0),
        };
        match stepped {
            Some(points) => self.add(points),
            None => self.over(),
        }
    }
}

fn alpha(color: u32, a: f32) -> Rgba {
    rgba((color << 8) | (a.clamp(0., 1.) * 255.) as u32)
}

fn glow(color: u32, a: f32, blur: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: alpha(color, a).into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// The accent, a touch brighter on dark themes where it sits close to the canvas.
fn lit() -> u32 {
    if is_dark() {
        mix(c_blue(), 0xffffff, 0.75)
    } else {
        c_blue()
    }
}

const EMBER: u32 = 0xd97757;

impl Arcade {
    /// The faint dot grid every field sits on, drifting by `shift` pixels.
    fn dots(shift: f32) -> Div {
        let step = 22.;
        let mut field = div().absolute().inset_0();
        let off = shift % step;
        let dot = alpha(c_text(), if is_dark() { 0.09 } else { 0.11 });
        let mut y = step / 2.;
        while y < H {
            let mut x = step / 2. - off;
            while x < W {
                field = field.child(
                    div()
                        .absolute()
                        .left(px(x - 1.))
                        .top(px(y - 1.))
                        .size(px(2.))
                        .rounded_full()
                        .bg(dot),
                );
                x += step;
            }
            y += step;
        }
        field
    }

    fn spark_field(&self) -> Div {
        let mut field = div().child(Self::dots(self.spark.scroll * 0.4));
        let gate = mix(c_blue(), c_canvas(), if is_dark() { 0.28 } else { 0.22 });
        let rim = mix(c_blue(), c_canvas(), 0.55);
        for g in &self.spark.gates {
            for (top, h) in [(-12., g.gap + 12.), (g.gap + GAP, H - g.gap - GAP + 12.)] {
                field = field.child(
                    div()
                        .absolute()
                        .left(px(g.x))
                        .top(px(top))
                        .w(px(GATE_W))
                        .h(px(h))
                        .rounded(px(12.))
                        .bg(rgb(gate))
                        .border_1()
                        .border_color(rgb(rim)),
                );
            }
        }
        // The spark leans into its motion, and leaves a short fading trail.
        let tilt = (self.spark.vy / 600.).clamp(-1., 1.);
        for (i, a) in [0.28, 0.16, 0.08].into_iter().enumerate() {
            let back = (i as f32 + 1.) * 9.;
            let r = SPARK_R * (1. - 0.18 * (i as f32 + 1.));
            field = field.child(
                div()
                    .absolute()
                    .left(px(SPARK_X - back - r))
                    .top(px(self.spark.y - tilt * back * 0.5 - r))
                    .size(px(r * 2.))
                    .rounded_full()
                    .bg(alpha(EMBER, a)),
            );
        }
        field.child(
            div()
                .absolute()
                .left(px(SPARK_X - SPARK_R))
                .top(px(self.spark.y - SPARK_R))
                .size(px(SPARK_R * 2.))
                .rounded_full()
                .bg(rgb(mix(0xffffff, EMBER, 0.25)))
                .border_2()
                .border_color(rgb(EMBER))
                .shadow(glow(EMBER, 0.7, 18.)),
        )
    }

    fn commits_field(&self) -> Div {
        let ox = (W - COLS as f32 * CELL) / 2.;
        let oy = (H - ROWS as f32 * CELL) / 2.;
        let mut field = div().child(Self::dots(0.));
        let (fc, fr) = self.commits.food;
        // The commit: a ring with a dot, as in a git graph.
        field = field.child(
            div()
                .absolute()
                .left(px(ox + fc as f32 * CELL + 3.))
                .top(px(oy + fr as f32 * CELL + 3.))
                .size(px(CELL - 6.))
                .rounded_full()
                .border_2()
                .border_color(rgb(c_green()))
                .shadow(glow(c_green(), 0.55, 12.))
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(4.)).rounded_full().bg(rgb(c_green()))),
        );
        // The snake is a branch in a git graph: a line through its commits, with the
        // newest commit at its head.
        let n = self.commits.body.len().max(1) as f32;
        let centre = |(c, r): (i32, i32)| {
            (
                ox + c as f32 * CELL + CELL / 2.,
                oy + r as f32 * CELL + CELL / 2.,
            )
        };
        let body: Vec<(i32, i32)> = self.commits.body.iter().copied().collect();
        const LINE: f32 = 4.;
        for (i, pair) in body.windows(2).enumerate() {
            let ((x1, y1), (x2, y2)) = (centre(pair[0]), centre(pair[1]));
            let t = i as f32 / n;
            field = field.child(
                div()
                    .absolute()
                    .left(px(x1.min(x2) - LINE / 2.))
                    .top(px(y1.min(y2) - LINE / 2.))
                    .w(px((x1 - x2).abs() + LINE))
                    .h(px((y1 - y2).abs() + LINE))
                    .rounded_full()
                    .bg(alpha(mix(c_blue(), lit(), t * 0.6), 1. - t * 0.5)),
            );
        }
        for (i, &cell) in body.iter().enumerate().rev() {
            let (x, y) = centre(cell);
            let t = i as f32 / n;
            let color = mix(c_blue(), lit(), t * 0.6);
            let seg = if i == 0 {
                div()
                    .absolute()
                    .left(px(x - 8.))
                    .top(px(y - 8.))
                    .size(px(16.))
                    .rounded(px(5.))
                    .bg(rgb(lit()))
                    .shadow(glow(c_blue(), 0.7, 14.))
            } else {
                div()
                    .absolute()
                    .left(px(x - 5.))
                    .top(px(y - 5.))
                    .size(px(10.))
                    .rounded_full()
                    .border_2()
                    .border_color(alpha(color, 1. - t * 0.5))
                    .bg(rgb(c_canvas()))
            };
            field = field.child(seg);
        }
        field
    }

    fn merge_field(&self) -> Div {
        const SIDE: f32 = 270.;
        const GAPS: f32 = 8.;
        let tile = (SIDE - GAPS * 5.) / 4.;
        let mut board = div()
            .absolute()
            .left(px((W - SIDE) / 2.))
            .top(px((H - SIDE) / 2.))
            .size(px(SIDE))
            .rounded(px(14.))
            .bg(alpha(c_text(), if is_dark() { 0.05 } else { 0.06 }));
        for r in 0..4 {
            for c in 0..4 {
                let v = self.merge.cells[r][c];
                let left = GAPS + c as f32 * (tile + GAPS);
                let top = GAPS + r as f32 * (tile + GAPS);
                if v == 0 {
                    board = board.child(
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .size(px(tile))
                            .rounded(px(10.))
                            .bg(alpha(c_text(), 0.04)),
                    );
                    continue;
                }
                // 2 is a quiet tile; the accent comes in by 32, and ember takes over past 256.
                let level = (v as f32).log2();
                let (bg, fg) = if level <= 1. {
                    (mix(c_blue(), c_canvas(), 0.14), c_text())
                } else if level < 5. {
                    (mix(c_blue(), c_canvas(), 0.14 + level * 0.12), c_text())
                } else if level < 8. {
                    (mix(EMBER, c_blue(), (level - 5.) / 3.), 0xffffff)
                } else {
                    (EMBER, 0xffffff)
                };
                let size = match v {
                    v if v >= 1024 => 20.,
                    v if v >= 128 => 24.,
                    _ => 28.,
                };
                let mut el = div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .size(px(tile))
                    .rounded(px(10.))
                    .bg(rgb(bg))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(size))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(fg))
                    .child(v.to_string());
                if level >= 5. {
                    el = el.shadow(glow(bg, 0.5, 16.));
                }
                if self.merge.fresh == Some((r, c)) {
                    board = board.child(el.animated(
                        ElementId::Name(format!("merge-{}-{r}-{c}", self.merge.moves).into()),
                        Animation::new(Duration::from_millis(160)).with_easing(ease_out_quint()),
                        |el, t| el.opacity(t),
                    ));
                } else {
                    board = board.child(el.into_any_element());
                }
            }
        }
        div().child(board)
    }

    /// The card over the field before a round, when paused and after a crash.
    fn banner(&self) -> Option<Div> {
        let (title, line) = match self.phase {
            Phase::Playing => return None,
            Phase::Ready => (
                self.game.name().to_string(),
                format!("{} · Space to start", self.game.hint()),
            ),
            Phase::Paused => ("Paused".into(), "Space to carry on".into()),
            Phase::Over if self.record => (
                format!("New best · {}", self.score),
                "Space to play again".into(),
            ),
            Phase::Over => (
                format!("Score {}", self.score),
                format!("Best {} · Space to play again", self.best()),
            ),
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .v_flex()
                        .items_center()
                        .gap_1()
                        .px_5()
                        .py_3()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(float_edge())
                        .bg(float(c_raised()))
                        .shadow_lg()
                        .child(
                            div()
                                .text_size(px(17.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(if self.record && self.phase == Phase::Over {
                                    EMBER
                                } else {
                                    c_text()
                                }))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(c_muted()))
                                .child(line),
                        ),
                ),
        )
    }
}

impl Render for Arcade {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance();
        if self.phase == Phase::Playing && self.game != Game::Merge {
            window.request_animation_frame();
        }
        let field = match self.game {
            Game::Spark => self.spark_field(),
            Game::Commits => self.commits_field(),
            Game::Merge => self.merge_field(),
        };
        let tabs = Game::ALL.into_iter().enumerate().fold(
            div()
                .flex()
                .p(px(3.))
                .gap(px(2.))
                .rounded(px(9.))
                .bg(tint(c_subtle())),
            |row, (i, game)| {
                let on = game == self.game;
                row.child(
                    div()
                        .id(("arcade-tab", i))
                        .h(px(26.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(px(7.))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .text_color(rgb(if on { c_text() } else { c_muted() }))
                        .when(on, |el| el.bg(rgb(c_raised())).shadow_sm())
                        .hover(|el| el.text_color(rgb(c_text())))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick(game);
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }))
                        .child(game.name()),
                )
            },
        );
        let score = div()
            .flex()
            .items_center()
            .gap_3()
            .text_size(px(12.))
            .text_color(rgb(c_muted()))
            .child(
                div().flex().items_baseline().gap_1().child("Score").child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_text()))
                        .child(self.score.to_string()),
                ),
            )
            .child(
                div().flex().items_baseline().gap_1().child("Best").child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(if self.record { EMBER } else { c_secondary() }))
                        .child(self.best().to_string()),
                ),
            );
        div()
            .id("arcade")
            .track_focus(&self.focus)
            .key_context("Arcade")
            .v_flex()
            .gap_3()
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.key(e.keystroke.key.as_str()) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(tabs)
                    .child(score),
            )
            .child(
                div()
                    .id("arcade-field")
                    .relative()
                    .w(px(W))
                    .h(px(H))
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(divider())
                    .bg(alpha(c_canvas(), if glass_on() { 0.55 } else { 1. }))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            window.focus(&this.focus, cx);
                            if this.game == Game::Spark {
                                this.lift();
                            } else if this.phase != Phase::Playing {
                                this.start();
                            }
                            cx.notify();
                        }),
                    )
                    .child(field.absolute().inset_0())
                    .children(self.banner()),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(self.game.hint())
                    .child("1 2 3 switch · P pause · R restart"),
            )
    }
}

impl RunoApp {
    /// Opens the Arcade, making it the first time, with the round as it was left.
    pub(crate) fn open_arcade(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let arcade = self
            .arcade
            .get_or_insert_with(|| cx.new(Arcade::new))
            .clone();
        // Remember who was working when it opened, so it can say when they are done.
        self.arcade_open = true;
        self.arcade_waiting = self.working_count();
        window.focus(&arcade.read(cx).focus.clone(), cx);
        cx.notify();
    }

    pub(crate) fn close_arcade(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.arcade_open = false;
        if let Some(a) = &self.arcade {
            a.update(cx, |a, _| a.pause());
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Sessions with an agent running right now.
    pub(crate) fn working_count(&self) -> usize {
        self.state
            .sessions
            .iter()
            .filter(|s| s.status == Status::Working)
            .count()
    }

    /// The small chip beside "Working" that offers a game while the agent runs.
    pub(crate) fn arcade_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("arcade-chip")
            .h(px(22.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded_full()
            .border_1()
            .border_color(divider())
            .text_size(px(11.5))
            .text_color(rgb(c_muted()))
            .cursor_pointer()
            .hover(|el| el.text_color(rgb(c_text())).bg(tint(c_subtle())))
            .on_click(cx.listener(|this, _, window, cx| this.open_arcade(window, cx)))
            .child(icon(IconName::Gamepad2, 12., c_muted()))
            .child("Play while you wait")
            .into_any_element()
    }

    pub(crate) fn arcade_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.arcade_open {
            return None;
        }
        let arcade = self.arcade.clone()?;
        let working = self.working_count();
        let done = self.arcade_waiting > 0 && working == 0;
        let status = if done {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().size(px(7.)).rounded_full().bg(rgb(c_green())))
                .child(
                    div()
                        .text_color(rgb(c_text()))
                        .child("Your agents are done"),
                )
                .child(
                    self.button("arcade-back", "Back to work", false)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_arcade(window, cx);
                            if this.current().is_some() {
                                this.view = View::Session;
                            }
                        })),
                )
        } else if working > 0 {
            div().child(working_indicator(Some(format!(
                "{working} agent{}",
                if working == 1 { "" } else { "s" }
            ))))
        } else {
            div().text_color(rgb(c_muted())).child("No agents running")
        };
        let card = div()
            .id("arcade-card")
            .p_5()
            .v_flex()
            .gap_4()
            .rounded(px(crate::shell::ISLAND_R))
            .border_1()
            .border_color(float_edge())
            .bg(float(c_sheet()))
            .shadow_2xl()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon(IconName::Gamepad2, 16., c_blue()))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Arcade"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_size(px(12.))
                            .child(status)
                            .child(
                                div()
                                    .id("arcade-close")
                                    .size(px(24.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .hover(|el| el.bg(tint(c_subtle())))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_arcade(window, cx)
                                    }))
                                    .child(icon(IconName::X, 14., c_muted())),
                            ),
                    ),
            )
            .child(arcade);
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.45)
        } else {
            hsla(0., 0., 0., 0.22)
        };
        Some(fade(
            div()
                .id("arcade-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(backdrop)
                .occlude()
                .on_click(cx.listener(|this, _, window, cx| this.close_arcade(window, cx)))
                .child(rise(card, "arcade-rise", 300)),
            "arcade-fade",
            200,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn merges_each_pair_once() {
        assert_eq!(Merge::slide([2, 2, 2, 2]), ([4, 4, 0, 0], 8));
        assert_eq!(Merge::slide([0, 4, 4, 8]), ([8, 8, 0, 0], 8));
        assert_eq!(Merge::slide([2, 0, 0, 2]), ([4, 0, 0, 0], 4));
        assert_eq!(Merge::slide([2, 4, 8, 16]), ([2, 4, 8, 16], 0));
    }

    #[test]
    fn shifting_right_slides_toward_the_right_edge() {
        let mut rng = Rng::new();
        let mut m = Merge {
            cells: [[2, 2, 0, 0], [0; 4], [0; 4], [0; 4]],
            fresh: None,
            moves: 0,
        };
        assert_eq!(m.shift((0, 1), &mut rng), Some(4));
        assert_eq!(m.cells[0][3], 4);
        // A board that can't move doesn't count as a move or deal a tile.
        let mut full = Merge {
            cells: [[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 2]],
            fresh: None,
            moves: 0,
        };
        assert_eq!(full.shift((0, -1), &mut rng), None);
        assert!(full.stuck());
    }

    #[test]
    fn the_snake_cannot_reverse_into_itself() {
        let mut rng = Rng::new();
        let mut c = Commits::new(&mut rng);
        c.turn((-1, 0));
        assert_eq!(c.next, (1, 0));
        c.turn((0, 1));
        assert_eq!(c.next, (0, 1));
    }

    #[test]
    fn the_spark_crashes_into_the_floor() {
        let mut rng = Rng::new();
        let mut s = Spark::new();
        let mut alive = true;
        for _ in 0..200 {
            if s.step(1. / 60., &mut rng).is_none() {
                alive = false;
                break;
            }
        }
        assert!(!alive);
    }
}
