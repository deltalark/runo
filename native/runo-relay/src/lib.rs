//! A mailbox on the internet between Runo's phone app and a computer running
//! `runo --phone --relay <url>`, so the phone works away from home.
//!
//! The computer keeps asking the relay for the next message (`GET .../next`, held open
//! for a while); the phone posts a message (`POST .../ask`) and waits while the
//! computer answers it (`POST .../reply/<id>`). Both sides only connect out, so nothing
//! at home needs an open port.
//!
//! Every message is sealed with a key made from the pairing secret, which the relay never
//! sees: it only passes opaque text between the two sides of a room. A room's name is a
//! hash of that secret too, so nobody can find or join it without the pairing code.
//! Rooms live in memory and go away when idle.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The port the relay listens on unless an address is given.
pub const PORT: u16 = 7632;
/// How long the computer's `next` is held open when nothing is waiting.
const POLL: Duration = Duration::from_secs(25);
/// How long the phone's `ask` waits for the computer's answer.
const ANSWER: Duration = Duration::from_secs(30);
/// A computer that hasn't asked for messages for this long counts as offline.
const ONLINE: Duration = Duration::from_secs(60);
/// Rooms nobody used for this long are dropped.
const IDLE: Duration = Duration::from_secs(10 * 60);
const BODY_MAX: usize = 512 * 1024;
const ROOMS_MAX: usize = 1000;
const WAITING_MAX: usize = 64;

#[derive(Default)]
struct Room {
    asks: VecDeque<(u64, String)>,
    replies: HashMap<u64, String>,
    polled: Option<Instant>,
    used: Option<Instant>,
}

#[derive(Default)]
struct Rooms {
    rooms: HashMap<String, Room>,
    next_id: u64,
}

/// The relay's shared state: rooms behind one lock, and one signal for any change.
#[derive(Clone, Default)]
pub struct Relay {
    inner: Arc<(Mutex<Rooms>, Condvar)>,
}

/// A response: status code, content type and body.
type Reply = (u16, &'static str, String);

fn json(status: u16, value: &str) -> Reply {
    (status, "application/json", value.to_string())
}

fn error(status: u16, message: &str) -> Reply {
    let text = message.replace('\\', "\\\\").replace('"', "\\\"");
    json(status, &format!("{{\"error\":\"{text}\"}}"))
}

impl Relay {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Rooms> {
        self.inner.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drop idle rooms. Called with the lock held.
    fn sweep(rooms: &mut Rooms, now: Instant) {
        rooms
            .rooms
            .retain(|_, r| r.used.is_some_and(|t| now.duration_since(t) < IDLE));
    }

    /// The phone's message: wait for the computer to answer it.
    fn ask(&self, room: &str, body: String) -> Reply {
        let now = Instant::now();
        let mut rooms = self.lock();
        Self::sweep(&mut rooms, now);
        let online = rooms
            .rooms
            .get(room)
            .and_then(|r| r.polled)
            .is_some_and(|t| now.duration_since(t) < ONLINE);
        if !online {
            return error(503, "The computer isn't connected to the relay. Check that Runo's phone bridge is running there.");
        }
        rooms.next_id += 1;
        let id = rooms.next_id;
        let r = rooms.rooms.entry(room.to_string()).or_default();
        if r.asks.len() >= WAITING_MAX {
            return error(429, "Too many messages are waiting for this computer.");
        }
        r.asks.push_back((id, body));
        r.used = Some(now);
        self.inner.1.notify_all();
        let deadline = now + ANSWER;
        loop {
            let r = rooms.rooms.entry(room.to_string()).or_default();
            if let Some(reply) = r.replies.remove(&id) {
                return (200, "text/plain", reply);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                r.asks.retain(|(a, _)| *a != id);
                return error(504, "The computer didn't answer in time.");
            }
            rooms = self
                .inner
                .1
                .wait_timeout(rooms, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// The computer asks for the next message, waiting a while for one.
    fn next(&self, room: &str) -> Reply {
        let now = Instant::now();
        let mut rooms = self.lock();
        Self::sweep(&mut rooms, now);
        if !rooms.rooms.contains_key(room) && rooms.rooms.len() >= ROOMS_MAX {
            return error(503, "This relay is full.");
        }
        let deadline = now + POLL;
        loop {
            let r = rooms.rooms.entry(room.to_string()).or_default();
            r.polled = Some(Instant::now());
            r.used = r.polled;
            if let Some((id, body)) = r.asks.pop_front() {
                return json(200, &format!("{{\"id\":{id},\"body\":\"{body}\"}}"));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return (204, "application/json", String::new());
            }
            rooms = self
                .inner
                .1
                .wait_timeout(rooms, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// The computer's answer to message `id`.
    fn reply(&self, room: &str, id: u64, body: String) -> Reply {
        let mut rooms = self.lock();
        let Some(r) = rooms.rooms.get_mut(room) else {
            return error(404, "unknown room");
        };
        r.replies.insert(id, body);
        // Answers nobody collects (the phone gave up) don't pile up.
        if r.replies.len() > WAITING_MAX {
            let oldest = r.replies.keys().min().copied();
            if let Some(oldest) = oldest {
                r.replies.remove(&oldest);
            }
        }
        self.inner.1.notify_all();
        json(200, "{\"ok\":true}")
    }

    /// Route one request.
    pub fn respond(&self, method: &str, path: &str, body: Vec<u8>) -> Reply {
        let parts: Vec<&str> = path
            .split('?')
            .next()
            .unwrap_or("")
            .trim_matches('/')
            .split('/')
            .collect();
        let room_ok = |r: &str| r.len() == 64 && r.bytes().all(|b| b.is_ascii_hexdigit());
        // Sealed messages are base64, so they can go into JSON as they are.
        let text = match String::from_utf8(body) {
            Ok(t)
                if t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b)) =>
            {
                t
            }
            _ => return error(400, "Messages must be sealed base64."),
        };
        match (method, parts.as_slice()) {
            ("GET", ["v1", "hello"]) => json(200, "{\"app\":\"runo-relay\",\"api\":1}"),
            ("POST", ["v1", "rooms", room, "ask"]) if room_ok(room) && !text.is_empty() => {
                self.ask(room, text)
            }
            ("GET", ["v1", "rooms", room, "next"]) if room_ok(room) => self.next(room),
            ("POST", ["v1", "rooms", room, "reply", id]) if room_ok(room) => match id.parse() {
                Ok(id) => self.reply(room, id, text),
                Err(_) => error(404, "not found"),
            },
            _ => error(404, "not found"),
        }
    }

    /// Answer connections until the listener fails, each on its own thread.
    pub fn serve(&self, listener: TcpListener) {
        for stream in listener.incoming().flatten() {
            let relay = self.clone();
            std::thread::spawn(move || relay.handle(stream));
        }
    }

    fn handle(&self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let reply = match read_request(&mut stream) {
            Some((method, path, body)) => self.respond(&method, &path, body),
            None => error(400, "bad request"),
        };
        let _ = write_reply(&mut stream, reply);
    }
}

fn read_request(stream: &mut impl Read) -> Option<(String, String, Vec<u8>)> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut length = 0usize;
    let mut header_bytes = 0usize;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).ok()?;
        header_bytes += n;
        if n == 0 || header_bytes > 64 * 1024 {
            return None;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok()?;
            }
        }
    }
    if length > BODY_MAX {
        return None;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some((method, path, body))
}

fn write_reply(stream: &mut impl Write, (status, kind, body): Reply) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: &str = "aa00000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn passes_a_message_to_the_computer_and_back() {
        let relay = Relay::new();
        let path = format!("/v1/rooms/{ROOM}");
        // Nobody listening yet.
        assert_eq!(
            relay
                .respond("POST", &format!("{path}/ask"), b"aGk=".to_vec())
                .0,
            503
        );

        let computer = relay.clone();
        let poll = path.clone();
        let mac = std::thread::spawn(move || {
            let (status, _, body) = computer.respond("GET", &format!("{poll}/next"), Vec::new());
            assert_eq!(status, 200);
            assert!(body.contains("\"body\":\"aGk=\""), "{body}");
            let id = body
                .split("\"id\":")
                .nth(1)
                .unwrap()
                .split(',')
                .next()
                .unwrap()
                .to_string();
            computer.respond("POST", &format!("{poll}/reply/{id}"), b"b2s=".to_vec())
        });
        // Let the computer start polling, so the room counts as online.
        let start = Instant::now();
        while relay
            .lock()
            .rooms
            .get(ROOM)
            .and_then(|r| r.polled)
            .is_none()
        {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        let (status, _, body) = relay.respond("POST", &format!("{path}/ask"), b"aGk=".to_vec());
        assert_eq!((status, body.as_str()), (200, "b2s="));
        assert_eq!(mac.join().unwrap().0, 200);
    }

    #[test]
    fn refuses_bad_rooms_and_unsealed_bodies() {
        let relay = Relay::new();
        assert_eq!(
            relay.respond("GET", "/v1/rooms/short/next", Vec::new()).0,
            404
        );
        let ask = format!("/v1/rooms/{ROOM}/ask");
        assert_eq!(relay.respond("POST", &ask, b"{\"x\":1}".to_vec()).0, 400);
        assert_eq!(relay.respond("GET", "/v1/hello", Vec::new()).0, 200);
    }

    #[test]
    fn serves_over_a_real_socket() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || Relay::new().serve(listener));
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(b"GET /v1/hello HTTP/1.1\r\n\r\n").unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        assert!(
            out.starts_with("HTTP/1.1 200 OK") && out.ends_with("\"api\":1}"),
            "{out}"
        );
    }
}
