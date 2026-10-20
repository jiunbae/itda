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
    PANE.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.window
                .set_always_on_top(!(v["screen"] == true && v["access"] == true));
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
                return Ok(());
            }
            "show" => {
                open(target)?;
                if let Some(permission) = v["permission"].as_str() {
                    request(permission)?;
                }
            }
            "permission_request" => request(v["permission"].as_str().unwrap_or(""))?,
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
fn request(permission: &str) -> Result<(), String> {
    if matches!(permission, "screen" | "access") {
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
    if !PANE.with(|p| p.borrow().as_ref().is_some_and(|p| p.window.id() == id)) {
        return false;
    }
    match event {
        WindowEvent::CloseRequested => shutdown(),
        WindowEvent::Focused(true) => status(),
        _ => {}
    }
    true
}
pub(super) fn shutdown() {
    REQUESTED.with(|r| *r.borrow_mut() = None);
    let pane = PANE.with(|p| p.borrow_mut().take());
    drop(pane);
}

#[cfg(test)]
mod tests {
    use super::*;
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
