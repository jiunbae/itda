//! Itda's high-performance sharing on a Mac: what macOS provides to the VNC server in
//! server.rs. Apple's own Screen Sharing only offers ZRLE/Zlib to other clients (~8 fps at
//! 1080p); this serves the same screen with hardware H.264 (RFB encoding 50) to viewers that
//! ask for it, on its own port so it never takes over Apple's 5900. It listens on this Mac
//! only: viewers come through an SSH tunnel (the same SSH that set it up), so the stream is
//! encrypted and the macOS firewall never asks about a new incoming port.
//!
//! - capture: ScreenCaptureKit, the main display in points (or opt-in native pixels), up to 60 fps, the
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
use objc2::runtime::{AnyClass, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, Message};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode, CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventType, CGMainDisplayID, CGMouseButton, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess, CGScrollEventUnit};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress};
use objc2_foundation::{NSArray, NSError, NSProcessInfo, NSString};
use objc2_screen_capture_kit::{SCContentFilter, SCDisplay, SCShareableContent, SCWindow, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamErrorCode, SCStreamErrorDomain, SCStreamOutput, SCStreamOutputType};
use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Do not resolve a typed SCK class on an OS where the weak-linked framework is absent.
/// The rest of Itda (including remote viewing and files) does not require this feature.
pub fn capture_support() -> Result<(), &'static str> {
    let v = NSProcessInfo::processInfo().operatingSystemVersion();
    if !crate::capture_support::macos_supported(v.majorVersion, v.minorVersion) {
        return Err(crate::capture_support::MACOS_CAPTURE_REQUIREMENT);
    }
    if [c"SCShareableContent", c"SCStream", c"SCContentFilter", c"SCStreamConfiguration"]
        .into_iter().any(|name| AnyClass::get(name).is_none()) {
        return Err(crate::capture_support::CAPTURE_UNAVAILABLE);
    }
    Ok(())
}

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
pub fn request_permission(kind: &str) {
    if capture_support().is_err() { return; }
    let (screen, access) = permissions();
    if kind == "screen" && !screen { CGRequestScreenCaptureAccess(); }
    if kind == "access" && !access {
        unsafe {
            use objc2_core_foundation::{CFBoolean, CFDictionary, CFString};
            let key: &CFString = &*(kAXTrustedCheckOptionPrompt as *const CFString);
            let d = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[CFBoolean::new(true)]);
            AXIsProcessTrustedWithOptions(CFRetained::as_ptr(&d).as_ptr() as *const c_void);
        }
    }
}
pub fn request_permissions() -> (bool, bool) {
    request_permission("screen"); request_permission("access"); permissions()
}

/// Can this Mac's screen be served: Screen Recording granted to this (re)launched app, and the
/// screen unlocked with this user at it. (Locked, or another user at the screen, capture shows
/// only the desktop picture; the viewer's Itda then uses Screen Sharing, which shows the lock
/// screen.)
pub(crate) fn ready() -> Result<(), String> {
    check_intent(&USER_STOPPED)?;
    capture_support().map_err(str::to_owned)?;
    if !CGPreflightScreenCaptureAccess() {
        return Err("이 Mac에서 화면 기록 권한이 아직 허용되지 않았습니다".into());
    }
    match session_state() {
        Some((true, _)) => Err("화면이 잠겨 있어 고성능 모드로 보여 줄 수 없습니다".into()),
        Some((_, false)) => Err("다른 사용자가 이 Mac 화면을 쓰고 있어 고성능 모드로 보여 줄 수 없습니다".into()),
        _ => Ok(()),
    }
}

/// (screen locked, this session on the console) for the user Itda runs as.
fn session_state() -> Option<(bool, bool)> {
    use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType};
    let d = objc2_core_graphics::CGSessionCopyCurrentDictionary()?;
    let d: &CFDictionary<CFString, CFType> = unsafe { d.cast_unchecked() };
    let flag = |key: &str| d.get(&CFString::from_str(key)).and_then(|v| v.downcast_ref::<CFBoolean>().map(|b| b.as_bool()));
    Some((flag("CGSSessionScreenIsLocked").unwrap_or(false), flag("kCGSSessionOnConsoleKey").unwrap_or(true)))
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
    fn encode_buffer(&mut self, buffer: &CVPixelBuffer, key: bool) -> Option<io::Result<Vec<u8>>> {
        Some(self.0.encode_buffer(buffer, key).map(|u| u.data).map_err(io::Error::other))
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

/// Capture geometry is frozen for all viewers until the last one leaves. This keeps
/// pointer input and monitor metadata in the same units as the shared framebuffer.
static SHARED_CAPTURE: Mutex<std::sync::Weak<Mutex<Capture>>> = Mutex::new(std::sync::Weak::new());
// Concurrent connections share a failed attempt too: they must not each queue another
// full startup timeout while holding the capture lock.
static CAPTURE_FAILURE: Mutex<Option<(Instant, String)>> = Mutex::new(None);
// User intent outlives a stream, its last viewer, and late callbacks until app relaunch.
static USER_STOPPED: AtomicBool = AtomicBool::new(false);
const CAPTURE_COOLDOWN: Duration = Duration::from_secs(2);
const START_WAIT: Duration = Duration::from_secs(3);
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);

fn native_pixels_requested() -> bool {
    std::env::args().any(|a| a == "--native-pixels") ||
        std::fs::read_to_string(crate::store::dir().join("hp-quality")).is_ok_and(|s| s.trim() == "native")
}

fn capture_dimensions(points: (usize, usize), pixels: (usize, usize), native: bool) -> (usize, usize) {
    let (mut w, mut h) = if native { pixels } else { points };
    if native && w.max(h) > 4096 {
        let longest = w.max(h);
        w = w * 4096 / longest;
        h = h * 4096 / longest;
    }
    (w.max(16) & !1, h.max(16) & !1)
}

fn backing_dimensions(points: (usize, usize), mode_points: (usize, usize), mode_pixels: (usize, usize)) -> (usize, usize) {
    if mode_points.0 == 0 || mode_points.1 == 0 { return points; }
    // Apply backing density to the desktop bounds, rather than using the mode's
    // orientation. A portrait desktop can have a landscape display-mode record.
    (points.0.saturating_mul(mode_pixels.0) / mode_points.0,
     points.1.saturating_mul(mode_pixels.1) / mode_points.1)
}

fn display_dimensions(native: bool) -> (usize, usize) {
    let id = CGMainDisplayID();
    let b = CGDisplayBounds(id);
    let points = (b.size.width as usize, b.size.height as usize);
    // CGDisplayPixelsWide/High can return the logical mode dimensions on HiDPI
    // displays. The display mode's pixel dimensions describe the Retina backing.
    let mode = CGDisplayCopyDisplayMode(id);
    let pixels = mode.as_deref().map(|m| backing_dimensions(points, (CGDisplayMode::width(Some(m)), CGDisplayMode::height(Some(m))), (CGDisplayMode::pixel_width(Some(m)), CGDisplayMode::pixel_height(Some(m))))).unwrap_or(points);
    capture_dimensions(points, pixels, native)
}

fn active_capture() -> Option<Arc<Mutex<Capture>>> {
    SHARED_CAPTURE.lock().unwrap().upgrade()
}

fn frame_dimensions() -> (usize, usize) {
    active_capture().map(|c| { let c = c.lock().unwrap(); (c.w, c.h) })
        .unwrap_or_else(|| display_dimensions(native_pixels_requested()))
}

fn point_from_frame(x: u16, y: u16, frame: (usize, usize), points: (f64, f64)) -> (f64, f64) {
    (f64::from(x).min(frame.0.saturating_sub(1) as f64) * points.0 / frame.0 as f64,
     f64::from(y).min(frame.1.saturating_sub(1) as f64) * points.1 / frame.1 as f64)
}

/// The main display in framebuffer pixels: (id, left, top, right, bottom).
pub(crate) fn monitors() -> Vec<(u32, i32, i32, i32, i32)> {
    let (w, h) = frame_dimensions();
    vec![(CGMainDisplayID(), 0, 0, w as i32, h as i32)]
}

/// The newest captured frame, handed from ScreenCaptureKit's queue to the pump.
/// (With when it arrived, under ITDA_HP_TRACE.)
type Latest = Arc<Mutex<Option<(CFRetained<CVPixelBuffer>, Option<Instant>)>>>;

#[derive(Clone)]
struct StreamHealth {
    last_frame: Instant,
    frames: u64,
    error: Option<String>,
    user_stopped: bool,
}
impl StreamHealth {
    fn new() -> Self { Self { last_frame: Instant::now(), frames: 0, error: None, user_stopped: false } }
}
type Health = Arc<Mutex<StreamHealth>>;

const QUIET_PROBE: Duration = Duration::from_secs(8);
const RECOVERY_BACKOFF: Duration = Duration::from_secs(2);
const RECOVERY_WINDOW: Duration = Duration::from_secs(60);
const CAPTURE_STOPPED: &str = "이 Mac에서 화면 공유를 중지했습니다. 다시 공유하려면 이 Mac에서 Itda를 다시 열어 주세요.";
const CAPTURE_LOST: &str = "고성능 화면 캡처가 멈춰 연결을 종료합니다. 잠시 후 다시 연결해 주세요.";

fn intentional_stop(sc_domain: bool, code: isize) -> bool {
    sc_domain && matches!(SCStreamErrorCode(code), SCStreamErrorCode::UserStopped | SCStreamErrorCode::UserDeclined)
}
fn check_intent(latch: &AtomicBool) -> Result<(), String> {
    if latch.load(Ordering::Acquire) { Err(CAPTURE_STOPPED.into()) } else { Ok(()) }
}
fn record_capture_error(latch: &AtomicBool, sc_domain: bool, code: isize, reason: String) -> String {
    if intentional_stop(sc_domain, code) {
        latch.store(true, Ordering::Release);
        CAPTURE_STOPPED.into()
    } else { reason }
}
fn capture_error(error: &NSError) -> String {
    record_capture_error(&USER_STOPPED, unsafe { error.domain().isEqualToString(SCStreamErrorDomain) },
        error.code(), error.localizedDescription().to_string())
}

/// A quiet desktop is normal: lack of images asks for a liveness probe, never a restart
/// by itself. Only an explicit stream failure or an unanswered/failed probe restarts it.
struct Watchdog {
    probed: Instant,
    retry_at: Instant,
    reason: Option<String>,
    attempts: Vec<Instant>,
    failures: u8,
    probe_failures: u8,
    observed_frames: Option<u64>,
    delegate_error: bool,
}
impl Watchdog {
    fn new(now: Instant) -> Self {
        Self { probed: now, retry_at: now, reason: None, attempts: Vec::new(), failures: 0,
            probe_failures: 0, observed_frames: None, delegate_error: false }
    }
    fn step<T>(&mut self, now: Instant, health: &StreamHealth,
        probe: impl FnOnce() -> Result<(), String>,
        restart: impl FnOnce(bool) -> Result<Option<T>, String>,
        completed: impl Fn() -> Instant) -> Result<Option<T>, String> {
        if health.user_stopped { return Err(CAPTURE_STOPPED.into()); }
        if self.observed_frames != Some(health.frames) {
            self.observed_frames = Some(health.frames);
            self.probe_failures = 0;
            // Actual frames cancel a probe suspicion/retry; a delegate-stopped stream stays failed.
            if !self.delegate_error { self.reason = None; self.failures = 0; }
        }
        if let Some(error) = &health.error {
            self.reason = Some(error.clone());
            self.delegate_error = true;
        }
        if self.reason.is_none() && now.saturating_duration_since(health.last_frame) >= QUIET_PROBE
            && now.saturating_duration_since(self.probed) >= QUIET_PROBE {
            let result = probe();
            self.probed = completed();
            match result {
                Ok(()) => self.probe_failures = 0,
                Err(error) if error == CAPTURE_STOPPED => return Err(error),
                Err(error) => {
                    self.probe_failures += 1;
                    // A single delay after wake/load is not evidence that the stream is broken.
                    if self.probe_failures >= 2 { self.reason = Some(error); }
                }
            }
        }
        if self.reason.as_deref() == Some(CAPTURE_STOPPED) { return Err(CAPTURE_STOPPED.into()); }
        if self.reason.is_none() || now < self.retry_at { return Ok(None); }
        self.attempts.retain(|at| now.saturating_duration_since(*at) < RECOVERY_WINDOW);
        if self.attempts.len() >= 3 { return Err(CAPTURE_LOST.into()); }
        self.attempts.push(now);
        crate::log!("hp: restarting stalled capture: {}", self.reason.as_deref().unwrap_or_default());
        // Stop first ONLY after an explicit delegate error. Probe suspicion retains the old stream.
        let restarted = restart(self.delegate_error);
        let completed = completed();
        match restarted {
            Ok(capture) => {
                self.reason = None;
                self.delegate_error = false;
                self.failures = 0;
                self.probe_failures = 0;
                self.probed = completed;
                Ok(capture) // None means the old stream recovered while a candidate failed.
            }
            Err(error) => {
                if error == CAPTURE_STOPPED { return Err(error); }
                crate::log!("hp: capture restart failed: {error}");
                self.failures += 1;
                self.retry_at = completed + RECOVERY_BACKOFF;
                if self.failures >= 2 { Err(CAPTURE_LOST.into()) } else { Ok(None) }
            }
        }
    }
}

/// Called when a new frame arrives.
type Waker = Box<dyn Fn() + Send>;
static FRAME_WAKERS: Mutex<Vec<(u64, Waker)>> = Mutex::new(Vec::new());
static NEXT_WAKER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Have `wake` called whenever ScreenCaptureKit delivers a new frame, until `off_frame(id)`.
pub(crate) fn on_frame(wake: Waker) -> u64 {
    let id = NEXT_WAKER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    FRAME_WAKERS.lock().unwrap().push((id, wake));
    id
}

pub(crate) fn off_frame(id: u64) {
    FRAME_WAKERS.lock().unwrap().retain(|(i, _)| *i != id);
}

struct OutputIvars {
    latest: Latest,
    health: Health,
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
                { let mut h = self.ivars().health.lock().unwrap(); h.last_frame = Instant::now(); h.frames = h.frames.wrapping_add(1); }
                let arrived = crate::trace::on().then(|| {
                    // From the frame's display time to its delivery here.
                    let (now, shown) = unsafe { (objc2_core_media::CMClock::host_time_clock().time().seconds(), sample.presentation_time_stamp().seconds()) };
                    crate::trace::add("sck", ((now - shown) * 1000.0) as f32);
                    Instant::now()
                });
                *self.ivars().latest.lock().unwrap() = Some((image, arrived));
                // Wake every viewer's pump now, rather than at its next poll.
                for (_, wake) in FRAME_WAKERS.lock().unwrap().iter() {
                    wake();
                }
            }
        }
    }
    unsafe impl SCStreamDelegate for Output {
        #[unsafe(method(stream:didStopWithError:))]
        fn stopped(&self, _stream: &SCStream, error: &NSError) {
            let reason = capture_error(error); // Latch immediately, including late/discarded streams.
            let mut health = self.ivars().health.lock().unwrap();
            health.user_stopped |= reason == CAPTURE_STOPPED;
            health.error = Some(reason);
        }
    }
);

impl Output {
    fn new(latest: Latest, health: Health) -> Retained<Self> {
        let this = Self::alloc().set_ivars(OutputIvars { latest, health });
        unsafe { msg_send![super(this), init] }
    }
}

/// The one capture all viewers share (each diffs it against its own last frame); stopped
/// when the last viewer leaves.
pub(crate) fn capture() -> Result<Arc<Mutex<Capture>>, String> {
    check_intent(&USER_STOPPED)?;
    let mut g = SHARED_CAPTURE.lock().unwrap();
    if let Some(c) = g.upgrade() {
        let error = c.lock().unwrap().failed.clone();
        return match error { Some(error) => Err(error), None => Ok(c) };
    }
    if let Some((at, why)) = CAPTURE_FAILURE.lock().unwrap().as_ref() {
        if at.elapsed() < CAPTURE_COOLDOWN { return Err(why.clone()); }
    }
    match Capture::new() {
        Ok(c) => {
            check_intent(&USER_STOPPED)?;
            let c = Arc::new(Mutex::new(c));
            supervise_capture(Arc::downgrade(&c))?;
            *g = Arc::downgrade(&c);
            *CAPTURE_FAILURE.lock().unwrap() = None;
            Ok(c)
        }
        Err(why) => {
            crate::log!("hp: screen capture unavailable: {why}");
            // Keep framework diagnostics out of the auth reason (including words
            // like "password" which RFB clients classify as bad credentials).
            let stopped = why == CAPTURE_STOPPED;
            let why = if stopped { CAPTURE_STOPPED } else { "이 Mac의 화면 캡처를 시작하지 못했습니다. 잠시 후 다시 시도해 주세요." }.to_string();
            *CAPTURE_FAILURE.lock().unwrap() = Some((Instant::now(), why.clone()));
            Err(why)
        }
    }
}

/// A fixed offscreen framebuffer for the opt-in real encoder/pump refinement probe.
#[cfg(test)]
pub(crate) fn fixed_capture(w: usize, h: usize, pixels: Vec<u32>) -> Capture {
    assert_eq!(pixels.len(), w * h);
    Capture { x: 0, y: 0, w, h, native_pixels: false, buf: Arc::new(pixels), native: None,
        latest: Arc::new(Mutex::new(None)), health: Arc::new(Mutex::new(StreamHealth::new())), failed: None, generation: 0, stream: None, _output: None }
}

pub(crate) struct Capture {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: usize,
    pub(crate) h: usize,
    native_pixels: bool,
    buf: Arc<Vec<u32>>,
    native: Option<CFRetained<CVPixelBuffer>>,
    latest: Latest,
    health: Health,
    failed: Option<String>,
    /// Counts frames copied into `buf`: each viewer remembers the one it last grabbed, since
    /// all viewers share this capture and the first to grab takes the pending frame.
    generation: u64,
    stream: Option<Retained<SCStream>>,
    _output: Option<Retained<Output>>,
}

/// One immutable capture shared with a viewer. Keep both representations together: the
/// native buffer encodes the same image that `pixels` compares and repaints losslessly.
pub(crate) struct Snapshot {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub pixels: Arc<Vec<u32>>,
    pub native: Option<CFRetained<CVPixelBuffer>>,
    pub generation: u64,
}

// The stream and output are only touched under the mutex; frames arrive through `latest`.
unsafe impl Send for Capture {}

impl Capture {
    fn new() -> Result<Capture, String> {
        let native_pixels = native_pixels_requested();
        let (w, h) = display_dimensions(native_pixels);
        Self::with_dimensions(w, h, native_pixels)
    }

    fn with_dimensions(w: usize, h: usize, native_pixels: bool) -> Result<Capture, String> {
        check_intent(&USER_STOPPED)?;
        let (stream, output, latest, health) = start_stream(w, h)?;
        let mut c = Capture { x: 0, y: 0, w, h, native_pixels, buf: Arc::new(vec![0; w * h]), native: None, latest, health, failed: None, generation: 0, stream: Some(stream), _output: Some(output) };
        // Success means a usable image exists, not only that startCapture returned.
        // Drop stops the stream on failure; no empty capture is published to viewers.
        let until = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            check_intent(&USER_STOPPED)?;
            c.update();
            let health = c.health.lock().unwrap().clone();
            if health.user_stopped { return Err(CAPTURE_STOPPED.into()); }
            if let Some(error) = health.error { return Err(error); }
            if c.generation != 0 { return Ok(c); }
            if Instant::now() >= until { return Err("ScreenCaptureKit이 첫 화면을 보내지 않았습니다".into()); }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The capture's size and quality are fixed for its life (one display).
    pub(crate) fn fit(&mut self) {}

    pub(crate) fn check(&self) -> io::Result<()> {
        check_intent(&USER_STOPPED).map_err(io::Error::other)?;
        match &self.failed { Some(error) => Err(io::Error::other(error.clone())), None => Ok(()) }
    }

    /// Has a frame arrived that the viewer who last grabbed at `seen` hasn't had? (Pending, or
    /// grabbed by another viewer since. ScreenCaptureKit sends none while nothing changes.)
    pub(crate) fn fresh_since(&self, seen: u64) -> bool {
        self.generation != seen || self.latest.lock().unwrap().is_some()
    }

    /// The frame `buf` holds, for `fresh_since`.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    fn update(&mut self) {
        let pending = self.latest.lock().unwrap().take();
        if let Some((pb, arrived)) = pending {
            if let Some(at) = arrived {
                crate::trace::since("wait", at);
            }
            let copy = crate::trace::on().then(Instant::now);
            if unsafe { CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags::ReadOnly) } == 0 {
                let base = CVPixelBufferGetBaseAddress(&pb) as *const u8;
                let (bw, bh, bpr) = (CVPixelBufferGetWidth(&pb), CVPixelBufferGetHeight(&pb), CVPixelBufferGetBytesPerRow(&pb));
                if !base.is_null() && bw > 0 && bh > 0 {
                    let (cw, ch) = (bw.min(self.w), bh.min(self.h));
                    // A viewer may still be encoding the previous image. Reuse its storage
                    // only when unshared; never overwrite a snapshot another viewer holds.
                    let pixels = Arc::make_mut(&mut self.buf);
                    for y in 0..ch {
                        let row = unsafe { std::slice::from_raw_parts(base.add(y * bpr) as *const u32, cw) };
                        for (d, s) in pixels[y * self.w..y * self.w + cw].iter_mut().zip(row) {
                            *d = s & 0x00ff_ffff;
                        }
                    }
                    self.generation = self.generation.wrapping_add(1);
                    // A cropped/mismatched SCK image still uses the existing CPU fallback;
                    // the encoder must never receive a differently sized native buffer.
                    self.native = (bw == self.w && bh == self.h).then(|| pb.clone());
                }
                unsafe { CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags::ReadOnly) };
            }
            if let Some(t) = copy {
                crate::trace::since("copy", t);
            }
        }
    }

    pub(crate) fn snapshot(&mut self) -> Snapshot {
        self.update();
        Snapshot { x: self.x, y: self.y, w: self.w, h: self.h, pixels: self.buf.clone(),
            native: self.native.clone(), generation: self.generation }
    }

    pub(crate) fn grab(&mut self) -> (usize, usize, &[u32]) {
        self.update();
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

fn wake_viewers() {
    for (_, wake) in FRAME_WAKERS.lock().unwrap().iter() { wake(); }
}

/// Dropping an unadopted candidate (including a departed last viewer) stops only that candidate.
/// The replaced capture is dropped after releasing the framebuffer mutex.
fn adopt_capture<T>(weak: &std::sync::Weak<Mutex<T>>, mut fresh: T,
    prepare: impl FnOnce(&mut T, &T)) -> bool {
    let Some(capture) = weak.upgrade() else { return false; };
    let old = {
        let mut old = capture.lock().unwrap();
        prepare(&mut fresh, &old);
        std::mem::replace(&mut *old, fresh)
    };
    drop(old);
    true
}

/// The supervisor owns only a Weak capture between operations. No framework startup wait
/// holds the framebuffer mutex or keeps a viewer-less capture alive.
fn supervise_capture(weak: std::sync::Weak<Mutex<Capture>>) -> Result<(), String> {
    std::thread::Builder::new().name("hp-capture-watchdog".into()).spawn(move || {
        let mut watchdog = Watchdog::new(Instant::now());
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let (w, h, native, stream, health) = {
                let Some(capture) = weak.upgrade() else { return; };
                let c = capture.lock().unwrap();
                if c.failed.is_some() { return; }
                (c.w, c.h, c.native_pixels, c.stream.clone(), c.health.clone())
            };
            let Some(stream) = stream else { return; };
            let mut observed = health.lock().unwrap().clone();
            observed.user_stopped |= USER_STOPPED.load(Ordering::Acquire);
            let result = watchdog.step(Instant::now(), &observed,
                || probe_stream(&stream, w, h, &health),
                |stop_first| {
                    if weak.strong_count() == 0 { return Err("viewer left".into()); }
                    if stop_first { unsafe { stream.stopCaptureWithCompletionHandler(None); } }
                    // Keep negotiated w/h for existing viewers and pointer coordinates.
                    // Probe recovery leaves the old stream alive until a usable candidate is adopted.
                    let result = Capture::with_dimensions(w, h, native);
                    let old = health.lock().unwrap();
                    if result.is_err() && !stop_first && old.error.is_none() && old.frames != observed.frames {
                        Ok(None) // Old capture resumed; don't turn a failed candidate into terminal failure.
                    } else { result.map(Some) }
                }, Instant::now);
            // A user stop may arrive while a replacement is starting. Never publish it then.
            let stopped = USER_STOPPED.load(Ordering::Acquire) || health.lock().unwrap().user_stopped || match &result {
                Ok(Some(fresh)) => fresh.health.lock().unwrap().user_stopped,
                _ => false,
            };
            let result = if stopped { Err(CAPTURE_STOPPED.into()) } else { result };
            match result {
                Ok(Some(fresh)) => {
                    if !adopt_capture(&weak, fresh, |fresh, old| {
                        fresh.generation = old.generation.wrapping_add(1);
                    }) { return; }
                    wake_viewers();
                    crate::log!("hp: capture recovered");
                }
                Ok(None) => (),
                Err(error) => {
                    let Some(capture) = weak.upgrade() else { return; };
                    capture.lock().unwrap().failed = Some(error.clone());
                    *CAPTURE_FAILURE.lock().unwrap() = Some((Instant::now(), error));
                    wake_viewers(); // Pump checks failure even with no update demand.
                    return;
                }
            }
        }
    }).map(|_| ()).map_err(|e| e.to_string())
}

fn stream_configuration(w: usize, h: usize) -> Retained<SCStreamConfiguration> {
    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setWidth(w);
        config.setHeight(h);
        config.setPixelFormat(u32::from_be_bytes(*b"BGRA"));
        config.setMinimumFrameInterval(objc2_core_media::CMTime::new(1, 60));
        config.setShowsCursor(true);
        config.setQueueDepth(3);
    }
    config
}

/// A static screen may never send images. A healthy idle stream must still answer a
/// configuration command; a broken replayd connection errors or misses this deadline.
fn probe_stream(stream: &SCStream, w: usize, h: usize, health: &Health) -> Result<(), String> {
    check_intent(&USER_STOPPED)?;
    let before = health.lock().unwrap().frames;
    let (tx, rx) = std::sync::mpsc::channel();
    let done = RcBlock::new(move |error: *mut NSError| {
        let _ = tx.send(unsafe { error.as_ref() }.map(capture_error));
    });
    let config = stream_configuration(w, h);
    unsafe { stream.updateConfiguration_completionHandler(&config, Some(&done)); }
    let result = wait_capture(&rx).and_then(|error| start_result(Ok(error)));
    // A real image arriving during a delayed command is stronger evidence than its timeout.
    let health = health.lock().unwrap();
    if health.user_stopped { Err(CAPTURE_STOPPED.into()) }
    else if health.error.is_none() && health.frames != before { Ok(()) } else { result }
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use std::cell::Cell;
    struct FakeCapture { frame: u8 }
    fn step<T>(watch: &mut Watchdog, now: Instant, health: &StreamHealth,
        probe: impl FnOnce() -> Result<(), String>, restart: impl FnOnce() -> Result<T, String>) -> Result<Option<T>, String> {
        watch.step(now, health, probe, |_| restart().map(Some), || now)
    }

    #[test]
    fn watchdog_healthy_idle_only_probes_and_recent_frames_skip_probe() {
        let at = Instant::now();
        let mut watch = Watchdog::new(at);
        let mut health = StreamHealth { last_frame: at, frames: 1, user_stopped: false, error: None };
        let probes = Cell::new(0);
        for n in 1..=60 {
            let result = step::<FakeCapture>(&mut watch, at + Duration::from_secs(n * 8), &health,
                || { probes.set(probes.get()+1); Ok(()) }, || panic!("static screen restarted"));
            assert!(result.unwrap().is_none());
        }
        assert_eq!(probes.get(), 60);
        health.last_frame = at + Duration::from_secs(488);
        assert!(step::<FakeCapture>(&mut watch, health.last_frame, &health,
            || panic!("recent healthy frame was probed"), || panic!("healthy stream restarted")).unwrap().is_none());
    }

    #[test]
    fn watchdog_delegate_error_and_unanswered_probe_recover_with_new_frame() {
        let at = Instant::now();
        let mut watch = Watchdog::new(at);
        let mut health = StreamHealth { last_frame: at, frames: 1, user_stopped: false, error: Some("fake stream stopped".into()) };
        let recovered = step(&mut watch, at, &health, || panic!("error must restart directly"),
            || Ok(FakeCapture { frame: 2 })).unwrap().unwrap();
        assert_eq!(recovered.frame, 2);
        health.error = None;
        assert!(step::<FakeCapture>(&mut watch, at + QUIET_PROBE, &health, || Err("fake command timeout".into()),
            || panic!("single probe failure restarted the stream")).unwrap().is_none());
        let recovered = step(&mut watch, at + QUIET_PROBE * 2, &health, || Err("fake command timeout".into()),
            || Ok(FakeCapture { frame: 3 })).unwrap().unwrap();
        assert_eq!(recovered.frame, 3);
        assert!(step::<FakeCapture>(&mut watch, at + QUIET_PROBE * 2 + Duration::from_millis(1), &health,
            || panic!("probe not due"), || panic!("recovery repeated")).unwrap().is_none());
    }

    #[test]
    fn watchdog_probe_streak_resets_on_success_or_actual_frames() {
        let at = Instant::now();
        let mut watch = Watchdog::new(at);
        let mut health = StreamHealth { last_frame: at, frames: 1, error: None, user_stopped: false };
        for (n, ok) in [(1,false),(2,true),(3,false)] {
            assert!(step::<FakeCapture>(&mut watch, at + QUIET_PROBE * n, &health,
                || if ok { Ok(()) } else { Err("one delayed probe".into()) },
                || panic!("nonconsecutive probe failures restarted capture")).unwrap().is_none());
        }
        health.frames += 1;
        health.last_frame = at + QUIET_PROBE * 4;
        assert!(step::<FakeCapture>(&mut watch, health.last_frame, &health,
            || panic!("fresh frame probed"), || panic!("fresh frame restarted")).unwrap().is_none());
        assert_eq!(watch.probe_failures,0);
        assert!(watch.reason.is_none());
    }

    #[test]
    fn watchdog_probe_candidate_failure_keeps_old_until_successful_adoption() {
        struct FakeStream { running: Arc<AtomicBool>, lock_was_free: Arc<AtomicBool>, owner: Option<std::sync::Weak<Mutex<FakeStream>>> }
        impl Drop for FakeStream {
            fn drop(&mut self) {
                self.running.store(false,Ordering::Release);
                if let Some(owner) = self.owner.as_ref().and_then(|w|w.upgrade()) {
                    self.lock_was_free.store(owner.try_lock().is_ok(),Ordering::Release);
                }
            }
        }
        let at=Instant::now(); let mut watch=Watchdog::new(at);
        let health=StreamHealth {last_frame:at,frames:1,error:None,user_stopped:false};
        let old_running=Arc::new(AtomicBool::new(true)); let unlocked=Arc::new(AtomicBool::new(false));
        let old=Arc::new(Mutex::new(FakeStream {running:old_running.clone(),lock_was_free:unlocked.clone(),owner:None}));
        let weak=Arc::downgrade(&old); old.lock().unwrap().owner=Some(weak.clone());
        assert!(watch.step::<FakeStream>(at+QUIET_PROBE,&health,||Err("timeout".into()),
            |_|panic!("first probe failure started candidate"),||at+QUIET_PROBE).unwrap().is_none());
        assert!(watch.step::<FakeStream>(at+QUIET_PROBE*2,&health,||Err("timeout".into()),
            |stop_first|{assert!(!stop_first);assert!(old_running.load(Ordering::Acquire));Err("candidate has no first frame".into())},
            ||at+QUIET_PROBE*2).unwrap().is_none());
        assert!(old_running.load(Ordering::Acquire));
        let new_running=Arc::new(AtomicBool::new(true));
        let now=at+QUIET_PROBE*2+RECOVERY_BACKOFF;
        let candidate=watch.step(now,&health,||unreachable!(),|stop_first| {
            assert!(!stop_first); assert!(old_running.load(Ordering::Acquire));
            Ok(Some(FakeStream {running:new_running.clone(),lock_was_free:Arc::default(),owner:None}))
        },||now).unwrap().unwrap();
        assert!(old_running.load(Ordering::Acquire)); // A usable candidate alone does not stop old capture.
        assert!(adopt_capture(&weak,candidate,|_,_|{}));
        assert!(!old_running.load(Ordering::Acquire));
        assert!(unlocked.load(Ordering::Acquire));
        drop(old);
        assert!(!new_running.load(Ordering::Acquire));
        let abandoned=Arc::new(AtomicBool::new(true));
        assert!(!adopt_capture(&weak,FakeStream{running:abandoned.clone(),lock_was_free:Arc::default(),owner:None},|_,_|panic!("departed viewer adopted")));
        assert!(!abandoned.load(Ordering::Acquire));
    }

    #[test]
    fn watchdog_delegate_failure_may_stop_first_and_resumed_old_capture_cancels_retry() {
        let at=Instant::now(); let mut watch=Watchdog::new(at);
        let mut health=StreamHealth{last_frame:at,frames:1,error:Some("delegate stopped".into()),user_stopped:false};
        assert!(watch.step::<FakeCapture>(at,&health,||unreachable!(),|stop_first|{assert!(stop_first);Ok(None)},||at).unwrap().is_none());
        health.error=None;
        assert!(watch.step::<FakeCapture>(at+QUIET_PROBE,&health,||Err("timeout".into()),|_|unreachable!(),||at+QUIET_PROBE).unwrap().is_none());
        assert!(watch.step::<FakeCapture>(at+QUIET_PROBE*2,&health,||Err("timeout".into()),|stop_first|{assert!(!stop_first);Ok(None)},||at+QUIET_PROBE*2).unwrap().is_none());
        assert!(watch.reason.is_none()); assert_eq!(watch.failures,0);
    }

    #[test]
    fn watchdog_backoff_and_two_failed_restarts_become_terminal() {
        let at = Instant::now();
        let mut watch = Watchdog::new(at);
        let health = StreamHealth { last_frame: at, frames: 1, user_stopped: false, error: Some("fake stopped".into()) };
        assert!(step::<FakeCapture>(&mut watch, at, &health, || unreachable!(), || Err("no first frame".into())).unwrap().is_none());
        assert!(step::<FakeCapture>(&mut watch, at + RECOVERY_BACKOFF / 2, &health,
            || unreachable!(), || panic!("retry ignored backoff")).unwrap().is_none());
        assert_eq!(step::<FakeCapture>(&mut watch, at + RECOVERY_BACKOFF, &health,
            || unreachable!(), || Err("fake restart timed out".into())).err().as_deref(), Some(CAPTURE_LOST));
    }

    #[test]
    fn watchdog_slow_restart_backoff_starts_after_completion() {
        let at = Instant::now();
        let finished = at + Duration::from_secs(14);
        let mut watch = Watchdog::new(at);
        let health = StreamHealth { last_frame: at, frames: 1, user_stopped: false, error: Some("fake stopped".into()) };
        assert!(watch.step::<FakeCapture>(at, &health, || unreachable!(),
            |_| Err("slow fake restart".into()), || finished).unwrap().is_none());
        assert_eq!(watch.retry_at, finished + RECOVERY_BACKOFF);
        assert!(step::<FakeCapture>(&mut watch, finished + RECOVERY_BACKOFF / 2, &health,
            || unreachable!(), || panic!("retry ran before post-failure backoff")).unwrap().is_none());
    }

    #[test]
    fn watchdog_never_restarts_a_user_stopped_or_declined_capture() {
        assert!(intentional_stop(true, SCStreamErrorCode::UserStopped.0));
        assert!(intentional_stop(true, SCStreamErrorCode::UserDeclined.0));
        assert!(!intentional_stop(false, SCStreamErrorCode::UserStopped.0));
        assert!(!intentional_stop(true, SCStreamErrorCode::InternalError.0));
        let at = Instant::now();
        let mut watch = Watchdog::new(at);
        let health = StreamHealth { last_frame: at, frames: 1, error: None, user_stopped: true };
        assert_eq!(step::<FakeCapture>(&mut watch, at, &health,
            || panic!("user stop was probed"), || panic!("user stop was overridden")).err().as_deref(),Some(CAPTURE_STOPPED));
        let health = StreamHealth { user_stopped: false, error: Some("stream error".into()), ..health };
        assert_eq!(step::<FakeCapture>(&mut watch, at, &health, || unreachable!(),
            || Err(CAPTURE_STOPPED.into())).err().as_deref(),Some(CAPTURE_STOPPED));
    }

    #[test]
    fn watchdog_late_old_callback_latches_intent_after_replacement_or_last_viewer() {
        let intent = AtomicBool::new(false);
        assert!(check_intent(&intent).is_ok());
        // Model the callback retaining only application intent, after private old health is gone.
        let late_callback = || record_capture_error(&intent, true, SCStreamErrorCode::UserStopped.0, "stopped".into());
        let _replacement = FakeCapture { frame: 2 };
        assert_eq!(late_callback(), CAPTURE_STOPPED);
        assert_eq!(check_intent(&intent), Err(CAPTURE_STOPPED.into()));
        let _ = record_capture_error(&intent, true, SCStreamErrorCode::InternalError.0, "late other error".into());
        assert!(check_intent(&intent).is_err(), "a later callback erased user intent");
    }

    #[test]
    fn watchdog_start_completion_refusal_is_classified_and_never_retried() {
        let intent = AtomicBool::new(false);
        let stopped = record_capture_error(&intent, true, SCStreamErrorCode::UserDeclined.0, "declined".into());
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Some(stopped)).unwrap();
        let mut tries = 0;
        let result = retry_start::<()>(|| {
            tries += 1;
            start_result(Ok(wait_capture(&rx)?))
        });
        assert_eq!(result, Err(CAPTURE_STOPPED.into()));
        assert_eq!(tries, 1);
        assert!(check_intent(&intent).is_err());
    }

    #[test]
    fn watchdog_flapping_is_bounded_and_terminal_capture_check_rejects_stale_screen() {
        let at = Instant::now();
        let mut watch = Watchdog::new(at);
        let health = StreamHealth { last_frame: at, frames: 1, user_stopped: false, error: Some("repeated stream error".into()) };
        for n in 0..3 {
            assert!(step(&mut watch, at + Duration::from_secs(n), &health, || unreachable!(),
                || Ok(FakeCapture { frame: n as u8 })).unwrap().is_some());
        }
        assert!(step::<FakeCapture>(&mut watch, at + Duration::from_secs(3), &health,
            || unreachable!(), || panic!("unbounded restart loop")).is_err());
        let mut capture = fixed_capture(16,16,vec![7;256]);
        let held = capture.snapshot();
        capture.failed = Some(CAPTURE_LOST.into());
        assert_eq!(capture.check().unwrap_err().to_string(), CAPTURE_LOST);
        assert_eq!(held.pixels[0],7); // Old immutable pictures survive until their viewer drops them.
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use objc2_core_video::{CVPixelBufferCreate, kCVPixelFormatType_32BGRA};
    use std::ptr::{null_mut, NonNull};

    #[test]
    fn retina_geometry_preserves_aspect_and_input_coordinates() {
        assert_eq!(backing_dimensions((1512, 982), (1512, 982), (3024, 1964)), (3024, 1964));
        assert_eq!(backing_dimensions((1692, 3008), (3008, 1692), (6016, 3384)), (3384, 6016));
        assert_eq!(backing_dimensions((1512, 982), (0, 0), (0, 0)), (1512, 982));
        assert_eq!(capture_dimensions((1512, 982), (3024, 1964), false), (1512, 982));
        assert_eq!(capture_dimensions((1512, 982), (3024, 1964), true), (3024, 1964));
        assert_eq!(capture_dimensions((3008, 1692), (6016, 3384), true), (4096, 2304));
        assert_eq!(capture_dimensions((1692, 3008), (3384, 6016), true), (2304, 4096));
        assert_eq!(point_from_frame(1512, 982, (3024, 1964), (1512., 982.)), (756., 491.));
        assert_eq!(point_from_frame(2048, 1152, (4096, 2304), (3008., 1692.)), (1504., 846.));
        assert_eq!(point_from_frame(756, 491, (1512, 982), (1512., 982.)), (756., 491.));
        let (x, y) = point_from_frame(u16::MAX, u16::MAX, (3024, 1964), (1512., 982.));
        assert!(x < 1512. && y < 982.);
    }

    fn buffer(w: usize, h: usize, color: u32) -> CFRetained<CVPixelBuffer> {
        let mut raw = null_mut();
        let status = unsafe { CVPixelBufferCreate(None, w, h, kCVPixelFormatType_32BGRA, None, NonNull::from(&mut raw)) };
        assert_eq!(status, 0);
        let pb = unsafe { CFRetained::from_raw(NonNull::new(raw).unwrap()) };
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags(0)), 0);
            let base = CVPixelBufferGetBaseAddress(&pb) as *mut u8;
            let stride = CVPixelBufferGetBytesPerRow(&pb);
            for y in 0..h {
                std::slice::from_raw_parts_mut(base.add(y * stride) as *mut u32, w).fill(color);
            }
            assert_eq!(CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags(0)), 0);
        }
        pb
    }

    fn capture() -> Capture {
        Capture { x: 0, y: 0, w: 16, h: 16, native_pixels: false, buf: Arc::new(vec![0; 256]), native: None,
            latest: Arc::new(Mutex::new(None)), health: Arc::new(Mutex::new(StreamHealth::new())), failed: None, generation: 0, stream: None, _output: None }
    }

    #[test]
    fn held_snapshot_survives_next_capture_and_capture_drop() {
        let mut c = capture();
        *c.latest.lock().unwrap() = Some((buffer(16, 16, 0xff123456), None));
        let first = c.snapshot();
        let unchanged = c.snapshot();
        assert!(Arc::ptr_eq(&first.pixels, &unchanged.pixels));
        assert!(!c.fresh_since(first.generation));
        *c.latest.lock().unwrap() = Some((buffer(16, 16, 0xff654321), None));
        assert!(c.fresh_since(first.generation));
        let next = c.snapshot();
        assert_ne!(first.generation, next.generation);
        assert!(c.fresh_since(first.generation)); // A second viewer still sees the change.
        assert!(!c.fresh_since(next.generation));
        assert!(first.pixels.iter().all(|p| *p == 0x123456));
        assert!(next.pixels.iter().all(|p| *p == 0x654321));
        drop(c);
        let pb = first.native.as_ref().unwrap();
        unsafe {
            assert_eq!(CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly), 0);
            assert_eq!(*(CVPixelBufferGetBaseAddress(pb) as *const u32), 0xff123456);
            CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
        }
    }

    #[test]
    fn mismatched_buffer_uses_cpu_fallback() {
        let mut c = capture();
        *c.latest.lock().unwrap() = Some((buffer(20, 20, 0xffabcdef), None));
        let s = c.snapshot();
        assert!(s.native.is_none());
        assert_eq!((s.w, s.h), (16, 16));
        assert!(s.pixels.iter().all(|p| *p == 0xabcdef));
    }
}

/// Start a ScreenCaptureKit stream of the main display at w×h, BGRA, ≤60 fps.
fn start_stream(w: usize, h: usize) -> Result<(Retained<SCStream>, Retained<Output>, Latest, Health), String> {
    check_intent(&USER_STOPPED)?;
    capture_support().map_err(str::to_owned)?;
    if !CGPreflightScreenCaptureAccess() {
        return Err("Screen Recording permission not granted".into());
    }
    retry_start(|| {
        // A timed-out attempt can still deliver a late frame. Its private slot must
        // never satisfy the next attempt's first-frame check.
        let latest: Latest = Arc::new(Mutex::new(None));
        let health = Arc::new(Mutex::new(StreamHealth::new()));
        start_stream_once(w, h, latest.clone(), health.clone()).map(|(stream, output)| (stream, output, latest, health))
    })
}

// Two attempts, at most 3 s for content + 3 s for start each, then 2 s for a frame:
// stay below the RFB client's 20 s read timeout so it receives the fallback reason.
fn retry_start<T>(mut create: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    let mut last = String::new();
    for _ in 0..2 {
        check_intent(&USER_STOPPED)?;
        match create() { Ok(v) => return Ok(v), Err(e) if e == CAPTURE_STOPPED => return Err(e), Err(e) => last = e }
    }
    Err(last)
}

fn start_stream_once(w: usize, h: usize, latest: Latest, health: Health) -> Result<(Retained<SCStream>, Retained<Output>), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Retained<SCShareableContent>, String>>();
    let handler = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        let r = if let Some(error) = unsafe { error.as_ref() } { Err(capture_error(error)) }
            else { unsafe { content.as_ref() }.map(|c| c.retain()).ok_or_else(|| "no shareable content".into()) };
        let _ = tx.send(r);
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let content = wait_capture(&rx)??;
    check_intent(&USER_STOPPED)?;
    let main = CGMainDisplayID();
    let displays = unsafe { content.displays() };
    let display = displays.iter().find(|d| unsafe { d.displayID() } == main).or_else(|| displays.iter().next()).ok_or("no display")?;
    unsafe {
        let (filter, found) = content_filter(&content, &display);
        let config = stream_configuration(w, h);
        let output = Output::new(latest, health);
        let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, Some(ProtocolObject::from_ref(&*output)));
        let queue = DispatchQueue::new("itda.capture", None);
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(ProtocolObject::from_ref(&*output), SCStreamOutputType::Screen, Some(&queue))
            .map_err(|e| capture_error(&e))?;
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        let late = objc2::rc::Weak::new(&*stream);
        let done = RcBlock::new(move |error: *mut NSError| {
            if tx.send(error.as_ref().map(capture_error)).is_err() {
                // No waiter means startup was abandoned. Do not leave a stream
                // running when a delayed start callback finally arrives.
                if let Some(stream) = late.load() { stream.stopCaptureWithCompletionHandler(None); }
            }
        });
        stream.startCaptureWithCompletionHandler(Some(&done));
        match wait_capture(&rx).and_then(|error| start_result(Ok(error))) {
            Ok(()) => {}
            Err(e) => {
                // Close before requesting stop: callbacks after this point must take
                // their own cleanup path, even if start completes after stop was queued.
                drop(rx);
                stream.stopCaptureWithCompletionHandler(None);
                return Err(e);
            }
        }
        if !found {
            // (A curtain window made while this stream started: leave it out now.)
            let stream = SendStream(stream.clone());
            std::thread::spawn(move || {
                let stream = stream;
                for _ in 0..5 {
                    if update_filter(&stream.0) { break }
                    std::thread::sleep(Duration::from_millis(500));
                }
            });
        }
        Ok((stream, output))
    }
}

/// Windows the capture leaves out (the curtain's), by window number.
static EXCLUDED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// The display's filter, and whether every window to leave out was found.
fn content_filter(content: &SCShareableContent, display: &SCDisplay) -> (Retained<SCContentFilter>, bool) {
    let excluded = EXCLUDED.lock().unwrap().clone();
    let windows: Vec<Retained<SCWindow>> = unsafe { content.windows() }.iter().filter(|w| excluded.contains(&unsafe { w.windowID() })).collect();
    let filter = unsafe { SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), display, &NSArray::from_retained_slice(&windows)) };
    (filter, windows.len() == excluded.len())
}

/// Leave these windows out of the capture, from the running stream's next frame on (and of
/// any stream started later). False if the running stream couldn't be updated.
pub(crate) fn exclude_windows(ids: &[u32]) -> bool {
    *EXCLUDED.lock().unwrap() = ids.to_vec();
    let Some(capture) = SHARED_CAPTURE.lock().unwrap().upgrade() else { return true };
    let Some(stream) = capture.lock().unwrap().stream.clone() else { return true };
    drop(capture);
    update_filter(&stream)
}

struct SendStream(Retained<SCStream>);
// (ScreenCaptureKit's stream may be updated from any thread.)
unsafe impl Send for SendStream {}

/// Give `stream` the filter leaving out the windows in EXCLUDED.
fn update_filter(stream: &SCStream) -> bool {
    let (tx, rx) = std::sync::mpsc::channel::<Option<Retained<SCShareableContent>>>();
    let handler = RcBlock::new(move |content: *mut SCShareableContent, _: *mut NSError| {
        let _ = tx.send(unsafe { content.as_ref() }.map(|c| c.retain()));
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let Ok(Some(content)) = rx.recv_timeout(START_WAIT) else {
        crate::log!("hp: no shareable content to update the capture");
        return false;
    };
    let main = CGMainDisplayID();
    let displays = unsafe { content.displays() };
    let Some(display) = displays.iter().find(|d| unsafe { d.displayID() } == main).or_else(|| displays.iter().next()) else { return false };
    let (filter, found) = content_filter(&content, &display);
    if !found {
        crate::log!("hp: windows to leave out of the capture aren't listed yet");
        return false;
    }
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let done = RcBlock::new(move |error: *mut NSError| {
        let _ = tx.send(unsafe { error.as_ref() }.map(capture_error));
    });
    unsafe { stream.updateContentFilter_completionHandler(&filter, Some(&done)) };
    match rx.recv_timeout(START_WAIT) {
        Ok(None) => true,
        Ok(Some(e)) => { crate::log!("hp: couldn't update the capture: {e}"); false }
        Err(_) => { crate::log!("hp: the capture update didn't answer"); false }
    }
}

/// Whether this Mac's screen is locked now.
pub(crate) fn screen_locked() -> bool {
    session_state().is_some_and(|(locked, _)| locked)
}

/// Poll intent as well as the bounded framework deadline; a user stop cancels pending startup.
fn wait_capture<T>(rx: &std::sync::mpsc::Receiver<T>) -> Result<T, String> {
    let until = Instant::now() + START_WAIT;
    loop {
        check_intent(&USER_STOPPED)?;
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() { return Err("ScreenCaptureKit didn't answer".into()); }
        match rx.recv_timeout(left.min(Duration::from_millis(50))) {
            Ok(value) => { check_intent(&USER_STOPPED)?; return Ok(value); }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
            Err(_) => return Err("ScreenCaptureKit callback disconnected".into()),
        }
    }
}

fn start_result(result: Result<Option<String>, std::sync::mpsc::RecvTimeoutError>) -> Result<(), String> {
    match result {
        Ok(None) => Ok(()),
        Ok(Some(e)) => Err(e),
        Err(_) => Err("ScreenCaptureKit이 화면 캡처 시작에 응답하지 않았습니다".into()),
    }
}

#[cfg(test)]
mod startup_tests {
    #[test]
    fn startup_retries_once_and_stops_on_success() {
        let mut tries = 0;
        assert_eq!(super::retry_start(|| { tries += 1; if tries == 1 { Err("timeout".into()) } else { Ok(7) } }), Ok(7));
        assert_eq!(tries, 2);
        tries = 0;
        assert_eq!(super::retry_start::<()>(|| { tries += 1; Err(format!("failure {tries}")) }), Err("failure 2".into()));
        assert_eq!(tries, 2);
        tries = 0;
        assert_eq!(super::retry_start(|| { tries += 1; Ok(9) }), Ok(9));
        assert_eq!(tries, 1);
    }

    #[test]
    fn only_a_successful_start_callback_is_ready() {
        use std::sync::mpsc::RecvTimeoutError::{Timeout, Disconnected};
        assert!(super::start_result(Ok(None)).is_ok());
        assert_eq!(super::start_result(Ok(Some("capture failed".into()))), Err("capture failed".into()));
        assert!(super::start_result(Err(Timeout)).is_err());
        assert!(super::start_result(Err(Disconnected)).is_err());
    }
}

// ---------------------------------------------------------------- input

pub(crate) struct Input {
    // A connection holds the same shared capture for its whole life. Do not take
    // the capture mutex on every mouse event while a large snapshot is copying.
    frame_size: (usize, usize),
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

/// The flags a real keyboard puts on these keys: arrows are keypad + Fn keys, function and
/// navigation keys Fn keys. macOS's own shortcuts match them with these flags (Mission
/// Control is ⌃↑ *with* the arrow's flags), so without them ⌃↑, ⌃←, F11… do nothing.
fn key_flags(code: u16) -> CGEventFlags {
    match code {
        123..=126 => CGEventFlags(CGEventFlags::MaskNumericPad.0 | CGEventFlags::MaskSecondaryFn.0),
        96..=101 | 103 | 105..=107 | 109 | 111 | 113 | 115..=122 => CGEventFlags::MaskSecondaryFn,
        _ => CGEventFlags(0),
    }
}

/// The viewer's Mac screen actions (Mission Control…) arrive as these keysyms plus the action's
/// macOS symbolic hotkey number: no keyboard sends them, and old servers ignore them.
const ACTION_KEYSYM: u32 = 0x1100_0000;

/// The shortcut this Mac's user has for a symbolic hotkey (System Settings › Keyboard ›
/// Shortcuts), or the default one: (key code, modifier flags), None when switched off.
fn hotkey(id: u32) -> Option<(u16, u64)> {
    let default = match id {
        32 => (126, 0x84_0000), // Mission Control: ⌃↑
        33 => (125, 0x84_0000), // Application windows: ⌃↓
        36 => (103, 0x80_0000), // Show Desktop: F11
        79 => (123, 0x84_0000), // Move left a space: ⌃←
        81 => (124, 0x84_0000), // Move right a space: ⌃→
        64 => (49, 0x10_0000),  // Spotlight: ⌘Space
        _ => return None,
    };
    let export = std::process::Command::new("/usr/bin/defaults").args(["export", "com.apple.symbolichotkeys", "-"]).output().ok()?;
    let mut json = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let _ = std::io::Write::write_all(&mut json.stdin.take()?, &export.stdout);
    let v: serde_json::Value = serde_json::from_slice(&json.wait_with_output().ok()?.stdout).unwrap_or_default();
    hotkey_in(&v, id, default)
}

fn hotkey_in(v: &serde_json::Value, id: u32, default: (u16, u64)) -> Option<(u16, u64)> {
    let entry = &v["AppleSymbolicHotKeys"][id.to_string()];
    if entry.is_null() {
        return Some(default); // never changed
    }
    if entry["enabled"] == false || entry["enabled"] == 0 {
        return None;
    }
    let p = &entry["value"]["parameters"];
    Some((p[1].as_u64().filter(|c| *c < 0xffff)? as u16, p[2].as_u64()?))
}

/// Run a screen action with this Mac's own shortcut for it.
fn screen_action(id: u32) {
    let Some((code, mods)) = hotkey(id) else {
        crate::log!("hp: screen action {id} has no shortcut on this Mac");
        return;
    };
    // The modifiers go down first and come up last, as on a keyboard.
    let held: Vec<(u16, u64)> = [(59, 0x4_0000), (58, 0x8_0000), (56, 0x2_0000), (55, 0x10_0000)].into_iter().filter(|(_, f)| mods & f != 0).collect();
    let mut flags = 0;
    for (k, f) in &held {
        flags |= f;
        let e = CGEvent::new_keyboard_event(None, *k, true);
        CGEvent::set_flags(e.as_deref(), CGEventFlags(flags));
        post(e);
    }
    for down in [true, false] {
        let e = CGEvent::new_keyboard_event(None, code, down);
        CGEvent::set_flags(e.as_deref(), CGEventFlags(mods | key_flags(code).0));
        post(e);
    }
    for (k, f) in held.iter().rev() {
        flags &= !f;
        let e = CGEvent::new_keyboard_event(None, *k, false);
        CGEvent::set_flags(e.as_deref(), CGEventFlags(flags));
        post(e);
    }
}

fn post(e: Option<CFRetained<CGEvent>>) {
    // (Marked as the viewer's: it passes the curtain, which stops this Mac's own input.)
    CGEvent::set_integer_value_field(e.as_deref(), CGEventField::EventSourceUserData, crate::curtain_mac::VIEWER_EVENT);
    CGEvent::post(CGEventTapLocation::HIDEventTap, e.as_deref());
}

impl Input {
    pub(crate) fn new(_cmd_as_ctrl: bool, view_only: bool) -> Input {
        Input { frame_size: frame_dimensions(), view_only, buttons: 0, down: HashMap::new(), flags: CGEventFlags(0), last_key: Instant::now(), last_click: (Instant::now() - Duration::from_secs(5), 0, 0, 0), at: (0, 0) }
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
        if (ACTION_KEYSYM..ACTION_KEYSYM + 0x100).contains(&sym) {
            // One at a time, off the input thread (reading the shortcuts runs `defaults`); a
            // few queued at most, more are dropped.
            static ACTIONS: std::sync::OnceLock<std::sync::mpsc::SyncSender<u32>> = std::sync::OnceLock::new();
            if down {
                let tx = ACTIONS.get_or_init(|| {
                    let (tx, rx) = std::sync::mpsc::sync_channel::<u32>(4);
                    std::thread::spawn(move || rx.into_iter().for_each(screen_action));
                    tx
                });
                let _ = tx.try_send(sym - ACTION_KEYSYM);
            }
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
        CGEvent::set_flags(e.as_deref(), CGEventFlags(self.flags.0 | key_flags(code).0));
        post(e);
    }

    pub(crate) fn pointer(&mut self, mask: u8, x: u16, y: u16) {
        if self.view_only {
            return;
        }
        let b = CGDisplayBounds(CGMainDisplayID());
        let (px, py) = point_from_frame(x, y, self.frame_size, (b.size.width, b.size.height));
        let at = CGPoint { x: b.origin.x + px, y: b.origin.y + py };
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
    capture_support().is_ok() && crate::store::dir().join("hp-sharing").exists()
}

pub fn set_enabled(on: bool) {
    let path = crate::store::dir().join("hp-sharing");
    let _ = if on { std::fs::write(&path, b"1") } else { std::fs::remove_file(&path) };
}

/// Where the background server reports its permissions, for an Itda setting it up over SSH:
/// `screen=0|1 access=0|1 port=N`.
fn write_status(screen: bool, access: bool) {
    let current = active_capture();
    let (native, dimensions) = current.as_ref().map(|c| { let c = c.lock().unwrap(); (c.native_pixels, (c.w, c.h)) })
        .unwrap_or_else(|| { let native = native_pixels_requested(); (native, display_dimensions(native)) });
    let quality = if native { "native" } else { "points" };
    let _ = std::fs::write(crate::store::dir().join("hp-status"), format!("screen={} access={} port={PORT} quality={quality} quality_active={} capture_width={} capture_height={} actions=1 curtain={} curtain_error={}\n", screen as u8, access as u8, current.is_some() as u8, dimensions.0, dimensions.1, crate::curtain_mac::state().0 as u8, crate::curtain_mac::state().1));
}

/// Start serving in the background of the running app (when `enabled`).
pub fn start_background() {
    if let Err(reason) = capture_support() { crate::log!("hp sharing: {reason}"); return; }
    std::thread::spawn(|| {
        let (screen, access) = permissions();
        crate::log!("hp sharing: screen recording {screen}, accessibility {access}");
        // Consent: it listens on this Mac only, so every viewer has already signed in to this
        // Mac's account over SSH and knows the 0600 password; no extra "allow?" dialog.
        let cfg = crate::server::Config { port: PORT, password: password(), ask: false, trust_near: false, cmd_as_ctrl: false, view_only: false, local_only: true };
        if let Err(e) = crate::server::start(cfg) {
            let _=std::fs::remove_file(crate::store::dir().join("hp-status"));
            crate::log!("hp sharing: {e}");
            return;
        }
        write_status(screen, access);
        crate::curtain_mac::start();
        // Rewritten every 2 s while it serves: an Itda setting this up over SSH waits for a
        // fresh one (also when sharing was already running), and sees permissions granted.
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(2));
            let (screen, access) = permissions();
            write_status(screen, access);
        });

    });
}

/// `itda --hp-server [port]`: serve this Mac until killed (asks for missing permissions).
pub fn serve_forever(port: u16) -> ! {
    if let Err(reason) = capture_support() { eprintln!("{reason}"); std::process::exit(1); }
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

#[cfg(test)]
mod session_tests {
    /// The session's lock and console state can be read (values depend on the machine).
    #[test]
    fn session_state_reads() {
        eprintln!("session (locked, on console) = {:?}", super::session_state());
    }
}

#[cfg(test)]
mod action_tests {
    /// A Mac with its own shortcuts (Mission Control on ⌃H), one switched off, one never changed.
    #[test]
    fn screen_action_uses_this_macs_shortcut() {
        let v: serde_json::Value = serde_json::from_str(r#"{"AppleSymbolicHotKeys":{
            "32":{"enabled":true,"value":{"parameters":[104,4,262144],"type":"standard"}},
            "36":{"enabled":false,"value":{"parameters":[65535,103,8388608],"type":"standard"}}}}"#).unwrap();
        assert_eq!(super::hotkey_in(&v, 32, (126, 0x84_0000)), Some((4, 0x4_0000)));
        assert_eq!(super::hotkey_in(&v, 36, (103, 0x80_0000)), None);
        assert_eq!(super::hotkey_in(&v, 33, (125, 0x84_0000)), Some((125, 0x84_0000)));
    }
}
