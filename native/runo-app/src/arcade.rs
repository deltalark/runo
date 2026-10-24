//! The Arcade: four small games to play while agents work. Spark (fly a spark through
//! the gates), Commits (a snake that grows on commits), Merge (2048) and Stack (Tetris).
//! Each has its own palette and scene, with particles, score pops and a shake on a crash
//! that calm down when the system asks for less motion. Each keeps its best score in
//! `arcade.json`, and pauses when the Arcade closes, so a round picks up where it left off.

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
    Stack,
}

impl Game {
    const ALL: [Game; 4] = [Game::Spark, Game::Commits, Game::Merge, Game::Stack];

    fn name(self) -> &'static str {
        match self {
            Game::Spark => "Spark",
            Game::Commits => "Commits",
            Game::Merge => "Merge",
            Game::Stack => "Stack",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Game::Spark => "spark",
            Game::Commits => "commits",
            Game::Merge => "merge",
            Game::Stack => "stack",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Game::Spark => "Space or click to lift",
            Game::Commits => "Arrows to steer",
            Game::Merge => "Arrows to slide",
            Game::Stack => "Arrows to move, ↑ to turn, Space to drop",
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
    /// The tiles the last move made by merging, so they can bump.
    merged: Vec<(usize, usize)>,
    moves: u64,
}

impl Merge {
    fn new(rng: &mut Rng) -> Self {
        let mut m = Merge {
            cells: [[0; 4]; 4],
            fresh: None,
            merged: Vec::new(),
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
    #[cfg(test)]
    fn slide(line: [u32; 4]) -> ([u32; 4], u32) {
        let (out, score, _) = Self::slide_marked(line);
        (out, score)
    }

    /// `slide`, also marking which places in the line hold a merged tile.
    fn slide_marked(line: [u32; 4]) -> ([u32; 4], u32, [bool; 4]) {
        let tiles: Vec<u32> = line.into_iter().filter(|&v| v != 0).collect();
        let mut out = [0; 4];
        let mut marks = [false; 4];
        let (mut i, mut n, mut score) = (0, 0, 0);
        while i < tiles.len() {
            if i + 1 < tiles.len() && tiles[i] == tiles[i + 1] {
                out[n] = tiles[i] * 2;
                marks[n] = true;
                score += out[n];
                i += 2;
            } else {
                out[n] = tiles[i];
                i += 1;
            }
            n += 1;
        }
        (out, score, marks)
    }

    /// Slides the board one way, `(dr, dc)`. Returns the points scored, or None when
    /// nothing moved.
    fn shift(&mut self, d: (i32, i32), rng: &mut Rng) -> Option<u32> {
        let before = self.cells;
        let mut score = 0;
        let mut merged = Vec::new();
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
            let (out, s, marks) = Self::slide_marked(line);
            score += s;
            for (i, v) in out.into_iter().enumerate() {
                let (r, c) = at(i);
                self.cells[r][c] = v;
                if marks[i] {
                    merged.push((r, c));
                }
            }
        }
        if self.cells == before {
            return None;
        }
        self.moves += 1;
        self.merged = merged;
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

// Stack: Tetris in a 10 by 20 well, with the seven tetrominoes, a next piece and levels
// that speed it up.

const WELL_W: usize = 10;
const WELL_H: usize = 20;
const BLOCK: f32 = 14.;
/// The least time a piece rests on the stack before it locks, so it can still slide.
const LOCK: f32 = 0.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    I,
    O,
    T,
    S,
    Z,
    J,
    L,
}

impl Kind {
    const ALL: [Kind; 7] = [
        Kind::I,
        Kind::O,
        Kind::T,
        Kind::S,
        Kind::Z,
        Kind::J,
        Kind::L,
    ];

    /// The side of the square box the piece turns in.
    fn size(self) -> i32 {
        match self {
            Kind::I => 4,
            Kind::O => 2,
            _ => 3,
        }
    }

    /// Its cells in its box, `(col, row)`, as it first appears.
    fn cells(self) -> [(i32, i32); 4] {
        match self {
            Kind::I => [(0, 1), (1, 1), (2, 1), (3, 1)],
            Kind::O => [(0, 0), (1, 0), (0, 1), (1, 1)],
            Kind::T => [(1, 0), (0, 1), (1, 1), (2, 1)],
            Kind::S => [(1, 0), (2, 0), (0, 1), (1, 1)],
            Kind::Z => [(0, 0), (1, 0), (1, 1), (2, 1)],
            Kind::J => [(0, 0), (0, 1), (1, 1), (2, 1)],
            Kind::L => [(2, 0), (0, 1), (1, 1), (2, 1)],
        }
    }

    /// The classic colours, vivid on the well's dark glass in either theme.
    fn color(self) -> u32 {
        match self {
            Kind::I => 0x22d3ee,
            Kind::O => 0xfacc15,
            Kind::T => 0xa855f7,
            Kind::S => 0x22c55e,
            Kind::Z => 0xef4444,
            Kind::J => 0x3b82f6,
            Kind::L => 0xf97316,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Piece {
    kind: Kind,
    /// Quarter turns clockwise.
    rot: u8,
    col: i32,
    row: i32,
}

impl Piece {
    fn spawn(kind: Kind) -> Self {
        Piece {
            kind,
            rot: 0,
            col: (WELL_W as i32 - kind.size()) / 2,
            // The I piece lies on its box's second row; lift it so it starts at the top.
            row: if kind == Kind::I { -1 } else { 0 },
        }
    }

    /// Its cells in the well, `(col, row)`.
    fn cells(self) -> [(i32, i32); 4] {
        let n = self.kind.size();
        self.kind.cells().map(|(mut c, mut r)| {
            for _ in 0..self.rot {
                (c, r) = (n - 1 - r, c);
            }
            (self.col + c, self.row + r)
        })
    }

    fn moved(self, dc: i32, dr: i32) -> Self {
        Piece {
            col: self.col + dc,
            row: self.row + dr,
            ..self
        }
    }
}

struct Stack {
    well: [[Option<Kind>; WELL_W]; WELL_H],
    piece: Piece,
    next: Kind,
    /// What's left of the current shuffled set of all seven, so no piece is kept away long.
    bag: Vec<Kind>,
    lines: u32,
    /// The rows the last lock cleared, counted before they went, so they can flash.
    cleared: Vec<usize>,
    /// Seconds since the piece last fell a row.
    acc: f32,
}

impl Stack {
    fn new(rng: &mut Rng) -> Self {
        let mut s = Stack {
            well: [[None; WELL_W]; WELL_H],
            piece: Piece::spawn(Kind::T),
            next: Kind::T,
            bag: Vec::new(),
            lines: 0,
            cleared: Vec::new(),
            acc: 0.,
        };
        s.piece = Piece::spawn(s.draw(rng));
        s.next = s.draw(rng);
        s
    }

    fn draw(&mut self, rng: &mut Rng) -> Kind {
        if self.bag.is_empty() {
            self.bag = Kind::ALL.to_vec();
            for i in (1..self.bag.len()).rev() {
                self.bag.swap(i, rng.below(i + 1));
            }
        }
        self.bag.pop().unwrap_or(Kind::T)
    }

    fn level(&self) -> u32 {
        1 + self.lines / 10
    }

    /// Seconds per row: each level falls about 15% faster.
    fn gravity(&self) -> f32 {
        (0.8 * 0.85f32.powi(self.level() as i32 - 1)).max(0.04)
    }

    /// Whether a piece sits inside the well without overlapping the stack. Cells above
    /// the top are allowed, so a piece can turn as it enters.
    fn fits(&self, p: Piece) -> bool {
        p.cells().into_iter().all(|(c, r)| {
            c >= 0
                && c < WELL_W as i32
                && r < WELL_H as i32
                && (r < 0 || self.well[r as usize][c as usize].is_none())
        })
    }

    fn grounded(&self) -> bool {
        !self.fits(self.piece.moved(0, 1))
    }

    fn shift(&mut self, dc: i32) -> bool {
        let to = self.piece.moved(dc, 0);
        let ok = self.fits(to);
        if ok {
            self.piece = to;
        }
        ok
    }

    /// Turns the piece a quarter, clockwise for 1 and back for -1, nudging it off a wall
    /// or the stack when it would not fit where it is.
    fn rotate(&mut self, dir: i32) -> bool {
        if self.piece.kind == Kind::O {
            return false;
        }
        let turned = Piece {
            rot: (self.piece.rot as i32 + dir).rem_euclid(4) as u8,
            ..self.piece
        };
        for (dc, dr) in [(0, 0), (-1, 0), (1, 0), (-2, 0), (2, 0), (0, -1)] {
            let to = turned.moved(dc, dr);
            if self.fits(to) {
                self.piece = to;
                return true;
            }
        }
        false
    }

    /// Where the piece would land, drawn as a faint outline.
    fn ghost(&self) -> Piece {
        let mut p = self.piece;
        while self.fits(p.moved(0, 1)) {
            p = p.moved(0, 1);
        }
        p
    }

    /// Fixes the piece into the stack, clears full rows and brings on the next piece.
    /// Returns the points scored, or None when the stack has reached the top.
    fn lock(&mut self, rng: &mut Rng) -> Option<u32> {
        for (c, r) in self.piece.cells() {
            if r < 0 {
                return None;
            }
            self.well[r as usize][c as usize] = Some(self.piece.kind);
        }
        self.cleared = (0..WELL_H)
            .filter(|&r| self.well[r].iter().all(Option::is_some))
            .collect();
        let kept: Vec<[Option<Kind>; WELL_W]> = self
            .well
            .iter()
            .copied()
            .filter(|row| row.iter().any(Option::is_none))
            .collect();
        let cleared = WELL_H - kept.len();
        let mut well = [[None; WELL_W]; WELL_H];
        well[cleared..].copy_from_slice(&kept);
        self.well = well;
        let points = [0, 100, 300, 500, 800][cleared] * self.level();
        self.lines += cleared as u32;
        self.piece = Piece::spawn(self.next);
        self.next = self.draw(rng);
        self.acc = 0.;
        self.fits(self.piece).then_some(points)
    }

    /// Moves the piece down one row, a point for the row, or locks it if it has landed.
    fn soft_drop(&mut self, rng: &mut Rng) -> Option<u32> {
        if self.grounded() {
            return self.lock(rng);
        }
        self.piece = self.piece.moved(0, 1);
        self.acc = 0.;
        Some(1)
    }

    /// Drops the piece straight down and locks it, two points for each row.
    fn hard_drop(&mut self, rng: &mut Rng) -> Option<u32> {
        let ghost = self.ghost();
        let rows = (ghost.row - self.piece.row) as u32;
        self.piece = ghost;
        Some(rows * 2 + self.lock(rng)?)
    }

    fn step(&mut self, dt: f32, rng: &mut Rng) -> Option<u32> {
        self.acc += dt;
        let mut points = 0;
        loop {
            let grounded = self.grounded();
            let wait = if grounded {
                self.gravity().max(LOCK)
            } else {
                self.gravity()
            };
            if self.acc < wait {
                return Some(points);
            }
            self.acc -= wait;
            if grounded {
                points += self.lock(rng)?;
            } else {
                self.piece = self.piece.moved(0, 1);
            }
        }
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
    stack: Stack,
    last: Option<Instant>,
    /// Particles, score pops, shake and the counters, on their own clock.
    fx: Fx,
    /// Draws for the effects, kept apart so they never change what the games deal.
    fx_rng: Rng,
    /// Animations are calmed: the system asks for less motion, or Runo's are off.
    calm: bool,
    /// Counts tab picks, so the picked cartridge bounces each time.
    picks: u32,
    /// Counts rounds, so each round's cards play in afresh.
    rounds: u32,
}

impl Arcade {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let mut rng = Rng::new();
        let commits = Commits::new(&mut rng);
        let merge = Merge::new(&mut rng);
        let stack = Stack::new(&mut rng);
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
            stack,
            last: None,
            fx: Fx::default(),
            fx_rng: Rng(Rng::new().0.rotate_left(29) | 1),
            calm: calm_motion(cx),
            picks: 0,
            rounds: 0,
        }
    }

    /// Reads the motion settings again: called as the Arcade opens.
    pub(crate) fn refresh_motion(&mut self, cx: &App) {
        self.calm = calm_motion(cx);
    }

    fn best(&self) -> u32 {
        self.bests.get(self.game.key()).copied().unwrap_or(0)
    }

    fn reset(&mut self) {
        self.score = 0;
        self.record = false;
        self.last = None;
        self.rounds += 1;
        match self.game {
            Game::Spark => self.spark = Spark::new(),
            Game::Commits => self.commits = Commits::new(&mut self.rng),
            Game::Merge => self.merge = Merge::new(&mut self.rng),
            Game::Stack => self.stack = Stack::new(&mut self.rng),
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
            self.picks += 1;
            self.fx.clear();
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

    /// Counts a move's points, or ends the round when the move crashed.
    fn land(&mut self, points: Option<u32>) {
        match points {
            Some(points) => self.add(points),
            None => self.over(),
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
        if !self.calm {
            self.fx.squash = SQUASH;
        }
    }

    /// Handles a key, then plays whatever it set off.
    fn press(&mut self, key: &str) -> bool {
        let before = self.before();
        let handled = self.key(key);
        self.react(before);
        handled
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
            "4" => self.pick(Game::Stack),
            "p" if self.phase == Phase::Playing => self.pause(),
            "p" if self.phase == Phase::Paused => self.start(),
            "r" => self.reset(),
            "space" | "up" | "w" if self.game == Game::Spark => self.lift(),
            "space" | "enter" if self.phase != Phase::Playing => self.start(),
            "space" if self.game == Game::Stack => {
                let points = self.stack.hard_drop(&mut self.rng);
                self.land(points);
            }
            "z" | "x" if self.game == Game::Stack && self.phase == Phase::Playing => {
                self.stack.rotate(if key == "z" { -1 } else { 1 });
            }
            _ => match (self.game, dir) {
                (Game::Stack, Some(d)) => {
                    if self.phase != Phase::Playing {
                        self.start();
                    }
                    match d {
                        (-1, 0) => {
                            self.stack.rotate(1);
                        }
                        (1, 0) => {
                            let points = self.stack.soft_drop(&mut self.rng);
                            self.land(points);
                        }
                        (_, dc) => {
                            self.stack.shift(dc);
                        }
                    }
                }
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
        let before = self.before();
        let stepped = match self.game {
            Game::Spark => self.spark.step(dt, &mut self.rng),
            Game::Commits => self.commits.step(dt, &mut self.rng),
            Game::Merge => Some(0),
            Game::Stack => self.stack.step(dt, &mut self.rng),
        };
        self.land(stepped);
        self.react(before);
    }
}

fn alpha(color: u32, a: f32) -> Rgba {
    rgba((color << 8) | (a.clamp(0., 1.) * 255.) as u32)
}

fn glow(color: u32, a: f32, blur: f32) -> Vec<BoxShadow> {
    vec![soft(color, a, blur, 0.)]
}

/// A soft shadow `lift` pixels below, or a glow when `lift` is 0.
fn soft(color: u32, a: f32, blur: f32, lift: f32) -> BoxShadow {
    BoxShadow {
        color: alpha(color, a).into(),
        offset: point(px(0.), px(lift)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }
}

/// A two-stop gradient, `angle` in degrees with 0 pointing up.
fn grad(angle: f32, from: impl Into<Hsla>, to: impl Into<Hsla>) -> Background {
    linear_gradient(
        angle,
        linear_color_stop(from, 0.),
        linear_color_stop(to, 1.),
    )
}

fn lighten(color: u32, t: f32) -> u32 {
    mix(0xffffff, color, t)
}

fn darken(color: u32, t: f32) -> u32 {
    mix(0x000000, color, t)
}

/// The gold of a record: confetti, the NEW BEST ribbon.
const GOLD: u32 = 0xffc83d;

/// Whether to calm the Arcade's motion: the system asks for less (Accessibility >
/// Display > Reduce motion), or animations are off in Runo's settings.
fn calm_motion(cx: &App) -> bool {
    !anim_on() || crate::motion::calm(cx)
}

/// Overshoots a little and settles: a bounce in.
fn back_out(t: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    let u = t - 1.;
    1. + c3 * u * u * u + c1 * u * u
}

impl Game {
    /// Its own colour, bright: cartridges, the bezel, particles.
    fn hue(self) -> u32 {
        match self {
            Game::Spark => 0xff7a2f,
            Game::Commits => 0x22b36b,
            Game::Merge => 0xec4f9b,
            Game::Stack => 0x7b5cff,
        }
    }

    /// Its colour, deep enough to carry white text.
    fn deep(self) -> u32 {
        match self {
            Game::Spark => 0xc94b14,
            Game::Commits => 0x13804a,
            Game::Merge => 0xc0367a,
            Game::Stack => 0x5b3fdc,
        }
    }

    /// Its colour as text on the sheet: deep on light themes, bright on dark ones.
    fn ink(self) -> u32 {
        if is_dark() {
            lighten(self.hue(), 0.75)
        } else {
            self.deep()
        }
    }

    fn glyph(self) -> IconName {
        match self {
            Game::Spark => IconName::Zap,
            Game::Commits => IconName::GitCommitHorizontal,
            Game::Merge => IconName::LayoutGrid,
            Game::Stack => IconName::Layers,
        }
    }

    /// The keys of its hint, drawn as keycaps, each with what it does.
    fn keys(self) -> &'static [(&'static [&'static str], &'static str)] {
        match self {
            Game::Spark => &[(&["Space"], "or click to lift")],
            Game::Commits => &[(&["←", "↑", "↓", "→"], "steer")],
            Game::Merge => &[(&["←", "↑", "↓", "→"], "slide")],
            Game::Stack => &[
                (&["←", "→"], "move"),
                (&["↑"], "turn"),
                (&["Space"], "drop"),
            ],
        }
    }
}

// The effects: particles, floating score pops, a shake on a crash, a squash on a lift,
// a glow over cleared rows, and counters that tick up to the score. None of them touch
// the games themselves.

/// How long the spark squashes after a lift, and the snake's head swells after a commit.
const SQUASH: f32 = 0.14;
const SHAKE: f32 = 0.45;
const FLASH: f32 = 0.42;
const CHEER: f32 = 1.6;
const POP: f32 = 0.85;
const BIG_POP: f32 = 1.4;
/// At most this many particles at once, so a busy moment can't slow the frame.
const MAX_PARTS: usize = 180;

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    span: f32,
    size: f32,
    color: u32,
    /// Confetti: a little square instead of a round spark.
    square: bool,
}

struct Pop {
    x: f32,
    y: f32,
    text: SharedString,
    color: u32,
    age: f32,
    /// An announcement (NEW BEST!, a multi-line clear), larger and longer.
    big: bool,
}

#[derive(Default)]
struct Fx {
    parts: Vec<Particle>,
    pops: Vec<Pop>,
    shake: f32,
    squash: f32,
    flash: f32,
    flash_rows: Vec<usize>,
    /// Seconds left of the NEW BEST glow on the Best counter.
    cheer: f32,
    /// The score counter's bump when points land.
    bump: f32,
    shown: f32,
    shown_best: f32,
    time: f32,
    last: Option<Instant>,
}

impl Fx {
    fn clear(&mut self) {
        let time = self.time;
        *self = Fx {
            time,
            ..Fx::default()
        };
    }

    fn busy(&self, score: u32, best: u32) -> bool {
        !self.parts.is_empty()
            || !self.pops.is_empty()
            || self.shake > 0.
            || self.squash > 0.
            || self.flash > 0.
            || self.cheer > 0.
            || self.bump > 0.
            || self.shown != score as f32
            || self.shown_best != best as f32
    }

    fn step(&mut self, dt: f32, score: u32, best: u32, calm: bool) {
        self.time += dt;
        for p in &mut self.parts {
            p.vy += 520. * dt;
            p.vx *= (1. - 1.8 * dt).max(0.);
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.life -= dt;
        }
        self.parts.retain(|p| p.life > 0.);
        for p in &mut self.pops {
            p.age += dt;
        }
        self.pops
            .retain(|p| p.age < if p.big { BIG_POP } else { POP });
        for t in [
            &mut self.shake,
            &mut self.squash,
            &mut self.flash,
            &mut self.cheer,
        ] {
            *t = (*t - dt).max(0.);
        }
        self.bump = (self.bump - dt * 5.).max(0.);
        // The counters run up to the score, quicker the further behind, and drop at once
        // when a round starts over.
        let tick = |shown: &mut f32, to: u32| {
            let to = to as f32;
            if calm || to < *shown {
                *shown = to;
            } else if *shown < to {
                *shown = (*shown + ((to - *shown) * 9.).max(14.) * dt).min(to);
            }
        };
        tick(&mut self.shown, score);
        tick(&mut self.shown_best, best);
    }
}

/// The state an action starts from, to tell afterwards what it did.
struct Before {
    game: Game,
    phase: Phase,
    score: u32,
    record: bool,
    best: u32,
    food: (i32, i32),
    head: (i32, i32),
    lines: u32,
    rounds: u32,
}

// Where things sit in the field, for effects to start from.

const M_SIDE: f32 = 270.;
const M_GAP: f32 = 8.;

fn merge_tile() -> f32 {
    (M_SIDE - M_GAP * 5.) / 4.
}

fn merge_origin() -> (f32, f32) {
    ((W - M_SIDE) / 2., (H - M_SIDE) / 2.)
}

fn merge_centre((r, c): (usize, usize)) -> (f32, f32) {
    let (ox, oy) = merge_origin();
    let t = merge_tile();
    (
        ox + M_GAP + c as f32 * (t + M_GAP) + t / 2.,
        oy + M_GAP + r as f32 * (t + M_GAP) + t / 2.,
    )
}

fn well_origin() -> (f32, f32) {
    (
        (W - WELL_W as f32 * BLOCK) / 2. - 40.,
        (H - WELL_H as f32 * BLOCK) / 2.,
    )
}

fn cell_centre((c, r): (i32, i32)) -> (f32, f32) {
    (c as f32 * CELL + CELL / 2., r as f32 * CELL + CELL / 2.)
}

/// The tile colours of 2048, light to deep as the value climbs, with text that reads.
fn tile_colors(v: u32) -> (u32, u32) {
    let ink = 0x5a3412;
    match v {
        2 => (0xfff1c9, ink),
        4 => (0xffdf94, ink),
        8 => (0xffbd6b, ink),
        16 => (0xff9a5c, ink),
        32 => (0xf2557a, 0xffffff),
        64 => (0xd2409f, 0xffffff),
        128 => (0x8e55f0, 0xffffff),
        256 => (0x5470e8, 0xffffff),
        512 => (0x1f8fd0, 0xffffff),
        1024 => (0x139a76, 0xffffff),
        2048 => (0x4f9a24, 0xffffff),
        _ => (0x2b2d42, GOLD),
    }
}

/// A key drawn as a small keycap.
fn keycap(label: &str) -> Div {
    div()
        .flex_none()
        .h(px(19.))
        .min_w(px(19.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(grad(180., rgb(lighten(c_raised(), 0.92)), rgb(c_raised())))
        .border_1()
        .border_b_2()
        .border_color(rgb(mix(c_border(), c_text(), 0.8)))
        .shadow(vec![soft(
            0x000000,
            if is_dark() { 0.35 } else { 0.1 },
            2.,
            1.,
        )])
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(c_text()))
        .child(label.to_string())
}

/// Keycaps followed by what they do.
fn key_hint(keys: &[&str], what: &str) -> Div {
    keys.iter()
        .fold(div().flex().items_center().gap(px(3.)), |row, k| {
            row.child(keycap(k))
        })
        .child(div().ml(px(3.)).child(what.to_string()))
}

impl Arcade {
    fn before(&self) -> Before {
        Before {
            game: self.game,
            phase: self.phase,
            score: self.score,
            record: self.record,
            best: self.best(),
            food: self.commits.food,
            head: self.commits.body[0],
            lines: self.stack.lines,
            rounds: self.rounds,
        }
    }

    /// Plays what an action did: points pop, clears burst, a crash shakes.
    fn react(&mut self, b: Before) {
        if b.game != self.game || b.rounds != self.rounds {
            return;
        }
        let gained = self.score.saturating_sub(b.score);
        if gained > 0 {
            self.fx.bump = 1.;
        }
        let hue = self.game.hue();
        match self.game {
            Game::Spark if gained > 0 => {
                let y = self.spark.y;
                self.pop(SPARK_X + 16., y - 22., format!("+{gained}"), hue, false);
                self.burst(SPARK_X, y, &[GOLD, 0xffffff, hue], 10, 150.);
            }
            Game::Commits if self.commits.food != b.food && gained > 0 => {
                let (x, y) = cell_centre(b.food);
                self.pop(x, y - 16., format!("+{gained}"), hue, false);
                self.burst(x, y, &[GOLD, 0x7ef2a8, hue], 14, 170.);
                if !self.calm {
                    self.fx.squash = SQUASH;
                }
            }
            Game::Merge if gained > 0 => {
                let merged = self.merge.merged.clone();
                for &at in &merged {
                    let (x, y) = merge_centre(at);
                    let v = self.merge.cells[at.0][at.1];
                    self.burst(x, y, &[tile_colors(v).0, GOLD, 0xffffff], 8, 150.);
                }
                let top = merged
                    .iter()
                    .copied()
                    .max_by_key(|&(r, c)| self.merge.cells[r][c])
                    .unwrap_or((0, 0));
                let (x, y) = merge_centre(top);
                self.pop(x, y - 24., format!("+{gained}"), hue, false);
            }
            Game::Stack if self.stack.lines > b.lines => {
                let rows = self.stack.cleared.clone();
                let (ox, oy) = well_origin();
                let ww = WELL_W as f32 * BLOCK;
                for &r in &rows {
                    let y = oy + r as f32 * BLOCK + BLOCK / 2.;
                    for k in 0..5 {
                        let x = ox + ww * (k as f32 + 0.5) / 5.;
                        self.burst(x, y, &[0xffffff, 0x22d3ee, 0xfacc15, 0xa855f7], 3, 170.);
                    }
                }
                self.fx.flash_rows = rows;
                self.fx.flash = FLASH;
                let cheer = match self.stack.lines - b.lines {
                    1 => None,
                    2 => Some("Double!"),
                    3 => Some("Triple!"),
                    _ => Some("STACK!"),
                };
                let (x, y) = (ox + ww / 2., oy + 60.);
                self.pop(x, y + 26., format!("+{gained}"), hue, false);
                if let Some(word) = cheer {
                    self.pop(x, y, word, GOLD, true);
                }
            }
            _ => {}
        }
        // Beating the record mid-round: a cheer, once.
        if !b.record && self.record && b.best > 0 {
            self.cheer();
        }
        if b.phase != Phase::Over && self.phase == Phase::Over {
            let (x, y) = match self.game {
                Game::Spark => (SPARK_X, self.spark.y),
                Game::Commits => cell_centre(b.head),
                Game::Merge => (W / 2., H / 2.),
                Game::Stack => {
                    let (ox, oy) = well_origin();
                    (ox + WELL_W as f32 * BLOCK / 2., oy + 16.)
                }
            };
            self.burst(x, y, &[c_red(), 0xff9a3d, GOLD, 0xffffff], 34, 260.);
            if !self.calm {
                self.fx.shake = SHAKE;
            }
            if self.record {
                self.cheer();
            }
        }
    }

    fn cheer(&mut self) {
        self.fx.cheer = CHEER;
        self.pop(W / 2., 54., "NEW BEST!", GOLD, true);
        let hue = self.game.hue();
        for x in [W * 0.25, W / 2., W * 0.75] {
            self.burst(x, 40., &[GOLD, hue, 0xffffff, 0x22d3ee, 0xec4f9b], 16, 280.);
        }
    }

    fn burst(&mut self, x: f32, y: f32, colors: &[u32], n: usize, speed: f32) {
        if self.calm {
            return;
        }
        let r = &mut self.fx_rng;
        for i in 0..n {
            let a = r.unit() * std::f32::consts::TAU;
            let s = speed * (0.35 + 0.65 * r.unit());
            let span = 0.45 + 0.45 * r.unit();
            self.fx.parts.push(Particle {
                x,
                y,
                vx: a.cos() * s,
                vy: a.sin() * s - speed * 0.35,
                life: span,
                span,
                size: 3. + 3.5 * r.unit(),
                color: colors[i % colors.len().max(1)],
                square: i % 3 == 0,
            });
        }
        let over = self.fx.parts.len().saturating_sub(MAX_PARTS);
        self.fx.parts.drain(..over);
    }

    fn pop(&mut self, x: f32, y: f32, text: impl Into<SharedString>, color: u32, big: bool) {
        // A newer announcement replaces an older one, so they never stack up unread.
        if big {
            self.fx.pops.retain(|p| !p.big);
        }
        self.fx.pops.push(Pop {
            x,
            y,
            text: text.into(),
            color,
            age: 0.,
            big,
        });
    }

    /// Moves the effects on to now.
    fn advance_fx(&mut self) {
        let (score, best) = (self.score, self.best());
        // A running round keeps the clock going too, for the commit's breathing.
        let running = self.phase == Phase::Playing && self.game != Game::Merge;
        if !running && !self.fx.busy(score, best) {
            self.fx.last = None;
            return;
        }
        let now = Instant::now();
        let dt = self.fx.last.map_or(1. / 60., |t| {
            now.duration_since(t).as_secs_f32().min(1. / 20.)
        });
        self.fx.last = Some(now);
        self.fx.step(dt, score, best, self.calm);
    }

    /// Particles and pops, over the field.
    fn fx_layer(&self) -> Div {
        let mut layer = div().absolute().inset_0();
        for p in &self.fx.parts {
            let t = (p.life / p.span).clamp(0., 1.);
            let s = p.size * (0.5 + 0.5 * t);
            let el = div()
                .absolute()
                .left(px(p.x - s / 2.))
                .top(px(p.y - s / 2.))
                .w(px(s))
                .h(px(if p.square { s * 0.7 } else { s }))
                .bg(alpha(p.color, t.min(1.) * 0.95));
            layer = layer.child(if p.square {
                el.rounded(px(1.5))
            } else {
                el.rounded_full()
            });
        }
        for p in &self.fx.pops {
            let life = if p.big { BIG_POP } else { POP };
            let t = p.age / life;
            let rise = if self.calm {
                0.
            } else {
                (1. - (1. - t).powi(3)) * 30.
            };
            let fade = if t > 0.6 { 1. - (t - 0.6) / 0.4 } else { 1. };
            // Grows in with a little overshoot, then holds.
            let grow = if self.calm {
                1.
            } else {
                back_out((p.age / 0.22).min(1.))
            };
            let size = (if p.big { 20. } else { 14. }) * (0.6 + 0.4 * grow);
            let (bg, fg) = if p.big {
                (
                    grad(180., rgb(lighten(p.color, 0.6)), rgb(p.color)),
                    0x4a2a00,
                )
            } else {
                (
                    grad(180., rgb(lighten(p.color, 0.8)), rgb(darken(p.color, 0.82))),
                    0xffffff,
                )
            };
            layer = layer.child(
                div()
                    .absolute()
                    .left(px(p.x - 80.))
                    .w(px(160.))
                    .top(px(p.y - rise - 12.))
                    .flex()
                    .justify_center()
                    .opacity(fade)
                    .child(
                        div()
                            .px(px(if p.big { 12. } else { 7. }))
                            .py(px(1.))
                            .rounded_full()
                            .bg(bg)
                            .border_1()
                            .border_color(alpha(0xffffff, 0.7))
                            .shadow(vec![soft(0x000000, 0.25, 6., 2.)])
                            .text_size(px(size))
                            .font_weight(FontWeight::BLACK)
                            .text_color(rgb(fg))
                            .whitespace_nowrap()
                            .child(p.text.clone()),
                    ),
            );
        }
        layer
    }

    /// Spark flies through a dusk (or daytime) sky over rolling hills.
    fn spark_field(&self) -> Div {
        let dark = is_dark();
        let scroll = self.spark.scroll;
        let (top, bottom) = if dark {
            (0x16123d, 0x4a2f6e)
        } else {
            (0x7fcbff, 0xffe0c2)
        };
        let mut field = div().bg(grad(180., rgb(top), rgb(bottom)));
        // The moon, or the sun.
        let orb = if dark { 0xfff0c2 } else { 0xfff4c4 };
        field = field.child(
            div()
                .absolute()
                .left(px(356.))
                .top(px(26.))
                .size(px(44.))
                .rounded_full()
                .bg(grad(135., rgb(0xffffff), rgb(orb)))
                .shadow(glow(if dark { 0xffe6a0 } else { 0xffc24a }, 0.7, 36.)),
        );
        // Stars at night, clouds by day, drifting slower than the gates.
        for i in 0..10u32 {
            let y = 16. + (i * 41 % 150) as f32;
            let x = ((i * 157 % 520) as f32 - scroll * (0.08 + 0.02 * (i % 3) as f32))
                .rem_euclid(W + 80.)
                - 40.;
            field = field.child(if dark {
                let s = 2. + (i % 3) as f32;
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .size(px(s))
                    .rounded_full()
                    .bg(alpha(0xffffff, 0.55 + 0.15 * (i % 3) as f32))
                    .shadow(glow(0xffffff, 0.5, 4.))
            } else if i % 2 == 0 {
                let w = 46. + (i % 4) as f32 * 8.;
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(16.))
                    .rounded_full()
                    .bg(alpha(0xffffff, 0.85))
                    .child(
                        div()
                            .absolute()
                            .left(px(w * 0.25))
                            .top(px(-9.))
                            .size(px(22.))
                            .rounded_full()
                            .bg(alpha(0xffffff, 0.85)),
                    )
            } else {
                div()
            });
        }
        // Two rows of hills, the nearer one quicker.
        let hills = if dark {
            [
                (0.2, 0x2b2258, H - 66., 200.),
                (0.4, 0x33286a, H - 40., 150.),
            ]
        } else {
            [
                (0.2, 0xb6e3a8, H - 66., 200.),
                (0.4, 0x8fd481, H - 40., 150.),
            ]
        };
        for (speed, color, top, d) in hills {
            let step = d * 0.78;
            let mut x = -(scroll * speed).rem_euclid(step) - d * 0.3;
            while x < W {
                field = field.child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(top))
                        .size(px(d))
                        .rounded_full()
                        .bg(rgb(color)),
                );
                x += step;
            }
        }
        // The gates: green pillars with a lip at the gap, lit from the left.
        let (light, base, edge) = (0x7ff09c, 0x2fbf5f, 0x1b8a43);
        for g in &self.spark.gates {
            for (top, h, lip) in [
                (-12., g.gap + 12., g.gap - 14.),
                (g.gap + GAP, H - g.gap - GAP + 12., g.gap + GAP),
            ] {
                field = field
                    .child(
                        div()
                            .absolute()
                            .left(px(g.x + 3.))
                            .top(px(top))
                            .w(px(GATE_W - 6.))
                            .h(px(h))
                            .rounded(px(6.))
                            .bg(grad(90., rgb(light), rgb(edge)))
                            .border_1()
                            .border_color(rgb(darken(edge, 0.8)))
                            .child(
                                div()
                                    .absolute()
                                    .left(px(7.))
                                    .top_0()
                                    .bottom_0()
                                    .w(px(5.))
                                    .rounded_full()
                                    .bg(alpha(0xffffff, 0.35)),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(g.x))
                            .top(px(lip))
                            .w(px(GATE_W))
                            .h(px(14.))
                            .rounded(px(5.))
                            .bg(grad(90., rgb(lighten(light, 0.7)), rgb(base)))
                            .border_1()
                            .border_color(rgb(darken(edge, 0.8)))
                            .shadow(vec![soft(0x000000, 0.25, 6., 3.)])
                            .child(
                                div()
                                    .absolute()
                                    .left(px(7.))
                                    .top(px(2.))
                                    .w(px(GATE_W - 14.))
                                    .h(px(3.))
                                    .rounded_full()
                                    .bg(alpha(0xffffff, 0.45)),
                            ),
                    );
            }
        }
        // A glowing trail behind the spark.
        let tilt = (self.spark.vy / 600.).clamp(-1., 1.);
        for (i, a) in [0.42, 0.26, 0.14, 0.06].into_iter().enumerate() {
            let back = (i as f32 + 1.) * 8.;
            let r = SPARK_R * (1. - 0.16 * (i as f32 + 1.));
            field = field.child(
                div()
                    .absolute()
                    .left(px(SPARK_X - back - r))
                    .top(px(self.spark.y - tilt * back * 0.5 - r))
                    .size(px(r * 2.))
                    .rounded_full()
                    .bg(alpha(mix(GOLD, 0xff6a2b, 1. - i as f32 * 0.25), a)),
            );
        }
        // The spark stretches as it falls and squashes as it lifts.
        let (mut sx, mut sy) = (1., 1.);
        if !self.calm {
            let stretch = (self.spark.vy.abs() / 900.).min(0.18);
            sx -= stretch * 0.6;
            sy += stretch;
            let q = self.fx.squash / SQUASH;
            sx += 0.3 * q;
            sy -= 0.25 * q;
        }
        let (w, h) = (SPARK_R * 2. * sx, SPARK_R * 2. * sy);
        let eye = |dx: f32| {
            div()
                .absolute()
                .left(px(w / 2. + dx))
                .top(px(h * 0.32))
                .w(px(2.5))
                .h(px(3.5))
                .rounded_full()
                .bg(rgb(0x3a1a08))
        };
        field.child(
            div()
                .absolute()
                .left(px(SPARK_X - w / 2.))
                .top(px(self.spark.y - h / 2.))
                .w(px(w))
                .h(px(h))
                .rounded_full()
                .bg(grad(150., rgb(0xffe08a), rgb(0xff5a1f)))
                .border_1()
                .border_color(rgb(0xd9440f))
                .shadow(vec![
                    soft(0xff7a2f, 0.75, 18., 0.),
                    soft(0x000000, 0.25, 4., 2.),
                ])
                .child(
                    div()
                        .absolute()
                        .left(px(w * 0.18))
                        .top(px(h * 0.12))
                        .w(px(w * 0.34))
                        .h(px(h * 0.24))
                        .rounded_full()
                        .bg(alpha(0xffffff, 0.8)),
                )
                .child(eye(0.5))
                .child(eye(4.5)),
        )
    }

    /// Commits: a snake in a moonlit (or sunny) garden, a glowing commit to chase.
    fn commits_field(&self) -> Div {
        let dark = is_dark();
        let (top, bottom, check) = if dark {
            (0x0d2b29, 0x113d31, alpha(0xffffff, 0.035))
        } else {
            (0xe6f9ee, 0xcdf0dc, alpha(0x0f6b3c, 0.06))
        };
        let mut field = div().bg(grad(180., rgb(top), rgb(bottom)));
        // A checkered lawn.
        for r in 0..ROWS {
            for c in 0..COLS {
                if (r + c) % 2 == 0 {
                    field = field.child(
                        div()
                            .absolute()
                            .left(px(c as f32 * CELL))
                            .top(px(r as f32 * CELL))
                            .size(px(CELL))
                            .bg(check),
                    );
                }
            }
        }
        // The commit: a gem that breathes while the round runs.
        let (fx, fy) = cell_centre(self.commits.food);
        let pulse = if self.calm {
            0.
        } else {
            (self.fx.time * 5.).sin() * 0.5 + 0.5
        };
        let s = CELL - 5. + pulse * 2.;
        field = field.child(
            div()
                .absolute()
                .left(px(fx - s / 2.))
                .top(px(fy - s / 2.))
                .size(px(s))
                .rounded_full()
                .bg(grad(150., rgb(0xffe27a), rgb(0xff8a3d)))
                .border_2()
                .border_color(rgb(0xffffff))
                .shadow(glow(0xffb03d, 0.55 + 0.3 * pulse, 14.))
                .child(
                    div()
                        .absolute()
                        .left(px(s * 0.2))
                        .top(px(s * 0.14))
                        .size(px(s * 0.28))
                        .rounded_full()
                        .bg(alpha(0xffffff, 0.85)),
                ),
        );
        // The snake: glossy green segments joined into one body, a head with eyes.
        let body: Vec<(i32, i32)> = self.commits.body.iter().copied().collect();
        let n = body.len().max(2) as f32 - 1.;
        let shade = |i: usize| mix(0x1aa38a, 0x5ef08f, i as f32 / n);
        for (i, pair) in body.windows(2).enumerate().rev() {
            let ((x1, y1), (x2, y2)) = (cell_centre(pair[0]), cell_centre(pair[1]));
            const THICK: f32 = CELL - 6.;
            field = field.child(
                div()
                    .absolute()
                    .left(px(x1.min(x2) - THICK / 2.))
                    .top(px(y1.min(y2) - THICK / 2.))
                    .w(px((x1 - x2).abs() + THICK))
                    .h(px((y1 - y2).abs() + THICK))
                    .rounded(px(6.))
                    .bg(rgb(shade(i))),
            );
        }
        for (i, &cell) in body.iter().enumerate().rev() {
            let (x, y) = cell_centre(cell);
            let color = shade(i);
            if i == 0 {
                let swell = if self.calm {
                    0.
                } else {
                    4. * self.fx.squash / SQUASH
                };
                let s = CELL + swell;
                let (dx, dy) = (self.commits.dir.0 as f32, self.commits.dir.1 as f32);
                // Eyes sit forward, either side of the way it faces.
                let eye = |side: f32| {
                    let ex = s / 2. + dx * 3. - dy * side * 4.5;
                    let ey = s / 2. + dy * 3. + dx * side * 4.5;
                    div()
                        .absolute()
                        .left(px(ex - 3.))
                        .top(px(ey - 3.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(rgb(0xffffff))
                        .child(
                            div()
                                .absolute()
                                .left(px(1.5 + dx * 1.2))
                                .top(px(1.5 + dy * 1.2))
                                .size(px(3.))
                                .rounded_full()
                                .bg(rgb(0x10261b)),
                        )
                };
                field = field.child(
                    div()
                        .absolute()
                        .left(px(x - s / 2.))
                        .top(px(y - s / 2.))
                        .size(px(s))
                        .rounded(px(7.))
                        .bg(grad(150., rgb(lighten(color, 0.6)), rgb(color)))
                        .border_1()
                        .border_color(rgb(darken(color, 0.7)))
                        .shadow(vec![
                            soft(0x2fe08a, 0.6, 12., 0.),
                            soft(0x000000, 0.25, 4., 2.),
                        ])
                        .child(eye(-1.))
                        .child(eye(1.)),
                );
            } else {
                let s = CELL - 4.;
                field = field.child(
                    div()
                        .absolute()
                        .left(px(x - s / 2.))
                        .top(px(y - s / 2.))
                        .size(px(s))
                        .rounded(px(6.))
                        .bg(grad(150., rgb(lighten(color, 0.75)), rgb(color)))
                        .border_1()
                        .border_color(rgb(darken(color, 0.75)))
                        .child(
                            div()
                                .absolute()
                                .left(px(3.))
                                .top(px(2.))
                                .w(px(s * 0.4))
                                .h(px(3.))
                                .rounded_full()
                                .bg(alpha(0xffffff, 0.4)),
                        ),
                );
            }
        }
        field
    }

    /// Merge: candy tiles on a chunky board, over a warm glow.
    fn merge_field(&self) -> Div {
        let dark = is_dark();
        let (top, bottom) = if dark {
            (0x2a1840, 0x431f3f)
        } else {
            (0xfff1de, 0xffdcea)
        };
        let mut field = div().bg(grad(160., rgb(top), rgb(bottom)));
        // Soft lights behind the board.
        for (x, y, d, c) in [
            (20., 30., 90., 0xff9a5c),
            (350., 180., 110., 0xec4f9b),
            (40., 210., 70., 0x8e55f0),
        ] {
            field = field.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .size(px(d))
                    .rounded_full()
                    .bg(alpha(c, if dark { 0.16 } else { 0.2 }))
                    .shadow(glow(c, 0.25, 40.)),
            );
        }
        let tile = merge_tile();
        let (ox, oy) = merge_origin();
        let frame = if dark { 0x24142f } else { 0xc98f6a };
        let mut board = div()
            .absolute()
            .left(px(ox))
            .top(px(oy))
            .size(px(M_SIDE))
            .rounded(px(18.))
            .bg(alpha(frame, if dark { 0.85 } else { 0.55 }))
            .border_b_4()
            .border_color(alpha(darken(frame, 0.7), if dark { 0.9 } else { 0.6 }))
            .shadow(vec![soft(0x000000, if dark { 0.4 } else { 0.18 }, 18., 6.)]);
        for r in 0..4 {
            for c in 0..4 {
                let v = self.merge.cells[r][c];
                let left = M_GAP + c as f32 * (tile + M_GAP);
                let top = M_GAP + r as f32 * (tile + M_GAP);
                if v == 0 {
                    board = board.child(
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .size(px(tile))
                            .rounded(px(12.))
                            .bg(alpha(0xffffff, if dark { 0.06 } else { 0.28 })),
                    );
                    continue;
                }
                let (bg, fg) = tile_colors(v);
                let size = match v {
                    v if v >= 1024 => 20.,
                    v if v >= 128 => 24.,
                    _ => 28.,
                };
                let el = div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .size(px(tile))
                    .rounded(px(12.))
                    .bg(grad(180., rgb(lighten(bg, 0.7)), rgb(bg)))
                    .border_b_4()
                    .border_color(rgb(darken(bg, 0.74)))
                    .shadow(if v >= 32 {
                        vec![soft(bg, 0.55, 16., 0.), soft(0x000000, 0.2, 4., 3.)]
                    } else {
                        vec![soft(0x000000, 0.18, 4., 3.)]
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(size))
                    .font_weight(FontWeight::BLACK)
                    .text_color(rgb(fg))
                    .child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .top(px(4.))
                            .w(px(tile - 16.))
                            .h(px(6.))
                            .rounded_full()
                            .bg(alpha(0xffffff, 0.35)),
                    )
                    .child(v.to_string());
                let fresh = self.merge.fresh == Some((r, c));
                let bumped = self.merge.merged.contains(&(r, c));
                let moves = self.merge.moves;
                board = board.child(if (fresh || bumped) && !self.calm {
                    let id = format!("merge-{moves}-{r}-{c}");
                    let ms = if fresh { 200 } else { 240 };
                    el.animated(
                        ElementId::Name(id.into()),
                        Animation::new(Duration::from_millis(ms)),
                        move |el, t| {
                            // A new tile grows in; a merged one swells and settles.
                            let grow = if fresh {
                                -tile * 0.5 * (1. - back_out(t))
                            } else {
                                7. * (t * std::f32::consts::PI).sin()
                            };
                            el.left(px(left - grow / 2.))
                                .top(px(top - grow / 2.))
                                .size(px(tile + grow))
                                .opacity(if fresh { t.min(1.) } else { 1. })
                        },
                    )
                } else {
                    el.into_any_element()
                });
            }
        }
        field.child(board)
    }

    /// One block of a tetromino, glossy, `left`/`top` in pixels within its parent.
    fn block(left: f32, top: f32, color: u32) -> Div {
        div()
            .absolute()
            .left(px(left + 0.5))
            .top(px(top + 0.5))
            .size(px(BLOCK - 1.))
            .rounded(px(3.))
            .bg(grad(150., rgb(lighten(color, 0.55)), rgb(color)))
            .border_1()
            .border_b_2()
            .border_color(rgb(darken(color, 0.65)))
            .child(
                div()
                    .absolute()
                    .left(px(2.))
                    .top(px(1.5))
                    .w(px(BLOCK - 8.))
                    .h(px(2.5))
                    .rounded_full()
                    .bg(alpha(0xffffff, 0.55)),
            )
    }

    /// Stack: classic pieces in a glass well, under a night sky.
    fn stack_field(&self) -> Div {
        let dark = is_dark();
        let (top, bottom) = if dark {
            (0x0e1230, 0x231a52)
        } else {
            (0xe3e8ff, 0xf1e4ff)
        };
        let (ww, wh) = (WELL_W as f32 * BLOCK, WELL_H as f32 * BLOCK);
        let (ox, oy) = well_origin();
        let glass = 0x0a0c22;
        let mut well = div()
            .absolute()
            .left(px(ox))
            .top(px(oy))
            .w(px(ww))
            .h(px(wh))
            .rounded(px(6.))
            .bg(grad(180., rgb(glass), rgb(0x151a3d)))
            .shadow(vec![
                soft(0x7b5cff, if dark { 0.45 } else { 0.3 }, 18., 0.),
                soft(0x000000, 0.3, 8., 4.),
            ]);
        for c in 1..WELL_W {
            well = well.child(
                div()
                    .absolute()
                    .left(px(c as f32 * BLOCK))
                    .top_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(alpha(0xffffff, 0.04)),
            );
        }
        for (r, row) in self.stack.well.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if let Some(kind) = cell {
                    well = well.child(Self::block(
                        c as f32 * BLOCK,
                        r as f32 * BLOCK,
                        kind.color(),
                    ));
                }
            }
        }
        let color = self.stack.piece.kind.color();
        let ghost = self.stack.ghost();
        if ghost != self.stack.piece {
            for (c, r) in ghost.cells().into_iter().filter(|&(_, r)| r >= 0) {
                well = well.child(
                    div()
                        .absolute()
                        .left(px(c as f32 * BLOCK + 1.))
                        .top(px(r as f32 * BLOCK + 1.))
                        .size(px(BLOCK - 2.))
                        .rounded(px(3.))
                        .bg(alpha(color, 0.16))
                        .border_1()
                        .border_color(alpha(color, 0.7)),
                );
            }
        }
        for (c, r) in self
            .stack
            .piece
            .cells()
            .into_iter()
            .filter(|&(_, r)| r >= 0)
        {
            well = well.child(
                Self::block(c as f32 * BLOCK, r as f32 * BLOCK, color)
                    .shadow(glow(color, 0.55, 10.)),
            );
        }
        // Rows that just cleared glow white and fade.
        if self.fx.flash > 0. {
            let t = self.fx.flash / FLASH;
            for &r in &self.fx.flash_rows {
                well = well.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(r as f32 * BLOCK - 2.))
                        .w(px(ww))
                        .h(px(BLOCK + 4.))
                        .rounded(px(4.))
                        .bg(alpha(0xffffff, 0.85 * t))
                        .shadow(glow(0x22d3ee, 0.9 * t, 18.)),
                );
            }
        }
        // Beside the well: the next piece, then the level and lines.
        let next = self.stack.next;
        let box_side = 4. * BLOCK + 8.;
        // Centre the piece's own cells in the box, not the box it turns in.
        let cells = next.cells();
        let span = |f: fn(&(i32, i32)) -> i32| {
            let lo = cells.iter().map(f).min().unwrap_or(0);
            let hi = cells.iter().map(f).max().unwrap_or(0);
            (box_side - (hi - lo + 1) as f32 * BLOCK) / 2. - lo as f32 * BLOCK
        };
        let (pad_x, pad_y) = (span(|p| p.0), span(|p| p.1));
        let preview = cells.into_iter().fold(
            div()
                .relative()
                .size(px(box_side))
                .rounded(px(10.))
                .bg(grad(180., rgb(glass), rgb(0x151a3d)))
                .border_1()
                .border_color(alpha(next.color(), 0.5))
                .shadow(glow(next.color(), 0.35, 12.)),
            |el, (c, r)| {
                el.child(Self::block(
                    pad_x + c as f32 * BLOCK,
                    pad_y + r as f32 * BLOCK,
                    next.color(),
                ))
            },
        );
        let label = |text: &'static str| {
            div()
                .text_size(px(10.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if dark { 0xb8b4e8 } else { 0x4b3f9e }))
                .child(text)
        };
        let stat = |text: &'static str, value: String| {
            div()
                .v_flex()
                .w(px(box_side))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(10.))
                .bg(alpha(
                    if dark { 0xffffff } else { 0x5b3fdc },
                    if dark { 0.07 } else { 0.1 },
                ))
                .border_b_2()
                .border_color(alpha(0x5b3fdc, 0.3))
                .child(label(text))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::BLACK)
                        .text_color(rgb(c_text()))
                        .child(value),
                )
        };
        let side = div()
            .absolute()
            .left(px(ox + ww + 22.))
            .top(px(oy))
            .v_flex()
            .gap(px(10.))
            .child(
                div()
                    .v_flex()
                    .gap(px(4.))
                    .child(label("NEXT"))
                    .child(preview),
            )
            .child(stat("LEVEL", self.stack.level().to_string()))
            .child(stat("LINES", self.stack.lines.to_string()));
        div()
            .bg(grad(180., rgb(top), rgb(bottom)))
            .child(well)
            .child(side)
    }

    /// A line that bobs gently, for the prompt on a card.
    fn bobbing(&self, el: Div, id: String) -> AnyElement {
        if self.calm {
            return el.into_any_element();
        }
        el.relative().animated(
            ElementId::Name(id.into()),
            Animation::new(Duration::from_millis(1100)).repeat(),
            |el, t| el.top(px(-3.5 * (t * std::f32::consts::TAU).sin().abs())),
        )
    }

    /// The card over the field before a round, when paused and after a crash.
    fn banner(&self) -> Option<AnyElement> {
        if self.phase == Phase::Playing {
            return None;
        }
        let game = self.game;
        let prompt = |what: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_secondary()))
                .child(keycap("Space"))
                .child(what.to_string())
        };
        let badge = |name: IconName| {
            div()
                .size(px(44.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(14.))
                .bg(grad(150., rgb(lighten(game.hue(), 0.7)), rgb(game.deep())))
                .border_b_4()
                .border_color(rgb(darken(game.deep(), 0.7)))
                .shadow(glow(game.hue(), 0.5, 16.))
                .child(icon(name, 22., 0xffffff))
        };
        let title = |text: String, size: f32| {
            div()
                .text_size(px(size))
                .font_weight(FontWeight::BLACK)
                .text_color(rgb(c_text()))
                .child(text)
        };
        let mut card = div().v_flex().items_center().gap(px(8.));
        let id = format!("arcade-card-{:?}-{}", self.phase, self.rounds);
        card = match self.phase {
            Phase::Playing => return None,
            Phase::Ready => card
                .child(badge(game.glyph()))
                .child(title(game.name().to_string(), 24.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(c_muted()))
                        .child(game.hint()),
                )
                .child(self.bobbing(prompt("to start"), format!("{id}-bob"))),
            Phase::Paused => card
                .child(badge(IconName::Pause))
                .child(title("Paused".into(), 24.))
                .child(self.bobbing(prompt("to carry on"), format!("{id}-bob"))),
            Phase::Over => {
                let stat = |label: &'static str, value: u32, color: u32| {
                    div()
                        .v_flex()
                        .items_center()
                        .min_w(px(84.))
                        .px(px(12.))
                        .py(px(6.))
                        .rounded(px(12.))
                        .bg(tint(c_subtle()))
                        .border_b_2()
                        .border_color(divider())
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(c_muted()))
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(26.))
                                .font_weight(FontWeight::BLACK)
                                .text_color(rgb(color))
                                .child(value.to_string()),
                        )
                };
                let best_ink = if is_dark() { GOLD } else { 0x9a5b00 };
                let card = card.child(title("Game over!".into(), 30.)).child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .child(stat("SCORE", self.score, c_text()))
                        .child(stat(
                            "BEST",
                            self.best(),
                            if self.record { best_ink } else { c_text() },
                        )),
                );
                let card = if self.record {
                    card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(12.))
                            .h(px(26.))
                            .rounded_full()
                            .bg(grad(180., rgb(0xffe48a), rgb(GOLD)))
                            .border_b_2()
                            .border_color(rgb(0xc98a00))
                            .shadow(glow(GOLD, 0.6, 14.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::BLACK)
                            .text_color(rgb(0x4a2a00))
                            .child(icon(IconName::Trophy, 13., 0x4a2a00))
                            .child("NEW BEST!"),
                    )
                } else {
                    card
                };
                card.child(self.bobbing(prompt("to play again"), format!("{id}-bob")))
            }
        };
        let hue = game.hue();
        let panel = div()
            .p(px(3.))
            .rounded(px(22.))
            .bg(grad(135., rgb(lighten(hue, 0.6)), rgb(game.deep())))
            .shadow(vec![
                soft(0x000000, 0.35, 28., 10.),
                soft(hue, 0.35, 24., 0.),
            ])
            .child(
                card.px(px(26.))
                    .py(px(16.))
                    .rounded(px(19.))
                    .bg(float(c_raised())),
            );
        let panel = if self.calm {
            panel.into_any_element()
        } else {
            panel.relative().animated(
                ElementId::Name(id.into()),
                Animation::new(Duration::from_millis(380)),
                |el, t| el.opacity(t.min(1.)).top(px(14. * (1. - back_out(t)))),
            )
        };
        let dim = if is_dark() {
            hsla(0., 0., 0., 0.35)
        } else {
            hsla(0., 0., 1., 0.35)
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(dim)
                .child(panel)
                .into_any_element(),
        )
    }

    /// The cartridges, one per game.
    fn tabs(&self, cx: &mut Context<Self>) -> Div {
        let lift = if self.calm { 0. } else { -2. };
        Game::ALL
            .into_iter()
            .enumerate()
            .fold(div().flex().gap(px(8.)), |row, (i, game)| {
                let on = game == self.game;
                let (hue, deep) = (game.hue(), game.deep());
                let glyph = div()
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .bg(if on {
                        grad(180., alpha(0xffffff, 0.35), alpha(0xffffff, 0.15))
                    } else {
                        grad(150., rgb(lighten(hue, 0.7)), rgb(deep))
                    })
                    .child(icon(game.glyph(), 13., 0xffffff));
                let tab = div()
                    .id(("arcade-tab", i))
                    .relative()
                    .flex_1()
                    .h(px(38.))
                    .pl(px(8.))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .rounded(px(12.))
                    .border_b_4()
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .cursor_pointer()
                    .map(|el| {
                        if on {
                            el.bg(grad(180., rgb(lighten(hue, 0.85)), rgb(deep)))
                                .border_color(rgb(darken(deep, 0.65)))
                                .text_color(rgb(0xffffff))
                                .shadow(vec![soft(hue, 0.5, 14., 2.)])
                        } else {
                            el.bg(alpha(hue, if is_dark() { 0.14 } else { 0.1 }))
                                .border_color(alpha(hue, 0.35))
                                .text_color(rgb(c_text()))
                                .hover(|el| {
                                    el.bg(alpha(hue, if is_dark() { 0.24 } else { 0.18 }))
                                        .top(px(lift))
                                })
                        }
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick(game);
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }))
                    .child(glyph)
                    .child(game.name());
                row.child(if on && !self.calm {
                    tab.animated(
                        ElementId::Name(format!("arcade-pick-{}", self.picks).into()),
                        Animation::new(Duration::from_millis(420)),
                        |el, t| el.top(px(-7. * (1. - back_out(t)).abs() * (1. - t))),
                    )
                } else {
                    tab.into_any_element()
                })
            })
    }

    /// The marquee over the field: the game's name, then Score and Best.
    fn marquee(&self) -> Div {
        let game = self.game;
        let counter = |label: &'static str, value: u32, color: u32, bump: f32| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(26.))
                .px(px(10.))
                .rounded(px(9.))
                .bg(alpha(0x000000, 0.24))
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(alpha(0xffffff, 0.82))
                        .child(label),
                )
                .child(
                    div()
                        .min_w(px(22.))
                        .text_size(px(16. + 4. * bump))
                        .font_weight(FontWeight::BLACK)
                        .text_color(rgb(color))
                        .child(value.to_string()),
                )
        };
        let bump = if self.calm { 0. } else { self.fx.bump };
        let cheer = if self.calm { 0. } else { self.fx.cheer / CHEER };
        let best = self.fx.shown_best.round() as u32;
        let best_color = if self.record { GOLD } else { 0xffffff };
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(36.))
            .px(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(2.))
                    .text_size(px(14.))
                    .font_weight(FontWeight::BLACK)
                    .text_color(rgb(0xffffff))
                    .child(icon(game.glyph(), 15., 0xffffff))
                    .child(game.name().to_uppercase()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(counter(
                        "SCORE",
                        self.fx.shown.round() as u32,
                        0xffffff,
                        bump,
                    ))
                    .child(
                        counter("BEST", best, best_color, cheer * 0.5)
                            .when(self.record, |el| {
                                el.shadow(glow(GOLD, 0.35 + 0.5 * cheer, 12.))
                            })
                            .child(icon(IconName::Trophy, 12., best_color)),
                    ),
            )
    }
}

impl Render for Arcade {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance();
        self.advance_fx();
        let clocked = self.phase == Phase::Playing && self.game != Game::Merge;
        if clocked || self.fx.busy(self.score, self.best()) {
            window.request_animation_frame();
        }
        let field = match self.game {
            Game::Spark => self.spark_field(),
            Game::Commits => self.commits_field(),
            Game::Merge => self.merge_field(),
            Game::Stack => self.stack_field(),
        };
        // A crash shakes the field, fading out.
        let (dx, dy) = if self.fx.shake > 0. {
            let amp = 6. * self.fx.shake / SHAKE;
            let t = self.fx.time;
            ((t * 71.).sin() * amp, (t * 53.).cos() * amp * 0.5)
        } else {
            (0., 0.)
        };
        let game = self.game;
        let footer = game.keys().iter().fold(
            div().flex().items_center().gap(px(10.)),
            |row, (keys, what)| row.child(key_hint(keys, what)),
        );
        let footer_right = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(key_hint(&["1–4"], "switch"))
            .child(key_hint(&["P"], "pause"))
            .child(key_hint(&["R"], "restart"));
        div()
            .id("arcade")
            .track_focus(&self.focus)
            .key_context("Arcade")
            .v_flex()
            .gap(px(12.))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.press(e.keystroke.key.as_str()) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(self.tabs(cx))
            .child(
                // The cabinet: a bezel in the game's colour, its marquee over the field.
                div()
                    .v_flex()
                    .p(px(6.))
                    .pt_0()
                    .rounded(px(18.))
                    .bg(grad(100., rgb(lighten(game.hue(), 0.9)), rgb(game.deep())))
                    .border_b_4()
                    .border_color(rgb(darken(game.deep(), 0.7)))
                    .shadow(vec![
                        soft(game.hue(), if is_dark() { 0.35 } else { 0.25 }, 22., 4.),
                        soft(0x000000, 0.2, 6., 3.),
                    ])
                    .child(self.marquee())
                    .child(
                        div()
                            .id("arcade-field")
                            .relative()
                            .w(px(W))
                            .h(px(H))
                            .overflow_hidden()
                            .rounded(px(12.))
                            .bg(rgb(c_canvas()))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    window.focus(&this.focus, cx);
                                    let before = this.before();
                                    if this.game == Game::Spark {
                                        this.lift();
                                    } else if this.phase != Phase::Playing {
                                        this.start();
                                    }
                                    this.react(before);
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(dx))
                                    .top(px(dy))
                                    .w(px(W))
                                    .h(px(H))
                                    .child(field.absolute().inset_0())
                                    .child(self.fx_layer()),
                            )
                            .children(self.banner()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.5))
                    .text_color(rgb(c_muted()))
                    .child(footer)
                    .child(footer_right),
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
        arcade.update(cx, |a, cx| a.refresh_motion(cx));
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
            .child(icon(IconName::Gamepad2, 12., c_blue()))
            .child("Play while you wait")
            .into_any_element()
    }

    /// "ARCADE", each letter in a game's colour, hopping in as the Arcade opens.
    fn arcade_title(calm: bool) -> Div {
        let colors = [
            Game::Spark,
            Game::Commits,
            Game::Merge,
            Game::Stack,
            Game::Spark,
            Game::Commits,
        ];
        "ARCADE".chars().zip(colors).enumerate().fold(
            div().flex().items_end().gap(px(1.)),
            |row, (i, (ch, game))| {
                let letter = div()
                    .relative()
                    .text_size(px(22.))
                    .line_height(px(26.))
                    .font_weight(FontWeight::BLACK)
                    .text_color(rgb(game.ink()))
                    .child(ch.to_string());
                row.child(if calm {
                    letter.into_any_element()
                } else {
                    // Each letter starts a beat after the one before.
                    let lead = i as f32 * 0.09;
                    letter.animated(
                        ElementId::Name(format!("arcade-letter-{i}").into()),
                        Animation::new(Duration::from_millis(900)),
                        move |el, t| {
                            let u = ((t - lead) / (1. - lead)).clamp(0., 1.);
                            el.top(px(-8. * (u * std::f32::consts::PI).sin() * (1. - u)))
                        },
                    )
                })
            },
        )
    }

    pub(crate) fn arcade_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.arcade_open {
            return None;
        }
        let arcade = self.arcade.clone()?;
        let (game, calm) = {
            let a = arcade.read(cx);
            (a.game, a.calm)
        };
        let working = self.working_count();
        let done = self.arcade_waiting > 0 && working == 0;
        let chip = |color: u32| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.))
                .px(px(10.))
                .rounded_full()
                .bg(alpha(color, if is_dark() { 0.14 } else { 0.1 }))
                .border_1()
                .border_color(alpha(color, 0.35))
        };
        let status = if done {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    chip(c_green())
                        .child(
                            div()
                                .size(px(8.))
                                .rounded_full()
                                .bg(rgb(c_green()))
                                .shadow(glow(c_green(), 0.7, 8.)),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(c_text()))
                                .child("Your agents are done"),
                        ),
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
            chip(c_blue()).child(working::indicator(
                "Working",
                Some(format!(
                    "{working} agent{}",
                    if working == 1 { "" } else { "s" }
                )),
            ))
        } else {
            chip(c_muted())
                .text_color(rgb(c_muted()))
                .child("No agents running")
        };
        let accent = c_blue();
        let hue = game.hue();
        let r = crate::shell::ISLAND_R + 4.;
        let card = div()
            .id("arcade-card")
            .relative()
            .p_5()
            .v_flex()
            .gap_4()
            .rounded(px(r))
            .bg(float(c_sheet()))
            .overflow_hidden()
            // A wash of the accent and the game's colour across the top.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(140.))
                    .bg(grad(
                        180.,
                        alpha(mix(accent, hue, 0.5), if is_dark() { 0.22 } else { 0.14 }),
                        alpha(hue, 0.),
                    )),
            )
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
                            .gap(px(10.))
                            .child(
                                div()
                                    .size(px(34.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(11.))
                                    .bg(grad(150., rgb(lighten(accent, 0.75)), rgb(game.deep())))
                                    .border_b_4()
                                    .border_color(rgb(darken(game.deep(), 0.7)))
                                    .shadow(glow(mix(accent, hue, 0.5), 0.5, 14.))
                                    .child(icon(IconName::Gamepad2, 18., 0xffffff)),
                            )
                            .child(
                                div().v_flex().child(Self::arcade_title(calm)).child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(c_muted()))
                                        .child("Play while your agents work"),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.))
                            .child(status)
                            .child(
                                div()
                                    .id("arcade-close")
                                    .size(px(28.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
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
        // A ring that runs from the accent to the game's colour, with a glow of both.
        let frame = div()
            .id("arcade-frame")
            .p(px(3.))
            .rounded(px(r + 3.))
            .bg(grad(135., rgb(accent), rgb(hue)))
            .shadow(vec![
                soft(0x000000, if is_dark() { 0.5 } else { 0.2 }, 48., 18.),
                soft(
                    mix(accent, hue, 0.5),
                    if is_dark() { 0.4 } else { 0.28 },
                    40.,
                    0.,
                ),
            ])
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(card);
        let backdrop = if is_dark() {
            hsla(0., 0., 0., 0.5)
        } else {
            hsla(0., 0., 0., 0.26)
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
                .child(rise(frame, "arcade-rise", 300)),
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
            merged: Vec::new(),
            moves: 0,
        };
        assert_eq!(m.shift((0, 1), &mut rng), Some(4));
        assert_eq!(m.cells[0][3], 4);
        // A board that can't move doesn't count as a move or deal a tile.
        let mut full = Merge {
            cells: [[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 2]],
            fresh: None,
            merged: Vec::new(),
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

    fn stack_with(rows: &[(usize, &[usize])]) -> Stack {
        let mut s = Stack::new(&mut Rng::new());
        for &(r, cols) in rows {
            for &c in cols {
                s.well[r][c] = Some(Kind::O);
            }
        }
        s
    }

    #[test]
    fn every_tetromino_turns_back_to_where_it_started() {
        for kind in Kind::ALL {
            let p = Piece {
                kind,
                rot: 0,
                col: 3,
                row: 5,
            };
            let mut cells = p.cells();
            cells.sort();
            let mut turned = Piece { rot: 4, ..p }.cells();
            turned.sort();
            assert_eq!(cells, turned, "{kind:?}");
            // A quarter turn keeps the four cells distinct.
            let mut q = Piece { rot: 1, ..p }.cells().to_vec();
            q.sort();
            q.dedup();
            assert_eq!(q.len(), 4, "{kind:?}");
        }
    }

    #[test]
    fn the_bag_deals_all_seven_before_repeating() {
        let mut rng = Rng::new();
        let mut s = Stack::new(&mut rng);
        s.bag.clear();
        let mut dealt: Vec<Kind> = (0..7).map(|_| s.draw(&mut rng)).collect();
        dealt.sort_by_key(|k| *k as u8);
        assert_eq!(dealt, Kind::ALL.to_vec());
    }

    #[test]
    fn pieces_stop_at_the_walls_and_kick_off_them_to_turn() {
        let mut s = stack_with(&[]);
        s.piece = Piece::spawn(Kind::T);
        while s.shift(-1) {}
        assert_eq!(s.piece.cells().iter().map(|c| c.0).min(), Some(0));
        // An upright I against the right wall still turns flat, nudged inward.
        s.piece = Piece {
            kind: Kind::I,
            rot: 1,
            col: 7,
            row: 3,
        };
        while s.shift(1) {}
        assert!(s.rotate(1));
        assert!(s.piece.cells().iter().all(|c| c.0 < WELL_W as i32));
    }

    #[test]
    fn a_full_row_clears_and_scores_by_level() {
        let mut rng = Rng::new();
        // The bottom row is full but for the last two columns, which an O fills.
        let mut s = stack_with(&[(19, &[0, 1, 2, 3, 4, 5, 6, 7])]);
        s.piece = Piece {
            kind: Kind::O,
            rot: 0,
            col: 8,
            row: 0,
        };
        assert_eq!(s.hard_drop(&mut rng), Some(18 * 2 + 100));
        assert_eq!(s.lines, 1);
        // The O's top half falls into the cleared row.
        assert_eq!(s.well[19][8], Some(Kind::O));
        assert_eq!(s.well[19][0], None);
        assert_eq!(s.well[18][8], None);
        // Ten lines make level 2, which falls faster and doubles line points.
        s.lines = 10;
        assert_eq!(s.level(), 2);
        assert!(s.gravity() < 0.8);
    }

    #[test]
    fn a_piece_that_cannot_enter_ends_the_round() {
        let mut rng = Rng::new();
        // A block where the next T appears.
        let mut s = stack_with(&[(0, &[4])]);
        s.next = Kind::T;
        s.piece = Piece {
            kind: Kind::O,
            rot: 0,
            col: 0,
            row: 0,
        };
        assert_eq!(s.hard_drop(&mut rng), None);
    }

    #[test]
    fn gravity_drops_the_piece_and_locks_it_on_the_floor() {
        let mut rng = Rng::new();
        let mut s = stack_with(&[]);
        s.piece = Piece::spawn(Kind::O);
        let mut steps = 0;
        while s.well[19].iter().all(Option::is_none) {
            assert!(s.step(0.1, &mut rng).is_some());
            steps += 1;
            assert!(steps < 1000);
        }
        assert_eq!(s.well[19][4], Some(Kind::O));
    }
}
