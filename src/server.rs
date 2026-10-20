//! The VNC server behind "Share this PC" (Windows) and Itda's high-performance sharing on a
//! Mac: authentication, the viewer protocol and the update pump, shared by both. What a
//! platform provides — the screen capture, input injection, clipboard, the "allow?" prompt,
//! the display list and an optional hardware video encoder — lives in `platform` (share.rs on
//! Windows, host_mac.rs on macOS) under the same names.
//!
//! Updates are ZRLE (tiles diffed against the viewer's last frame), or H.264 (RFB encoding 50)
//! for viewers that ask for it when the platform has an encoder: the whole screen as one video
//! rectangle while it changes, then a lossless ZRLE repaint of what changed once it stands
//! still (4:2:0 video blurs text and colour edges).

use crate::demand::{self, Demand};
use crate::enc::{ext_desktop_size, screens_in, Encoder, PixelFormat, Screen, Update, ENC_EXT_DESKTOP_SIZE, ENC_H264, ENC_ZRLE};
use crate::outbox::Outbox;
use crate::rfb::{vnc_response, PIXEL_FORMAT};
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(windows)]
use crate::share as platform;
#[cfg(target_os = "macos")]
use crate::host_mac as platform;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub password: String,
    /// Ask the person at this PC before each viewer gets in.
    pub ask: bool,
    /// ...but not viewers on this network or this tailnet (see `near`).
    pub trust_near: bool,
    /// A Mac's ⌘ (Super) acts as Ctrl here, so ⌘C/⌘V copy and paste.
    pub cmd_as_ctrl: bool,
    pub view_only: bool,
    /// Listen on this machine only (reached through an SSH tunnel), not the network.
    pub local_only: bool,
}

struct Running {
    port: u16,
    cfg: Arc<Config>,
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
    mdns: Option<crate::mdns::Registration>,
}

#[derive(Clone, serde::Serialize)]
pub struct Viewer {
    pub peer: String,
    pub since: u64,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static VIEWERS: Mutex<Vec<(u64, Viewer, TcpStream)>> = Mutex::new(Vec::new());
/// Called (on a worker thread) when viewers come or go; the home turns it into a message.
pub static ON_CHANGE: OnceLock<fn()> = OnceLock::new();

fn changed() {
    if let Some(f) = ON_CHANGE.get() {
        f();
    }
}

pub fn running() -> Option<u16> {
    RUNNING.lock().unwrap().as_ref().map(|r| r.port)
}

pub fn viewers() -> Vec<Viewer> {
    VIEWERS.lock().unwrap().iter().map(|(_, v, _)| v.clone()).collect()
}

pub fn kick_all() {
    for (_, _, s) in VIEWERS.lock().unwrap().iter() {
        let _ = s.shutdown(Shutdown::Both);
    }
}

pub fn start(cfg: Config) -> io::Result<()> {
    stop();
    let l = TcpListener::bind((if cfg.local_only { "127.0.0.1" } else { "0.0.0.0" }, cfg.port)).map_err(|e| io::Error::new(e.kind(), format!("포트 {}을(를) 열 수 없습니다: {e}", cfg.port)))?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let cfg = Arc::new(cfg);
    let port = l.local_addr()?.port(); // the real one, also when cfg.port is 0
    let c2 = cfg.clone();
    // Blocking accept, so a viewer is served at once; `stop()` wakes it with a connect.
    let thread = std::thread::spawn(move || {
        while !flag.load(Relaxed) {
            match l.accept() {
                Ok(_) if flag.load(Relaxed) => break,
                Ok((s, peer)) => {
                    let (cfg, flag) = (c2.clone(), flag.clone());
                    std::thread::spawn(move || {
                        if let Err(e) = serve(s, peer, &cfg, &flag) {
                            crate::log!("share {peer}: {e}");
                        }
                    });
                }
                Err(e) => {
                    crate::log!("share accept: {e}");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });
    let mdns = if cfg.local_only { None } else { crate::mdns::register("_rfb._tcp", port, platform::MDNS_TXT) };
    crate::log!("sharing this {} on port {port} (bonjour {})", platform::KIND, mdns.is_some());
    *RUNNING.lock().unwrap() = Some(Running { port, cfg, stop, thread, mdns });
    Ok(())
}

/// The web viewer's way in to this PC: the person already signed in with the web PIN, so
/// there's no on-screen prompt. Still a normal VNC login, over a private loopback socket.
/// `peer` is the browser (shown in the viewer list).
pub fn open_local(peer: SocketAddr) -> io::Result<crate::rfb::Raw> {
    let (cfg, stop) = {
        let r = RUNNING.lock().unwrap();
        let r = r.as_ref().ok_or_else(|| io::Error::new(io::ErrorKind::ConnectionRefused, "이 PC 공유가 꺼져 있습니다"))?;
        (Config { ask: false, ..(*r.cfg).clone() }, r.stop.clone())
    };
    let l = TcpListener::bind(("127.0.0.1", 0))?;
    let addr = l.local_addr()?;
    let pass = cfg.password.clone();
    std::thread::spawn(move || {
        if let Ok((s, _)) = l.accept() {
            if let Err(e) = serve(s, peer, &cfg, &stop) {
                crate::log!("share {peer} (web): {e}");
            }
        }
    });
    crate::rfb::open(&addr.to_string(), &crate::rfb::Creds { user: String::new(), pass })
}

pub fn stop() {
    let r = RUNNING.lock().unwrap().take();
    if let Some(r) = r {
        r.stop.store(true, Relaxed);
        // Wake the blocked accept; if that fails the thread can't be joined, so leave it.
        let woke = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], r.port)), Duration::from_secs(1));
        match woke {
            Ok(_) => {
                let _ = r.thread.join();
            }
            Err(e) => crate::log!("share stop: accept not woken: {e}"),
        }
        drop(r.mdns);
        kick_all();
        crate::log!("sharing stopped");
    }
}

// ---------------------------------------------------------------- one viewer

enum Msg {
    Format(PixelFormat),
    Encodings(Vec<i32>),
    Request { incremental: bool },
    /// EnableContinuousUpdates, Fence.
    Ext(demand::Ext),
    /// SetDesktopSize: this PC's displays can't be resized from here; it gets an answer.
    Resize,
}

fn rd<const N: usize>(r: &mut impl Read) -> io::Result<[u8; N]> {
    let mut b = [0u8; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

/// A viewer on this local network (private, link-local or loopback address) or on this PC's
/// tailnet (Tailscale's 100.64/10 and fd7a:115c:a1e0::/48): the user's own devices. The share
/// port isn't something people forward from the internet, so these can't be strangers'.
fn near(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    let ip = match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    };
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || (o[0] == 100 && o[1] & 0xc0 == 64)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_loopback() || s[0] & 0xffc0 == 0xfe80 || s[0] & 0xfe00 == 0xfc00 // link-local, ULA (Tailscale's too)
        }
    }
}

fn serve(mut c: TcpStream, peer: SocketAddr, cfg: &Config, stop: &AtomicBool) -> io::Result<()> {
    c.set_nodelay(true)?;
    c.set_read_timeout(Some(Duration::from_secs(30)))?;
    c.write_all(b"RFB 003.008\n")?;
    let v: [u8; 12] = rd(&mut c)?;
    let minor: u32 = std::str::from_utf8(&v[8..11]).ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    if minor >= 7 {
        c.write_all(&[1, 2])?;
        if rd::<1>(&mut c)?[0] != 2 {
            return Err(io::Error::other("client didn't pick VNC auth"));
        }
    } else {
        c.write_all(&2u32.to_be_bytes())?;
    }
    let mut challenge = [0u8; 16];
    let _ = getrandom::getrandom(&mut challenge);
    c.write_all(&challenge)?;
    let resp: [u8; 16] = rd(&mut c)?;
    let deny = |c: &mut TcpStream, why: &str| -> io::Result<()> {
        c.write_all(&1u32.to_be_bytes())?;
        if minor >= 8 {
            c.write_all(&(why.len() as u32).to_be_bytes())?;
            c.write_all(why.as_bytes())?;
        }
        Ok(())
    };
    let expected = vnc_response(&challenge, &cfg.password);
    if expected.iter().zip(resp).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 {
        crate::log!("share {peer}: wrong password");
        std::thread::sleep(Duration::from_secs(1)); // slow down guessing
        return deny(&mut c, "Authentication failed");
    }
    let trusted = cfg.trust_near && near(peer.ip());
    if trusted && cfg.ask {
        crate::log!("share {peer}: nearby device, no prompt");
    }
    if cfg.ask && !trusted && !platform::ask_user(&peer) {
        crate::log!("share {peer}: refused by the user");
        return deny(&mut c, "The person at this PC declined");
    }
    // A platform that can't show the screen yet (macOS: Screen Recording not granted) says so
    // instead of sending black: the viewer's Itda falls back to the usual way in.
    if let Err(why) = platform::ready() {
        crate::log!("share {peer}: not ready: {why}");
        return deny(&mut c, &why);
    }
    c.write_all(&0u32.to_be_bytes())?;
    let _shared: [u8; 1] = rd(&mut c)?;
    c.set_read_timeout(None)?;

    let cap = platform::capture();
    let size = {
        let mut c = cap.lock().unwrap();
        c.fit();
        (c.w, c.h)
    };
    let name = platform::name();
    let mut init = Vec::new();
    init.extend_from_slice(&(size.0 as u16).to_be_bytes());
    init.extend_from_slice(&(size.1 as u16).to_be_bytes());
    init.extend_from_slice(&PIXEL_FORMAT);
    init.extend_from_slice(&(name.len() as u32).to_be_bytes());
    init.extend_from_slice(name.as_bytes());
    c.write_all(&init)?;
    crate::log!("share {peer}: viewer in ({}x{})", size.0, size.1);

    let id = {
        let mut b = [0u8; 8];
        let _ = getrandom::getrandom(&mut b);
        u64::from_le_bytes(b)
    };
    VIEWERS.lock().unwrap().push((id, Viewer { peer: peer.ip().to_string(), since: crate::store::now() }, c.try_clone()?));
    changed();

    let (tx, rx) = channel();
    let reader = c.try_clone()?;
    let cfg2 = cfg.clone();
    std::thread::spawn(move || {
        let mut input = platform::Input::new(cfg2.cmd_as_ctrl, cfg2.view_only);
        let _ = read_client(BufReader::new(reader), &tx, &mut input);
        input.release_all();
    });
    let r = Outbox::new(c.try_clone()?).and_then(|out| pump(&out, &rx, &cap, size, stop));
    let _ = c.shutdown(Shutdown::Both);
    VIEWERS.lock().unwrap().retain(|(i, _, _)| *i != id);
    changed();
    crate::log!("share {peer}: viewer left");
    r
}

/// Client → server messages. Input is applied right here; the rest goes to the pump.
fn read_client(mut r: BufReader<TcpStream>, tx: &Sender<Msg>, input: &mut platform::Input) -> io::Result<()> {
    loop {
        // A held Ctrl/Alt/Win must not outlive the person holding it: a phone that goes to
        // sleep mid-gesture keeps the socket open but silent, and the PC would keep zooming
        // every app on Ctrl+wheel. Release modifiers after a quiet spell or a long hold.
        if input.modifiers_held() && input.last_key.elapsed() > platform::MOD_HOLD_MAX {
            input.release_modifiers("held too long");
        }
        let wait = r.buffer().is_empty() && input.modifiers_held();
        r.get_ref().set_read_timeout(wait.then_some(platform::MOD_QUIET))?;
        let t = match rd::<1>(&mut r) {
            Ok(b) => b[0],
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                input.release_modifiers("viewer went quiet");
                continue;
            }
            Err(e) => return Err(e),
        };
        if wait {
            r.get_ref().set_read_timeout(None)?;
        }
        match t {
            0 => {
                let b: [u8; 19] = rd(&mut r)?;
                let _ = tx.send(Msg::Format(PixelFormat::parse(&b[3..])));
            }
            2 => {
                let b: [u8; 3] = rd(&mut r)?;
                let n = u16::from_be_bytes([b[1], b[2]]) as usize;
                let mut e = vec![0u8; n * 4];
                r.read_exact(&mut e)?;
                let _ = tx.send(Msg::Encodings(e.chunks_exact(4).map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect()));
            }
            3 => {
                let b: [u8; 9] = rd(&mut r)?;
                let _ = tx.send(Msg::Request { incremental: b[0] != 0 });
            }
            4 => {
                let b: [u8; 7] = rd(&mut r)?;
                input.key(b[0] != 0, u32::from_be_bytes([b[3], b[4], b[5], b[6]]));
            }
            5 => {
                let b: [u8; 5] = rd(&mut r)?;
                input.pointer(b[0], u16::from_be_bytes([b[1], b[2]]), u16::from_be_bytes([b[3], b[4]]));
            }
            6 => {
                let b: [u8; 7] = rd(&mut r)?;
                let n = i32::from_be_bytes([b[3], b[4], b[5], b[6]]);
                let mut d = vec![0u8; n.unsigned_abs().min(16 << 20) as usize];
                r.read_exact(&mut d)?;
                if n > 0 {
                    let text = String::from_utf8(d).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
                    platform::clipboard_set(&text);
                }
            }
            251 => {
                // SetDesktopSize: we can't resize the PC's screens; the pump says so.
                let b: [u8; 7] = rd(&mut r)?;
                let mut d = vec![0u8; b[5] as usize * 16];
                r.read_exact(&mut d)?;
                let _ = tx.send(Msg::Resize);
            }
            t @ (demand::MSG_CONTINUOUS | demand::MSG_FENCE) => {
                let _ = tx.send(Msg::Ext(demand::read(t, &mut r)?));
            }
            other => return Err(io::Error::other(format!("unknown client message {other}"))),
        }
    }
}

const ENC_DESKTOP_SIZE: i32 = -223;

/// Shortest time between two updates (~30 fps), and with a video encoder (~60 fps).
const FRAME: Duration = Duration::from_millis(33);
const VIDEO_FRAME: Duration = Duration::from_millis(16);
/// How long the screen stands still before video is repainted losslessly.
const REFINE: Duration = Duration::from_millis(250);
/// Largest picture sent as video (hardware encoders and the viewers' decoders handle this).
const VIDEO_MAX: usize = 4096;

/// A platform's hardware video encoder for one viewer (one H.264 stream).
pub(crate) trait VideoEncode: Send {
    /// The encoded picture size (the screen's, rounded down to even).
    fn size(&self) -> (usize, usize);
    /// Encode a frame (rows of `stride` pixels); `key` asks for an IDR with parameter sets.
    fn encode(&mut self, px: &[u32], stride: usize, key: bool) -> io::Result<Vec<u8>>;
}
pub(crate) type Video = Box<dyn VideoEncode>;

/// Server → client: capture, diff, encode, send.
fn pump(out: &Outbox, rx: &Receiver<Msg>, cap: &Mutex<platform::Capture>, mut size: (usize, usize), stop: &AtomicBool) -> io::Result<()> {
    let mut pf = PixelFormat::parse(&PIXEL_FORMAT);
    let mut encs: Vec<i32> = Vec::new();
    let mut demand = Demand::default();
    // The displays the viewer was last told about (ExtendedDesktopSize), re-read every
    // second; and an unanswered SetDesktopSize.
    let mut told: Option<Vec<Screen>> = None;
    let mut screens: Vec<Screen> = Vec::new();
    let mut screens_read = Instant::now() - Duration::from_secs(10);
    let mut refuse_resize = false;
    let mut last_capture = Instant::now() - FRAME;
    let mut prev: Vec<u32> = Vec::new();
    let mut enc = Encoder::new();
    // H.264 for this viewer: the encoder (made when first needed), whether the platform has
    // one, whether the next video rectangle must reset the viewer's decoder, the last frame
    // painted losslessly, and when the screen last changed.
    let mut video: Option<Video> = None;
    let mut video_ok = true;
    let mut reset = true;
    let mut exact: Vec<u32> = Vec::new();
    let mut last_change = Instant::now();
    let mut u = Update::default();
    let mut clip_seq = platform::clipboard_seq();
    let mut clip_check = Instant::now();
    loop {
        if stop.load(Relaxed) {
            return Ok(());
        }
        if out.dead() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "viewer connection lost"));
        }
        // Wake for the viewer's messages, the next capture when an update is due, or a
        // clipboard check. Captures stay FRAME apart also when the screen doesn't change.
        let gap = if video.is_some() { VIDEO_FRAME } else { FRAME };
        let wait = match demand.due() {
            Some(_) if out.room() => gap.saturating_sub(last_capture.elapsed()),
            _ => gap,
        };
        let mut next = match rx.recv_timeout(wait.max(Duration::from_millis(1))) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let mut reply = Vec::new();
        while let Some(m) = next {
            match m {
                Msg::Format(p) => {
                    pf = p;
                    prev.clear();
                }
                Msg::Encodings(e) => {
                    demand.encodings(&e, &mut reply);
                    if !e.contains(&ENC_H264) {
                        video = None; // back to ZRLE; a later switch starts a new stream
                    }
                    reset = true;
                    encs = e;
                    told = None; // (re)introduce the displays
                }
                Msg::Resize => refuse_resize = true,
                Msg::Request { incremental } => demand.request(incremental),
                Msg::Ext(m) => {
                    if demand.ext(m, size.0, size.1, &mut reply) {
                        prev.clear();
                    }
                }
            }
            next = rx.try_recv().ok();
        }
        out.send(reply)?;
        // Local clipboard changed → offer it to the viewer.
        if clip_check.elapsed() > Duration::from_millis(500) {
            clip_check = Instant::now();
            let seq = platform::clipboard_seq();
            if seq != clip_seq {
                clip_seq = seq;
                if let Some(t) = platform::clipboard_get() {
                    let b = t.replace("\r\n", "\n").into_bytes();
                    let mut m = vec![3, 0, 0, 0];
                    m.extend_from_slice(&(b.len() as u32).to_be_bytes());
                    m.extend_from_slice(&b);
                    out.send(m)?;
                }
            }
        }
        if demand.stalled() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "viewer stopped acknowledging updates"));
        }
        let Some(incremental) = demand.due() else { continue };
        if last_capture.elapsed() < gap || !out.room() {
            continue; // (a viewer that's behind gets the changes together later)
        }
        last_capture = Instant::now();
        // The size this viewer was last given (another viewer's capture may have resized it).
        let (w0, h0) = size;
        let mut cap = cap.lock().unwrap();
        cap.fit(); // (grab does too) the virtual desktop's corner, read before the frame borrows cap
        let (ox, oy) = (cap.x, cap.y);
        let (w, h, frame) = cap.grab();
        size = (w, h);
        u.start();
        let ext = encs.contains(&ENC_EXT_DESKTOP_SIZE);
        if ext && ((w, h) != (w0, h0) || screens_read.elapsed() >= Duration::from_secs(1)) {
            screens_read = Instant::now();
            screens = screens_in(ox, oy, w, h, &platform::monitors());
        }
        if ext && (told.as_ref() != Some(&screens) || (w, h) != (w0, h0) || refuse_resize) {
            // Each display of this PC, so the viewer can show one of them (or one window each).
            // Reason 1 / status 1 answers a SetDesktopSize: resizing isn't allowed here.
            let answer = u16::from(refuse_resize);
            ext_desktop_size(&mut u.buf, answer, answer, w as u16, h as u16, &screens);
            u.rects += 1;
            told = Some(screens.clone());
            refuse_resize = false;
            if (w, h) != (w0, h0) {
                prev.clear();
            }
        } else if (w, h) != (w0, h0) && encs.contains(&ENC_DESKTOP_SIZE) {
            u.buf.extend_from_slice(&[0, 0, 0, 0]);
            u.buf.extend_from_slice(&(w as u16).to_be_bytes());
            u.buf.extend_from_slice(&(h as u16).to_be_bytes());
            u.buf.extend_from_slice(&ENC_DESKTOP_SIZE.to_be_bytes());
            u.rects += 1;
            prev.clear();
        }
        let full = !incremental || prev.len() != frame.len();
        let zrle = encs.contains(&ENC_ZRLE);
        let want_video = video_ok && encs.contains(&ENC_H264) && w <= VIDEO_MAX && h <= VIDEO_MAX;
        if want_video && video.as_ref().map(|v| v.size()) != Some((w & !1, h & !1)) {
            video = platform::video(w, h);
            video_ok = video.is_some();
            reset = true;
        }
        let mut lossless = true;
        match video.as_mut().filter(|_| want_video) {
            Some(v) if full || prev[..] != frame[..] => {
                let (vw, vh) = v.size();
                match v.encode(frame, w, reset || !incremental) {
                    Ok(data) => {
                        u.h264(0, 0, vw as u16, vh as u16, if reset { crate::enc::H264_RESET } else { 0 }, &data);
                        reset = false;
                        lossless = false;
                        last_change = Instant::now();
                    }
                    Err(e) => {
                        crate::log!("video encoder failed, back to ZRLE: {e}");
                        (video, video_ok) = (None, false);
                        enc.changes(&mut u, frame, &prev, w, h, true, |x, y, tw, th| demand.covers(x, y, tw, th), &pf, zrle)?;
                    }
                }
            }
            // Still: repaint what changed since the last exact picture, losslessly.
            Some(_) => {
                if last_change.elapsed() >= REFINE && exact[..] != frame[..] {
                    enc.changes(&mut u, frame, &exact, w, h, exact.len() != frame.len(), |x, y, tw, th| demand.covers(x, y, tw, th), &pf, zrle)?;
                }
            }
            None => enc.changes(&mut u, frame, &prev, w, h, full, |x, y, tw, th| demand.covers(x, y, tw, th), &pf, zrle)?,
        }
        if !u.finish() {
            continue; // nothing changed: keep the request pending
        }
        if lossless {
            exact.clear();
            exact.extend_from_slice(frame);
        }
        demand.sent(u.buf.len(), &mut u.buf);
        out.send(std::mem::replace(&mut u.buf, out.buffer()))?;
        prev.clear();
        prev.extend_from_slice(frame);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nearby_devices_are_this_network_or_tailnet() {
        let near = |s: &str| super::near(s.parse().unwrap());
        for ip in ["192.168.0.54", "10.0.0.7", "172.20.1.2", "127.0.0.1", "169.254.3.4", "100.101.10.3", "fe80::1c2d:3e4f:5a6b:7c8d", "fd7a:115c:a1e0::1", "::ffff:192.168.0.9"] {
            assert!(near(ip), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "211.234.10.1", "2001:db8::1", "::ffff:1.2.3.4"] {
            assert!(!near(ip), "{ip}");
        }
    }
}
