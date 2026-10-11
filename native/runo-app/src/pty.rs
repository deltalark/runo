//! Interactive shells: a real PTY per terminal tab, parsed with a VT100 emulator.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Lines kept above the visible screen.
const SCROLLBACK: usize = 2000;

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
}

/// A program to run in the PTY instead of the login shell.
pub struct Program {
    pub bin: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Variables to drop from the inherited environment.
    pub unset: Vec<String>,
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
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
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
        })
    }

    pub fn write(&self, bytes: &[u8]) {
        self.last_input.store(now_ms(), Ordering::Release);
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
        // Typing jumps back to the live screen.
        self.scroll_to(0);
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
}
