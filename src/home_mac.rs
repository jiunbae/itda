//! macOS side of the home: the WKWebView window (tao + wry), the menu bar, notifications,
//! and each connection in its own window over the web viewer (the gateway's local listener
//! logs in with the saved password; noVNC shows the screen).

use super::{drain, home, is_dark, send, Event, QUEUE};
use crate::store::{self, Host, Settings};
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock};
use tao::dpi::{LogicalSize, PhysicalSize};
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::window::{Fullscreen, Theme, Window, WindowBuilder};
use wry::{http, WebView, WebViewBuilder, WebViewBuilderExtMacos, WebViewExtMacOS};

#[path = "files_mac.rs"]
mod native_files;
#[path = "permissions_mac.rs"]
mod permission_guide;
pub fn permission_helper(permission: Option<&str>) { permission_guide::show(permission); }
pub(super) fn native_file_copy(v: Value) {native_files::copy(v);}
pub(super) fn native_file_paste(v: Value) {native_files::paste(v);}
pub(super) fn native_file_copy_ready(v: &mut Value) { native_files::finish_copy(v); }
pub(super) fn native_file_drag_done(v: &Value) { native_files::drag_done(v); }
pub(super) fn native_file_promise_done(v: &Value) { native_files::promise_done(v); }

pub const PLATFORM: &str = "mac";

#[derive(Clone)]
enum Wake {
    Drain,
    Menu(muda::MenuId),
    Permissions(Value),
    Quit,
    /// This Mac's pasteboard changed (checked while a connection window is in front).
    Pasteboard,
}

static PROXY: OnceLock<Mutex<EventLoopProxy<Wake>>> = OnceLock::new();
/// Without a window loop (the test bridge), the UI thread waits here instead.
static HEADLESS: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// End the app the normal way (views released before thread-local teardown).
pub fn quit() {
    if let Some(p) = PROXY.get() { let _ = p.lock().unwrap().send_event(Wake::Quit); }
}

pub fn wake() {
    match PROXY.get() {
        Some(p) => {
            let _ = p.lock().unwrap().send_event(Wake::Drain);
        }
        None => {
            *HEADLESS.0.lock().unwrap() = true;
            HEADLESS.1.notify_all();
        }
    }
}

struct Pane {
    // WebKit must release its native view before Tao closes the containing NSWindow.
    view: WebView,
    window: Window,
}

impl Drop for Pane {
    fn drop(&mut self) {
        // Wry can retain the native WKWebView after detaching it. Stop its decoder/RAF
        // explicitly rather than relying on WebKit garbage collection of opener links.
        let _=self.view.evaluate_script("window.ItdaMirrorStop?.();window.ItdaMonitorOwner?.shutdown?.()");
    }
}
thread_local! {
    static HOME_PANE: RefCell<Option<Pane>> = const { RefCell::new(None) };
    /// Connection windows by connection id.
    static FILE_PANES: RefCell<HashMap<String, Pane>> = RefCell::new(HashMap::new());
    static FILE_PAGES: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    static FILE_HOSTS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static FILE_REQUESTS: RefCell<Vec<(String, bool, bool, Option<String>)>> = const { RefCell::new(Vec::new()) };
    static MONITOR_PANES: RefCell<HashMap<(String, u32), Pane>> = RefCell::new(HashMap::new());
    static MONITOR_AUTO_FULL: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static HIDDEN_OWNERS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static MONITOR_CLOSE: RefCell<Vec<(String, Option<u32>)>> = const { RefCell::new(Vec::new()) };
    static MONITOR_FULL: RefCell<Vec<(String, u32)>> = const { RefCell::new(Vec::new()) };
    static SESSIONS: RefCell<HashMap<String, Pane>> = RefCell::new(HashMap::new());
    /// Windows to open (id, URL, title, full screen) and to close, once the current event is handled.
    static TO_OPEN: RefCell<Vec<(String, String, String, bool)>> = const { RefCell::new(Vec::new()) };
    static TO_CLOSE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TO_FULL: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Each Mac connection's clipboard over SSH (see clip_ssh), while its window is open.
    static CLIP_BRIDGES: RefCell<HashMap<String, crate::clip_ssh::Bridge>> = RefCell::new(HashMap::new());
}
/// The connection window in front, if any: copying here while it's in front goes there too,
/// and only its Mac's copies come here (read by the clipboard bridges' threads as well).
static FRONT_SESSION: Mutex<Option<String>> = Mutex::new(None);
/// The pasteboard text last handed to a connection, or brought from one (set from the
/// clipboard bridges' threads too).
static LAST_CLIP: Mutex<String> = Mutex::new(String::new());

/// A connection: its window, opened and closed on the UI thread. Dropping it closes it.
pub struct Proc {
    id: String,
}

impl Drop for Proc {
    fn drop(&mut self) {
        crate::gateway::set_typed(&self.id, None);
        crate::gateway::close_local(&self.id);
        TO_CLOSE.with(|c| c.borrow_mut().push(self.id.clone()));
    }
}

fn output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

pub fn system_dark() -> bool {
    output("/usr/bin/defaults", &["read", "-g", "AppleInterfaceStyle"]).eq_ignore_ascii_case("dark")
}

pub fn apply_theme(dark: bool) {
    let system = home(|m| m.settings.theme == "system");
    with_home(|p| p.window.set_theme(if system { None } else if dark { Some(Theme::Dark) } else { Some(Theme::Light) }));
}

/// Macs have no Remote Desktop server.
pub fn rdp_enabled() -> bool {
    false
}

pub fn computer_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| output("/usr/sbin/scutil", &["--get", "ComputerName"])).clone()
}

/// The Bonjour name other devices reach this Mac by ("mac-mini.local").
pub fn local_name() -> String {
    let n = output("/usr/sbin/scutil", &["--get", "LocalHostName"]);
    if n.is_empty() { n } else { format!("{n}.local") }
}

fn with_home(f: impl FnOnce(&Pane)) {
    HOME_PANE.with(|h| {
        if let Some(p) = h.borrow().as_ref() {
            f(p);
        }
    });
}

pub fn file_windows() -> Vec<String> {
    FILE_HOSTS.with(|p| p.borrow().values().cloned().collect())
}

pub fn files_window(id: &str, close: bool) {
    FILE_REQUESTS.with(|q| q.borrow_mut().push((id.into(), close, false, None)));
    wake();
}

pub fn files_new_window(id: &str) { files_new_window_at(id, None); }
pub fn files_new_window_at(id: &str, path: Option<&str>) {
    FILE_REQUESTS.with(|q| q.borrow_mut().push((id.into(), false, true, path.map(str::to_string))));
    wake();
}
/// The main window is the one in front (a message there is seen without a notification).
pub fn home_focused() -> bool {
    HOME_PANE.with(|h| h.borrow().as_ref().is_some_and(|p| p.window.is_visible() && p.window.is_focused()))
}

pub fn files_window_context(window: &str, id: &str, page: &str) {
    let title = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| format!("{} — 파일", h.label())).unwrap_or_else(|| "네트워크 — Itda".into()));
    let exists = FILE_PANES.with(|p| { if let Some(p) = p.borrow().get(window) { p.window.set_title(&title); true } else { false } });
    // A reloaded page has a new token: the old page is gone, so its transfers become the app's.
    if exists { if let Some(old) = FILE_PAGES.with(|p| p.borrow_mut().insert(window.into(), page.into())).filter(|old| old != page) { crate::files::release_page(&old); } FILE_HOSTS.with(|p| {p.borrow_mut().insert(window.into(), id.into());}); super::send_hosts(); }
}

pub fn eval(js: &str) {
    FILE_PANES.with(|all| { for p in all.borrow().values() { let _ = p.view.evaluate_script(js); } });
    with_home(|p| {
        let _ = p.view.evaluate_script(js);
    });
}

pub fn apply_zoom(pct: u16) {
    let z = if (50..=300).contains(&pct) { pct } else { 100 };
    with_home(|p| {
        let _ = p.view.zoom(z as f64 / 100.0);
    });
}

/// Open a file, folder or link with its default app.
pub fn open(target: &str) {
    let _ = Command::new("/usr/bin/open").arg(target).spawn();
}

/// Build a password-free URL for Apple's separate Screen Sharing client.
fn apple_screen_sharing_url(address: &str, user: &str) -> Result<String, String> {
    let address = address.trim();
    let explicit_port = if let Some(rest) = address.strip_prefix('[') {
        let (_, suffix) = rest.split_once(']').ok_or_else(|| "화면 공유 주소를 확인하세요".to_string())?;
        if suffix.is_empty() { None } else { Some(suffix.strip_prefix(':').ok_or_else(|| "화면 공유 포트를 확인하세요".to_string())?) }
    } else if address.matches(':').count() == 1 { address.rsplit_once(':').map(|(_, port)| port) } else { None };
    if explicit_port.is_some_and(|port| port.is_empty() || port.parse::<u16>().map_or(true, |p| p == 0)) {
        return Err("화면 공유 포트를 확인하세요".into());
    }
    let (host, port) = crate::tunnel::split_host(address, 5900);
    if host.is_empty() || port == 0 || !host.bytes().all(|c| c.is_ascii_alphanumeric() || b".-_:%".contains(&c)) {
        return Err("화면 공유 주소를 확인하세요".into());
    }
    let host = if host.contains(':') {
        let (ip, zone) = host.split_once('%').map_or((host.as_str(), None), |(ip, zone)| (ip, Some(zone)));
        if ip.parse::<std::net::Ipv6Addr>().is_err() || zone.is_some_and(|z| z.is_empty() || !z.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))) {
            return Err("화면 공유 IPv6 주소를 확인하세요".into());
        }
        format!("[{}]", host.replace('%', "%25"))
    } else {
        if host.contains('%') { return Err("화면 공유 주소를 확인하세요".into()); }
        host
    };
    let user: String = user.bytes().map(|c| if c.is_ascii_alphanumeric() || b"-._~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect();
    Ok(format!("vnc://{}{}:{port}", if user.is_empty() { String::new() } else { format!("{user}@") }, host))
}
fn apple_screen_sharing(id: &str) {
    let target = home(|m| {
        let h = m.hosts.iter().find(|h| h.id == id && matches!(h.os.as_str(), "mac" | "macos") && !h.is_rdp())
            .ok_or_else(|| "Mac 연결을 선택하세요".to_string())?;
        let user = m.accounts.iter().find(|a| !h.account.is_empty() && a.id == h.account).map_or(h.user.as_str(), |a| a.user.as_str());
        apple_screen_sharing_url(&h.host, user)
    });
    let id = id.to_string();
    std::thread::spawn(move || {
        let result = target.and_then(|url| Command::new("/usr/bin/open").args(["-b", "com.apple.ScreenSharing", &url]).status()
            .map_err(|e| format!("Apple 화면 공유 앱을 열 수 없습니다: {e}"))
            .and_then(|status| if status.success() { Ok(()) } else { Err("Apple 화면 공유 앱을 열 수 없습니다".into()) }));
        super::post(Event::Page(json!({"type":"apple_screen_sharing_result","id":id,"ok":result.is_ok(),"error":result.err()})));
    });
}

pub fn minimize() {
    with_home(|p| p.window.set_minimized(true));
}

pub fn show_home() {
    with_home(|p| {
        p.window.set_visible(true);
        p.window.set_minimized(false);
        p.window.set_focus();
    });
}

pub fn screenshots_dir() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join("Pictures").join(crate::APP)
}

/// AppleScript string literal.
fn as_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn notify(title: &str, text: &str) {
    let script = format!("display notification {} with title {}", as_str(text), as_str(title));
    let _ = Command::new("/usr/bin/osascript").args(["-e", &script]).spawn();
}

/// The system "choose file" dialog for an SSH private key, starting in ~/.ssh.
pub fn pick_file(field: String) {
    std::thread::spawn(move || {
        let ssh = std::env::var("HOME").map(|h| format!("{h}/.ssh")).unwrap_or_default();
        let at = if std::path::Path::new(&ssh).is_dir() { format!(" default location (POSIX file {})", as_str(&ssh)) } else { String::new() };
        let script = format!("POSIX path of (choose file with prompt {}{at} with invisibles)", as_str("SSH 키 파일 선택"));
        let path = output("/usr/bin/osascript", &["-e", &script]);
        if !path.is_empty() {
            super::post(Event::Page(json!({"type": "picked", "field": field, "path": path})));
        }
    });
}

// ---------------------------------------------------------------- connection windows

pub fn spawn_session(host: &Host, _login: &store::Login, pass: String, _settings: &Settings) -> Result<Proc, String> {
    crate::gateway::set_typed(&host.id, Some(pass));
    let url = crate::gateway::local_url(&host.id).map_err(|e| e.to_string())?;
    TO_OPEN.with(|o| o.borrow_mut().push((host.id.clone(), url, host.label().to_string(), host.opts.fullscreen)));
    wake();
    Ok(Proc { id: host.id.clone() })
}

pub fn session_connected(p: &mut Proc, v: &Value, label: &str) {
    let name = v["name"].as_str().unwrap_or("");
    let (w, h) = (v["w"].as_f64().unwrap_or(0.0), v["h"].as_f64().unwrap_or(0.0));
    SESSIONS.with(|s| {
        let s = s.borrow();
        let Some(pane) = s.get(&p.id) else { return };
        pane.window.set_title(&if name.is_empty() || name == label { label.to_string() } else { format!("{label} — {name}") });
        // Size the window to the remote screen, within most of this one.
        if let (Some(mon), true) = (pane.window.current_monitor(), w > 0.0 && h > 0.0) {
            let scale = mon.scale_factor();
            let PhysicalSize { width, height } = mon.size();
            let (sw, sh) = (width as f64 / scale * 0.9, height as f64 / scale * 0.85);
            let k = (sw / w).min(sh / h).min(1.0);
            if !pane.window.is_maximized() {
                pane.window.set_inner_size(LogicalSize::new((w * k).max(480.0), (h * k).max(320.0)));
            }
        }
    });
}

pub fn stop_session(p: &mut Proc) {
    TO_CLOSE.with(|c| c.borrow_mut().push(p.id.clone()));
    wake();
}

pub fn focus_session(p: &Proc) {
    SESSIONS.with(|s| {
        if let Some(pane) = s.borrow().get(&p.id) {
            pane.window.set_visible(true);
            HIDDEN_OWNERS.with(|h| { h.borrow_mut().remove(&p.id); });
            pane.window.set_minimized(false);
            pane.window.set_focus();
        }
    });
}

/// A window ending is never a surprise here (closing it is how people leave).
pub fn reap(p: Proc) -> bool {
    drop(p);
    false
}

fn open_window(target: &EventLoopWindowTarget<Wake>, id: String, url: String, title: String, full: bool) {
    let window = match WindowBuilder::new()
        .with_title(&title)
        .with_fullscreen(full.then_some(Fullscreen::Borderless(None)))
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .build(target)
    {
        Ok(w) => w,
        Err(e) => return super::post(Event::Session(id, json!({"ev": "error", "kind": "other", "msg": format!("창을 열 수 없습니다: {e}")}))),
    };
    let sid = id.clone();
    let popup_id = id.clone();
    let popup_target = (*target).clone();
    let popup_origin = url.parse::<http::Uri>().ok().and_then(|u| u.authority().map(|a| a.to_string())).unwrap_or_default();
    let view = WebViewBuilder::new()
        .with_url(&url)
        .with_accept_first_mouse(true)
        .with_devtools(std::env::var_os("ITDA_DEVTOOLS").is_some())
        .with_back_forward_navigation_gestures(false)
        .with_background_color((24, 24, 24, 255))
        .with_ipc_handler(move |req| {
            let Ok(v) = serde_json::from_str::<Value>(req.body()) else { return };
            if v["ev"] == "apple-screen-sharing" { apple_screen_sharing(&sid); return; }
            if v["ev"] == "trace" { crate::log!("viewer trace: {}", v["text"].as_str().unwrap_or("")); return; }
            if v["ev"] == "monitor-close" || v["ev"] == "monitor-remove" {
                let screen = if v["ev"] == "monitor-remove" { v["screen"].as_u64().and_then(|n| u32::try_from(n).ok()) } else { None };
                let _ = MONITOR_CLOSE.try_with(|q| q.borrow_mut().push((sid.clone(), screen)));
                return wake();
            }
            if v["ev"] == "monitor-window" {
                if let Some(screen)=v["screen"].as_u64().and_then(|n|u32::try_from(n).ok()) {
                    if v["action"] == "full" {MONITOR_FULL.with(|q|q.borrow_mut().push((sid.clone(),screen)));}
                    if v["action"] == "close" {MONITOR_CLOSE.with(|q|q.borrow_mut().push((sid.clone(),Some(screen))));}
                    wake();
                }
                return;
            }
            if v["ev"] == "monitor-state" {
                let full=v["mode"]==2 && v["full"]==true && v["screens"].as_array().is_some_and(|s|s.len()>1);
                SESSIONS.with(|all| {if let Some(p)=all.borrow().get(&sid) {
                    if full && p.window.fullscreen().is_none() {
                        MONITOR_AUTO_FULL.with(|f| {f.borrow_mut().insert(sid.clone());});
                        p.window.set_fullscreen(Some(Fullscreen::Borderless(p.window.current_monitor())));
                    } else if !full && MONITOR_AUTO_FULL.with(|f| f.borrow_mut().remove(&sid)) {p.window.set_fullscreen(None);}
                }});
                return;
            }
            if v["ev"] == "fullscreen" {
                TO_FULL.with(|f| f.borrow_mut().push(sid.clone()));
                return wake();
            }
            if v["ev"] == "clip" {
                // Text copied on the remote device lands on this Mac's pasteboard.
                let text = v["text"].as_str().unwrap_or("").to_string();
                *LAST_CLIP.lock().unwrap() = text.clone();
                std::thread::spawn(move || pbcopy(&text));
                return;
            }
            super::post(Event::Session(sid.clone(), v));
        })
        .with_navigation_handler(|u| u.starts_with("http://127.0.0.1:") || u.starts_with("about:"))
        .with_new_window_req_handler(move |u, features| {
            if let Ok(uri) = u.parse::<http::Uri>() {
                if uri.scheme_str() == Some("http") && uri.authority().is_some_and(|a| a.as_str() == popup_origin)
                    && matches!(uri.path(), "/" | "/index.html") {
                    let query = uri.query().unwrap_or("");
                    let arg = |key: &str| query.split('&').find_map(|part| part.split_once('=').filter(|(k,_)| *k == key).map(|(_,v)| v));
                    if arg("open") == Some(popup_id.as_str()) {
                        if let Some(screen) = arg("mirror").and_then(|v| v.parse::<u32>().ok()) {
                            let index = arg("mirror_index").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(15);
                            return open_monitor_window(&popup_target, &popup_id, screen, index, arg("monitor_full")==Some("1"), arg("monitor_w").and_then(|v|v.parse::<u32>().ok()).unwrap_or(1100), arg("monitor_h").and_then(|v|v.parse::<u32>().ok()).unwrap_or(740), features);
                        }
                    }
                }
            }
            if u.starts_with("https://") { open(&u); }
            wry::NewWindowResponse::Deny
        })
        .build(&window);
    match view {
        Ok(view) => SESSIONS.with(|s| {
            // Only the strict same-origin mirror handler above can create popups.
            unsafe { view.webview().configuration().preferences().setJavaScriptCanOpenWindowsAutomatically(true); }
            s.borrow_mut().insert(id.clone(), Pane { window, view });
            // Text copied on that Mac lands on this Mac's pasteboard.
            let from = id.clone();
            if let Some(bridge) = crate::clip_ssh::start(&id, move |text| {
                // (A Mac whose window is behind mustn't replace what the one in use works with.)
                if FRONT_SESSION.lock().unwrap().as_deref() != Some(from.as_str()) {
                    return;
                }
                *LAST_CLIP.lock().unwrap() = text.clone();
                pbcopy(&text);
            }) {
                CLIP_BRIDGES.with(|b| b.borrow_mut().insert(id, bridge));
            }
        }),
        Err(e) => super::post(Event::Session(id, json!({"ev": "error", "kind": "other", "msg": format!("화면을 만들 수 없습니다: {e}")}))),
    }
}

fn open_monitor_window(target: &EventLoopWindowTarget<Wake>, owner: &str, screen: u32, index: usize, full: bool, remote_w: u32, remote_h: u32, features: wry::NewWindowFeatures) -> wry::NewWindowResponse {
    let key = (owner.to_string(), screen);
    if MONITOR_PANES.with(|p| p.borrow().contains_key(&key)) { return wry::NewWindowResponse::Deny; }
    if MONITOR_PANES.with(|p| p.borrow().keys().filter(|(id,_)| id == owner).count()) >= 15 { return wry::NewWindowResponse::Deny; }
    let local = SESSIONS.with(|all| {
        let all = all.borrow(); let p = all.get(owner)?;
        let current = p.window.current_monitor();
        let others: Vec<_> = p.window.available_monitors().filter(|m| Some(m) != current.as_ref()).collect();
        Some((others.get(index.saturating_sub(1)).cloned(), current))
    });
    let Some((other, current)) = local else { return wry::NewWindowResponse::Deny };
    let monitor = other.clone().or(current);
    let label = home(|m| m.hosts.iter().find(|h| h.id == owner).map(|h| h.label().to_string()).unwrap_or_else(|| crate::APP.into()));
    let mut builder = WindowBuilder::new().with_title(format!("{label} — 모니터 {}", index+1))
        .with_inner_size(LogicalSize::new(1100.0,740.0)).with_min_inner_size(LogicalSize::new(480.0,320.0));
    if let Some(mon) = monitor.clone() {
        let size = mon.size(); let pos = mon.position(); let scale = mon.scale_factor();
        let rw=remote_w.clamp(320,16384) as f64;let rh=remote_h.clamp(200,16384) as f64;
        let fit=(size.width as f64/scale*0.88/rw).min(size.height as f64/scale*0.82/rh).min(1.0);
        builder = builder.with_inner_size(LogicalSize::new((rw*fit).max(480.0),(rh*fit).max(320.0)))
            .with_position(tao::dpi::PhysicalPosition::new(pos.x+40+(if other.is_none(){index as i32*32}else{0}),pos.y+45+(if other.is_none(){index as i32*24}else{0})));
        if full && other.is_some() { builder = builder.with_fullscreen(Some(Fullscreen::Borderless(Some(mon)))); }
    }
    let Ok(window) = builder.build(target) else { return wry::NewWindowResponse::Deny };
    // The related configuration shares the owner's IPC controller. Never register a
    // second handler with the same WebKit name (nor remove it when a mirror is dropped).
    let view = WebViewBuilder::new().with_webview_configuration(features.opener.target_configuration)
        .with_background_color((24,24,24,255)).with_accept_first_mouse(true).with_back_forward_navigation_gestures(false)
        .with_navigation_handler(|u| u.starts_with("http://127.0.0.1:") || u == "about:blank")
        .with_new_window_req_handler(|_,_| wry::NewWindowResponse::Deny).build(&window);
    let Ok(view) = view else { return wry::NewWindowResponse::Deny };
    let webview=view.webview().into_super();
    MONITOR_PANES.with(|p| {p.borrow_mut().insert(key,Pane {window,view});});
    wry::NewWindowResponse::Create { webview }
}
fn close_monitor(owner: &str, screen: u32) {
    let removed=MONITOR_PANES.with(|p| p.borrow_mut().remove(&(owner.to_string(),screen)));
    if removed.is_none() { return; }
    SESSIONS.with(|s| {if let Some(p)=s.borrow().get(owner) {
        let _=p.view.evaluate_script(&format!("window.ItdaMonitorOwner?.closed({screen})"));
    }});
    drop(removed);
    if HIDDEN_OWNERS.with(|h| h.borrow().contains(owner)) && !MONITOR_PANES.with(|p| p.borrow().keys().any(|(id,_)| id==owner)) {
        TO_CLOSE.with(|q| q.borrow_mut().push(owner.to_string()));
    }
}

fn pbcopy(text: &str) {
    use std::io::Write;
    if let Ok(mut c) = Command::new("/usr/bin/pbcopy").stdin(std::process::Stdio::piped()).spawn() {
        if let Some(mut i) = c.stdin.take() {
            let _ = i.write_all(text.as_bytes());
        }
        let _ = c.wait();
    }
}

/// Run `js` in connection `id`'s window (its web viewer).
pub(super) fn session_eval(id: &str, js: &str) {
    SESSIONS.with(|s| {
        if let Some(p) = s.borrow().get(id) {
            let _ = p.view.evaluate_script(js);
        }
    });
}

/// Text copied on this Mac goes to the connection when its window comes forward.
fn offer_clipboard(id: &str) {
    let text = output("/usr/bin/pbpaste", &[]);
    if text.is_empty() || text.len() > 256 << 10 || *LAST_CLIP.lock().unwrap() == text {
        return;
    }
    *LAST_CLIP.lock().unwrap() = text.clone();
    SESSIONS.with(|s| {
        if let Some(p) = s.borrow().get(id) {
            let _ = p.view.evaluate_script(&format!("window.itdaPaste&&itdaPaste({})", json!(text)));
        }
    });
    // A Mac whose screen comes from Screen Sharing gets it over SSH as well.
    CLIP_BRIDGES.with(|b| if let Some(b) = b.borrow().get(id) { b.send(&text) });
}

/// Open and close the windows asked for while handling the last event.
fn flush(target: &EventLoopWindowTarget<Wake>) {
    let file_requests = FILE_REQUESTS.with(|q| std::mem::take(&mut *q.borrow_mut()));
    let files_changed = !file_requests.is_empty();
    for (id, close, new, path) in file_requests {
        if close { crate::files::preview::revoke_owner(&id); if let Some(page) = FILE_PAGES.with(|p| p.borrow_mut().remove(&id)) {crate::files::release_page(&page);}
            let pane = FILE_PANES.with(|p| p.borrow_mut().remove(&id)); FILE_HOSTS.with(|p| {p.borrow_mut().remove(&id);}); drop(pane); continue; }
        let existing = if new { None } else { FILE_HOSTS.with(|p| p.borrow().iter().find(|(_,host)| *host == &id).map(|(key,_)| key.clone())) };
        let exists = FILE_PANES.with(|p| {
            let panes = p.borrow();
            if let Some(p) = existing.as_ref().and_then(|key| panes.get(key)) { p.window.set_minimized(false); p.window.set_focus(); true } else { false }
        });
        if exists { continue; }
        let window_id = crate::store::new_id();
        let title = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| format!("{} — 파일", h.label())).unwrap_or_else(|| "네트워크 — Itda".into()));
        match WindowBuilder::new().with_title(title).with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true).with_inner_size(LogicalSize::new(1000.0, 700.0)).with_min_inner_size(LogicalSize::new(720.0, 480.0)).build(target) {
            Ok(window) => match home_view(&window, home(|m| is_dark(&m.settings)), Some(&id), Some(&window_id), path.as_deref()) {
                Ok(view) => { crate::log!("file window up: {id}"); FILE_HOSTS.with(|p| {p.borrow_mut().insert(window_id.clone(), id);}); FILE_PANES.with(|p| { p.borrow_mut().insert(window_id, Pane { window, view }); }); }
                Err(e) => { crate::files::preview::revoke_owner(&window_id); crate::log!("files webview: {e}"); },
            },
            Err(e) => crate::log!("files window: {e}"),
        }
    }
    if files_changed { super::send_hosts(); }
    for (owner, screen) in MONITOR_CLOSE.with(|q| std::mem::take(&mut *q.borrow_mut())) {
        if let Some(screen)=screen { close_monitor(&owner,screen); }
        else {
            HIDDEN_OWNERS.with(|h| {h.borrow_mut().remove(&owner);});
            SESSIONS.with(|s| {if let Some(p)=s.borrow().get(&owner) {p.window.set_visible(true);p.window.set_focus();}});
            MONITOR_PANES.with(|p| p.borrow_mut().retain(|(id,_),_| id!=&owner));
        }
    }
    for (owner,screen) in MONITOR_FULL.with(|q| std::mem::take(&mut *q.borrow_mut())) {
        MONITOR_PANES.with(|p| {if let Some(p)=p.borrow().get(&(owner,screen)) {p.window.set_fullscreen(if p.window.fullscreen().is_some(){None}else{Some(Fullscreen::Borderless(p.window.current_monitor()))});}});
    }
    for id in TO_CLOSE.with(|c| std::mem::take(&mut *c.borrow_mut())) {
        crate::gateway::close_local(&id);
        HIDDEN_OWNERS.with(|h| {h.borrow_mut().remove(&id);});
        MONITOR_AUTO_FULL.with(|h| {h.borrow_mut().remove(&id);});
        MONITOR_PANES.with(|p| p.borrow_mut().retain(|(owner,_),_| owner!=&id));
        CLIP_BRIDGES.with(|b| b.borrow_mut().remove(&id));
        {
            let mut front = FRONT_SESSION.lock().unwrap();
            if front.as_deref() == Some(id.as_str()) {
                *front = None;
            }
        }
        if let Some(p) = SESSIONS.with(|s| s.borrow_mut().remove(&id)) {
            drop(p);
            super::post(Event::Exit(id));
        }
    }
    for id in TO_FULL.with(|f| std::mem::take(&mut *f.borrow_mut())) {
        SESSIONS.with(|s| {
            if let Some(p) = s.borrow().get(&id) {
                p.window.set_fullscreen(if p.window.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
            }
        });
    }
    for (id, url, title, full) in TO_OPEN.with(|o| std::mem::take(&mut *o.borrow_mut())) {
        open_window(target, id, url, title, full);
    }
}

// ---------------------------------------------------------------- the app

const MENU_SETTINGS: &str = "settings";
const MENU_QUIT: &str = "quit";
const MENU_NEW: &str = "new";
const MENU_FILE_NEW: &str = "file-window-new";
const MENU_HELP: &str = "help";
const MENU_HOME: &str = "home";

/// The menu bar. Without Edit items, ⌘C/⌘V/⌘A don't work in WKWebView text fields.
fn menu() -> Menu {
    let about = AboutMetadata {
        name: Some(crate::APP.into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("내 기기를 잇다".into()),
        website: Some("https://jiun.dev/itda/".into()),
        ..Default::default()
    };
    let item = |id: &str, text: &str, key: Option<(Modifiers, Code)>| MenuItem::with_id(id, text, true, key.map(|(m, c)| Accelerator::new(Some(m), c)));
    let settings = item(MENU_SETTINGS, "설정…", Some((Modifiers::SUPER, Code::Comma)));
    // Not the predefined Quit: it would end the app at once, mid-transfer.
    let quit = item(MENU_QUIT, &format!("{} 종료", crate::APP), Some((Modifiers::SUPER, Code::KeyQ)));
    let app = Submenu::with_items(
        crate::APP,
        true,
        &[
            &PredefinedMenuItem::about(None, Some(about)),
            &PredefinedMenuItem::separator(),
            &settings,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &quit,
        ],
    );
    let new = item(MENU_NEW, "새 연결", Some((Modifiers::SUPER, Code::KeyN)));
    let new_file = item(MENU_FILE_NEW, "새 파일 창", None);
    let file = Submenu::with_items("파일", true, &[&new_file, &new, &PredefinedMenuItem::separator(), &PredefinedMenuItem::close_window(None)]);
    let edit = Submenu::with_items(
        "편집",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &item("file-cut", "잘라내기", Some((Modifiers::SUPER, Code::KeyX))),
            &item("file-copy", "복사", Some((Modifiers::SUPER, Code::KeyC))),
            &item("file-paste", "붙여넣기", Some((Modifiers::SUPER, Code::KeyV))),
            &PredefinedMenuItem::select_all(None),
        ],
    );
    let view = Submenu::with_items("보기", true, &[&PredefinedMenuItem::fullscreen(None)]);
    let home_item = item(MENU_HOME, "Itda 홈", Some((Modifiers::SUPER, Code::Digit1)));
    let window = Submenu::with_items(
        "윈도우",
        true,
        &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None), &PredefinedMenuItem::separator(), &home_item],
    );
    let help_item = item(MENU_HELP, "Itda 도움말", None);
    let help = Submenu::with_items("도움말", true, &[&help_item]);
    let m = Menu::new();
    for s in [app, file, edit, view, window, help].iter().flatten() {
        let _ = m.append(s);
    }
    m
}

fn on_menu(id: &muda::MenuId) {
    if id.as_ref() == MENU_QUIT { super::request_quit(super::QuitReason::User); return; }
    if let Some(action)=match id.as_ref(){"file-copy"=>Some("copy"),"file-cut"=>Some("cut"),"file-paste"=>Some("paste"),_=>None}{
        let script=format!("window.ItdaFiles?.nativeEdit({})",json!(action));
        let handled=FILE_PANES.with(|p|{let p=p.borrow();if let Some(p)=p.values().find(|p|p.window.is_focused()){let _=p.view.evaluate_script(&script);true}else{false}})
            || HOME_PANE.with(|p|p.borrow().as_ref().is_some_and(|p|{if p.window.is_focused(){let _=p.view.evaluate_script(&script);true}else{false}}));
        if !handled{native_files::system_edit(action);}
        return;
    }
    if id.as_ref() == MENU_FILE_NEW || id.as_ref() == MENU_NEW {
        let script = format!("window.ItdaFiles?.nativeNew({})", id.as_ref() == MENU_FILE_NEW);
        let handled = FILE_PANES.with(|p| { let p=p.borrow(); if let Some(p)=p.values().find(|p|p.window.is_focused()) {let _=p.view.evaluate_script(&script);true}else{false} })
            || HOME_PANE.with(|p| p.borrow().as_ref().is_some_and(|p| {if p.window.is_focused(){let _=p.view.evaluate_script(&script);true}else{false}}));
        if handled { return; }
        if id.as_ref() == MENU_FILE_NEW { files_new_window(crate::files::locations::NETWORK); return; }
    }
    show_home();
    match id.as_ref() {
        MENU_SETTINGS => send(json!({"type": "go", "page": "settings"})),
        MENU_HELP => send(json!({"type": "go", "page": "help"})),
        MENU_NEW => send(json!({"type": "new"})),
        _ => {}
    }
}

fn home_view(window: &Window, dark: bool, file: Option<&str>, file_window: Option<&str>, file_path: Option<&str>) -> wry::Result<WebView> {
    let bg = if dark { (32, 32, 32, 255) } else { (243, 243, 243, 255) };
    let drop_id = file_window.map(str::to_string);
    let drag_id = file_window.map(str::to_string);
    let protocol_owner = file_window.unwrap_or("home").to_string();
    let ipc_owner = protocol_owner.clone();
    let navigation_owner = protocol_owner.clone();
    let view = WebViewBuilder::new()
        .with_initialization_script(&format!("window.itdaNativeFiles=true;window.ITDA_FILE_PATH={};", json!(file_path)))
        .with_accept_first_mouse(file.is_some())
        .with_custom_protocol("itda".into(), move |_id, req| {
            if req.uri().path().starts_with("/preview/") { return crate::files::preview::response(&protocol_owner, &req); }
            let resp = match super::asset(req.uri().path()) {
                Some((body, mime)) => http::Response::builder().header("Content-Type", mime).header("Cache-Control", "no-store").body(body),
                None => http::Response::builder().status(404).body(Cow::Borrowed(&b"not found"[..])),
            };
            resp.unwrap_or_else(|_| http::Response::new(Cow::Borrowed(&b""[..])))
        })
        .with_drag_drop_handler(move |event| {
            if let wry::DragDropEvent::Drop { paths, position } = event {
                super::post(Event::Page(json!({"type":"files_drop","window":drop_id,"paths":paths,"position":[position.0,position.1],"origin":native_files::drop_origin()})));
            }
            true
        })
        .with_ipc_handler(move |req| match serde_json::from_str::<Value>(req.body()) {
            // Handled after this callback returns, like every other event.
            Ok(v) => {
                if !crate::files::preview::admit_ipc(&v, &ipc_owner) { return; }
                match v["cmd"].as_str() {
                    Some("apple_screen_sharing") => { apple_screen_sharing(v["id"].as_str().unwrap_or("")); return; }
                    Some("files_system_edit") => {native_files::system_edit(v["action"].as_str().unwrap_or(""));return;}
                    Some("files_native_copy") => { native_files::copy(v); return; }
                    Some("files_native_paste") => { native_files::paste(v); return; }
                    Some("files_native_drag") => {
                        let id=crate::files::scoped_id(v["id"].as_str().unwrap_or(""), &v["window"]);
                        // Release the pane-map borrow before calling AppKit, which can
                        // enter a nested drag event loop and run window callbacks.
                        let view=if let Some(window_id)=&drag_id {
                            FILE_PANES.with(|panes|panes.borrow().get(window_id).map(|p|p.view.webview()))
                        } else { HOME_PANE.with(|pane|pane.borrow().as_ref().map(|p|p.view.webview())) };
                        let result=view.ok_or_else(||"파일 창을 찾을 수 없습니다".to_string()).and_then(|view|native_files::drag(&view,v));
                        if let Err(error)=result { super::post(Event::Page(json!({"type":"files_native_drag_end","id":id,"error":error}))); }
                        return;
                    }
                    _ => {}
                }
                if v["cmd"] == "files_drag" {
                    if let Some(id) = &drag_id {
                        FILE_PANES.with(|panes| { if let Some(p) = panes.borrow().get(id) { let _ = p.window.drag_window(); } });
                    }
                    return;
                }
                QUEUE.lock().unwrap().push(Event::Session("__ipc".into(), v));
                wake();
            }
            Err(e) => crate::log!("ipc: bad message {e}"),
        })
        .with_navigation_handler(move |url| {
            // External links open in the browser, never inside the app.
            if url.starts_with("itda://") {
                crate::files::preview::revoke_owner(&navigation_owner);
                return true;
            }
            if url.starts_with("https://") {
                open(&url);
            }
            false
        })
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_url(format!("itda://localhost/index.html{}", file.map(|id| {
            let host = id.bytes().map(|b| format!("%{b:02X}")).collect::<String>();
            format!("?files={host}&fileWindow={}", file_window.unwrap_or(""))
        }).unwrap_or_default()))
        .with_background_color(bg)
        .with_devtools(std::env::var_os("ITDA_DEVTOOLS").is_some())
        .with_back_forward_navigation_gestures(false)
        .build(window)?;
    native_files::register_drop(&view.webview());
    Ok(view)
}

pub fn run(quick: Option<String>) {
    super::init(quick);
    // Test hook: no windows; the page runs in a browser over the test bridge.
    if let Some(port) = std::env::var("ITDA_TEST_HTTP").ok().and_then(|p| p.parse::<u16>().ok()) {
        super::test_bridge::start(port);
        super::started();
        loop {
            let g = HEADLESS.0.lock().unwrap();
            let mut g = HEADLESS.1.wait_while(g, |w| !*w).unwrap();
            *g = false;
            drop(g);
            drain();
            // No windows here: say where each one would open, so a test can open it instead.
            for (id, url, ..) in TO_OPEN.with(|o| std::mem::take(&mut *o.borrow_mut())) {
                println!("session {id} {url}");
            }
            for id in TO_CLOSE.with(|o| std::mem::take(&mut *o.borrow_mut())) {
                println!("close {id}");
            }
        }
    }

    let event_loop = EventLoopBuilder::<Wake>::with_user_event().build();
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));
    // Copying on this Mac while a connection window is in front sends it there at once (it
    // was sent only when the window came forward).
    std::thread::spawn(|| {
        let mut seen = crate::host_mac::clipboard_seq();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(700));
            let now = crate::host_mac::clipboard_seq();
            if now != seen {
                seen = now;
                if let Some(p) = PROXY.get() {
                    if p.lock().unwrap().send_event(Wake::Pasteboard).is_err() {
                        return;
                    }
                }
            }
        }
    });
    MenuEvent::set_event_handler(Some(|e: MenuEvent| {
        if let Some(p) = PROXY.get() {
            let _ = p.lock().unwrap().send_event(Wake::Menu(e.id));
        }
    }));
    let menu = menu();
    menu.init_for_nsapp();
    let dark = home(|m| is_dark(&m.settings));
    // Started in the background (another Itda turning on high-performance sharing here over
    // SSH): no home window, only the permission helper; the Dock icon brings the home back.
    // Also when macOS reopens it mid-setup ("Quit & Reopen" after a permission is switched on).
    let background = crate::autostart::AT_LOGIN.load(std::sync::atomic::Ordering::Relaxed)
        || (crate::host_mac::enabled() && permission_guide::pending_recently());
    let window = WindowBuilder::new()
        .with_title(crate::APP)
        .with_inner_size(LogicalSize::new(1080.0, 720.0))
        .with_min_inner_size(LogicalSize::new(720.0, 500.0))
        .with_visible(!background)
        .build(&event_loop)
        .expect("window");
    let view = match home_view(&window, dark, None, None, None) {
        Ok(v) => v,
        Err(e) => {
            crate::log!("webview: {e}");
            return;
        }
    };
    HOME_PANE.with(|h| *h.borrow_mut() = Some(Pane { window, view }));
    apply_theme(dark);
    crate::log!("window up");
    super::started();
    // Missing permissions: guide them. Back from "Quit & Reopen" mid-setup: say it is done.
    if let Some(next) = permission_guide::next_missing() { permission_helper(Some(next)); }
    else if (crate::host_mac::enabled() && crate::host_mac::permissions() != (true,true)) || permission_guide::pending() { permission_helper(None); }
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("ITDA_NATIVE_VIEWER_TEST") {
        if url.parse::<http::Uri>().ok().is_some_and(|u| u.scheme_str()==Some("http") && u.host()==Some("127.0.0.1")) {
            open_window(&event_loop, "monitors-fixture".into(), url, "Itda monitor fixture".into(), false);
        }
    }

    event_loop.run(move |event, target, flow| {
        *flow = ControlFlow::Wait;
        let _ = &menu;
        match event {
            // Launched: AppKit has installed its own quit handler by now; ours replaces it.
            TaoEvent::NewEvents(tao::event::StartCause::Init) => {
                quit_event::install();
                let _ = crate::files::PROMISE_BUSY.set(native_files::promise_busy);
            }
            TaoEvent::LoopDestroyed => {
                permission_guide::shutdown();
                crate::files::shutdown();
                native_files::shutdown();
                // Drop related views while all thread locals are still available.
                SESSIONS.with(|p| {for id in p.borrow().keys(){crate::gateway::close_local(id);}});
                let mirrors=MONITOR_PANES.with(|p| std::mem::take(&mut *p.borrow_mut()));drop(mirrors);
                let sessions=SESSIONS.with(|p| std::mem::take(&mut *p.borrow_mut()));drop(sessions);
                let files=FILE_PANES.with(|p| std::mem::take(&mut *p.borrow_mut()));drop(files);
                let home=HOME_PANE.with(|p| p.borrow_mut().take());drop(home);
                crate::log!("all native views released before thread-local teardown");
            },
            TaoEvent::UserEvent(Wake::Drain) => drain(),
            TaoEvent::UserEvent(Wake::Pasteboard) => {
                if let Some(id) = FRONT_SESSION.lock().unwrap().clone() {
                    offer_clipboard(&id);
                }
            }
            TaoEvent::UserEvent(Wake::Quit) => {
                native_files::release_test_pasteboard();
                *flow = ControlFlow::Exit;
            }
            TaoEvent::UserEvent(Wake::Permissions(v)) => permission_guide::handle(v,target),
            TaoEvent::UserEvent(Wake::Menu(id)) => on_menu(&id),
            // Clicking the Dock icon brings the home back; during permission setup (also when
            // `open` reaches the running app to show the helper) the helper instead, alone.
            TaoEvent::Reopen { .. } => if permission_guide::showing() { permission_helper(None) } else { show_home() },
            TaoEvent::Opened { urls } => {
                for url in urls {
                    match url.as_str().trim_end_matches('/') {
                        "itda://permissions" => {permission_helper(None);continue;},
                        "itda://permissions/screen" => {permission_helper(Some("screen"));continue;},
                        "itda://permissions/access" => {permission_helper(Some("access"));continue;},
                        _=>{}
                    }
                    if let Some(id) = crate::files::locations::device_from_url(url.as_str()) {
                        if id == crate::files::locations::NETWORK || home(|m| m.hosts.iter().any(|h| h.id == id)) { files_window(&id, false); }
                    }
                }
            }
            TaoEvent::WindowEvent { window_id, event, .. } => {
                if permission_guide::event(window_id,&event){return;}
                let is_home = HOME_PANE.with(|h| h.borrow().as_ref().is_some_and(|p| p.window.id() == window_id));
                let fid = FILE_PANES.with(|s| s.borrow().iter().find(|(_, p)| p.window.id() == window_id).map(|(k, _)| k.clone()));
                let mid = MONITOR_PANES.with(|s| s.borrow().iter().find(|(_,p)| p.window.id()==window_id).map(|(k,_)| k.clone()));
                let sid = SESSIONS.with(|s| s.borrow().iter().find(|(_, p)| p.window.id() == window_id).map(|(k, _)| k.clone()));
                match event {
                    // The home only hides: the web viewer and connections keep running.
                    WindowEvent::CloseRequested if is_home => with_home(|p| p.window.set_visible(false)),
                    WindowEvent::CloseRequested => {
                        if let Some(id) = fid { FILE_REQUESTS.with(|q| q.borrow_mut().push((id, true, false, None))); }
                        if let Some((owner,screen))=mid { close_monitor(&owner,screen); }
                        if let Some(id) = sid {
                            if MONITOR_PANES.with(|p| p.borrow().keys().any(|(owner,_)| owner==&id)) {
                                HIDDEN_OWNERS.with(|h| {h.borrow_mut().insert(id.clone());});
                                SESSIONS.with(|s| {if let Some(p)=s.borrow().get(&id) {p.window.set_visible(false);}});
                            } else {TO_CLOSE.with(|c| c.borrow_mut().push(id));}
                        }
                    }
                    WindowEvent::Focused(true) => {
                        if is_home { crate::home::home_focus(true); }
                        if let Some((ref owner,_))=mid { offer_clipboard(owner); }
                        let front = sid.clone().or_else(|| mid.as_ref().map(|(owner, _)| owner.clone()));
                        *FRONT_SESSION.lock().unwrap() = front;
                        if let Some(id) = sid {
                            offer_clipboard(&id);
                        }
                    }
                    WindowEvent::Focused(false) if is_home => crate::home::home_focus(false),
                    WindowEvent::Focused(false) if sid.is_some() || mid.is_some() => *FRONT_SESSION.lock().unwrap() = None,
                    WindowEvent::ThemeChanged(_) if is_home => send(json!({"type": "system_theme", "dark": system_dark()})),
                    _ => {}
                }
            }
            _ => {}
        }
        flush(target);
    });
}

#[cfg(test)]
mod apple_screen_sharing_tests {
    use super::apple_screen_sharing_url;
    #[test]
    fn urls_keep_ports_ipv6_and_encoded_usernames_without_passwords() {
        assert_eq!(apple_screen_sharing_url("mac.local", "june").unwrap(), "vnc://june@mac.local:5900");
        assert_eq!(apple_screen_sharing_url("mac.local:5901", "a:b@한글").unwrap(), "vnc://a%3Ab%40%ED%95%9C%EA%B8%80@mac.local:5901");
        assert_eq!(apple_screen_sharing_url("[fe80::1%en0]:5902", "").unwrap(), "vnc://[fe80::1%25en0]:5902");
        assert_eq!(apple_screen_sharing_url("::1", "").unwrap(), "vnc://[::1]:5900");
        for address in ["", "mac/path", "user:password@mac", "mac?x", "mac#x", "mac\n--args", "https://mac", "mac:0", "mac:no", "mac:65536", "[::1]oops", "[::1]:bad"] {
            assert!(apple_screen_sharing_url(address, "user").is_err(), "accepted {address}");
        }
    }
}

/// The Dock's Quit, logging out and AppleScript `quit` reach the app as the Apple Event
/// kAEQuitApplication. AppKit's own handler terminates at once (tao answers no
/// applicationShouldTerminate:), so a handler of ours takes that event (public
/// NSAppleEventManager API; tao's delegate is not touched) and asks like ⌘Q does.
mod quit_event {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{define_class, msg_send, sel, AnyThread, ClassType};

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "ItdaQuitEventHandler"]
        struct Handler;
        impl Handler {
            #[unsafe(method(handleQuit:withReplyEvent:))]
            fn handle_quit(&self, _event: &AnyObject, reply: &AnyObject) {
                // Not ending now (asked, or transfers being stopped): the sender (logout,
                // AppleScript) is told the quit was cancelled, userCanceledErr. A quit the
                // person then confirms is their own.
                if !super::super::request_quit(super::super::QuitReason::User) {
                    unsafe {
                        let code: *mut AnyObject = msg_send![objc2::class!(NSAppleEventDescriptor), descriptorWithInt32: USER_CANCELED];
                        if let Some(code) = code.as_ref() {
                            let _: () = msg_send![reply, setParamDescriptor: code, forKeyword: KEY_ERROR_NUMBER];
                        }
                    }
                }
            }
        }
    );

    const AEVT: u32 = u32::from_be_bytes(*b"aevt");
    const KEY_ERROR_NUMBER: u32 = u32::from_be_bytes(*b"errn");
    const USER_CANCELED: i32 = -128;
    const QUIT: u32 = u32::from_be_bytes(*b"quit");

    pub fn install() {
        let handler: Retained<Handler> = unsafe { msg_send![Handler::alloc(), init] };
        unsafe {
            let manager: *mut AnyObject = msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager];
            let Some(manager) = manager.as_ref() else { return };
            let _: () = msg_send![manager, setEventHandler: &*handler, andSelector: sel!(handleQuit:withReplyEvent:), forEventClass: AEVT, andEventID: QUIT];
        }
        // The manager doesn't retain its handler: kept for the app's life.
        std::mem::forget(handler);
        let _ = Handler::class();
    }
}

pub fn eval_files(owner: &str, js: &str) {
    if owner == "home" { with_home(|p| { let _ = p.view.evaluate_script(js); }); }
    else { FILE_PANES.with(|all| { if let Some(p) = all.borrow().get(owner) { let _ = p.view.evaluate_script(js); } }); }
}
