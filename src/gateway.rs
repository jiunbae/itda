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
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INDEX: &str = include_str!("../web/viewer/index.html");
const CSS: &str = include_str!("../web/viewer/viewer.css");
const JS: &str = include_str!("../web/viewer/viewer.js");
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
static FAILS: Mutex<(u32, Option<Instant>)> = Mutex::new((0, None));
/// Open browser sessions of the web viewer, closed when it stops.
static LIVE: Mutex<Vec<TcpStream>> = Mutex::new(Vec::new());

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
    Ok(format!("http://127.0.0.1:{port}/auth?t={tok}&open={id}"))
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

fn typed(id: &str) -> Option<String> {
    TYPED.lock().unwrap().iter().find(|(k, _)| k == id).map(|(_, p)| p.clone())
}

fn random_token() -> String {
    let mut b = [0u8; 24];
    let _ = getrandom::getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Blocking accept, so a new connection is served at once; `stop()` wakes it with a connect.
fn serve(l: TcpListener, local: bool, stop: Arc<AtomicBool>) -> io::Result<std::thread::JoinHandle<()>> {
    Ok(std::thread::spawn(move || {
        while !stop.load(Relaxed) {
            match l.accept() {
                Ok(_) if stop.load(Relaxed) => break,
                Ok((c, peer)) => {
                    // Small frames (cursor, one tile) must not wait for Nagle + delayed ACK.
                    let _ = c.set_nodelay(true);
                    std::thread::spawn(move || {
                        if let Err(e) = handle(c, peer, local) {
                            crate::log!("web {peer}: {e}");
                        }
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
    for s in LIVE.lock().unwrap().drain(..) {
        let _ = s.shutdown(Shutdown::Both);
    }
    TOKENS.lock().unwrap().retain(|t| t.2);
    *FAILS.lock().unwrap() = (0, None);
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

fn read_req(r: &mut BufReader<TcpStream>) -> io::Result<Req> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let target = it.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_string(), query.to_string());
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
        if headers.len() > 64 {
            return Err(io::Error::other("too many headers"));
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0; len.min(64 << 10)];
    r.read_exact(&mut body)?;
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

fn login(c: &mut TcpStream, req: &Req) -> io::Result<()> {
    {
        let mut f = FAILS.lock().unwrap();
        if let Some(until) = f.1 {
            if Instant::now() < until {
                return json_resp(c, "429 Too Many Requests", json!({"error": "잠시 후 다시 시도하세요 (PIN을 여러 번 틀렸습니다)"}));
            }
            *f = (0, None);
        }
    }
    let pin = serde_json::from_slice::<serde_json::Value>(&req.body).ok().and_then(|v| v["pin"].as_str().map(str::to_string)).unwrap_or_default();
    if !eq_ct(pin.trim().as_bytes(), store::web_pin(false).as_bytes()) {
        let mut f = FAILS.lock().unwrap();
        f.0 += 1;
        if f.0 >= MAX_FAILS {
            f.1 = Some(Instant::now() + LOCKOUT);
        }
        crate::log!("web: wrong PIN ({})", f.0);
        return json_resp(c, "401 Unauthorized", json!({"error": "PIN이 맞지 않습니다"}));
    }
    *FAILS.lock().unwrap() = (0, None);
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
    let cookie = new_session(true);
    respond(c, "302 Found", "text/plain", &format!("{cookie}Location: /?open={open}\r\n"), b"")
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
                "last_used": h.last_used,
                "thumb": std::fs::metadata(store::thumb_path(&h.id)).is_ok(),
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
            let _ = std::fs::remove_file(store::thumb_path(&id));
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

/// Add, change or remove a connection or account from the browser.
fn edit(c: &mut TcpStream, req: &Req) -> io::Result<()> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&req.body) else {
        return json_resp(c, "400 Bad Request", json!({"error": "잘못된 요청입니다"}));
    };
    let str_of = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.trim().to_string());
    let id = str_of("id").unwrap_or_default();
    let cmd = match str_of("cmd").as_deref() {
        Some("save_host") => {
            let mut h = store::load_hosts().into_iter().find(|h| !id.is_empty() && h.id == id).unwrap_or_default();
            // Only the fields the web editor shows; options stay as the PC set them.
            for (k, f) in [("name", &mut h.name), ("host", &mut h.host), ("user", &mut h.user), ("group", &mut h.group), ("account", &mut h.account)] {
                if let Some(s) = str_of(k) {
                    *f = s;
                }
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
        ("GET", "/viewer.css") => asset(&mut c, &req, "text/css; charset=utf-8", CSS.as_bytes()),
        ("GET", "/viewer.js") => asset(&mut c, &req, "text/javascript; charset=utf-8", JS.as_bytes()),
        ("GET", "/novnc.min.js") => asset(&mut c, &req, "text/javascript; charset=utf-8", NOVNC.as_bytes()),
        ("GET", "/icon.svg") => asset(&mut c, &req, "image/svg+xml", ICON.as_bytes()),
        ("GET", "/icon-192.png") => asset(&mut c, &req, "image/png", ICON_PNG),
        ("GET", "/manifest.webmanifest") => asset(&mut c, &req, "application/manifest+json", MANIFEST.as_bytes()),
        ("GET", "/icon-512.png") => asset(&mut c, &req, "image/png", ICON_PNG_512),
        ("GET", "/sw.js") => respond(&mut c, "200 OK", "text/javascript; charset=utf-8", "", SW.as_bytes()),
        ("GET", "/auth") if local => local_auth(&mut c, &req),
        ("POST", "/api/login") if same_origin(&req) && !local => login(&mut c, &req),
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
        ("POST", "/api/edit") if ok && same_origin(&req) => edit(&mut c, &req),
        ("GET", _) if ok && p.starts_with("/thumb/") => {
            let id = p.trim_start_matches("/thumb/").trim_end_matches(".png");
            match std::fs::read(store::thumb_path(id)).ok().filter(|_| id.chars().all(|c| c.is_ascii_hexdigit())) {
                Some(b) => respond(&mut c, "200 OK", "image/png", "", &b),
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
    let tx = ws::Sender::new(c.try_clone()?);
    if !local {
        LIVE.lock().unwrap().push(c.try_clone()?);
    }
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
    relay(c, raw, tx, id, tunnel)
}

fn open_host(host: &store::Host, login: &store::Login, pass: String, tunnel: &mut Option<Arc<Tunnel>>, size: Option<(u32, u32)>) -> io::Result<rfb::Raw> {
    if host.is_rdp() {
        return crate::rdp::open(&host.host, &login.user, &pass, size.filter(|_| host.opts.resize));
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
        };
        let t = Tunnel::open(cfg)?;
        let a = t.addr()?;
        *tunnel = Some(t);
        rfb::open(&a, &creds)
    } else {
        rfb::open(&rfb::addr(&host.host), &creds)
    }
}

fn relay(c: TcpStream, raw: rfb::Raw, tx: ws::Sender, id: &str, tunnel: Option<Arc<Tunnel>>) -> io::Result<()> {
    let rfb::Raw { reader, mut writer, server_init, .. } = raw;
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
    // Whatever the 1 MB read buffer holds goes out as one message: no copy into a smaller
    // buffer first, and fewer, larger messages for the browser to handle.
    let pump = std::thread::spawn(move || {
        loop {
            let n = match reader.fill_buf() {
                Ok([]) | Err(_) => break,
                Ok(b) => {
                    if down.binary(b).is_err() {
                        break;
                    }
                    b.len()
                }
            };
            reader.consume(n);
        }
        down.close(1000, "원격 기기와의 연결이 끊겼습니다");
    });
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match rx.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if writer.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = mac_sock.shutdown(Shutdown::Both);
    let _ = pump.join();
    drop(tunnel);
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
    use super::*;

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
