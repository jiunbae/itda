//! Itda's high-performance sharing on a Mac: what macOS provides to the VNC server in
//! server.rs. Apple's own Screen Sharing only offers ZRLE/Zlib to other clients (~8 fps at
//! 1080p); this serves the same screen with hardware H.264 (RFB encoding 50) to viewers that
//! ask for it, on its own port so it never takes over Apple's 5900. It listens on this Mac
//! only: viewers come through an SSH tunnel (the same SSH that set it up), so the stream is
//! encrypted and the macOS firewall never asks about a new incoming port.
//!
//! - capture: ScreenCaptureKit, the main display at its size in points, up to 60 fps, the
//!   cursor drawn in (needs Screen Recording permission);
//! - input: CGEvent posted to the HID tap, Mac key codes and modifiers; ⌘ stays Command
//!   (needs Accessibility permission);
//! - clipboard: the general pasteboard;
//! - consent: an on-screen "allow?" dialog unless the viewer is trusted (server.rs).

use crate::h264_mac::H264;
use crate::server::{VideoEncode, Video};
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, Message};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{CGDisplayBounds, CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventType, CGMainDisplayID, CGMouseButton, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess, CGScrollEventUnit};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress};
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_screen_capture_kit::{SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType};
use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const KIND: &str = "Mac";
pub(crate) const MDNS_TXT: &[(&str, &str)] = &[("itda", "1"), ("os", "mac"), ("hp", "1")];
/// Default port: next to Apple's 5900, which stays Apple's.
pub const PORT: u16 = crate::host_mac_port();
pub(crate) const MOD_HOLD_MAX: Duration = Duration::from_secs(15);
pub(crate) const MOD_QUIET: Duration = Duration::from_secs(4);

pub(crate) fn name() -> String {
    std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Mac".into())
}

// ---------------------------------------------------------------- permissions

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}

/// (Screen Recording, Accessibility) granted to this app.
pub fn permissions() -> (bool, bool) {
    (CGPreflightScreenCaptureAccess(), unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) })
}

/// Ask for whatever is missing: macOS shows its own prompts (and System Settings).
pub fn request_permissions() -> (bool, bool) {
    let (screen, access) = permissions();
    if !screen {
        CGRequestScreenCaptureAccess();
    }
    if !access {
        unsafe {
            use objc2_core_foundation::{CFBoolean, CFDictionary, CFString};
            let key: &CFString = &*(kAXTrustedCheckOptionPrompt as *const CFString);
            let d = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[CFBoolean::new(true)]);
            AXIsProcessTrustedWithOptions(CFRetained::as_ptr(&d).as_ptr() as *const c_void);
        }
    }
    permissions()
}

/// Can this Mac's screen be served: Screen Recording granted to this (re)launched app.
pub(crate) fn ready() -> Result<(), String> {
    if CGPreflightScreenCaptureAccess() {
        Ok(())
    } else {
        Err("Screen Recording permission is not granted on this Mac yet".into())
    }
}

// ---------------------------------------------------------------- consent and clipboard

pub(crate) fn ask_user(peer: &SocketAddr) -> bool {
    let text = format!("{}에서 이 Mac의 화면을 보고 조작하려고 합니다. 허용할까요?", peer.ip());
    let script = format!(
        "display dialog {} with title \"Itda — 화면 공유 요청\" buttons {{\"거부\", \"허용\"}} default button \"허용\" giving up after 60",
        applescript_string(&text)
    );
    std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("button returned:허용"))
        .unwrap_or(false)
}

fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn clipboard_seq() -> u64 {
    NSPasteboard::generalPasteboard().changeCount() as u64
}

pub(crate) fn clipboard_get() -> Option<String> {
    unsafe { NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString) }.map(|s| s.to_string()).filter(|s| !s.is_empty())
}

pub(crate) fn clipboard_set(text: &str) {
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString);
    }
}

// ---------------------------------------------------------------- video

struct Hw(H264);

impl VideoEncode for Hw {
    fn size(&self) -> (usize, usize) {
        (self.0.width, self.0.height)
    }
    fn encode(&mut self, px: &[u32], stride: usize, key: bool) -> io::Result<Vec<u8>> {
        self.0.encode(px, stride, key).map(|u| u.data).map_err(io::Error::other)
    }
}

pub(crate) fn video(w: usize, h: usize) -> Option<Video> {
    // ~0.1 bit per pixel at 60 fps, within 4–40 Mbit/s.
    let bitrate = ((w * h * 6) as i64).clamp(4_000_000, 40_000_000);
    match H264::new(w, h, bitrate) {
        Ok(e) => Some(Box::new(Hw(e))),
        Err(e) => {
            crate::log!("hp: no H.264 encoder: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------- displays and capture

/// The main display in desktop points: (id, left, top, right, bottom).
pub(crate) fn monitors() -> Vec<(u32, i32, i32, i32, i32)> {
    let b = CGDisplayBounds(CGMainDisplayID());
    vec![(CGMainDisplayID(), 0, 0, b.size.width as i32, b.size.height as i32)]
}

/// The newest captured frame, handed from ScreenCaptureKit's queue to the pump.
type Latest = Arc<Mutex<Option<CFRetained<CVPixelBuffer>>>>;

struct OutputIvars {
    latest: Latest,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ItdaScreenOutput"]
    #[ivars = OutputIvars]
    struct Output;
    unsafe impl NSObjectProtocol for Output {}
    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn frame(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind != SCStreamOutputType::Screen {
                return;
            }
            // Idle frames (nothing changed) carry no image.
            if let Some(image) = unsafe { sample.image_buffer() } {
                *self.ivars().latest.lock().unwrap() = Some(image);
            }
        }
    }
);

impl Output {
    fn new(latest: Latest) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars { latest });
        unsafe { msg_send![super(this), init] }
    }
}

/// The one capture all viewers share (each diffs it against its own last frame); stopped
/// when the last viewer leaves.
pub(crate) fn capture() -> Arc<Mutex<Capture>> {
    static SHARED: Mutex<std::sync::Weak<Mutex<Capture>>> = Mutex::new(std::sync::Weak::new());
    let mut g = SHARED.lock().unwrap();
    g.upgrade().unwrap_or_else(|| {
        let c = Arc::new(Mutex::new(Capture::new()));
        *g = Arc::downgrade(&c);
        c
    })
}

pub(crate) struct Capture {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: usize,
    pub(crate) h: usize,
    buf: Vec<u32>,
    latest: Latest,
    stream: Option<Retained<SCStream>>,
    _output: Option<Retained<Output>>,
}

// The stream and output are only touched under the mutex; frames arrive through `latest`.
unsafe impl Send for Capture {}

impl Capture {
    fn new() -> Capture {
        let b = CGDisplayBounds(CGMainDisplayID());
        let (w, h) = ((b.size.width as usize).max(16) & !1, (b.size.height as usize).max(16) & !1);
        let latest: Latest = Arc::new(Mutex::new(None));
        let mut c = Capture { x: 0, y: 0, w, h, buf: vec![0; w * h], latest: latest.clone(), stream: None, _output: None };
        match start_stream(w, h, latest) {
            Ok((stream, output)) => (c.stream, c._output) = (Some(stream), Some(output)),
            Err(e) => crate::log!("hp: screen capture unavailable ({e}); viewers see a black screen"),
        }
        // Give the first frame a moment, so the viewer's first picture isn't black.
        let until = Instant::now() + Duration::from_millis(800);
        while c.latest.lock().unwrap().is_none() && c.stream.is_some() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        c
    }

    /// The capture's size is fixed for its life (one display, in points).
    pub(crate) fn fit(&mut self) {}

    pub(crate) fn grab(&mut self) -> (usize, usize, &[u32]) {
        if let Some(pb) = self.latest.lock().unwrap().take() {
            let pb = &*pb;
            if unsafe { CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) } == 0 {
                let base = CVPixelBufferGetBaseAddress(pb) as *const u8;
                let (bw, bh, bpr) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb));
                if !base.is_null() {
                    let (cw, ch) = (bw.min(self.w), bh.min(self.h));
                    for y in 0..ch {
                        let row = unsafe { std::slice::from_raw_parts(base.add(y * bpr) as *const u32, cw) };
                        for (d, s) in self.buf[y * self.w..y * self.w + cw].iter_mut().zip(row) {
                            *d = s & 0x00ff_ffff;
                        }
                    }
                }
                unsafe { CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) };
            }
        }
        (self.w, self.h, &self.buf)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(s) = &self.stream {
            unsafe { s.stopCaptureWithCompletionHandler(None) };
        }
    }
}

/// Start a ScreenCaptureKit stream of the main display at w×h, BGRA, ≤60 fps.
fn start_stream(w: usize, h: usize, latest: Latest) -> Result<(Retained<SCStream>, Retained<Output>), String> {
    if !CGPreflightScreenCaptureAccess() {
        return Err("Screen Recording permission not granted".into());
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<Retained<SCShareableContent>, String>>();
    let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        let r = match unsafe { content.as_ref() } {
            Some(c) => Ok(c.retain()),
            None => Err(unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "no shareable content".into())),
        };
        let _ = tx.send(r);
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let content = rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "ScreenCaptureKit didn't answer".to_string())??;
    let main = CGMainDisplayID();
    let displays = unsafe { content.displays() };
    let display = displays.iter().find(|d| unsafe { d.displayID() } == main).or_else(|| displays.iter().next()).ok_or("no display")?;
    unsafe {
        let filter = SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::new());
        let config = SCStreamConfiguration::new();
        config.setWidth(w);
        config.setHeight(h);
        config.setPixelFormat(u32::from_be_bytes(*b"BGRA"));
        config.setMinimumFrameInterval(objc2_core_media::CMTime::new(1, 60));
        config.setShowsCursor(true);
        config.setQueueDepth(3);
        let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None);
        let output = Output::new(latest);
        let queue = DispatchQueue::new("itda.capture", None);
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(ProtocolObject::from_ref(&*output), SCStreamOutputType::Screen, Some(&queue))
            .map_err(|e| e.localizedDescription().to_string())?;
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        let done = RcBlock::new(move |error: *mut NSError| {
            let _ = tx.send(error.as_ref().map(|e| e.localizedDescription().to_string()));
        });
        stream.startCaptureWithCompletionHandler(Some(&done));
        if let Ok(Some(e)) = rx.recv_timeout(Duration::from_secs(5)) {
            return Err(e);
        }
        Ok((stream, output))
    }
}

// ---------------------------------------------------------------- input

pub(crate) struct Input {
    view_only: bool,
    buttons: u8,
    /// keysym → key code pressed for it, so the release matches.
    down: HashMap<u32, u16>,
    flags: CGEventFlags,
    pub(crate) last_key: Instant,
    last_click: (Instant, u16, u16, i64),
    /// Where the pointer is now (button releases on cleanup happen here).
    at: (u16, u16),
}

/// Mac key codes (ANSI layout) for X keysyms; the bool marks modifiers (with their flag).
fn keycode(sym: u32) -> Option<(u16, Option<CGEventFlags>)> {
    let m = |c: u16, f: CGEventFlags| Some((c, Some(f)));
    let k = |c: u16| Some((c, None));
    match sym {
        0xffe1 => m(56, CGEventFlags::MaskShift),
        0xffe2 => m(60, CGEventFlags::MaskShift),
        0xffe3 => m(59, CGEventFlags::MaskControl),
        0xffe4 => m(62, CGEventFlags::MaskControl),
        0xffe9 | 0xffe7 if sym == 0xffe9 => m(58, CGEventFlags::MaskAlternate),
        0xffea => m(61, CGEventFlags::MaskAlternate),
        0xffeb | 0xffe7 => m(55, CGEventFlags::MaskCommand),
        0xffec | 0xffe8 => m(54, CGEventFlags::MaskCommand),
        0xffe5 => k(57), // Caps Lock
        0xff0d | 0xff8d => k(36),
        0xff09 => k(48),
        0x20 => k(49),
        0xff08 => k(51),
        0xff1b => k(53),
        0xffff => k(117),
        0xff50 => k(115),
        0xff57 => k(119),
        0xff55 => k(116),
        0xff56 => k(121),
        0xff51 => k(123),
        0xff53 => k(124),
        0xff54 => k(125),
        0xff52 => k(126),
        0xffbe..=0xffc9 => k([122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111][(sym - 0xffbe) as usize]),
        _ => {
            let c = char::from_u32(sym).filter(|c| c.is_ascii())?.to_ascii_lowercase();
            let code = match c {
                'a' => 0, 's' => 1, 'd' => 2, 'f' => 3, 'h' => 4, 'g' => 5, 'z' => 6, 'x' => 7, 'c' => 8, 'v' => 9,
                'b' => 11, 'q' => 12, 'w' => 13, 'e' => 14, 'r' => 15, 'y' => 16, 't' => 17,
                '1' | '!' => 18, '2' | '@' => 19, '3' | '#' => 20, '4' | '$' => 21, '6' | '^' => 22, '5' | '%' => 23,
                '=' | '+' => 24, '9' | '(' => 25, '7' | '&' => 26, '-' | '_' => 27, '8' | '*' => 28, '0' | ')' => 29,
                ']' | '}' => 30, 'o' => 31, 'u' => 32, '[' | '{' => 33, 'i' => 34, 'p' => 35, 'l' => 37, 'j' => 38,
                '\'' | '"' => 39, 'k' => 40, ';' | ':' => 41, '\\' | '|' => 42, ',' | '<' => 43, '/' | '?' => 44,
                'n' => 45, 'm' => 46, '.' | '>' => 47, '`' | '~' => 50,
                _ => return None,
            };
            k(code)
        }
    }
}

fn post(e: Option<CFRetained<CGEvent>>) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, e.as_deref());
}

impl Input {
    pub(crate) fn new(_cmd_as_ctrl: bool, view_only: bool) -> Input {
        Input { view_only, buttons: 0, down: HashMap::new(), flags: CGEventFlags(0), last_key: Instant::now(), last_click: (Instant::now() - Duration::from_secs(5), 0, 0, 0), at: (0, 0) }
    }

    pub(crate) fn modifiers_held(&self) -> bool {
        self.flags.0 != 0
    }

    pub(crate) fn release_modifiers(&mut self, why: &str) {
        let held: Vec<u32> = self.down.keys().copied().filter(|s| keycode(*s).is_some_and(|k| k.1.is_some())).collect();
        for s in held {
            self.key(false, s);
        }
        self.flags = CGEventFlags(0);
        crate::log!("hp: released modifier keys ({why})");
    }

    pub(crate) fn key(&mut self, down: bool, sym: u32) {
        self.last_key = Instant::now();
        if self.view_only {
            return;
        }
        let (code, flag) = match keycode(sym) {
            Some(k) => k,
            None => {
                // Not on the keyboard map: type the character itself.
                if !down {
                    return;
                }
                let Some(ch) = char::from_u32(if sym >= 0x0100_0000 { sym - 0x0100_0000 } else { sym }) else { return };
                let mut b = [0u16; 2];
                let units = ch.encode_utf16(&mut b);
                for d in [true, false] {
                    let e = CGEvent::new_keyboard_event(None, 0, d);
                    unsafe { CGEvent::keyboard_set_unicode_string(e.as_deref(), units.len() as _, units.as_ptr()) };
                    post(e);
                }
                return;
            }
        };
        if down {
            self.down.insert(sym, code);
        } else if self.down.remove(&sym).is_none() && flag.is_none() {
            return; // a release for a key we never pressed
        }
        if flag.is_some() {
            // From the modifiers still held: releasing Left Shift keeps Right Shift's flag.
            self.flags = CGEventFlags(self.down.keys().filter_map(|s| keycode(*s).and_then(|k| k.1)).fold(0, |a, f| a | f.0));
        }
        let e = CGEvent::new_keyboard_event(None, code, down);
        CGEvent::set_flags(e.as_deref(), self.flags);
        post(e);
    }

    pub(crate) fn pointer(&mut self, mask: u8, x: u16, y: u16) {
        if self.view_only {
            return;
        }
        let at = CGPoint { x: x as f64, y: y as f64 };
        self.at = (x, y);
        let changed = mask ^ self.buttons;
        let mouse = |kind: CGEventType, button: CGMouseButton, clicks: i64| {
            let e = CGEvent::new_mouse_event(None, kind, at, button);
            CGEvent::set_flags(e.as_deref(), self.flags);
            if clicks > 0 {
                CGEvent::set_integer_value_field(e.as_deref(), CGEventField::MouseEventClickState, clicks);
            }
            post(e);
        };
        // Double clicks: a press soon after one at about the same place counts up.
        let clicks = if changed & mask & 7 != 0 {
            let (t, lx, ly, n) = self.last_click;
            let n = if t.elapsed() < Duration::from_millis(450) && lx.abs_diff(x) < 5 && ly.abs_diff(y) < 5 { n + 1 } else { 1 };
            self.last_click = (Instant::now(), x, y, n);
            n
        } else {
            self.last_click.3
        };
        let buttons = [(1u8, CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left), (4, CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGMouseButton::Right), (2, CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGMouseButton::Center)];
        let mut sent = false;
        for (bit, press, lift, button) in buttons {
            if changed & bit != 0 {
                mouse(if mask & bit != 0 { press } else { lift }, button, clicks.max(1));
                sent = true;
            }
        }
        if !sent {
            let (kind, button) = if mask & 1 != 0 {
                (CGEventType::LeftMouseDragged, CGMouseButton::Left)
            } else if mask & 4 != 0 {
                (CGEventType::RightMouseDragged, CGMouseButton::Right)
            } else if mask & 2 != 0 {
                (CGEventType::OtherMouseDragged, CGMouseButton::Center)
            } else {
                (CGEventType::MouseMoved, CGMouseButton::Left)
            };
            mouse(kind, button, 0);
        }
        // Wheel: buttons 4/5 (up/down) and 6/7 (left/right) are clicks.
        for (bit, dy, dx) in [(8u8, 3, 0), (16, -3, 0), (32, 0, 3), (64, 0, -3)] {
            if changed & bit != 0 && mask & bit != 0 {
                post(CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 2, dy, dx, 0));
            }
        }
        self.buttons = mask;
    }

    pub(crate) fn release_all(&mut self) {
        let held: Vec<u32> = self.down.keys().copied().collect();
        for s in held {
            self.key(false, s);
        }
        if self.buttons & 7 != 0 {
            self.pointer(0, self.at.0, self.at.1);
        }
    }
}

// ---------------------------------------------------------------- running it

/// The password viewers use for this Mac's high-performance sharing: Itda's data folder
/// (0600), made on first use. Another Itda that installs this over SSH reads it the same way.
pub fn password() -> String {
    let path = crate::store::dir().join("hp-password");
    if let Ok(p) = std::fs::read_to_string(&path) {
        let p = p.trim().to_string();
        if p.len() >= 6 {
            return p;
        }
    }
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let p: String = b.iter().map(|x| CHARS[*x as usize % CHARS.len()] as char).collect();
    let _ = std::fs::create_dir_all(crate::store::dir());
    let _ = std::fs::write(&path, &p);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    p
}

/// Itda's high-performance sharing is on for this Mac (a marker in Itda's data folder, set
/// here or by another Itda over SSH), so the app serves it whenever it runs, including
/// after macOS relaunches it for a newly granted Screen Recording permission.
pub fn enabled() -> bool {
    crate::store::dir().join("hp-sharing").exists()
}

pub fn set_enabled(on: bool) {
    let path = crate::store::dir().join("hp-sharing");
    let _ = if on { std::fs::write(&path, b"1") } else { std::fs::remove_file(&path) };
}

/// Where the background server reports its permissions, for an Itda setting it up over SSH:
/// `screen=0|1 access=0|1 port=N`.
fn write_status(screen: bool, access: bool) {
    let _ = std::fs::write(crate::store::dir().join("hp-status"), format!("screen={} access={} port={PORT}\n", screen as u8, access as u8));
}

/// Start serving in the background of the running app (when `enabled`).
pub fn start_background() {
    std::thread::spawn(|| {
        let (screen, access) = request_permissions();
        crate::log!("hp sharing: screen recording {screen}, accessibility {access}");
        write_status(screen, access);
        // Rewritten every 2 s while it serves: an Itda setting this up over SSH waits for a
        // fresh one (also when sharing was already running), and sees permissions granted.
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(2));
            let (screen, access) = permissions();
            write_status(screen, access);
        });
        // Consent: it listens on this Mac only, so every viewer has already signed in to this
        // Mac's account over SSH and knows the 0600 password; no extra "allow?" dialog.
        let cfg = crate::server::Config { port: PORT, password: password(), ask: false, trust_near: false, cmd_as_ctrl: false, view_only: false, local_only: true };
        if let Err(e) = crate::server::start(cfg) {
            crate::log!("hp sharing: {e}");
        }
    });
}

/// `itda --hp-server [port]`: serve this Mac until killed (asks for missing permissions).
pub fn serve_forever(port: u16) -> ! {
    // Tests: no permission prompts and no "allow?" dialog (and loopback only, by trust).
    let quiet = std::env::var_os("ITDA_HP_TEST").is_some();
    let (screen, access) = if quiet { permissions() } else { request_permissions() };
    crate::log!("hp server: screen recording {screen}, accessibility {access}");
    let cfg = crate::server::Config { port, password: password(), ask: !quiet, trust_near: false, cmd_as_ctrl: false, view_only: quiet, local_only: true };
    if let Err(e) = crate::server::start(cfg) {
        crate::log!("hp server: {e}");
        eprintln!("{e}");
        std::process::exit(1);
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
