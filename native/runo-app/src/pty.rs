//! Interactive shells: a real PTY per terminal tab, parsed with a VT100 emulator.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Lines kept above the visible screen, from the Terminal settings.
static SCROLLBACK: AtomicUsize = AtomicUsize::new(10_000);

/// Set how many lines new shells keep above the screen.
pub fn set_scrollback(lines: usize) {
    SCROLLBACK.store(lines.clamp(500, 100_000), Ordering::Relaxed);
}

/// A spot in the terminal: `back` counts lines up from the bottom of the live screen,
/// so it stays put while the view scrolls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub back: usize,
    pub col: u16,
}

/// What the terminal view keeps between frames: selection, find and where it is drawn.
#[derive(Clone, Debug, Default)]
pub struct ViewState {
    /// Where a mouse selection started and where it is now.
    pub selection: Option<(Spot, Spot)>,
    pub selecting: bool,
    /// The find bar's text, while it is open.
    pub find: Option<String>,
    /// The match shown, counted from the newest.
    pub find_at: usize,
    /// Top-left of the drawn grid in window pixels, and the cell size.
    pub origin: (f32, f32),
    pub cell: (f32, f32),
    /// The match being shown (lines up, start and end column) and how many there are.
    pub find_hit: Option<(usize, usize, usize)>,
    pub find_count: usize,
}

impl ViewState {
    /// The selection ordered from its top-left to its bottom-right.
    pub fn ordered(&self) -> Option<(Spot, Spot)> {
        let (a, b) = self.selection?;
        let key = |s: Spot| (std::cmp::Reverse(s.back), s.col);
        Some(if key(a) <= key(b) { (a, b) } else { (b, a) })
    }

    /// Whether the cell at `back` lines up and column `col` is selected.
    pub fn selected(&self, back: usize, col: u16) -> bool {
        let Some((start, end)) = self.ordered() else {
            return false;
        };
        if start == end || back > start.back || back < end.back {
            return false;
        }
        let after_start = back < start.back || col >= start.col;
        let before_end = back > end.back || col < end.col;
        after_start && before_end
    }
}

pub struct Shell {
    pub id: u64,
    pub session: u64,
    pub title: String,
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Box<dyn MasterPty + Send>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    exited: Arc<AtomicBool>,
    /// Set by the reader when output arrives, cleared when the UI repaints.
    dirty: Arc<AtomicBool>,
    size: Mutex<(u16, u16)>,
    /// When output last arrived, in milliseconds since the Unix epoch.
    last_output: Arc<AtomicU64>,
    /// When the person last typed, in the same clock.
    last_input: AtomicU64,
    /// Selection, find and layout of the view drawing this shell.
    pub view: Arc<Mutex<ViewState>>,
}

/// A program to run in the PTY instead of the login shell.
pub struct Program {
    pub bin: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Variables to drop from the inherited environment.
    pub unset: Vec<String>,
}

impl Program {
    /// The program, arguments and environment a command was set up with, for a PTY.
    pub fn of(command: &std::process::Command) -> Program {
        Program {
            bin: command.get_program().to_string_lossy().into_owned(),
            args: command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
            env: command
                .get_envs()
                .filter_map(|(k, v)| {
                    Some((
                        k.to_string_lossy().into_owned(),
                        v?.to_string_lossy().into_owned(),
                    ))
                })
                .collect(),
            unset: Vec::new(),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Shell {
    #[cfg_attr(not(test), allow(dead_code))]
    /// Start the user's login shell in `dir`. `wake` is called (at most once per
    /// repaint) when there is new output, and once more when the shell exits.
    pub fn spawn(
        id: u64,
        session: u64,
        dir: &str,
        rows: u16,
        cols: u16,
        wake: impl Fn(u64) + Send + 'static,
    ) -> Result<Self, String> {
        Self::spawn_program(id, session, dir, rows, cols, None, wake)
    }

    /// Start `program` (or the login shell) in `dir`.
    pub fn spawn_program(
        id: u64,
        session: u64,
        dir: &str,
        rows: u16,
        cols: u16,
        program: Option<Program>,
        wake: impl Fn(u64) + Send + 'static,
    ) -> Result<Self, String> {
        let (rows, cols) = (rows.max(2), cols.max(10));
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        let custom = program.is_some();
        let mut cmd = match &program {
            Some(p) => {
                let mut c = CommandBuilder::new(&p.bin);
                c.args(&p.args);
                for k in &p.unset {
                    c.env_remove(k);
                }
                for (k, v) in &p.env {
                    c.env(k, v);
                }
                c
            }
            None => CommandBuilder::new_default_prog(),
        };
        if !dir.is_empty() {
            cmd.cwd(dir);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            rows,
            cols,
            SCROLLBACK.load(Ordering::Relaxed),
        )));
        let exited = Arc::new(AtomicBool::new(false));
        let dirty = Arc::new(AtomicBool::new(false));
        let last_output = Arc::new(AtomicU64::new(now_ms()));
        {
            let (parser, exited, dirty, last) = (
                parser.clone(),
                exited.clone(),
                dirty.clone(),
                last_output.clone(),
            );
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            last.store(now_ms(), Ordering::Release);
                            if let Ok(mut p) = parser.lock() {
                                p.process(&buf[..n]);
                            }
                            if !dirty.swap(true, Ordering::AcqRel) {
                                wake(id);
                            }
                        }
                    }
                }
                exited.store(true, Ordering::Release);
                wake(id);
            });
        }
        let name = program
            .filter(|_| custom)
            .map(|p| p.bin)
            .or_else(|| std::env::var("SHELL").ok())
            .and_then(|s| s.rsplit(['/', '\\']).next().map(str::to_string))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| if cfg!(windows) { "pwsh" } else { "shell" }.into());
        Ok(Self {
            id,
            session,
            title: name,
            parser,
            writer: Mutex::new(writer),
            master: pair.master,
            child: Mutex::new(child),
            exited,
            dirty,
            size: Mutex::new((rows, cols)),
            last_output,
            last_input: AtomicU64::new(0),
            view: Arc::new(Mutex::new(ViewState::default())),
        })
    }

    pub fn write(&self, bytes: &[u8]) {
        self.last_input.store(now_ms(), Ordering::Release);
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
        // Typing jumps back to the live screen and drops the selection.
        self.scroll_to(0);
        if let Ok(mut v) = self.view.lock() {
            v.selection = None;
        }
    }

    /// The view state, to read or change.
    pub fn view<R>(&self, f: impl FnOnce(&mut ViewState) -> R) -> R {
        let mut v = self.view.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut v)
    }

    /// Every line kept, oldest first: the scrollback, then the screen.
    pub fn history(&self) -> Vec<String> {
        let mut p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let at = p.screen().scrollback();
        p.set_scrollback(usize::MAX);
        let total = p.screen().scrollback();
        let (rows, cols) = p.screen().size();
        let mut lines: Vec<String> = Vec::with_capacity(total + rows as usize);
        let mut off = total;
        loop {
            p.set_scrollback(off);
            for (r, text) in p.screen().rows(0, cols).enumerate() {
                if total - off + r == lines.len() {
                    lines.push(text.trim_end().to_string());
                }
            }
            if off == 0 {
                break;
            }
            off = off.saturating_sub(rows as usize);
        }
        p.set_scrollback(at);
        lines
    }

    /// Show the line `back` lines up from the bottom, scrolling only when it is off screen.
    pub fn reveal(&self, back: usize) {
        let (rows, at) = self.with_screen(|s| (s.size().0 as usize, s.scrollback()));
        if back < at || back >= at + rows {
            self.scroll_to(back.saturating_sub(rows / 2));
        }
    }

    /// Look for the find bar's text and show a match: `step` 0 stays on the current
    /// one, 1 moves to an older one, -1 to a newer one.
    pub fn find_step(&self, step: isize) {
        let Some(query) = self.view(|v| v.find.clone()) else {
            return;
        };
        let lines = self.history();
        let hits = find_in(&lines, &query);
        let shown = self.view(|v| {
            v.find_count = hits.len();
            if hits.is_empty() {
                v.find_at = 0;
                v.find_hit = None;
                return None;
            }
            let at = (v.find_at as isize + step).rem_euclid(hits.len() as isize) as usize;
            v.find_at = at;
            let (line, start, end) = hits[hits.len() - 1 - at];
            let back = lines.len() - 1 - line;
            v.find_hit = Some((back, start, end));
            Some(back)
        });
        if let Some(back) = shown {
            self.reveal(back);
        }
    }

    /// The selected text, lines joined with newlines.
    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.view(|v| v.ordered())?;
        if start == end {
            return None;
        }
        let lines = self.history();
        let n = lines.len();
        let mut out = Vec::new();
        for back in (end.back..=start.back).rev() {
            let Some(line) = n.checked_sub(back + 1).and_then(|i| lines.get(i)) else {
                continue;
            };
            let chars: Vec<char> = line.chars().collect();
            let from = if back == start.back {
                start.col as usize
            } else {
                0
            };
            let to = if back == end.back {
                end.col as usize
            } else {
                chars.len()
            };
            let to = to.min(chars.len());
            out.push(if from < to {
                chars[from..to].iter().collect::<String>()
            } else {
                String::new()
            });
        }
        let text = out.join("\n");
        (!text.trim().is_empty()).then_some(text)
    }

    /// Send bytes the program asked for, such as mouse reports, without the side effects
    /// of typing.
    pub fn write_raw(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    /// Paste text, wrapped in bracketed-paste markers when the program asked for them.
    pub fn paste(&self, text: &str) {
        let bracketed = self.with_screen(|s| s.bracketed_paste());
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        let Ok(mut size) = self.size.lock() else {
            return;
        };
        if *size == (rows, cols) {
            return;
        }
        *size = (rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
    }

    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        let p = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        f(p.screen())
    }

    /// Scroll the view by `lines` (positive is back in history).
    pub fn scroll_by(&self, lines: isize) {
        let current = self.with_screen(|s| s.scrollback()) as isize;
        self.scroll_to((current + lines).max(0) as usize);
    }

    fn scroll_to(&self, lines: usize) {
        if let Ok(mut p) = self.parser.lock() {
            p.set_scrollback(lines);
        }
    }

    /// Whether the program is redrawing on its own, the way an agent CLI animates while it
    /// works: output in the last `window` ms that is not just the echo of recent typing.
    pub fn busy(&self, window: u64) -> bool {
        let out = self.last_output.load(Ordering::Acquire);
        let typed = self.last_input.load(Ordering::Acquire);
        now_ms().saturating_sub(out) < window && out.saturating_sub(typed) > window
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    /// The screen as plain text, for copying and tests.
    pub fn text(&self) -> String {
        self.with_screen(|s| s.contents())
    }

    pub fn kill(&self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }

    /// Whether the program exited successfully, once it has exited. A program that
    /// closed its terminal but hasn't quit yet is given half a second.
    pub fn success(&self) -> Option<bool> {
        let mut c = self.child.lock().ok()?;
        for _ in 0..50 {
            if let Ok(Some(status)) = c.try_wait() {
                return Some(status.success());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    /// Ask the program to finish up: Runo's connection runner ends the program on the
    /// connection, then syncs back before it exits. Elsewhere this is a kill.
    pub fn finish(&self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.lock().ok().and_then(|c| c.process_id()) {
            // SAFETY: signalling a child process this shell started.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGUSR1);
            }
            return;
        }
        self.kill();
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Bytes a keystroke sends to the program, xterm style.
pub fn key_bytes(
    key: &str,
    key_char: Option<&str>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let cursor = |c: char| -> Vec<u8> {
        let mods = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
        if mods > 1 {
            format!("\x1b[1;{mods}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let bytes = match key {
        "enter" => b"\r".to_vec(),
        "backspace" if alt => b"\x1b\x7f".to_vec(),
        "backspace" if ctrl => b"\x08".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "tab" if shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        "delete" => b"\x1b[3~".to_vec(),
        "insert" => b"\x1b[2~".to_vec(),
        "space" if ctrl => vec![0],
        _ if ctrl && key.chars().count() == 1 => {
            let c = key.chars().next()?.to_ascii_lowercase();
            let b = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '6' => 0x1e,
                '/' | '-' | '7' => 0x1f,
                '@' | '2' => 0,
                _ => return None,
            };
            if alt {
                vec![0x1b, b]
            } else {
                vec![b]
            }
        }
        _ => {
            let text = key_char
                .map(str::to_string)
                .or_else(|| (key == "space").then(|| " ".to_string()))?;
            let mut v = if alt { vec![0x1b] } else { vec![] };
            v.extend(text.into_bytes());
            v
        }
    };
    Some(bytes)
}

/// xterm's 256-colour palette, with the first 16 tuned to the app's theme.
pub fn palette(i: u8, dark: bool) -> u32 {
    const DARK: [u32; 16] = [
        0x1e1e22, 0xf87171, 0x4ade80, 0xfacc15, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xd4d4d8, 0x71717a,
        0xfca5a5, 0x86efac, 0xfde047, 0x93c5fd, 0xd8b4fe, 0x67e8f9, 0xfafafa,
    ];
    const LIGHT: [u32; 16] = [
        0x18181b, 0xdc2626, 0x16a34a, 0xca8a04, 0x2563eb, 0x9333ea, 0x0891b2, 0x52525b, 0x71717a,
        0xef4444, 0x22c55e, 0xeab308, 0x3b82f6, 0xa855f7, 0x06b6d4, 0x27272a,
    ];
    match i {
        0..=15 => (if dark { DARK } else { LIGHT })[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v as u32 };
            (level(i / 36) << 16) | (level((i / 6) % 6) << 8) | level(i % 6)
        }
        _ => {
            let v = 8 + 10 * (i as u32 - 232);
            (v << 16) | (v << 8) | v
        }
    }
}

/// A mouse event to report to a program that asked for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAct {
    Press(u8),
    Release(u8),
    /// Moved, with the button held if any.
    Move(Option<u8>),
    /// Wheel up (true) or down.
    Wheel(bool),
}

/// The bytes reporting a mouse event at 0-based `row`/`col`, or `None` when the
/// program's mouse mode does not ask for it. `mods` is shift, alt, ctrl.
pub fn mouse_report(
    mode: vt100::MouseProtocolMode,
    encoding: vt100::MouseProtocolEncoding,
    act: MouseAct,
    row: u16,
    col: u16,
    (shift, alt, ctrl): (bool, bool, bool),
) -> Option<Vec<u8>> {
    use vt100::MouseProtocolMode as M;
    let wanted = match act {
        MouseAct::Press(_) | MouseAct::Wheel(_) => mode != M::None,
        MouseAct::Release(_) => !matches!(mode, M::None | M::Press),
        MouseAct::Move(Some(_)) => matches!(mode, M::ButtonMotion | M::AnyMotion),
        MouseAct::Move(None) => mode == M::AnyMotion,
    };
    if !wanted {
        return None;
    }
    let sgr = encoding == vt100::MouseProtocolEncoding::Sgr;
    let mut code: u32 = match act {
        MouseAct::Press(b) => b as u32,
        MouseAct::Release(b) if sgr => b as u32,
        MouseAct::Release(_) => 3,
        MouseAct::Move(b) => 32 + b.map_or(3, |b| b as u32),
        MouseAct::Wheel(up) => 64 + (!up) as u32,
    };
    code += 4 * shift as u32 + 8 * alt as u32 + 16 * ctrl as u32;
    let (x, y) = (col as u32 + 1, row as u32 + 1);
    if sgr {
        let end = if matches!(act, MouseAct::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    let mut out = b"\x1b[M".to_vec();
    for v in [code + 32, x + 32, y + 32] {
        if encoding == vt100::MouseProtocolEncoding::Utf8 {
            let c = char::from_u32(v.min(2047))?;
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        } else {
            out.push(v.min(255) as u8);
        }
    }
    Some(out)
}

/// Web links in a line of terminal text, as character columns `[start, end)` and the URL.
pub fn links(line: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let text: String = chars.iter().collect();
    let mut found = Vec::new();
    let mut from = 0;
    while from < chars.len() {
        let rest: String = chars[from..].iter().collect();
        let Some(at) = ["https://", "http://", "file://"]
            .iter()
            .filter_map(|p| rest.find(p))
            .min()
        else {
            break;
        };
        let start = from + rest[..at].chars().count();
        let mut end = start;
        while end < chars.len() && !chars[end].is_whitespace() && !"<>\"'`".contains(chars[end]) {
            end += 1;
        }
        // Leave closing punctuation that usually ends the sentence, not the link.
        while end > start && ".,;:!?)]}".contains(chars[end - 1]) {
            let c = chars[end - 1];
            let open = match c {
                ')' => '(',
                ']' => '[',
                '}' => '{',
                _ => '\0',
            };
            let opens = chars[start..end].iter().filter(|&&x| x == open).count();
            let closes = chars[start..end].iter().filter(|&&x| x == c).count();
            if open != '\0' && opens >= closes {
                break;
            }
            end -= 1;
        }
        let url: String = chars[start..end].iter().collect();
        if url.contains("://") && !url.ends_with("://") {
            found.push((start, end, url));
        }
        from = end.max(start + 1);
    }
    let _ = text;
    found
}

/// Where `query` appears in `lines` (oldest first), case-insensitively, as
/// (line, start column, end column), newest last.
pub fn find_in(lines: &[String], query: &str) -> Vec<(usize, usize, usize)> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        if chars.len() < query.len() {
            continue;
        }
        let mut c = 0;
        while c + query.len() <= chars.len() {
            if chars[c..c + query.len()] == query[..] {
                hits.push((i, c, c + query.len()));
                c += query.len();
            } else {
                c += 1;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn keys_map_to_xterm_sequences() {
        let k = |key, ch: Option<&str>, ctrl, alt| key_bytes(key, ch, ctrl, alt, false, false);
        assert_eq!(k("c", Some("c"), true, false), Some(vec![3]));
        assert_eq!(k("a", Some("a"), false, false), Some(b"a".to_vec()));
        assert_eq!(k("b", Some("b"), false, true), Some(b"\x1bb".to_vec()));
        assert_eq!(k("enter", None, false, false), Some(b"\r".to_vec()));
        assert_eq!(k("up", None, false, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            key_bytes("up", None, false, false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(k("left", None, true, false), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(k("shift", None, false, false), None);
        assert_eq!(palette(196, true), 0xff0000);
        assert_eq!(palette(232, true), 0x080808);
    }

    /// A setup command runs in a terminal that can ask questions, and its result is known.
    #[cfg(unix)]
    #[test]
    fn programs_take_answers_and_report_how_they_ended() {
        let mut command = std::process::Command::new("sh");
        command
            .args([
                "-c",
                "printf 'Name? '; read name; echo \"hi $name\"; exit 4",
            ])
            .env("RUNO_TEST", "1");
        let program = Program::of(&command);
        assert_eq!(program.bin, "sh");
        assert_eq!(
            program.env,
            vec![("RUNO_TEST".to_string(), "1".to_string())]
        );
        let dir = std::env::temp_dir();
        let shell =
            Shell::spawn_program(1, 1, dir.to_str().unwrap(), 10, 40, Some(program), |_| {})
                .unwrap();
        let start = Instant::now();
        while !shell.text().contains("Name?") && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        shell.write(b"runo\r");
        while !shell.exited() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(shell.text().contains("hi runo"), "{}", shell.text());
        assert_eq!(shell.success(), Some(false));
    }

    #[cfg(unix)]
    #[test]
    fn shells_run_commands_and_resize() {
        let dir = std::env::temp_dir();
        let shell = Shell::spawn(1, 1, dir.to_str().unwrap(), 10, 40, |_| {}).unwrap();
        shell.write(b"echo runo-$((40+2))\r");
        let start = Instant::now();
        while !shell.text().contains("runo-42") && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(shell.text().contains("runo-42"), "{}", shell.text());
        assert!(shell.take_dirty());
        shell.resize(20, 100);
        assert_eq!(shell.with_screen(|s| s.size()), (20, 100));
        shell.write(b"exit\r");
        let start = Instant::now();
        while !shell.exited() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(shell.exited());
    }

    #[test]
    fn mouse_reports_follow_the_programs_mode() {
        use vt100::{MouseProtocolEncoding as E, MouseProtocolMode as M};
        let none = (false, false, false);
        let r = |mode, enc, act| mouse_report(mode, enc, act, 4, 9, none);
        assert_eq!(r(M::None, E::Sgr, MouseAct::Press(0)), None);
        assert_eq!(
            r(M::PressRelease, E::Sgr, MouseAct::Press(0)),
            Some(b"\x1b[<0;10;5M".to_vec())
        );
        assert_eq!(
            r(M::PressRelease, E::Sgr, MouseAct::Release(0)),
            Some(b"\x1b[<0;10;5m".to_vec())
        );
        assert_eq!(r(M::Press, E::Sgr, MouseAct::Release(0)), None);
        assert_eq!(
            r(M::PressRelease, E::Default, MouseAct::Press(2)),
            Some(vec![0x1b, b'[', b'M', 34, 42, 37])
        );
        assert_eq!(
            r(M::PressRelease, E::Sgr, MouseAct::Wheel(false)),
            Some(b"\x1b[<65;10;5M".to_vec())
        );
        assert_eq!(r(M::ButtonMotion, E::Sgr, MouseAct::Move(None)), None);
        assert_eq!(
            r(M::AnyMotion, E::Sgr, MouseAct::Move(Some(0))),
            Some(b"\x1b[<32;10;5M".to_vec())
        );
    }

    #[test]
    fn links_and_find_in_terminal_text() {
        assert_eq!(
            links("see https://example.com/a_(b), then http://x.io."),
            vec![
                (4, 29, "https://example.com/a_(b)".to_string()),
                (36, 47, "http://x.io".to_string())
            ]
        );
        assert!(links("no links: https:// here").is_empty());
        let lines = vec![
            "Error: one".to_string(),
            "fine".into(),
            "error two error".into(),
        ];
        assert_eq!(
            find_in(&lines, "ERROR"),
            vec![(0, 0, 5), (2, 0, 5), (2, 10, 15)]
        );
        assert!(find_in(&lines, "").is_empty());
    }

    #[test]
    fn selections_cover_whole_lines_between_their_ends() {
        let mut v = ViewState::default();
        let spot = |back, col| Spot { back, col };
        v.selection = Some((spot(1, 5), spot(3, 2)));
        assert_eq!(v.ordered(), Some((spot(3, 2), spot(1, 5))));
        assert!(!v.selected(3, 1));
        assert!(v.selected(3, 2));
        assert!(v.selected(2, 0));
        assert!(v.selected(1, 4));
        assert!(!v.selected(1, 5));
        assert!(!v.selected(0, 0));
    }

    #[cfg(unix)]
    #[test]
    fn history_and_selection_reach_into_the_scrollback() {
        let dir = std::env::temp_dir();
        let shell = Shell::spawn(1, 1, dir.to_str().unwrap(), 5, 40, |_| {}).unwrap();
        shell.write(b"for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo line-$i; done\r");
        let start = Instant::now();
        while !shell.history().iter().any(|l| l == "line-12")
            && start.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        let lines = shell.history();
        let first = lines
            .iter()
            .position(|l| l == "line-1")
            .expect("line-1 kept");
        assert_eq!(lines[first + 11], "line-12");
        assert!(lines.len() > 5, "scrollback kept");
        let back = lines.len() - 1 - first;
        shell.view(|v| {
            v.selection = Some((
                Spot { back, col: 0 },
                Spot {
                    back: back - 1,
                    col: 6,
                },
            ))
        });
        assert_eq!(shell.selection_text().as_deref(), Some("line-1\nline-2"));
        shell.kill();
    }
}
