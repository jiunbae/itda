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
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock};
use tao::dpi::{LogicalSize, PhysicalSize};
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Fullscreen, Theme, Window, WindowBuilder};
use wry::{http, WebView, WebViewBuilder};

pub const PLATFORM: &str = "mac";

enum Wake {
    Drain,
    Menu(muda::MenuId),
}

static PROXY: OnceLock<Mutex<EventLoopProxy<Wake>>> = OnceLock::new();
/// Without a window loop (the test bridge), the UI thread waits here instead.
static HEADLESS: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

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
    window: Window,
    view: WebView,
}

thread_local! {
    static HOME_PANE: RefCell<Option<Pane>> = const { RefCell::new(None) };
    /// Connection windows by connection id.
    static SESSIONS: RefCell<HashMap<String, Pane>> = RefCell::new(HashMap::new());
    /// Windows to open (id, URL, title, full screen) and to close, once the current event is handled.
    static TO_OPEN: RefCell<Vec<(String, String, String, bool)>> = const { RefCell::new(Vec::new()) };
    static TO_CLOSE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TO_FULL: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// The pasteboard text last handed to a connection.
    static LAST_CLIP: RefCell<String> = const { RefCell::new(String::new()) };
}

/// A connection: its window, opened and closed on the UI thread. Dropping it closes it.
pub struct Proc {
    id: String,
}

impl Drop for Proc {
    fn drop(&mut self) {
        crate::gateway::set_typed(&self.id, None);
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

pub fn eval(js: &str) {
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
    let view = WebViewBuilder::new()
        .with_url(&url)
        .with_devtools(std::env::var_os("ITDA_DEVTOOLS").is_some())
        .with_back_forward_navigation_gestures(false)
        .with_background_color((24, 24, 24, 255))
        .with_ipc_handler(move |req| {
            let Ok(v) = serde_json::from_str::<Value>(req.body()) else { return };
            if v["ev"] == "fullscreen" {
                TO_FULL.with(|f| f.borrow_mut().push(sid.clone()));
                return wake();
            }
            if v["ev"] == "clip" {
                // Text copied on the remote device lands on this Mac's pasteboard.
                let text = v["text"].as_str().unwrap_or("").to_string();
                LAST_CLIP.with(|c| *c.borrow_mut() = text.clone());
                std::thread::spawn(move || pbcopy(&text));
                return;
            }
            super::post(Event::Session(sid.clone(), v));
        })
        .with_navigation_handler(|u| u.starts_with("http://127.0.0.1:") || u.starts_with("about:"))
        .with_new_window_req_handler(|u, _| {
            if u.starts_with("https://") {
                open(&u);
            }
            wry::NewWindowResponse::Deny
        })
        .build(&window);
    match view {
        Ok(view) => SESSIONS.with(|s| {
            s.borrow_mut().insert(id, Pane { window, view });
        }),
        Err(e) => super::post(Event::Session(id, json!({"ev": "error", "kind": "other", "msg": format!("화면을 만들 수 없습니다: {e}")}))),
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

/// Text copied on this Mac goes to the connection when its window comes forward.
fn offer_clipboard(id: &str) {
    let text = output("/usr/bin/pbpaste", &[]);
    if text.is_empty() || text.len() > 256 << 10 || LAST_CLIP.with(|c| *c.borrow() == text) {
        return;
    }
    LAST_CLIP.with(|c| *c.borrow_mut() = text.clone());
    SESSIONS.with(|s| {
        if let Some(p) = s.borrow().get(id) {
            let _ = p.view.evaluate_script(&format!("window.itdaPaste&&itdaPaste({})", json!(text)));
        }
    });
}

/// Open and close the windows asked for while handling the last event.
fn flush(target: &EventLoopWindowTarget<Wake>) {
    for id in TO_CLOSE.with(|c| std::mem::take(&mut *c.borrow_mut())) {
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
const MENU_NEW: &str = "new";
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
            &PredefinedMenuItem::quit(None),
        ],
    );
    let new = item(MENU_NEW, "새 연결", Some((Modifiers::SUPER, Code::KeyN)));
    let file = Submenu::with_items("파일", true, &[&new, &PredefinedMenuItem::separator(), &PredefinedMenuItem::close_window(None)]);
    let edit = Submenu::with_items(
        "편집",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
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
    show_home();
    match id.as_ref() {
        MENU_SETTINGS => send(json!({"type": "go", "page": "settings"})),
        MENU_HELP => send(json!({"type": "go", "page": "help"})),
        MENU_NEW => send(json!({"type": "new"})),
        _ => {}
    }
}

fn home_view(window: &Window, dark: bool) -> wry::Result<WebView> {
    let bg = if dark { (32, 32, 32, 255) } else { (243, 243, 243, 255) };
    WebViewBuilder::new()
        .with_custom_protocol("itda".into(), |_id, req| {
            let resp = match super::asset(req.uri().path()) {
                Some((body, mime)) => http::Response::builder().header("Content-Type", mime).header("Cache-Control", "no-store").body(body),
                None => http::Response::builder().status(404).body(Cow::Borrowed(&b"not found"[..])),
            };
            resp.unwrap_or_else(|_| http::Response::new(Cow::Borrowed(&b""[..])))
        })
        .with_ipc_handler(|req| match serde_json::from_str::<Value>(req.body()) {
            // Handled after this callback returns, like every other event.
            Ok(v) => {
                QUEUE.lock().unwrap().push(Event::Session("__ipc".into(), v));
                wake();
            }
            Err(e) => crate::log!("ipc: bad message {e}"),
        })
        .with_navigation_handler(|url| {
            // External links open in the browser, never inside the app.
            if url.starts_with("itda://") {
                return true;
            }
            if url.starts_with("https://") {
                open(&url);
            }
            false
        })
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_url("itda://localhost/index.html")
        .with_background_color(bg)
        .with_devtools(std::env::var_os("ITDA_DEVTOOLS").is_some())
        .with_back_forward_navigation_gestures(false)
        .build(window)
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
    MenuEvent::set_event_handler(Some(|e: MenuEvent| {
        if let Some(p) = PROXY.get() {
            let _ = p.lock().unwrap().send_event(Wake::Menu(e.id));
        }
    }));
    let menu = menu();
    menu.init_for_nsapp();
    let dark = home(|m| is_dark(&m.settings));
    let window = WindowBuilder::new()
        .with_title(crate::APP)
        .with_inner_size(LogicalSize::new(1080.0, 720.0))
        .with_min_inner_size(LogicalSize::new(720.0, 500.0))
        .build(&event_loop)
        .expect("window");
    let view = match home_view(&window, dark) {
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

    event_loop.run(move |event, target, flow| {
        *flow = ControlFlow::Wait;
        let _ = &menu;
        match event {
            TaoEvent::UserEvent(Wake::Drain) => drain(),
            TaoEvent::UserEvent(Wake::Menu(id)) => on_menu(&id),
            // Clicking the Dock icon brings the home back.
            TaoEvent::Reopen { .. } => show_home(),
            TaoEvent::WindowEvent { window_id, event, .. } => {
                let is_home = HOME_PANE.with(|h| h.borrow().as_ref().is_some_and(|p| p.window.id() == window_id));
                let sid = SESSIONS.with(|s| s.borrow().iter().find(|(_, p)| p.window.id() == window_id).map(|(k, _)| k.clone()));
                match event {
                    // The home only hides: the web viewer and connections keep running.
                    WindowEvent::CloseRequested if is_home => with_home(|p| p.window.set_visible(false)),
                    WindowEvent::CloseRequested => {
                        if let Some(id) = sid {
                            TO_CLOSE.with(|c| c.borrow_mut().push(id));
                        }
                    }
                    WindowEvent::Focused(true) => {
                        if let Some(id) = sid {
                            offer_clipboard(&id);
                        }
                    }
                    WindowEvent::ThemeChanged(_) if is_home => send(json!({"type": "system_theme", "dark": system_dark()})),
                    _ => {}
                }
            }
            _ => {}
        }
        flush(target);
    });
}
