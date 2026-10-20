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

fn app_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension().is_some_and(|s| s == "app") && app.is_dir()).then(|| app.to_path_buf())
}
fn state() -> Value {
    let (screen, access) = crate::host_mac::permissions();
    json!({"type":"permissions","screen":screen,"access":access,"app_path":app_path(),"enabled":crate::host_mac::enabled(),"requested":REQUESTED.with(|r|r.borrow().clone()),"requested_seq":REQUEST_SEQ.with(Cell::get)})
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
        PANE.with(|p| {
            if let Some(p) = p.borrow().as_ref() {
                p.window.set_always_on_top(false);
            }
        });
    }
    reply(v);
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
            if matches!(v["cmd"].as_str(),Some("permission_status"|"permission_request"|"permission_reveal"|"permission_copy"|"permission_close")) {
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
            "show" => {
                open(target)?;
                if let Some(permission) = v["permission"].as_str() {
                    request(permission)?;
                }
            }
            "permission_request" => request(v["permission"].as_str().unwrap_or(""))?,
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
