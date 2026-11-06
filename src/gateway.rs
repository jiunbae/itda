//! Web viewer gateway. Other devices (phones, tablets, Macs, Linux, any browser) open
//! http://<this PC>:<port>/, sign in with the access PIN and pick a device from the address
//! book (or this PC itself). The gateway logs in to the device with the saved password (over
//! SSH when the connection uses it; RDP is translated to RFB here), then hands the browser a
//! plain RFB stream over a WebSocket, so passwords never reach the browser. The browser can
//! also manage the address book and accounts. On macOS a second, local listener (127.0.0.1,
//! signed in with a token instead of a PIN) serves the app's own connection windows.

use crate::rfb::{self, Creds};
use crate::store;
use crate::tunnel::{self, SshCfg, Tunnel};
use crate::ws;
use serde_json::json;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INDEX: &str = include_str!("../web/viewer/index.html");
const CSS: &str = include_str!("../web/viewer/viewer.css");
const JS: &str = include_str!("../web/viewer/viewer.js");
const INTERACTION: &str = include_str!("../web/viewer/interaction.js");
const MONITORS: &str = include_str!("../web/viewer/monitors.js");
const NOVNC: &str = include_str!("../web/viewer/novnc.min.js");
const ICON: &str = include_str!("../assets/itda.svg");
const ICON_PNG: &[u8] = include_bytes!("../assets/itda-192.png");
const MANIFEST: &str = include_str!("../web/viewer/manifest.webmanifest");
const ICON_PNG_512: &[u8] = include_bytes!("../assets/itda-512.png");
const SW: &str = include_str!("../web/viewer/sw.js");

const SESSION_TTL: Duration = Duration::from_secs(12 * 3600);
const MAX_FAILS: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(60);

struct Running {
    port: u16,
    stop: Arc<AtomicBool>,
    /// The accept loop; joined on stop so the port is free again right away.
    thread: std::thread::JoinHandle<()>,
}

/// The web viewer for other devices (0.0.0.0, PIN).
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
/// Signed-in browser sessions: (token, since, from the local listener).
static TOKENS: Mutex<Vec<(String, Instant, bool)>> = Mutex::new(Vec::new());
/// Wrong PINs per source (address): (count, locked until). One device guessing locks only
/// itself out; ALL_FAILS bounds guessing from many addresses at once.
static FAILS: Mutex<Option<Fails>> = Mutex::new(None);
const MAX_ALL_FAILS: u32 = 50;
const MAX_SOURCES: usize = 1024;

#[derive(Default)]
struct Fails {
    per: HashMap<std::net::IpAddr, (u32, Option<Instant>)>,
    all: (u32, Option<Instant>),
}
/// Open browser sessions of the web viewer, closed when it stops.
/// Each entry leaves with its connection (`LiveConn`), so remote viewers don't pile up sockets.
static LIVE: Mutex<Vec<(u64, TcpStream)>> = Mutex::new(Vec::new());
struct LiveConn(u64);
impl LiveConn {
    fn track(c: &TcpStream) -> io::Result<Self> {
        let ticket = LOCAL_STREAM_ID.fetch_add(1, Relaxed);
        LIVE.lock().unwrap().push((ticket, c.try_clone()?));
        Ok(Self(ticket))
    }
}
impl Drop for LiveConn {
    fn drop(&mut self) {
        LIVE.lock().unwrap().retain(|(ticket, _)| *ticket != self.0);
    }
}
// Native monitor views share a relay; closing the final view must stop both sockets,
// without waiting for WebKit to collect cross-window JavaScript references.
static LOCAL_STREAMS: Mutex<Vec<(u64, String, TcpStream, TcpStream)>> = Mutex::new(Vec::new());
static LOCAL_STREAM_ID: AtomicU64 = AtomicU64::new(1);
struct LocalRelay(u64);
impl LocalRelay {
    fn track(id: &str, client: &TcpStream, remote: &TcpStream) -> io::Result<Self> {
        let ticket=LOCAL_STREAM_ID.fetch_add(1,Relaxed);
        LOCAL_STREAMS.lock().unwrap().push((ticket,id.into(),client.try_clone()?,remote.try_clone()?));
        Ok(Self(ticket))
    }
}
impl Drop for LocalRelay {
    fn drop(&mut self) { LOCAL_STREAMS.lock().unwrap().retain(|(ticket,_,_,_)| *ticket!=self.0); }
}
#[cfg_attr(windows, allow(dead_code))]
pub fn close_local(id: &str) {
    LOCAL_STREAMS.lock().unwrap().retain(|(_,owner,client,remote)| {
        if owner!=id {return true;}
        let _=client.shutdown(Shutdown::Both);let _=remote.shutdown(Shutdown::Both);false
    });
}

/// The local listener (the macOS app's connection windows): 127.0.0.1 only, signed in with
/// this token instead of a PIN. Runs beside the web viewer and is never stopped.
static LOCAL: Mutex<Option<(u16, String)>> = Mutex::new(None);

/// Passwords typed for one connection (not saved), for the app's own windows only.
static TYPED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

pub fn running() -> Option<u16> {
    RUNNING.lock().unwrap().as_ref().map(|r| r.port)
}

/// Start (or restart on a new port). Errors are for the person: port busy etc.
pub fn start(port: u16) -> io::Result<()> {
    if running() == Some(port) {
        return Ok(());
    }
    stop();
    let l = TcpListener::bind(("0.0.0.0", port)).map_err(|e| io::Error::new(e.kind(), format!("포트 {port}을(를) 열 수 없습니다: {e}")))?;
    let port = l.local_addr()?.port(); // the real one, also when asked for 0
    let stop = Arc::new(AtomicBool::new(false));
    let thread = serve(l, false, stop.clone())?;
    crate::log!("web viewer on port {port}");
    *RUNNING.lock().unwrap() = Some(Running { port, stop, thread });
    Ok(())
}

/// The URL that opens connection `id` in one of this machine's own windows, starting the
/// local listener (127.0.0.1, a free port) the first time.
#[cfg_attr(windows, allow(dead_code))]
pub fn local_url(id: &str) -> io::Result<String> {
    let mut g = LOCAL.lock().unwrap();
    if g.is_none() {
        let l = TcpListener::bind(("127.0.0.1", 0))?;
        let port = l.local_addr()?.port();
        serve(l, true, Arc::new(AtomicBool::new(false)))?;
        crate::log!("local viewer on port {port}");
        *g = Some((port, random_token()));
    }
    let (port, tok) = g.as_ref().unwrap();
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    // (ITDA_HP_TRACE: the viewer also times its own part of each frame.)
    let trace = if crate::trace::on() { "&trace=1" } else { "" };
    Ok(format!("http://127.0.0.1:{port}/auth?t={tok}&open={id}{trace}"))
}

/// Use this password (typed, not saved) for connection `id`; None forgets it.
#[cfg_attr(windows, allow(dead_code))]
pub fn set_typed(id: &str, pass: Option<String>) {
    let mut t = TYPED.lock().unwrap();
    t.retain(|(k, _)| k != id);
    if let Some(p) = pass {
        t.push((id.to_string(), p));
    }
}

pub(crate) fn typed(id: &str) -> Option<String> {
    TYPED.lock().unwrap().iter().find(|(k, _)| k == id).map(|(_, p)| p.clone())
}

fn random_token() -> String {
    let mut b = [0u8; 24];
    let _ = getrandom::getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

static OPEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const MAX_CONNECTIONS: usize = 128;
/// A request line or header longer than this, or a head slower than REQUEST_TIME, ends the
/// connection (before anyone has signed in, so it must not hold memory or a thread for long).
const MAX_LINE: u64 = 8 << 10;
const REQUEST_TIME: Duration = Duration::from_secs(if cfg!(test) { 1 } else { 15 });

/// Blocking accept, so a new connection is served at once; `stop()` wakes it with a connect.
fn serve(l: TcpListener, local: bool, stop: Arc<AtomicBool>) -> io::Result<std::thread::JoinHandle<()>> {
    Ok(std::thread::spawn(move || {
        while !stop.load(Relaxed) {
            match l.accept() {
                Ok(_) if stop.load(Relaxed) => break,
                Ok((c, peer)) => {
                    // Each connection has its own thread: past MAX_CONNECTIONS (viewer windows,
                    // file panes and page loads stay far below it) new ones are turned away.
                    if OPEN.fetch_add(1, Relaxed) >= MAX_CONNECTIONS {
                        OPEN.fetch_sub(1, Relaxed);
                        drop(c);
                        continue;
                    }
                    // Small frames (cursor, one tile) must not wait for Nagle + delayed ACK.
                    let _ = c.set_nodelay(true);
                    std::thread::spawn(move || {
                        if let Err(e) = handle(c, peer, local) {
                            crate::log!("web {peer}: {e}");
                        }
                        OPEN.fetch_sub(1, Relaxed);
                    });
                }
                Err(e) => {
                    crate::log!("web accept: {e}");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        crate::log!("web viewer stopped");
    }))
}

pub fn stop() {
    let r = RUNNING.lock().unwrap().take();
    if let Some(r) = r {
        r.stop.store(true, Relaxed);
        // Wake the blocked accept; if that fails the thread can't be joined, so leave it.
        match TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], r.port)), Duration::from_secs(1)) {
            Ok(_) => {
                let _ = r.thread.join();
            }
            Err(e) => crate::log!("web viewer stop: accept not woken: {e}"),
        }
    }
    for (_, s) in LIVE.lock().unwrap().drain(..) {
        let _ = s.shutdown(Shutdown::Both);
    }
    TOKENS.lock().unwrap().retain(|t| t.2);
    *FAILS.lock().unwrap() = None;
}

/// Addresses other devices can use: every LAN IPv4 (Tailscale's 100.x included).
pub fn urls(port: u16) -> Vec<Url> {
    let mut ips = local_ipv4();
    ips.retain(|ip| !ip.is_loopback() && !ip.is_link_local());
    ips.dedup();
    // Home Wi-Fi first (that is what the QR code encodes), Tailscale next; 172.16/12 is
    // usually a WSL/Hyper-V/Docker adapter that phones can't reach, so drop it when a
    // real LAN address exists.
    let rank = |ip: &Ipv4Addr| match ip.octets() {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        [100, b, ..] if (64..128).contains(&b) => 2,
        [172, b, ..] if (16..32).contains(&b) => 4,
        _ => 3,
    };
    ips.sort_by_key(|ip| (rank(ip), ip.octets()));
    if ips.iter().any(|ip| rank(ip) < 2) {
        ips.retain(|ip| rank(ip) != 4);
    }
    ips.into_iter()
        .map(|ip| Url { url: format!("http://{ip}:{port}/"), kind: if rank(&ip) == 2 { "tailscale" } else { "lan" } })
        .collect()
}

#[cfg(windows)]
fn local_ipv4() -> Vec<Ipv4Addr> {
    use std::net::ToSocketAddrs;
    let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into());
    (host.as_str(), 0)
        .to_socket_addrs()
        .map(|it| it.filter_map(|a| if let SocketAddr::V4(a) = a { Some(*a.ip()) } else { None }).collect())
        .unwrap_or_default()
}

/// macOS: the interfaces' IPv4 addresses, as ifconfig lists them.
#[cfg(not(windows))]
fn local_ipv4() -> Vec<Ipv4Addr> {
    let out = std::process::Command::new("/sbin/ifconfig").output().map(|o| o.stdout).unwrap_or_default();
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("inet ")?.split_whitespace().next()?.parse().ok())
        .collect()
}

#[derive(serde::Serialize)]
pub struct Url {
    pub url: String,
    /// "lan" | "tailscale"
    pub kind: &'static str,
}

// ---------------------------------------------------------------- HTTP

struct Req {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// Wait for more of the request, but never past `deadline` (each read gets only what is left,
/// so a client sending a byte at a time can't stretch it).
fn more(r: &mut BufReader<TcpStream>, deadline: Instant) -> io::Result<&[u8]> {
    let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero());
    let left = left.ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request too slow"))?;
    r.get_ref().set_read_timeout(Some(left))?;
    r.fill_buf()
}

/// One line of the request head, at most MAX_LINE long.
fn head_line(r: &mut BufReader<TcpStream>, deadline: Instant) -> io::Result<String> {
    let mut line = Vec::new();
    loop {
        let buf = more(r, deadline)?;
        if buf.is_empty() {
            break; // closed
        }
        let take = buf.iter().position(|&b| b == b'\n').map_or(buf.len(), |i| i + 1);
        line.extend_from_slice(&buf[..take]);
        r.consume(take);
        if line.len() as u64 > MAX_LINE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request line too long"));
        }
        if line.ends_with(b"\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

fn read_req(r: &mut BufReader<TcpStream>) -> io::Result<Req> {
    let started = Instant::now() + REQUEST_TIME;
    let line = head_line(r, started)?;
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let target = it.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_string(), query.to_string());
    let mut headers = HashMap::new();
    let mut lines = 0;
    loop {
        let h = head_line(r, started)?;
        if h.is_empty() || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
        lines += 1;
        if lines > 64 {
            return Err(io::Error::other("too many headers"));
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = Vec::with_capacity(len.min(64 << 10));
    while body.len() < len.min(64 << 10) {
        let buf = more(r, started)?;
        if buf.is_empty() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "request body cut short"));
        }
        let take = buf.len().min(len.min(64 << 10) - body.len());
        body.extend_from_slice(&buf[..take]);
        r.consume(take);
    }
    // (Back to the per-read timeout for what follows, e.g. a WebSocket.)
    r.get_ref().set_read_timeout(Some(Duration::from_secs(15)))?;
    Ok(Req { method, path, query, headers, body })
}

fn respond(c: &mut TcpStream, status: &str, mime: &str, extra: &str, body: &[u8]) -> io::Result<()> {
    send(c, status, mime, "no-store", extra, body)
}

/// A built-in file: cached, but revalidated by its content hash, so an update shows at once
/// while an unchanged noVNC (189 KB) is a 304 instead of a fresh download per window.
fn asset(c: &mut impl Write, req: &Req, mime: &str, body: &'static [u8]) -> io::Result<()> {
    let etag = etag(body);
    let extra = format!("ETag: {etag}\r\n");
    if req.headers.get("if-none-match").is_some_and(|v| matches_etag(v, &etag)) {
        // The head a 200 would have, Content-Length included (RFC 9110 §8.6), and no body.
        return send_head(c, "304 Not Modified", mime, "no-cache", &extra, body.len(), b"");
    }
    send(c, "200 OK", mime, "no-cache", &extra, body)
}

/// The strong ETag of a built-in file: its FNV-1a hash, worked out once per file.
fn etag(body: &'static [u8]) -> String {
    static TAGS: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    let mut tags = TAGS.lock().unwrap();
    let key = body.as_ptr() as usize;
    if let Some((_, t)) = tags.iter().find(|(k, _)| *k == key) {
        return t.clone();
    }
    let h = body.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    let t = format!("\"{h:016x}\"");
    tags.push((key, t.clone()));
    t
}

/// If-None-Match: "*" or any listed tag, compared weakly (W/ prefixes ignored).
fn matches_etag(header: &str, etag: &str) -> bool {
    let bare = |t: &str| t.trim().trim_start_matches("W/").to_string();
    header.split(',').any(|t| t.trim() == "*" || bare(t) == bare(etag))
}

fn send(c: &mut impl Write, status: &str, mime: &str, cache: &str, extra: &str, body: &[u8]) -> io::Result<()> {
    send_head(c, status, mime, cache, extra, body.len(), body)
}

/// Head with Content-Length `len`, then `body` (empty for a 304's head-only answer).
fn send_head(c: &mut impl Write, status: &str, mime: &str, cache: &str, extra: &str, len: usize, body: &[u8]) -> io::Result<()> {
    // One write for the head: some clients read the status line in a single recv.
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {len}\r\nCache-Control: {cache}\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self' ws: wss:\r\n\
         {extra}Connection: close\r\n\r\n"
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    c.write_all(&out)
}

fn json_resp(c: &mut TcpStream, status: &str, v: serde_json::Value) -> io::Result<()> {
    respond(c, status, "application/json; charset=utf-8", "", v.to_string().as_bytes())
}

fn token_of(req: &Req) -> Option<String> {
    req.headers.get("cookie")?.split(';').find_map(|kv| kv.trim().strip_prefix("itda=").map(str::to_string))
}

fn authed(req: &Req, local: bool) -> bool {
    let Some(t) = token_of(req) else { return false };
    let mut toks = TOKENS.lock().unwrap();
    toks.retain(|(_, at, _)| at.elapsed() < SESSION_TTL);
    toks.iter().any(|(k, _, l)| *l == local && eq_ct(k.as_bytes(), t.as_bytes()))
}

/// Constant-time comparison (PINs and tokens).
fn eq_ct(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Same-origin check for state-changing requests and the WebSocket (cross-site hijacking).
fn same_origin(req: &Req) -> bool {
    match (req.headers.get("origin"), req.headers.get("host")) {
        (Some(o), Some(h)) => o.strip_prefix("http://").or_else(|| o.strip_prefix("https://")) == Some(h.as_str()),
        (None, _) => true, // non-browser clients; still need the session cookie
        _ => false,
    }
}

/// Who is signing in, for counting wrong PINs: the peer, or behind `tailscale serve` (which
/// connects from this machine) the address it forwards for.
fn source(req: &Req, peer: SocketAddr) -> std::net::IpAddr {
    let fwd = || req.headers.get("x-forwarded-for").and_then(|v| v.split(',').next()).and_then(|v| v.trim().parse().ok());
    if peer.ip().is_loopback() { fwd().unwrap_or(peer.ip()) } else { peer.ip() }
}

fn login(c: &mut TcpStream, req: &Req, peer: SocketAddr) -> io::Result<()> {
    let from = source(req, peer);
    // (A lock that has run out starts the count afresh.)
    let locked = |f: &mut (u32, Option<Instant>)| match f.1 {
        Some(until) if Instant::now() < until => true,
        Some(_) => { *f = (0, None); false }
        None => false,
    };
    {
        let mut g = FAILS.lock().unwrap();
        let f = g.get_or_insert_with(Fails::default);
        if locked(&mut f.all) || f.per.get_mut(&from).is_some_and(locked) {
            return json_resp(c, "429 Too Many Requests", json!({"error": "잠시 후 다시 시도하세요 (PIN을 여러 번 틀렸습니다)"}));
        }
    }
    let pin = serde_json::from_slice::<serde_json::Value>(&req.body).ok().and_then(|v| v["pin"].as_str().map(str::to_string)).unwrap_or_default();
    if !eq_ct(pin.trim().as_bytes(), store::web_pin(false).as_bytes()) {
        let mut g = FAILS.lock().unwrap();
        let f = g.get_or_insert_with(Fails::default);
        if !f.per.contains_key(&from) && f.per.len() >= MAX_SOURCES {
            // Bounded memory: keep the sources still locked out; past that, only the total counts.
            f.per.retain(|_, x| x.1.is_some_and(|u| Instant::now() < u));
        }
        let n = if f.per.contains_key(&from) || f.per.len() < MAX_SOURCES {
            let mine = f.per.entry(from).or_insert((0, None));
            mine.0 += 1;
            if mine.0 >= MAX_FAILS {
                mine.1 = Some(Instant::now() + LOCKOUT);
            }
            mine.0
        } else {
            0
        };
        f.all.0 += 1;
        if f.all.0 >= MAX_ALL_FAILS {
            f.all.1 = Some(Instant::now() + LOCKOUT);
        }
        crate::log!("web: wrong PIN from {from} ({n})");
        return json_resp(c, "401 Unauthorized", json!({"error": "PIN이 맞지 않습니다"}));
    }
    if let Some(f) = FAILS.lock().unwrap().as_mut() {
        f.per.remove(&from);
    }
    let cookie = new_session(false);
    respond(c, "200 OK", "application/json; charset=utf-8", &cookie, b"{\"ok\":true}")
}

/// A fresh session; returns its Set-Cookie header line.
fn new_session(local: bool) -> String {
    let tok = random_token();
    TOKENS.lock().unwrap().push((tok.clone(), Instant::now(), local));
    format!("Set-Cookie: itda={tok}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}\r\n", SESSION_TTL.as_secs())
}

/// Local listener: an app window opens /auth?t=<token>&open=<id>, gets a session cookie
/// and goes straight to that connection.
fn local_auth(c: &mut TcpStream, req: &Req) -> io::Result<()> {
    let arg = |k: &str| req.query.split('&').find_map(|kv| kv.strip_prefix(k)).unwrap_or("");
    let good = LOCAL.lock().unwrap().as_ref().is_some_and(|(_, t)| eq_ct(t.as_bytes(), arg("t=").as_bytes()));
    if !good {
        return respond(c, "403 Forbidden", "text/plain", "", b"forbidden");
    }
    let open: String = arg("open=").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let trace = if arg("trace=") == "1" { "&trace=1" } else { "" };
    let cookie = new_session(true);
    respond(c, "302 Found", "text/plain", &format!("{cookie}Location: /?open={open}{trace}\r\n"), b"")
}

/// The address-book entry that stands for this PC itself (when "Share this PC" is on).
pub const THIS_PC: &str = "this-pc";

fn hosts_json() -> serde_json::Value {
    let hosts = store::load_hosts();
    let accounts = store::load_accounts();
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut list: Vec<_> = hosts
        .iter()
        .map(|h| {
            let login = store::resolve(h, &accounts);
            json!({
                "id": h.id,
                "label": h.label(),
                "name": h.name,
                "host": h.host,
                "user": login.user,
                "own_user": h.user,
                "account": h.account,
                "group": h.group,
                "groups": h.groups,
                "favorite": h.favorite,
                "kind": if h.is_rdp() { "rdp" } else { "vnc" },
                "os": if h.is_rdp() { "windows" } else if h.os.is_empty() { "mac" } else { h.os.as_str() },
                "ssh": h.ssh.on,
                "ssh_user": h.ssh.user,
                "ssh_port": h.ssh.port,
                "rdp": h.is_rdp(),
                "has_pw": login.pass.is_some(),
                "remote_size": h.remote_size,
                "remote_name": h.remote_name,
                "opts": h.opts,
                "hp": h.hp,
                "last_used": h.last_used,
                // The preview's version (its time), 0 = none: part of its URL, so it's cached
                // until it changes.
                "thumb": std::fs::metadata(store::shown_thumb_path(&h.id))
                    .and_then(|m| m.modified())
                    .map_or(0, |t| t.duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs().max(1))),
            })
        })
        .collect();
    #[cfg(windows)]
    if crate::share::running().is_some() {
        let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into());
        list.insert(0, json!({"id": THIS_PC, "label": format!("{name} (이 PC)"), "host": name, "this": true, "kind": "vnc", "os": "windows", "has_pw": true}));
    }
    json!(list)
}

/// Devices nearby that aren't in the address book yet: Bonjour on this network plus the
/// Tailscale tailnet, with what kind of device each probably is.
fn nearby_json() -> serde_json::Value {
    let ts = std::thread::spawn(crate::tailscale::status);
    let mut out: Vec<serde_json::Value> = crate::mdns::browse(Duration::from_millis(1500))
        .into_iter()
        .map(|f| {
            let os = if f.os == "windows" { "windows" } else { "mac" };
            // A PC with Itda but sharing off can still be opened over Remote Desktop.
            let kind = if os == "windows" && !f.share && f.rdp { "rdp" } else { "vnc" };
            json!({"name": f.name, "host": f.host, "os": os, "kind": kind, "via": "lan", "off": f.itda && !f.share && kind != "rdp"})
        })
        .collect();
    if let Ok(st) = ts.join() {
        if st.running {
            for p in st.peers.iter().filter(|p| p.online) {
                let (os, kind) = match p.os.as_str() {
                    "macOS" => ("mac", "vnc"),
                    "windows" => ("windows", "rdp"),
                    "linux" => ("linux", "vnc"),
                    _ => continue,
                };
                let host = if p.dns.is_empty() { p.ip.clone() } else { p.dns.clone() };
                out.push(json!({"name": p.name, "host": host, "os": os, "kind": kind, "via": "tailscale"}));
            }
        }
    }
    let book: Vec<String> = store::load_hosts().iter().map(|h| tunnel::split_host(&h.host, 0).0.to_ascii_lowercase()).collect();
    let me = this_machine().to_ascii_lowercase();
    out.retain(|f| {
        let h = tunnel::split_host(f["host"].as_str().unwrap_or(""), 0).0.to_ascii_lowercase();
        let name = f["name"].as_str().unwrap_or("").to_ascii_lowercase();
        !book.contains(&h) && (me.is_empty() || (name != me && h.trim_end_matches(".local") != me && !h.starts_with(&format!("{me}."))))
    });
    json!(out)
}

/// This computer's network name (to leave it out of "nearby").
fn this_machine() -> String {
    #[cfg(windows)]
    return std::env::var("COMPUTERNAME").unwrap_or_default();
    #[cfg(not(windows))]
    return std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
}

/// Without a home window (macOS), the gateway owns the address book: apply a change the
/// way the Windows home does (`home.rs` page commands).
fn apply_edit(cmd: &serde_json::Value) -> bool {
    let s = |k: &str| cmd.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    match s("cmd").as_str() {
        "save_host" => {
            let Ok(host) = serde_json::from_value::<store::Host>(cmd["host"].clone()) else { return false };
            if let Some(p) = cmd.get("password").and_then(|x| x.as_str()).filter(|p| !p.is_empty()) {
                store::set_password(&host.id, Some(p));
            }
            let mut hosts = store::load_hosts();
            match hosts.iter_mut().find(|x| x.id == host.id) {
                Some(x) => *x = host,
                None => hosts.push(host),
            }
            store::save_hosts(&hosts);
        }
        "delete_host" => {
            let id = s("id");
            let mut hosts = store::load_hosts();
            hosts.retain(|x| x.id != id);
            store::set_password(&id, None);
            store::remove_thumbs(&id);
            store::save_hosts(&hosts);
        }
        "save_account" => {
            let Ok(a) = serde_json::from_value::<store::Account>(cmd["account"].clone()) else { return false };
            if let Some(p) = cmd.get("password").and_then(|x| x.as_str()).filter(|p| !p.is_empty()) {
                store::set_password(&store::acct_key(&a.id), Some(p));
            }
            let mut accounts = store::load_accounts();
            match accounts.iter_mut().find(|x| x.id == a.id) {
                Some(x) => *x = a,
                None => accounts.push(a),
            }
            store::save_accounts(&accounts);
        }
        "delete_account" => {
            let id = s("id");
            let mut accounts = store::load_accounts();
            let gone = accounts.iter().find(|x| x.id == id).cloned();
            accounts.retain(|x| x.id != id);
            // Connections keep the user name and ask for a password next time.
            let mut hosts = store::load_hosts();
            for h in hosts.iter_mut().filter(|h| h.account == id) {
                h.account.clear();
                if let Some(a) = &gone {
                    h.user = a.user.clone();
                }
            }
            store::set_password(&store::acct_key(&id), None);
            store::set_password(&store::acct_keypass(&id), None);
            store::save_accounts(&accounts);
            store::save_hosts(&hosts);
        }
        _ => return false,
    }
    true
}

/// Remember a successful connection (macOS; on Windows the session process reports it).
fn mark_used(id: &str, raw: &rfb::Raw) {
    if ON_EDIT.get().is_some() {
        return;
    }
    let mut hosts = store::load_hosts();
    if let Some(h) = hosts.iter_mut().find(|h| h.id == id) {
        h.last_used = store::now();
        h.remote_name = raw.name.clone();
        h.remote_size = format!("{}×{}", raw.width, raw.height);
        store::save_hosts(&hosts);
    }
}

fn accounts_json() -> serde_json::Value {
    let hosts = store::load_hosts();
    json!(store::load_accounts()
        .iter()
        .map(|a| json!({
            "id": a.id,
            "name": a.name,
            "user": a.user,
            "ssh_key": a.ssh_key,
            "has_pw": store::get_password(&store::acct_key(&a.id)).is_some(),
            "uses": hosts.iter().filter(|h| h.account == a.id).count(),
        }))
        .collect::<Vec<_>>())
}

/// Applies an address-book change through the home window (so its list stays in step);
/// set by the home. Returns once the change is saved.
pub static ON_EDIT: std::sync::OnceLock<fn(serde_json::Value) -> bool> = std::sync::OnceLock::new();
/// Per connection attempt (device id + the viewer's attempt token): whether it got
/// high-performance sharing ({"hp":bool,"reason"}). Kept 10 minutes.
static MODES: Mutex<std::collections::BTreeMap<String, (Instant, serde_json::Value)>> = Mutex::new(std::collections::BTreeMap::new());
thread_local! {
    /// The attempt this connection thread is opening (from the viewer's ?attempt=).
    static ATTEMPT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    /// This connection is Screen Sharing standing in for a Mac whose screen was locked.
    static LOCKED_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The viewer couldn't show this device's high-performance video (?nohp=1): this attempt
    /// uses Screen Sharing whatever the saved choice.
    static NO_HP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// This connection is to the Mac's high-performance sharing (not Screen Sharing).
    static HP_LIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Why a connection used Screen Sharing after the viewer asked (?nohp=1).
const HP_UNDECODABLE: &str = "이 기기에서 그 Mac의 고성능 영상을 표시하지 못했습니다";
/// How long the Screen Sharing session that unlocked a Mac outlives the last connection to it:
/// the viewer may be reconnecting (to high-performance once it's unlocked, back to Screen
/// Sharing after video it couldn't decode, after a drop).
#[cfg(not(test))]
const LOCKED_FALLBACK_LINGER: Duration = Duration::from_secs(10);
#[cfg(test)]
const LOCKED_FALLBACK_LINGER: Duration = Duration::from_millis(500);
/// Per device: connections open to it now, and the Screen Sharing sessions kept from unlocking
/// it (how each ends). macOS locks a screen again when the Screen Sharing session that unlocked
/// it ends (seen on jiun-iMac: the high-performance capture lost its display seconds later), so
/// such a session belongs to the device, not to one connection: it lasts while any connection
/// to that device does (whichever way it went: high-performance, Screen Sharing, a second
/// viewer), and ends `LOCKED_FALLBACK_LINGER` after the last one.
#[derive(Default)]
struct Device {
    conns: usize,
    /// Each with when it was kept (a `seq()` stamp, comparable with a connection's `began`).
    kept: Vec<(u64, Box<dyn FnOnce() + Send>)>,
    /// New whenever a connection begins (unique across devices and their entries coming and
    /// going): a pending end from before is called off.
    epoch: u64,
}
static DEVICES: Mutex<std::collections::BTreeMap<String, Device>> = Mutex::new(std::collections::BTreeMap::new());
/// Increasing stamps for connections beginning and sessions being kept, so they can be ordered.
fn seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
/// A connection to device `id`, open from before it reaches the device (so a kept session is
/// already held while high-performance sharing takes its time to start) until it's over;
/// `began` is when it began.
struct Conn(String, u64);
impl Conn {
    fn begin(id: &str) -> Conn {
        let mut d = DEVICES.lock().unwrap();
        let dev = d.entry(id.to_string()).or_default();
        dev.conns += 1;
        dev.epoch = seq();
        Conn(id.to_string(), dev.epoch)
    }
}
impl Drop for Conn {
    fn drop(&mut self) {
        let mut d = DEVICES.lock().unwrap();
        let Some(dev) = d.get_mut(&self.0) else { return };
        dev.conns = dev.conns.saturating_sub(1);
        if dev.conns > 0 {
            return;
        }
        if dev.kept.is_empty() {
            d.remove(&self.0);
            return;
        }
        let (id, epoch) = (self.0.clone(), dev.epoch);
        drop(d);
        crate::log!("web viewer {id}: the Screen Sharing session that unlocked it ends in {}s unless a viewer comes back", LOCKED_FALLBACK_LINGER.as_secs());
        std::thread::spawn(move || {
            std::thread::sleep(LOCKED_FALLBACK_LINGER);
            let mut d = DEVICES.lock().unwrap();
            if !d.get(&id).is_some_and(|dev| dev.conns == 0 && dev.epoch == epoch) {
                return; // someone connected again meanwhile
            }
            let ends = d.remove(&id).map(|dev| dev.kept).unwrap_or_default();
            drop(d);
            ends.into_iter().for_each(|(_, end)| end());
            crate::log!("web viewer {id}: ended the Screen Sharing session that unlocked it");
        });
    }
}
/// This connection's Screen Sharing session unlocked device `id` (or may yet): keep it for the
/// device. Called while the connection still counts.
fn keep_unlocked(id: &str, end: Box<dyn FnOnce() + Send>) {
    DEVICES.lock().unwrap().entry(id.to_string()).or_default().kept.push((seq(), end));
}
/// A high-performance session that `began` then (so the Mac was unlocked then) ended because the
/// Mac locked: what was kept before it began holds no unlock any more. End it now rather than
/// let it pile up while a window stays open through lock after lock. (Anything kept after that
/// session began, e.g. by a second viewer, may be what unlocks it next, and stays.)
fn relocked_since(id: &str, began: u64) {
    let stale: Vec<(u64, Box<dyn FnOnce() + Send>)> = {
        let mut d = DEVICES.lock().unwrap();
        let Some(dev) = d.get_mut(id) else { return };
        let (stale, keep) = std::mem::take(&mut dev.kept).into_iter().partition(|(at, _)| *at < began);
        dev.kept = keep;
        stale
    };
    if !stale.is_empty() {
        crate::log!("web viewer {id}: ended {} Screen Sharing session(s) that unlocked it before it locked again", stale.len());
    }
    stale.into_iter().for_each(|(_, end)| end());
}
/// Whether a Screen Sharing session that unlocked device `id` is being kept.
fn keeps_unlocked(id: &str) -> bool {
    DEVICES.lock().unwrap().get(id).is_some_and(|d| !d.kept.is_empty())
}
fn record_mode(id: &str, v: serde_json::Value) {
    let attempt = ATTEMPT.with(|a| a.borrow().clone());
    if attempt.is_empty() {
        return;
    }
    let mut m = MODES.lock().unwrap();
    m.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(600));
    if m.len() < 256 {
        m.insert(format!("{id}\x1f{attempt}"), (Instant::now(), v));
    }
}

/// Add, change or remove a connection or account from the browser.
fn edit(c: &mut TcpStream, req: &Req) -> io::Result<()> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&req.body) else {
        return json_resp(c, "400 Bad Request", json!({"error": "잘못된 요청입니다"}));
    };
    let str_of = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.trim().to_string());
    let id = str_of("id").unwrap_or_default();
    let cmd = match str_of("cmd").as_deref() {
        Some("viewer_opts") => {
            let Some(mut h) = store::load_hosts().into_iter().find(|h| h.id == id) else {
                return json_resp(c, "404 Not Found", json!({"error":"연결을 찾을 수 없습니다"}));
            };
            let Ok(opts) = serde_json::from_value::<store::HostOpts>(v["opts"].clone()) else {
                return json_resp(c, "400 Bad Request", json!({"error":"잘못된 보기 설정입니다"}));
            };
            h.opts = opts;
            json!({"cmd":"save_host","host":h})
        }
        Some("save_host") => {
            let mut h = store::load_hosts().into_iter().find(|h| !id.is_empty() && h.id == id).unwrap_or_default();
            // Only the fields the web editor shows; options stay as the PC set them.
            for (k, f) in [("name", &mut h.name), ("host", &mut h.host), ("user", &mut h.user), ("group", &mut h.group), ("account", &mut h.account)] {
                if let Some(s) = str_of(k) {
                    *f = s;
                }
            }
            // The web editor edits groups as one comma-separated field.
            if str_of("group").is_some() {
                h.groups.clear();
                h.normalize_groups();
            }
            if let Some(k) = str_of("kind") {
                h.kind = if k == "rdp" { k } else { String::new() };
            }
            if let Some(os) = str_of("os").filter(|o| ["mac", "windows", "linux"].contains(&o.as_str())) {
                h.os = os;
            }
            if let Some(on) = v.get("ssh").and_then(|x| x.as_bool()) {
                h.ssh.on = on && !h.is_rdp();
            }
            if h.host.is_empty() {
                return json_resp(c, "400 Bad Request", json!({"error": "주소를 입력하세요"}));
            }
            if h.id.is_empty() {
                h.id = store::new_id();
            }
            let pw = str_of("password").filter(|p| !p.is_empty());
            json!({"cmd": "save_host", "host": h, "password": pw, "save_password": if pw.is_some() { json!(true) } else { json!(null) }})
        }
        Some("save_account") => {
            let mut a = store::load_accounts().into_iter().find(|a| !id.is_empty() && a.id == id).unwrap_or_default();
            for (k, f) in [("name", &mut a.name), ("user", &mut a.user)] {
                if let Some(s) = str_of(k) {
                    *f = s;
                }
            }
            if a.user.is_empty() {
                return json_resp(c, "400 Bad Request", json!({"error": "사용자 이름을 입력하세요"}));
            }
            if a.id.is_empty() {
                a.id = store::new_id();
            }
            json!({"cmd": "save_account", "account": a, "password": str_of("password").unwrap_or_default()})
        }
        Some(k @ ("delete_host" | "delete_account")) if !id.is_empty() && id != THIS_PC => json!({"cmd": k, "id": id}),
        _ => return json_resp(c, "400 Bad Request", json!({"error": "알 수 없는 요청입니다"})),
    };
    let saved = ["/host/id", "/account/id", "/id"].iter().find_map(|p| cmd.pointer(p).cloned());
    let done = match ON_EDIT.get() {
        Some(f) => f(cmd),
        None => apply_edit(&cmd),
    };
    if done {
        json_resp(c, "200 OK", json!({"ok": true, "id": saved, "hosts": hosts_json(), "accounts": accounts_json()}))
    } else {
        json_resp(c, "503 Service Unavailable", json!({"error": "PC의 Itda가 응답하지 않습니다"}))
    }
}

fn handle(mut c: TcpStream, peer: SocketAddr, local: bool) -> io::Result<()> {
    c.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut r = BufReader::new(c.try_clone()?);
    let req = read_req(&mut r)?;
    let ok = authed(&req, local);
    let p = req.path.as_str();
    match (req.method.as_str(), p) {
        ("GET", "/" | "/index.html") => asset(&mut c, &req, "text/html; charset=utf-8", INDEX.as_bytes()),
        ("GET", "/hp-setup.css") => asset(&mut c, &req, "text/css; charset=utf-8", include_bytes!("../web/hp-setup.css")),
        ("GET", "/hp-setup.js") => asset(&mut c, &req, "text/javascript; charset=utf-8", include_bytes!("../web/hp-setup.js")),
        ("GET", "/viewer.css") => asset(&mut c, &req, "text/css; charset=utf-8", CSS.as_bytes()),
        ("GET", "/viewer.js") => asset(&mut c, &req, "text/javascript; charset=utf-8", JS.as_bytes()),
        ("GET", "/interaction.js") => asset(&mut c, &req, "text/javascript; charset=utf-8", INTERACTION.as_bytes()),
        ("GET", "/monitors.js") => asset(&mut c, &req, "text/javascript; charset=utf-8", MONITORS.as_bytes()),
        ("GET", "/novnc.min.js") => asset(&mut c, &req, "text/javascript; charset=utf-8", NOVNC.as_bytes()),
        ("GET", "/icon.svg") => asset(&mut c, &req, "image/svg+xml", ICON.as_bytes()),
        ("GET", "/icon-192.png") => asset(&mut c, &req, "image/png", ICON_PNG),
        ("GET", "/manifest.webmanifest") => asset(&mut c, &req, "application/manifest+json", MANIFEST.as_bytes()),
        ("GET", "/icon-512.png") => asset(&mut c, &req, "image/png", ICON_PNG_512),
        ("GET", "/sw.js") => respond(&mut c, "200 OK", "text/javascript; charset=utf-8", "", SW.as_bytes()),
        ("GET", "/auth") if local => local_auth(&mut c, &req),
        ("POST", "/api/login") if same_origin(&req) && !local => login(&mut c, &req, peer),
        ("POST", "/api/logout") if same_origin(&req) => {
            if let Some(t) = token_of(&req) {
                TOKENS.lock().unwrap().retain(|(k, _, _)| *k != t);
            }
            respond(&mut c, "200 OK", "application/json", "Set-Cookie: itda=; Path=/; Max-Age=0\r\n", b"{}")
        }
        (_, "/api/me") => {
            // The viewer's wheel follows the app's Mac scroll setting.
            let scroll = ok.then(|| {
                let st = store::load_settings();
                json!({"mac": st.mac_scroll})
            });
            json_resp(&mut c, if ok { "200 OK" } else { "401 Unauthorized" }, json!({"ok": ok, "local": local, "scroll": scroll}))
        }
        (_, "/api/hosts") if ok => json_resp(&mut c, "200 OK", hosts_json()),
        ("GET", "/api/nearby") if ok => json_resp(&mut c, "200 OK", nearby_json()),
        ("GET", "/api/accounts") if ok => json_resp(&mut c, "200 OK", accounts_json()),
        // How the last connection to a device with high-performance mode on was made.
        ("GET", _) if ok && p.starts_with("/api/mode/") => {
            let id = p.trim_start_matches("/api/mode/");
            let attempt = req.query.split('&').find_map(|kv| kv.strip_prefix("attempt=")).unwrap_or("");
            let mode = MODES.lock().unwrap().get(&format!("{id}\x1f{attempt}")).map(|(_, v)| v.clone());
            json_resp(&mut c, "200 OK", mode.unwrap_or(serde_json::Value::Null))
        }
        // Whether that Mac's high-performance sharing would take a connection now (a viewer on
        // Screen Sharing because the screen was locked asks until it's unlocked).
        ("GET", _) if ok && p.starts_with("/api/hp-ready/") => {
            let id = p.trim_start_matches("/api/hp-ready/");
            let ready = match store::load_hosts().into_iter().find(|h| h.id == id && h.hp.on) {
                None => json!({"ready": false}),
                Some(host) => {
                    let login = store::resolve(&host, &store::load_accounts());
                    match local.then(|| typed(id)).flatten().or_else(|| login.pass.clone()) {
                        None => json!({"ready": false}),
                        Some(pass) => match open_hp(&host.reachable(), &login, &pass) {
                            Ok(_) => json!({"ready": true}),
                            Err(e) => json!({"ready": false, "locked": locked_refusal(&e)}),
                        },
                    }
                }
            };
            json_resp(&mut c, "200 OK", ready)
        }
        ("POST", "/api/edit") if ok && same_origin(&req) => edit(&mut c, &req),
        ("GET", _) if ok && p.starts_with("/thumb/") => {
            let id = p.trim_start_matches("/thumb/").trim_end_matches(".png");
            // (The id is checked before it becomes a path: on Windows a "\\\\host\\share" one
            // would otherwise reach out over SMB.)
            let id = Some(id).filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()));
            match id.and_then(|id| std::fs::read(store::shown_thumb_path(id)).ok()) {
                // Asked for by version (?v=), so the browser may keep it.
                Some(b) => send(&mut c, "200 OK", "image/png", "private, max-age=86400", "", &b),
                None => respond(&mut c, "404 Not Found", "text/plain", "", b"not found"),
            }
        }
        ("GET", _) if p.starts_with("/ws/") => {
            if !ok || !same_origin(&req) {
                return respond(&mut c, "403 Forbidden", "text/plain", "", b"forbidden");
            }
            let id = p.trim_start_matches("/ws/").to_string();
            let Some(key) = req.headers.get("sec-websocket-key").cloned() else {
                return respond(&mut c, "400 Bad Request", "text/plain", "", b"websocket expected");
            };
            // noVNC asks for the "binary" subprotocol; echo it when offered.
            let proto = req
                .headers
                .get("sec-websocket-protocol")
                .filter(|v| v.split(',').any(|p| p.trim() == "binary"))
                .map_or(String::new(), |_| "Sec-WebSocket-Protocol: binary\r\n".into());
            write!(c, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n{proto}\r\n", ws::accept_key(&key))?;
            c.set_read_timeout(None)?;
            crate::log!("web {peer}: viewer for {id}");
            // The viewer's size, for a Remote Desktop that can match it.
            let arg = |k: &str| req.query.split('&').find_map(|kv| kv.strip_prefix(k)).and_then(|v| v.parse::<u32>().ok());
            let size = arg("w=").zip(arg("h="));
            // (Tells this attempt's high-performance outcome apart from another viewer's.)
            let attempt: String = req.query.split('&').find_map(|kv| kv.strip_prefix("attempt=")).unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect();
            ATTEMPT.with(|a| *a.borrow_mut() = attempt);
            LOCKED_FALLBACK.with(|l| l.set(false));
            HP_LIVE.with(|l| l.set(false));
            NO_HP.with(|n| n.set(req.query.split('&').any(|kv| kv == "nohp=1")));
            proxy(c, &id, peer, local, size)
        }
        (_, _) if p.starts_with("/api/") || p.starts_with("/thumb/") => {
            json_resp(&mut c, "401 Unauthorized", json!({"error": "로그인이 필요합니다"}))
        }
        _ => respond(&mut c, "404 Not Found", "text/plain", "", b"not found"),
    }
}

// ---------------------------------------------------------------- RFB relay

/// Log in to the device, then act as a no-auth RFB server for the browser and pipe bytes.
fn proxy(c: TcpStream, id: &str, peer: SocketAddr, local: bool, size: Option<(u32, u32)>) -> io::Result<()> {
    // (Holds a Screen Sharing session kept from unlocking this device until it's over.)
    let conn = Conn::begin(id);
    let tx = ws::Sender::new(c.try_clone()?);
    // (Removed from LIVE however this connection ends.)
    let _live = if local { None } else { Some(LiveConn::track(&c)?) };
    let fail = |code: u16, why: String| {
        crate::log!("web viewer {id}: {why}");
        tx.close(code, &why);
        Ok(())
    };
    let mut tunnel: Option<Arc<Tunnel>> = None;
    #[cfg(not(windows))]
    let _ = peer;
    let raw = if id == THIS_PC {
        // The PIN already proved who this is, so no "allow?" prompt at the PC.
        #[cfg(windows)]
        let r = crate::share::open_local(peer);
        #[cfg(not(windows))]
        let r = Err(io::Error::new(io::ErrorKind::ConnectionRefused, "이 기기에서는 화면 공유를 제공하지 않습니다"));
        r
    } else {
        let Some(host) = store::load_hosts().into_iter().find(|h| h.id == id) else {
            return fail(4004, "연결을 찾을 수 없습니다".into());
        };
        let login = store::resolve(&host, &store::load_accounts());
        let Some(pass) = local.then(|| typed(id)).flatten().or_else(|| login.pass.clone()) else {
            return fail(4001, "이 연결에는 저장된 암호가 없습니다. 연결을 편집해 암호를 저장하세요.".into());
        };
        open_host(&host, &login, pass, &mut tunnel, size)
    };
    let raw = match raw {
        Ok(r) => r,
        Err(e) => return fail(4000 + kind_code(rfb::classify(&e)), e.to_string()),
    };
    if id != THIS_PC {
        mark_used(id, &raw);
    }
    // A high-performance session the Mac ends asks it why: one that ended because its screen
    // locked (the capture there shows only the lock screen's picture) or another user took it
    // closes with LOCKED_CLOSE, and the viewer reconnects at once, getting Screen Sharing
    // (where a locked screen can be unlocked).
    let probe = tunnel.clone().filter(|_| HP_LIVE.with(|l| l.get())).map(|t| {
        let id = id.to_string();
        Box::new(move || hp_unavailable(&t, &id)) as Probe
    });
    relay(c, raw, tx, id, tunnel, local, probe, conn.1)
}

fn open_host(host: &store::Host, login: &store::Login, pass: String, tunnel: &mut Option<Arc<Tunnel>>, size: Option<(u32, u32)>) -> io::Result<rfb::Raw> {
    let host = &host.reachable();
    let at_lan = !host.key_alias.is_empty();
    if host.is_rdp() {
        // The server must show the key it had the first time (checked before the login goes
        // out); at the LAN address there must be one already.
        let saved = store::rdp_key(&host.id);
        if at_lan && saved.is_none() {
            return Err(rfb::denied("저장된 기기인지 확인할 수 없어 같은 네트워크 주소로 연결하지 않았습니다"));
        }
        let (raw, key) = crate::rdp::open(&host.host, &login.user, &pass, size.filter(|_| host.opts.resize), saved.as_deref())?;
        if saved.is_none() {
            store::set_rdp_key(&host.id, &key);
        }
        return Ok(raw);
    }
    // Itda's high-performance sharing on this Mac: through SSH to its loopback port. If it
    // can't be reached (Itda not running there, SSH refused), fall back to the usual way.
    if host.hp.on && NO_HP.with(|n| n.get()) {
        crate::log!("high-performance sharing on {}: the viewer couldn't decode its video, using Screen Sharing", host.host);
        record_mode(&host.id, json!({"hp": false, "setup": false, "locked": false, "reason": HP_UNDECODABLE}));
    } else if host.hp.on {
        match open_hp(host, login, &pass) {
            Ok((t, raw)) => {
                *tunnel = Some(t);
                HP_LIVE.with(|l| l.set(true));
                record_mode(&host.id, json!({"hp": true}));
                // (Screen Sharing kept from unlocking this Mac lasts as long as this connection.)
                if keeps_unlocked(&host.id) {
                    crate::log!("high-performance sharing on {}: keeping the Screen Sharing session that unlocked it", host.host);
                }
                return Ok(raw);
            }
            Err(e) if at_lan && !host.ssh.on => return Err(e), // (plain VNC can't check the device there)
            Err(e) => {
                crate::log!("high-performance sharing on {} unavailable, using Screen Sharing: {e}", host.host);
                // The viewer shows what it actually got, not the saved choice.
                // Set up again (not retried) when it's high-performance sharing itself that is off
                // or no longer matches; SSH problems keep their own message.
                let setup = e.get_ref().is_some_and(|r| r.is::<HpSetup>());
                // (Locked, or another user has the screen: either way the viewer watches for it to
                // be over and switches back.)
                let why = unavailable(&e);
                LOCKED_FALLBACK.with(|l| l.set(why.is_some()));
                record_mode(&host.id, json!({"hp": false, "setup": setup, "locked": why.is_some(), "other_user": why == Some(Unavailable::OtherUser), "reason": e.to_string()}));
            }
        }
    }
    let creds = Creds { user: login.user.clone(), pass: pass.clone() };
    if host.ssh.on {
        let (h, vnc_port) = tunnel::split_host(&host.host, 5900);
        let cfg = SshCfg {
            host: h,
            port: if host.ssh.port == 0 { 22 } else { host.ssh.port },
            user: if host.ssh.user.is_empty() { login.user.clone() } else { host.ssh.user.clone() },
            pass,
            vnc_port,
            key: login.ssh_key.clone(),
            key_pass: login.key_pass.clone().unwrap_or_default(),
            key_alias: host.key_alias.clone(),
        };
        let t = Tunnel::open(cfg)?;
        let a = t.addr()?;
        *tunnel = Some(t);
        rfb::open(&a, &creds)
    } else if at_lan {
        Err(rfb::denied("같은 네트워크 주소로는 SSH로만 연결합니다"))
    } else {
        rfb::open(&rfb::addr(&host.host), &creds)
    }
}

/// The connection's password for SSH (saved, or typed for it); the server's own for VNC.
/// High-performance sharing itself is off or out of step on that Mac (SSH worked): setting it
/// up again is the fix, not a retry.
#[derive(Debug)]
pub(crate) struct HpSetup(&'static str);
impl std::fmt::Display for HpSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str(self.0) }
}
impl std::error::Error for HpSetup {}

pub(crate) fn open_hp(host: &store::Host, login: &store::Login, ssh_pass: &str) -> io::Result<(Arc<Tunnel>, rfb::Raw)> {
    let setup = |why| io::Error::new(io::ErrorKind::NotFound, HpSetup(why));
    let pass = store::get_password(&crate::hp_upgrade::password_key(&host.id)).ok_or_else(|| setup("이 기기에 고성능 공유 암호가 없습니다"))?;
    let cfg = crate::hp_upgrade::ssh_cfg_with(host, login, ssh_pass.to_string());
    let t = Tunnel::open(cfg.clone())?;
    let quiet = |e: &io::Error| matches!(e.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted);
    let mut raw = rfb::open(&t.addr()?, &Creds { user: String::new(), pass: pass.clone() });
    // Nothing answering: Itda may just not be running there (quit, or the Mac restarted). If
    // sharing is on for that Mac, start it in the background and try once more.
    if raw.as_ref().is_err_and(quiet) {
        match crate::hp_upgrade::start_sharing(&cfg) {
            Ok(()) => {
                crate::log!("high-performance sharing on {}: started Itda there", host.host);
                raw = rfb::open(&t.addr()?, &Creds { user: String::new(), pass });
            }
            Err(e) => crate::log!("high-performance sharing on {}: couldn't start Itda there: {e}", host.host),
        }
    }
    let raw = raw.map_err(|e| match e.kind() {
        // Nothing answering behind the tunnel: sharing isn't on, or Itda isn't running there.
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => setup("원격 Mac의 Itda 고성능 공유에 연결하지 못했습니다"),
        // The password it has changed (e.g. Itda was reinstalled there).
        io::ErrorKind::PermissionDenied if e.to_string().starts_with("인증 실패") => setup("원격 Mac의 고성능 공유 암호가 바뀌었습니다"),
        _ => e,
    })?;
    Ok((t, raw))
}

/// Why the Mac's high-performance sharing can't show its screen now, from its refusal: locked,
/// or another user's session on it (Screen Sharing stands in either way, and the viewer
/// switches back once it's over).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Unavailable { Locked, OtherUser }
fn unavailable(e: &io::Error) -> Option<Unavailable> {
    let s = e.to_string();
    if s.contains(crate::hp_upgrade::SCREEN_LOCKED) {
        Some(Unavailable::Locked)
    } else if s.contains(crate::hp_upgrade::OTHER_USER) {
        Some(Unavailable::OtherUser)
    } else {
        None
    }
}
fn locked_refusal(e: &io::Error) -> bool {
    unavailable(e).is_some()
}

/// Close code for a high-performance session that ended because the Mac's screen locked or
/// another user took it (the viewer reconnects at once and gets Screen Sharing).
const LOCKED_CLOSE: u16 = 4030;
fn unavailable_close_text(u: Unavailable) -> &'static str {
    match u {
        Unavailable::Locked => "그 Mac의 화면이 잠겨 기본 화면 공유로 다시 연결합니다",
        Unavailable::OtherUser => "다른 사용자가 그 Mac 화면을 쓰기 시작해 기본 화면 공유로 다시 연결합니다",
    }
}

type Probe = Box<dyn FnOnce() -> Option<Unavailable> + Send>;

/// Asks the Mac's high-performance sharing, through the session's own tunnel (no new SSH login,
/// and none at all once that's gone), whether it refuses now because its screen is locked or
/// another user has it.
fn hp_unavailable(t: &Tunnel, id: &str) -> Option<Unavailable> {
    let addr = t.live_addr()?;
    let pass = store::get_password(&crate::hp_upgrade::password_key(id))?;
    // Bounded: a stalled forward mustn't keep the viewer frozen or hold up cleanup. A late
    // answer is dropped (the attempt ends with its own timeouts; it holds no tunnel).
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(rfb::open(&addr, &Creds { user: String::new(), pass }).err().and_then(|e| unavailable(&e)));
    });
    rx.recv_timeout(HP_LOCKED_PROBE).ok().flatten()
}
/// How long the gateway waits for that answer.
const HP_LOCKED_PROBE: Duration = Duration::from_secs(3);

/// How to close the viewer's side once the device ended the session, given why that Mac can't
/// show its screen now (asked of high-performance sharing only).
fn device_closed(why: Option<Unavailable>) -> (u16, &'static str) {
    match why {
        Some(u) => (LOCKED_CLOSE, unavailable_close_text(u)),
        None => (1000, "원격 기기와의 연결이 끊겼습니다"),
    }
}

#[allow(clippy::too_many_arguments)]
fn relay(c: TcpStream, raw: rfb::Raw, tx: ws::Sender, id: &str, tunnel: Option<Arc<Tunnel>>, local: bool, probe: Option<Probe>, began: u64) -> io::Result<()> {
    let rfb::Raw { reader, mut writer, server_init, .. } = raw;
    let _local_relay=if local {Some(LocalRelay::track(id,&c,&writer)?)} else {None};
    // The handshake timeout must not cut a quiet (unchanging) screen.
    writer.set_read_timeout(None)?;

    // Our side of the handshake towards the browser: RFB 3.8, security None.
    let mut rx = ws::Receiver::new(c.try_clone()?, tx.clone());
    tx.binary(b"RFB 003.008\n")?;
    let mut v = [0u8; 12];
    rx.read_exact(&mut v)?;
    tx.binary(&[1, 1])?;
    let mut pick = [0u8; 1];
    rx.read_exact(&mut pick)?;
    tx.binary(&[0, 0, 0, 0])?;
    let mut shared = [0u8; 1];
    rx.read_exact(&mut shared)?;
    tx.binary(&server_init)?;

    // Mac → browser on its own thread; browser → Mac here.
    let down = tx.clone();
    let mac_sock = writer.try_clone()?;
    let mut reader = reader;
    // The viewer left first (its side ended this relay): no reason to ask the device for.
    let viewer_left = Arc::new(AtomicBool::new(false));
    let (left, who) = (viewer_left.clone(), id.to_string());
    // Whatever the 1 MB read buffer holds goes out as one message: no copy into a smaller
    // buffer first, and fewer, larger messages for the browser to handle.
    let pump = std::thread::spawn(move || {
        let device_ended = loop {
            let n = match reader.fill_buf() {
                Ok([]) | Err(_) => break true,
                Ok(b) => {
                    if down.binary(b).is_err() {
                        break false;
                    }
                    b.len()
                }
            };
            reader.consume(n);
        };
        let viewer_gone = !device_ended || left.load(std::sync::atomic::Ordering::Acquire);
        // (Asked only when the device ended it: a viewer that left needs no reason.)
        let unavailable = probe.filter(|_| !viewer_gone).and_then(|p| p());
        let (code, why) = device_closed(unavailable);
        if code == LOCKED_CLOSE {
            crate::log!("web viewer {who}: that Mac's screen became unavailable during high-performance sharing ({why}); the viewer reconnects through Screen Sharing");
        }
        if unavailable == Some(Unavailable::Locked) {
            relocked_since(&who, began);
        }
        down.close(code, why);
    });
    let mut buf = vec![0u8; 1 << 16];
    let viewer_ended = loop {
        match rx.read(&mut buf) {
            Ok(0) | Err(_) => break true,
            Ok(n) => {
                if writer.write_all(&buf[..n]).is_err() {
                    break false;
                }
            }
        }
    };
    if viewer_ended {
        viewer_left.store(true, std::sync::atomic::Ordering::Release);
    }
    let end = move || {
        let _ = mac_sock.shutdown(Shutdown::Both);
        let _ = pump.join();
        drop(tunnel);
    };
    if LOCKED_FALLBACK.with(|l| l.get()) {
        // (The viewer may be switching to high-performance now that it's unlocked.)
        crate::log!("web viewer {id}: closed; its Screen Sharing session stays for the switch");
        keep_unlocked(id, Box::new(end));
        return Ok(());
    }
    end();
    crate::log!("web viewer {id}: closed");
    Ok(())
}

/// Stable close codes so the page can explain what happened.
fn kind_code(kind: &str) -> u16 {
    match kind {
        "auth" => 10,
        "need_user" => 11,
        "refused" => 12,
        "timeout" | "unreachable" => 13,
        "dns" => 14,
        "ssh_auth" => 20,
        "ssh_hostkey" => 21,
        "ssh_refused" => 22,
        "ssh_missing" => 23,
        _ => 99,
    }
}

#[cfg(test)]
mod tests {
    /// A remote viewer's socket leaves LIVE when its connection ends; stop still closes the
    /// ones open.
    #[test]
    fn live_entry_leaves_with_its_connection() {
        use std::net::{TcpListener, TcpStream};
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let _a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (c, _) = l.accept().unwrap();
        let has = |t: u64| super::LIVE.lock().unwrap().iter().any(|(x, _)| *x == t);
        let live = super::LiveConn::track(&c).unwrap();
        let ticket = live.0;
        assert!(has(ticket));
        drop(live);
        assert!(!has(ticket), "entry outlived its connection");
    }

    #[test]
    fn a_device_ended_session_says_locked_only_when_the_mac_refuses_for_its_lock() {
        use super::{device_closed, LOCKED_CLOSE};
        assert_eq!(device_closed(None).0, 1000, "an ordinary drop (Screen Sharing, or an unlocked Mac's end)");
        for u in [super::Unavailable::Locked, super::Unavailable::OtherUser] {
            let (code, why) = device_closed(Some(u));
            assert_eq!(code, LOCKED_CLOSE);
            // A WebSocket close reason carries at most 123 bytes; ws::Sender::close cuts at 120.
            assert!(why.len() <= 120, "close reason cut short: {} bytes", why.len());
        }
    }

    /// The lock answer the gateway matches is the one the Mac's sharing gives.
    #[test]
    fn locked_refusals_are_recognised() {
        let refused = std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("인증 실패: {}", crate::hp_upgrade::SCREEN_LOCKED));
        assert_eq!(super::unavailable(&refused), Some(super::Unavailable::Locked));
        let taken = std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("인증 실패: {}", crate::hp_upgrade::OTHER_USER));
        assert_eq!(super::unavailable(&taken), Some(super::Unavailable::OtherUser));
        assert!(!super::locked_refusal(&std::io::Error::other("원격 Mac의 Itda 고성능 공유에 연결하지 못했습니다")));
    }

    /// A kept Screen Sharing session belongs to its device: it lasts while any connection to
    /// it does (a second viewer, a reconnect the other way, one still starting), ends a moment
    /// after the last one, and another device's is its own.
    #[test]
    fn kept_screen_sharing_lasts_while_the_device_has_a_connection() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let linger = super::LOCKED_FALLBACK_LINGER;
        let ended = Arc::new(AtomicUsize::new(0));
        let end = |n: usize| { let e = ended.clone(); Box::new(move || { e.fetch_add(n, Ordering::SeqCst); }) as Box<dyn FnOnce() + Send> };
        let wait = || std::thread::sleep(linger * 2);
        let now = || ended.load(Ordering::SeqCst);
        // (Timer threads may run late on a busy machine: wait for an end, don't assume it.)
        let ends_to = |n: usize| {
            let until = std::time::Instant::now() + linger * 20;
            while now() != n && std::time::Instant::now() < until {
                std::thread::sleep(linger / 10);
            }
            assert_eq!(now(), n);
        };
        // Screen Sharing for the locked Mac; a second viewer (high-performance) joins.
        let fallback = super::Conn::begin("dev-mac");
        let second = super::Conn::begin("dev-mac");
        super::keep_unlocked("dev-mac", end(1));
        let other = super::Conn::begin("other-mac");
        super::keep_unlocked("other-mac", end(100));
        drop(fallback);
        wait();
        assert_eq!(now(), 0, "ended while another viewer of that Mac was still connected");
        assert!(super::keeps_unlocked("dev-mac"));
        // The last one goes, and a replacement comes back in time (e.g. still starting up).
        drop(second);
        let back = super::Conn::begin("dev-mac");
        wait();
        assert_eq!(now(), 0, "ended under a connection that came back in time");
        // A reconnect that drops again soon gets the full time again, not what was left of the
        // first wait.
        drop(back);
        std::thread::sleep(linger / 2);
        let brief = super::Conn::begin("dev-mac");
        drop(brief);
        std::thread::sleep(linger * 7 / 10);
        assert_eq!(now(), 0, "ended on the earlier connection's timer");
        let back = super::Conn::begin("dev-mac");
        // Gone for good: it ends a moment later, once; the other Mac's stays.
        drop(back);
        assert_eq!(now(), 0, "ended at once, with no time to come back");
        ends_to(1);
        assert!(!super::keeps_unlocked("dev-mac"));
        drop(other);
        ends_to(101);
    }

    /// A window that stays open through lock after lock keeps only what may still hold an
    /// unlock: once a high-performance session (so the Mac was unlocked when it began) ends
    /// because it locked, what was kept before that session began ends; what was kept after
    /// (a second viewer, perhaps unlocking it now) stays, and nothing ends before that's known.
    #[test]
    fn kept_screen_sharing_does_not_pile_up_across_locks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let ended = Arc::new(AtomicUsize::new(0));
        let end = |n: usize| { let e = ended.clone(); Box::new(move || { e.fetch_add(n, Ordering::SeqCst); }) as Box<dyn FnOnce() + Send> };
        let now = || ended.load(Ordering::SeqCst);
        // Locked: Screen Sharing, unlocked through it, switched to high-performance (kept).
        let first = super::Conn::begin("cycle-mac");
        super::keep_unlocked("cycle-mac", end(1));
        let hp = super::Conn::begin("cycle-mac");
        drop(first);
        // A second window's Screen Sharing, kept while high-performance is on.
        let other = super::Conn::begin("cycle-mac");
        super::keep_unlocked("cycle-mac", end(10));
        drop(other);
        assert_eq!(now(), 0, "ended before the Mac was known to have locked again");
        // High-performance ends: the Mac locked.
        super::relocked_since("cycle-mac", hp.1);
        assert_eq!(now(), 1, "the session kept before the high-performance one began should end, and only it");
        drop(hp);
        assert!(super::keeps_unlocked("cycle-mac"));
        let until = std::time::Instant::now() + super::LOCKED_FALLBACK_LINGER * 20;
        while now() != 11 && std::time::Instant::now() < until {
            std::thread::sleep(super::LOCKED_FALLBACK_LINGER / 10);
        }
        assert_eq!(now(), 11);
    }

    #[test]
    fn wrong_pins_count_per_source() {
        let req = |xff: Option<&str>| super::Req { method: "POST".into(), path: "/api/login".into(), query: String::new(), headers: xff.map(|v| [("x-forwarded-for".to_string(), v.to_string())].into_iter().collect()).unwrap_or_default(), body: Vec::new() };
        let lan: std::net::SocketAddr = "192.168.0.9:5000".parse().unwrap();
        let local: std::net::SocketAddr = "127.0.0.1:5000".parse().unwrap();
        // Direct clients can't pick their own source with a header.
        assert_eq!(super::source(&req(Some("10.9.9.9")), lan), lan.ip());
        // Behind tailscale serve (from this machine): the address it forwards for.
        assert_eq!(super::source(&req(Some("100.64.1.2, 127.0.0.1")), local).to_string(), "100.64.1.2");
        assert_eq!(super::source(&req(None), local), local.ip());
    }

    #[test]
    fn request_heads_are_bounded() {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let send = |bytes: Vec<u8>| {
            let mut c = std::net::TcpStream::connect(addr).unwrap();
            std::thread::spawn(move || { let _ = c.write_all(&bytes); std::thread::sleep(std::time::Duration::from_millis(200)); });
            let (s, _) = l.accept().unwrap();
            super::read_req(&mut std::io::BufReader::new(s))
        };
        let ok = send(b"GET /x?a=1 HTTP/1.1\r\nHost: h\r\n\r\n".to_vec()).unwrap();
        assert_eq!((ok.path.as_str(), ok.query.as_str()), ("/x", "a=1"));
        // A line without an end, longer than MAX_LINE: refused, not buffered without limit.
        let mut long = b"GET /".to_vec();
        long.resize(super::MAX_LINE as usize * 4, b'a');
        assert!(send(long).is_err());
        let mut header = b"GET / HTTP/1.1\r\nX: ".to_vec();
        header.resize(super::MAX_LINE as usize * 2, b'b');
        assert!(send(header).is_err());
        // A byte at a time stays under every per-read timeout but not the whole-head deadline.
        // (Timing only bounds the failure: a slow CI machine must not fail a passing case.)
        let mut c = std::net::TcpStream::connect(addr).unwrap();
        let drip = std::thread::spawn(move || {
            for _ in 0..400 {
                if c.write_all(b"a").is_err() { break; }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        let (s, _) = l.accept().unwrap();
        let t = std::time::Instant::now();
        let r = super::read_req(&mut std::io::BufReader::new(s));
        assert!(r.is_err() && t.elapsed() < super::REQUEST_TIME * 3, "slow head allowed for {:?} (the drip lasts 8 s)", t.elapsed());
        let _ = drip.join();
    }

    use super::*;

    #[test]
    fn closing_native_owner_stops_both_streams() {
        fn pair() -> (TcpStream, TcpStream) {
            let l=TcpListener::bind(("127.0.0.1",0)).unwrap();
            let a=TcpStream::connect(l.local_addr().unwrap()).unwrap();
            let b=l.accept().unwrap().0;
            b.set_read_timeout(Some(Duration::from_millis(100))).unwrap();(a,b)
        }
        use std::net::TcpListener;
        let (client,mut browser)=pair();let (remote,mut server)=pair();
        let (unrelated,mut other)=pair();let (unrelated_remote,_)=pair();
        let _guard=LocalRelay::track("native-close-test",&client,&remote).unwrap();
        let _other=LocalRelay::track("unrelated-close-test",&unrelated,&unrelated_remote).unwrap();
        close_local("native-close-test");
        let mut byte=[0];assert_eq!(browser.read(&mut byte).unwrap(),0);assert_eq!(server.read(&mut byte).unwrap(),0);
        assert!(matches!(other.read(&mut byte).unwrap_err().kind(),io::ErrorKind::WouldBlock|io::ErrorKind::TimedOut));
    }
    #[test]
    fn constant_time_eq() {
        assert!(eq_ct(b"123456", b"123456"));
        assert!(!eq_ct(b"123456", b"123457"));
        assert!(!eq_ct(b"12345", b"123456"));
    }

    fn get(inm: Option<&str>) -> String {
        let mut headers = HashMap::new();
        if let Some(v) = inm {
            headers.insert("if-none-match".to_string(), v.to_string());
        }
        let req = Req { method: "GET".into(), path: "/novnc.min.js".into(), query: String::new(), headers, body: Vec::new() };
        let mut out = Vec::new();
        asset(&mut out, &req, "text/javascript", NOVNC.as_bytes()).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn conditional_get() {
        let full = get(None);
        assert!(full.starts_with("HTTP/1.1 200 OK\r\n") && full.ends_with(NOVNC));
        let len = format!("Content-Length: {}\r\n", NOVNC.len());
        assert!(full.contains(&len));
        let tag = full.lines().find_map(|l| l.strip_prefix("ETag: ")).unwrap().to_string();
        assert_eq!(tag, etag(NOVNC.as_bytes()), "cached tag is stable");
        for inm in [tag.clone(), format!("W/{tag}"), format!("\"x\", {tag}"), "*".into()] {
            let r = get(Some(&inm));
            assert!(r.starts_with("HTTP/1.1 304 Not Modified\r\n"), "{inm}");
            assert!(r.contains(&len), "304 carries the 200's Content-Length");
            assert!(r.ends_with("\r\n\r\n"), "and no body");
        }
        assert!(get(Some("\"stale\"")).starts_with("HTTP/1.1 200 OK"));
    }
}
