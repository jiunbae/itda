//! Shared state between the UI thread and the network thread, and the receive loop.

use crate::fb::Fb;
use crate::rfb::*;
use flate2::{Decompress, FlushDecompress};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::windows::io::AsRawSocket;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering::*};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use windows_sys::Win32::Networking::WinSock::{WSAIoctl, TCP_INFO_v0};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub const WM_FB: u32 = WM_APP + 1;
pub const WM_CURSOR: u32 = WM_APP + 2;
pub const WM_RESIZED: u32 = WM_APP + 3;
pub const WM_NETERR: u32 = WM_APP + 4;
pub const WM_STATUS: u32 = WM_APP + 5;
pub const WM_CLIP: u32 = WM_APP + 6;

const SIO_TCP_INFO: u32 = 0xd800_0027;
/// Silence longer than this at a message boundary triggers a liveness probe.
const IDLE_PROBE: Duration = Duration::from_secs(4);

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
}

static SHARED: OnceLock<Shared> = OnceLock::new();

pub fn sh() -> &'static Shared {
    SHARED.get().unwrap()
}

pub fn init(out: TcpStream, w: u16, h: u16) {
    let s = Shared {
        fb: Mutex::new(Fb::new(w as usize, h as usize)),
        screens: Mutex::new(Vec::new()),
        ext_clip: AtomicBool::new(false),
        clip_out: Mutex::new(String::new()),
        clip_in: Mutex::new(None),
        cursor: Mutex::new(None),
        error: Mutex::new(None),
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
    };
    let _ = SHARED.set(s);
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
        self.send(&update_request(incremental, 0, 0, w as u16, h as u16));
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
    std::thread::spawn(move || loop {
        let _ = reader.get_ref().0.set_read_timeout(Some(IDLE_PROBE));
        let e = recv_loop(reader).unwrap_err();
        crate::log!("disconnected: {e}");
        let s = sh();
        if !s.auto_reconnect.load(Relaxed) {
            *s.error.lock().unwrap() = Some(format!("연결이 끊어졌습니다 ({e})"));
            s.post(WM_NETERR);
            return;
        }
        let mut attempt = 0u32;
        reader = loop {
            attempt += 1;
            s.attempt.store(attempt, Relaxed);
            crate::session::emit(serde_json::json!({"ev": "reconnecting", "n": attempt}));
            s.set_status(format!("다시 연결하는 중… ({attempt}번째 시도)"));
            let ms = if attempt == 1 { 300 } else { (500u64 << attempt.min(5)).min(8000) };
            std::thread::sleep(Duration::from_millis(ms));
            match dial().and_then(|addr| connect(&addr, &creds)) {
                Ok(sess) => {
                    s.sock.store(sess.writer.as_raw_socket(), Relaxed);
                    s.ext_clip.store(false, Relaxed);
                    s.screens.lock().unwrap().clear();
                    *s.out.lock().unwrap() = sess.writer;
                    let (w, h) = (sess.width as u32, sess.height as u32);
                    if s.size() != (w, h) {
                        s.fb.lock().unwrap().resize(w as usize, h as usize);
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
                    return err("Mac 응답 없음");
                }
                s.send(&update_request(false, 0, 0, 1, 1));
                probed = true;
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
                // Ask for the next frame now so the server encodes it while we decode this one.
                s.request(true);
                let mut resized = false;
                for _ in 0..n {
                    let x = rd_u16(&mut r)? as usize;
                    let y = rd_u16(&mut r)? as usize;
                    let w = rd_u16(&mut r)? as usize;
                    let h = rd_u16(&mut r)? as usize;
                    let enc = rd_u32(&mut r)? as i32;
                    if verbose {
                        crate::log!("update #{updates}: rect {x},{y} {w}x{h} enc={enc}");
                    }
                    match enc {
                        ENC_RAW => {
                            raw.resize(w * h * 4, 0);
                            r.read_exact(&mut raw)?;
                            let mut fb = s.fb.lock().unwrap();
                            if fb.fits(x, y, w, h) {
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
                            let len = rd_u32(&mut r)? as usize;
                            zin.resize(len, 0);
                            r.read_exact(&mut zin)?;
                            inflate(&mut z, &zin, &mut zout)?;
                            let mut fb = s.fb.lock().unwrap();
                            if fb.fits(x, y, w, h) {
                                fb.zrle(&zout, x, y, w, h).ok_or_else(|| io::Error::other("bad ZRLE data"))?;
                                fb.mark(x, y, w, h);
                            }
                        }
                        ENC_CURSOR => {
                            let pix = rd_vec(&mut r, w * h * 4)?;
                            let mask = rd_vec(&mut r, (w + 7) / 8 * h)?;
                            *s.cursor.lock().unwrap() = Some(make_cursor(w, h, x, y, &pix, &mask));
                            s.post(WM_CURSOR);
                        }
                        ENC_DESKTOP_SIZE => {
                            resize(s, w, h);
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
                                resize(s, w, h);
                                resized = true;
                            }
                            s.post(WM_RESIZED);
                        }
                        e => return err(format!("지원하지 않는 인코딩 {e}")),
                    }
                }
                if resized {
                    s.request(false);
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
            t => return err(format!("알 수 없는 서버 메시지 {t}")),
        }
    }
}

fn resize(s: &Shared, w: usize, h: usize) {
    s.fb.lock().unwrap().resize(w, h);
    s.fb_w.store(w as u32, Relaxed);
    s.fb_h.store(h as u32, Relaxed);
    s.post(WM_RESIZED);
}

/// Inflate one ZRLE rect's worth of data (the zlib stream persists across rects).
fn inflate(z: &mut Decompress, input: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    out.clear();
    let mut used = 0;
    loop {
        if out.capacity() - out.len() < 1 << 16 {
            out.reserve(out.capacity().max(1 << 18));
        }
        let (i0, o0) = (z.total_in(), out.len());
        z.decompress_vec(&input[used..], out, FlushDecompress::Sync).map_err(io::Error::other)?;
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
