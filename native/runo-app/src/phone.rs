//! The phone bridge: a small HTTP API that Runo's mobile app talks to.
//!
//! `runo --phone [address]` serves it without a window. Reads come straight from the
//! saved state (`State::peek`) and the live approvals folder, so the phone sees what the
//! desktop app sees. Permission requests are answered right away, the same way the
//! corner card answers them. Everything that starts or stops an agent (a follow-up, a
//! plan approval, answers to questions, Stop) is left in `~/.runo/phone-inbox`, and the
//! running app picks it up within a couple of seconds and runs it as if it were typed.
//!
//! Every request needs `Authorization: Bearer <token>`. The token is made on first run
//! and kept in `~/.runo/phone.json`; the pairing link printed at start carries it. The
//! bridge speaks plain HTTP, so reach it over a private network such as Tailscale or
//! your home Wi-Fi, never straight from the internet.

use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The port the bridge listens on unless an address is given.
pub const PORT: u16 = 7631;
/// The API version in every path (`/v1/...`) and in `/v1/hello`.
const API: u32 = 1;
/// Messages sent per session; the phone shows the recent conversation.
const MESSAGES: usize = 200;
/// The longest request body accepted: prompts, answers and approvals are small.
const BODY_MAX: usize = 256 * 1024;
const PREVIEW_CHARS: usize = 160;

/// Something the phone asked the running app to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// A follow-up prompt; it waits in the session's queue while the agent works.
    Prompt {
        session: u64,
        text: String,
    },
    Stop {
        session: u64,
    },
    /// Approve the plan the agent proposed and let it build.
    ApprovePlan {
        session: u64,
    },
    /// Allow tools the agent was denied, for one turn, and continue.
    Allow {
        session: u64,
        tools: Vec<String>,
    },
    /// Answers to the agent's questions, one pick per question (`None` skips it).
    Answers {
        session: u64,
        picks: Vec<Option<String>>,
    },
}

impl Request {
    fn session(&self) -> u64 {
        match self {
            Request::Prompt { session, .. }
            | Request::Stop { session }
            | Request::ApprovePlan { session }
            | Request::Allow { session, .. }
            | Request::Answers { session, .. } => *session,
        }
    }
}

/// Where the phone's requests wait for the app.
pub fn inbox(base: &Path) -> PathBuf {
    base.join("phone-inbox")
}

/// Leave a request for the app, under a temporary name first so it never reads half a file.
pub fn send(base: &Path, request: &Request) -> std::io::Result<()> {
    let dir = inbox(base);
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = format!("{stamp:032}-{n}");
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(request)?)?;
    std::fs::rename(tmp, dir.join(format!("{name}.json")))
}

/// Take every waiting request, oldest first.
pub fn take(base: &Path) -> Vec<Request> {
    let Ok(entries) = std::fs::read_dir(inbox(base)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read(&path).ok();
            let _ = std::fs::remove_file(&path);
            serde_json::from_slice(&text?).ok()
        })
        .collect()
}

#[derive(Default, Serialize, Deserialize)]
struct Config {
    token: String,
}

/// The bridge's token, made on first use.
pub fn token(base: &Path) -> String {
    let path = base.join("phone.json");
    let config: Config = store::read_json_or_backup(&path);
    if config.token.len() >= 32 {
        return config.token;
    }
    let token = new_token();
    store::write_json(
        &path,
        &Config {
            token: token.clone(),
        },
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    token
}

/// 256 random bits from the OS, as hex.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("the OS has no random source");
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compare without stopping at the first difference, so timing says nothing about the token.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// `runo --phone [address] [--relay URL]`: serve the bridge until stopped.
pub fn serve(args: &[String]) -> i32 {
    let mut address = format!("0.0.0.0:{PORT}");
    let mut relay = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--relay" => relay = rest.next().map(|u| u.trim_end_matches('/').to_string()),
            _ => address = arg.clone(),
        }
    }
    let base = store::data_dir();
    let token = token(&base);
    let listener = match TcpListener::bind(&address) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("runo: could not listen on {address}: {e}");
            return 1;
        }
    };
    let bound = listener.local_addr().ok();
    let port = bound.map(|a| a.port()).unwrap_or(PORT);
    let host = match bound.map(|a| a.ip()) {
        Some(ip) if !ip.is_unspecified() => ip.to_string(),
        _ => local_ip().unwrap_or_else(|| "this-computer".into()),
    };
    let url = format!("http://{host}:{port}");
    println!("Runo phone bridge on {address}");
    if let Some(relay) = &relay {
        println!("Away from home the phone goes through the relay at {relay}.");
    }
    println!();
    let link = pair_link(&url, &token, relay.as_deref());
    println!("Scan this with the Runo mobile app, or copy the link below to your phone:");
    println!();
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        println!("{}", qr(&link));
    }
    println!("  {link}");
    println!();
    if relay.is_none() {
        println!("This works on the same network. To reach this computer from anywhere, run");
        println!("runo-relay on a server and add --relay https://<that server>.");
    }
    println!("Keep Runo open: prompts and Stop are carried out by the app.");
    if let Some(relay) = relay {
        let (base, token) = (base.clone(), token.clone());
        std::thread::spawn(move || relay_loop(&relay, &base, &token));
    }
    accept(listener, &base, &token);
    0
}

/// Answer connections until the listener fails, each on its own thread.
fn accept(listener: TcpListener, base: &Path, token: &str) {
    for stream in listener.incoming().flatten() {
        let (base, token) = (base.to_path_buf(), token.to_string());
        std::thread::spawn(move || handle(stream, &base, &token));
    }
}

/// The link the mobile app pairs with: this computer's address, the token, and the relay
/// to fall back on away from home.
pub fn pair_link(url: &str, token: &str, relay: Option<&str>) -> String {
    let mut link = format!("runo://pair?url={}&token={token}", encode(url));
    if let Some(relay) = relay {
        link.push_str(&format!("&relay={}", encode(relay)));
    }
    link
}

/// The link as a QR code drawn with half-block characters, two rows per line, in black on
/// white whatever the terminal's colours, so phones can read it.
fn qr(text: &str) -> String {
    use qrcode::render::unicode::Dense1x2;
    let Ok(code) = qrcode::QrCode::new(text) else {
        return String::new();
    };
    code.render::<Dense1x2>()
        .quiet_zone(true)
        .build()
        .lines()
        .map(|line| format!("  \x1b[30;47m{line}\x1b[0m\n"))
        .collect()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// This computer's address on the network it reaches the internet through. Nothing is
/// sent: connecting a UDP socket only picks the route.
fn local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// One parsed HTTP request.
#[derive(Debug, Default)]
struct Http {
    method: String,
    path: String,
    auth: String,
    if_none_match: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut impl Read) -> Option<Http> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let mut http = Http {
        method: parts.next()?.to_string(),
        path: parts.next()?.to_string(),
        ..Default::default()
    };
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
        let Some((name, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok()?,
            "authorization" => http.auth = value.to_string(),
            "if-none-match" => http.if_none_match = value.to_string(),
            _ => {}
        }
    }
    if length > BODY_MAX {
        return None;
    }
    http.body = vec![0; length];
    reader.read_exact(&mut http.body).ok()?;
    Some(http)
}

/// A response: status code and JSON body.
type Reply = (u16, Value);

fn handle(mut stream: TcpStream, base: &Path, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Some(http) = read_request(&mut stream) else {
        let _ = write_reply(&mut stream, (400, json!({ "error": "bad request" })), "");
        return;
    };
    let (status, body) = respond(&http, base, token);
    let _ = write_reply(&mut stream, (status, body), &http.if_none_match);
}

fn write_reply(
    stream: &mut impl Write,
    (status, body): Reply,
    if_none_match: &str,
) -> std::io::Result<()> {
    let (status, text, tag) = finish(status, &body, if_none_match);
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nETag: {tag}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )?;
    stream.flush()
}

/// The body as sent, its ETag, and 304 with no body when the phone already has it.
fn finish(status: u16, body: &Value, if_none_match: &str) -> (u16, String, String) {
    let text = body.to_string();
    let tag = format!("\"{:016x}\"", fnv(text.as_bytes()));
    if status == 200 && if_none_match == tag {
        (304, String::new(), tag)
    } else {
        (status, text, tag)
    }
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Route one request. Separate from the socket so tests can call it directly.
fn respond(http: &Http, base: &Path, token: &str) -> Reply {
    let given = http.auth.strip_prefix("Bearer ").unwrap_or("");
    if !same(given, token) {
        return (
            401,
            json!({ "error": "This phone isn't paired. Pair it again with the link from `runo --phone`." }),
        );
    }
    let path = http.path.split('?').next().unwrap_or("");
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let body: Value = if http.body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&http.body) {
            Ok(v) => v,
            Err(_) => return (400, json!({ "error": "The body isn't JSON." })),
        }
    };
    let get = http.method == "GET";
    let post = http.method == "POST";
    match parts.as_slice() {
        ["v1", "hello"] if get => (
            200,
            json!({ "app": "runo", "version": env!("CARGO_PKG_VERSION"), "api": API }),
        ),
        ["v1", "sessions"] if get => (200, sessions(base)),
        ["v1", "sessions", id] if get => match id.parse().ok().and_then(|id| session(base, id)) {
            Some(v) => (200, v),
            None => not_found(),
        },
        ["v1", "sessions", id, action] if post => {
            let Some(id) = id.parse::<u64>().ok().filter(|id| known(base, *id)) else {
                return not_found();
            };
            let request = match *action {
                "prompt" => match body["text"].as_str().map(str::trim) {
                    Some(text) if !text.is_empty() => Request::Prompt {
                        session: id,
                        text: text.to_string(),
                    },
                    _ => return (400, json!({ "error": "Write a message first." })),
                },
                "stop" => Request::Stop { session: id },
                "plan" => Request::ApprovePlan { session: id },
                "allow" => Request::Allow {
                    session: id,
                    tools: strings(&body["tools"]),
                },
                "answers" => Request::Answers {
                    session: id,
                    picks: body["picks"]
                        .as_array()
                        .map(|a| a.iter().map(|p| p.as_str().map(str::to_string)).collect())
                        .unwrap_or_default(),
                },
                _ => return not_found(),
            };
            match send(base, &request) {
                Ok(()) => (202, json!({ "ok": true })),
                Err(e) => (
                    500,
                    json!({ "error": format!("Couldn't hand this to Runo: {e}") }),
                ),
            }
        }
        ["v1", "approvals", id] if post => {
            let folder = base.join("approvals");
            let pending = approvals::pending(&folder);
            let Some(request) = pending.iter().find(|r| r.id == *id) else {
                return (
                    404,
                    json!({ "error": "That request was already answered or timed out." }),
                );
            };
            let answer = if body["allow"].as_bool() == Some(true) {
                approvals::Answer::Allow {
                    updated_input: request.input.clone(),
                }
            } else {
                approvals::Answer::Deny {
                    message: body["message"]
                        .as_str()
                        .filter(|m| !m.trim().is_empty())
                        .unwrap_or("The person denied this from their phone.")
                        .to_string(),
                }
            };
            match approvals::answer(&folder, id, &answer) {
                Ok(()) => (200, json!({ "ok": true })),
                Err(_) => (
                    404,
                    json!({ "error": "That request was already answered or timed out." }),
                ),
            }
        }
        ["v1", ..] if !get && !post => (405, json!({ "error": "method not allowed" })),
        _ => not_found(),
    }
}

fn not_found() -> Reply {
    (404, json!({ "error": "not found" }))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn peek(base: &Path) -> State {
    crate::persist::read(&base.join("state.json"))
        .map(|(state, ..)| state)
        .unwrap_or_default()
}

fn known(base: &Path, id: u64) -> bool {
    peek(base).session(id).is_some()
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Idle => "idle",
        Status::Working => "working",
        Status::Done => "done",
        Status::Failed => "failed",
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Agent => "agent",
        Role::System => "system",
    }
}

/// One line of a reply for the list, without code fence markers.
fn preview(text: &str) -> String {
    let line = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ");
    let mut cut: String = line.chars().take(PREVIEW_CHARS).collect();
    if cut.len() < line.len() {
        cut.push('…');
    }
    cut
}

/// What a session waits on from the person, if anything.
fn needs(s: &Session, pending: &[approvals::Request]) -> Value {
    if s.archived {
        return Value::Null;
    }
    if let Some(r) = pending.iter().find(|r| r.dir == s.dir) {
        return json!({ "kind": "approval", "text": format!("Wants to {}", approvals::describe(&r.tool, &r.input)) });
    }
    match alerts::waiting_on(s) {
        Some((_, alerts::Waiting::Denied(tools))) => {
            json!({ "kind": "denied", "text": format!("Wants to use {}", tools.join(", ")) })
        }
        Some((_, alerts::Waiting::Questions(q))) => json!({
            "kind": "questions",
            "text": q.first().map(|q| q.question.clone()).unwrap_or_default(),
        }),
        Some((_, alerts::Waiting::Plan)) => {
            json!({ "kind": "plan", "text": "Proposed a plan and waits for your go-ahead" })
        }
        None => Value::Null,
    }
}

fn summary(s: &Session, pending: &[approvals::Request]) -> Value {
    let last = s.messages.iter().rev().find(|m| !m.text.trim().is_empty());
    json!({
        "id": s.id,
        "project": s.project_id,
        "title": s.title,
        "agent": agent::provider(&s.provider).name,
        "branch": s.branch,
        "status": status_name(s.status),
        "unread": s.unread,
        "pinned": s.pinned,
        "archived": s.archived,
        "elsewhere": s.elsewhere(),
        "updated": last.map(|m| m.time.clone()).unwrap_or_default(),
        "preview": last.map(|m| preview(&m.text)).unwrap_or_default(),
        "needs": needs(s, pending),
    })
}

fn sessions(base: &Path) -> Value {
    let state = peek(base);
    let pending = approvals::pending(&base.join("approvals"));
    json!({
        "projects": state.projects.iter().map(|p| json!({
            "id": p.id,
            "name": p.name,
            "color": p.color,
        })).collect::<Vec<_>>(),
        "sessions": state.sessions.iter().map(|s| summary(s, &pending)).collect::<Vec<_>>(),
    })
}

fn session(base: &Path, id: u64) -> Option<Value> {
    let state = peek(base);
    let s = state.session(id)?;
    let pending = approvals::pending(&base.join("approvals"));
    let skip = s.messages.len().saturating_sub(MESSAGES);
    Some(json!({
        "session": summary(s, &pending),
        "messages": s.messages.iter().skip(skip).map(|m| json!({
            "role": role_name(m.role),
            // A playbook's message holds the whole prompt; the phone shows its name.
            "text": if m.playbook.is_empty() { m.text.clone() } else { m.note.clone() },
            "playbook": m.playbook,
            "steps": m.steps,
            "time": m.time,
            "meta": m.meta,
            "plan": m.plan,
            "questions": m.questions.iter().map(|q| json!({
                "question": q.question,
                "options": q.options,
            })).collect::<Vec<_>>(),
            "denied": m.denied,
        })).collect::<Vec<_>>(),
        "queue": s.queue.iter().map(|q| q.text.clone()).collect::<Vec<_>>(),
        "approvals": pending.iter().filter(|r| r.dir == s.dir).map(|r| json!({
            "id": r.id,
            "tool": r.tool,
            "what": approvals::describe(&r.tool, &r.input),
            "asked": r.asked,
        })).collect::<Vec<_>>(),
    }))
}

// ---------- the relay ----------
//
// Away from home the phone reaches this computer through `runo-relay`. Both sides only
// connect out to it. Each message is sealed with ChaCha20-Poly1305 under a key made from
// the pairing token, so the relay passes along text it can't read or forge. The room
// the two meet in is named by another hash of the token.

const KEY_LABEL: &str = "runo-phone-key-v1:";
const ROOM_LABEL: &str = "runo-phone-room-v1:";
const ASK_AAD: &[u8] = b"runo-ask";
const REPLY_AAD: &[u8] = b"runo-reply";
/// How far a sealed request's clock may be from this one's, in seconds.
const SKEW: u64 = 120;

fn derive(label: &str, token: &str) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(format!("{label}{token}").as_bytes()).into()
}

/// The relay room for a token.
pub fn room(token: &str) -> String {
    hex(&derive(ROOM_LABEL, token))
}

/// Seal `plain` as base64 of nonce then ciphertext. Returns the nonce too.
fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> (String, [u8; 12]) {
    use base64::Engine;
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).expect("the OS has no random source");
    let cipher = chacha20poly1305::ChaCha20Poly1305::new(key.into());
    let sealed = cipher
        .encrypt(&nonce.into(), Payload { msg: plain, aad })
        .expect("sealing never fails");
    let mut out = nonce.to_vec();
    out.extend(sealed);
    (base64::engine::general_purpose::STANDARD.encode(out), nonce)
}

/// Open what `seal` made with the same key and `aad`; `None` if it was made otherwise.
fn open(key: &[u8; 32], aad: &[u8], sealed: &str) -> Option<(Vec<u8>, [u8; 12])> {
    use base64::Engine;
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    let raw = base64::engine::general_purpose::STANDARD
        .decode(sealed.trim())
        .ok()?;
    if raw.len() < 12 + 16 {
        return None;
    }
    let nonce: [u8; 12] = raw[..12].try_into().ok()?;
    let cipher = chacha20poly1305::ChaCha20Poly1305::new(key.into());
    let plain = cipher
        .decrypt(
            &nonce.into(),
            Payload {
                msg: &raw[12..],
                aad,
            },
        )
        .ok()?;
    Some((plain, nonce))
}

/// What the phone asks, inside the seal: the request it would send directly.
#[derive(Debug, Serialize, Deserialize)]
struct Ask {
    method: String,
    path: String,
    #[serde(default)]
    if_none_match: String,
    #[serde(default)]
    body: Value,
    /// When it was sealed, in seconds since 1970.
    time: u64,
}

/// The answer, inside the seal.
#[derive(Debug, Serialize, Deserialize)]
struct Answer {
    status: u16,
    etag: String,
    body: Value,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Requests already answered, by nonce, so a copy sent again is ignored.
#[derive(Default)]
struct Seen(HashMap<[u8; 12], u64>);

impl Seen {
    fn first(&mut self, nonce: [u8; 12], now: u64) -> bool {
        self.0.retain(|_, at| now.saturating_sub(*at) <= 2 * SKEW);
        self.0.insert(nonce, now).is_none()
    }
}

/// Open a request from the relay, answer it like a direct one, and seal the answer.
/// Anything not sealed with this token, too old, or seen before gets no answer.
fn answer_sealed(sealed: &str, base: &Path, token: &str, seen: &mut Seen) -> Option<String> {
    let key = derive(KEY_LABEL, token);
    let (plain, nonce) = open(&key, ASK_AAD, sealed)?;
    let ask: Ask = serde_json::from_slice(&plain).ok()?;
    let now = now_secs();
    if ask.time.abs_diff(now) > SKEW || !seen.first(nonce, now) {
        return None;
    }
    let http = Http {
        method: ask.method,
        path: ask.path,
        auth: format!("Bearer {token}"),
        if_none_match: ask.if_none_match.clone(),
        body: if ask.body.is_null() {
            Vec::new()
        } else {
            ask.body.to_string().into_bytes()
        },
    };
    let (status, body) = respond(&http, base, token);
    let (status, _, etag) = finish(status, &body, &ask.if_none_match);
    let answer = Answer {
        status,
        etag,
        body: if status == 304 { Value::Null } else { body },
    };
    let aad = [REPLY_AAD, &nonce].concat();
    Some(seal(&key, &aad, &serde_json::to_vec(&answer).ok()?).0)
}

/// Run `curl` (as the rest of Runo does for HTTP) and return the status and body.
fn curl(args: &[&str], input: Option<&str>) -> Result<(u16, String), String> {
    let mut child = std::process::Command::new("curl")
        .args(["-sS", "-w", "\n%{http_code}"])
        .args(args)
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run curl: {e}"))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    match code.trim().parse() {
        Ok(code) if code != 0 => Ok((code, body.to_string())),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// Keep asking the relay for the phone's messages and answer them, until stopped.
fn relay_loop(relay: &str, base: &Path, token: &str) {
    let rooms = format!("{relay}/v1/rooms/{}", room(token));
    let next = format!("{rooms}/next");
    let mut seen = Seen::default();
    let mut wait = Duration::from_secs(1);
    let mut down: Option<String> = None;
    loop {
        let problem = match curl(&["--max-time", "40", &next], None) {
            Ok((200, text)) => {
                let message: Value = serde_json::from_str(&text).unwrap_or_default();
                let id = message["id"].as_u64();
                let sealed = message["body"].as_str().unwrap_or("");
                if let (Some(id), Some(reply)) = (id, answer_sealed(sealed, base, token, &mut seen))
                {
                    let url = format!("{rooms}/reply/{id}");
                    let args = [
                        "--max-time",
                        "15",
                        "-X",
                        "POST",
                        "--data-binary",
                        "@-",
                        &url,
                    ];
                    let _ = curl(&args, Some(&reply));
                }
                None
            }
            Ok((204, _)) => None,
            Ok((code, text)) => Some(format!("the relay answered {code}: {}", text.trim())),
            Err(e) => Some(e),
        };
        match problem {
            None => {
                if down.take().is_some() {
                    println!("Connected to the relay again.");
                }
                wait = Duration::from_secs(1);
            }
            Some(problem) => {
                if down.as_deref() != Some(problem.as_str()) {
                    eprintln!("runo: can't reach the relay at {relay} ({problem}); retrying.");
                    down = Some(problem);
                }
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_secs(30));
            }
        }
    }
}

impl RunoApp {
    /// Carry out what the phone asked for, as if it were typed here.
    pub(crate) fn take_phone_requests(&mut self, cx: &mut Context<Self>) {
        if self.demo {
            return;
        }
        for request in take(&store::data_dir()) {
            let id = request.session();
            let Some(session) = self.state.session(id) else {
                continue;
            };
            let provider = session.provider.clone();
            let running = self.handles.contains_key(&Target::Session(id));
            let opts = cx.global::<AppSettings>().agent(&provider);
            match request {
                Request::Prompt { text, .. } => self.start_turn(id, text, Vec::new(), &opts),
                Request::Stop { .. } => self.stop_session(id),
                Request::ApprovePlan { .. } if !running => {
                    // A playbook that only reads must be widened on this computer.
                    let Some(s) = self.state.session_mut(id) else {
                        continue;
                    };
                    if !s.limits.is_empty() && !s.limits.iter().any(|p| p == "edit") {
                        self.notice = Some(format!(
                            "“{}” runs a playbook that only reads, so its plan has to be approved here.",
                            s.title
                        ));
                        continue;
                    }
                    s.opts.insert("mode".into(), "edit".into());
                    self.answer_session(
                        id,
                        "The plan is approved. Go ahead and implement it.".into(),
                        Vec::new(),
                        cx,
                    );
                }
                Request::Allow { tools, .. } if !running && !tools.is_empty() => {
                    let prompt = format!(
                        "You now have permission to use {}. Continue.",
                        tools.join(", ")
                    );
                    self.answer_session(id, prompt, tools, cx);
                }
                Request::Answers { picks, .. } if !running => {
                    let Some((_, alerts::Waiting::Questions(questions))) =
                        alerts::waiting_on(session)
                    else {
                        continue;
                    };
                    if picks.iter().all(Option::is_none) {
                        continue;
                    }
                    let prompt = turns::answers_prompt(&questions, &picks);
                    self.answer_session(id, prompt, Vec::new(), cx);
                }
                _ => continue,
            }
            if let Some(s) = self.state.session(id) {
                let title = s.title.clone();
                self.state
                    .log(id, format!("Answered from the phone: {title}"));
            }
            self.save();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` brings in gpui's own `test` attribute; use the built-in one.
    use std::prelude::v1::test;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "runo-phone-{name}-{}-{}",
            std::process::id(),
            new_token()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn call(base: &Path, method: &str, path: &str, body: Value) -> Reply {
        let http = Http {
            method: method.into(),
            path: path.into(),
            auth: "Bearer secret-token".into(),
            body: if body.is_null() {
                Vec::new()
            } else {
                body.to_string().into_bytes()
            },
            ..Default::default()
        };
        respond(&http, base, "secret-token")
    }

    fn seed(base: &Path) {
        let mut state = State::default();
        state.projects.push(Project {
            id: 1,
            name: "runo".into(),
            path: "/p/runo".into(),
            ..Default::default()
        });
        let mut s = Session {
            id: 7,
            project_id: 1,
            title: "Fix the sidebar".into(),
            provider: "claude".into(),
            dir: "/p/runo/.wt/7".into(),
            status: Status::Done,
            ..Default::default()
        };
        s.messages.push(Message::new(Role::User, "Fix it"));
        let mut reply = Message::new(Role::Agent, "Which way?");
        reply.questions.push(store::Question {
            question: "Left or right?".into(),
            options: vec!["Left".into(), "Right".into()],
        });
        s.messages.push(reply);
        state.sessions.push(s);
        let mut written = Default::default();
        crate::persist::write_all(base, &mut state, &mut written);
    }

    #[test]
    fn refuses_without_the_token() {
        let base = temp("auth");
        let http = Http {
            method: "GET".into(),
            path: "/v1/hello".into(),
            auth: "Bearer wrong".into(),
            ..Default::default()
        };
        assert_eq!(respond(&http, &base, "secret-token").0, 401);
        assert_eq!(call(&base, "GET", "/v1/hello", Value::Null).0, 200);
    }

    #[test]
    fn lists_sessions_with_what_they_need() {
        let base = temp("list");
        seed(&base);
        let (status, body) = call(&base, "GET", "/v1/sessions", Value::Null);
        assert_eq!(status, 200);
        assert_eq!(body["projects"][0]["name"], "runo");
        let s = &body["sessions"][0];
        assert_eq!(s["title"], "Fix the sidebar");
        assert_eq!(s["status"], "done");
        assert_eq!(s["needs"]["kind"], "questions");
        assert_eq!(s["needs"]["text"], "Left or right?");

        let (status, body) = call(&base, "GET", "/v1/sessions/7", Value::Null);
        assert_eq!(status, 200);
        assert_eq!(body["messages"][1]["questions"][0]["options"][1], "Right");
        assert_eq!(call(&base, "GET", "/v1/sessions/8", Value::Null).0, 404);
    }

    #[test]
    fn actions_wait_in_the_inbox() {
        let base = temp("inbox");
        seed(&base);
        assert_eq!(
            call(
                &base,
                "POST",
                "/v1/sessions/7/prompt",
                json!({ "text": "  " })
            )
            .0,
            400
        );
        assert_eq!(
            call(
                &base,
                "POST",
                "/v1/sessions/7/prompt",
                json!({ "text": "Go left" })
            )
            .0,
            202
        );
        assert_eq!(
            call(&base, "POST", "/v1/sessions/7/stop", Value::Null).0,
            202
        );
        assert_eq!(
            call(
                &base,
                "POST",
                "/v1/sessions/7/answers",
                json!({ "picks": ["Left"] })
            )
            .0,
            202
        );
        assert_eq!(
            call(&base, "POST", "/v1/sessions/9/stop", Value::Null).0,
            404
        );
        assert_eq!(
            take(&base),
            vec![
                Request::Prompt {
                    session: 7,
                    text: "Go left".into()
                },
                Request::Stop { session: 7 },
                Request::Answers {
                    session: 7,
                    picks: vec![Some("Left".into())]
                },
            ]
        );
        assert!(take(&base).is_empty());
    }

    #[test]
    fn answers_a_live_approval() {
        let base = temp("approve");
        seed(&base);
        let folder = base.join("approvals");
        let asking = {
            let folder = folder.clone();
            std::thread::spawn(move || {
                approvals::ask(
                    &folder,
                    "/p/runo/.wt/7",
                    "Bash",
                    &json!({ "command": "cargo test" }),
                    Duration::from_secs(10),
                )
            })
        };
        let id = loop {
            if let Some(r) = approvals::pending(&folder).pop() {
                break r.id;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let (_, body) = call(&base, "GET", "/v1/sessions/7", Value::Null);
        assert_eq!(body["approvals"][0]["what"], "run `cargo test`");
        assert_eq!(body["session"]["needs"]["kind"], "approval");
        let path = format!("/v1/approvals/{id}");
        assert_eq!(call(&base, "POST", &path, json!({ "allow": true })).0, 200);
        assert!(matches!(
            asking.join().unwrap(),
            approvals::Answer::Allow { .. }
        ));
        assert_eq!(call(&base, "POST", &path, json!({ "allow": true })).0, 404);
    }

    #[test]
    fn parses_requests_and_answers_304() {
        let raw = b"POST /v1/sessions/7/prompt HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer t\r\nContent-Length: 15\r\n\r\n{\"text\":\"hi!\"}\n";
        let http = read_request(&mut &raw[..]).unwrap();
        assert_eq!(http.method, "POST");
        assert_eq!(http.path, "/v1/sessions/7/prompt");
        assert_eq!(http.auth, "Bearer t");
        assert_eq!(http.body.len(), 15);

        let mut out = Vec::new();
        write_reply(&mut out, (200, json!({ "a": 1 })), "").unwrap();
        let text = String::from_utf8(out).unwrap();
        let tag = text
            .lines()
            .find_map(|l| l.strip_prefix("ETag: "))
            .unwrap()
            .to_string();
        let mut again = Vec::new();
        write_reply(&mut again, (200, json!({ "a": 1 })), &tag).unwrap();
        assert!(String::from_utf8(again)
            .unwrap()
            .starts_with("HTTP/1.1 304"));
    }

    #[test]
    fn serves_over_a_real_socket() {
        let base = temp("socket");
        seed(&base);
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = base.clone();
        std::thread::spawn(move || accept(listener, &served, "secret-token"));
        let ask = |request: String| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            let mut out = String::new();
            stream.read_to_string(&mut out).unwrap();
            out
        };
        let list =
            ask("GET /v1/sessions HTTP/1.1\r\nAuthorization: Bearer secret-token\r\n\r\n".into());
        assert!(list.starts_with("HTTP/1.1 200 OK"), "{list}");
        assert!(list.contains("\"Fix the sidebar\""));
        let tag = list
            .lines()
            .find_map(|l| l.strip_prefix("ETag: "))
            .unwrap()
            .to_string();
        let again = ask(format!(
            "GET /v1/sessions HTTP/1.1\r\nAuthorization: Bearer secret-token\r\nIf-None-Match: {tag}\r\n\r\n"
        ));
        assert!(again.starts_with("HTTP/1.1 304"), "{again}");
        let body = r#"{"text":"Go left"}"#;
        let sent = ask(format!(
            "POST /v1/sessions/7/prompt HTTP/1.1\r\nAuthorization: Bearer secret-token\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ));
        assert!(sent.starts_with("HTTP/1.1 202"), "{sent}");
        assert_eq!(take(&base).len(), 1);
        let refused = ask("GET /v1/sessions HTTP/1.1\r\n\r\n".into());
        assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
    }

    /// What the phone app does: seal a request the way `answer_sealed` expects.
    fn phone_ask(token: &str, method: &str, path: &str, body: Value) -> (String, [u8; 12]) {
        let ask = Ask {
            method: method.into(),
            path: path.into(),
            if_none_match: String::new(),
            body,
            time: now_secs(),
        };
        seal(
            &derive(KEY_LABEL, token),
            ASK_AAD,
            &serde_json::to_vec(&ask).unwrap(),
        )
    }

    fn phone_open(token: &str, nonce: [u8; 12], sealed: &str) -> Answer {
        let aad = [REPLY_AAD, &nonce].concat();
        let (plain, _) = open(&derive(KEY_LABEL, token), &aad, sealed).unwrap();
        serde_json::from_slice(&plain).unwrap()
    }

    #[test]
    fn sealed_requests_are_answered_once_and_only_with_the_token() {
        let base = temp("sealed");
        seed(&base);
        let token = "secret-token";
        let mut seen = Seen::default();
        let (ask, nonce) = phone_ask(token, "GET", "/v1/sessions/7", Value::Null);
        let reply = answer_sealed(&ask, &base, token, &mut seen).unwrap();
        let answer = phone_open(token, nonce, &reply);
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body["session"]["title"], "Fix the sidebar");
        // The same message again, another token's, or a changed one: no answer.
        assert!(answer_sealed(&ask, &base, token, &mut seen).is_none());
        let (other, _) = phone_ask("other-token", "GET", "/v1/sessions", Value::Null);
        assert!(answer_sealed(&other, &base, token, &mut seen).is_none());
        let (ask, _) = phone_ask(token, "GET", "/v1/sessions", Value::Null);
        let mut bytes = ask.into_bytes();
        let last = bytes.len() - 3;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(answer_sealed(&tampered, &base, token, &mut seen).is_none());
        // A reply can't be passed off as the answer to a different request.
        let (ask, _) = phone_ask(token, "GET", "/v1/hello", Value::Null);
        let reply = answer_sealed(&ask, &base, token, &mut seen).unwrap();
        let aad = [REPLY_AAD, &[0u8; 12][..]].concat();
        assert!(open(&derive(KEY_LABEL, token), &aad, &reply).is_none());
        assert_eq!(room(token).len(), 64);
        assert_ne!(room(token), hex(&derive(KEY_LABEL, token)));
    }

    #[test]
    fn reaches_the_computer_through_the_relay() {
        let base = temp("relay");
        seed(&base);
        let token = "secret-token";
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || runo_relay::Relay::new().serve(listener));
        let relay = format!("http://127.0.0.1:{port}");
        let (served, url) = (base.clone(), relay.clone());
        std::thread::spawn(move || relay_loop(&url, &served, token));

        let send = |sealed: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(
                stream,
                "POST /v1/rooms/{}/ask HTTP/1.1\r\nContent-Length: {}\r\n\r\n{sealed}",
                room(token),
                sealed.len()
            )
            .unwrap();
            let mut out = String::new();
            stream.read_to_string(&mut out).unwrap();
            out
        };
        // The computer has to be polling first; until then the relay says it's offline.
        let start = std::time::Instant::now();
        let (out, nonce) = loop {
            let (ask, nonce) = phone_ask(token, "GET", "/v1/sessions", Value::Null);
            let out = send(&ask);
            if !out.starts_with("HTTP/1.1 503") {
                break (out, nonce);
            }
            assert!(start.elapsed() < Duration::from_secs(10), "{out}");
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        let answer = phone_open(token, nonce, out.split("\r\n\r\n").nth(1).unwrap());
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body["sessions"][0]["title"], "Fix the sidebar");

        let (ask, nonce) = phone_ask(
            token,
            "POST",
            "/v1/sessions/7/prompt",
            json!({ "text": "Go left" }),
        );
        let out = send(&ask);
        let answer = phone_open(token, nonce, out.split("\r\n\r\n").nth(1).unwrap());
        assert_eq!(answer.status, 202);
        assert_eq!(
            take(&base),
            vec![Request::Prompt {
                session: 7,
                text: "Go left".into()
            }]
        );
    }

    #[test]
    fn previews_drop_code_fences() {
        assert_eq!(
            preview("Updated it.\n```sh\nnpm run shots\n```\nAll pass."),
            "Updated it. npm run shots All pass."
        );
    }

    #[test]
    fn pair_link_carries_url_and_token() {
        assert_eq!(
            pair_link("http://100.64.0.2:7631", "abc", None),
            "runo://pair?url=http%3A%2F%2F100.64.0.2%3A7631&token=abc"
        );
        assert_eq!(
            pair_link("http://h:1", "abc", Some("https://relay.example")),
            "runo://pair?url=http%3A%2F%2Fh%3A1&token=abc&relay=https%3A%2F%2Frelay.example"
        );
        let t = new_token();
        assert_eq!(t.len(), 64);
        assert_ne!(t, new_token());
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
