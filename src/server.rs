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
use crate::rfb::{vnc_response, ENC_EXT_CLIPBOARD, PIXEL_FORMAT};
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
    /// The capture has a new frame (macOS): wakes the pump to send it right away.
    Frame,
    /// The viewer's connection was closed (the frame waker keeps the channel open).
    Gone,
    /// A reply from the reader (extended clipboard), sent as is.
    Send(Vec<u8>),
}

// ---------------------------------------------------------------- extended clipboard
// Classic CutText is Latin-1 (noVNC turns anything else into '?', and reads our bytes as
// Latin-1): with the viewer's Extended Clipboard, text goes as UTF-8 both ways.

const CLIP_TEXT: u32 = 1;
const CLIP_CAPS: u32 = 1 << 24;
const CLIP_REQUEST: u32 = 1 << 25;
const CLIP_PEEK: u32 = 1 << 26;
const CLIP_NOTIFY: u32 = 1 << 27;
const CLIP_PROVIDE: u32 = 1 << 28;
const CLIP_MAX: u32 = 16 << 20;

/// ServerCutText carrying an extended clipboard message.
fn clip_msg(flags: u32, data: &[u8]) -> Vec<u8> {
    let mut m = vec![3, 0, 0, 0];
    m.extend_from_slice(&(-(4 + data.len() as i32)).to_be_bytes());
    m.extend_from_slice(&flags.to_be_bytes());
    m.extend_from_slice(data);
    m
}

fn clip_provide(text: &str) -> Vec<u8> {
    use flate2::{write::ZlibEncoder, Compression};
    let t = text.replace("\r\n", "\n");
    let mut inner = ((t.len() + 1) as u32).to_be_bytes().to_vec();
    inner.extend_from_slice(t.as_bytes());
    inner.push(0);
    let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
    let _ = z.write_all(&inner);
    clip_msg(CLIP_PROVIDE | CLIP_TEXT, &z.finish().unwrap_or_default())
}

/// The text in the viewer's Provide (zlib: length, UTF-8 text with a trailing NUL).
fn clip_text(data: &[u8]) -> Option<String> {
    let mut inner = Vec::new();
    // (Bounded: a small compressed message must not expand without end.)
    flate2::read::ZlibDecoder::new(data).take(CLIP_MAX as u64 + 4).read_to_end(&mut inner).ok()?;
    let n = u32::from_be_bytes(inner.get(..4)?.try_into().ok()?) as usize;
    let body = &inner[4..(4 + n).min(inner.len())];
    let body = body.strip_suffix(&[0]).unwrap_or(body);
    Some(String::from_utf8_lossy(body).replace("\r\n", "\n"))
}

/// The viewer's extended clipboard message: the reply to send, if any.
fn on_clip(p: &[u8], view_only: bool) -> Option<Vec<u8>> {
    let flags = u32::from_be_bytes(p.get(..4)?.try_into().ok()?);
    if flags & CLIP_CAPS != 0 {
        None // its capabilities: nothing to answer
    } else if flags & CLIP_REQUEST != 0 && flags & CLIP_TEXT != 0 {
        (!view_only).then(|| clip_provide(&platform::clipboard_get().unwrap_or_default()))
    } else if flags & CLIP_PEEK != 0 {
        Some(clip_msg(CLIP_NOTIFY | if view_only { 0 } else { CLIP_TEXT }, &[]))
    } else if flags & CLIP_NOTIFY != 0 {
        // It has new text: fetch it (unless this viewer may only watch).
        (flags & CLIP_TEXT != 0 && !view_only).then(|| clip_msg(CLIP_REQUEST | CLIP_TEXT, &[]))
    } else if flags & CLIP_PROVIDE != 0 && flags & CLIP_TEXT != 0 && !view_only {
        if let Some(t) = clip_text(&p[4..]) {
            platform::clipboard_set(&t);
        }
        None
    } else {
        None
    }
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
    // At most one Frame waits in the channel: the pump takes the newest frame anyway.
    let woken = Arc::new(AtomicBool::new(false));
    let (wake, woken2) = (tx.clone(), woken.clone());
    let waker = FrameWaker(platform::on_frame(Box::new(move || {
        if !woken2.swap(true, Relaxed) {
            let _ = wake.send(Msg::Frame);
        }
    })));
    let reader = c.try_clone()?;
    let cfg2 = cfg.clone();
    std::thread::spawn(move || {
        // The waker keeps the channel open, so say when the viewer is gone (also on a panic).
        let gone = Gone(tx);
        let mut input = platform::Input::new(cfg2.cmd_as_ctrl, cfg2.view_only);
        let _ = read_client(BufReader::new(reader), &gone.0, &mut input, cfg2.view_only);
        input.release_all();
    });
    let r = Outbox::new(c.try_clone()?).and_then(|out| pump(&out, &rx, &woken, &cap, size, stop, cfg.view_only));
    drop(waker);
    let _ = c.shutdown(Shutdown::Both);
    VIEWERS.lock().unwrap().retain(|(i, _, _)| *i != id);
    changed();
    crate::log!("share {peer}: viewer left");
    r
}

/// Client → server messages. Input is applied right here; the rest goes to the pump.
/// `view_only`: besides input, the viewer's clipboard isn't taken either.
fn read_client(mut r: BufReader<TcpStream>, tx: &Sender<Msg>, input: &mut platform::Input, view_only: bool) -> io::Result<()> {
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
                let _ = tx.send(Msg::Format(PixelFormat::from_viewer(&b[3..])?));
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
                if n < 0 {
                    if let Some(reply) = on_clip(&d, view_only) {
                        let _ = tx.send(Msg::Send(reply));
                    }
                } else if n > 0 && !view_only {
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
/// (macOS: a new ScreenCaptureKit frame goes out as soon as it arrives, at least VIDEO_FRAME
/// apart, woken by the capture (Msg::Frame) or at the latest every VIDEO_POLL; Windows keeps
/// its 16 ms capture timer.)
const VIDEO_FRAME: Duration = Duration::from_millis(if cfg!(target_os = "macos") { 14 } else { 16 });
const VIDEO_POLL: Duration = Duration::from_millis(16);
/// How long the screen stands still before video is repainted losslessly. On macOS long enough
/// that a short pause in motion doesn't start a full repaint that delays the next frames.
const REFINE: Duration = Duration::from_millis(if cfg!(target_os = "macos") { 500 } else { 250 });
/// Largest picture sent as video (hardware encoders and the viewers' decoders handle this).
const VIDEO_MAX: usize = 4096;

/// A platform's hardware video encoder for one viewer (one H.264 stream).
pub(crate) trait VideoEncode: Send {
    /// The encoded picture size (the screen's, rounded down to even).
    fn size(&self) -> (usize, usize);
    /// Encode a frame (rows of `stride` pixels); `key` asks for an IDR with parameter sets.
    fn encode(&mut self, px: &[u32], stride: usize, key: bool) -> io::Result<Vec<u8>>;
    /// macOS can encode the immutable SCK buffer itself, avoiding an encoder staging copy.
    /// Other implementations can keep using the CPU-pixel path.
    #[cfg(target_os = "macos")]
    fn encode_buffer(&mut self, _buffer: &objc2_core_video::CVPixelBuffer, _key: bool) -> Option<io::Result<Vec<u8>>> {
        None
    }
}
pub(crate) type Video = Box<dyn VideoEncode>;

/// Server → client: capture, diff, encode, send.
/// Frames the encoder gave nothing for. Skipping them is right for a rate-control drop, which
/// the next attempt makes up for; but nothing may stay unsent for long (an encoder that keeps
/// answering empty, the last picture before the screen stops): past MAX_SKIP the picture goes
/// out losslessly instead, as before.
#[derive(Default)]
struct Drops {
    since: Option<Instant>,
}

const MAX_SKIP: Duration = Duration::from_millis(100);

impl Drops {
    /// The encoder gave nothing at `now`: skip this frame?
    fn skip(&mut self, now: Instant) -> bool {
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) < MAX_SKIP {
            return true;
        }
        self.since = None; // sent losslessly: the next drop may be skipped again
        false
    }

    /// The encoder gave a picture.
    fn clear(&mut self) {
        self.since = None;
    }
}

/// How long a macOS video pump waits for its next capture: the rest of the frame gap once a
/// new frame has arrived, else until the fallback poll.
fn video_wait(gap: Duration, since_capture: Duration, arrived: bool) -> Duration {
    gap.saturating_sub(since_capture).max(if arrived { Duration::ZERO } else { VIDEO_POLL })
}

/// Stops a viewer's frame wakes when its connection ends.
struct FrameWaker(u64);
impl Drop for FrameWaker {
    fn drop(&mut self) {
        platform::off_frame(self.0);
    }
}

/// Tells the pump its viewer's connection was closed once the reader thread ends.
struct Gone(Sender<Msg>);
impl Drop for Gone {
    fn drop(&mut self) {
        let _ = self.0.send(Msg::Gone);
    }
}

fn pump(out: &Outbox, rx: &Receiver<Msg>, woken: &AtomicBool, cap: &Mutex<platform::Capture>, mut size: (usize, usize), stop: &AtomicBool, view_only: bool) -> io::Result<()> {
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
    // A capture frame arrived since the last capture (macOS): send it once the gap allows.
    let mut arrived = false;
    // The encoder dropped the last frame (macOS: its rate control under heavy motion): encode
    // the picture again at the next frame time, even if the capture has nothing newer.
    let mut redo = false;
    let mut drops = Drops::default();
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
    // The shared capture's frame this viewer last grabbed (see Capture::fresh_since).
    let mut seen = u64::MAX;
    let mut u = Update::default();
    let mut clip_seq = platform::clipboard_seq();
    let mut clip_check = Instant::now();
    let mut ext_clip = false;
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
            Some(_) if out.room() && video.is_some() && cfg!(target_os = "macos") => video_wait(gap, last_capture.elapsed(), arrived || redo),
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
                    if !e.contains(&ENC_H264) && video.take().is_some() {
                        // Back to ZRLE while the viewer shows a lossy picture: repaint it all.
                        prev.clear();
                        exact.clear();
                    }
                    if e.contains(&ENC_EXT_CLIPBOARD) && !ext_clip {
                        // Our capabilities: text, and the actions noVNC needs to send UTF-8.
                        ext_clip = true;
                        reply.extend(clip_msg(CLIP_CAPS | CLIP_TEXT | CLIP_REQUEST | CLIP_PEEK | CLIP_NOTIFY | CLIP_PROVIDE, &CLIP_MAX.to_be_bytes()));
                    }
                    reset = true;
                    encs = e;
                    told = None; // (re)introduce the displays
                }
                Msg::Resize => refuse_resize = true,
                Msg::Frame => {
                    woken.store(false, Relaxed);
                    arrived = true;
                }
                Msg::Gone => return Ok(()),
                Msg::Send(m) => reply.extend(m),
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
        // Local clipboard changed → offer it to the viewer (not one that may only watch).
        if !view_only && clip_check.elapsed() > Duration::from_millis(500) {
            clip_check = Instant::now();
            let seq = platform::clipboard_seq();
            if seq != clip_seq {
                clip_seq = seq;
                if let Some(t) = platform::clipboard_get().filter(|_| ext_clip) {
                    out.send(clip_provide(&t))?;
                } else if let Some(t) = platform::clipboard_get() {
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
        // With video and nothing new captured, there is nothing to encode until the still
        // repaint is due: skip the copy and compare of a whole frame.
        // (A viewer whose history was just reset, with `prev` cleared, always grabs.)
        if video.is_some() && incremental && !prev.is_empty() && last_change.elapsed() < REFINE && !redo && !cap.lock().unwrap().fresh_since(seen) {
            // (A Frame can name a frame this pump already took: back to the fallback poll.)
            arrived = false;
            continue;
        }
        last_capture = Instant::now();
        arrived = false;
        redo = false;
        // The size this viewer was last given (another viewer's capture may have resized it).
        let (w0, h0) = size;
        #[cfg(target_os = "macos")]
        let snapshot = cap.lock().unwrap().snapshot();
        #[cfg(target_os = "macos")]
        let (ox, oy, w, h, frame) = (snapshot.x, snapshot.y, snapshot.w, snapshot.h, snapshot.pixels.as_slice());
        #[cfg(target_os = "macos")]
        { seen = snapshot.generation; }
        // Windows keeps its Desktop Duplication frame borrowed until this update is done.
        // On macOS the capture lock is already released: slow encoders and lossless
        // repaints cannot stop another viewer from taking its own immutable snapshot.
        #[cfg(not(target_os = "macos"))]
        let mut cap = cap.lock().unwrap();
        #[cfg(not(target_os = "macos"))]
        cap.fit(); // (grab does too) the virtual desktop's corner, read before the frame borrows cap
        #[cfg(not(target_os = "macos"))]
        let (ox, oy) = (cap.x, cap.y);
        // The frame this grab gives (one that turns up meanwhile only costs an extra pass).
        #[cfg(not(target_os = "macos"))]
        let g = cap.generation();
        #[cfg(not(target_os = "macos"))]
        { seen = g.wrapping_add(u64::from(cap.fresh_since(g))); }
        #[cfg(not(target_os = "macos"))]
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
                let key = reset || !incremental;
                let encode = Instant::now();
                #[cfg(target_os = "macos")]
                let encoded = snapshot.native.as_deref().and_then(|buffer| v.encode_buffer(buffer, key))
                    .unwrap_or_else(|| v.encode(frame, w, key));
                #[cfg(not(target_os = "macos"))]
                let encoded = v.encode(frame, w, key);
                crate::trace::since("encode", encode);
                match encoded {
                    // Dropped by the encoder's rate control (macOS, heavy motion such as video):
                    // send nothing now and encode the picture again next frame. A lossless
                    // repaint of fast-changing pixels would be large and hold up what follows.
                    Ok(data) if data.is_empty() && cfg!(target_os = "macos") && incremental && !full && u.rects == 0 && drops.skip(Instant::now()) => {
                        crate::trace::add("dropped", 1.0);
                        redo = true;
                        continue;
                    }
                    // Nothing out for this frame (an encoder still buffering): send it losslessly
                    // and keep any pending reset for the first real picture.
                    Ok(data) if data.is_empty() => {
                        enc.changes(&mut u, frame, &prev, w, h, full, |x, y, tw, th| demand.covers(x, y, tw, th), &pf, zrle)?;
                        // Only the changed tiles are exact now: unless the picture was exact before
                        // (no lossy video since), the still repaint must still cover everything.
                        lossless = full || (exact.len() == prev.len() && exact[..] == prev[..]);
                    }
                    Ok(data) => {
                        drops.clear();
                        u.h264(0, 0, vw as u16, vh as u16, if reset { crate::enc::H264_RESET } else { 0 }, &data);
                        reset = false;
                        lossless = false;
                        last_change = Instant::now();
                        // The whole picture is now lossy: the still repaint must cover it all.
                        exact.clear();
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
        crate::trace::since("update", last_capture);
        prev.clear();
        prev.extend_from_slice(frame);
    }
}

#[cfg(test)]
mod tests {
    /// Korean text survives the Extended Clipboard both ways (Provide is what noVNC parses),
    /// and the viewer's Notify is answered with a Request.
    #[test]
    fn extended_clipboard_text() {
        use super::*;
        let m = clip_provide("안녕 Itda\r\n둘째 줄");
        assert_eq!(&m[..4], &[3, 0, 0, 0]);
        assert_eq!(i32::from_be_bytes(m[4..8].try_into().unwrap()), -((m.len() - 8) as i32));
        assert_eq!(u32::from_be_bytes(m[8..12].try_into().unwrap()), CLIP_PROVIDE | CLIP_TEXT);
        assert_eq!(clip_text(&m[12..]).as_deref(), Some("안녕 Itda\n둘째 줄"));
        let notify = (CLIP_NOTIFY | CLIP_TEXT).to_be_bytes();
        assert_eq!(on_clip(&notify, false), Some(clip_msg(CLIP_REQUEST | CLIP_TEXT, &[])));
        assert_eq!(on_clip(&notify, true), None);
        assert_eq!(on_clip(&(CLIP_CAPS | CLIP_TEXT).to_be_bytes(), false), None);
    }

    /// Real VideoToolbox -> production pump -> TCP -> independent ZRLE solid-tile reader.
    /// Opt-in: no screen capture, input, permissions or production listener involved.
    /// ITDA_REFINE_DIMS=3840x2160 cargo test --release fixed_frame_refines -- --ignored --nocapture
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn fixed_frame_refines_losslessly_after_video() {
        use super::*;
        let dims = std::env::var("ITDA_REFINE_DIMS").unwrap_or_else(|_| "1920x1080".into());
        let (w, h) = dims.split_once('x').unwrap();
        let (w, h): (usize, usize) = (w.parse().unwrap(), h.parse().unwrap());
        assert!((16..=4096).contains(&w) && (16..=4096).contains(&h));
        let frame: Vec<u32> = (0..w*h).map(|i| if (i % w / 64 + i / w / 64) % 2 == 0 { 0xffffff } else { 0x202020 }).collect();
        let cap = Mutex::new(platform::fixed_capture(w, h, frame.clone()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (socket, _) = listener.accept().unwrap();
        let (tx, rx) = channel();
        let worker = std::thread::spawn(move || {
            let out = Outbox::new(socket).unwrap();
            pump(&out, &rx, &AtomicBool::new(false), &cap, (w, h), &AtomicBool::new(false), true).unwrap();
        });
        tx.send(Msg::Encodings(vec![ENC_H264, ENC_ZRLE])).unwrap();
        tx.send(Msg::Request { incremental: false }).unwrap();
        let read = |c: &mut TcpStream| {
            let mut header = [0; 4]; c.read_exact(&mut header).unwrap();
            assert_eq!(header[0], 0);
            let count = u16::from_be_bytes([header[2], header[3]]);
            let mut rects = Vec::new();
            for _ in 0..count {
                let mut r = [0; 12]; c.read_exact(&mut r).unwrap();
                let enc = i32::from_be_bytes(r[8..12].try_into().unwrap());
                assert!(enc == ENC_H264 || enc == ENC_ZRLE);
                let mut len = [0; 4]; c.read_exact(&mut len).unwrap();
                let len = u32::from_be_bytes(len) as usize; assert!(len < 64*1024*1024);
                if enc == ENC_H264 { let mut flags = [0; 4]; c.read_exact(&mut flags).unwrap(); }
                let mut bytes = vec![0; len]; c.read_exact(&mut bytes).unwrap();
                rects.push((r, enc, bytes));
            }
            rects
        };
        let video = read(&mut client);
        assert_eq!(video.len(), 1); assert_eq!(video[0].1, ENC_H264);
        let last_video = Instant::now();
        tx.send(Msg::Request { incremental: true }).unwrap();
        let exact = read(&mut client);
        let delay = last_video.elapsed();
        let (mut area, mut bytes) = (0, 0);
        let mut z = flate2::Decompress::new(true);
        for (r, enc, payload) in exact {
            assert_eq!(enc, ENC_ZRLE);
            let u16_at = |i| u16::from_be_bytes([r[i], r[i+1]]) as usize;
            let (x, y, rw, rh) = (u16_at(0), u16_at(2), u16_at(4), u16_at(6));
            let mut raw = Vec::with_capacity(rw*rh*4+4096);
            z.decompress_vec(&payload, &mut raw, flate2::FlushDecompress::Sync).unwrap();
            let mut at = 0;
            for ty in (0..rh).step_by(64) { for tx in (0..rw).step_by(64) {
                assert_eq!(raw[at], 1, "expected independent solid-tile representation");
                let v = (frame[(y+ty)*w+x+tx] & 255) as u8;
                assert_eq!(&raw[at+1..at+4], &[v, v, v]); at += 4;
            }}
            assert_eq!(at, raw.len()); area += rw*rh; bytes += payload.len();
        }
        assert_eq!(area, w*h, "refinement covers every framebuffer pixel");
        assert!(delay >= REFINE.saturating_sub(Duration::from_millis(20)) && delay < REFINE+Duration::from_secs(2), "{delay:?}");
        tx.send(Msg::Gone).unwrap(); worker.join().unwrap();
        println!("refine {w}x{h}: received after {:.2} ms; {bytes} compressed bytes; {area} exact pixels", delay.as_secs_f64()*1000.);
    }

    #[test]
    fn dropped_frames_are_skipped_only_briefly() {
        use super::{Drops, MAX_SKIP};
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut d = Drops::default();
        // A few drops, then a picture: all skipped, and the next drops start afresh.
        assert!(d.skip(at(0)) && d.skip(at(16)) && d.skip(at(32)));
        d.clear();
        assert!(d.skip(at(200)));
        // Empty answers that go on (the last picture before the screen stops, a failing
        // encoder): past MAX_SKIP the picture is sent losslessly.
        let mut t = 200;
        while d.skip(at(t)) {
            t += 16;
            assert!(Duration::from_millis(t - 200) <= MAX_SKIP + Duration::from_millis(16), "skipped for {t} ms");
        }
        assert!(Duration::from_millis(t - 200) >= MAX_SKIP);
        // Then a following drop may be skipped again.
        assert!(d.skip(at(t + 16)));
    }

    #[test]
    fn video_waits_for_the_frame_gap_or_the_fallback_poll() {
        use super::{video_wait, VIDEO_FRAME, VIDEO_POLL};
        use std::time::Duration;
        let ms = Duration::from_millis;
        // A frame arrived: go as soon as the gap allows.
        assert_eq!(video_wait(VIDEO_FRAME, ms(4), true), VIDEO_FRAME - ms(4));
        assert_eq!(video_wait(VIDEO_FRAME, VIDEO_FRAME + ms(5), true), Duration::ZERO);
        // None arrived (or the one named was already taken): don't spin, poll as a fallback.
        assert_eq!(video_wait(VIDEO_FRAME, VIDEO_FRAME + ms(5), false), VIDEO_POLL);
    }

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
