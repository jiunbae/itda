//! Permission guide owned by Itda itself: the same app macOS evaluates for capture/input.
//! User actions open settings or request one permission; polling only reads status.
use super::{Pane, Wake};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    path::PathBuf,
};
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event::WindowEvent,
    event_loop::EventLoopWindowTarget,
    window::{WindowBuilder, WindowId},
};
use wry::{http, WebViewBuilder, WebViewExtMacOS};
thread_local! { static PANE: RefCell<Option<Pane>> = const {RefCell::new(None)}; static REQUESTED: RefCell<Option<String>> = const {RefCell::new(None)}; static REQUEST_SEQ: Cell<u64> = const {Cell::new(0)}; }
thread_local! {
    static FRESH: RefCell<Option<Value>> = const { RefCell::new(None) };
    static CHECKING: Cell<bool> = const { Cell::new(false) };
    static CHECKED: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
}
const RESTART_HELPER: &str = "n=0; while kill -0 \"$1\" 2>/dev/null; do n=$((n+1)); [ \"$n\" -lt 150 ] || exit 1; sleep 0.1; done; /usr/bin/open -a \"$2\" itda://permissions";

/// Set when a permission is opened, cleared once both are granted: a relaunch in between
/// (macOS's "Quit & Reopen") brings the helper back to finish or to say it is done.
fn pending_marker() -> PathBuf {
    crate::store::dir().join("permission-setup-pending")
}
pub(super) fn pending() -> bool {
    pending_marker().exists()
}
/// Mid-setup relaunch: the permission still to do, so the guide carries on by itself.
pub(super) fn next_missing() -> Option<&'static str> {
    if !pending() { return None; }
    match crate::host_mac::permissions() {
        (false, _) => Some("screen"),
        (true, false) => Some("access"),
        _ => None,
    }
}

fn app_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension().is_some_and(|s| s == "app") && app.is_dir()).then(|| app.to_path_buf())
}
fn state() -> Value {
    let (screen, access) = crate::host_mac::permissions();
    let mut v = json!({"type":"permissions","screen":screen,"access":access,"app_path":app_path(),"version":crate::home::VERSION,"enabled":crate::host_mac::enabled(),"requested":REQUESTED.with(|r|r.borrow().clone()),"requested_seq":REQUEST_SEQ.with(Cell::get)});
    FRESH.with(|fresh| {
        if let Some(fresh) = fresh.borrow().as_ref() {
            v["configured_screen"] = fresh["screen"].clone();
            v["configured_access"] = fresh["access"].clone();
        }
    });
    v
}
fn reply(v: Value) {
    PANE.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            let _ = p
                .view
                .evaluate_script(&format!("window.itda&&itda.recv({v})"));
        }
    });
}
fn status() {
    let v = state();
    if v["screen"] == true && v["access"] == true {
        let _ = std::fs::remove_file(pending_marker());
    }
    PANE.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            // Never above System Settings while the guide beside it is up.
            let guiding = GUIDE.with(|g| g.borrow().is_some());
            p.window
                .set_always_on_top(!guiding && !(v["screen"] == true && v["access"] == true));
        }
    });
    reply(v);
    refresh();
}

// A running process can retain a negative permission answer after the setting
// changes. Check the same installed executable without requesting either grant.
// Limit this to one child at a time and at most once per ten seconds.
fn refresh() {
    if CHECKING.with(Cell::get)
        || CHECKED.with(|at| at.get().is_some_and(|at| at.elapsed().as_secs() < 10))
    {
        return;
    }
    CHECKING.with(|v| v.set(true));
    CHECKED.with(|v| v.set(Some(std::time::Instant::now())));
    std::thread::spawn(|| {
        let result = fresh_permissions();
        if let Some(proxy) = super::PROXY.get() {
            let _ = proxy.lock().unwrap().send_event(Wake::Permissions(
                json!({"cmd":"fresh_permissions","value":result}),
            ));
        }
    });
}

fn fresh_permissions() -> Option<Value> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().ok()?)
        .arg("--permission-status")
        .env_remove("SSH_ASKPASS_REQUIRE")
        .env_remove("ITDA_ASKPASS_SECRET")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(exit)) if exit.success() => break,
            Ok(Some(_)) => return None,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) if std::time::Instant::now() >= until => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    let v: Value = serde_json::from_slice(&child.wait_with_output().ok()?.stdout).ok()?;
    (v["screen"].is_boolean() && v["access"].is_boolean()).then_some(v)
}
pub(super) fn show(permission: Option<&str>) {
    if let Some(proxy) = super::PROXY.get() {
        let _ = proxy.lock().unwrap().send_event(Wake::Permissions(
            json!({"cmd":"show","permission":permission}),
        ));
    }
}
fn open(target: &EventLoopWindowTarget<Wake>) -> Result<(), String> {
    if PANE.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.window.set_visible(true);
            p.window.set_minimized(false);
            p.window.set_focus();
            true
        } else {
            false
        }
    }) {
        return Ok(());
    }
    let window = WindowBuilder::new()
        .with_title("Itda — 권한 도우미")
        .with_inner_size(LogicalSize::new(420., 720.))
        .with_min_inner_size(LogicalSize::new(360., 480.))
        .with_always_on_top(true)
        .build(target)
        .map_err(|e| e.to_string())?;
    if let Some(monitor) = window.current_monitor() {
        let pos = monitor.position();
        window.set_outer_position(PhysicalPosition::new(pos.x + 24, pos.y + 80));
    }
    let view=WebViewBuilder::new()
        .with_custom_protocol("itda".into(),|_,req| {
            let body=match req.uri().path(){"/permissions.html"|"/permissions.css"|"/permissions.js"|"/icon.svg"=>super::super::asset(req.uri().path()),_=>None};
            match body {Some((body,mime))=>http::Response::builder().header("Content-Type",mime).header("Cache-Control","no-store").body(body).unwrap(),None=>http::Response::builder().status(404).body(Cow::Borrowed(&b"not found"[..])).unwrap()}
        })
        .with_ipc_handler(|req| {
            let Ok(v)=serde_json::from_str::<Value>(req.body()) else{return};
            if v["cmd"]=="permission_drag" {
                let result=(||{
                    let app=app_path().ok_or("응용 프로그램 폴더의 Itda 앱을 실행해 주세요")?;
                    let view=PANE.with(|p|p.borrow().as_ref().map(|p|p.view.webview())).ok_or("권한 도우미가 닫혔습니다")?;
                    super::native_files::drag(&view,json!({"side":"local","path":app.parent(),"items":[{"name":app.file_name().unwrap().to_string_lossy(),"dir":true}]}))
                })();
                if let Err(error)=result {let mut v=state();v["error"]=json!(error);reply(v);}
                return;
            }
            if matches!(v["cmd"].as_str(),Some("permission_status"|"permission_request"|"permission_reveal"|"permission_copy"|"permission_close"|"permission_restart")) {
                if let Some(proxy)=super::PROXY.get(){let _=proxy.lock().unwrap().send_event(Wake::Permissions(v));}
            }
        })
        .with_navigation_handler(|url|url.starts_with("itda://localhost/permissions.html"))
        .with_new_window_req_handler(|_,_|wry::NewWindowResponse::Deny)
        .with_url("itda://localhost/permissions.html")
        .with_back_forward_navigation_gestures(false).build(&window).map_err(|e|e.to_string())?;
    PANE.with(|p| *p.borrow_mut() = Some(Pane { window, view }));
    Ok(())
}
pub(super) fn handle(v: Value, target: &EventLoopWindowTarget<Wake>) {
    let result = (|| -> Result<(), String> {
        match v["cmd"].as_str().unwrap_or("") {
            "guide_tick" => {
                guide_tick(v["seq"].as_u64().unwrap_or(0), target);
                return Ok(());
            }
            "guide_ready" => {
                guide_intro();
                return Ok(());
            }
            "guide_reveal" | "guide_copy" => {
                let app = app_path().ok_or("응용 프로그램 폴더의 Itda 앱을 실행해 주세요")?;
                if v["cmd"] == "guide_reveal" {
                    std::process::Command::new("/usr/bin/open").arg("-R").arg(&app).spawn().map_err(|e| e.to_string())?;
                } else {
                    let pb = NSPasteboard::generalPasteboard();
                    pb.clearContents();
                    unsafe { pb.setString_forType(&NSString::from_str(&app.to_string_lossy()), NSPasteboardTypeString) };
                    guide_reply(json!({"type":"guide","state":"앱 위치를 복사했어요. + 버튼의 파일 선택 창에서 ⇧⌘G → ⌘V → Return"}));
                }
                return Ok(());
            }
            "fresh_permissions" => {
                CHECKING.with(|v| v.set(false));
                FRESH.with(|fresh| {
                    *fresh.borrow_mut() = (!v["value"].is_null()).then(|| v["value"].clone())
                });
                if !PANE.with(|p| p.borrow().is_some()) {
                    return Ok(());
                }
            }
            "permission_restart" => {
                restart_app()?;
                return Ok(());
            }
            "show" => {
                open(target)?;
                if let Some(permission) = v["permission"].as_str() {
                    request(permission)?;
                    start_guide(permission, target)?;
                }
            }
            "permission_request" => {
                let permission = v["permission"].as_str().unwrap_or("");
                request(permission)?;
                start_guide(permission, target)?;
            }
            "permission_status" => {
                // Manual checks may be frequent; the ordinary poll remains bounded.
                if v["refresh"] == true {
                    CHECKED.with(|v| v.set(None));
                }
            }
            "permission_reveal" => {
                let app = app_path().ok_or("응용 프로그램 폴더의 Itda 앱을 실행해 주세요")?;
                std::process::Command::new("/usr/bin/open")
                    .arg("-R")
                    .arg(app)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            "permission_copy" => {
                let app = app_path().ok_or("응용 프로그램 폴더의 Itda 앱을 실행해 주세요")?;
                let pb = NSPasteboard::generalPasteboard();
                pb.clearContents();
                if !unsafe {
                    pb.setString_forType(
                        &NSString::from_str(&app.to_string_lossy()),
                        NSPasteboardTypeString,
                    )
                } {
                    return Err("앱 위치를 복사하지 못했습니다".into());
                }
                let mut v = state();
                v["copied"] = json!(true);
                reply(v);
                return Ok(());
            }
            "permission_close" => {
                // "나중에" / "완료": a later relaunch should not reopen it by itself.
                let _ = std::fs::remove_file(pending_marker());
                shutdown();
                return Ok(());
            }
            _ => {}
        }
        status();
        Ok(())
    })();
    if let Err(error) = result {
        crate::log!("permission helper: {error}");
        let mut v = state();
        v["error"] = json!(error);
        reply(v);
    }
}
/// Quit, then open this same app again on the helper (it reports the result there).
fn restart_app() -> Result<(), String> {
    let app = app_path().ok_or("응용 프로그램 폴더의 Itda 앱을 실행해 주세요")?;
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(RESTART_HELPER)
        .arg("itda-permission-restart")
        .arg(std::process::id().to_string())
        .arg(app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    super::quit();
    Ok(())
}

fn request(permission: &str) -> Result<(), String> {
    if matches!(permission, "screen" | "access") {
        let _ = std::fs::write(pending_marker(), permission);
        REQUESTED.with(|r| *r.borrow_mut() = Some(permission.into()));
        REQUEST_SEQ.with(|r| r.set(r.get().wrapping_add(1)));
    }
    let url = match permission {
        "screen" => {
            crate::host_mac::request_permission("screen");
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        "access" => {
            crate::host_mac::request_permission("access");
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        _ => return Err("설정할 권한을 확인해 주세요".into()),
    };
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub(super) fn event(id: WindowId, event: &WindowEvent<'_>) -> bool {
    if GUIDE.with(|g| g.borrow().as_ref().is_some_and(|g| g.pane.window.id() == id)) {
        if matches!(event, WindowEvent::CloseRequested) { close_guide(); }
        return true;
    }
    if !PANE.with(|p| p.borrow().as_ref().is_some_and(|p| p.window.id() == id)) {
        return false;
    }
    match event {
        // Closing the helper is the same choice as its "나중에" / "완료" buttons.
        WindowEvent::CloseRequested => {
            let _ = std::fs::remove_file(pending_marker());
            shutdown()
        }
        WindowEvent::Focused(true) => status(),
        _ => {}
    }
    true
}
pub(super) fn shutdown() {
    close_guide();
    REQUESTED.with(|r| *r.borrow_mut() = None);
    let pane = PANE.with(|p| p.borrow_mut().take());
    drop(pane);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guide_sits_beside_settings_on_its_screen() {
        let screen = (0., 0., 1512., 982.);
        // Room on the right: glued to the right edge, arrow points left at Settings.
        let (x, y, side) = guide_origin((200., 100., 700., 600.), screen);
        assert_eq!((x, y, side), (910., 100., "left"));
        // No room on the right: on the left, arrow points right.
        let (x, _, side) = guide_origin((800., 100., 700., 600.), screen);
        assert_eq!((x, side), (490., "right"));
        // Settings filling the screen: stays on screen.
        let (x, y, _) = guide_origin((0., 0., 1512., 982.), screen);
        assert!(x + GUIDE_W <= 1512. && y + GUIDE_H <= 982.);
        // A second display to the right.
        let (x, _, _) = guide_origin((1600., 50., 900., 700.), (1512., 0., 1920., 1080.));
        assert_eq!(x, 2510.);
    }

    #[test]
    fn restart_waits_then_opens_the_same_app_and_guide() {
        use std::{
            fs,
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let root =
            std::env::temp_dir().join(format!("itda-permission-restart-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let stub = root.join("open-stub.sh");
        let output = root.join("args");
        fs::write(
            &stub,
            "printf '%s\\n' \"$@\" > \"$ITDA_RESTART_TEST_OUT\"\n",
        )
        .unwrap();
        let app = root.join("App with spaces/Itda.app");
        let mut old = Command::new("/bin/sleep")
            .arg("0.3")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let script = RESTART_HELPER.replace("/usr/bin/open", "/bin/sh \"$ITDA_RESTART_TEST_STUB\"");
        let before = Instant::now();
        let mut helper = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .arg("itda-permission-restart")
            .arg(old.id().to_string())
            .arg(&app)
            .env("ITDA_RESTART_TEST_STUB", &stub)
            .env("ITDA_RESTART_TEST_OUT", &output)
            .spawn()
            .unwrap();
        assert!(!output.exists());
        assert!(old.wait().unwrap().success());
        assert!(helper.wait().unwrap().success());
        assert!(before.elapsed() >= Duration::from_millis(250));
        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            format!("-a\n{}\nitda://permissions\n", app.display())
        );
        fs::remove_dir_all(root).unwrap();
    }
}

// ---------------------------------------------------------------- guide beside System Settings
//
// While System Settings shows the list for a permission, a small window sits right next to it
// with Itda's icon to drag into that list (or a reminder to flip the switch). It follows the
// Settings window, steps aside when another app is in front, shows ✓ once the permission is
// on, and closes by itself. Window bounds and the frontmost app need no permission to read.

struct Guide {
    pane: Pane,
    permission: String,
    started: std::time::Instant,
    checked: std::time::Instant,
    done: Option<std::time::Instant>,
    side: &'static str,
}
thread_local! { static GUIDE: RefCell<Option<Guide>> = const { RefCell::new(None) }; }
static GUIDE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const SETTINGS_ID: &str = "com.apple.systempreferences";
const GUIDE_W: f64 = 300.;
const GUIDE_H: f64 = 470.;

fn guide_reply(v: Value) {
    GUIDE.with(|g| {
        if let Some(g) = g.borrow().as_ref() {
            let _ = g.pane.view.evaluate_script(&format!("window.itda&&itda.recv({v})"));
        }
    });
}

fn guide_intro() {
    let permission = GUIDE.with(|g| g.borrow().as_ref().map(|g| g.permission.clone()));
    let Some(permission) = permission else { return };
    let step = if permission == "screen" { "1/2 단계 · 화면 보기" } else { "2/2 단계 · 키보드·마우스 조작" };
    guide_reply(json!({"type":"guide","permission":permission,"step":step,"app_path":app_path(),"state":"설정 창의 목록이 보이면 Itda를 켜세요."}));
}

fn start_guide(permission: &str, target: &EventLoopWindowTarget<Wake>) -> Result<(), String> {
    let now = std::time::Instant::now();
    let reused = GUIDE.with(|g| {
        let mut g = g.borrow_mut();
        let Some(g) = g.as_mut() else { return false };
        (g.permission, g.started, g.checked, g.done) = (permission.into(), now, now, None);
        true
    });
    if !reused {
        let window = WindowBuilder::new()
            .with_title("Itda 권한 도우미")
            .with_inner_size(LogicalSize::new(GUIDE_W, GUIDE_H))
            .with_resizable(false)
            .with_visible(false)
            .with_always_on_top(true)
            .build(target)
            .map_err(|e| e.to_string())?;
        let view = WebViewBuilder::new()
            .with_custom_protocol("itda".into(), |_, req| {
                let body = match req.uri().path() {
                    "/permission-guide.html" | "/permission-guide.css" | "/permission-guide.js" | "/icon.svg" => super::super::asset(req.uri().path()),
                    _ => None,
                };
                match body {
                    Some((body, mime)) => http::Response::builder().header("Content-Type", mime).header("Cache-Control", "no-store").body(body).unwrap(),
                    None => http::Response::builder().status(404).body(Cow::Borrowed(&b"not found"[..])).unwrap(),
                }
            })
            .with_ipc_handler(|req| {
                let Ok(v) = serde_json::from_str::<Value>(req.body()) else { return };
                if v["cmd"] == "guide_drag" {
                    // Must start inside this mouse-drag event, on the UI thread.
                    let result = (|| {
                        let app = app_path().ok_or("응용 프로그램 폴더의 Itda 앱을 실행해 주세요")?;
                        let view = GUIDE.with(|g| g.borrow().as_ref().map(|g| g.pane.view.webview())).ok_or("권한 도우미가 닫혔습니다")?;
                        super::native_files::drag(&view, json!({"side":"local","path":app.parent(),"items":[{"name":app.file_name().unwrap().to_string_lossy(),"dir":true}]}))
                    })();
                    if let Err(e) = result { guide_reply(json!({"type":"guide","state":e,"error":true})); }
                    return;
                }
                if matches!(v["cmd"].as_str(), Some("guide_ready" | "guide_reveal" | "guide_copy")) {
                    if let Some(proxy) = super::PROXY.get() { let _ = proxy.lock().unwrap().send_event(Wake::Permissions(v)); }
                }
            })
            .with_navigation_handler(|url| url.starts_with("itda://localhost/permission-guide.html"))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_url("itda://localhost/permission-guide.html")
            .with_back_forward_navigation_gestures(false)
            .build(&window)
            .map_err(|e| e.to_string())?;
        GUIDE.with(|g| *g.borrow_mut() = Some(Guide { pane: Pane { window, view }, permission: permission.into(), started: now, checked: now, done: None, side: "" }));
    }
    // The main helper steps back so it never covers the Settings list.
    PANE.with(|p| if let Some(p) = p.borrow().as_ref() { p.window.set_always_on_top(false) });
    guide_intro();
    let seq = GUIDE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        while GUIDE_SEQ.load(std::sync::atomic::Ordering::SeqCst) == seq {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let Some(proxy) = super::PROXY.get() else { return };
            if proxy.lock().unwrap().send_event(Wake::Permissions(json!({"cmd":"guide_tick","seq":seq}))).is_err() { return; }
        }
    });
    Ok(())
}

fn close_guide() {
    GUIDE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let guide = GUIDE.with(|g| g.borrow_mut().take());
    drop(guide);
}

fn ns_window(w: &tao::window::Window) -> &objc2_app_kit::NSWindow {
    use tao::platform::macos::WindowExtMacOS;
    // SAFETY: tao owns a live NSWindow for as long as `w`.
    unsafe { &*(w.ns_window() as *const objc2_app_kit::NSWindow) }
}

fn frontmost() -> Option<String> {
    objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?.bundleIdentifier().map(|s| s.to_string())
}

/// The main System Settings window (largest normal-layer window), top-left based points.
fn settings_frame() -> Option<(f64, f64, f64, f64)> {
    use objc2::runtime::AnyObject;
    use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};
    use objc2_foundation::{NSArray, NSDictionary, NSNumber};
    let apps = objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(SETTINGS_ID));
    let pid = apps.firstObject()?.processIdentifier();
    let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements, 0)?;
    // SAFETY: CFArray of CFDictionary is toll-free bridged to NSArray of NSDictionary.
    let list: &NSArray<NSDictionary<NSString, AnyObject>> = unsafe { &*(objc2_core_foundation::CFRetained::as_ptr(&list).as_ptr() as *const _) };
    let num = |d: &NSDictionary<NSString, AnyObject>, k: &str| d.objectForKey(&NSString::from_str(k)).and_then(|o| o.downcast::<NSNumber>().ok()).map(|n| n.doubleValue());
    let mut best: Option<(f64, f64, f64, f64)> = None;
    for i in 0..list.count() {
        let d = list.objectAtIndex(i);
        if num(&d, "kCGWindowOwnerPID") != Some(pid as f64) || num(&d, "kCGWindowLayer") != Some(0.) { continue; }
        let Some(b) = d.objectForKey(&NSString::from_str("kCGWindowBounds")).and_then(|o| o.downcast::<NSDictionary>().ok()) else { continue };
        // SAFETY: the bounds dictionary maps NSString keys to NSNumber values.
        let b: &NSDictionary<NSString, AnyObject> = unsafe { &*(objc2::rc::Retained::as_ptr(&b) as *const _) };
        let (Some(x), Some(y), Some(w), Some(h)) = (num(b, "X"), num(b, "Y"), num(b, "Width"), num(b, "Height")) else { continue };
        if w < 400. || h < 300. { continue; }
        if best.is_none_or(|(_, _, bw, bh)| w * h > bw * bh) { best = Some((x, y, w, h)); }
    }
    best
}

/// Beside the Settings window on its screen: to the right if it fits, else to the left.
fn guide_origin(s: (f64, f64, f64, f64), screen: (f64, f64, f64, f64)) -> (f64, f64, &'static str) {
    let (sx, sy, sw, _) = s;
    let (mx, my, mw, mh) = screen;
    let gap = 10.;
    let (x, side) = if sx + sw + gap + GUIDE_W <= mx + mw { (sx + sw + gap, "left") } else if sx - gap - GUIDE_W >= mx { (sx - gap - GUIDE_W, "right") } else { (mx + mw - GUIDE_W - gap, "left") };
    let y = sy.max(my + 30.).min(my + mh - GUIDE_H - gap);
    (x, y, side)
}

fn guide_granted(permission: &str) -> (bool, bool) {
    let (screen, access) = crate::host_mac::permissions();
    let fresh = FRESH.with(|f| f.borrow().as_ref().map(|f| (f["screen"] == true, f["access"] == true))).unwrap_or((false, false));
    match permission {
        "screen" => (screen || fresh.0, screen),
        _ => (access || fresh.1, access),
    }
}

/// After one permission is on: open the other one's list right away; with both on, relaunch
/// once if Screen Recording still needs it, else bring the helper back to say it is done.
fn advance(permission: &str, target: &EventLoopWindowTarget<Wake>) {
    let other = if permission == "screen" { "access" } else { "screen" };
    if !guide_granted(other).0 {
        let next = request(other).and_then(|()| start_guide(other, target));
        if let Err(e) = next {
            guide_reply(json!({"type":"guide","state":e,"error":true}));
            close_guide();
        }
        return;
    }
    if !crate::host_mac::permissions().0 {
        // Relaunching closes every Itda window: only when nothing else is open in it.
        let busy = super::super::home(|m| m.sessions.len()) + super::file_windows().len();
        if busy == 0 {
            guide_reply(json!({"type":"guide","done":"모두 허용됐어요. 적용하려고 Itda를 다시 여는 중…"}));
            match restart_app() {
                Ok(()) => { GUIDE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst); return; }
                Err(e) => { close_guide(); let _ = open(target); let mut v = state(); v["error"] = json!(format!("Itda를 다시 열지 못했어요: {e}")); reply(v); return; }
            }
        }
        // The helper offers "Itda 다시 열기" itself, naming what will close.
        close_guide();
        let _ = open(target);
        let mut v = state();
        v["notice"] = json!(format!("모두 허용됐어요. 열려 있는 Itda 창 {busy}개를 닫은 뒤 ‘Itda 다시 열기’를 누르면 적용돼요."));
        reply(v);
        return;
    }
    close_guide();
    let _ = open(target);
    status();
}

fn guide_tick(seq: u64, target: &EventLoopWindowTarget<Wake>) {
    if GUIDE_SEQ.load(std::sync::atomic::Ordering::SeqCst) != seq { return; }
    let now = std::time::Instant::now();
    let Some((permission, started, checked, done)) = GUIDE.with(|g| g.borrow().as_ref().map(|g| (g.permission.clone(), g.started, g.checked, g.done))) else { return };
    if done.is_none() && now.duration_since(started).as_secs() > 180 {
        close_guide();
        status();
        return;
    }
    if done.is_some_and(|d| now.duration_since(d).as_millis() > 1400) {
        advance(&permission, target);
        return;
    }
    // Place (or hide) next to the Settings window.
    let front = frontmost();
    let ours = front.as_deref() == Some("dev.jiun.itda") || front.is_none();
    let frame = (front.as_deref() == Some(SETTINGS_ID) || ours).then(settings_frame).flatten();
    GUIDE.with(|g| {
        let mut g = g.borrow_mut();
        let Some(g) = g.as_mut() else { return };
        let w = ns_window(&g.pane.window);
        match frame {
            Some(s) => {
                let screen = target.available_monitors().map(|m| {
                    let k = m.scale_factor();
                    let (p, z) = (m.position(), m.size());
                    (p.x as f64 / k, p.y as f64 / k, z.width as f64 / k, z.height as f64 / k)
                }).find(|(mx, my, mw, mh)| s.0 + s.2 / 2. >= *mx && s.0 + s.2 / 2. < mx + mw && s.1 >= *my - 1. && s.1 < my + mh).unwrap_or((0., 0., 1440., 900.));
                let (x, y, side) = guide_origin(s, screen);
                g.pane.window.set_outer_position(tao::dpi::LogicalPosition::new(x, y));
                if g.side != side { g.side = side; let v = json!({"type":"guide","side":side}); let _ = g.pane.view.evaluate_script(&format!("window.itda&&itda.recv({v})")); }
                if !w.isVisible() { w.setLevel(objc2_app_kit::NSFloatingWindowLevel); w.orderFrontRegardless(); }
            }
            None if done.is_none() => if w.isVisible() { w.orderOut(None) },
            None => {}
        }
    });
    // Watch for the switch (a fresh process sees it even when this one cannot yet).
    if done.is_none() && now.duration_since(checked).as_millis() >= 1500 {
        GUIDE.with(|g| if let Some(g) = g.borrow_mut().as_mut() { g.checked = now });
        let (on, here) = guide_granted(&permission);
        if on {
            GUIDE.with(|g| if let Some(g) = g.borrow_mut().as_mut() { g.done = Some(now) });
            let text = match (permission.as_str(), here) {
                ("screen", false) => "화면 기록이 켜졌어요. Itda를 다시 열면 적용돼요",
                ("screen", true) => "화면 기록이 허용됐어요",
                (_, _) => "키보드·마우스 조작이 허용됐어요",
            };
            guide_reply(json!({"type":"guide","done":text}));
            status();
        } else {
            CHECKED.with(|c| c.set(None));
            refresh();
        }
    }
}
