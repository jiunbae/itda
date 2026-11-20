//! Shared state between the UI thread and the network thread, and the receive loop.

use crate::fb::Fb;
use crate::rfb::*;
use flate2::{Decompress, FlushDecompress};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::windows::io::AsRawSocket;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicU8, Ordering::*};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Networking::WinSock::{WSAIoctl, TCP_INFO_v0};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub const WM_FB: u32 = WM_APP + 1;
pub const WM_CURSOR: u32 = WM_APP + 2;
pub const WM_RESIZED: u32 = WM_APP + 3;
pub const WM_NETERR: u32 = WM_APP + 4;
pub const WM_STATUS: u32 = WM_APP + 5;
pub const WM_CLIP: u32 = WM_APP + 6;

const SIO_TCP_INFO: u32 = 0xd800_0027;
/// Who is looking at the remote screen (`Shared::set_view`). Each update is requested only
/// after the last one arrived, so this sets how fast the Mac sends: as fast as it can while
/// a window is in front; BACKGROUND_EVERY apart while they're all behind other windows; not
/// at all while they're all minimized (the liveness probe still runs, every PAUSED_PROBE).
/// A busy Mac screen otherwise streams tens of Mbps to a window nobody is watching.
pub const VIEW_LIVE: u8 = 0;
pub const VIEW_BACKGROUND: u8 = 1;
pub const VIEW_PAUSED: u8 = 2;
const BACKGROUND_EVERY: Duration = Duration::from_millis(500);
/// A PC slower than the video (decoding and drawing a picture took longer than this): the next
/// picture is asked for BUSY_WAIT (as a fraction) of that time later, so this viewer keeps no
/// more than about two thirds of the time busy with it rather than all of it. (A 4-core PC
/// decoding 2560x1440 took 1.6 cores for 19 pictures a second.) Faster PCs are never held back.
const BUSY: Duration = Duration::from_millis(20);
const BUSY_WAIT: (u32, u32) = (3, 2);
/// How long a new view scale must hold before the server hears of it (a window being dragged
/// larger passes through many).
const VIEW_SCALE_HOLD: Duration = Duration::from_millis(400);

/// Silence longer than this at a message boundary triggers a liveness probe.
const IDLE_PROBE: Duration = Duration::from_secs(4);
/// While every window is minimized, silence is expected: probe this rarely instead. A probe is
/// a non-incremental request, which an H.264 server (Itda's own) answers with a key frame of
/// the whole screen, about 280 KB at 1920x1080: every IDLE_PROBE, that alone kept a minimized
/// viewer pulling ~70 KB/s from a busy Mac.
const PAUSED_PROBE: Duration = Duration::from_secs(30);

pub struct Cursor {
    pub w: usize,
    pub h: usize,
    pub hx: usize,
    pub hy: usize,
    /// Premultiplied BGRA (alpha is 0 or 255).
    pub px: Vec<u32>,
}

/// One physical display inside the framebuffer (from ExtendedDesktopSize).
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub id: u32,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub flags: u32,
}

pub struct Shared {
    pub fb: Mutex<Fb>,
    /// Displays reported by the server; empty if it doesn't support ExtendedDesktopSize.
    pub screens: Mutex<Vec<Screen>>,
    pub ext_clip: AtomicBool,
    /// Latest local clipboard text, served on request.
    pub clip_out: Mutex<String>,
    /// Clipboard text from the server, waiting for the UI thread.
    pub clip_in: Mutex<Option<String>>,
    pub cursor: Mutex<Option<Cursor>>,
    pub error: Mutex<Option<String>>,
    /// `error` is the server turning down the credentials (`rfb::auth_failed`).
    pub error_auth: AtomicBool,
    /// Connection status shown in the title ("" = connected).
    pub status: Mutex<String>,
    out: Mutex<TcpStream>,
    sock: AtomicU64,
    pub hwnd: AtomicIsize,
    /// Reconnect automatically when the link drops (per-host option).
    pub auto_reconnect: AtomicBool,
    /// Reconnect attempts so far (0 = connected), for the overlay.
    pub attempt: AtomicU32,
    pub fb_w: AtomicU32,
    pub fb_h: AtomicU32,
    pub updates: AtomicU64,
    pending: AtomicBool,
    /// VIEW_LIVE / VIEW_BACKGROUND / VIEW_PAUSED.
    view: AtomicU8,
    /// An update arrived and the next one wasn't requested yet (not live).
    owed: AtomicBool,
    last_request: Mutex<Instant>,
    /// The owed request waits until then (BUSY).
    not_before: Mutex<Option<Instant>>,
    /// How long the last video picture took to decode and draw.
    busy: Mutex<Duration>,
    /// The largest scale a window shows the screen at, in thousandths (1000: full size or
    /// more), since when, and what the server was last told (ENC_ITDA_VIEW_SCALE).
    view_scale: Mutex<(u32, Instant)>,
    scale_told: AtomicU32,
    /// This connection's server has sent H.264 (only Itda's do): it may hear the view scale.
    video_seen: AtomicBool,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

pub fn sh() -> &'static Shared {
    SHARED.get().unwrap()
}

pub fn init(out: TcpStream, w: u16, h: u16) -> io::Result<()> {
    let fb = Fb::new(w as usize, h as usize).ok_or_else(|| io::Error::other(tr!("net.noMemory")))?;
    let s = Shared {
        fb: Mutex::new(fb),
        screens: Mutex::new(Vec::new()),
        ext_clip: AtomicBool::new(false),
        clip_out: Mutex::new(String::new()),
        clip_in: Mutex::new(None),
        cursor: Mutex::new(None),
        error: Mutex::new(None),
        error_auth: AtomicBool::new(false),
        status: Mutex::new(String::new()),
        sock: AtomicU64::new(out.as_raw_socket()),
        out: Mutex::new(out),
        hwnd: AtomicIsize::new(0),
        auto_reconnect: AtomicBool::new(true),
        attempt: AtomicU32::new(0),
        fb_w: AtomicU32::new(w as u32),
        fb_h: AtomicU32::new(h as u32),
        updates: AtomicU64::new(0),
        pending: AtomicBool::new(false),
        view: AtomicU8::new(VIEW_LIVE),
        owed: AtomicBool::new(false),
        last_request: Mutex::new(Instant::now()),
        not_before: Mutex::new(None),
        busy: Mutex::new(Duration::ZERO),
        view_scale: Mutex::new((1000, Instant::now())),
        scale_told: AtomicU32::new(1000),
        video_seen: AtomicBool::new(false),
    };
    let _ = SHARED.set(s);
    Ok(())
}

impl Shared {
    pub fn send(&self, b: &[u8]) {
        let _ = self.out.lock().unwrap().write_all(b);
    }

    pub fn pointer(&self, mask: u8, x: u16, y: u16) {
        let [x0, x1] = x.to_be_bytes();
        let [y0, y1] = y.to_be_bytes();
        self.send(&[5, mask, x0, x1, y0, y1]);
    }

    pub fn key(&self, down: bool, sym: u32) {
        let [a, b, c, d] = sym.to_be_bytes();
        self.send(&[4, down as u8, 0, 0, a, b, c, d]);
    }

    pub fn size(&self) -> (u32, u32) {
        (self.fb_w.load(Relaxed), self.fb_h.load(Relaxed))
    }

    fn request(&self, incremental: bool) {
        let (w, h) = self.size();
        *self.last_request.lock().unwrap() = Instant::now();
        self.send(&update_request(incremental, 0, 0, w as u16, h as u16));
    }

    /// An update arrived: ask for the next one now, later, or when someone looks again.
    fn request_next(&self) {
        let busy = *self.busy.lock().unwrap();
        if self.view.load(SeqCst) == VIEW_LIVE && busy < BUSY {
            self.request(true);
        } else {
            if busy >= BUSY {
                *self.not_before.lock().unwrap() = Some(Instant::now() + busy * BUSY_WAIT.0 / BUSY_WAIT.1);
            }
            self.owed.store(true, SeqCst);
        }
    }

    /// How long the last video picture took to decode and draw (BUSY).
    fn video_took(&self, t: Duration) {
        *self.busy.lock().unwrap() = t;
    }

    /// UI side: the largest scale (device pixels per remote pixel) any shown window draws the
    /// screen at. Itda's server then sends no larger a video picture.
    pub fn set_view_scale(&self, scale: f64) {
        let n = crate::enc::view_scale_encoding(scale).map_or(1000, |e| (e - crate::enc::ENC_ITDA_VIEW_SCALE) as u32);
        let mut v = self.view_scale.lock().unwrap();
        if v.0 != n {
            *v = (n, Instant::now());
        }
    }

    /// Tell the server a view scale that has held for VIEW_SCALE_HOLD, once it has sent H.264
    /// (SetEncodings again: other servers never see the list change).
    fn tell_view_scale(&self) {
        let (n, since) = *self.view_scale.lock().unwrap();
        let told = self.scale_told.load(SeqCst);
        if n == told || since.elapsed() < VIEW_SCALE_HOLD || !self.video_seen.load(SeqCst) || !crate::h264dec_win::available() {
            return;
        }
        // (A reconnect in between resets what was told: then the new connection hears it.)
        if self.scale_told.compare_exchange(told, n, SeqCst, SeqCst).is_err() {
            return;
        }
        crate::log!("view scale: {n}/1000");
        self.send(&set_encodings(true, n));
    }

    /// UI side: whether any window shows the remote screen (see VIEW_LIVE).
    pub fn set_view(&self, v: u8) {
        if self.view.swap(v, SeqCst) != v {
            crate::log!("view: {}", ["live", "background", "paused"][v as usize % 3]);
            self.pace();
        }
    }

    /// Silence at a message boundary: time to check the link is alive? (Rarely while paused.)
    fn probe_due(&self) -> bool {
        self.view.load(SeqCst) != VIEW_PAUSED || self.last_request.lock().unwrap().elapsed() >= PAUSED_PROBE
    }

    /// The liveness probe: a 1x1 refresh, which any server answers.
    fn probe(&self) {
        *self.last_request.lock().unwrap() = Instant::now();
        self.send(&update_request(false, 0, 0, 1, 1));
    }

    /// Send an owed request if the view allows it now.
    fn pace(&self) {
        let due = match self.view.load(SeqCst) {
            VIEW_LIVE => self.not_before.lock().unwrap().is_none_or(|t| Instant::now() >= t),
            VIEW_BACKGROUND => self.last_request.lock().unwrap().elapsed() >= BACKGROUND_EVERY,
            _ => false,
        };
        if due && self.owed.swap(false, SeqCst) {
            *self.not_before.lock().unwrap() = None;
            self.request(true);
        }
    }

    pub fn post(&self, msg: u32) {
        unsafe { PostMessageW(self.hwnd.load(Relaxed) as _, msg, 0, 0) };
    }

    fn set_status(&self, st: String) {
        *self.status.lock().unwrap() = st;
        self.post(WM_STATUS);
    }

    /// UI side: acknowledge the WM_FB notification before taking `dirty`.
    pub fn ack(&self) {
        self.pending.store(false, SeqCst);
    }

    /// Ask the server to change its resolution (ExtendedDesktopSize servers only).
    pub fn set_desktop_size(&self, w: u16, h: u16) {
        let screens = self.screens.lock().unwrap().clone();
        let Some(first) = screens.first() else { return };
        let mut m = vec![251, 0];
        m.extend_from_slice(&w.to_be_bytes());
        m.extend_from_slice(&h.to_be_bytes());
        m.extend_from_slice(&[1, 0]);
        m.extend_from_slice(&first.id.to_be_bytes());
        m.extend_from_slice(&[0, 0, 0, 0]);
        m.extend_from_slice(&w.to_be_bytes());
        m.extend_from_slice(&h.to_be_bytes());
        m.extend_from_slice(&first.flags.to_be_bytes());
        crate::log!("SetDesktopSize {w}x{h}");
        self.send(&m);
    }

    /// Drop the current connection; the network thread reconnects.
    pub fn reconnect(&self) {
        let _ = self.out.lock().unwrap().shutdown(Shutdown::Both);
    }

    /// Kernel-measured TCP round-trip time.
    pub fn rtt_ms(&self) -> Option<f64> {
        let ver = 0u32;
        let mut info: TCP_INFO_v0 = unsafe { std::mem::zeroed() };
        let mut n = 0u32;
        let r = unsafe {
            WSAIoctl(self.sock.load(Relaxed) as usize, SIO_TCP_INFO, &ver as *const u32 as _, 4,
                &mut info as *mut TCP_INFO_v0 as _, size_of::<TCP_INFO_v0>() as u32, &mut n, std::ptr::null_mut(), None)
        };
        (r == 0).then(|| info.RttUs as f64 / 1000.0)
    }
}

/// Produces the address to (re)connect to: the Mac itself, or the local end of an SSH tunnel.
pub type Dial = std::sync::Arc<dyn Fn() -> io::Result<String> + Send + Sync>;

/// Receive on a background thread; on connection loss, reconnect with backoff.
pub fn spawn(mut reader: BufReader<Rx>, dial: Dial, creds: Creds) {
    // Requests held back while nobody is looking go out from here.
    std::thread::spawn(|| loop {
        // (Finer while a request waits out BUSY.)
        let wait = sh().not_before.lock().unwrap().map_or(Duration::from_millis(50), |t| t.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(1), Duration::from_millis(50)));
        std::thread::sleep(wait);
        sh().pace();
        sh().tell_view_scale();
    });
    std::thread::spawn(move || loop {
        let _ = reader.get_ref().0.set_read_timeout(Some(IDLE_PROBE));
        let e = recv_loop(reader).unwrap_err();
        crate::log!("disconnected: {e}");
        let s = sh();
        if !s.auto_reconnect.load(Relaxed) {
            *s.error.lock().unwrap() = Some(tr!("net.lost", error = e));
            s.post(WM_NETERR);
            return;
        }
        let mut attempt = 0u32;
        reader = loop {
            attempt += 1;
            s.attempt.store(attempt, Relaxed);
            crate::session::emit(serde_json::json!({"ev": "reconnecting", "n": attempt}));
            s.set_status(tr!("net.reconnecting", n = attempt));
            let ms = if attempt == 1 { 300 } else { (500u64 << attempt.min(5)).min(8000) };
            std::thread::sleep(Duration::from_millis(ms));
            match dial().and_then(|addr| connect(&addr, &creds)) {
                Ok(sess) => {
                    s.sock.store(sess.writer.as_raw_socket(), Relaxed);
                    s.ext_clip.store(false, Relaxed);
                    s.video_seen.store(false, SeqCst);
                    s.scale_told.store(1000, SeqCst);
                    s.video_took(Duration::ZERO);
                    *s.not_before.lock().unwrap() = None;
                    s.screens.lock().unwrap().clear();
                    *s.out.lock().unwrap() = sess.writer;
                    let (w, h) = (sess.width as u32, sess.height as u32);
                    if s.size() != (w, h) {
                        if !s.fb.lock().unwrap().resize(w as usize, h as usize) {
                            *s.error.lock().unwrap() = Some(tr!("net.noResizeMemory").into());
                            s.post(WM_NETERR);
                            return;
                        }
                        s.fb_w.store(w, Relaxed);
                        s.fb_h.store(h, Relaxed);
                        s.post(WM_RESIZED);
                    }
                    s.attempt.store(0, Relaxed);
                    s.set_status(String::new());
                    crate::session::emit(serde_json::json!({"ev": "reconnected"}));
                    crate::log!("reconnected after {attempt} attempt(s)");
                    break sess.reader;
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    s.error_auth.store(auth_failed(&e), Relaxed);
                    *s.error.lock().unwrap() = Some(e.to_string());
                    s.post(WM_NETERR);
                    return;
                }
                Err(e) => crate::log!("reconnect failed: {e}"),
            }
        };
    });
}

/// Block until the next server message starts. A static screen means silence, so on
/// timeout ask for a 1x1 refresh; if that goes unanswered too, the link is dead.
fn wait_msg(r: &mut BufReader<Rx>, s: &Shared) -> io::Result<()> {
    let mut probed = false;
    loop {
        match r.fill_buf() {
            Ok(b) if b.is_empty() => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(_) => return Ok(()),
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {
                if probed {
                    return err(tr!("net.noResponse"));
                }
                if s.probe_due() {
                    s.probe();
                    probed = true;
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn recv_loop(mut r: BufReader<Rx>) -> io::Result<()> {
    let s = sh();
    let mut z = Decompress::new(true);
    let mut zin = Vec::new();
    let mut zout = Vec::new();
    let mut raw = Vec::new();
    let mut video = Video::default();
    let mut updates = 0u64;
    loop {
        wait_msg(&mut r, s)?;
        match rd_u8(&mut r)? {
            0 => {
                rd_u8(&mut r)?;
                let n = rd_u16(&mut r)?;
                updates += 1;
                s.updates.fetch_add(1, Relaxed);
                let verbose = updates <= 3;
                // Ask for the next frame now so the server encodes it while we decode this one
                // (unless nobody is looking: request_next).
                s.request_next();
                let mut resized = false;
                let mut had_video = false;
                for _ in 0..n {
                    let x = rd_u16(&mut r)? as usize;
                    let y = rd_u16(&mut r)? as usize;
                    let w = rd_u16(&mut r)? as usize;
                    let h = rd_u16(&mut r)? as usize;
                    let enc = rd_u32(&mut r)? as i32;
                    if verbose {
                        crate::log!("update #{updates}: rect {x},{y} {w}x{h} enc={enc}");
                    }
                    // Pixel data is read into buffers of the rectangle's size: one that isn't
                    // on the screen (or a cursor larger than any) ends the connection first.
                    let outside = match enc {
                        ENC_RAW | ENC_ZRLE => !s.fb.lock().unwrap().fits(x, y, w, h),
                        ENC_CURSOR => w > 512 || h > 512,
                        _ => false,
                    };
                    if outside {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, tr!("net.outside")));
                    }
                    match enc {
                        ENC_RAW => {
                            raw.resize(w * h * 4, 0);
                            r.read_exact(&mut raw)?;
                            let mut fb = s.fb.lock().unwrap();
                            // (An empty rectangle has nothing to draw.)
                            if w > 0 && h > 0 && fb.fits(x, y, w, h) {
                                fb.raw(x, y, w, h, &raw);
                                fb.mark(x, y, w, h);
                            }
                        }
                        ENC_COPYRECT => {
                            let sx = rd_u16(&mut r)? as usize;
                            let sy = rd_u16(&mut r)? as usize;
                            let mut fb = s.fb.lock().unwrap();
                            if fb.fits(x, y, w, h) && fb.fits(sx, sy, w, h) {
                                fb.copy(sx, sy, x, y, w, h);
                                fb.mark(x, y, w, h);
                            }
                        }
                        ENC_ZRLE => {
                            // A rectangle's data can't sensibly be larger than its pixels; a
                            // server claiming more (or inflating past that) is cut off.
                            let most = w * h * 4 + (1 << 20);
                            let len = rd_u32(&mut r)? as usize;
                            if len > most {
                                return Err(io::Error::new(io::ErrorKind::InvalidData, "ZRLE rectangle too large"));
                            }
                            zin.resize(len, 0);
                            r.read_exact(&mut zin)?;
                            inflate(&mut z, &zin, &mut zout, most)?;
                            let mut fb = s.fb.lock().unwrap();
                            if w > 0 && h > 0 && fb.fits(x, y, w, h) {
                                fb.zrle(&zout, x, y, w, h).ok_or_else(|| io::Error::other("bad ZRLE data"))?;
                                fb.mark(x, y, w, h);
                            }
                        }
                        ENC_H264 => {
                            // Like ZRLE, the length is bounded before anything is allocated; a
                            // server past that is broken, not merely hard to decode.
                            let len = rd_u32(&mut r)? as usize;
                            let flags = rd_u32(&mut r)?;
                            if len > crate::yuv::h264_most(w, h) {
                                return Err(io::Error::new(io::ErrorKind::InvalidData, "H.264 rectangle too large"));
                            }
                            raw.resize(len, 0);
                            r.read_exact(&mut raw)?;
                            s.video_seen.store(true, SeqCst);
                            let t = Instant::now();
                            video.rect(s, &raw, flags, x, y, w, h);
                            // (A reset makes a new decoder: not what each picture costs.)
                            had_video |= flags & crate::enc::H264_RESET == 0;
                            if flags & crate::enc::H264_RESET == 0 {
                                s.video_took(t.elapsed());
                            }
                        }
                        ENC_CURSOR => {
                            let pix = rd_vec(&mut r, w * h * 4)?;
                            let mask = rd_vec(&mut r, (w + 7) / 8 * h)?;
                            *s.cursor.lock().unwrap() = Some(make_cursor(w, h, x, y, &pix, &mask));
                            s.post(WM_CURSOR);
                        }
                        ENC_DESKTOP_SIZE => {
                            crate::rfb::check_size(w, h)?;
                            resize(s, w, h)?;
                            resized = true;
                        }
                        ENC_EXT_DESKTOP_SIZE => {
                            let n = rd_u8(&mut r)? as usize;
                            rd_vec(&mut r, 3)?;
                            let mut screens = Vec::with_capacity(n);
                            for _ in 0..n {
                                let b = rd_vec(&mut r, 16)?;
                                let u16_at = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
                                screens.push(Screen {
                                    id: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                                    x: u16_at(4),
                                    y: u16_at(6),
                                    w: u16_at(8),
                                    h: u16_at(10),
                                    flags: u32::from_be_bytes([b[12], b[13], b[14], b[15]]),
                                });
                            }
                            // x = reason (1 = our request), y = status (0 = ok)
                            crate::log!("ExtendedDesktopSize reason={x} status={y} {w}x{h} screens={screens:?}");
                            *s.screens.lock().unwrap() = screens;
                            if !(x == 1 && y != 0) && s.size() != (w as u32, h as u32) {
                                crate::rfb::check_size(w, h)?;
                                resize(s, w, h)?;
                                resized = true;
                            }
                            s.post(WM_RESIZED);
                        }
                        e => return err(tr!("net.encoding", encoding = e)),
                    }
                }
                if resized {
                    s.request(false);
                }
                // Only pictures decoded hold the next request back (BUSY): a still repaint,
                // a cursor or a new decoder doesn't.
                if !had_video {
                    s.video_took(Duration::ZERO);
                }
                if updates == 100 || updates % 1000 == 0 {
                    crate::log!("{updates} updates received");
                }
                if !s.pending.swap(true, SeqCst) {
                    s.post(WM_FB);
                }
            }
            1 => {
                // SetColourMapEntries — irrelevant for true colour, skip.
                rd_vec(&mut r, 3)?;
                let n = rd_u16(&mut r)? as usize;
                rd_vec(&mut r, n * 6)?;
            }
            2 => {} // Bell
            3 => {
                rd_vec(&mut r, 3)?;
                let n = rd_u32(&mut r)? as i32;
                let data = rd_vec(&mut r, n.unsigned_abs().min(64 << 20) as usize)?;
                if n < 0 {
                    crate::clip::on_extended(s, &data);
                } else {
                    crate::clip::on_classic(s, data);
                }
            }
            t => return err(tr!("net.message", kind = t)),
        }
    }
}

/// The H.264 decoder of one connection. A decoding failure doesn't end the session: on the GPU,
/// the decoder is replaced by a software one (once; later decoders of this process stay in
/// software); in software, the decoder is dropped and the server asked for ZRLE again (which
/// it answers by repainting the whole screen losslessly); rectangles still in flight until
/// then are skipped.
#[derive(Default)]
struct Video {
    dec: Option<crate::h264dec_win::Decoder>,
    failed: bool,
    /// The GPU decoder failed in this connection: software from now on.
    soft: bool,
    /// Rectangles skipped since a new decoder took over mid-stream, waiting for a key frame
    /// (0: not waiting).
    waiting: u32,
    /// Smaller pictures scaled up to their rectangle (ENC_ITDA_SCALED_VIDEO).
    scaler: crate::yuv::Scaler,
    /// The last picture size drawn into which rectangle size (logged when it changes).
    drawn: ((usize, usize), (usize, usize)),
}

/// Key frames a new decoder waits for, asked again every so many rectangles, before it gives up.
const IDR_ASK_EVERY: u32 = 60;
const IDR_GIVE_UP: u32 = 600;

impl Video {
    fn rect(&mut self, s: &Shared, data: &[u8], flags: u32, x: usize, y: usize, w: usize, h: usize) {
        if self.failed {
            return;
        }
        let key = crate::yuv::has_nal(data, 5);
        if self.waiting > 0 {
            if !key {
                self.waiting += 1;
                if self.waiting % IDR_ASK_EVERY == 0 {
                    s.request(false);
                }
                if self.waiting < IDR_GIVE_UP {
                    return;
                }
                return self.give_up(s, &format!("no key frame in {IDR_GIVE_UP} rectangles"), data, flags, [x, y, w, h]);
            }
            crate::log!("h264: key frame after {} skipped rectangles", self.waiting - 1);
            self.waiting = 0;
        }
        let reset = flags & crate::enc::H264_RESET != 0;
        if reset && self.dec.as_ref().is_some_and(|d| d.made != (w, h)) {
            self.dec = None; // another rectangle: a fresh decoder rather than a flushed one
        }
        let gpu = !self.soft && crate::h264dec_win::gpu_wanted();
        let (scaler, drawn) = (&mut self.scaler, &mut self.drawn);
        let view = s.view_scale.lock().unwrap().0 as usize;
        let r = match &mut self.dec {
            Some(d) => Ok(d),
            None => crate::h264dec_win::Decoder::new(w, h, gpu).map(|d| {
                crate::log!("h264: decoding {w}x{h} at {x},{y}{}", if d.gpu() { " on the GPU" } else { "" });
                self.dec.insert(d)
            }),
        }
        .and_then(|d| {
            d.decode(data, reset, |pic, yuv| {
                if *drawn != ((pic.w, pic.h), (w, h)) {
                    *drawn = ((pic.w, pic.h), (w, h));
                    if pic.w < w || pic.h < h {
                        crate::log!("h264: picture {}x{} scaled to {w}x{h}", pic.w, pic.h);
                    } else {
                        crate::log!("h264: picture {}x{} in {w}x{h}", pic.w, pic.h);
                    }
                }
                // Smooth only where the windows show more pixels than the picture has (Scaler).
                scaler.set_smooth(pic.w * 1000 < w * view * 97 / 100);
                let mut fb = s.fb.lock().unwrap();
                match fb.video([x, y, w, h], pic, yuv, scaler) {
                    Some((dw, dh)) if dw > 0 && dh > 0 => fb.mark(x, y, dw, dh),
                    Some(_) => {}
                    None => crate::log!("h264: picture {}x{} stride {} doesn't fit its planes", pic.w, pic.h, pic.stride),
                }
            })
        });
        let Err(e) = r else { return };
        if self.dec.as_ref().is_some_and(|d| d.gpu()) {
            crate::log!("h264: GPU decoding failed, using the software decoder: {e} (rect {x},{y} {w}x{h}, {} bytes, flags {flags})", data.len());
            crate::h264dec_win::gpu_failed();
            self.soft = true;
            self.dec = None;
            if key {
                // This rectangle starts a picture of its own: the software decoder starts there.
                return self.rect(s, data, flags | crate::enc::H264_RESET, x, y, w, h);
            }
            // Mid-stream: the next pictures need the ones before; ask for a key frame.
            self.waiting = 1;
            s.request(false);
            return;
        }
        self.give_up(s, &e, data, flags, [x, y, w, h]);
    }

    /// Back to ZRLE for this connection, and no more H.264 in this process.
    fn give_up(&mut self, s: &Shared, e: &str, data: &[u8], flags: u32, [x, y, w, h]: [usize; 4]) {
        crate::log!("h264: DECODING FAILED, back to ZRLE: {e} (rect {x},{y} {w}x{h}, {} bytes, flags {flags})", data.len());
        self.dec = None;
        self.failed = true;
        self.waiting = 0;
        crate::h264dec_win::disable();
        s.video_took(Duration::ZERO);
        s.send(&set_encodings(false, 1000));
        s.request(false);
    }
}

fn resize(s: &Shared, w: usize, h: usize) -> io::Result<()> {
    if !s.fb.lock().unwrap().resize(w, h) {
        return Err(io::Error::other(tr!("net.noResizeMemory")));
    }
    s.fb_w.store(w as u32, Relaxed);
    s.fb_h.store(h as u32, Relaxed);
    s.post(WM_RESIZED);
    Ok(())
}

/// Inflate one ZRLE rect's worth of data (the zlib stream persists across rects).
fn inflate(z: &mut Decompress, input: &[u8], out: &mut Vec<u8>, most: usize) -> io::Result<()> {
    out.clear();
    let mut used = 0;
    loop {
        if out.capacity() - out.len() < 1 << 16 {
            out.reserve(out.capacity().max(1 << 18));
        }
        let (i0, o0) = (z.total_in(), out.len());
        z.decompress_vec(&input[used..], out, FlushDecompress::Sync).map_err(io::Error::other)?;
        if out.len() > most {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ZRLE data inflates too far"));
        }
        used += (z.total_in() - i0) as usize;
        let full = out.len() == out.capacity();
        if (used >= input.len() && !full) || (z.total_in() == i0 && out.len() == o0) {
            return Ok(());
        }
    }
}

fn make_cursor(w: usize, h: usize, hx: usize, hy: usize, pix: &[u8], mask: &[u8]) -> Cursor {
    let mb = (w + 7) / 8;
    let mut px = vec![0u32; w * h];
    for y in 0..h {
        for x in 0..w {
            if mask[y * mb + x / 8] & (0x80 >> (x % 8)) != 0 {
                let p = &pix[(y * w + x) * 4..];
                px[y * w + x] = 0xff00_0000 | u32::from_le_bytes([p[0], p[1], p[2], 0]);
            }
        }
    }
    Cursor { w, h, hx, hy, px }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// What the viewer asked the server for since the last call (10-byte update requests).
    fn requests(peer: &mut TcpStream) -> usize {
        peer.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
        let mut n = 0;
        let mut buf = [0u8; 10];
        while peer.read_exact(&mut buf).is_ok() {
            assert_eq!(buf[0], 3, "FramebufferUpdateRequest");
            assert_eq!(buf[1], 1, "incremental");
            n += 1;
        }
        n
    }

    #[test]
    fn nobody_looking_means_no_new_frames() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let out = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (mut peer, _) = l.accept().unwrap();
        init(out, 64, 32).unwrap();
        let s = sh();
        // In front: each update asks for the next at once.
        s.request_next();
        assert_eq!(requests(&mut peer), 1);
        // Minimized: updates keep nothing coming, however long it waits.
        s.set_view(VIEW_PAUSED);
        s.request_next();
        s.pace();
        assert_eq!(requests(&mut peer), 0);
        // Restored: the held-back request goes out once.
        s.set_view(VIEW_LIVE);
        assert_eq!(requests(&mut peer), 1);
        s.pace();
        assert_eq!(requests(&mut peer), 0);
        // Behind other windows: at most one request per BACKGROUND_EVERY.
        s.set_view(VIEW_BACKGROUND);
        s.request_next();
        s.pace();
        assert_eq!(requests(&mut peer), 0, "too soon after the last one");
        std::thread::sleep(BACKGROUND_EVERY);
        s.pace();
        assert_eq!(requests(&mut peer), 1);
        // The liveness probe: whenever silence lasts, except while minimized, where it waits
        // PAUSED_PROBE since the last request.
        assert!(s.probe_due(), "behind other windows");
        s.set_view(VIEW_PAUSED);
        assert!(!s.probe_due(), "just asked");
        *s.last_request.lock().unwrap() = Instant::now() - PAUSED_PROBE;
        assert!(s.probe_due(), "silent for PAUSED_PROBE");
        s.probe();
        let mut probe = [0u8; 10];
        peer.read_exact(&mut probe).unwrap();
        assert_eq!(probe, [3, 0, 0, 0, 0, 0, 0, 1, 0, 1], "a non-incremental 1x1 request");
        assert_eq!(requests(&mut peer), 0);
        assert!(!s.probe_due(), "the probe counts as asking");
        s.set_view(VIEW_LIVE);
        assert!(s.probe_due());

        // A slow PC: the next picture is asked for half again as long after the last one took.
        s.video_took(Duration::from_millis(40));
        s.request_next();
        s.pace();
        assert_eq!(requests(&mut peer), 0, "held back");
        std::thread::sleep(Duration::from_millis(60));
        s.pace();
        assert_eq!(requests(&mut peer), 1);
        s.video_took(Duration::from_millis(3));
        s.request_next();
        assert_eq!(requests(&mut peer), 1, "a fast one asks at once");

        // The view scale goes out once it has held, and only after the server sent H.264.
        if !crate::h264dec_win::available() {
            return eprintln!("skipped the view scale: no H.264 decoder here");
        }
        s.set_view_scale(1280.0 / 3840.0);
        std::thread::sleep(VIEW_SCALE_HOLD);
        s.tell_view_scale();
        peer.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
        assert!(peer.read(&mut [0u8; 1]).is_err(), "nothing before H.264");
        s.video_seen.store(true, SeqCst);
        s.set_view_scale(0.5);
        s.tell_view_scale();
        assert!(peer.read(&mut [0u8; 1]).is_err(), "not before it held");
        std::thread::sleep(VIEW_SCALE_HOLD);
        s.tell_view_scale();
        let want = set_encodings(true, 500);
        let mut got = vec![0u8; want.len()];
        peer.read_exact(&mut got).unwrap();
        assert_eq!(got, want);
        assert_eq!(crate::enc::view_scale_of(&got[4..].chunks(4).map(|c| i32::from_be_bytes(c.try_into().unwrap())).collect::<Vec<_>>()), 500);
        s.tell_view_scale();
        assert!(peer.read(&mut [0u8; 1]).is_err(), "told once");
    }
}
