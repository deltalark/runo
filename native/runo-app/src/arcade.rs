//! The Arcade: four small games to play while agents work. Spark (fly a spark through
//! the gates), Commits (a snake that grows on commits), Merge (2048) and Stack (Tetris).
//! Each is drawn in the theme's own colours, keeps its best score in `arcade.json`, and
//! pauses when the Arcade closes, so a round picks up where it left off.

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

    fn color(self) -> u32 {
        match self {
            Kind::I => lit(),
            Kind::O => c_amber(),
            Kind::T => mix(c_blue(), c_red(), 0.5),
            Kind::S => c_green(),
            Kind::Z => c_red(),
            Kind::J => c_blue(),
            Kind::L => EMBER,
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
        let stepped = match self.game {
            Game::Spark => self.spark.step(dt, &mut self.rng),
            Game::Commits => self.commits.step(dt, &mut self.rng),
            Game::Merge => Some(0),
            Game::Stack => self.stack.step(dt, &mut self.rng),
        };
        self.land(stepped);
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

    /// One block of a tetromino, `left`/`top` in pixels within its parent.
    fn block(left: f32, top: f32, color: u32) -> Div {
        div()
            .absolute()
            .left(px(left + 1.))
            .top(px(top + 1.))
            .size(px(BLOCK - 2.))
            .rounded(px(3.))
            .bg(rgb(color))
            .border_1()
            .border_color(rgb(mix(0xffffff, color, 0.3)))
    }

    fn stack_field(&self) -> Div {
        let (ww, wh) = (WELL_W as f32 * BLOCK, WELL_H as f32 * BLOCK);
        let ox = (W - ww) / 2. - 40.;
        let oy = (H - wh) / 2.;
        let mut well = div()
            .absolute()
            .left(px(ox))
            .top(px(oy))
            .w(px(ww))
            .h(px(wh))
            .rounded(px(6.))
            .bg(alpha(c_text(), if is_dark() { 0.05 } else { 0.06 }));
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
                        .border_1()
                        .border_color(alpha(color, 0.5)),
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
                    .shadow(glow(color, 0.45, 8.)),
            );
        }
        // Beside the well: the next piece, then the level and lines.
        let next = self.stack.next;
        let box_side = 4. * BLOCK;
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
                .rounded(px(6.))
                .bg(alpha(c_text(), 0.04)),
            |el, (c, r)| {
                el.child(Self::block(
                    pad_x + c as f32 * BLOCK,
                    pad_y + r as f32 * BLOCK,
                    next.color(),
                ))
            },
        );
        let stat = |label: &'static str, value: String| {
            div()
                .v_flex()
                .gap(px(1.))
                .text_size(px(11.))
                .text_color(rgb(c_muted()))
                .child(label)
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(c_text()))
                        .child(value),
                )
        };
        let side = div()
            .absolute()
            .left(px(ox + ww + 24.))
            .top(px(oy))
            .v_flex()
            .gap_3()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .text_size(px(11.))
                    .text_color(rgb(c_muted()))
                    .child("Next")
                    .child(preview),
            )
            .child(stat("Level", self.stack.level().to_string()))
            .child(stat("Lines", self.stack.lines.to_string()));
        div().child(Self::dots(0.)).child(well).child(side)
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
            Game::Stack => self.stack_field(),
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
                    .child("1–4 switch · P pause · R restart"),
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
