//! The curtain for high-performance sharing: while a viewer asks for it, this Mac's screens
//! show "원격으로 사용 중" rather than what the viewer does, the keyboard and mouse here do
//! nothing, and when the last viewer leaves the screen locks. Viewers still see the desktop:
//! the curtain's windows are left out of the capture.
//!
//! The viewer's Itda asks over SSH, with the file `hp-curtain` (as with `hp-quality`); it is
//! removed when no viewer is left. Safety net: a guard process locks the screen if this app
//! ends while the curtain is up (its windows and the input block end with the app).

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSColor, NSFont, NSScreen, NSTextAlignment, NSTextField, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
use objc2_core_foundation::{kCFRunLoopCommonModes, CFMachPort, CFRetained, CFRunLoop};
use objc2_core_graphics::{CGEvent, CGEventField, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType, CGShieldingWindowLevel};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Marks the events Itda posts for a viewer (kCGEventSourceUserData), which pass the curtain.
pub(crate) const VIEWER_EVENT: i64 = 0x4974_6461; // "Itda"

/// The guard's argument, and what it is told when the curtain comes down on purpose.
const GUARD_ARG: &str = "--curtain-guard";
const GUARD_OK: &[u8] = b"ok";

static UP: AtomicBool = AtomicBool::new(false);
static GUARD: Mutex<Option<Child>> = Mutex::new(None);
/// The input block's run loop (its thread runs until that stops).
static TAP: Mutex<Option<SendLoop>> = Mutex::new(None);
/// The tap itself, re-enabled when the system turns it off.
static TAP_PORT: Mutex<Option<SendPort>> = Mutex::new(None);

struct SendLoop(CFRetained<CFRunLoop>);
unsafe impl Send for SendLoop {}
struct SendPort(CFRetained<CFMachPort>);
unsafe impl Send for SendPort {}

thread_local! {
    /// The curtain's windows (main thread).
    static WINDOWS: RefCell<Vec<Retained<NSWindow>>> = const { RefCell::new(Vec::new()) };
}

fn flag() -> std::path::PathBuf {
    crate::store::dir().join("hp-curtain")
}

/// Whether the curtain is up now (reported in hp-status).
pub fn active() -> bool {
    UP.load(Ordering::Relaxed)
}

/// Follow the request while this app serves high-performance viewers.
pub fn start() {
    std::thread::spawn(|| loop {
        let viewers = crate::server::viewers().len();
        let asked = flag().exists();
        if asked && viewers == 0 {
            // (A request outlives no session: the next viewer asks for itself.)
            let _ = std::fs::remove_file(flag());
        }
        let want = asked && viewers > 0;
        if want != active() {
            if want { up() } else { down(viewers == 0) }
        }
        std::thread::sleep(Duration::from_millis(400));
    });
}

fn up() {
    crate::log!("hp curtain: up");
    UP.store(true, Ordering::Relaxed);
    *GUARD.lock().unwrap() = std::env::current_exe().and_then(|exe| Command::new(exe).arg(GUARD_ARG).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()).map_err(|e| crate::log!("hp curtain: no guard: {e}")).ok();
    block_input();
    let (tx, rx) = std::sync::mpsc::channel();
    dispatch2::DispatchQueue::main().exec_async(move || {
        let _ = tx.send(show_windows());
    });
    let ids = rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default();
    // Out of the capture first, then visible: the viewers never see the curtain.
    crate::host_mac::exclude_windows(&ids);
    dispatch2::DispatchQueue::main().exec_async(|| WINDOWS.with(|w| w.borrow().iter().for_each(|w| w.setAlphaValue(1.0))));
}

/// Take the curtain down; `lock` (the last viewer left): lock the screen before it shows.
fn down(lock: bool) {
    crate::log!("hp curtain: down{}", if lock { ", locking" } else { "" });
    if lock {
        lock_screen();
        std::thread::sleep(Duration::from_millis(800));
    }
    dispatch2::DispatchQueue::main().exec_async(|| WINDOWS.with(|w| w.borrow_mut().drain(..).for_each(|w| w.close())));
    unblock_input();
    if let Some(mut guard) = GUARD.lock().unwrap().take() {
        if let Some(mut input) = guard.stdin.take() {
            let _ = input.write_all(GUARD_OK);
        }
        let _ = guard.wait();
    }
    crate::host_mac::exclude_windows(&[]);
    UP.store(false, Ordering::Relaxed);
}

/// A black window over each screen, transparent to the mouse (a viewer's clicks reach the
/// apps beneath), shown with alpha 0 until the capture leaves it out. Returns their numbers.
fn show_windows() -> Vec<u32> {
    let Some(mtm) = MainThreadMarker::new() else { return Vec::new() };
    let mut ids = Vec::new();
    WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        for screen in NSScreen::screens(mtm).iter() {
            let frame = screen.frame();
            let w = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, NSWindowStyleMask::Borderless, NSBackingStoreType::Buffered, false) };
            unsafe { w.setReleasedWhenClosed(false) };
            w.setLevel(CGShieldingWindowLevel() as isize);
            w.setBackgroundColor(Some(&NSColor::blackColor()));
            w.setOpaque(true);
            w.setIgnoresMouseEvents(true);
            w.setHasShadow(false);
            w.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::FullScreenAuxiliary | NSWindowCollectionBehavior::IgnoresCycle);
            w.setAlphaValue(0.0);
            let label = NSTextField::labelWithString(&NSString::from_str("원격으로 사용 중"), mtm);
            label.setFont(Some(&NSFont::systemFontOfSize(28.0)));
            label.setTextColor(Some(&NSColor::colorWithWhite_alpha(0.6, 1.0)));
            label.setAlignment(NSTextAlignment::Center);
            let size = NSSize::new(frame.size.width, 40.0);
            label.setFrame(NSRect::new(NSPoint::new(0.0, (frame.size.height - size.height) / 2.0), size));
            if let Some(view) = w.contentView() {
                view.addSubview(&label);
            }
            w.orderFrontRegardless();
            ids.push(w.windowNumber() as u32);
            windows.push(w);
        }
    });
    ids
}

// ---------------------------------------------------------------- input

/// Drop this Mac's own keyboard and mouse (everything not posted for a viewer).
fn block_input() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mask = [
            CGEventType::KeyDown, CGEventType::KeyUp, CGEventType::FlagsChanged,
            CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGEventType::RightMouseDown, CGEventType::RightMouseUp,
            CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGEventType::LeftMouseDragged, CGEventType::RightMouseDragged,
            CGEventType::OtherMouseDragged, CGEventType::MouseMoved, CGEventType::ScrollWheel,
        ]
        .iter()
        .fold(1u64 << 14, |m, t| m | 1 << t.0); // (14: system-defined, such as media keys)
        let Some(port) = (unsafe { CGEvent::tap_create(CGEventTapLocation::SessionEventTap, CGEventTapPlacement::HeadInsertEventTap, CGEventTapOptions::Default, mask, Some(on_event), std::ptr::null_mut()) }) else {
            crate::log!("hp curtain: no event tap (accessibility?), this Mac's keyboard and mouse still work");
            let _ = tx.send(());
            return;
        };
        let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else {
            let _ = tx.send(());
            return;
        };
        let Some(run_loop) = CFRunLoop::current() else {
            let _ = tx.send(());
            return;
        };
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        *TAP_PORT.lock().unwrap() = Some(SendPort(port.clone()));
        *TAP.lock().unwrap() = Some(SendLoop(run_loop));
        let _ = tx.send(());
        CFRunLoop::run();
        CFMachPort::invalidate(&port);
    });
    let _ = rx.recv_timeout(Duration::from_secs(2));
}

fn unblock_input() {
    TAP_PORT.lock().unwrap().take();
    if let Some(run_loop) = TAP.lock().unwrap().take() {
        run_loop.0.stop();
    }
}

unsafe extern "C-unwind" fn on_event(_: CGEventTapProxy, kind: CGEventType, event: NonNull<CGEvent>, _: *mut std::ffi::c_void) -> *mut CGEvent {
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        if let Some(port) = TAP_PORT.lock().unwrap().as_ref() {
            CGEvent::tap_enable(&port.0, true);
        }
        return event.as_ptr();
    }
    if CGEvent::integer_value_field(Some(unsafe { event.as_ref() }), CGEventField::EventSourceUserData) == VIEWER_EVENT {
        event.as_ptr()
    } else {
        std::ptr::null_mut()
    }
}

// ---------------------------------------------------------------- lock

/// Lock this Mac's screen now (as the menu's "Lock Screen").
pub fn lock_screen() {
    extern "C" {
        fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void;
        fn dlsym(handle: *mut std::ffi::c_void, name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    }
    unsafe {
        let login = dlopen(c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login".as_ptr(), 2);
        let lock = if login.is_null() { std::ptr::null_mut() } else { dlsym(login, c"SACLockScreenImmediate".as_ptr()) };
        if !lock.is_null() {
            let lock: unsafe extern "C" fn() -> i32 = std::mem::transmute(lock);
            if lock() == 0 {
                return;
            }
        }
    }
    crate::log!("hp curtain: no SACLockScreenImmediate, sleeping the display");
    let _ = Command::new("/usr/bin/pmset").arg("displaysleepnow").status();
}

/// `itda --curtain-guard`: lock the screen unless told the curtain came down on purpose
/// before the app that started it ends. Called before anything else starts.
pub fn guard_mode() -> bool {
    if std::env::args().nth(1).as_deref() != Some(GUARD_ARG) {
        return false;
    }
    let mut said = Vec::new();
    let _ = std::io::stdin().take(16).read_to_end(&mut said);
    if said != GUARD_OK {
        crate::log!("hp curtain: the app ended with the curtain up, locking the screen");
        lock_screen();
    }
    true
}
