//! "Share this PC": a small VNC server so other devices (Itda on another PC, the web
//! viewer, macOS Screen Sharing, any VNC client) can see and control this Windows PC.
//!
//! GDI capture of the whole virtual desktop (all monitors), 64×64 tile diffing, ZRLE
//! (solid/raw tiles over one zlib stream), SendInput for mouse and keyboard, clipboard both
//! ways, VNC-password auth plus an on-screen "allow?" prompt, and DNS-SD (Bonjour)
//! registration so nearby Itdas list this PC.

use crate::enc::{encode_rect, PixelFormat, ENC_ZRLE, TILE};
use crate::rfb::{vnc_response, PIXEL_FORMAT};
use flate2::{Compress, Compression};
use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub password: String,
    /// Ask the person at this PC before each viewer gets in.
    pub ask: bool,
    /// A Mac's ⌘ (Super) acts as Ctrl here, so ⌘C/⌘V copy and paste.
    pub cmd_as_ctrl: bool,
    pub view_only: bool,
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
    let l = TcpListener::bind(("0.0.0.0", cfg.port)).map_err(|e| io::Error::new(e.kind(), format!("포트 {}을(를) 열 수 없습니다: {e}", cfg.port)))?;
    l.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let cfg = Arc::new(cfg);
    let port = cfg.port;
    let c2 = cfg.clone();
    let thread = std::thread::spawn(move || {
        while !flag.load(Relaxed) {
            match l.accept() {
                Ok((s, peer)) => {
                    let _ = s.set_nonblocking(false);
                    let (cfg, flag) = (c2.clone(), flag.clone());
                    std::thread::spawn(move || {
                        if let Err(e) = serve(s, peer, &cfg, &flag) {
                            crate::log!("share {peer}: {e}");
                        }
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(150)),
                Err(e) => {
                    crate::log!("share accept: {e}");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });
    let mdns = crate::mdns::register("_rfb._tcp", port, &[("itda", "1"), ("os", "windows")]);
    crate::log!("sharing this PC on port {port} (bonjour {})", mdns.is_some());
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
        let _ = r.thread.join();
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
}

fn rd<const N: usize>(r: &mut impl Read) -> io::Result<[u8; N]> {
    let mut b = [0u8; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn ask_user(peer: &SocketAddr) -> bool {
    let text = format!(
        "{}에서 이 PC의 화면을 보고 조작하려고 합니다.\n\n허용할까요?\n\n(허용하면 연결이 끝날 때까지 화면과 입력이 공유됩니다. 알림 영역의 Itda에서 언제든 끊을 수 있습니다.)",
        peer.ip()
    );
    let t = crate::ui::wide(&text);
    let title = crate::ui::wide("Itda — 화면 공유 요청");
    unsafe { MessageBoxW(null_mut(), t.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND | MB_SYSTEMMODAL) == IDYES }
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
    if cfg.ask && !ask_user(&peer) {
        crate::log!("share {peer}: refused by the user");
        return deny(&mut c, "The person at this PC declined");
    }
    c.write_all(&0u32.to_be_bytes())?;
    let _shared: [u8; 1] = rd(&mut c)?;
    c.set_read_timeout(None)?;

    let mut cap = Capture::new();
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into());
    let mut init = Vec::new();
    init.extend_from_slice(&(cap.w as u16).to_be_bytes());
    init.extend_from_slice(&(cap.h as u16).to_be_bytes());
    init.extend_from_slice(&PIXEL_FORMAT);
    init.extend_from_slice(&(name.len() as u32).to_be_bytes());
    init.extend_from_slice(name.as_bytes());
    c.write_all(&init)?;
    crate::log!("share {peer}: viewer in ({}x{})", cap.w, cap.h);

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
        let mut input = Input::new(cfg2.cmd_as_ctrl, cfg2.view_only);
        let _ = read_client(BufReader::new(reader), &tx, &mut input);
        input.release_all();
    });
    let r = pump(&mut c, &rx, &mut cap, stop);
    let _ = c.shutdown(Shutdown::Both);
    VIEWERS.lock().unwrap().retain(|(i, _, _)| *i != id);
    changed();
    crate::log!("share {peer}: viewer left");
    r
}

/// Client → server messages. Input is applied right here; the rest goes to the pump.
fn read_client(mut r: BufReader<TcpStream>, tx: &Sender<Msg>, input: &mut Input) -> io::Result<()> {
    loop {
        // A held Ctrl/Alt/Win must not outlive the person holding it: a phone that goes to
        // sleep mid-gesture keeps the socket open but silent, and the PC would keep zooming
        // every app on Ctrl+wheel. Release modifiers after a quiet spell or a long hold.
        if input.modifiers_held() && input.last_key.elapsed() > MOD_HOLD_MAX {
            input.release_modifiers("held too long");
        }
        let wait = r.buffer().is_empty() && input.modifiers_held();
        r.get_ref().set_read_timeout(wait.then_some(MOD_QUIET))?;
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
                    clipboard_set(&text);
                }
            }
            251 => {
                // SetDesktopSize: we can't resize the PC's screens; skip the payload.
                let b: [u8; 7] = rd(&mut r)?;
                let mut d = vec![0u8; b[5] as usize * 16];
                r.read_exact(&mut d)?;
            }
            other => return Err(io::Error::other(format!("unknown client message {other}"))),
        }
    }
}

const ENC_DESKTOP_SIZE: i32 = -223;

/// Server → client: capture, diff, encode, send. ~30 fps at most.
fn pump(c: &mut TcpStream, rx: &Receiver<Msg>, cap: &mut Capture, stop: &AtomicBool) -> io::Result<()> {
    let mut pf = PixelFormat::parse(&PIXEL_FORMAT);
    let mut encs: Vec<i32> = Vec::new();
    let mut want: Option<bool> = None; // pending request: Some(incremental)
    let mut prev: Vec<u32> = Vec::new();
    let mut z = Compress::new(Compression::new(5), true);
    let mut clip_seq = unsafe { GetClipboardSequenceNumber() };
    let mut clip_check = Instant::now();
    loop {
        if stop.load(Relaxed) {
            return Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(33)) {
            Ok(m) => {
                match m {
                    Msg::Format(p) => {
                        pf = p;
                        prev.clear();
                    }
                    Msg::Encodings(e) => encs = e,
                    Msg::Request { incremental } => want = Some(want.unwrap_or(true) && incremental),
                }
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Msg::Format(p) => {
                            pf = p;
                            prev.clear();
                        }
                        Msg::Encodings(e) => encs = e,
                        Msg::Request { incremental } => want = Some(want.unwrap_or(true) && incremental),
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        // Local clipboard changed → offer it to the viewer.
        if clip_check.elapsed() > Duration::from_millis(500) {
            clip_check = Instant::now();
            let seq = unsafe { GetClipboardSequenceNumber() };
            if seq != clip_seq {
                clip_seq = seq;
                if let Some(t) = clipboard_get() {
                    let b = t.replace("\r\n", "\n").into_bytes();
                    let mut m = vec![3, 0, 0, 0];
                    m.extend_from_slice(&(b.len() as u32).to_be_bytes());
                    m.extend_from_slice(&b);
                    c.write_all(&m)?;
                }
            }
        }
        let Some(incremental) = want else { continue };
        let (w0, h0) = (cap.w, cap.h);
        let (w, h, frame) = cap.grab();
        let mut out = Vec::new();
        let mut rects = 0u16;
        if (w, h) != (w0, h0) && encs.contains(&ENC_DESKTOP_SIZE) {
            out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&(w as u16).to_be_bytes());
            out.extend_from_slice(&(h as u16).to_be_bytes());
            out.extend_from_slice(&ENC_DESKTOP_SIZE.to_be_bytes());
            rects += 1;
            prev.clear();
        }
        let full = !incremental || prev.len() != frame.len();
        let zrle = encs.contains(&ENC_ZRLE);
        // Changed tiles, merged into horizontal runs per tile row.
        for ty in (0..h).step_by(TILE) {
            let th = TILE.min(h - ty);
            let mut tx = 0;
            while tx < w {
                let dirty = |x: usize| {
                    full || (ty..ty + th).any(|y| {
                        let a = y * w + x;
                        let tw = TILE.min(w - x);
                        frame[a..a + tw] != prev[a..a + tw]
                    })
                };
                if !dirty(tx) {
                    tx += TILE;
                    continue;
                }
                let start = tx;
                while tx < w && dirty(tx) {
                    tx += TILE;
                }
                let rw = tx.min(w) - start;
                encode_rect(&mut out, &frame, w, start, ty, rw, th, &pf, zrle, &mut z)?;
                rects += 1;
            }
        }
        if rects == 0 {
            continue; // nothing changed: keep the request pending
        }
        let mut msg = vec![0u8, 0];
        msg.extend_from_slice(&rects.to_be_bytes());
        msg.extend_from_slice(&out);
        c.write_all(&msg)?;
        prev = frame.to_vec();
        want = None;
    }
}

// ---------------------------------------------------------------- capture

struct Capture {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    mdc: HDC,
    bmp: HBITMAP,
    bits: *mut u32,
}

impl Capture {
    fn new() -> Capture {
        let mut c = Capture { x: 0, y: 0, w: 0, h: 0, mdc: null_mut(), bmp: null_mut(), bits: null_mut() };
        c.fit();
        c
    }

    /// (Re)size to the virtual desktop (all monitors).
    fn fit(&mut self) {
        let (x, y, w, h) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as usize,
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as usize,
            )
        };
        if (x, y, w, h) == (self.x, self.y, self.w, self.h) && !self.bmp.is_null() {
            return;
        }
        self.free();
        unsafe {
            let screen = GetDC(null_mut());
            self.mdc = CreateCompatibleDC(screen);
            let bi = crate::fb::dib_info(w as i32, h as i32);
            let mut bits = null_mut();
            self.bmp = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            self.bits = bits as *mut u32;
            SelectObject(self.mdc, self.bmp);
            ReleaseDC(null_mut(), screen);
        }
        (self.x, self.y, self.w, self.h) = (x, y, w, h);
    }

    fn grab(&mut self) -> (usize, usize, &[u32]) {
        self.fit();
        unsafe {
            let screen = GetDC(null_mut());
            BitBlt(self.mdc, 0, 0, self.w as i32, self.h as i32, screen, self.x, self.y, SRCCOPY | CAPTUREBLT);
            ReleaseDC(null_mut(), screen);
            // GDI capture leaves the cursor out: draw it in.
            let mut ci: CURSORINFO = std::mem::zeroed();
            ci.cbSize = size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut ci) != 0 && ci.flags & CURSOR_SHOWING != 0 {
                let mut ii: ICONINFO = std::mem::zeroed();
                if GetIconInfo(ci.hCursor, &mut ii) != 0 {
                    DrawIconEx(self.mdc, ci.ptScreenPos.x - self.x - ii.xHotspot as i32, ci.ptScreenPos.y - self.y - ii.yHotspot as i32, ci.hCursor, 0, 0, 0, null_mut(), DI_NORMAL);
                    if !ii.hbmMask.is_null() {
                        DeleteObject(ii.hbmMask);
                    }
                    if !ii.hbmColor.is_null() {
                        DeleteObject(ii.hbmColor);
                    }
                }
            }
            GdiFlush();
            let px = std::slice::from_raw_parts_mut(self.bits, self.w * self.h);
            for p in px.iter_mut() {
                *p &= 0x00ff_ffff;
            }
            (self.w, self.h, px)
        }
    }

    fn free(&mut self) {
        unsafe {
            if !self.mdc.is_null() {
                DeleteDC(self.mdc);
            }
            if !self.bmp.is_null() {
                DeleteObject(self.bmp);
            }
        }
        self.mdc = null_mut();
        self.bmp = null_mut();
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.free();
    }
}

// ---------------------------------------------------------------- input

/// Longest a modifier may stay down without any other key, and how long a silent viewer
/// may hold one (see `read_client`).
const MOD_HOLD_MAX: Duration = Duration::from_secs(15);
const MOD_QUIET: Duration = Duration::from_secs(4);

struct Input {
    cmd_as_ctrl: bool,
    view_only: bool,
    buttons: u8,
    /// keysym → what we pressed for it, so the release matches.
    down: HashMap<u32, Pressed>,
    /// Last key event, for releasing forgotten modifiers.
    last_key: Instant,
}

#[derive(Clone, Copy)]
enum Pressed {
    Vk(u16, bool),
    Unicode(u16),
}

fn send(inputs: &[INPUT]) {
    let n = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if n == 0 {
        // Blocked: no input desktop (locked/secure screen) or an elevated window has focus.
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Relaxed) {
            crate::log!("share: input blocked by Windows (error {}); is the screen locked or an admin window focused?", unsafe { GetLastError() });
        }
    }
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    let mut i: INPUT = unsafe { std::mem::zeroed() };
    i.r#type = INPUT_KEYBOARD;
    i.Anonymous.ki = KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    i
}

impl Input {
    fn new(cmd_as_ctrl: bool, view_only: bool) -> Input {
        Input { cmd_as_ctrl, view_only, buttons: 0, down: HashMap::new(), last_key: Instant::now() }
    }

    fn is_modifier(p: &Pressed) -> bool {
        matches!(p, Pressed::Vk(vk, _) if [VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT].contains(vk))
    }

    fn release_modifiers(&mut self, why: &str) {
        let held: Vec<u32> = self.down.iter().filter(|(_, p)| Self::is_modifier(p)).map(|(s, _)| *s).collect();
        for s in held {
            if let Some(p) = self.down.remove(&s) {
                Self::release(p);
            }
        }
        crate::log!("share: released modifier keys ({why})");
    }

    fn special(&self, sym: u32) -> Option<(u16, bool)> {
        // (virtual key, extended)
        Some(match sym {
            0xff08 => (VK_BACK, false),
            0xff09 => (VK_TAB, false),
            0xff0d => (VK_RETURN, false),
            0xff8d => (VK_RETURN, true),
            0xff13 => (VK_PAUSE, false),
            0xff14 => (VK_SCROLL, false),
            0xff1b => (VK_ESCAPE, false),
            0xffff => (VK_DELETE, true),
            0xff50 => (VK_HOME, true),
            0xff51 => (VK_LEFT, true),
            0xff52 => (VK_UP, true),
            0xff53 => (VK_RIGHT, true),
            0xff54 => (VK_DOWN, true),
            0xff55 => (VK_PRIOR, true),
            0xff56 => (VK_NEXT, true),
            0xff57 => (VK_END, true),
            0xff61 => (VK_SNAPSHOT, false),
            0xff63 => (VK_INSERT, true),
            0xff67 => (VK_APPS, true),
            0xff7f => (VK_NUMLOCK, false),
            0xffaa => (VK_MULTIPLY, false),
            0xffab => (VK_ADD, false),
            0xffad => (VK_SUBTRACT, false),
            0xffae => (VK_DECIMAL, false),
            0xffaf => (VK_DIVIDE, true),
            0xffb0..=0xffb9 => (VK_NUMPAD0 + (sym - 0xffb0) as u16, false),
            0xffbe..=0xffc9 => (VK_F1 + (sym - 0xffbe) as u16, false),
            0xffe1 => (VK_LSHIFT, false),
            0xffe2 => (VK_RSHIFT, false),
            0xffe3 => (VK_LCONTROL, false),
            0xffe4 => (VK_RCONTROL, true),
            0xffe5 => (VK_CAPITAL, false),
            0xffe7 | 0xffe9 => (VK_LMENU, false),
            0xffe8 | 0xffea => (VK_RMENU, true),
            0xffeb | 0xffec if self.cmd_as_ctrl => (VK_LCONTROL, false),
            0xffeb => (VK_LWIN, true),
            0xffec => (VK_RWIN, true),
            _ => return None,
        })
    }

    fn modifiers_held(&self) -> bool {
        self.down.values().any(|p| matches!(p, Pressed::Vk(vk, _) if [VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN].contains(vk)))
    }

    fn key(&mut self, down: bool, sym: u32) {
        if self.view_only {
            return;
        }
        self.last_key = Instant::now();
        if !down {
            if let Some(p) = self.down.remove(&sym) {
                Self::release(p);
            }
            return;
        }
        let pressed = if let Some((vk, ext)) = self.special(sym) {
            Pressed::Vk(vk, ext)
        } else {
            let cp = if sym >= 0x0100_0000 { sym & 0x00ff_ffff } else { sym };
            let Some(ch) = char::from_u32(cp) else { return };
            if self.modifiers_held() && ch.is_ascii_alphanumeric() {
                // Shortcuts (Ctrl+C, Alt+F…) need real keys, not Unicode characters.
                Pressed::Vk(ch.to_ascii_uppercase() as u16, false)
            } else if self.modifiers_held() {
                let vk = unsafe { VkKeyScanW(ch as u16) };
                if vk == -1 {
                    return;
                }
                Pressed::Vk((vk & 0xff) as u16, false)
            } else {
                let mut b = [0u16; 2];
                let s = ch.encode_utf16(&mut b);
                if s.len() != 1 {
                    // Outside the BMP: send the surrogate pair as one keystroke.
                    send(&[key_input(0, s[0], KEYEVENTF_UNICODE), key_input(0, s[1], KEYEVENTF_UNICODE)]);
                    return;
                }
                Pressed::Unicode(s[0])
            }
        };
        match pressed {
            Pressed::Vk(vk, ext) => send(&[key_input(vk, 0, if ext { KEYEVENTF_EXTENDEDKEY } else { 0 })]),
            Pressed::Unicode(u) => send(&[key_input(0, u, KEYEVENTF_UNICODE)]),
        }
        self.down.insert(sym, pressed);
    }

    fn release(p: Pressed) {
        match p {
            Pressed::Vk(vk, ext) => send(&[key_input(vk, 0, KEYEVENTF_KEYUP | if ext { KEYEVENTF_EXTENDEDKEY } else { 0 })]),
            Pressed::Unicode(u) => send(&[key_input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]),
        }
    }

    fn pointer(&mut self, mask: u8, x: u16, y: u16) {
        if self.view_only {
            return;
        }
        let (vw, vh) = unsafe { (GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2) as i64, GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2) as i64) };
        // Windows maps back with (n * size) >> 16; rounding up lands on exactly this pixel.
        let norm = |v: u16, size: i64| ((v as i64 * 65536 + size - 1) / size).min(65535) as i32;
        let mut i: INPUT = unsafe { std::mem::zeroed() };
        i.r#type = INPUT_MOUSE;
        let mut flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        let mut data: i32 = 0;
        let changed = mask ^ self.buttons;
        let press = |bit: u8| changed & bit != 0 && mask & bit != 0;
        let lift = |bit: u8| changed & bit != 0 && mask & bit == 0;
        if press(1) { flags |= MOUSEEVENTF_LEFTDOWN; }
        if lift(1) { flags |= MOUSEEVENTF_LEFTUP; }
        if press(2) { flags |= MOUSEEVENTF_MIDDLEDOWN; }
        if lift(2) { flags |= MOUSEEVENTF_MIDDLEUP; }
        if press(4) { flags |= MOUSEEVENTF_RIGHTDOWN; }
        if lift(4) { flags |= MOUSEEVENTF_RIGHTUP; }
        if press(8) { flags |= MOUSEEVENTF_WHEEL; data = 120; }
        if press(16) { flags |= MOUSEEVENTF_WHEEL; data = -120; }
        if press(32) { flags |= MOUSEEVENTF_HWHEEL; data = -120; }
        if press(64) { flags |= MOUSEEVENTF_HWHEEL; data = 120; }
        self.buttons = mask;
        i.Anonymous.mi = MOUSEINPUT {
            dx: norm(x, vw),
            dy: norm(y, vh),
            mouseData: data as u32,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        send(&[i]);
    }

    fn release_all(&mut self) {
        for (_, p) in self.down.drain() {
            Self::release(p);
        }
        if self.buttons & 7 != 0 {
            self.pointer(0, 0, 0);
        }
    }
}

// ---------------------------------------------------------------- clipboard

const CF_UNICODETEXT: u32 = 13;

fn clipboard_get() -> Option<String> {
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
            return None;
        }
        let mut out = None;
        let mem = GetClipboardData(CF_UNICODETEXT);
        if !mem.is_null() {
            let p = GlobalLock(mem as _) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                GlobalUnlock(mem as _);
            }
        }
        CloseClipboard();
        out.filter(|s| !s.is_empty())
    }
}

fn clipboard_set(text: &str) {
    let t = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let w: Vec<u16> = t.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
            return;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
        if !mem.is_null() {
            let p = GlobalLock(mem) as *mut u16;
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            GlobalUnlock(mem);
            SetClipboardData(CF_UNICODETEXT, mem as _);
        }
        CloseClipboard();
    }
}
