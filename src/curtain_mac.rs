//! The curtain for high-performance sharing: while a viewer asks for it, this Mac's screens
//! show "원격으로 사용 중" rather than what the viewer does, the keyboard and mouse here do
//! nothing, and when the last viewer leaves the screen locks. Viewers still see the desktop:
//! the curtain's windows are left out of the capture.
//!
//! The viewer's Itda asks over SSH, with the file `hp-curtain` (as with `hp-quality`); it is
//! removed when no viewer is left. Safety nets: a guard process locks the screen if this app
//! ends while the curtain is up (its windows and the input block end with the app), and
//! ⌃⌘Q here (the system's Lock Screen) takes the curtain down and locks, so a viewer that
//! vanished without its connection closing can't keep anyone out of this Mac.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSColor, NSFont, NSScreen, NSTextAlignment, NSTextField, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
use objc2_core_foundation::{kCFRunLoopCommonModes, CFMachPort, CFRetained, CFRunLoop};
use objc2_core_graphics::{CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Marks the events Itda posts for a viewer (kCGEventSourceUserData), which pass the curtain.
pub(crate) const VIEWER_EVENT: i64 = 0x4974_6461; // "Itda"

/// The guard's argument, and what it is told when the curtain comes down on purpose.
const GUARD_ARG: &str = "--curtain-guard";
const GUARD_OK: &[u8] = b"ok";
/// A viewer reconnecting (another quality, a network blip) within this keeps the curtain up
/// without locking.
const GRACE: Duration = Duration::from_secs(8);
/// Above the menu bar, the Dock and other apps' windows; below the lock screen.
const LEVEL: isize = 1000; // NSScreenSaverWindowLevel
/// ⌃⌘Q's key (Q).
const KEY_Q: i64 = 12;

static UP: AtomicBool = AtomicBool::new(false);
/// ⌃⌘Q was pressed on this Mac.
static ESCAPE: AtomicBool = AtomicBool::new(false);
/// After ⌃⌘Q here, requests are refused for this long (a viewer reconnecting doesn't cover
/// the Mac again on whoever is in front of it).
const REFUSE_AFTER_ESCAPE: Duration = Duration::from_secs(600);
/// Why the last request couldn't be met ("capture", "input"), for hp-status.
static FAILED: Mutex<&str> = Mutex::new("");
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
    /// Windows being replaced (the displays changed): kept until the new ones show.
    static RETIRING: RefCell<Vec<Retained<NSWindow>>> = const { RefCell::new(Vec::new()) };
}

/// A lock that a panic elsewhere doesn't make unusable: this code must always come down.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn flag() -> std::path::PathBuf {
    crate::store::dir().join("hp-curtain")
}

/// Whether the curtain is up now, and why the last request failed (reported in hp-status).
pub fn state() -> (bool, &'static str) {
    (UP.load(Ordering::Relaxed), *lock(&FAILED))
}

/// Follow the request while this app serves high-performance viewers.
pub fn start() {
    // (A request left by an app that ended is nobody's now.)
    let _ = std::fs::remove_file(flag());
    std::thread::spawn(|| {
        let mut escaped: Option<Instant> = None;
        let mut empty_since: Option<Instant> = None;
        let mut ticks = 0u32;
        loop {
            ticks += 1;
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(&mut empty_since, &mut escaped, ticks)));
            if r.is_err() {
                let _ = std::fs::remove_file(flag());
                if UP.load(Ordering::Relaxed) {
                    crate::log!("hp curtain: failed, taking it down");
                    let _ = std::panic::catch_unwind(|| down(true));
                }
                UP.store(false, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    });
}

fn tick(empty_since: &mut Option<Instant>, escaped: &mut Option<Instant>, ticks: u32) {
    if !UP.load(Ordering::Relaxed) && lock(&TAP).is_some() {
        // (An input block that started after its curtain had already failed.)
        unblock_input();
    }
    let viewers = crate::server::viewers().len();
    if viewers > 0 {
        *empty_since = None;
    } else {
        empty_since.get_or_insert_with(Instant::now);
    }
    let gone = empty_since.is_some_and(|t| t.elapsed() >= GRACE);
    if ESCAPE.swap(false, Ordering::Relaxed) {
        crate::log!("hp curtain: ⌃⌘Q on this Mac");
        let _ = std::fs::remove_file(flag());
        *escaped = Some(Instant::now());
        *lock(&FAILED) = "local";
        if UP.load(Ordering::Relaxed) {
            down(true);
        } else {
            unblock_input();
        }
        return;
    }
    if UP.load(Ordering::Relaxed) && crate::host_mac::exclusion_lost() {
        crate::log!("hp curtain: a restarted capture shows the curtain, locking");
        *lock(&FAILED) = "capture";
        let _ = std::fs::remove_file(flag());
        down(true);
        return;
    }
    let mut asked = flag().exists();
    if asked && escaped.is_some_and(|t| t.elapsed() < REFUSE_AFTER_ESCAPE) {
        let _ = std::fs::remove_file(flag());
        asked = false;
    }
    if asked && gone {
        // (A request outlives no session: the next viewer asks for itself.)
        let _ = std::fs::remove_file(flag());
    }
    let up = UP.load(Ordering::Relaxed);
    let want = asked && !gone;
    if want && !up {
        if viewers > 0 {
            if let Err(why) = up_now() {
                crate::log!("hp curtain: couldn't cover this Mac ({why}), taking it down");
                *lock(&FAILED) = why;
                let _ = std::fs::remove_file(flag());
                down(false);
            }
        }
    } else if !want && up {
        down(gone);
    } else if up && ticks.is_multiple_of(5) && screens_changed() {
        crate::log!("hp curtain: the displays changed, covering them again");
        if let Err(why) = cover() {
            crate::log!("hp curtain: couldn't cover the displays again ({why}), locking");
            *lock(&FAILED) = why;
            let _ = std::fs::remove_file(flag());
            down(true);
        }
    }
}

fn up_now() -> Result<(), &'static str> {
    crate::log!("hp curtain: up");
    *lock(&FAILED) = "";
    *lock(&GUARD) = std::env::current_exe().and_then(|exe| Command::new(exe).arg(GUARD_ARG).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()).map_err(|e| crate::log!("hp curtain: no guard: {e}")).ok();
    if !block_input() {
        return Err("input");
    }
    cover()?;
    // (Up once it covers: hp-status says so only then.)
    UP.store(true, Ordering::Relaxed);
    Ok(())
}

/// (Re)make the windows over every display: out of the capture first, then visible, so the
/// viewers never see them. Windows being replaced stay until the new ones show.
fn cover() -> Result<(), &'static str> {
    let (old, new) = on_main(Duration::from_secs(3), || {
        let old: Vec<u32> = WINDOWS.with(|w| {
            let mut w = w.borrow_mut();
            let ids = w.iter().map(|w| w.windowNumber() as u32).collect();
            RETIRING.with(|r| r.borrow_mut().extend(w.drain(..)));
            ids
        });
        (old, show_windows())
    })
    .unwrap_or_default();
    if new.is_empty() {
        return Err("capture");
    }
    let all: Vec<u32> = old.into_iter().chain(new).collect();
    // (A window just shown may take a moment to be listed.)
    if !(0..6).any(|i| {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(300));
        }
        crate::host_mac::exclude_windows(&all)
    }) {
        return Err("capture");
    }
    dispatch2::DispatchQueue::main().exec_async(|| {
        WINDOWS.with(|w| w.borrow().iter().for_each(|w| w.setAlphaValue(1.0)));
        RETIRING.with(|r| r.borrow_mut().drain(..).for_each(|w| w.close()));
    });
    Ok(())
}

/// Take the curtain down; `lock`: lock the screen first (the last viewer left).
fn down(lock_first: bool) {
    crate::log!("hp curtain: down{}", if lock_first { ", locking" } else { "" });
    // Down from here (hp-status says so, and a viewer that comes back asks again).
    UP.store(false, Ordering::Relaxed);
    if lock_first {
        lock_screen();
    }
    dispatch2::DispatchQueue::main().exec_async(close_windows);
    unblock_input();
    if let Some(mut guard) = lock(&GUARD).take() {
        if let Some(mut input) = guard.stdin.take() {
            let _ = input.write_all(GUARD_OK);
        }
        let _ = guard.wait();
    }
    crate::host_mac::exclude_windows(&[]);
}

/// Run `f` on the main thread and wait (up to `wait`) for its result.
fn on_main<T: Send + 'static>(wait: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    dispatch2::DispatchQueue::main().exec_async(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(wait).ok()
}

fn close_windows() {
    for list in [&WINDOWS, &RETIRING] {
        list.with(|w| w.borrow_mut().drain(..).for_each(|w| w.close()));
    }
}

/// Whether the displays no longer match the windows (one added, moved or resized).
fn screens_changed() -> bool {
    on_main(Duration::from_secs(2), || {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        let screens: Vec<NSRect> = NSScreen::screens(mtm).iter().map(|s| s.frame()).collect();
        WINDOWS.with(|w| {
            let w = w.borrow();
            w.len() != screens.len() || w.iter().zip(&screens).any(|(w, s)| w.frame() != *s)
        })
    })
    .unwrap_or(false)
}

/// A black window over each display, transparent to the mouse (a viewer's clicks reach the
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
            w.setLevel(LEVEL);
            w.setBackgroundColor(Some(&NSColor::blackColor()));
            w.setOpaque(true);
            w.setIgnoresMouseEvents(true);
            w.setHasShadow(false);
            w.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::FullScreenAuxiliary | NSWindowCollectionBehavior::IgnoresCycle);
            w.setAlphaValue(0.0);
            if let Some(view) = w.contentView() {
                for (text, size, white, y) in [("원격으로 사용 중", 28.0, 0.6, 10.0), ("이 Mac에서 ⌃⌘Q를 누르면 가림을 끝내고 화면을 잠급니다", 13.0, 0.4, -30.0)] {
                    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
                    label.setFont(Some(&NSFont::systemFontOfSize(size)));
                    label.setTextColor(Some(&NSColor::colorWithWhite_alpha(white, 1.0)));
                    label.setAlignment(NSTextAlignment::Center);
                    let box_size = NSSize::new(frame.size.width, size + 12.0);
                    label.setFrame(NSRect::new(NSPoint::new(0.0, frame.size.height / 2.0 + y), box_size));
                    view.addSubview(&label);
                }
            }
            w.orderFrontRegardless();
            ids.push(w.windowNumber() as u32);
            windows.push(w);
        }
    });
    ids
}

// ---------------------------------------------------------------- input

/// Drop this Mac's own keyboard, mouse, trackpad and tablet (everything not posted for a
/// viewer). False if the tap couldn't be made (no Accessibility permission).
fn block_input() -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let typed = [
            CGEventType::KeyDown, CGEventType::KeyUp, CGEventType::FlagsChanged,
            CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGEventType::RightMouseDown, CGEventType::RightMouseUp,
            CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGEventType::LeftMouseDragged, CGEventType::RightMouseDragged,
            CGEventType::OtherMouseDragged, CGEventType::MouseMoved, CGEventType::ScrollWheel,
            CGEventType::TabletPointer, CGEventType::TabletProximity,
        ];
        // Also system-defined (media keys) 14, and the trackpad's gestures: rotate 18, begin
        // and end 19–20, gesture 29, magnify 30, swipe 31, smart magnify 32, pressure 34.
        let mask = typed.iter().map(|t| t.0).chain([14, 18, 19, 20, 29, 30, 31, 32, 34]).fold(0u64, |m, t| m | 1 << t);
        let Some(port) = (unsafe { CGEvent::tap_create(CGEventTapLocation::SessionEventTap, CGEventTapPlacement::HeadInsertEventTap, CGEventTapOptions::Default, mask, Some(on_event), std::ptr::null_mut()) }) else {
            crate::log!("hp curtain: no event tap (Accessibility permission?)");
            let _ = tx.send(false);
            return;
        };
        let (Some(source), Some(run_loop)) = (CFMachPort::new_run_loop_source(None, Some(&port), 0), CFRunLoop::current()) else {
            CFMachPort::invalidate(&port);
            let _ = tx.send(false);
            return;
        };
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        *lock(&TAP_PORT) = Some(SendPort(port.clone()));
        *lock(&TAP) = Some(SendLoop(run_loop));
        let _ = tx.send(true);
        CFRunLoop::run();
        CFMachPort::invalidate(&port);
    });
    rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
}

fn unblock_input() {
    lock(&TAP_PORT).take();
    if let Some(run_loop) = lock(&TAP).take() {
        run_loop.0.stop();
    }
}

unsafe extern "C-unwind" fn on_event(_: CGEventTapProxy, kind: CGEventType, event: NonNull<CGEvent>, _: *mut std::ffi::c_void) -> *mut CGEvent {
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        if let Some(port) = lock(&TAP_PORT).as_ref() {
            CGEvent::tap_enable(&port.0, true);
        }
        return event.as_ptr();
    }
    let e = Some(unsafe { event.as_ref() });
    if CGEvent::integer_value_field(e, CGEventField::EventSourceUserData) == VIEWER_EVENT {
        return event.as_ptr();
    }
    let escape = CGEventFlags::MaskCommand | CGEventFlags::MaskControl;
    let held = CGEvent::flags(e) & (escape | CGEventFlags::MaskShift | CGEventFlags::MaskAlternate);
    if kind == CGEventType::KeyDown && CGEvent::integer_value_field(e, CGEventField::KeyboardEventKeycode) == KEY_Q && held == escape {
        ESCAPE.store(true, Ordering::Relaxed);
    }
    std::ptr::null_mut()
}

// ---------------------------------------------------------------- lock

/// Lock this Mac's screen now (as the menu's "Lock Screen"), waiting until it is.
pub fn lock_screen() {
    extern "C" {
        fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void;
        fn dlsym(handle: *mut std::ffi::c_void, name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    }
    let locked = |wait: Duration| {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if crate::host_mac::screen_locked() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    };
    unsafe {
        let login = dlopen(c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login".as_ptr(), 2);
        let sac = if login.is_null() { std::ptr::null_mut() } else { dlsym(login, c"SACLockScreenImmediate".as_ptr()) };
        if !sac.is_null() {
            let sac: unsafe extern "C" fn() -> i32 = std::mem::transmute(sac);
            if sac() == 0 && locked(Duration::from_secs(3)) {
                return;
            }
        }
    }
    crate::log!("hp curtain: locking with ⌃⌘Q");
    for down in [true, false] {
        let e = CGEvent::new_keyboard_event(None, KEY_Q as u16, down);
        CGEvent::set_flags(e.as_deref(), CGEventFlags::MaskCommand | CGEventFlags::MaskControl);
        CGEvent::set_integer_value_field(e.as_deref(), CGEventField::EventSourceUserData, VIEWER_EVENT);
        CGEvent::post(CGEventTapLocation::HIDEventTap, e.as_deref());
    }
    if locked(Duration::from_secs(3)) {
        return;
    }
    crate::log!("hp curtain: couldn't lock the screen, sleeping the display");
    let _ = Command::new("/usr/bin/pmset").arg("displaysleepnow").status();
}

/// `itda --curtain-guard`: lock the screen unless told the curtain came down on purpose
/// before the app that started it ends. Called before anything else starts.
pub fn guard_mode() -> bool {
    if std::env::args().nth(1).as_deref() != Some(GUARD_ARG) {
        return false;
    }
    extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
    }
    // Ended only by the app's end (an update's pkill of the app's processes included): then
    // it still locks.
    unsafe {
        signal(1, 1); // SIGHUP: ignored
        signal(15, 1); // SIGTERM: ignored
    }
    let mut said = Vec::new();
    let _ = std::io::stdin().take(16).read_to_end(&mut said);
    if said != GUARD_OK {
        crate::log!("hp curtain: the app ended with the curtain up, locking the screen");
        lock_screen();
    }
    true
}
